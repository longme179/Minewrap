"""Daemon configuration schema (TOML) as pydantic models.

Relative paths in a config file resolve against the config file's directory
(the loader in the daemon applies that rule; these models just validate).
Extras are forbidden so typos fail loudly. Secrets use SecretStr and never
appear in repr/json.
"""

from __future__ import annotations

import re
from typing import Any, Literal, cast

from pydantic import BaseModel, ConfigDict, Field, SecretStr, field_validator, model_validator

INSTANCE_ID_RE = re.compile(r"^[a-z0-9](?:[a-z0-9_-]{0,62}[a-z0-9])?$")

DEFAULT_DONE_REGEX = r"Done \(\d+(\.\d+)?s\)! For help"


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


class InstanceDefaults(_StrictModel):
    """Per-instance defaults; each instance may override."""

    startup_timeout: float = Field(default=300.0, ge=0.1)
    stop_timeout: float = Field(default=60.0, ge=0.1)
    kill_grace_seconds: float = Field(default=5.0, ge=0.1)
    done_regex: str = DEFAULT_DONE_REGEX
    log_buffer_lines: int = Field(default=500, ge=10)
    log_max_bytes: int = Field(default=10 * 1024 * 1024, ge=1024)
    log_backups: int = Field(default=3, ge=0, le=100)


class DaemonSettings(_StrictModel):
    """Bind + data dir + global cadence, plus per-instance defaults."""

    host: str = "127.0.0.1"
    port: int = Field(default=8080, ge=1, le=65535)
    data_dir: str = "./data"
    token: str | None = None  # optional explicit token (reverse-proxy setups)
    status_poll_interval: float = Field(default=15.0, ge=1.0)
    unresponsive_after_failures: int = Field(default=2, ge=1)
    defaults: InstanceDefaults = Field(default_factory=InstanceDefaults)


class ProbeConfig(_StrictModel):
    host: str = "127.0.0.1"
    port: int = Field(default=25565, ge=1, le=65535)
    query_enabled: bool = False
    query_port: int | None = None
    timeout: float = Field(default=3.0, ge=0.1)

    @field_validator("query_port")
    @classmethod
    def _query_port_range(cls, v: int | None) -> int | None:
        if v is not None and not 1 <= v <= 65535:
            raise ValueError("query_port must be between 1 and 65535")
        return v


class RestartConfig(_StrictModel):
    policy: Literal["never", "on-crash"] = "on-crash"
    max_retries: int = Field(default=5, ge=0, le=100)
    backoff_base_seconds: float = Field(default=5.0, ge=0.1)
    backoff_max_seconds: float = Field(default=300.0, ge=1.0)
    reset_after_online_seconds: float = Field(default=600.0, ge=1.0)


class RconConfig(_StrictModel):
    """Optional RCON. Password comes from an env var named here, never from config."""

    enabled: bool = False
    host: str = "127.0.0.1"
    port: int = Field(default=25575, ge=1, le=65535)
    password_env: str = "MC_RCON_PASSWORD"


class DiscordConfig(_StrictModel):
    enabled: bool = False
    webhook_url: SecretStr | None = None
    mode: Literal["edit-in-place", "new-message"] = "edit-in-place"
    interval_seconds: float = Field(default=600.0, ge=10.0)
    first_push_delay_seconds: float = Field(default=10.0, ge=0.0)
    log_lines: int = Field(default=15, ge=0, le=50)
    max_log_chars: int = Field(default=1500, ge=0, le=4096)
    state_change_posts: bool = True
    state_change_debounce_seconds: float = Field(default=30.0, ge=1.0)
    redact_regexes: list[str] = Field(default_factory=list)

    @field_validator("redact_regexes")
    @classmethod
    def _compile_regexes(cls, v: list[str]) -> list[str]:
        for pattern in v:
            try:
                re.compile(pattern)
            except re.error as exc:
                raise ValueError(f"invalid redact regex {pattern!r}: {exc}") from exc
        return v


class InstanceConfig(_StrictModel):
    id: str
    name: str = ""
    work_dir: str
    java_path: str = "java"
    jvm_args: list[str] = Field(default_factory=list)
    launch_args: list[str] = Field(default_factory=lambda: ["-jar", "server.jar", "nogui"])
    autostart: bool = False
    probe: ProbeConfig = Field(default_factory=ProbeConfig)
    restart: RestartConfig = Field(default_factory=RestartConfig)
    rcon: RconConfig | None = None
    discord: DiscordConfig = Field(default_factory=DiscordConfig)

    @field_validator("id")
    @classmethod
    def _id_shape(cls, v: str) -> str:
        if not INSTANCE_ID_RE.fullmatch(v):
            raise ValueError(
                "instance id must be 1-64 chars of [a-z0-9_-], "
                "not starting or ending with '-' or '_'"
            )
        return v

    @model_validator(mode="before")
    @classmethod
    def _default_name(cls, data: object) -> object:
        if not isinstance(data, dict):
            return data
        raw = cast(dict[str, Any], data)
        if not raw.get("name"):
            return {**raw, "name": raw.get("id", "")}
        return raw


class DaemonConfig(_StrictModel):
    daemon: DaemonSettings = Field(default_factory=DaemonSettings)
    instances: list[InstanceConfig] = []

    @field_validator("instances")
    @classmethod
    def _unique_ids(cls, v: list[InstanceConfig]) -> list[InstanceConfig]:
        seen: set[str] = set()
        for inst in v:
            if inst.id in seen:
                raise ValueError(f"duplicate instance id: {inst.id!r}")
            seen.add(inst.id)
        return v
