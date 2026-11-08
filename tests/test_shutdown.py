"""Graceful shutdown: manager.stop() stops pollers, notifiers and children."""

from __future__ import annotations

import os
import socket
import sys
from pathlib import Path

import pytest
from mcpanel_common.config import DaemonConfig
from mcpanel_daemon.events import EventBus
from mcpanel_daemon.instances import InstanceManager
from mcpanel_daemon.process_manager import State

FAKE_SERVER = Path(__file__).resolve().parent / "fake_mc_server.py"


def _closed_port() -> int:
    """A loopback port that is free right now (so probes reliably fail)."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


async def test_shutdown_stops_children_pollers_and_notifiers(tmp_path: Path) -> None:
    config = DaemonConfig.model_validate(
        {
            "daemon": {"data_dir": str(tmp_path / "data"), "status_poll_interval": 120},
            "instances": [
                {
                    "id": "test",
                    "work_dir": str(tmp_path),
                    "java_path": sys.executable,
                    "launch_args": [str(FAKE_SERVER)],
                    "autostart": True,
                    "restart": {"policy": "never"},
                    "discord": {"enabled": False},
                    "probe": {"host": "127.0.0.1", "port": _closed_port()},
                }
            ],
        }
    )
    manager = InstanceManager(config, tmp_path / "config.toml", EventBus())
    await manager.start()
    instance = manager.require("test")
    await instance.process.wait_state(State.ONLINE, timeout_s=5)

    pid = instance.process.status().pid
    assert pid is not None
    assert instance.poller.running

    await manager.stop()

    # The child was killed and reaped, not left as a zombie.
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)
    assert not instance.poller.running
    assert not instance.notifier.running


async def test_autostart_failure_does_not_abort_manager_start(tmp_path: Path) -> None:
    config = DaemonConfig.model_validate(
        {
            "daemon": {"data_dir": str(tmp_path / "data"), "status_poll_interval": 120},
            "instances": [
                {
                    "id": "broken",
                    "work_dir": str(tmp_path),
                    "java_path": str(tmp_path / "does-not-exist-java"),
                    "launch_args": ["-version"],
                    "autostart": True,
                    "restart": {"policy": "never"},
                    "probe": {"host": "127.0.0.1", "port": _closed_port()},
                }
            ],
        }
    )
    manager = InstanceManager(config, tmp_path / "config.toml", EventBus())
    try:
        await manager.start()  # must not raise; the instance becomes crashed
        instance = manager.require("broken")
        assert instance.process.state is State.CRASHED
    finally:
        await manager.stop()
