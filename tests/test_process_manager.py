"""ProcessManager integration tests against tests/fake_mc_server.py."""

from __future__ import annotations

import asyncio
import sys
from collections.abc import AsyncGenerator, Callable
from contextlib import asynccontextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from mcpanel_common.config import InstanceConfig, InstanceDefaults
from mcpanel_daemon.events import Event, EventBus, RestartLoopEvent, StateChange
from mcpanel_daemon.log_buffer import LogBuffer
from mcpanel_daemon.log_file import RotatingLogFile
from mcpanel_daemon.process_manager import ProcessError, ProcessManager, State

FAKE_SERVER = Path(__file__).resolve().parent / "fake_mc_server.py"
INSTANCE = "test"


@dataclass
class Rig:
    manager: ProcessManager
    bus: EventBus
    buffer: LogBuffer
    log_path: Path


def _config(tmp_path: Path, fake_args: list[str], restart: dict[str, Any] | None) -> InstanceConfig:
    return InstanceConfig.model_validate(
        {
            "id": INSTANCE,
            "work_dir": str(tmp_path),
            "java_path": sys.executable,
            "launch_args": [str(FAKE_SERVER), *fake_args],
            "restart": restart or {"policy": "never"},
        }
    )


def _defaults(**overrides: float) -> InstanceDefaults:
    values: dict[str, Any] = {
        "startup_timeout": 5.0,
        "stop_timeout": 1.0,
        "kill_grace_seconds": 0.5,
        **overrides,
    }
    return InstanceDefaults.model_validate(values)


@asynccontextmanager
async def rig(
    tmp_path: Path,
    *fake_args: str,
    restart: dict[str, Any] | None = None,
    **defaults_overrides: float,
) -> AsyncGenerator[Rig, None]:
    bus = EventBus()
    buffer = LogBuffer(max_lines=200)
    log_path = tmp_path / "logs" / "server.log"
    log_file = RotatingLogFile(log_path, max_bytes=1_000_000, backups=1)
    manager = ProcessManager(
        _config(tmp_path, list(fake_args), restart),
        _defaults(**defaults_overrides),
        bus,
        buffer,
        log_file,
    )
    try:
        yield Rig(manager=manager, bus=bus, buffer=buffer, log_path=log_path)
    finally:
        await manager.close()


async def wait_until(predicate: Callable[[], bool], timeout_s: float = 5.0) -> None:
    async with asyncio.timeout(timeout_s):
        while not predicate():  # noqa: ASYNC110 - test polling helper
            await asyncio.sleep(0.01)


def drain(queue: asyncio.Queue[Event]) -> list[Event]:
    events: list[Event] = []
    while not queue.empty():
        events.append(queue.get_nowait())
    return events


def state_changes(events: list[Event]) -> list[StateChange]:
    return [event for event in events if isinstance(event, StateChange)]


async def test_start_reaches_online_and_emits_transitions(tmp_path: Path) -> None:
    async with rig(tmp_path) as r:
        events = r.bus.subscribe(INSTANCE)
        await r.manager.start()
        await r.manager.wait_state(State.ONLINE)

        status = r.manager.status()
        assert status.pid is not None
        assert status.started_at is not None
        assert status.uptime_seconds is not None

        text = "\n".join(line.line for line in r.buffer.tail(50))
        assert 'Done (1.234s)! For help, type "help" or "?"' in text
        await wait_until(
            lambda: any(
                "Alice[/192.168.50.7:54321] logged in with entity id 42" in line.line
                for line in r.buffer.tail(50)
            )
        )

        changes = state_changes(drain(events))
        assert [change.new_state for change in changes] == ["starting", "online"]
        assert changes[0].reason == "start_requested"
        assert changes[1].reason == "done_line"

        await wait_until(lambda: r.log_path.exists() and "Done" in r.log_path.read_text())


async def test_send_command_is_written_to_stdin(tmp_path: Path) -> None:
    async with rig(tmp_path) as r:
        await r.manager.start()
        await r.manager.wait_state(State.ONLINE)
        await r.manager.send_command("say hi")
        await wait_until(
            lambda: any(
                "Executing command: say hi" in line.line for line in r.buffer.tail(50)
            )
        )


async def test_send_command_rejected_when_not_online(tmp_path: Path) -> None:
    async with rig(tmp_path) as r:
        with pytest.raises(ProcessError, match="offline"):
            await r.manager.send_command("list")


async def test_stop_is_clean_and_flags_requested_stop(tmp_path: Path) -> None:
    async with rig(tmp_path) as r:
        await r.manager.start()
        await r.manager.wait_state(State.ONLINE)
        await r.manager.stop()
        await r.manager.wait_state(State.OFFLINE)

        status = r.manager.status()
        assert status.requested_stop is True
        assert status.forced_stop is False
        assert status.exit_code == 0
        assert status.stopped_at is not None

        await wait_until(lambda: "Stopping the server" in r.log_path.read_text())


