"""QtCore-only view models: testable without a display."""

# pyright: basic

from __future__ import annotations

from mcpanel_common.api import InstanceSnapshot, InstanceSummary, LogLine
from PySide6.QtCore import QObject, Signal

CONNECTION_CONNECTING = "connecting"
CONNECTION_CONNECTED = "connected"
CONNECTION_OFFLINE = "offline"
CONNECTION_UNAUTHORIZED = "unauthorized"
CONNECTION_INCOMPATIBLE = "incompatible"

STATE_UNKNOWN = "unknown"


class ConsoleModel(QObject):
    line_added = Signal(object)

    def __init__(self, max_lines: int = 5000, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._lines: list[LogLine] = []
        self._max_lines = max_lines
        self._history: list[str] = []
        self._history_index = 0

    @property
    def lines(self) -> list[LogLine]:
        return list(self._lines)

    def add_line(self, line: LogLine) -> None:
        if self._lines and line.seq <= self._lines[-1].seq:
            return  # duplicate after a backfill overlap
        self._lines.append(line)
        if len(self._lines) > self._max_lines:
            del self._lines[: len(self._lines) - self._max_lines]
        self.line_added.emit(line)

    def clear(self) -> None:
        self._lines.clear()

    def add_command(self, command: str) -> None:
        command = command.strip()
        if not command:
            return
        if not self._history or self._history[-1] != command:
            self._history.append(command)
        self._history_index = len(self._history)

    @property
    def history(self) -> list[str]:
        return list(self._history)

    def history_previous(self) -> str | None:
        if not self._history:
            return None
        if self._history_index > 0:
            self._history_index -= 1
        return self._history[self._history_index]

    def history_next(self) -> str | None:
        if not self._history:
            return None
        if self._history_index < len(self._history) - 1:
            self._history_index += 1
            return self._history[self._history_index]
        self._history_index = len(self._history)
        return ""


class InstanceViewModel(QObject):
    snapshot_changed = Signal(object)
    connection_changed = Signal(str)

    def __init__(self, instance_id: str, name: str, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self.instance_id = instance_id
        self.name = name
        self.snapshot: InstanceSnapshot | None = None
        self.summary: InstanceSummary | None = None
        self.connection = CONNECTION_CONNECTING

    @property
    def combined_state(self) -> str:
        if self.snapshot is not None:
            return self.snapshot.combined_state
        if self.summary is not None:
            return self.summary.combined_state
        return STATE_UNKNOWN

    def update_summary(self, summary: InstanceSummary) -> None:
        self.summary = summary
        if summary.name:
            self.name = summary.name

    def update_snapshot(self, snapshot: InstanceSnapshot) -> None:
        self.snapshot = snapshot
        self.name = snapshot.name
        self.snapshot_changed.emit(snapshot)

    def set_connection(self, state: str) -> None:
        if state != self.connection:
            self.connection = state
            self.connection_changed.emit(state)


class NodeViewModel(QObject):
    instances_changed = Signal(list)
    connection_changed = Signal(str)

    def __init__(
        self, node_id: str, name: str, url: str, parent: QObject | None = None
    ) -> None:
        super().__init__(parent)
        self.node_id = node_id
        self.name = name
        self.url = url
        self.connection = CONNECTION_CONNECTING
        self.instances: dict[str, InstanceViewModel] = {}

    def apply_summaries(self, summaries: list[InstanceSummary]) -> None:
        seen: set[str] = set()
        for summary in summaries:
            seen.add(summary.id)
            view_model = self.instances.get(summary.id)
            if view_model is None:
                view_model = InstanceViewModel(summary.id, summary.name, parent=self)
                self.instances[summary.id] = view_model
            view_model.update_summary(summary)
        for instance_id in list(self.instances):
            if instance_id not in seen:
                view_model = self.instances.pop(instance_id)
                view_model.deleteLater()
        self.instances_changed.emit(list(self.instances.values()))

    def get(self, instance_id: str) -> InstanceViewModel | None:
        return self.instances.get(instance_id)

    def set_connection(self, state: str) -> None:
        if state != self.connection:
            self.connection = state
            self.connection_changed.emit(state)
            for view_model in self.instances.values():
                view_model.set_connection(state)
