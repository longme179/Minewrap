"""Local daemon provisioning: config writing, command discovery, real spawn."""

from __future__ import annotations

import socket
import stat
import sys
import time
from pathlib import Path

import pytest
from mcpanel_common.config import DaemonConfig
from mcpanel_gui.apiclient import NodeApi
from mcpanel_gui.daemon_launcher import (
    InstanceSpec,
    LaunchError,
    build_config,
    connect_url,
    find_daemon_command,
    provision_local_daemon,
    stop_daemon,
    write_config,
)

FAKE_SERVER = Path(__file__).resolve().parent / "fake_mc_server.py"


def _which_path(_name: str) -> str:
    return "/usr/local/bin/mcpanel-daemon"


def _which_none(_name: str) -> None:
    return None


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _spec(tmp_path: Path, **overrides: object) -> InstanceSpec:
    values: dict[str, object] = {
        "instance_id": "wizard",
        "name": "Wizard",
        "work_dir": str(tmp_path),
        "java_path": sys.executable,
        "jvm_args": ["-Xmx1G"],
        "launch_args": [str(FAKE_SERVER)],
        "autostart": True,
        "probe_port": _free_port(),
    }
    values.update(overrides)
    return InstanceSpec(**values)  # type: ignore[arg-type]


def test_build_config_is_schema_valid(tmp_path: Path) -> None:
    spec = _spec(
        tmp_path,
        discord_enabled=True,
        webhook_url="https://discord.com/api/webhooks/1/token",
        discord_interval=300,
        discord_log_lines=20,
    )
    config = build_config(
        host="127.0.0.1",
        port=8080,
        data_dir=tmp_path / "data",
        token="secret-token",
        instance=spec,
    )
    parsed = DaemonConfig.model_validate(config)
    assert parsed.daemon.token == "secret-token"
    assert parsed.daemon.data_dir == str(tmp_path / "data")
    instance = parsed.instances[0]
    assert instance.id == "wizard"
    assert instance.launch_args == [str(FAKE_SERVER)]
    assert instance.jvm_args == ["-Xmx1G"]
    assert instance.discord.enabled is True
    assert instance.discord.interval_seconds == 300
    assert instance.discord.log_lines == 20
    assert instance.discord.webhook_url is not None


def test_write_config_refuses_overwrite_and_sets_0600(tmp_path: Path) -> None:
    config = build_config(
        host="127.0.0.1",
        port=8080,
        data_dir=tmp_path / "data",
        token=None,
        instance=_spec(tmp_path),
    )
    path = tmp_path / "config.toml"
    write_config(path, config)
    assert stat.S_IMODE(path.stat().st_mode) == 0o600

    with pytest.raises(LaunchError, match="already exists"):
        write_config(path, config)

    write_config(path, config, overwrite=True)


def test_connect_url_maps_wildcard_bind_to_loopback() -> None:
    assert connect_url("0.0.0.0", 8080) == "http://127.0.0.1:8080"
    assert connect_url("::", 8080) == "http://127.0.0.1:8080"
    assert connect_url("127.0.0.1", 9000) == "http://127.0.0.1:9000"


def test_find_daemon_command_prefers_appdir_layout(tmp_path: Path) -> None:
    exe = tmp_path / "app" / "usr" / "lib" / "mcpanel-gui" / "mcpanel-gui"
    exe.parent.mkdir(parents=True)
    exe.write_text("")
    daemon = tmp_path / "app" / "usr" / "bin" / "mcpanel-daemon"
    daemon.parent.mkdir(parents=True)
    daemon.write_text("")
    daemon.chmod(0o755)
    assert find_daemon_command(tmp_path / "config.toml", exe=exe) == [str(daemon)]


def test_find_daemon_command_dist_layout(tmp_path: Path) -> None:
    exe = tmp_path / "dist" / "mcpanel-gui" / "mcpanel-gui"
    exe.parent.mkdir(parents=True)
    exe.write_text("")
    daemon = tmp_path / "dist" / "mcpanel-daemon"
    daemon.write_text("")
    daemon.chmod(0o755)
    assert find_daemon_command(tmp_path / "config.toml", exe=exe) == [str(daemon)]


def test_find_daemon_command_path_fallback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("APPIMAGE", raising=False)
    monkeypatch.setattr(
        "mcpanel_gui.daemon_launcher.shutil.which",
        _which_path,
    )
    exe = tmp_path / "nowhere" / "gui"
    assert find_daemon_command(tmp_path / "config.toml", exe=exe) == [
        "/usr/local/bin/mcpanel-daemon"
    ]


