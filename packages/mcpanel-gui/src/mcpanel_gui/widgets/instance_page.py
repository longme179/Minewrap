"""Per-instance page: status panel, actions, console, settings."""

# pyright: basic

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from mcpanel_common.api import InstanceSnapshot
from PySide6.QtGui import QHideEvent, QShowEvent
from PySide6.QtWidgets import (
    QHBoxLayout,
    QLabel,
    QPushButton,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from mcpanel_gui.apiclient import NodeApi
from mcpanel_gui.session import ConsoleStream
from mcpanel_gui.viewmodels import CONNECTION_CONNECTED
from mcpanel_gui.widgets.console_widget import ConsoleWidget
from mcpanel_gui.widgets.settings_widget import SettingsWidget
from mcpanel_gui.workers import run_async

STATE_LABELS: dict[str, tuple[str, str]] = {
    "online": ("#2e7d32", "Online"),
    "starting": ("#b58900", "Starting"),
    "stopping": ("#b58900", "Stopping"),
    "unresponsive": ("#a83232", "Unresponsive"),
    "crashed": ("#c62828", "Crashed"),
    "offline": ("#6b7280", "Offline"),
    "unknown": ("#6b7280", "Unknown"),
}


class InstancePage(QWidget):
    def __init__(
        self,
        api_provider: Callable[[], NodeApi],
        instance_id: str,
        name: str,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._api_provider = api_provider
        self._instance_id = instance_id
        self._snapshot: InstanceSnapshot | None = None
        self._stream_connected = False

        layout = QVBoxLayout(self)

        title_row = QHBoxLayout()
        self._title = QLabel(name, self)
        self._title.setStyleSheet("font-size: 18px; font-weight: 600;")
        self._state = QLabel("Unknown", self)
        self._state.setStyleSheet("font-weight: 600;")
        title_row.addWidget(self._title)
        title_row.addStretch(1)
        title_row.addWidget(self._state)
        layout.addLayout(title_row)

        info_row = QHBoxLayout()
        self._players = QLabel("Players: —", self)
        self._latency = QLabel("Latency: —", self)
        self._uptime = QLabel("Uptime: —", self)
        self._version = QLabel("Version: —", self)
        for label in (self._players, self._latency, self._uptime, self._version):
            info_row.addWidget(label)
            info_row.addSpacing(12)
        info_row.addStretch(1)
        layout.addLayout(info_row)

        buttons = QHBoxLayout()
        self._start_button = QPushButton("Start", self)
        self._stop_button = QPushButton("Stop", self)
        self._restart_button = QPushButton("Restart", self)
        self._start_button.clicked.connect(lambda: self._action("start"))
        self._stop_button.clicked.connect(lambda: self._action("stop"))
        self._restart_button.clicked.connect(lambda: self._action("restart"))
        buttons.addWidget(self._start_button)
        buttons.addWidget(self._stop_button)
        buttons.addWidget(self._restart_button)
        buttons.addStretch(1)
        layout.addLayout(buttons)
        self._update_buttons()

        self._console = ConsoleWidget(self)
        self._console.command_submitted.connect(self._send_command)
        self._settings = SettingsWidget(api_provider, instance_id, self)
        tabs = QTabWidget(self)
        tabs.addTab(self._console, "Console")
        tabs.addTab(self._settings, "Settings")
        layout.addWidget(tabs, 1)

        self._status = QLabel("", self)
        self._status.setWordWrap(True)
        layout.addWidget(self._status)

        self._stream = ConsoleStream(api_provider, instance_id)
        self._stream.line_received.connect(self._console.append_line)
        self._stream.snapshot_received.connect(self.handle_snapshot)
        self._stream.command_result.connect(self._on_command_result)
        self._stream.connection_changed.connect(self._on_stream_connection)

        run_async(
            lambda: self._api_provider().get_snapshot(self._instance_id),
            self.handle_snapshot,
            self._show_error,
        )

    def refresh(self) -> None:
        """Re-fetch the snapshot (called on show and when the node reconnects)."""
        run_async(
            lambda: self._api_provider().get_snapshot(self._instance_id),
            self.handle_snapshot,
            self._show_error,
        )

    # --------------------------------------------------------------- lifecycle

    def showEvent(self, event: QShowEvent) -> None:
        super().showEvent(event)
        self._stream.start()
        self.refresh()

    def hideEvent(self, event: QHideEvent) -> None:
        self._stream.stop()
        super().hideEvent(event)

    def shutdown(self) -> None:
        self._stream.stop()

    # ------------------------------------------------------------- snapshots

    def handle_snapshot(self, snapshot: Any) -> None:
        if snapshot.id != self._instance_id:
            return
        self._snapshot = snapshot
        self._title.setText(snapshot.name)
        color, label = STATE_LABELS.get(snapshot.combined_state, STATE_LABELS["unknown"])
        self._state.setText(label)
        self._state.setStyleSheet(f"font-weight: 600; color: {color};")

        probe = snapshot.probe
        if probe.players_online is not None:
            maximum = probe.players_max if probe.players_max is not None else "?"
            self._players.setText(f"Players: {probe.players_online}/{maximum}")
        else:
            self._players.setText("Players: —")
        if probe.latency_ms is not None:
            self._latency.setText(f"Latency: {probe.latency_ms:.0f} ms")
        else:
            self._latency.setText("Latency: —")
        if snapshot.process.uptime_seconds is not None:
            uptime = int(snapshot.process.uptime_seconds)
            hours, remainder = divmod(uptime, 3600)
            minutes, seconds = divmod(remainder, 60)
            self._uptime.setText(f"Uptime: {hours:d}h {minutes:02d}m {seconds:02d}s")
        else:
            self._uptime.setText("Uptime: —")
        self._version.setText(f"Version: {probe.version or '—'}")
        self._update_buttons()

    def _update_buttons(self) -> None:
        process_state = self._snapshot.process.state if self._snapshot is not None else "offline"
        self._start_button.setEnabled(process_state in ("offline", "crashed"))
        self._stop_button.setEnabled(process_state in ("starting", "online"))
        self._restart_button.setEnabled(process_state in ("starting", "online"))
        can_command = process_state == "online" and self._stream_connected
        self._console.set_input_enabled(can_command)

    # ---------------------------------------------------------------- actions

    def _action(self, method: str) -> None:
        self._status.setText("")

        def call() -> Any:
            api = self._api_provider()
            return getattr(api, method)(self._instance_id)

        run_async(call, self._action_done, self._show_error)

    def _action_done(self, _result: Any) -> None:
        self._status.setText("Action accepted.")

    def _send_command(self, command: str) -> None:
        self._stream.send_command(command)

    def _on_command_result(self, accepted: bool, error: Any) -> None:
        if accepted:
            self._status.setText("Command sent.")
        else:
            self._status.setText(f"Command rejected: {error}")

    def _on_stream_connection(self, state: str) -> None:
        self._stream_connected = state == CONNECTION_CONNECTED
        self._update_buttons()

    def _show_error(self, exc: Exception) -> None:
        self._status.setText(f"Error: {exc}")
