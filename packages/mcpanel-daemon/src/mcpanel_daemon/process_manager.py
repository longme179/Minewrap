"""Process manager: one Minecraft server child process and its state machine.

States and transitions are documented in ARCHITECTURE.md. All transitions go
through :meth:`ProcessManager._transition`, are logged, and are published on
the event bus (topic = instance id).

Known limitation: a child dies with the daemon (no detached mode yet).
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import re
import signal
import sys
from dataclasses import dataclass
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path

from mcpanel_common.config import InstanceConfig, InstanceDefaults

from mcpanel_daemon.events import EventBus, LogLineEvent, RestartLoopEvent, StateChange
from mcpanel_daemon.log_buffer import LogBuffer
from mcpanel_daemon.log_file import RotatingLogFile

logger = logging.getLogger(__name__)


class State(StrEnum):
    OFFLINE = "offline"
    STARTING = "starting"
    ONLINE = "online"
    STOPPING = "stopping"
    CRASHED = "crashed"


_ALLOWED_TRANSITIONS: dict[State, frozenset[State]] = {
    State.OFFLINE: frozenset({State.STARTING}),
    State.STARTING: frozenset({State.ONLINE, State.STOPPING, State.CRASHED}),
    State.ONLINE: frozenset({State.STOPPING, State.CRASHED}),
    State.STOPPING: frozenset({State.OFFLINE}),
    State.CRASHED: frozenset({State.STARTING, State.OFFLINE}),
}


class ProcessError(Exception):
    """Invalid operation for the current state (mapped to HTTP errors later)."""


@dataclass(frozen=True, slots=True)
class ProcessStatus:
    state: State
    pid: int | None
    started_at: datetime | None
    stopped_at: datetime | None
    exit_code: int | None
    requested_stop: bool
    forced_stop: bool
    restart_count: int
    crash_count: int
    uptime_seconds: float | None


class ProcessManager:
    def __init__(
        self,
        config: InstanceConfig,
        defaults: InstanceDefaults,
        bus: EventBus,
        buffer: LogBuffer,
        log_file: RotatingLogFile | None = None,
    ) -> None:
        self._config = config
        self._defaults = defaults
        self._bus = bus
        self._buffer = buffer
        self._log_file = log_file
        self._done_re = re.compile(defaults.done_regex)

        self._state = State.OFFLINE
        self._process: asyncio.subprocess.Process | None = None
        self._started_at: datetime | None = None
        self._stopped_at: datetime | None = None
        self._exit_code: int | None = None
        self._requested_stop = False
        self._forced_stop = False
        self._restart_count = 0
        self._crash_count = 0

        self._reader_task: asyncio.Task[None] | None = None
        self._wait_task: asyncio.Task[None] | None = None
        self._startup_task: asyncio.Task[None] | None = None
        self._restart_task: asyncio.Task[None] | None = None
        self._reset_task: asyncio.Task[None] | None = None

        # Replaced on every transition; waiters re-check the state after waking.
        self._state_changed = asyncio.Event()

    # ------------------------------------------------------------- properties

    @property
    def instance_id(self) -> str:
        return self._config.id

    @property
    def state(self) -> State:
        return self._state

    def status(self) -> ProcessStatus:
        uptime: float | None = None
        if self._started_at is not None and self._state in (
            State.STARTING,
            State.ONLINE,
            State.STOPPING,
        ):
            uptime = (datetime.now(UTC) - self._started_at).total_seconds()
        pid: int | None = None
        if (
            self._process is not None
            and self._process.returncode is None
            and self._state in (State.STARTING, State.ONLINE, State.STOPPING)
        ):
            pid = self._process.pid
        return ProcessStatus(
            state=self._state,
            pid=pid,
            started_at=self._started_at,
            stopped_at=self._stopped_at,
            exit_code=self._exit_code,
            requested_stop=self._requested_stop,
            forced_stop=self._forced_stop,
            restart_count=self._restart_count,
            crash_count=self._crash_count,
            uptime_seconds=uptime,
        )

    # --------------------------------------------------------------- commands

    async def start(self) -> None:
        if self._state not in (State.OFFLINE, State.CRASHED):
            raise ProcessError(f"cannot start while {self._state.value}")
        self._cancel_auto_restart()
        self._reset_crash_counters()
        await self._spawn()

    async def stop(self) -> None:
        if self._state is State.OFFLINE:
            raise ProcessError("instance is offline")
        if self._state is State.CRASHED:
            self._cancel_auto_restart()
            self._reset_crash_counters()
            self._transition(State.OFFLINE, "stop_acknowledged")
            return

        self._requested_stop = True
        self._transition(State.STOPPING, "stop_requested")
        process = self._process
        if process is None:  # pragma: no cover - state invariant
            raise ProcessError("no process while running")

        with contextlib.suppress(OSError):
            await self._write_stdin("stop")

        if await self._wait_exit(process, self._defaults.stop_timeout):
            return
        logger.warning("[%s] graceful stop timed out; sending SIGTERM", self.instance_id)
        self._signal_group(signal.SIGTERM)
        if await self._wait_exit(process, self._defaults.kill_grace_seconds):
            return

        logger.warning("[%s] SIGTERM ignored; sending SIGKILL", self.instance_id)
        self._forced_stop = True
        self._signal_group(signal.SIGKILL)
        await process.wait()

    async def restart(self) -> None:
        if self._state is State.OFFLINE:
            raise ProcessError("instance is offline; use start")
        if self._state is State.CRASHED:
            await self._spawn_with_restart_count()
            return
        await self.stop()
        await self.wait_state(State.OFFLINE, timeout_s=self._defaults.kill_grace_seconds + 5)
        await self._spawn_with_restart_count()

    async def send_command(self, command: str) -> None:
        if self._state is not State.ONLINE:
            raise ProcessError(f"cannot send commands while {self._state.value}")
        if "\n" in command or "\r" in command:
            raise ProcessError("commands must be single-line")
        command = command.strip()
        if not command:
            raise ProcessError("command must not be empty")
        try:
            await self._write_stdin(command)
        except OSError as exc:
            raise ProcessError(f"failed to write to server stdin: {exc}") from exc

    async def wait_state(self, target: State, timeout_s: float = 10.0) -> None:
        async with asyncio.timeout(timeout_s):
            while self._state is not target:
                await self._state_changed.wait()

    def update_config(self, config: InstanceConfig) -> None:
        """Apply a validated config; launch changes take effect on next start."""
        if config.id != self._config.id:  # pragma: no cover - API keeps ids fixed
            raise ValueError("instance id is immutable")
        self._config = config

    async def close(self) -> None:
        """Stop background tasks and kill the child (daemon shutdown)."""
        for task in (
            self._reader_task,
            self._wait_task,
            self._startup_task,
            self._restart_task,
            self._reset_task,
        ):
            if task is not None and not task.done():
                task.cancel()
        process = self._process
        if process is not None and process.returncode is None:
            self._signal_group(signal.SIGKILL)
            await process.wait()
        if self._log_file is not None:
            self._log_file.close()

    # --------------------------------------------------------------- internals

    async def _spawn(self) -> None:
        argv = [self._config.java_path, *self._config.jvm_args, *self._config.launch_args]
        self._exit_code = None
        self._stopped_at = None
        self._requested_stop = False
        self._forced_stop = False
        self._transition(State.STARTING, "start_requested")
        logger.info("[%s] spawning: %r in %s", self.instance_id, argv, self._config.work_dir)
        try:
            self._process = await asyncio.create_subprocess_exec(
                *argv,
                cwd=Path(self._config.work_dir),
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT,
                start_new_session=True,
            )
        except OSError as exc:
            self._transition(State.CRASHED, f"spawn_failed: {exc}")
            raise ProcessError(f"failed to spawn server: {exc}") from exc

        self._started_at = datetime.now(UTC)
        self._reader_task = asyncio.create_task(self._read_output())
        self._wait_task = asyncio.create_task(self._wait_for_exit())
        self._startup_task = asyncio.create_task(self._startup_timeout())

    async def _spawn_with_restart_count(self) -> None:
        self._restart_count += 1
        await self._spawn()

    async def _read_output(self) -> None:
        process = self._process
        if process is None or process.stdout is None:  # pragma: no cover - invariant
            return
        async for raw in process.stdout:
            line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
            entry = self._buffer.append(line)
            if self._log_file is not None:
                self._log_file.write(line)
            self._bus.publish(self.instance_id, LogLineEvent(self.instance_id, entry))
            if self._state is State.STARTING and self._done_re.search(line):
                self._transition(State.ONLINE, "done_line")

    async def _wait_for_exit(self) -> None:
        process = self._process
        if process is None:  # pragma: no cover - invariant
            return
        exit_code = await process.wait()
        if self._reader_task is not None and self._reader_task is not asyncio.current_task():
            await asyncio.gather(self._reader_task, return_exceptions=True)

        self._exit_code = exit_code
        self._stopped_at = datetime.now(UTC)
        self._cancel_task(self._startup_task)
        self._cancel_task(self._reset_task)

        final_state = self._state
        if final_state is State.STOPPING:
            self._transition(State.OFFLINE, "stopped")
            return
        if final_state is State.STARTING:
            self._transition(State.CRASHED, "crash_during_start")
            self._handle_crash()
            return
        if final_state is State.ONLINE:
            self._transition(State.CRASHED, "crashed")
            self._handle_crash()
            return
        # Already CRASHED (startup timeout transitioned before the kill).
        self._handle_crash()

    async def _startup_timeout(self) -> None:
        try:
            await asyncio.sleep(self._defaults.startup_timeout)
        except asyncio.CancelledError:
            return
        if self._state is not State.STARTING:
            return
        logger.warning(
            "[%s] startup timed out after %.1fs", self.instance_id, self._defaults.startup_timeout
        )
        self._transition(State.CRASHED, "startup_timeout")
        self._signal_group(signal.SIGTERM)
        process = self._process
        if process is not None:
            try:
                await asyncio.wait_for(process.wait(), self._defaults.kill_grace_seconds)
            except TimeoutError:
                self._forced_stop = True
                self._signal_group(signal.SIGKILL)
                await process.wait()

    async def _auto_restart(self, delay: float) -> None:
        try:
            await asyncio.sleep(delay)
        except asyncio.CancelledError:
            return
        if self._state is not State.CRASHED:
            return
        self._restart_task = None
        logger.info(
            "[%s] auto-restarting (attempt %d/%d)",
            self.instance_id,
            self._crash_count,
            self._config.restart.max_retries,
        )
        await self._spawn_with_restart_count()

    async def _reset_crash_count_after_online(self) -> None:
        try:
            await asyncio.sleep(self._config.restart.reset_after_online_seconds)
        except asyncio.CancelledError:
            return
        if self._state is State.ONLINE:
            self._reset_crash_counters()

    def _handle_crash(self) -> None:
        self._crash_count += 1
        policy = self._config.restart
        if policy.policy == "on-crash" and self._crash_count <= policy.max_retries:
            delay = min(
                policy.backoff_base_seconds * (2 ** (self._crash_count - 1)),
                policy.backoff_max_seconds,
            )
            logger.info("[%s] crash; restarting in %.1fs", self.instance_id, delay)
            self._cancel_task(self._restart_task)
            self._restart_task = asyncio.create_task(self._auto_restart(delay))
        else:
            logger.warning(
                "[%s] restart loop: %d consecutive crashes (max_retries=%d)",
                self.instance_id,
                self._crash_count,
                policy.max_retries,
            )
            self._bus.publish(
                self.instance_id,
                RestartLoopEvent(self.instance_id, self._crash_count),
            )

    def _transition(self, new_state: State, reason: str) -> None:
        old_state = self._state
        if new_state is old_state:
            return
        allowed = _ALLOWED_TRANSITIONS[old_state]
        if new_state not in allowed:  # pragma: no cover - internal bug guard
            raise ProcessError(f"invalid transition {old_state.value} -> {new_state.value}")
        self._state = new_state
        logger.info(
            "[%s] state %s -> %s (%s)", self.instance_id, old_state.value, new_state.value, reason
        )
        self._bus.publish(
            self.instance_id,
            StateChange(self.instance_id, old_state.value, new_state.value, reason),
        )
        self._state_changed.set()
        self._state_changed = asyncio.Event()
        if new_state is not State.STARTING:
            self._cancel_task(self._startup_task)
        if new_state is State.ONLINE:
            self._cancel_task(self._reset_task)
            self._reset_task = asyncio.create_task(self._reset_crash_count_after_online())
        elif new_state is not State.STARTING:
            self._cancel_task(self._reset_task)

    async def _write_stdin(self, text: str) -> None:
        process = self._process
        if process is None or process.stdin is None:
            raise ProcessError("server process is not running")
        process.stdin.write((text + "\n").encode("utf-8"))
        await process.stdin.drain()

    async def _wait_exit(self, process: asyncio.subprocess.Process, timeout_s: float) -> bool:
        try:
            await asyncio.wait_for(process.wait(), timeout_s)
            return True
        except TimeoutError:
            return False

    def _signal_group(self, sig: signal.Signals) -> None:
        process = self._process
        if process is None or process.returncode is not None:
            return
        if sys.platform == "win32":  # pragma: no cover - Linux first
            if sig == signal.SIGTERM:
                process.terminate()
            else:
                process.kill()
            return
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.killpg(os.getpgid(process.pid), sig)

    def _cancel_auto_restart(self) -> None:
        task = self._restart_task
        if task is not None and task is not asyncio.current_task():
            self._restart_task = None
            task.cancel()

    @staticmethod
    def _cancel_task(task: asyncio.Task[None] | None) -> None:
        if task is not None and not task.done() and task is not asyncio.current_task():
            task.cancel()

    def _reset_crash_counters(self) -> None:
        self._crash_count = 0
        self._restart_count = 0
