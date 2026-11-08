"""In-memory log ring buffer with monotonic sequence numbers."""

from __future__ import annotations

from collections import deque
from datetime import UTC, datetime

from mcpanel_common.api import LogLine


class LogBuffer:
    def __init__(self, max_lines: int) -> None:
        self._lines: deque[LogLine] = deque(maxlen=max_lines)
        self._seq = 0

    def append(self, line: str, ts: datetime | None = None) -> LogLine:
        self._seq += 1
        entry = LogLine(
            seq=self._seq,
            ts=ts or datetime.now(UTC),
            line=line.rstrip("\r\n"),
        )
        self._lines.append(entry)
        return entry

    def tail(self, n: int) -> list[LogLine]:
        if n <= 0:
            return []
        return list(self._lines)[-n:]

    def since(self, seq: int) -> list[LogLine]:
        """All retained lines with ``line.seq > seq`` (oldest first)."""
        return [line for line in self._lines if line.seq > seq]

    @property
    def first_seq(self) -> int | None:
        return self._lines[0].seq if self._lines else None

    @property
    def last_seq(self) -> int | None:
        return self._lines[-1].seq if self._lines else None

    def __len__(self) -> int:
        return len(self._lines)
