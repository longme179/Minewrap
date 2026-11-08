"""Provision and start a local daemon from the GUI.

Qt-free on purpose (testable): builds/writes a schema-valid config.toml
(mode 0600), locates the daemon binary (AppImage/AppDir, dist/, PATH, or the
current Python), spawns it detached so it survives the GUI closing, waits for
/health, and resolves the API token.
"""

# pyright: basic

from __future__ import annotations

import contextlib
import os
import shutil
import signal
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import httpx
import tomli_w
from mcpanel_common.config import DaemonConfig

FILE_MODE = 0o600
DIR_MODE = 0o700


class LaunchError(Exception):
    """Local daemon bootstrap failed (message is safe to show in the UI)."""


@dataclass
class InstanceSpec:
    instance_id: str
    name: str
    work_dir: str
    java_path: str = "java"
    jvm_args: list[str] = field(default_factory=list)
    launch_args: list[str] = field(default_factory=list)
    autostart: bool = True
    probe_host: str = "127.0.0.1"
    probe_port: int = 25565
    query_enabled: bool = False
    discord_enabled: bool = False
    webhook_url: str | None = None
    discord_interval: float = 600.0
    discord_log_lines: int = 15


@dataclass
class LaunchResult:
    url: str
    token: str
    name: str
    config_path: Path
    data_dir: Path
    process: subprocess.Popen[bytes] | None


def connect_url(host: str, port: int) -> str:
    connect_host = "127.0.0.1" if host in ("", "0.0.0.0", "::", "[::]") else host
    return f"http://{connect_host}:{port}"


def build_config(
    *,
    host: str,
    port: int,
    data_dir: Path,
    token: str | None,
    instance: InstanceSpec,
) -> dict[str, Any]:
    daemon: dict[str, Any] = {
        "host": host,
        "port": port,
        "data_dir": str(data_dir),
    }
    if token:
        daemon["token"] = token

    instance_config: dict[str, Any] = {
        "id": instance.instance_id,
        "name": instance.name,
        "work_dir": instance.work_dir,
        "java_path": instance.java_path,
        "jvm_args": instance.jvm_args,
        "launch_args": instance.launch_args,
        "autostart": instance.autostart,
        "probe": {
            "host": instance.probe_host,
            "port": instance.probe_port,
            "query_enabled": instance.query_enabled,
        },
    }
    if instance.discord_enabled and instance.webhook_url:
        instance_config["discord"] = {
            "enabled": True,
            "webhook_url": instance.webhook_url,
            "interval_seconds": instance.discord_interval,
            "log_lines": instance.discord_log_lines,
        }
    return {"daemon": daemon, "instances": [instance_config]}


