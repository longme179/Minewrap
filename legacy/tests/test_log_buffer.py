"""LogBuffer: monotonic seq, tail/since, eviction."""

from __future__ import annotations

from datetime import UTC, datetime

from mcpanel_daemon.log_buffer import LogBuffer


def test_append_assigns_monotonic_sequence_and_timestamp() -> None:
    buf = LogBuffer(max_lines=10)
    fixed = datetime(2026, 11, 8, 12, 0, 0, tzinfo=UTC)
    first = buf.append("hello", ts=fixed)
    second = buf.append("world\n")
    assert first.seq == 1
    assert second.seq == 2
    assert first.ts == fixed
    assert second.line == "world"  # line endings stripped
    assert second.ts.tzinfo is not None


def test_tail_and_since() -> None:
    buf = LogBuffer(max_lines=10)
    for i in range(5):
        buf.append(f"line {i}")
    assert [line.seq for line in buf.tail(2)] == [4, 5]
    assert buf.tail(0) == []
    assert [line.line for line in buf.since(3)] == ["line 3", "line 4"]
    assert buf.since(5) == []


def test_eviction_keeps_sequence_numbers() -> None:
    buf = LogBuffer(max_lines=3)
    for i in range(6):
        buf.append(f"line {i}")
    assert len(buf) == 3
    assert buf.first_seq == 4
    assert buf.last_seq == 6
    assert [line.line for line in buf.since(0)] == ["line 3", "line 4", "line 5"]


def test_empty_buffer() -> None:
    buf = LogBuffer(max_lines=3)
    assert buf.first_seq is None
    assert buf.last_seq is None
    assert buf.tail(5) == []
    assert buf.since(0) == []
