"""GUI-side unit tests: node store, tokens, console model, view models."""

from __future__ import annotations

import json
import stat
from datetime import UTC, datetime
from pathlib import Path

import pytest
from mcpanel_common.api import InstanceSnapshot, InstanceSummary, LogLine, ProbeInfo, ProcessInfo
from mcpanel_gui.node_store import DEFAULT_LOCAL_URL, NodeStore, normalize_url
from mcpanel_gui.viewmodels import (
    CONNECTION_CONNECTED,
    ConsoleModel,
    InstanceViewModel,
    NodeViewModel,
)


def _line(seq: int) -> LogLine:
    return LogLine(seq=seq, ts=datetime(2026, 11, 8, tzinfo=UTC), line=f"line {seq}")


def _summary(instance_id: str, state: str) -> InstanceSummary:
    return InstanceSummary(
        id=instance_id,
        name=instance_id.title(),
        combined_state=state,  # type: ignore[arg-type]
    )


def test_normalize_url() -> None:
    assert normalize_url("127.0.0.1:8080") == "http://127.0.0.1:8080"
    assert normalize_url("https://example.com/") == "https://example.com"
    with pytest.raises(ValueError):
        normalize_url("   ")
    with pytest.raises(ValueError):
        normalize_url("ftp://example.com")


def test_node_store_persists_nodes_with_0600(tmp_path: Path) -> None:
    store = NodeStore(tmp_path, force_file_tokens=True)
    node = store.add("Local", DEFAULT_LOCAL_URL)
    assert node.url == "http://127.0.0.1:8080"

    path = tmp_path / "nodes.json"
    assert stat.S_IMODE(path.stat().st_mode) == 0o600

    reloaded = NodeStore(tmp_path, force_file_tokens=True)
    assert [n.id for n in reloaded.nodes] == [node.id]
    assert reloaded.nodes[0].name == "Local"

    reloaded.remove(node.id)
    assert NodeStore(tmp_path, force_file_tokens=True).nodes == []


def test_token_file_fallback(tmp_path: Path) -> None:
    store = NodeStore(tmp_path, force_file_tokens=True)
    node = store.add("Local", DEFAULT_LOCAL_URL)
    store.set_token(node.id, "s3cret")

    token_path = tmp_path / "tokens.json"
    assert stat.S_IMODE(token_path.stat().st_mode) == 0o600
    assert json.loads(token_path.read_text(encoding="utf-8"))[node.id] == "s3cret"
    assert store.get_token(node.id) == "s3cret"

    store.remove(node.id)
    assert store.get_token(node.id) is None


def test_console_model_dedupes_and_caps() -> None:
    model = ConsoleModel(max_lines=3)
    for seq in range(1, 6):
        model.add_line(_line(seq))
    assert [line.seq for line in model.lines] == [3, 4, 5]

    model.add_line(_line(3))  # duplicate after backfill overlap
    model.add_line(_line(6))
    assert [line.seq for line in model.lines] == [4, 5, 6]


def test_console_model_history() -> None:
    model = ConsoleModel()
    model.add_command("say hi")
    model.add_command("list")
    model.add_command("list")  # consecutive duplicates are collapsed
    assert model.history == ["say hi", "list"]

    assert model.history_previous() == "list"
    assert model.history_previous() == "say hi"
    assert model.history_previous() == "say hi"
    assert model.history_next() == "list"
    assert model.history_next() == ""
    assert model.history_next() == ""


def test_node_view_model_applies_summaries() -> None:
    view = NodeViewModel("n1", "Local", "http://127.0.0.1:8080")
    received: list[list[InstanceViewModel]] = []
    view.instances_changed.connect(received.append)

    view.apply_summaries([_summary("survival", "online"), _summary("creative", "offline")])
    assert set(view.instances) == {"survival", "creative"}
    survival = view.get("survival")
    assert survival is not None
    assert survival.combined_state == "online"

    view.apply_summaries([_summary("survival", "crashed")])
    assert set(view.instances) == {"survival"}
    survival = view.get("survival")
    assert survival is not None
    assert survival.combined_state == "crashed"
    assert received


def test_instance_view_model_snapshot_overrides_summary() -> None:
    view = InstanceViewModel("survival", "Survival")
    view.update_summary(_summary("survival", "offline"))
    assert view.combined_state == "offline"

    snapshot = InstanceSnapshot(
        id="survival",
        name="Survival",
        combined_state="online",
        process=ProcessInfo(state="online"),
        probe=ProbeInfo(ok=True),
        updated_at=datetime(2026, 11, 8, tzinfo=UTC),
    )
    view.update_snapshot(snapshot)
    assert view.combined_state == "online"


def test_node_view_model_connection_propagates_to_instances() -> None:
    view = NodeViewModel("n1", "Local", "http://127.0.0.1:8080")
    view.apply_summaries([_summary("survival", "online")])
    instance = view.get("survival")
    assert instance is not None

    view.set_connection(CONNECTION_CONNECTED)
    assert view.connection == CONNECTION_CONNECTED
    assert instance.connection == CONNECTION_CONNECTED
