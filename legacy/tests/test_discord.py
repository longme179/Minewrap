"""Discord notifier: hygiene, embeds, edit-in-place, retries, state posts."""

from __future__ import annotations

import asyncio
import json
import logging
import re
import stat
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx
import pytest
from mcpanel_common.api import InstanceSnapshot, LogLine, ProbeInfo, ProcessInfo
from mcpanel_common.config import DiscordConfig
from mcpanel_daemon.discord import (
    DiscordNotifier,
    build_status_embed,
    humanize_uptime,
    mask_webhook_url,
)
from mcpanel_daemon.events import EventBus, StateChange
from mcpanel_daemon.redact import redact_line
from mock_discord import MockDiscord

# --------------------------------------------------------------------- helpers


def make_config(**overrides: Any) -> DiscordConfig:
    values: dict[str, Any] = {
        "enabled": True,
        "webhook_url": "http://127.0.0.1:1/api/webhooks/1/token",
        "interval_seconds": 600,
        "first_push_delay_seconds": 10,
        "log_lines": 15,
        "max_log_chars": 1500,
        "state_change_posts": True,
        "state_change_debounce_seconds": 30,
    }
    values.update(overrides)
    return DiscordConfig.model_validate(values)


def mock_config(mock: MockDiscord, **overrides: Any) -> DiscordConfig:
    return make_config(webhook_url=mock.webhook_url, **overrides)


def make_snapshot(state: str = "online") -> InstanceSnapshot:
    return InstanceSnapshot(
        id="test",
        name="Test Server",
        combined_state=state,  # type: ignore[arg-type]
        process=ProcessInfo(state="online", uptime_seconds=65.0),
        probe=ProbeInfo(
            ok=True,
            latency_ms=12.0,
            players_online=2,
            players_max=20,
            version="1.21.4",
            motd="A Minecraft Server",
            player_names=["Alice", "Bob"],
        ),
        updated_at=datetime(2026, 11, 8, tzinfo=UTC),
    )


def make_lines(*texts: str) -> list[LogLine]:
    return [
        LogLine(seq=index + 1, ts=datetime(2026, 11, 8, tzinfo=UTC), line=text)
        for index, text in enumerate(texts)
    ]


def make_notifier(
    tmp_path: Path,
    *,
    config: DiscordConfig | None = None,
    snapshot: InstanceSnapshot | None = None,
    lines: list[LogLine] | None = None,
    client: httpx.AsyncClient | None = None,
    max_attempts: int = 3,
    backoff_base: float = 1.0,
) -> DiscordNotifier:
    fixed_snapshot = snapshot or make_snapshot()
    fixed_lines = lines if lines is not None else make_lines("hello world")
    return DiscordNotifier(
        instance_id="test",
        config=config or make_config(),
        state_dir=tmp_path / "discord",
        snapshot_provider=lambda: fixed_snapshot,
        log_provider=lambda n: fixed_lines[-n:] if n else [],
        bus=EventBus(),
        client=client,
        max_attempts=max_attempts,
        backoff_base=backoff_base,
    )


# --------------------------------------------------------------------- redaction


def test_redact_strips_ansi_and_ips_including_login_form() -> None:
    line = (
        "\x1b[32mAlice[/192.168.50.7:54321] logged in\x1b[0m "
        "from 10.0.0.5 and 2001:db8:85a3::8a2e:370:7334 and ::1"
    )
    out = redact_line(line)
    assert "\x1b[" not in out
    assert "192.168.50.7" not in out
    assert "10.0.0.5" not in out
    assert "2001:db8" not in out
    assert "Alice[/redacted]" in out


def test_redact_applies_user_patterns() -> None:
    pattern = re.compile(r"password=\S+")
    assert redact_line("login password=hunter2 ok", [pattern]) == "login [redacted] ok"


def test_redact_truncates_long_lines() -> None:
    out = redact_line("x" * 500, max_chars=50)
    assert len(out) <= 50
    assert out.endswith("…")


