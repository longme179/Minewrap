"""REST DTO + WebSocket message model round-trip tests."""

from __future__ import annotations

import json
from datetime import UTC, datetime

import pytest
from mcpanel_common.api import (
    CommandRequest,
    ConsoleLogMessage,
    HealthResponse,
    InstanceSnapshot,
    LogLine,
    ProbeInfo,
    ProcessInfo,
    StateChangeEvent,
    client_console_adapter,
    server_console_adapter,
    server_event_adapter,
)
from pydantic import ValidationError


def _snapshot() -> InstanceSnapshot:
    return InstanceSnapshot(
        id="survival",
        name="Survival",
        combined_state="online",
        process=ProcessInfo(state="online", pid=1234, uptime_seconds=42.5),
        probe=ProbeInfo(
            ok=True,
            latency_ms=12.3,
            players_online=2,
            players_max=20,
            version="1.21.4",
            motd="A Minecraft Server",
            player_names=["Alice", "Bob"],
            checked_at=datetime(2026, 11, 8, tzinfo=UTC),
        ),
        updated_at=datetime(2026, 11, 8, tzinfo=UTC),
    )


def test_snapshot_json_round_trip() -> None:
    snap = _snapshot()
    raw = json.loads(snap.model_dump_json())
    assert raw["combined_state"] == "online"
    assert raw["probe"]["player_names"] == ["Alice", "Bob"]
    again = InstanceSnapshot.model_validate(raw)
    assert again == snap


def test_health_response() -> None:
    h = HealthResponse(status="ok", api_version="0.1", daemon_version="0.1.0")
    assert h.model_dump()["status"] == "ok"


def test_command_request_strips_newlines() -> None:
    assert CommandRequest(command="say hi\nstop\n").command == "say histop"


def test_command_request_rejects_empty() -> None:
    with pytest.raises(ValidationError):
        CommandRequest(command="\n")


def test_console_client_message_discriminates() -> None:
    hello = client_console_adapter.validate_python({"type": "hello", "since_seq": 42})
    assert hello.type == "hello"
    cmd = client_console_adapter.validate_python({"type": "command", "command": "list"})
    assert cmd.type == "command"
    with pytest.raises(ValidationError):
        client_console_adapter.validate_python({"type": "nope", "command": "x"})


def test_console_server_message_discriminates() -> None:
    line = LogLine(seq=7, ts=datetime(2026, 11, 8, tzinfo=UTC), line="hello world")
    msg = server_console_adapter.validate_python({"type": "log", "line": line.model_dump()})
    assert isinstance(msg, ConsoleLogMessage) and msg.line.seq == 7

    resync = server_console_adapter.validate_python(
        {"type": "resync", "from_seq": 10, "to_seq": 30}
    )
    assert resync.type == "resync"

    with pytest.raises(ValidationError):
        server_console_adapter.validate_python({"type": "log", "line": {"seq": "x"}})


def test_events_server_message_discriminates() -> None:
    snap = _snapshot()
    ev = server_event_adapter.validate_python(
        {
            "type": "state_change",
            "instance_id": "survival",
            "old_state": "starting",
            "new_state": "online",
            "reason": "done_line",
            "snapshot": snap.model_dump(),
        }
    )
    assert isinstance(ev, StateChangeEvent)
    assert ev.reason == "done_line"
    assert ev.snapshot.probe.player_names == ["Alice", "Bob"]


def test_unknown_fields_rejected_in_ws_messages() -> None:
    with pytest.raises(ValidationError):
        server_console_adapter.validate_python({"type": "log", "line": {}, "extra": 1})