def write_config(path: Path, config: dict[str, Any], *, overwrite: bool = False) -> None:
    # Validate against the shared schema before anything touches disk.
    DaemonConfig.model_validate(config)
    if path.exists() and not overwrite:
        raise LaunchError(f"config already exists: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(tomli_w.dumps(config), encoding="utf-8")
    os.chmod(tmp, FILE_MODE)
    tmp.replace(path)
    os.chmod(path, FILE_MODE)


def _appimage_reexec(resolved_exe: Path) -> list[str] | None:
    """Re-exec our own AppImage with --daemon.

    ``APPIMAGE`` alone is not proof: child processes inherit it from whatever
    AppImage launched them (e.g. an editor). Only trust it when our own
    executable lives inside the AppImage runtime's ``APPDIR``.
    """
    appimage = os.environ.get("APPIMAGE")
    appdir = os.environ.get("APPDIR")
    if not appimage or not appdir:
        return None
    try:
        inside = resolved_exe.is_relative_to(Path(appdir).resolve())
    except OSError:
        inside = False
    if inside and os.access(appimage, os.X_OK):
        return [appimage, "--daemon"]
    return None


def find_daemon_command(config_path: Path, *, exe: Path | None = None) -> list[str]:
    """Locate the daemon binary for the current packaging layout."""
    executable = Path(exe) if exe is not None else Path(sys.executable)
    resolved = executable.resolve()

    candidates = [
        # AppImage / AppDir layout: usr/lib/mcpanel-gui/mcpanel-gui -> usr/bin/mcpanel-daemon
        resolved.parent.parent.parent / "bin" / "mcpanel-daemon",
        # dist/ layout: dist/mcpanel-gui/mcpanel-gui -> dist/mcpanel-daemon
        resolved.parent.parent / "mcpanel-daemon",
    ]
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return [str(candidate)]

    found = shutil.which("mcpanel-daemon")
    if found:
        return [found]

    reexec = _appimage_reexec(resolved)
    if reexec is not None:
        return reexec

    if getattr(sys, "frozen", False):
        raise LaunchError(
            "no mcpanel-daemon binary found next to the GUI; use the full "
            "AppImage/dist build or install the daemon on PATH"
        )
    return [sys.executable, "-m", "mcpanel_daemon"]


def _healthy(url: str) -> bool:
    try:
        response = httpx.get(f"{url}/health", timeout=1.0)
    except httpx.HTTPError:
        return False
    return response.status_code == 200


def _read_token(data_dir: Path, timeout: float = 5.0) -> str:
    token_path = data_dir / "token"
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            token = token_path.read_text(encoding="utf-8").strip()
        except OSError:
            token = ""
        if token:
            return token
        time.sleep(0.05)
    raise LaunchError(
        f"daemon started but no token was found at {token_path}; "
        "enter a token explicitly or check the daemon log"
    )


def _verify_token(url: str, token: str) -> None:
    try:
        response = httpx.get(
            f"{url}/api/v1/instances",
            headers={"Authorization": f"Bearer {token}"},
            timeout=5.0,
        )
    except httpx.HTTPError as exc:
        raise LaunchError(f"cannot reach daemon at {url}: {type(exc).__name__}") from exc
    if response.status_code == 401:
        raise LaunchError(
            "a daemon is running on that port but rejected the token; "
            "paste the existing token or stop it first"
        )
    if response.status_code >= 400:
        raise LaunchError(f"daemon responded with HTTP {response.status_code}")


def _config_matches(path: Path, config: dict[str, Any]) -> bool:
    try:
        existing = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return False
    return existing == config


def provision_local_daemon(
    *,
    config_path: Path,
    host: str,
    port: int,
    data_dir: Path,
    instance: InstanceSpec,
    token: str | None = None,
    overwrite: bool = False,
    startup_timeout: float = 20.0,
    exe: Path | None = None,
) -> LaunchResult:
    """Write the config, start the daemon if needed, and return URL + token."""
    config_path = config_path.expanduser().resolve()
    data_dir = data_dir.expanduser().resolve()

    config = build_config(
        host=host, port=port, data_dir=data_dir, token=token, instance=instance
    )

    data_dir.mkdir(parents=True, exist_ok=True)
    os.chmod(data_dir, DIR_MODE)

    url = connect_url(host, port)
    resolved_token = token
    process: subprocess.Popen[bytes] | None = None
    log_path = data_dir / "daemon.log"

    running = _healthy(url)
    if running and not _config_matches(config_path, config):
        raise LaunchError(
            f"a daemon is already running at {url} with a different configuration; "
            "stop it first, or edit the instance in the GUI Settings tab"
        )
    if not running:
        write_config(config_path, config, overwrite=overwrite)
        command = [*find_daemon_command(config_path, exe=exe), "--config", str(config_path)]
        log_file = log_path.open("ab")
        try:
            process = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=log_file,
                stderr=subprocess.STDOUT,
                start_new_session=True,
                cwd=str(config_path.parent),
                close_fds=True,
            )
        finally:
            log_file.close()
        with contextlib.suppress(OSError):
            os.chmod(log_path, FILE_MODE)

        deadline = time.monotonic() + startup_timeout
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise LaunchError(
                    f"daemon exited during startup (code {process.returncode}); "
                    f"see {log_path}"
                )
            if _healthy(url):
                break
            time.sleep(0.1)
        else:
            raise LaunchError(f"daemon did not become healthy in time; see {log_path}")

    if resolved_token is None:
        resolved_token = _read_token(data_dir)
    _verify_token(url, resolved_token)
    return LaunchResult(
        url=url,
        token=resolved_token,
        name=instance.name,
        config_path=config_path,
        data_dir=data_dir,
        process=process,
    )


def stop_daemon(process: subprocess.Popen[bytes]) -> None:
    """Stop a daemon spawned by provision_local_daemon (used by tests/tools)."""
    if process.poll() is not None:
        return
    try:
        os.killpg(os.getpgid(process.pid), signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        with contextlib.suppress(OSError):
            os.killpg(os.getpgid(process.pid), signal.SIGKILL)
        process.wait(timeout=5)
