"""Settings tab: launch/probe/restart/Discord config for one instance."""

# pyright: basic

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from mcpanel_common.api import ConfigUpdateRequest, InstanceConfigView
from pydantic import ValidationError
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDoubleSpinBox,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)

from mcpanel_gui.apiclient import NodeApi
from mcpanel_gui.workers import run_async


def _lines(widget: QPlainTextEdit) -> list[str]:
    return [line.strip() for line in widget.toPlainText().splitlines() if line.strip()]


class SettingsWidget(QWidget):
    def __init__(
        self,
        api_provider: Callable[[], NodeApi],
        instance_id: str,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._api_provider = api_provider
        self._instance_id = instance_id
        self._rcon: Any = None
        self._build_ui()
        self.reload()

    # ------------------------------------------------------------------- UI

    def _build_ui(self) -> None:
        outer = QVBoxLayout(self)

        form_container = QWidget()
        form = QFormLayout(form_container)

        self.name = QLineEdit()
        self.work_dir = QLineEdit()
        self.java_path = QLineEdit()
        self.jvm_args = QPlainTextEdit()
        self.jvm_args.setPlaceholderText("One JVM argument per line")
        self.jvm_args.setMaximumHeight(80)
        self.launch_args = QPlainTextEdit()
        self.launch_args.setPlaceholderText(
            "One launch argument per line, e.g. -jar / server.jar / nogui"
        )
        self.launch_args.setMaximumHeight(80)
        self.autostart = QCheckBox("Start when the daemon starts")

        form.addRow("Name", self.name)
        form.addRow("Working directory", self.work_dir)
        form.addRow("Java path", self.java_path)
        form.addRow("JVM args", self.jvm_args)
        form.addRow("Launch args", self.launch_args)
        form.addRow("", self.autostart)

        probe_group = QGroupBox("Status probe")
        probe_form = QFormLayout(probe_group)
        self.probe_host = QLineEdit()
        self.probe_port = QSpinBox()
        self.probe_port.setRange(1, 65535)
        self.query_enabled = QCheckBox("Query enabled (player names)")
        self.query_port = QSpinBox()
        self.query_port.setRange(0, 65535)
        self.query_port.setSpecialValueText("same as probe port")
        self.probe_timeout = QDoubleSpinBox()
        self.probe_timeout.setRange(0.1, 60.0)
        self.probe_timeout.setDecimals(1)
        probe_form.addRow("Host", self.probe_host)
        probe_form.addRow("Port", self.probe_port)
        probe_form.addRow("", self.query_enabled)
        probe_form.addRow("Query port", self.query_port)
        probe_form.addRow("Timeout (s)", self.probe_timeout)
        form.addRow(probe_group)

        restart_group = QGroupBox("Restart policy")
        restart_form = QFormLayout(restart_group)
        self.restart_policy = QComboBox()
        self.restart_policy.addItem("Restart on crash", "on-crash")
        self.restart_policy.addItem("Never restart", "never")
        self.max_retries = QSpinBox()
        self.max_retries.setRange(0, 100)
        self.backoff_base = QDoubleSpinBox()
        self.backoff_base.setRange(0.1, 3600)
        self.backoff_base.setDecimals(1)
        self.backoff_max = QDoubleSpinBox()
        self.backoff_max.setRange(1, 7200)
        self.backoff_max.setDecimals(1)
        self.reset_after = QDoubleSpinBox()
        self.reset_after.setRange(1, 86400)
        self.reset_after.setDecimals(0)
        restart_form.addRow("Policy", self.restart_policy)
        restart_form.addRow("Max retries", self.max_retries)
        restart_form.addRow("Backoff base (s)", self.backoff_base)
        restart_form.addRow("Backoff max (s)", self.backoff_max)
        restart_form.addRow("Reset counter after (s online)", self.reset_after)
        form.addRow(restart_group)

        discord_group = QGroupBox("Discord")
        discord_form = QFormLayout(discord_group)
        self.discord_enabled = QCheckBox("Enabled")
        self.discord_mode = QComboBox()
        self.discord_mode.addItem("Edit message in place", "edit-in-place")
        self.discord_mode.addItem("New message each interval", "new-message")
        self.discord_interval = QDoubleSpinBox()
        self.discord_interval.setRange(0.1, 86400)
        self.discord_interval.setDecimals(0)
        self.discord_first_delay = QDoubleSpinBox()
        self.discord_first_delay.setRange(0, 3600)
        self.discord_first_delay.setDecimals(0)
        self.discord_log_lines = QSpinBox()
        self.discord_log_lines.setRange(0, 50)
        self.discord_max_chars = QSpinBox()
        self.discord_max_chars.setRange(0, 4096)
        self.discord_state_posts = QCheckBox("Post on state changes (debounced)")
        self.discord_debounce = QDoubleSpinBox()
        self.discord_debounce.setRange(0.1, 3600)
        self.discord_debounce.setDecimals(0)
        self.discord_webhook = QLineEdit()
        self.discord_webhook.setEchoMode(QLineEdit.EchoMode.Password)
        self.discord_webhook.setPlaceholderText("https://discord.com/api/webhooks/…")
        self.discord_redact = QPlainTextEdit()
        self.discord_redact.setPlaceholderText("Extra redaction regexes, one per line")
        self.discord_redact.setMaximumHeight(70)
        self.discord_test_button = QPushButton("Send test message")
        self.discord_test_button.clicked.connect(self.test_discord)

        discord_form.addRow("", self.discord_enabled)
        discord_form.addRow("Mode", self.discord_mode)
        discord_form.addRow("Interval (s)", self.discord_interval)
        discord_form.addRow("First push delay (s)", self.discord_first_delay)
        discord_form.addRow("Log lines", self.discord_log_lines)
        discord_form.addRow("Max log chars", self.discord_max_chars)
        discord_form.addRow("", self.discord_state_posts)
        discord_form.addRow("Debounce (s)", self.discord_debounce)
        discord_form.addRow("Webhook URL", self.discord_webhook)
        discord_form.addRow("Redaction regexes", self.discord_redact)
        discord_form.addRow("", self.discord_test_button)
        form.addRow(discord_group)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setWidget(form_container)
        outer.addWidget(scroll, 1)

        buttons = QHBoxLayout()
        self.save_button = QPushButton("Save")
        self.save_button.clicked.connect(self.save)
        self.reload_button = QPushButton("Reload")
        self.reload_button.clicked.connect(self.reload)
        buttons.addWidget(self.save_button)
        buttons.addWidget(self.reload_button)
        buttons.addStretch(1)
        outer.addLayout(buttons)

        self.status = QLabel("")
        self.status.setWordWrap(True)
        outer.addWidget(self.status)

    # ------------------------------------------------------------- loading

    def reload(self) -> None:
        self.status.setText("Loading…")
        run_async(
            lambda: self._api_provider().get_config(self._instance_id),
            self._apply_config,
            self._show_error,
        )

    def _apply_config(self, config: InstanceConfigView) -> None:
        self._rcon = config.rcon
        self.name.setText(config.name)
        self.work_dir.setText(config.work_dir)
        self.java_path.setText(config.java_path)
        self.jvm_args.setPlainText("\n".join(config.jvm_args))
        self.launch_args.setPlainText("\n".join(config.launch_args))
        self.autostart.setChecked(config.autostart)

        self.probe_host.setText(config.probe.host)
        self.probe_port.setValue(config.probe.port)
        self.query_enabled.setChecked(config.probe.query_enabled)
        self.query_port.setValue(config.probe.query_port or 0)
        self.probe_timeout.setValue(config.probe.timeout)

        index = self.restart_policy.findData(config.restart.policy)
        self.restart_policy.setCurrentIndex(max(0, index))
        self.max_retries.setValue(config.restart.max_retries)
        self.backoff_base.setValue(config.restart.backoff_base_seconds)
        self.backoff_max.setValue(config.restart.backoff_max_seconds)
        self.reset_after.setValue(config.restart.reset_after_online_seconds)

        discord = config.discord
        self.discord_enabled.setChecked(discord.enabled)
        index = self.discord_mode.findData(discord.mode)
        self.discord_mode.setCurrentIndex(max(0, index))
        self.discord_interval.setValue(discord.interval_seconds)
        self.discord_first_delay.setValue(discord.first_push_delay_seconds)
        self.discord_log_lines.setValue(discord.log_lines)
        self.discord_max_chars.setValue(discord.max_log_chars)
        self.discord_state_posts.setChecked(discord.state_change_posts)
        self.discord_debounce.setValue(discord.state_change_debounce_seconds)
        self.discord_redact.setPlainText("\n".join(discord.redact_regexes))
        self.discord_webhook.clear()
        if discord.webhook_set:
            self.discord_webhook.setPlaceholderText("•••••• set — leave blank to keep")
        else:
            self.discord_webhook.setPlaceholderText("https://discord.com/api/webhooks/…")
        self.status.setText("")

    # -------------------------------------------------------------- saving

    def _collect(self) -> ConfigUpdateRequest:
        webhook = self.discord_webhook.text().strip()
        data: dict[str, Any] = {
            "name": self.name.text().strip(),
            "work_dir": self.work_dir.text().strip(),
            "java_path": self.java_path.text().strip() or "java",
            "jvm_args": _lines(self.jvm_args),
            "launch_args": _lines(self.launch_args),
            "autostart": self.autostart.isChecked(),
            "probe": {
                "host": self.probe_host.text().strip() or "127.0.0.1",
                "port": self.probe_port.value(),
                "query_enabled": self.query_enabled.isChecked(),
                "query_port": self.query_port.value() or None,
                "timeout": self.probe_timeout.value(),
            },
            "restart": {
                "policy": self.restart_policy.currentData(),
                "max_retries": self.max_retries.value(),
                "backoff_base_seconds": self.backoff_base.value(),
                "backoff_max_seconds": self.backoff_max.value(),
                "reset_after_online_seconds": self.reset_after.value(),
            },
            "rcon": self._rcon.model_dump() if self._rcon is not None else None,
            "discord": {
                "enabled": self.discord_enabled.isChecked(),
                "mode": self.discord_mode.currentData(),
                "interval_seconds": self.discord_interval.value(),
                "first_push_delay_seconds": self.discord_first_delay.value(),
                "log_lines": self.discord_log_lines.value(),
                "max_log_chars": self.discord_max_chars.value(),
                "state_change_posts": self.discord_state_posts.isChecked(),
                "state_change_debounce_seconds": self.discord_debounce.value(),
                "redact_regexes": _lines(self.discord_redact),
                "webhook_url": webhook or None,
            },
        }
        return ConfigUpdateRequest.model_validate(data)

    def save(self) -> None:
        try:
            body = self._collect()
        except ValidationError as exc:
            self.status.setText(f"Invalid settings: {exc.errors()[0].get('msg', exc)}")
            return
        self.status.setText("Saving…")
        run_async(
            lambda: self._api_provider().update_config(self._instance_id, body),
            self._saved,
            self._show_error,
        )

    def _saved(self, config: InstanceConfigView) -> None:
        self._apply_config(config)
        self.status.setText("Saved. Launch/probe changes apply on the next start.")

    def test_discord(self) -> None:
        self.status.setText("Sending test message…")
        run_async(
            lambda: self._api_provider().discord_test(self._instance_id),
            self._tested,
            self._show_error,
        )

    def _tested(self, result: Any) -> None:
        if result.ok:
            self.status.setText("Test message sent.")
        else:
            self.status.setText(f"Test failed: {result.error}")

    def _show_error(self, exc: Exception) -> None:
        self.status.setText(f"Error: {exc}")
