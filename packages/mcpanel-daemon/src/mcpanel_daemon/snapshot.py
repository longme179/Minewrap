"""Merged per-instance state: process status + probe reachability.

Process state (what the daemon manages) and reachability (what the probe
sees) are kept as separate facts; ``combined_state`` is derived for the API.
A probe failure while the process runs never reports ``offline``.
"""

from __future__ import annotations

from datetime import UTC, datetime

from mcpanel_common.api import (
    CombinedState,
    InstanceSnapshot,
    InstanceSummary,
    ProbeInfo,
    ProcessInfo,
)

from mcpanel_daemon.process_manager import ProcessStatus, State


class InstanceState:
    def __init__(
        self,
        instance_id: str,
        name: str,
        unresponsive_after_failures: int = 2,
    ) -> None:
        self.instance_id = instance_id
        self.name = name
        self._unresponsive_after_failures = unresponsive_after_failures
        self._process = ProcessStatus(
            state=State.OFFLINE,
            pid=None,
            started_at=None,
            stopped_at=None,
            exit_code=None,
            requested_stop=False,
            forced_stop=False,
            restart_count=0,
            crash_count=0,
            uptime_seconds=None,
        )
        self._probe = ProbeInfo(ok=False)
        self._consecutive_failures = 0

    def update_process(self, status: ProcessStatus) -> None:
        self._process = status

    def update_probe(self, info: ProbeInfo) -> None:
        self._probe = info
        if info.ok:
            self._consecutive_failures = 0
        else:
            self._consecutive_failures += 1

    @property
    def combined_state(self) -> CombinedState:
        state = self._process.state
        if state is State.ONLINE:
            if self._consecutive_failures >= self._unresponsive_after_failures:
                return "unresponsive"
            return "online"
        if state is State.OFFLINE:
            return "offline"
        if state is State.STARTING:
            return "starting"
        if state is State.STOPPING:
            return "stopping"
        return "crashed"

    def summary(self) -> InstanceSummary:
        return InstanceSummary(
            id=self.instance_id,
            name=self.name,
            combined_state=self.combined_state,
            players_online=self._probe.players_online,
            players_max=self._probe.players_max,
            version=self._probe.version,
            latency_ms=self._probe.latency_ms,
        )

    def snapshot(self) -> InstanceSnapshot:
        process = self._process
        return InstanceSnapshot(
            id=self.instance_id,
            name=self.name,
            combined_state=self.combined_state,
            process=ProcessInfo(
                state=process.state.value,
                pid=process.pid,
                started_at=process.started_at,
                stopped_at=process.stopped_at,
                uptime_seconds=process.uptime_seconds,
                exit_code=process.exit_code,
                requested_stop=process.requested_stop,
                restart_count=process.restart_count,
                crash_count=process.crash_count,
            ),
            probe=self._probe,
            updated_at=datetime.now(UTC),
        )