def test_mask_webhook_url_hides_token() -> None:
    masked = mask_webhook_url("https://discord.com/api/webhooks/123/secret-token")
    assert masked == "https://discord.com/api/webhooks/***/***"
    assert "secret-token" not in masked
    assert mask_webhook_url("http://localhost:1/hook") == "***"


def test_humanize_uptime() -> None:
    assert humanize_uptime(None) == "—"
    assert humanize_uptime(5) == "5s"
    assert humanize_uptime(65) == "1m 05s"
    assert humanize_uptime(3665) == "1h 01m 05s"


# ------------------------------------------------------------------------ embeds


def test_status_embed_contents() -> None:
    lines = make_lines("line one", "Alice[/192.168.0.1:25565] joined")
    embed = build_status_embed(
        make_snapshot(), lines, redact_patterns=[], max_log_chars=1500
    )
    assert embed["title"] == "Test Server — online"
    assert embed["color"] == 0x57F287
    names = {field["name"] for field in embed["fields"]}
    assert {"Version", "Players", "Latency", "Uptime", "Last updated"} <= names
    players = next(field for field in embed["fields"] if field["name"] == "Players")
    assert players["value"].startswith("2/20")
    assert "Alice, Bob" in players["value"]
    assert "line one" in embed["description"]
    assert "192.168.0.1" not in embed["description"]
    assert "```" in embed["description"]


def test_status_embed_trims_oldest_lines_to_budget() -> None:
    lines = make_lines(*[f"line-{i:02d} " + "x" * 40 for i in range(10)])
    embed = build_status_embed(
        make_snapshot(), lines, redact_patterns=[], max_log_chars=120
    )
    description = embed["description"]
    assert len(description) <= 120 + len("```\n\n```")
    assert "line-09" in description
    assert "line-00" not in description


def test_status_embed_without_logs_has_no_description() -> None:
    embed = build_status_embed(make_snapshot(), [], redact_patterns=[], max_log_chars=100)
    assert "description" not in embed


def test_status_embed_escapes_code_fences() -> None:
    embed = build_status_embed(
        make_snapshot(),
        make_lines("evil ``` fence"),
        redact_patterns=[],
        max_log_chars=1500,
    )
    assert "'''" in embed["description"]
    assert embed["description"].count("```") == 2  # only our own wrapper


# ------------------------------------------------------------------ HTTP behave


