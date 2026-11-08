"""Status poller: periodic mcstatus probes mapped to ProbeInfo.

Probe errors are captured into ``ProbeInfo(ok=False, error=...)`` and never
raised out of the loop. mcstatus 14.x async API:
``JavaServer(host, port, timeout, query_port).async_status()/.async_query()``.
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Protocol

from mcpanel_common.api import ProbeInfo
from mcpanel_common.config import ProbeConfig
from mcstatus import JavaServer

logger = logging.getLogger(__name__)


class Prober(Protocol):
    async def probe(self) -> ProbeInfo: ...


class McstatusProber:
    def __init__(self, config: ProbeConfig) -> None:
        self._config = config

    async def probe(self) -> ProbeInfo:
        config = self._config
        server = JavaServer(
            host=config.host,
            port=config.port,
            timeout=config.timeout,
            query_port=config.query_port,
        )
        try:
            status = await server.async_status(tries=1)
        except Exception as exc:  # mcstatus raises OSError, TimeoutError, ValueError, ...
            return ProbeInfo(ok=False, error=f"{type(exc).__name__}: {exc}")

        player_names: list[str] = []
        if config.query_enabled:
            try:
                query = await server.async_query(tries=1)
                player_names = list(query.players.list)
            except Exception as exc:  # query is optional; keep the status data
                logger.debug(
                    "query probe failed for %s:%s: %s", config.host, config.port, exc
                )

        return ProbeInfo(
            ok=True,
            latency_ms=status.latency,
            players_online=status.players.online,
            players_max=status.players.max,
            version=status.version.name,
            motd=status.motd.to_plain(),
            player_names=player_names,
        )


class StatusPoller:
    def __init__(
        self,
        prober: Prober,
        interval: float,
        on_result: Callable[[ProbeInfo], None],
    ) -> None:
        self._prober = prober
        self._interval = interval
        self._on_result = on_result
        self._task: asyncio.Task[None] | None = None

    def start(self) -> None:
        if self._task is None or self._task.done():
            self._task = asyncio.create_task(self._run())

    async def stop(self) -> None:
        task = self._task
        self._task = None
        if task is not None and not task.done():
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)

    async def _run(self) -> None:
        loop = asyncio.get_running_loop()
        while True:
            started = loop.time()
            try:
                info = await self._prober.probe()
            except asyncio.CancelledError:
                raise
            except Exception as exc:  # belt and braces: a prober must never kill the loop
                info = ProbeInfo(ok=False, error=f"{type(exc).__name__}: {exc}")
            if not info.ok:
                logger.debug("probe failed for interval loop: %s", info.error)
            info = info.model_copy(update={"checked_at": datetime.now(UTC)})
            self._on_result(info)
            elapsed = loop.time() - started
            await asyncio.sleep(max(0.0, self._interval - elapsed))
