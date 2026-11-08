"""Config schema validation tests: every rule in mcpanel_common.config."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
from mcpanel_common.config import DaemonConfig, InstanceConfig
from pydantic import ValidationError


def _base_toml() -> dict[str, Any]:
    return {
        "daemon": {},
        "instances": [
            {
                "id": "survival",
                "work_dir": "/srv/minecraft/survival",
            }
        ],
    }


def _full_daemon_toml() -> dict[str, Any]:
    data = _base_toml()
    data["daemon"] = {
        "host": "0.0.0.0",
        "port": 9000,
        "token": "secret-token",
        "defaults": {"log_buffer_lines": 250, "stop_timeout": 30},
    }
    data["instances"][0].update(
        {
            "name": "Survival",
            "jvm_args": ["-Xms2G", "-Xmx4G"],
            "probe": {"port": 25566, "query_enabled": True},
            "restart": {"policy": "never"},
            "rcon": {"enabled": True, "password_env": "MC_RCON_PW"},
            "discord": {
                "enabled": True,
                "webhook_url": "https://discord.com/api/webhooks/123/abc",
                "redact_regexes": ["password=\\S+"],
            },
        }
    )
    return data


def test_minimal_config_uses_defaults() -> None:
    cfg = DaemonConfig.model_validate(_base_toml())
    assert cfg.daemon.host == "127.0.0.1"
    assert cfg.daemon.port == 8080
    assert cfg.daemon.defaults.log_buffer_lines == 500
    inst = cfg.instances[0]
    assert inst.name == "survival"  # defaults to id
    assert inst.launch_args == ["-jar", "server.jar", "nogui"]
    assert inst.restart.policy == "on-crash"
    assert inst.discord.enabled is False


def test_full_config_round_trip() -> None:
    cfg = DaemonConfig.model_validate(_full_daemon_toml())
    inst = cfg.instances[0]
    assert inst.probe.query_port is None  # defaults to probe port at use time
    assert inst.rcon is not None and inst.rcon.password_env == "MC_RCON_PW"
    assert inst.discord.webhook_url is not None
    assert inst.discord.webhook_url.get_secret_value() == (
        "https://discord.com/api/webhooks/123/abc"
    )


def test_example_config_file_is_valid() -> None:
    """config.example.toml must always load against the current models."""
    import tomllib

    root = Path(__file__).resolve().parent.parent
    raw = tomllib.loads((root / "config.example.toml").read_text())
    cfg = DaemonConfig.model_validate(raw)
    assert cfg.instances[0].id == "survival"


def test_webhook_never_appears_in_repr_or_json() -> None:
    inst = InstanceConfig.model_validate(
        {
            "id": "x",
            "work_dir": "/tmp/x",
            "discord": {"webhook_url": "https://discord.com/api/webhooks/TOP/SECRET"},
        }
    )
    assert "TOP" not in repr(inst)
    assert "SECRET" not in repr(inst)
    assert "SECRET" not in inst.model_dump_json()


@pytest.mark.parametrize(
    "bad_id",
    ["UPPER", "has space", "trailing-", "", ".hidden", "a" * 65],
)
def test_bad_instance_ids_rejected(bad_id: str) -> None:
    data = _base_toml()
    data["instances"][0]["id"] = bad_id
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)


def test_duplicate_instance_ids_rejected() -> None:
    data = _base_toml()
    data["instances"].append({"id": "survival", "work_dir": "/tmp/other"})
    with pytest.raises(ValidationError, match="duplicate instance id"):
        DaemonConfig.model_validate(data)


def test_unknown_keys_rejected_everywhere() -> None:
    data = _base_toml()
    data["daemon"]["typo_field"] = 1
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)

    data = _base_toml()
    data["instances"][0]["probe"] = {"port": 25565, "typo_field": 1}
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)


def test_out_of_range_ports_rejected() -> None:
    data = _base_toml()
    data["daemon"]["port"] = 70000
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)

    data = _base_toml()
    data["instances"][0]["probe"] = {"port": 0}
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)


def test_bad_restart_policy_rejected() -> None:
    data = _base_toml()
    data["instances"][0]["restart"] = {"policy": "always"}
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)


def test_bad_discord_mode_rejected() -> None:
    data = _base_toml()
    data["instances"][0]["discord"] = {"mode": "edit"}
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)


def test_bad_redact_regex_rejected() -> None:
    data = _base_toml()
    data["instances"][0]["discord"] = {"redact_regexes": ["(unclosed"]}
    with pytest.raises(ValidationError, match="invalid redact regex"):
        DaemonConfig.model_validate(data)


def test_interval_ranges_enforced() -> None:
    data = _base_toml()
    data["daemon"]["status_poll_interval"] = 0.5
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)

    data = _base_toml()
    data["instances"][0]["discord"] = {"interval_seconds": 5}
    with pytest.raises(ValidationError):
        DaemonConfig.model_validate(data)
