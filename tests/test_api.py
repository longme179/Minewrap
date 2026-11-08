"""REST + WebSocket API integration tests (fake Minecraft server, real app)."""

# pyright: basic
# starlette 1.10's TestClient shim (httpx/httpx2) is untyped; strict checking of
# this file would drown in Unknown types from the test client itself. Production
# modules under packages/ remain strict.

from __future__ import annotations

import sys
import time
import tomllib
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any, cast

import pytest
from mcpanel_common.config import DaemonConfig
from mcpanel_daemon.app import create_app
from mcpanel_daemon.auth import TokenAuth
from mcpanel_daemon.config_store import save_config
from mcpanel_daemon.events import EventBus
from mcpanel_daemon.instances import InstanceManager
from mock_discord import MockDiscord
from starlette.testclient import TestClient, WebSocketDenialResponse, WebSocketTestSession

FAKE_SERVER = Path(__file__).resolve().parent / "fake_mc_server.py"
TOKEN = "test-token"
AUTH = {"Authorization": f"Bearer {TOKEN}"}


def make_app(
    tmp_path: Path,
    *fake_args: str,
    discord: dict[str, Any] | None = None,
) -> tuple[TestClient, Path]:
    instance: dict[str, Any] = {
        "id": "test",
        "name": "Test Server",
        "work_dir": str(tmp_path),
        "java_path": sys.executable,
        "launch_args": [str(FAKE_SERVER), *fake_args],
        "restart": {"policy": "never"},
    }
    if discord is not None:
        instance["discord"] = discord
    config = DaemonConfig.model_validate(
        {
            "daemon": {
                "token": TOKEN,
                "data_dir": str(tmp_path / "data"),
                "status_poll_interval": 0.2,
            },
            "instances": [instance],
        }
    )
    config_path = tmp_path / "config.toml"
    save_config(config_path, config)
    manager = InstanceManager(config, config_path, EventBus())
    return TestClient(create_app(manager, TokenAuth(TOKEN))), config_path


@contextmanager
def running_app(
    tmp_path: Path,
    *fake_args: str,
    discord: dict[str, Any] | None = None,
) -> Iterator[tuple[TestClient, Path]]:
    client, config_path = make_app(tmp_path, *fake_args, discord=discord)
    with client:
        yield client, config_path


def wait_state(client: TestClient, state: str, timeout: float = 5.0) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    snapshot: dict[str, Any] = {}
    while time.monotonic() < deadline:
        response = client.get("/api/v1/instances/test", headers=AUTH)
        assert response.status_code == 200
        snapshot = response.json()
        if snapshot["process"]["state"] == state:
            return snapshot
        time.sleep(0.02)
    raise AssertionError(f"state {state!r} not reached; last snapshot: {snapshot}")


def recv_until(
    ws: WebSocketTestSession,
    predicate: Callable[[dict[str, Any]], bool],
    limit: int = 100,
) -> dict[str, Any]:
    for _ in range(limit):
        message = cast(dict[str, Any], ws.receive_json())
        if predicate(message):
            return message
    raise AssertionError("expected WebSocket message not received")


# ------------------------------------------------------------------ REST tests


def test_health_is_public_and_rest_requires_bearer(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _config_path):
        health = client.get("/health")
        assert health.status_code == 200
        body = health.json()
        assert body["status"] == "ok"
        assert body["api_version"]

        assert client.get("/api/v1/instances").status_code == 401
        assert (
            client.get("/api/v1/instances", headers={"Authorization": "Bearer nope"})
            .status_code
            == 401
        )
        assert client.get("/api/v1/instances", headers=AUTH).status_code == 200


