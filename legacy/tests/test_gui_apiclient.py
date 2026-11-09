"""ApiClient integration tests against a real daemon (uvicorn in a thread)."""

from __future__ import annotations

import json
import socket
import sys
import threading
import time
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest
import uvicorn
from mcpanel_common.api import ConfigUpdateRequest
from mcpanel_common.config import DaemonConfig
from mcpanel_daemon.app import create_app
from mcpanel_daemon.auth import TokenAuth
from mcpanel_daemon.config_store import save_config
from mcpanel_daemon.events import EventBus
from mcpanel_daemon.instances import InstanceManager
from mcpanel_gui.apiclient import (
    ApiError,
    IncompatibleVersionError,
    NodeApi,
    UnauthorizedError,
    api_major,
)

FAKE_SERVER = Path(__file__).resolve().parent / "fake_mc_server.py"
TOKEN = "gui-test-token"


def _closed_port() -> int:
    """A loopback port that is free right now (so probes reliably fail)."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


class DaemonProcess:
    def __init__(self, tmp_path: Path) -> None:
        config = DaemonConfig.model_validate(
            {
                "daemon": {
                    "token": TOKEN,
                    "data_dir": str(tmp_path / "data"),
                    "status_poll_interval": 120,
                },
                "instances": [
                    {
                        "id": "test",
                        "name": "Test Server",
                        "work_dir": str(tmp_path),
                        "java_path": sys.executable,
                        "launch_args": [str(FAKE_SERVER)],
                        "restart": {"policy": "never"},
                        "probe": {"host": "127.0.0.1", "port": _closed_port()},
                    }
                ],
            }
        )
        config_path = tmp_path / "config.toml"
        save_config(config_path, config)
        manager = InstanceManager(config, config_path, EventBus())
        app = create_app(manager, TokenAuth(TOKEN))
        self._server = uvicorn.Server(
            uvicorn.Config(
                app, host="127.0.0.1", port=0, log_level="warning", lifespan="on"
            )
        )
        self._thread = threading.Thread(target=self._server.run, daemon=True)

    def start(self) -> str:
        self._thread.start()
        deadline = time.monotonic() + 10
        while not self._server.started:
            if time.monotonic() > deadline:
                raise RuntimeError("daemon did not start")
            time.sleep(0.01)
        sockets = self._server.servers[0].sockets
        port = sockets[0].getsockname()[1]
        return f"http://127.0.0.1:{port}"

    def close(self) -> None:
        self._server.should_exit = True
        self._thread.join(timeout=10)


@pytest.fixture
def api(tmp_path: Path) -> Iterator[NodeApi]:
    daemon = DaemonProcess(tmp_path)
    url = daemon.start()
    client = NodeApi(url, TOKEN)
    try:
        yield client
    finally:
        client.close()
        daemon.close()


def wait_state(api: NodeApi, state: str, timeout: float = 5.0) -> Any:
    deadline = time.monotonic() + timeout
    snapshot = None
    while time.monotonic() < deadline:
        snapshot = api.get_snapshot("test")
        if snapshot.process.state == state:
            return snapshot
        time.sleep(0.02)
    raise AssertionError(f"state {state!r} not reached; last={snapshot}")


def recv_until(ws: Any, wanted: str, limit: int = 300) -> dict[str, Any]:
    for _ in range(limit):
        message = json.loads(ws.recv())
        if message.get("type") == wanted:
            return message
    raise AssertionError(f"no {wanted!r} message received")


# --------------------------------------------------------------------- REST


def test_health_and_version_check(api: NodeApi) -> None:
    health = api.health()
    assert health.status == "ok"
    assert api_major(health.api_version) == "0"
    assert api.check_compatible().daemon_version


def test_incompatible_version_is_detected(
    api: NodeApi, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("mcpanel_gui.apiclient.API_VERSION", "9.0")
    with pytest.raises(IncompatibleVersionError):
        api.check_compatible()


def test_wrong_token_is_unauthorized(tmp_path: Path) -> None:
    daemon = DaemonProcess(tmp_path)
    url = daemon.start()
    client = NodeApi(url, "wrong-token")
    try:
        with pytest.raises(UnauthorizedError):
            client.list_instances()
    finally:
        client.close()
        daemon.close()


def test_list_and_snapshot(api: NodeApi) -> None:
    summaries = api.list_instances()
    assert [summary.id for summary in summaries] == ["test"]
    assert summaries[0].combined_state == "offline"

    snapshot = api.get_snapshot("test")
    assert snapshot.process.state == "offline"


def test_lifecycle_logs_command_and_config(api: NodeApi, tmp_path: Path) -> None:
    api.start("test")
    wait_state(api, "online")

    logs = api.get_logs("test", limit=100)
    text = "\n".join(line.line for line in logs.lines)
    assert "Done (1.234s)!" in text

    result = api.send_command("test", "say hello from gui")
    assert result.accepted is True

    deadline = time.monotonic() + 5
    echoed = False
    while time.monotonic() < deadline and not echoed:
        logs = api.get_logs("test", limit=200)
        echoed = any("hello from gui" in line.line for line in logs.lines)
        time.sleep(0.02)
    assert echoed

    config = api.get_config("test")
    assert config.id == "test"
    assert config.discord.webhook_set is False

    updated = api.update_config(
        "test",
        ConfigUpdateRequest.model_validate(
            {
                "name": "Renamed",
                "work_dir": str(tmp_path),
                "discord": {"redact_regexes": ["password=\\S+"]},
            }
        ),
    )
    assert updated.name == "Renamed"

    api.stop("test")
    wait_state(api, "offline")


def test_api_errors_are_mapped(api: NodeApi) -> None:
    with pytest.raises(ApiError) as unknown:
        api.get_snapshot("nope")
    assert unknown.value.status_code == 404

    with pytest.raises(ApiError) as wrong_state:
        api.send_command("test", "list")
    assert wrong_state.value.status_code == 409

    discord = api.discord_test("test")
    assert discord.ok is False


# ---------------------------------------------------------------- WebSockets


def test_console_websocket_backfill_and_command(api: NodeApi) -> None:
    api.start("test")
    wait_state(api, "online")

    with api.connect_console("test") as ws:
        first = json.loads(ws.recv())
        assert first["type"] == "log"
        recv_until(ws, "caught_up")

        ws.send(json.dumps({"type": "command", "command": "say ws hello"}))
        result = recv_until(ws, "command_result")
        assert result["accepted"] is True

    api.stop("test")
    wait_state(api, "offline")


def test_events_websocket_hello_and_state_change(api: NodeApi) -> None:
    with api.connect_events() as ws:
        hello = json.loads(ws.recv())
        assert hello["type"] == "hello_ok"
        assert hello["instances"][0]["id"] == "test"

        api.start("test")
        online = None
        for _ in range(300):
            message = json.loads(ws.recv())
            if message.get("type") == "state_change" and message["new_state"] == "online":
                online = message
                break
        assert online is not None
        assert online["snapshot"]["process"]["state"] == "online"

    api.stop("test")
    wait_state(api, "offline")
