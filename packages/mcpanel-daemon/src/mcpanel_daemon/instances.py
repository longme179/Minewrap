"""InstanceManager: builds and owns per-instance runtime objects."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Coroutine
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from mcpanel_common.api import (
    ConfigUpdateRequest,
    DiscordConfigUpdate,
    DiscordConfigView,
    InstanceConfigView,
    InstanceSnapshot,
    InstanceSummary,
    LogLine,
    LogsResponse,
)
from mcpanel_common.config import DaemonConfig, DiscordConfig, InstanceConfig

from mcpanel_daemon.config_store import save_config
from mcpanel_daemon.events import EventBus
from mcpanel_daemon.log_buffer import LogBuffer
from mcpanel_daemon.log_file import RotatingLogFile
from mcpanel_daemon.poller import McstatusProber, StatusPoller
from mcpanel_daemon.process_manager import ProcessError, ProcessManager, State
from mcpanel_daemon.snapshot import InstanceState

logger = logging.getLogger(__name__)


class UnknownInstanceError(KeyError):
    """Raised when an instance id does not exist in the configuration."""


@dataclass
class ManagedInstance:
    config: InstanceConfig
    buffer: LogBuffer
    log_file: RotatingLogFile
    state: InstanceState
    process: ProcessManager
    poller: StatusPoller

    def summary(self) -> InstanceSummary:
        self.state.update_process(self.process.status())
        return self.state.summary()

    def snapshot(self) -> InstanceSnapshot:
        self.state.update_process(self.process.status())
        return self.state.snapshot()


class InstanceManager:
    def __init__(self, config: DaemonConfig, config_path: Path, bus: EventBus) -> None:
        self._config = config
        self._config_path = config_path
        self._bus = bus
        self._instances: dict[str, ManagedInstance] = {
            cfg.id: self._build(cfg) for cfg in config.instances
        }
        self._bg_tasks: set[asyncio.Task[None]] = set()
        self._running = False

    @property
    def bus(self) -> EventBus:
        return self._bus

    @property
    def config(self) -> DaemonConfig:
        return self._config

    # ---------------------------------------------------------------- lifecycle

    async def start(self) -> None:
        self._running = True
        for instance in self._instances.values():
            instance.poller.start()
        for instance in self._instances.values():
            if instance.config.autostart:
                try:
                    await instance.process.start()
                except ProcessError as exc:
                    logger.error("autostart failed for %s: %s", instance.config.id, exc)
        logger.info("instance manager started with %d instance(s)", len(self._instances))

    async def stop(self) -> None:
        self._running = False
        tasks = list(self._bg_tasks)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        for instance in self._instances.values():
            await instance.poller.stop()
        for instance in self._instances.values():
            await instance.process.close()

    # ------------------------------------------------------------------ lookups

    def get(self, instance_id: str) -> ManagedInstance | None:
        return self._instances.get(instance_id)

    def require(self, instance_id: str) -> ManagedInstance:
        instance = self._instances.get(instance_id)
        if instance is None:
            raise UnknownInstanceError(instance_id)
        return instance

    def summaries(self) -> list[InstanceSummary]:
        return [instance.summary() for instance in self._instances.values()]

    def snapshot(self, instance_id: str) -> InstanceSnapshot:
        return self.require(instance_id).snapshot()

    # ------------------------------------------------------------------ actions

    def request_start(self, instance_id: str) -> None:
        instance = self.require(instance_id)
        if instance.process.state not in (State.OFFLINE, State.CRASHED):
            raise ProcessError(f"cannot start while {instance.process.state.value}")
        self._run_bg(instance.process.start())

    def request_stop(self, instance_id: str) -> None:
        instance = self.require(instance_id)
        if instance.process.state is State.OFFLINE:
            raise ProcessError("instance is offline")
        self._run_bg(instance.process.stop())

    def request_restart(self, instance_id: str) -> None:
        instance = self.require(instance_id)
        if instance.process.state is State.OFFLINE:
            raise ProcessError("instance is offline; use start")
        self._run_bg(instance.process.restart())

    async def send_command(self, instance_id: str, command: str) -> None:
        instance = self.require(instance_id)
        await instance.process.send_command(command)

    # ------------------------------------------------------------- logs/config

    def logs(self, instance_id: str, limit: int, since_seq: int) -> LogsResponse:
        instance = self.require(instance_id)
        lines: list[LogLine] = instance.buffer.since(since_seq)
        if limit > 0:
            lines = lines[-limit:]
        return LogsResponse(
            instance_id=instance_id,
            first_seq=instance.buffer.first_seq,
            last_seq=instance.buffer.last_seq,
            lines=lines,
        )

    def backfill(self, instance_id: str, since_seq: int | None) -> list[LogLine]:
        instance = self.require(instance_id)
        if since_seq is None:
            return instance.buffer.tail(self._config.daemon.defaults.log_buffer_lines)
        return instance.buffer.since(since_seq)

    def config_view(self, instance_id: str) -> InstanceConfigView:
        return _config_view(self.require(instance_id).config)

    async def update_config(
        self, instance_id: str, request: ConfigUpdateRequest
    ) -> InstanceConfigView:
        instance = self.require(instance_id)
        existing = instance.config
        new_config = InstanceConfig(
            id=existing.id,
            name=request.name or existing.id,
            work_dir=request.work_dir,
            java_path=request.java_path,
            jvm_args=request.jvm_args,
            launch_args=request.launch_args,
            autostart=request.autostart,
            probe=request.probe,
            restart=request.restart,
            rcon=request.rcon,
            discord=_discord_config(existing.discord, request.discord),
        )

        instance.config = new_config
        instance.process.update_config(new_config)
        instance.state.name = new_config.name
        if self._running:
            await instance.poller.stop()
            instance.poller = StatusPoller(
                McstatusProber(new_config.probe),
                self._config.daemon.status_poll_interval,
                instance.state.update_probe,
            )
            instance.poller.start()

        for index, cfg in enumerate(self._config.instances):
            if cfg.id == instance_id:
                self._config.instances[index] = new_config
                break
        save_config(self._config_path, self._config)
        return _config_view(new_config)

    async def discord_test(self, instance_id: str) -> None:
        self.require(instance_id)
        raise NotImplementedError("Discord notifier lands in Phase 4")

    # ----------------------------------------------------------------- internals

    def _build(self, cfg: InstanceConfig) -> ManagedInstance:
        defaults = self._config.daemon.defaults
        buffer = LogBuffer(max_lines=defaults.log_buffer_lines)
        log_file = RotatingLogFile(
            Path(self._config.daemon.data_dir) / "logs" / f"{cfg.id}.log",
            max_bytes=defaults.log_max_bytes,
            backups=defaults.log_backups,
        )
        state = InstanceState(
            instance_id=cfg.id,
            name=cfg.name,
            unresponsive_after_failures=self._config.daemon.unresponsive_after_failures,
        )
        process = ProcessManager(cfg, defaults, self._bus, buffer, log_file)
        poller = StatusPoller(
            McstatusProber(cfg.probe),
            self._config.daemon.status_poll_interval,
            state.update_probe,
        )
        return ManagedInstance(
            config=cfg,
            buffer=buffer,
            log_file=log_file,
            state=state,
            process=process,
            poller=poller,
        )

    def _run_bg(self, coro: Coroutine[Any, Any, None]) -> None:
        task = asyncio.create_task(self._guard(coro))
        self._bg_tasks.add(task)
        task.add_done_callback(self._bg_tasks.discard)

    async def _guard(self, coro: Coroutine[Any, Any, None]) -> None:
        try:
            await coro
        except asyncio.CancelledError:
            raise
        except ProcessError as exc:
            logger.warning("background instance action failed: %s", exc)
        except Exception:
            logger.exception("background instance action failed")


def _discord_config(existing: DiscordConfig, update: DiscordConfigUpdate) -> DiscordConfig:
    data = update.model_dump(mode="python", exclude={"webhook_url"})
    if update.webhook_url is not None:
        data["webhook_url"] = update.webhook_url.get_secret_value()
    elif existing.webhook_url is not None:
        data["webhook_url"] = existing.webhook_url.get_secret_value()
    return DiscordConfig.model_validate(data)


def _config_view(cfg: InstanceConfig) -> InstanceConfigView:
    discord = cfg.discord
    return InstanceConfigView(
        id=cfg.id,
        name=cfg.name,
        work_dir=cfg.work_dir,
        java_path=cfg.java_path,
        jvm_args=cfg.jvm_args,
        launch_args=cfg.launch_args,
        autostart=cfg.autostart,
        probe=cfg.probe,
        restart=cfg.restart,
        rcon=cfg.rcon,
        discord=DiscordConfigView(
            enabled=discord.enabled,
            mode=discord.mode,
            interval_seconds=discord.interval_seconds,
            first_push_delay_seconds=discord.first_push_delay_seconds,
            log_lines=discord.log_lines,
            max_log_chars=discord.max_log_chars,
            state_change_posts=discord.state_change_posts,
            state_change_debounce_seconds=discord.state_change_debounce_seconds,
            redact_regexes=discord.redact_regexes,
            webhook_set=discord.webhook_url is not None,
        ),
    )
