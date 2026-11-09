"""config_store: path resolution, 0600 writes, secret round-trip, token handling."""

from __future__ import annotations

import stat
import tomllib
from pathlib import Path

import pytest
from mcpanel_common.config import DaemonConfig
from mcpanel_daemon.config_store import (
    ConfigError,
    load_config,
    resolve_token,
    save_config,
)


def _write_minimal(path: Path) -> Path:
    path.write_text(
        """
[daemon]
data_dir = "./data"

[[instances]]
id = "survival"
work_dir = "server"
""",
        encoding="utf-8",
    )
    return path


def test_load_resolves_relative_paths_against_config_dir(tmp_path: Path) -> None:
    config_path = _write_minimal(tmp_path / "config.toml")
    config = load_config(config_path)
    assert config.daemon.data_dir == str(tmp_path / "data")
    assert config.instances[0].work_dir == str(tmp_path / "server")


def test_load_missing_file_raises(tmp_path: Path) -> None:
    with pytest.raises(ConfigError, match="not found"):
        load_config(tmp_path / "nope.toml")


def test_load_invalid_toml_raises(tmp_path: Path) -> None:
    path = tmp_path / "config.toml"
    path.write_text("this is not toml [[[", encoding="utf-8")
    with pytest.raises(ConfigError, match="invalid TOML"):
        load_config(path)


def test_load_invalid_schema_raises(tmp_path: Path) -> None:
    path = tmp_path / "config.toml"
    path.write_text("[daemon]\nport = 70000\n", encoding="utf-8")
    with pytest.raises(ConfigError, match="invalid config"):
        load_config(path)


def test_save_round_trip_keeps_secret_and_sets_0600(tmp_path: Path) -> None:
    config = DaemonConfig.model_validate(
        {
            "daemon": {"data_dir": str(tmp_path / "data")},
            "instances": [
                {
                    "id": "survival",
                    "work_dir": str(tmp_path / "server"),
                    "discord": {
                        "enabled": True,
                        "webhook_url": "https://discord.com/api/webhooks/1/secret",
                    },
                }
            ],
        }
    )
    path = tmp_path / "config.toml"
    save_config(path, config)

    mode = stat.S_IMODE(path.stat().st_mode)
    assert mode == 0o600

    raw = tomllib.loads(path.read_text(encoding="utf-8"))
    assert (
        raw["instances"][0]["discord"]["webhook_url"]
        == "https://discord.com/api/webhooks/1/secret"
    )
    assert "**********" not in path.read_text(encoding="utf-8")

    again = load_config(path)
    assert again.instances[0].discord.webhook_url is not None
    assert (
        again.instances[0].discord.webhook_url.get_secret_value()
        == "https://discord.com/api/webhooks/1/secret"
    )


def test_explicit_token_wins(tmp_path: Path) -> None:
    config = DaemonConfig.model_validate({"daemon": {"token": "pinned-token"}})
    token, generated = resolve_token(config)
    assert token == "pinned-token"
    assert generated is False


def test_generated_token_is_stable_and_0600(tmp_path: Path) -> None:
    config = DaemonConfig.model_validate({"daemon": {"data_dir": str(tmp_path / "data")}})
    token, generated = resolve_token(config)
    assert generated is True
    assert len(token) >= 32

    token_path = tmp_path / "data" / "token"
    assert token_path.exists()
    assert stat.S_IMODE(token_path.stat().st_mode) == 0o600

    second, generated_again = resolve_token(config)
    assert second == token
    assert generated_again is False
