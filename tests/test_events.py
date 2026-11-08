"""EventBus: topics, wildcard delivery, slow-consumer drop-oldest."""

from __future__ import annotations

import asyncio
from datetime import UTC, datetime

from mcpanel_common.api import LogLine
from mcpanel_daemon.events import (
    WILDCARD,
    Event,
    EventBus,
    LogLineEvent,
    StateChange,
)


def _state_change(instance_id: str = "a") -> StateChange:
    return StateChange(instance_id, "offline", "starting", "test")


def _log_event(seq: int) -> LogLineEvent:
    return LogLineEvent("a", LogLine(seq=seq, ts=datetime(2026, 11, 8, tzinfo=UTC), line="x"))


def _drain(queue: asyncio.Queue[Event]) -> list[Event]:
    events: list[Event] = []
    while not queue.empty():
        events.append(queue.get_nowait())
    return events


def test_topic_delivery() -> None:
    bus = EventBus()
    qa = bus.subscribe("a")
    qb = bus.subscribe("b")
    bus.publish("a", _state_change())
    assert len(_drain(qa)) == 1
    assert _drain(qb) == []


def test_wildcard_receives_all_topics() -> None:
    bus = EventBus()
    wildcard = bus.subscribe(WILDCARD)
    bus.publish("a", _state_change())
    bus.publish("b", _state_change("b"))
    assert len(_drain(wildcard)) == 2


def test_unsubscribe_stops_delivery() -> None:
    bus = EventBus()
    queue = bus.subscribe("a")
    bus.unsubscribe("a", queue)
    bus.publish("a", _state_change())
    assert queue.empty()


def test_unsubscribe_unknown_topic_is_noop() -> None:
    bus = EventBus()
    queue = bus.subscribe("a")
    bus.unsubscribe("nope", queue)  # must not raise
    bus.publish("a", _state_change())
    assert len(_drain(queue)) == 1


def test_slow_consumer_drops_oldest() -> None:
    bus = EventBus()
    queue = bus.subscribe("a", maxsize=2)
    bus.publish("a", _log_event(1))
    bus.publish("a", _log_event(2))
    bus.publish("a", _log_event(3))
    events = _drain(queue)
    assert [e.line.seq for e in events if isinstance(e, LogLineEvent)] == [2, 3]