def test_find_daemon_command_appimage_reexec(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    appdir = tmp_path / "mount"
    exe = appdir / "usr" / "lib" / "mcpanel-gui" / "mcpanel-gui"
    exe.parent.mkdir(parents=True)
    exe.write_text("")
    appimage = tmp_path / "mcpanel.AppImage"
    appimage.write_text("")
    appimage.chmod(0o755)
    monkeypatch.setenv("APPIMAGE", str(appimage))
    monkeypatch.setenv("APPDIR", str(appdir))
    monkeypatch.setattr("mcpanel_gui.daemon_launcher.shutil.which", _which_none)
    assert find_daemon_command(tmp_path / "config.toml", exe=exe) == [
        str(appimage),
        "--daemon",
    ]


def test_find_daemon_command_ignores_foreign_appimage_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A child process can inherit APPIMAGE from an unrelated AppImage; it must
    # not be used when our executable is not inside that AppImage's APPDIR.
    other = tmp_path / "CommandCode.AppImage"
    other.write_text("")
    other.chmod(0o755)
    monkeypatch.setenv("APPIMAGE", str(other))
    monkeypatch.setenv("APPDIR", str(tmp_path / "other-mount"))
    monkeypatch.setattr("mcpanel_gui.daemon_launcher.shutil.which", _which_none)
    exe = tmp_path / "venv" / "bin" / "python"
    command = find_daemon_command(tmp_path / "config.toml", exe=exe)
    assert command[0] == sys.executable
    assert command[-2:] == ["-m", "mcpanel_daemon"]


def test_find_daemon_command_python_module_fallback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("APPIMAGE", raising=False)
    monkeypatch.delenv("APPDIR", raising=False)
    monkeypatch.setattr("mcpanel_gui.daemon_launcher.shutil.which", _which_none)
    exe = tmp_path / "nowhere" / "gui"
    command = find_daemon_command(tmp_path / "config.toml", exe=exe)
    assert command[0] == sys.executable
    assert command[-2:] == ["-m", "mcpanel_daemon"]


def test_find_daemon_command_frozen_without_binary_raises(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("APPIMAGE", raising=False)
    monkeypatch.delenv("APPDIR", raising=False)
    monkeypatch.setattr("mcpanel_gui.daemon_launcher.shutil.which", _which_none)
    monkeypatch.setattr("mcpanel_gui.daemon_launcher.sys.frozen", True, raising=False)
    exe = tmp_path / "nowhere" / "gui"
    with pytest.raises(LaunchError, match="no mcpanel-daemon binary"):
        find_daemon_command(tmp_path / "config.toml", exe=exe)


# --------------------------------------------------------- real daemon spawn


def test_provision_starts_daemon_with_explicit_token(tmp_path: Path) -> None:
    port = _free_port()
    data_dir = tmp_path / "data"
    result = provision_local_daemon(
        config_path=tmp_path / "config.toml",
        host="127.0.0.1",
        port=port,
        data_dir=data_dir,
        instance=_spec(tmp_path),
        token="wizard-token",
    )
    assert result.process is not None
    api = NodeApi(result.url, result.token)
    try:
        summaries = api.list_instances()
        assert [summary.id for summary in summaries] == ["wizard"]

        deadline = time.monotonic() + 10
        state = ""
        while time.monotonic() < deadline:
            state = api.get_snapshot("wizard").process.state
            if state == "online":
                break
            time.sleep(0.02)
        assert state == "online"
        assert not (data_dir / "token").exists()  # explicit token, nothing generated
        assert (data_dir / "daemon.log").exists()
    finally:
        api.close()
        stop_daemon(result.process)


def test_provision_reads_generated_token(tmp_path: Path) -> None:
    port = _free_port()
    data_dir = tmp_path / "data"
    result = provision_local_daemon(
        config_path=tmp_path / "config.toml",
        host="127.0.0.1",
        port=port,
        data_dir=data_dir,
        instance=_spec(tmp_path),
        token=None,
    )
    assert result.process is not None
    try:
        token_path = data_dir / "token"
        assert token_path.exists()
        assert stat.S_IMODE(token_path.stat().st_mode) == 0o600
        assert result.token == token_path.read_text(encoding="utf-8").strip()

        api = NodeApi(result.url, result.token)
        try:
            assert api.list_instances()[0].id == "wizard"
        finally:
            api.close()
    finally:
        stop_daemon(result.process)