def test_list_instances_reports_offline(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        instances = client.get("/api/v1/instances", headers=AUTH).json()
        assert len(instances) == 1
        assert instances[0]["id"] == "test"
        assert instances[0]["combined_state"] == "offline"


def test_unknown_instance_is_404(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        assert client.get("/api/v1/instances/nope", headers=AUTH).status_code == 404
        assert client.post("/api/v1/instances/nope/start", headers=AUTH).status_code == 404
        assert client.get("/api/v1/instances/nope/config", headers=AUTH).status_code == 404


def test_full_lifecycle_start_logs_command_stop(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        assert client.post("/api/v1/instances/test/start", headers=AUTH).status_code == 200
        wait_state(client, "online")

        logs = client.get("/api/v1/instances/test/logs?limit=200", headers=AUTH).json()
        text = "\n".join(line["line"] for line in logs["lines"])
        assert 'Done (1.234s)! For help, type "help" or "?"' in text
        assert logs["last_seq"] >= 1

        command = client.post(
            "/api/v1/instances/test/command", headers=AUTH, json={"command": "say hi"}
        )
        assert command.status_code == 200
        assert command.json()["accepted"] is True

        deadline = time.monotonic() + 5
        echoed = False
        while time.monotonic() < deadline and not echoed:
            logs = client.get("/api/v1/instances/test/logs?limit=200", headers=AUTH).json()
            echoed = any("Executing command: say hi" in line["line"] for line in logs["lines"])
            time.sleep(0.02)
        assert echoed

        assert client.post("/api/v1/instances/test/stop", headers=AUTH).status_code == 200
        snapshot = wait_state(client, "offline")
        assert snapshot["process"]["requested_stop"] is True
        assert snapshot["process"]["exit_code"] == 0


def test_command_while_offline_is_409(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        response = client.post(
            "/api/v1/instances/test/command", headers=AUTH, json={"command": "list"}
        )
        assert response.status_code == 409


def test_config_get_masks_webhook(tmp_path: Path) -> None:
    discord = {"enabled": True, "webhook_url": "https://discord.com/api/webhooks/1/old"}
    with running_app(tmp_path, discord=discord) as (client, _):
        response = client.get("/api/v1/instances/test/config", headers=AUTH)
        assert response.status_code == 200
        body = response.json()
        assert body["discord"]["webhook_set"] is True
        assert "old" not in response.text
        assert "webhook_url" not in response.text


def test_config_put_updates_persists_and_keeps_secret(tmp_path: Path) -> None:
    discord = {"enabled": True, "webhook_url": "https://discord.com/api/webhooks/1/old"}
    with running_app(tmp_path, discord=discord) as (client, config_path):
        body: dict[str, Any] = {
            "name": "Renamed",
            "work_dir": str(tmp_path),
            "discord": {
                "enabled": True,
                "interval_seconds": 300,
                "webhook_url": "https://discord.com/api/webhooks/2/new",
            },
        }
        response = client.put("/api/v1/instances/test/config", headers=AUTH, json=body)
        assert response.status_code == 200
        payload = response.json()
        assert payload["name"] == "Renamed"
        assert payload["discord"]["webhook_set"] is True
        assert "new" not in response.text

        saved = tomllib.loads(config_path.read_text(encoding="utf-8"))
        assert saved["instances"][0]["discord"]["webhook_url"].endswith("/2/new")
        assert saved["instances"][0]["name"] == "Renamed"

        # Omitting the webhook keeps the stored one.
        response = client.put(
            "/api/v1/instances/test/config",
            headers=AUTH,
            json={"name": "Renamed", "work_dir": str(tmp_path), "discord": {}},
        )
        assert response.status_code == 200
        saved = tomllib.loads(config_path.read_text(encoding="utf-8"))
        assert saved["instances"][0]["discord"]["webhook_url"].endswith("/2/new")

        summaries = client.get("/api/v1/instances", headers=AUTH).json()
        assert summaries[0]["name"] == "Renamed"

        bad = client.put(
            "/api/v1/instances/test/config",
            headers=AUTH,
            json={
                "name": "X",
                "work_dir": str(tmp_path),
                "discord": {"redact_regexes": ["(unclosed"]},
            },
        )
        assert bad.status_code == 422


def test_discord_test_endpoint_reports_disabled(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        response = client.post("/api/v1/instances/test/discord/test", headers=AUTH)
        assert response.status_code == 200
        body = response.json()
        assert body["ok"] is False
        assert body["error"] is not None


def test_discord_test_endpoint_sends_message(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        discord = {"enabled": True, "webhook_url": mock.webhook_url}
        with running_app(tmp_path, discord=discord) as (client, _):
            response = client.post("/api/v1/instances/test/discord/test", headers=AUTH)
            assert response.status_code == 200
            assert response.json()["ok"] is True
            assert any(request["method"] == "POST" for request in mock.requests)
    finally:
        mock.close()


# ------------------------------------------------------------- WebSocket tests


def test_console_ws_backfill_command_and_live_stream(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        client.post("/api/v1/instances/test/start", headers=AUTH)
        wait_state(client, "online")

        url = "/api/v1/instances/test/console"
        with client.websocket_connect(url, headers=AUTH) as ws:
            first = ws.receive_json()
            assert first["type"] == "log"
            assert "seq" in first["line"]

            caught_up = recv_until(ws, lambda m: m["type"] == "caught_up")
            assert caught_up["seq"] >= first["line"]["seq"]

            ws.send_json({"type": "command", "command": "say ws"})
            result = recv_until(ws, lambda m: m["type"] == "command_result")
            assert result["accepted"] is True

            echo = recv_until(
                ws,
                lambda m: m["type"] == "log" and "Executing command: say ws" in m["line"]["line"],
            )
            assert echo["line"]["seq"] > caught_up["seq"]


def test_console_ws_hello_since_seq_replays_from_start(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        client.post("/api/v1/instances/test/start", headers=AUTH)
        wait_state(client, "online")

        url = "/api/v1/instances/test/console"
        with client.websocket_connect(url, headers=AUTH) as ws:
            recv_until(ws, lambda m: m["type"] == "caught_up")
            ws.send_json({"type": "hello", "since_seq": 0})
            replayed = recv_until(ws, lambda m: m["type"] == "log")
            assert replayed["line"]["seq"] == 1
            # Drain the replay until the server confirms we are live again.
            recv_until(ws, lambda m: m["type"] == "caught_up")


def test_console_ws_rejects_bad_token(tmp_path: Path) -> None:
    url = "/api/v1/instances/test/console"
    with (
        running_app(tmp_path) as (client, _),
        pytest.raises(WebSocketDenialResponse) as excinfo,
        client.websocket_connect(url, headers={"Authorization": "Bearer nope"}),
    ):
        pass
    assert excinfo.value.status_code == 401


def test_console_ws_unknown_instance_sends_error(tmp_path: Path) -> None:
    url = "/api/v1/instances/nope/console"
    with (
        running_app(tmp_path) as (client, _),
        client.websocket_connect(url, headers=AUTH) as ws,
    ):
        message = ws.receive_json()
        assert message["type"] == "error"
        assert "unknown instance" in message["error"]


def test_events_ws_hello_and_state_changes(tmp_path: Path) -> None:
    with (
        running_app(tmp_path) as (client, _),
        client.websocket_connect("/api/v1/events", headers=AUTH) as ws,
    ):
        hello = ws.receive_json()
        assert hello["type"] == "hello_ok"
        assert hello["instances"][0]["id"] == "test"

        client.post("/api/v1/instances/test/start", headers=AUTH)
        starting = recv_until(
            ws,
            lambda m: m["type"] == "state_change" and m["new_state"] == "starting",
        )
        assert starting["old_state"] == "offline"
        assert starting["instance_id"] == "test"

        online = recv_until(
            ws, lambda m: m["type"] == "state_change" and m["new_state"] == "online"
        )
        assert online["snapshot"]["process"]["state"] == "online"
        assert online["snapshot"]["probe"]["ok"] is False  # not a real MC server


def test_events_ws_rejects_bad_token(tmp_path: Path) -> None:
    with running_app(tmp_path) as (client, _):
        with pytest.raises(WebSocketDenialResponse) as excinfo, client.websocket_connect(
            "/api/v1/events", headers={"Authorization": "Bearer nope"}
        ):
            pass
        assert excinfo.value.status_code == 401
