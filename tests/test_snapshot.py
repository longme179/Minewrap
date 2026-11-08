"""InstanceState: process/probe separation and combined-state derivation."""

from __future__ import annotations

from mcpanel_common.api import ProbeInfo
from mcpanel_daemon.process_manager import ProcessStatus, State
from mcpanel_daemon.snapshot import InstanceState


def make_status(state: State) -> ProcessStatus:
    return ProcessStatus(
        state=state,
        pid=123,
        started_at=None,
        stopped_at=None,
        exit_code=None,
        requested_stop=False,
        forced_stop=False,
        restart_count=0,
        crash_count=0,
        uptime_seconds=None,
    )


def test_initial_state_is_offline() -> None:
    state = InstanceState("survival", "Survival")
    assert state.combined_state == "offline"
    summary = state.summary()
    assert summary.id == "survival"
    assert summary.players_online is None
    assert summary.latency_ms is None


def test_online_requires_process_online_and_thresholded_probe_failures() -> None:
    state = InstanceState("survival", "Survival", unresponsive_after_failures=2)
    state.update_process(make_status(State.ONLINE))

    state.update_probe(ProbeInfo(ok=True, players_online=2, players_max=20, latency_ms=9.5))
    assert state.combined_state == "online"

    state.update_probe(ProbeInfo(ok=False, error="timeout"))
    assert state.combined_state == "online"  # tolerated transient failure
    assert state.snapshot().probe.ok is False

    state.update_probe(ProbeInfo(ok=False, error="timeout"))
    assert state.combined_state == "unresponsive"

    state.update_probe(ProbeInfo(ok=True, players_online=1))
    assert state.combined_state == "online"  # recovery resets the counter


def test_process_states_map_one_to_one_when_probe_is_not_relevant() -> None:
    state = InstanceState("survival", "Survival")
    for process_state, combined in (
        (State.STARTING, "starting"),
        (State.STOPPING, "stopping"),
        (State.CRASHED, "crashed"),
        (State.OFFLINE, "offline"),
    ):
        state.update_process(make_status(process_state))
        state.update_probe(ProbeInfo(ok=False, error="connection refused"))
        assert state.combined_state == combined


def test_probe_failure_never_reports_offline_while_process_runs() -> None:
    state = InstanceState("survival", "Survival", unresponsive_after_failures=1)
    state.update_process(make_status(State.ONLINE))
    state.update_probe(ProbeInfo(ok=False, error="connection refused"))
    assert state.combined_state == "unresponsive"


def test_snapshot_keeps_process_and_probe_separate() -> None:
    state = InstanceState("survival", "Survival", unresponsive_after_failures=1)
    state.update_process(make_status(State.ONLINE))
    state.update_probe(
        ProbeInfo(
            ok=False,
            error="timed out",
            players_online=5,
            players_max=10,
            version="1.21.4",
        )
    )
    snap = state.snapshot()
    assert snap.process.state == "online"
    assert snap.probe.ok is False
    assert snap.probe.error == "timed out"
    assert snap.combined_state == "unresponsive"
    assert snap.updated_at.tzinfo is not None


def test_summary_reflects_last_probe_values() -> None:
    state = InstanceState("survival", "Survival")
    state.update_process(make_status(State.ONLINE))
    state.update_probe(
        ProbeInfo(
            ok=True,
            players_online=3,
            players_max=20,
            version="1.21.4",
            latency_ms=11.0,
            player_names=["Alice", "Bob", "Carol"],
        )
    )
    summary = state.summary()
    assert summary.players_online == 3
    assert summary.players_max == 20
    assert summary.version == "1.21.4"
    assert summary.latency_ms == 11.0
