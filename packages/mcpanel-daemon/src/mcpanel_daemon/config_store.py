"""Daemon config loading/saving and API-token management.

Relative paths in the TOML file resolve against the config file's directory.
Files written here (config, token) are mode 0600. Secrets are written to TOML
in plaintext by necessity but never logged.
"""

from __future__ import annotations

import logging
import os
import secrets
import tomllib
from pathlib import Path
from typing import Any, cast

import tomli_w
from mcpanel_common.config import DaemonConfig
from pydantic import BaseModel, SecretStr

logger = logging.getLogger(__name__)

FILE_MODE = 0o600
DIR_MODE = 0o700
TOKEN_FILE_NAME = "token"


class ConfigError(Exception):
    """Unreadable or invalid configuration."""


def load_config(path: Path) -> DaemonConfig:
    try:
        with path.open("rb") as handle:
            raw = tomllib.load(handle)
    except FileNotFoundError as exc:
        raise ConfigError(f"config file not found: {path}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"invalid TOML in {path}: {exc}") from exc
    try:
        config = DaemonConfig.model_validate(raw)
    except ValueError as exc:
        raise ConfigError(f"invalid config {path}: {exc}") from exc
    _resolve_paths(config, path.parent)
    return config


def _resolve_paths(config: DaemonConfig, base: Path) -> None:
    if not Path(config.daemon.data_dir).is_absolute():
        config.daemon.data_dir = str((base / config.daemon.data_dir).resolve())
    for instance in config.instances:
        if not Path(instance.work_dir).is_absolute():
            instance.work_dir = str((base / instance.work_dir).resolve())


def save_config(path: Path, config: DaemonConfig) -> None:
    data = _plain(config.model_dump(mode="python"))
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(tomli_w.dumps(data), encoding="utf-8")
    os.chmod(tmp, FILE_MODE)
    tmp.replace(path)
    os.chmod(path, FILE_MODE)


def _plain(value: Any) -> Any:
    """Convert pydantic dump output to TOML-serializable values (real secrets).

    ``None`` values are dropped: TOML has no null, and an absent key means the
    same thing to the models.
    """
    if isinstance(value, SecretStr):
        return value.get_secret_value()
    if isinstance(value, BaseModel):  # pragma: no cover - dump() already flattens
        return {key: _plain(item) for key, item in value.model_dump(mode="python").items()}
    if isinstance(value, dict):
        result: dict[str, Any] = {}
        for key, item in cast(dict[Any, Any], value).items():
            converted = _plain(item)
            if converted is not None:
                result[str(key)] = converted
        return result
    if isinstance(value, list):
        return [_plain(item) for item in cast(list[Any], value)]
    return value


def ensure_data_dir(config: DaemonConfig) -> Path:
    data_dir = Path(config.daemon.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True, mode=DIR_MODE)
    os.chmod(data_dir, DIR_MODE)
    return data_dir


def resolve_token(config: DaemonConfig) -> tuple[str, bool]:
    """Return (token, was_generated).

    An explicit ``daemon.token`` wins. Otherwise the token is read from (or
    created in, mode 0600) ``<data_dir>/token``; the caller prints it once.
    """
    if config.daemon.token:
        return config.daemon.token, False
    data_dir = ensure_data_dir(config)
    token_path = data_dir / TOKEN_FILE_NAME
    if token_path.exists():
        return token_path.read_text(encoding="utf-8").strip(), False
    token = secrets.token_urlsafe(32)
    token_path.write_text(token + "\n", encoding="utf-8")
    os.chmod(token_path, FILE_MODE)
    return token, True
