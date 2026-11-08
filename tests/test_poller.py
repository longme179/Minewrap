"""StatusPoller loop behaviour + McstatusProber response mapping."""

from __future__ import annotations

import asyncio
from typing import Any, ClassVar

import pytest
from mcpanel_common.api import ProbeInfo
from mcpanel_common.config import ProbeConfig
from mcpanel_daemon import poller as poller_module
from mcpanel_daemon.poller import McstatusProber, StatusPoller


class FakeProber:
    def __init__(self, results: list[ProbeInfo | Exception]) -> None:
        self._results = list(results)
        self.calls = 0

    async def probe(self) -> ProbeInfo:
        self.calls += 1
        result = self._results.pop(0) if self._results else ProbeInfo(ok=True)
        if isinstance(result, Exception):
            raise result
        return result


def _config(**overrides: Any) -> ProbeConfig:
    values: dict[str, Any] = {"host": "mc.example.org", "port": 25566, "timeout": 2.0}
    values.update(overrides)
    return ProbeConfig.model_validate(values)


async def test_poller_delivers_results_and_stamps_checked_at() -> None:
    prober = FakeProber([ProbeInfo(ok=True, players_online=1), ProbeInfo(ok=False)])
    collected: list[ProbeInfo] = []
    done = asyncio.Event()

    def on_result(info: ProbeInfo) -> None:
        collected.append(info)
        if len(collected) >= 2:
            done.set()

    poller = StatusPoller(prober, interval=0.01, on_result=on_result)
    poller.start()
    try:
        await asyncio.wait_for(done.wait(), timeout=2)
    finally:
        await poller.stop()

    assert len(collected) >= 2
    assert collected[0].players_online == 1
    assert all(info.checked_at is not None for info in collected)


async def test_prober_exception_is_captured_and_loop_continues() -> None:
    prober = FakeProber([OSError("connection refused"), ProbeInfo(ok=True)])
    collected: list[ProbeInfo] = []
    done = asyncio.Event()

    def on_result(info: ProbeInfo) -> None:
        collected.append(info)
        if len(collected) >= 2:
            done.set()

    poller = StatusPoller(prober, interval=0.01, on_result=on_result)
    poller.start()
    try:
        await asyncio.wait_for(done.wait(), timeout=2)
    finally:
        await poller.stop()

    assert collected[0].ok is False
    assert collected[0].error is not None and "OSError" in collected[0].error
    assert collected[1].ok is True


async def test_stop_cancels_the_loop() -> None:
    prober = FakeProber([])
    poller = StatusPoller(prober, interval=0.01, on_result=lambda info: None)
    poller.start()
    await asyncio.sleep(0.03)
    await poller.stop()
    calls_after_stop = prober.calls
    await asyncio.sleep(0.05)
    assert prober.calls == calls_after_stop


# ------------------------------------------------ McstatusProber mapping tests


class FakeMotd:
    def to_plain(self) -> str:
        return "A Minecraft Server"


class FakePlayers:
    online = 3
    max = 20


class FakeVersion:
    name = "1.21.4"


class FakeStatus:
    latency = 12.5
    players = FakePlayers()
    version = FakeVersion()
    motd = FakeMotd()


class FakeQuery:
    class players:
        list: ClassVar[list[str]] = ["Alice", "Bob"]


class OkServer:
    last_kwargs: ClassVar[dict[str, Any]] = {}

    def __init__(self, **kwargs: Any) -> None:
        OkServer.last_kwargs = kwargs

    async def async_status(self, *, tries: int = 3) -> FakeStatus:
        return FakeStatus()

    async def async_query(self, *, tries: int = 3) -> FakeQuery:
        return FakeQuery()


class FailingServer:
    def __init__(self, **kwargs: Any) -> None:
        pass

    async def async_status(self, *, tries: int = 3) -> FakeStatus:
        raise OSError("connection refused")


class QueryFailingServer(OkServer):
    async def async_query(self, *, tries: int = 3) -> FakeQuery:
        raise ValueError("query disabled")


async def test_mcstatus_prober_maps_status_and_query(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(poller_module, "JavaServer", OkServer)
    prober = McstatusProber(
        _config(query_enabled=True, query_port=25567)
    )
    info = await prober.probe()

    assert OkServer.last_kwargs == {
        "host": "mc.example.org",
        "port": 25566,
        "timeout": 2.0,
        "query_port": 25567,
    }
    assert info.ok is True
    assert info.latency_ms == 12.5
    assert info.players_online == 3
    assert info.players_max == 20
    assert info.version == "1.21.4"
    assert info.motd == "A Minecraft Server"
    assert info.player_names == ["Alice", "Bob"]


async def test_mcstatus_prober_reports_status_errors(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(poller_module, "JavaServer", FailingServer)
    info = await McstatusProber(_config()).probe()
    assert info.ok is False
    assert info.error is not None and "OSError" in info.error


async def test_query_failure_keeps_status_data(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(poller_module, "JavaServer", QueryFailingServer)
    info = await McstatusProber(_config(query_enabled=True)).probe()
    assert info.ok is True
    assert info.players_online == 3
    assert info.player_names == []
