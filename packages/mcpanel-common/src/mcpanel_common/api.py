"""API contract: REST DTOs and WebSocket messages shared by daemon and GUI."""

from __future__ import annotations

from datetime import datetime
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, SecretStr, TypeAdapter, field_validator

from mcpanel_common.config import (
    DiscordConfig,
    ProbeConfig,
    RconConfig,
    RestartConfig,
    validate_redact_regexes,
)

ProcessState = Literal["offline", "starting", "online", "stopping", "crashed"]
CombinedState = Literal[
    "offline", "starting", "online", "stopping", "crashed", "unresponsive"
]


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


# ---------------------------------------------------------------- REST models


class HealthResponse(_StrictModel):
    status: Literal["ok"]
    api_version: str
    daemon_version: str


class InstanceSummary(_StrictModel):
    id: str
    name: str
    combined_state: CombinedState
    players_online: int | None = None
    players_max: int | None = None
    version: str | None = None
    latency_ms: float | None = None


class ProbeInfo(_StrictModel):
    ok: bool
    latency_ms: float | None = None
    players_online: int | None = None
    players_max: int | None = None
    version: str | None = None
    motd: str | None = None
    player_names: list[str] = Field(default_factory=list)
    error: str | None = None
    checked_at: datetime | None = None


class ProcessInfo(_StrictModel):
    state: ProcessState
    pid: int | None = None
    started_at: datetime | None = None
    stopped_at: datetime | None = None
    uptime_seconds: float | None = None
    exit_code: int | None = None
    requested_stop: bool = False
    restart_count: int = 0
    crash_count: int = 0


class InstanceSnapshot(_StrictModel):
    id: str
    name: str
    combined_state: CombinedState
    process: ProcessInfo
    probe: ProbeInfo
    updated_at: datetime


class LogLine(_StrictModel):
    seq: int
    ts: datetime
    line: str


class LogsResponse(_StrictModel):
    instance_id: str
    first_seq: int | None
    last_seq: int | None
    lines: list[LogLine]


class ActionResponse(_StrictModel):
    ok: bool
    error: str | None = None


class CommandRequest(_StrictModel):
    command: str

    @field_validator("command")
    @classmethod
    def _sanitize(cls, v: str) -> str:
        v = v.replace("\r", "").replace("\n", "").strip()
        if not v:
            raise ValueError("command must not be empty")
        return v


class CommandResponse(_StrictModel):
    accepted: bool
    error: str | None = None


class DiscordTestResponse(_StrictModel):
    ok: bool
    error: str | None = None


# Config views: secrets are write-only, never returned.


class DiscordConfigUpdate(_StrictModel):
    enabled: bool = DiscordConfig.model_fields["enabled"].default
    mode: Literal["edit-in-place", "new-message"] = DiscordConfig.model_fields["mode"].default
    interval_seconds: float = DiscordConfig.model_fields["interval_seconds"].default
    first_push_delay_seconds: float = DiscordConfig.model_fields[
        "first_push_delay_seconds"
    ].default
    log_lines: int = DiscordConfig.model_fields["log_lines"].default
    max_log_chars: int = DiscordConfig.model_fields["max_log_chars"].default
    state_change_posts: bool = DiscordConfig.model_fields["state_change_posts"].default
    state_change_debounce_seconds: float = DiscordConfig.model_fields[
        "state_change_debounce_seconds"
    ].default
    redact_regexes: list[str] = Field(default_factory=list)
    webhook_url: SecretStr | None = None  # None = keep the currently stored URL

    @field_validator("redact_regexes")
    @classmethod
    def _compile_regexes(cls, v: list[str]) -> list[str]:
        return validate_redact_regexes(v)


class ConfigUpdateRequest(_StrictModel):
    name: str
    work_dir: str
    java_path: str = "java"
    jvm_args: list[str] = Field(default_factory=list)
    launch_args: list[str] = Field(default_factory=lambda: ["-jar", "server.jar", "nogui"])
    autostart: bool = False
    probe: ProbeConfig = Field(default_factory=ProbeConfig)
    restart: RestartConfig = Field(default_factory=RestartConfig)
    rcon: RconConfig | None = None
    discord: DiscordConfigUpdate = Field(default_factory=DiscordConfigUpdate)


class DiscordConfigView(_StrictModel):
    enabled: bool
    mode: Literal["edit-in-place", "new-message"]
    interval_seconds: float
    first_push_delay_seconds: float
    log_lines: int
    max_log_chars: int
    state_change_posts: bool
    state_change_debounce_seconds: float
    redact_regexes: list[str]
    webhook_set: bool


class InstanceConfigView(_StrictModel):
    id: str
    name: str
    work_dir: str
    java_path: str
    jvm_args: list[str]
    launch_args: list[str]
    autostart: bool
    probe: ProbeConfig
    restart: RestartConfig
    rcon: RconConfig | None
    discord: DiscordConfigView


ConfigResponse = InstanceConfigView

# ------------------------------------------------------- WebSocket messages


class ConsoleHello(_StrictModel):
    type: Literal["hello"]
    since_seq: int | None = None


class ConsoleCommandMessage(_StrictModel):
    type: Literal["command"]
    command: str

    @field_validator("command")
    @classmethod
    def _sanitize(cls, v: str) -> str:
        v = v.replace("\r", "").replace("\n", "").strip()
        if not v:
            raise ValueError("command must not be empty")
        return v


ClientConsoleMessage = Annotated[
    ConsoleHello | ConsoleCommandMessage, Field(discriminator="type")
]


class ConsoleLogMessage(_StrictModel):
    type: Literal["log"]
    line: LogLine


class ConsoleCaughtUp(_StrictModel):
    type: Literal["caught_up"]
    seq: int


class ConsoleResync(_StrictModel):
    type: Literal["resync"]
    from_seq: int
    to_seq: int


class ConsoleSnapshotMessage(_StrictModel):
    type: Literal["snapshot"]
    snapshot: InstanceSnapshot


class ConsoleCommandResult(_StrictModel):
    type: Literal["command_result"]
    accepted: bool
    error: str | None = None


class ConsoleError(_StrictModel):
    type: Literal["error"]
    error: str


ServerConsoleMessage = Annotated[
    (
        ConsoleLogMessage
        | ConsoleCaughtUp
        | ConsoleResync
        | ConsoleSnapshotMessage
        | ConsoleCommandResult
        | ConsoleError
    ),
    Field(discriminator="type"),
]


class EventsHelloOk(_StrictModel):
    type: Literal["hello_ok"]
    instances: list[InstanceSummary]


class StateChangeEvent(_StrictModel):
    type: Literal["state_change"]
    instance_id: str
    old_state: ProcessState
    new_state: ProcessState
    reason: str
    snapshot: InstanceSnapshot


ServerEventMessage = Annotated[
    EventsHelloOk | StateChangeEvent, Field(discriminator="type")
]

# Discriminated unions are validated via TypeAdapter (they are not BaseModels).
client_console_adapter = TypeAdapter(ClientConsoleMessage)
server_console_adapter = TypeAdapter(ServerConsoleMessage)
server_event_adapter = TypeAdapter(ServerEventMessage)