async def test_edit_in_place_creates_then_edits(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(tmp_path, config=mock_config(mock), client=client)
            state_path = tmp_path / "discord" / "test.json"

            await notifier.push_status_once()
            assert mock.requests[0]["method"] == "POST"
            assert "wait=true" in mock.requests[0]["path"]
            assert state_path.exists()
            assert stat.S_IMODE(state_path.stat().st_mode) == 0o600
            saved = json.loads(state_path.read_text())
            assert saved["message_id"] == "msg-1"

            await notifier.push_status_once()
            assert mock.requests[1]["method"] == "PATCH"
            assert mock.requests[1]["path"].endswith("/messages/msg-1")
    finally:
        mock.close()


async def test_404_on_edit_recreates_message(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(tmp_path, config=mock_config(mock), client=client)
            await notifier.push_status_once()  # POST -> msg-1

            mock.queue_response(404, {"message": "Unknown Message"})
            mock.queue_response(200, {"id": "msg-2"})
            await notifier.push_status_once()  # PATCH -> 404 -> POST -> msg-2

            methods = [request["method"] for request in mock.requests]
            assert methods == ["POST", "PATCH", "POST"]
            saved = json.loads((tmp_path / "discord" / "test.json").read_text())
            assert saved["message_id"] == "msg-2"

            await notifier.push_status_once()  # PATCH -> msg-2
            assert mock.requests[-1]["path"].endswith("/messages/msg-2")
    finally:
        mock.close()


async def test_new_message_mode_always_posts(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(
                tmp_path, config=mock_config(mock, mode="new-message"), client=client
            )
            await notifier.push_status_once()
            await notifier.push_status_once()
            assert [request["method"] for request in mock.requests] == ["POST", "POST"]
            assert not (tmp_path / "discord" / "test.json").exists()
    finally:
        mock.close()


async def test_429_retry_after_is_honoured(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(tmp_path, config=mock_config(mock), client=client)
            mock.queue_response(429, {"retry_after": 0.01})
            await notifier.push_status_once()
            assert len(mock.requests) == 2
            assert mock.requests[-1]["method"] == "POST"
    finally:
        mock.close()


async def test_5xx_retries_then_gives_up_without_state(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(
                tmp_path,
                config=mock_config(mock),
                client=client,
                max_attempts=2,
                backoff_base=0.01,
            )
            mock.queue_response(500, {"message": "boom"})
            mock.queue_response(500, {"message": "boom"})
            await notifier.push_status_once()
            assert len(mock.requests) == 2
            assert not (tmp_path / "discord" / "test.json").exists()
    finally:
        mock.close()


async def test_webhook_url_never_logged(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(
                tmp_path,
                config=mock_config(mock),
                client=client,
                max_attempts=1,
                backoff_base=0.01,
            )
            mock.queue_response(500, {"message": "boom"})
            with caplog.at_level(logging.WARNING, logger="mcpanel_daemon.discord"):
                await notifier.push_status_once()
        assert "boom" in caplog.text or "HTTP 500" in caplog.text
        assert "test-token" not in caplog.text
        assert "/api/webhooks/***/***" in caplog.text
    finally:
        mock.close()


def test_redact_keeps_log_timestamps() -> None:
    assert redact_line("[01:48:51] [Server thread/INFO]: Done") == (
        "[01:48:51] [Server thread/INFO]: Done"
    )


async def test_send_test_disabled_returns_error(tmp_path: Path) -> None:
    async with httpx.AsyncClient() as client:
        notifier = make_notifier(tmp_path, config=make_config(enabled=False), client=client)
        response = await notifier.send_test()
        assert response.ok is False
        assert response.error is not None


async def test_send_test_posts_message(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(tmp_path, config=mock_config(mock), client=client)
            response = await notifier.send_test()
            assert response.ok is True
            assert mock.requests[-1]["method"] == "POST"
            assert mock.requests[-1]["payload"]["embeds"][0]["title"].startswith(
                "mcpanel test message"
            )
    finally:
        mock.close()


# --------------------------------------------------------------------- loop


async def test_periodic_loop_pushes_and_first_push_is_soon(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            notifier = make_notifier(
                tmp_path,
                client=client,
                config=make_config(
                    interval_seconds=0.1, first_push_delay_seconds=0.01,
                    webhook_url=mock.webhook_url,
                ),
            )
            await notifier.start()
            try:
                await wait_until(lambda: len(mock.requests) >= 2, timeout_s=3)
            finally:
                await notifier.stop()
            assert (tmp_path / "discord" / "test.json").exists()
    finally:
        mock.close()


async def test_state_change_posts_are_debounced(tmp_path: Path) -> None:
    mock = MockDiscord()
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            bus = EventBus()
            notifier = DiscordNotifier(
                instance_id="test",
                config=make_config(
                    webhook_url=mock.webhook_url,
                    first_push_delay_seconds=100,
                    interval_seconds=600,
                    state_change_debounce_seconds=0.2,
                ),
                state_dir=tmp_path / "discord",
                snapshot_provider=make_snapshot,
                log_provider=lambda n: [],
                bus=bus,
                client=client,
            )
            await notifier.start()
            try:
                bus.publish("test", StateChange("test", "offline", "starting", "start_requested"))
                bus.publish("test", StateChange("test", "starting", "online", "done_line"))
                await asyncio.sleep(0.4)
                assert len(mock.requests) == 2  # first immediate, second coalesced after debounce
            finally:
                await notifier.stop()
    finally:
        mock.close()


async def wait_until(predicate: Any, timeout_s: float = 3.0) -> None:
    async with asyncio.timeout(timeout_s):
        while not predicate():  # noqa: ASYNC110 - test polling helper
            await asyncio.sleep(0.01)