async def test_stop_when_offline_raises(tmp_path: Path) -> None:
    async with rig(tmp_path) as r:
        with pytest.raises(ProcessError, match="offline"):
            await r.manager.stop()


async def test_crash_after_done(tmp_path: Path) -> None:
    async with rig(tmp_path, "--crash", "after-done", "--crash-delay", "0.02") as r:
        await r.manager.start()
        await r.manager.wait_state(State.CRASHED)
        await wait_until(lambda: r.manager.status().crash_count == 1)

        status = r.manager.status()
        assert status.exit_code == 1
        assert status.requested_stop is False


async def test_crash_during_start_reason(tmp_path: Path) -> None:
    async with rig(tmp_path, "--crash", "during-start", "--crash-delay", "0.02") as r:
        events = r.bus.subscribe(INSTANCE)
        await r.manager.start()
        await r.manager.wait_state(State.CRASHED)

        changes = state_changes(drain(events))
        assert changes[-1].new_state == "crashed"
        assert changes[-1].reason == "crash_during_start"


async def test_startup_timeout_kills_and_crashes(tmp_path: Path) -> None:
    async with rig(
        tmp_path, "--done-delay", "10", startup_timeout=0.3, kill_grace_seconds=0.2
    ) as r:
        events = r.bus.subscribe(INSTANCE)
        await r.manager.start()
        await r.manager.wait_state(State.CRASHED, timeout_s=5)
        await wait_until(lambda: r.manager.status().crash_count == 1)

        changes = state_changes(drain(events))
        assert changes[-1].reason == "startup_timeout"
        assert r.manager.status().exit_code is not None


async def test_stop_escalates_to_sigkill(tmp_path: Path) -> None:
    async with rig(
        tmp_path,
        "--ignore-stop",
        "--ignore-sigterm",
        stop_timeout=0.2,
        kill_grace_seconds=0.3,
    ) as r:
        await r.manager.start()
        await r.manager.wait_state(State.ONLINE)
        await r.manager.stop()
        await r.manager.wait_state(State.OFFLINE, timeout_s=5)

        status = r.manager.status()
        assert status.forced_stop is True
        assert status.exit_code == -9  # SIGKILL


async def test_stop_during_startup(tmp_path: Path) -> None:
    async with rig(tmp_path, "--done-delay", "10", stop_timeout=0.3, kill_grace_seconds=0.3) as r:
        await r.manager.start()
        await r.manager.wait_state(State.STARTING)
        await r.manager.stop()
        await r.manager.wait_state(State.OFFLINE, timeout_s=5)
        assert r.manager.status().requested_stop is True


async def test_auto_restart_exhausts_retries_and_emits_loop(tmp_path: Path) -> None:
    async with rig(
        tmp_path,
        "--crash",
        "after-done",
        "--crash-delay",
        "0.02",
        restart={
            "policy": "on-crash",
            "max_retries": 1,
            "backoff_base_seconds": 0.1,
            "backoff_max_seconds": 1.0,
            "reset_after_online_seconds": 60,
        },
    ) as r:
        events = r.bus.subscribe(INSTANCE)
        await r.manager.start()
        await wait_until(lambda: r.manager.status().crash_count == 2)

        await asyncio.sleep(0.2)  # no further restarts once retries are exhausted
        status = r.manager.status()
        assert status.state is State.CRASHED
        assert status.crash_count == 2
        assert status.restart_count == 1
        assert any(isinstance(event, RestartLoopEvent) for event in drain(events))


async def test_manual_start_after_crash_resets_counters(tmp_path: Path) -> None:
    async with rig(tmp_path, "--crash", "during-start", "--crash-delay", "0.02") as r:
        await r.manager.start()
        await r.manager.wait_state(State.CRASHED)
        await wait_until(lambda: r.manager.status().crash_count == 1)

        await r.manager.start()  # manual start clears the crash counter
        assert r.manager.state is State.STARTING
        assert r.manager.status().crash_count == 0
        await r.manager.wait_state(State.CRASHED)
        await wait_until(lambda: r.manager.status().crash_count == 1)
        assert r.manager.status().crash_count == 1


async def test_restart_replaces_process(tmp_path: Path) -> None:
    async with rig(tmp_path) as r:
        await r.manager.start()
        await r.manager.wait_state(State.ONLINE)
        first_pid = r.manager.status().pid
        assert first_pid is not None

        await r.manager.restart()
        await r.manager.wait_state(State.ONLINE, timeout_s=10)
        status = r.manager.status()
        assert status.pid is not None
        assert status.pid != first_pid
        assert status.restart_count == 1
