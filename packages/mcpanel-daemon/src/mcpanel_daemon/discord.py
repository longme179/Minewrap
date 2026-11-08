"""Per-instance Discord webhook notifier.

Runs inside the daemon (keeps pushing when the GUI is closed).

- ``edit-in-place``: create with ``POST <webhook>?wait=true``, persist the
  message id in a state file, then ``PATCH <webhook>/messages/<id>``; a 404
  (message deleted) falls back to creating a new message.
- ``new-message``: a fresh message every interval.

Embed contents are built from the instance snapshot + the last N log lines,
after log hygiene (ANSI removal, IP/user-pattern redaction, truncation).
Discord limits are respected: embed description <= 4096, total embed <= 6000;
the log block is capped by ``max_log_chars`` (default 1500).

Failures never raise out of the notifier loop; every failure logs one line
with the webhook URL masked.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import re
from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, cast

import httpx
from mcpanel_common.api import DiscordTestResponse, InstanceSnapshot, LogLine
from mcpanel_common.config import DiscordConfig

from mcpanel_daemon.events import Event, EventBus, RestartLoopEvent, StateChange
from mcpanel_daemon.redact import compile_patterns, redact_line

logger = logging.getLogger(__name__)

STATE_COLORS: dict[str, int] = {
    "online": 0x57F287,
    "starting": 0xFEE75C,
    "stopping": 0xE67E22,
    "offline": 0x95A5A6,
    "crashed": 0xED4245,
    "unresponsive": 0x992D22,
}

DESCRIPTION_LIMIT = 4096
FIELD_VALUE_LIMIT = 1024
TITLE_LIMIT = 256
RETRY_AFTER_CAP = 60.0


class DiscordError(Exception):
    """Configuration problem (no webhook); never contains the URL."""


def humanize_uptime(seconds: float | None) -> str:
    if seconds is None:
        return "—"
    total = int(max(0, seconds))
    hours, remainder = divmod(total, 3600)
    minutes, secs = divmod(remainder, 60)
    if hours:
        return f"{hours}h {minutes:02d}m {secs:02d}s"
    if minutes:
        return f"{minutes}m {secs:02d}s"
    return f"{secs}s"


def mask_webhook_url(url: str) -> str:
    marker = "/api/webhooks/"
    index = url.find(marker)
    if index == -1:
        return "***"
    return url[: index + len(marker)] + "***/***"


def _log_block(
    lines: list[LogLine],
    patterns: list[re.Pattern[str]],
    max_chars: int,
) -> str:
    budget = max(0, min(max_chars, DESCRIPTION_LIMIT - len("```\n\n```")))
    cleaned = [redact_line(line.line, patterns).replace("```", "'''") for line in lines]
    while cleaned and len("\n".join(cleaned)) > budget:
        cleaned.pop(0)
    block = "\n".join(cleaned)
    if block and len(block) > budget:
        block = block[: max(0, budget - 1)] + "…"
    return block


def build_status_embed(
    snapshot: InstanceSnapshot,
    lines: list[LogLine],
    *,
    redact_patterns: list[re.Pattern[str]],
    max_log_chars: int,
) -> dict[str, Any]:
    probe = snapshot.probe
    fields: list[dict[str, Any]] = []

    if probe.version:
        fields.append(
            {"name": "Version", "value": probe.version[:FIELD_VALUE_LIMIT], "inline": True}
        )

    if probe.players_online is not None:
        players = str(probe.players_online)
        if probe.players_max is not None:
            players = f"{probe.players_online}/{probe.players_max}"
        if probe.player_names:
            players += "\n" + ", ".join(probe.player_names)
        fields.append(
            {"name": "Players", "value": players[:FIELD_VALUE_LIMIT], "inline": True}
        )

    if probe.latency_ms is not None:
        fields.append(
            {"name": "Latency", "value": f"{probe.latency_ms:.0f} ms", "inline": True}
        )

    fields.append(
        {
            "name": "Uptime",
            "value": humanize_uptime(snapshot.process.uptime_seconds),
            "inline": True,
        }
    )
    fields.append(
        {
            "name": "Last updated",
            "value": f"<t:{int(snapshot.updated_at.timestamp())}:R>",
            "inline": True,
        }
    )

    embed: dict[str, Any] = {
        "title": f"{snapshot.name} — {snapshot.combined_state}"[:TITLE_LIMIT],
        "color": STATE_COLORS.get(snapshot.combined_state, STATE_COLORS["offline"]),
        "fields": fields,
        "timestamp": snapshot.updated_at.astimezone(UTC).isoformat().replace("+00:00", "Z"),
        "footer": {"text": "mcpanel"},
    }
    block = _log_block(lines, redact_patterns, max_log_chars)
    if block:
        embed["description"] = f"```\n{block}\n```"
    return embed


def build_state_change_embed(
    snapshot: InstanceSnapshot,
    old_state: str,
    new_state: str,
    reason: str,
) -> dict[str, Any]:
    return {
        "title": f"{snapshot.name}: {old_state} → {new_state}"[:TITLE_LIMIT],
        "description": f"State changed: **{old_state} → {new_state}** ({reason})"[
            :DESCRIPTION_LIMIT
        ],
        "color": STATE_COLORS.get(new_state, STATE_COLORS["offline"]),
        "timestamp": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
    }


def build_restart_loop_embed(snapshot: InstanceSnapshot, crash_count: int) -> dict[str, Any]:
    return {
        "title": f"{snapshot.name}: restart loop"[:TITLE_LIMIT],
        "description": (
            f"{crash_count} consecutive crashes; auto-restart stopped. "
            "Check the console and the server logs."
        )[:DESCRIPTION_LIMIT],
        "color": STATE_COLORS["crashed"],
        "timestamp": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
    }


def _retry_after_seconds(response: httpx.Response) -> float:
    try:
        payload: Any = response.json()
    except ValueError:
        payload = None
    if isinstance(payload, dict):
        data = cast(dict[str, Any], payload)
        value = data.get("retry_after")
        if isinstance(value, int | float):
            return float(value)
    header = response.headers.get("retry-after")
    if header:
        try:
            return float(header)
        except ValueError:
            pass
    return 1.0


class DiscordNotifier:
    def __init__(
        self,
        instance_id: str,
        config: DiscordConfig,
        state_dir: Path,
        snapshot_provider: Callable[[], InstanceSnapshot],
        log_provider: Callable[[int], list[LogLine]],
        bus: EventBus,
        *,
        client: httpx.AsyncClient | None = None,
        timeout: float = 10.0,
        backoff_base: float = 1.0,
        max_attempts: int = 3,
    ) -> None:
        self._instance_id = instance_id
        self._config = config
        self._state_path = state_dir / f"{instance_id}.json"
        self._snapshot_provider = snapshot_provider
        self._log_provider = log_provider
        self._bus = bus
        self._client = client if client is not None else httpx.AsyncClient(timeout=timeout)
        self._owns_client = client is None
        self._backoff_base = backoff_base
        self._max_attempts = max_attempts
        self._patterns = compile_patterns(config.redact_regexes)
        self._message_id = self._load_message_id()

        self._queue: asyncio.Queue[Event] | None = None
        self._tasks: list[asyncio.Task[None]] = []
        self._debounce_task: asyncio.Task[None] | None = None
        self._pending_event: StateChange | RestartLoopEvent | None = None
        self._last_state_post = -1e9

    @property
    def enabled(self) -> bool:
        return self._config.enabled and self._config.webhook_url is not None

    @property
    def _masked_url(self) -> str:
        url = self._config.webhook_url
        if url is None:
            return "***"
        return mask_webhook_url(url.get_secret_value())

    async def start(self) -> None:
        if not self.enabled or self._tasks:
            return
        self._queue = self._bus.subscribe(self._instance_id)
        self._tasks = [
            asyncio.create_task(self._periodic(), name=f"discord-periodic-{self._instance_id}"),
            asyncio.create_task(self._handle_events(), name=f"discord-events-{self._instance_id}"),
        ]
        logger.info(
            "discord notifier started for %s (mode=%s)", self._instance_id, self._config.mode
        )

    async def stop(self) -> None:
        for task in self._tasks:
            task.cancel()
        await asyncio.gather(*self._tasks, return_exceptions=True)
        self._tasks = []
        if self._debounce_task is not None:
            self._debounce_task.cancel()
            await asyncio.gather(self._debounce_task, return_exceptions=True)
            self._debounce_task = None
        if self._queue is not None:
            self._bus.unsubscribe(self._instance_id, self._queue)
            self._queue = None
        if self._owns_client:
            await self._client.aclose()

    # -------------------------------------------------------------- operations

    async def push_status_once(self) -> None:
        webhook = self._webhook()
        snapshot = self._snapshot_provider()
        lines = self._log_provider(self._config.log_lines)
        embed = build_status_embed(
            snapshot,
            lines,
            redact_patterns=self._patterns,
            max_log_chars=self._config.max_log_chars,
        )
        if self._config.mode == "edit-in-place" and self._message_id:
            response = await self._request(
                "PATCH",
                f"{webhook}/messages/{self._message_id}",
                {"embeds": [embed]},
            )
            if response is not None and response.status_code == 404:
                logger.info(
                    "discord message for %s was deleted; creating a new one", self._instance_id
                )
                self._message_id = None
                await self._create_status_message(webhook, embed)
            return
        await self._create_status_message(webhook, embed)

    async def send_test(self) -> DiscordTestResponse:
        if not self.enabled:
            return DiscordTestResponse(
                ok=False, error="discord is disabled or no webhook is configured"
            )
        snapshot = self._snapshot_provider()
        embed: dict[str, Any] = {
            "title": f"mcpanel test message — {snapshot.name}"[:TITLE_LIMIT],
            "description": f"Instance `{self._instance_id}` can reach this webhook.",
            "color": STATE_COLORS["online"],
            "fields": [
                {"name": "State", "value": snapshot.combined_state, "inline": True}
            ],
            "timestamp": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
        }
        response = await self._request(
            "POST", f"{self._webhook()}?wait=true", {"embeds": [embed]}
        )
        if response is None or response.status_code >= 400:
            return DiscordTestResponse(
                ok=False, error="discord request failed (see daemon log)"
            )
        return DiscordTestResponse(ok=True)

    # ---------------------------------------------------------------- internals

    def _webhook(self) -> str:
        url = self._config.webhook_url
        if url is None:
            raise DiscordError("discord webhook is not configured")
        return url.get_secret_value().rstrip("/")

    async def _create_status_message(self, webhook: str, embed: dict[str, Any]) -> None:
        response = await self._request("POST", f"{webhook}?wait=true", {"embeds": [embed]})
        if response is None or response.status_code >= 400:
            return
        try:
            raw: Any = response.json()
        except ValueError:
            return
        if not isinstance(raw, dict):
            return
        payload = cast(dict[str, Any], raw)
        message_id = payload.get("id")
        if message_id is not None and self._config.mode == "edit-in-place":
            self._message_id = str(message_id)
            self._save_message_id()

    async def _periodic(self) -> None:
        await asyncio.sleep(self._config.first_push_delay_seconds)
        while True:
            try:
                await self.push_status_once()
            except asyncio.CancelledError:
                raise
            except Exception:
                logger.exception("discord status push crashed for %s", self._instance_id)
            await asyncio.sleep(self._config.interval_seconds)

    async def _handle_events(self) -> None:
        queue = self._queue
        if queue is None:  # pragma: no cover - start() guarantees it
            return
        while True:
            event = await queue.get()
            if not isinstance(event, StateChange | RestartLoopEvent):
                continue
            if not self._config.state_change_posts:
                continue
            self._pending_event = event
            loop = asyncio.get_running_loop()
            remaining = self._config.state_change_debounce_seconds - (
                loop.time() - self._last_state_post
            )
            if remaining <= 0:
                await self._post_pending()
            elif self._debounce_task is None or self._debounce_task.done():
                self._debounce_task = asyncio.create_task(self._delayed_post(remaining))

    async def _delayed_post(self, delay: float) -> None:
        await asyncio.sleep(delay)
        await self._post_pending()

    async def _post_pending(self) -> None:
        event = self._pending_event
        self._pending_event = None
        if event is None:
            return
        try:
            snapshot = self._snapshot_provider()
            if isinstance(event, StateChange):
                embed = build_state_change_embed(
                    snapshot, event.old_state, event.new_state, event.reason
                )
            else:
                embed = build_restart_loop_embed(snapshot, event.crash_count)
            await self._request("POST", self._webhook(), {"embeds": [embed]})
        except DiscordError:
            return
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("discord state-change post failed for %s", self._instance_id)
        self._last_state_post = asyncio.get_running_loop().time()

    async def _request(
        self,
        method: str,
        url: str,
        payload: dict[str, Any] | None = None,
    ) -> httpx.Response | None:
        for attempt in range(1, self._max_attempts + 1):
            try:
                response = await self._client.request(method, url, json=payload)
            except httpx.HTTPError as exc:
                logger.warning(
                    "discord request failed for %s (%s)", self._masked_url, type(exc).__name__
                )
                return None
            if response.status_code == 429:
                retry_after = min(_retry_after_seconds(response), RETRY_AFTER_CAP)
                logger.warning(
                    "discord rate limited for %s; retrying in %.1fs (%d/%d)",
                    self._masked_url,
                    retry_after,
                    attempt,
                    self._max_attempts,
                )
                await asyncio.sleep(retry_after)
                continue
            if response.status_code >= 500:
                delay = min(self._backoff_base * (2 ** (attempt - 1)), RETRY_AFTER_CAP)
                logger.warning(
                    "discord HTTP %d for %s; retrying in %.1fs (%d/%d)",
                    response.status_code,
                    self._masked_url,
                    delay,
                    attempt,
                    self._max_attempts,
                )
                await asyncio.sleep(delay)
                continue
            if response.status_code >= 400 and response.status_code != 404:
                logger.warning(
                    "discord rejected %s for %s: HTTP %d",
                    method,
                    self._masked_url,
                    response.status_code,
                )
            return response
        logger.warning(
            "discord push gave up for %s after %d attempts",
            self._masked_url,
            self._max_attempts,
        )
        return None

    def _load_message_id(self) -> str | None:
        try:
            raw: Any = json.loads(self._state_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None
        if not isinstance(raw, dict):
            return None
        payload = cast(dict[str, Any], raw)
        message_id = payload.get("message_id")
        return str(message_id) if message_id is not None else None

    def _save_message_id(self) -> None:
        self._state_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        tmp = self._state_path.with_name(self._state_path.name + ".tmp")
        tmp.write_text(json.dumps({"message_id": self._message_id}), encoding="utf-8")
        os.chmod(tmp, 0o600)
        tmp.replace(self._state_path)
