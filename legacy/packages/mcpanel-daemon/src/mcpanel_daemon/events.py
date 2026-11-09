"""In-process publish/subscribe event bus.

Single asyncio loop only: no locks. Consumers get bounded queues; a slow
consumer drops its oldest events (it can detect gaps via ``LogLine.seq`` and
re-sync from the buffer).
"""

from __future__ import annotations

import asyncio
import contextlib
from dataclasses import dataclass

from mcpanel_common.api import LogLine

WILDCARD = "*"


@dataclass(frozen=True, slots=True)
class StateChange:
    instance_id: str
    old_state: str
    new_state: str
    reason: str


@dataclass(frozen=True, slots=True)
class LogLineEvent:
    instance_id: str
    line: LogLine


@dataclass(frozen=True, slots=True)
class RestartLoopEvent:
    instance_id: str
    crash_count: int


Event = StateChange | LogLineEvent | RestartLoopEvent


class EventBus:
    def __init__(self) -> None:
        self._topics: dict[str, set[asyncio.Queue[Event]]] = {}

    def subscribe(self, topic: str, maxsize: int = 256) -> asyncio.Queue[Event]:
        queue: asyncio.Queue[Event] = asyncio.Queue(maxsize=maxsize)
        self._topics.setdefault(topic, set()).add(queue)
        return queue

    def unsubscribe(self, topic: str, queue: asyncio.Queue[Event]) -> None:
        subscribers = self._topics.get(topic)
        if subscribers is None:
            return
        subscribers.discard(queue)
        if not subscribers:
            del self._topics[topic]

    def publish(self, topic: str, event: Event) -> None:
        targets = set(self._topics.get(topic, ()))
        if topic != WILDCARD:
            targets |= self._topics.get(WILDCARD, set())
        for queue in targets:
            self._deliver(queue, event)

    @staticmethod
    def _deliver(queue: asyncio.Queue[Event], event: Event) -> None:
        try:
            queue.put_nowait(event)
        except asyncio.QueueFull:
            # Drop the oldest event to make room; consumers detect gaps by seq.
            with contextlib.suppress(asyncio.QueueEmpty):
                queue.get_nowait()
            queue.put_nowait(event)
