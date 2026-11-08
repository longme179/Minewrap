"""Local daemon setup wizard: writes config.toml and starts the daemon."""

# pyright: basic

from __future__ import annotations

import re
from pathlib import Path

from mcpanel_common.config import INSTANCE_ID_RE
from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)

from mcpanel_gui.daemon_launcher import (
    InstanceSpec,
    LaunchError,
    LaunchResult,
    provision_local_daemon,
)
from mcpanel_gui.workers import run_async


def slugify_instance_id(name: str) -> str:
    slug = re.sub(r"[^a-z0-9_-]+", "-", name.strip().lower())
    slug = slug.strip("-_")
    return (slug or "minecraft")[:64]


class LocalSetupDialog(QDialog):
    setup_complete = Signal(object)  # {"url": str, "token": str, "name": str}

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Set up a local daemon")
        self.setMinimumWidth(560)
        self._last_auto_id = ""

        layout = QVBoxLayout(self)

        intro = QLabel(
            "Creates a daemon config, starts the daemon (detached, so it keeps "
            "running when the GUI closes) and connects this node. The daemon "
            "starts and owns the Minecraft server."
        )
        intro.setWordWrap(True)
        layout.addWidget(intro)

        form_container = QWidget(self)
        form = QFormLayout(form_container)

        config_row = QHBoxLayout()
        self.config_path = QLineEdit(str(Path.home() / "mcpanel" / "config.toml"))
        config_browse = QPushButton("Browse…")
        config_browse.clicked.connect(self._browse_config)
        config_row.addWidget(self.config_path, 1)
        config_row.addWidget(config_browse)
        form.addRow("Config file", config_row)

        daemon_row = QHBoxLayout()
        self.daemon_host = QLineEdit("127.0.0.1")
        self.daemon_port = QSpinBox()
        self.daemon_port.setRange(1, 65535)
        self.daemon_port.setValue(8080)
        daemon_row.addWidget(QLabel("Host"))
        daemon_row.addWidget(self.daemon_host, 1)
        daemon_row.addWidget(QLabel("Port"))
        daemon_row.addWidget(self.daemon_port)
        form.addRow("Daemon bind", daemon_row)

        self.token = QLineEdit()
        self.token.setEchoMode(QLineEdit.EchoMode.Password)
        self.token.setPlaceholderText("leave blank to generate (stored 0600)")
        form.addRow("API token", self.token)

        layout.addWidget(form_container)

        server_group = QGroupBox("Minecraft server")
        server_form = QFormLayout(server_group)

        self.server_name = QLineEdit("Minecraft")
        self.server_name.textEdited.connect(self._on_name_edited)
        server_form.addRow("Display name", self.server_name)

        self.instance_id = QLineEdit("minecraft")
        self.instance_id.textEdited.connect(self._on_id_edited)
        server_form.addRow("Instance id", self.instance_id)

        work_row = QHBoxLayout()
        self.work_dir = QLineEdit("")
        self.work_dir.setPlaceholderText("/path/to/your/server")
        work_browse = QPushButton("Browse…")
        work_browse.clicked.connect(self._browse_work_dir)
        work_row.addWidget(self.work_dir, 1)
        work_row.addWidget(work_browse)
        server_form.addRow("Server directory", work_row)

        self.java_path = QLineEdit("java")
        server_form.addRow("Java path", self.java_path)

        self.jvm_args = QPlainTextEdit("-Xmx4G")
        self.jvm_args.setMaximumHeight(70)
        self.jvm_args.setPlaceholderText("One JVM argument per line")
        server_form.addRow("JVM args", self.jvm_args)

        jar_row = QHBoxLayout()
        self.jar_path = QLineEdit("")
        self.jar_path.setPlaceholderText("fabric-server-…-launcher.…jar")
        jar_browse = QPushButton("Browse…")
        jar_browse.clicked.connect(self._browse_jar)
        jar_row.addWidget(self.jar_path, 1)
        jar_row.addWidget(jar_browse)
        server_form.addRow("Server jar", jar_row)

        self.extra_args = QLineEdit("nogui")
        server_form.addRow("Extra launch args", self.extra_args)

        self.autostart = QCheckBox("Start the server when the daemon starts")
        self.autostart.setChecked(True)
        server_form.addRow("", self.autostart)

        probe_row = QHBoxLayout()
        self.probe_port = QSpinBox()
        self.probe_port.setRange(1, 65535)
        self.probe_port.setValue(25565)
        self.query_enabled = QCheckBox("Query player names (needs enable-query)")
        probe_row.addWidget(QLabel("Probe port"))
        probe_row.addWidget(self.probe_port)
        probe_row.addWidget(self.query_enabled, 1)
        server_form.addRow("Status probe", probe_row)

        layout.addWidget(server_group)

        discord_group = QGroupBox("Discord (optional)")
        discord_form = QFormLayout(discord_group)
        self.discord_enabled = QCheckBox("Enable Discord updates")
        discord_form.addRow("", self.discord_enabled)
        self.webhook = QLineEdit()
        self.webhook.setEchoMode(QLineEdit.EchoMode.Password)
        self.webhook.setPlaceholderText("https://discord.com/api/webhooks/…")
        discord_form.addRow("Webhook URL", self.webhook)
        discord_row = QHBoxLayout()
        self.discord_interval = QDoubleSpinBox()
        self.discord_interval.setRange(0.1, 86400)
        self.discord_interval.setDecimals(0)
        self.discord_interval.setValue(600)
        self.discord_log_lines = QSpinBox()
        self.discord_log_lines.setRange(0, 50)
        self.discord_log_lines.setValue(15)
        discord_row.addWidget(QLabel("Every (s)"))
        discord_row.addWidget(self.discord_interval)
        discord_row.addWidget(QLabel("Log lines"))
        discord_row.addWidget(self.discord_log_lines)
        discord_row.addStretch(1)
        discord_form.addRow("Updates", discord_row)
        layout.addWidget(discord_group)

        buttons = QHBoxLayout()
        self.create_button = QPushButton("Create & start")
        self.create_button.setDefault(True)
        self.create_button.clicked.connect(self._on_create)
        cancel = QDialogButtonBox(QDialogButtonBox.StandardButton.Cancel, self)
        cancel.rejected.connect(self.reject)
        buttons.addWidget(self.create_button)
        buttons.addWidget(cancel)
        layout.addLayout(buttons)

        self.status = QLabel("")
        self.status.setWordWrap(True)
        layout.addWidget(self.status)

    # ------------------------------------------------------------------ browse

    def _browse_config(self) -> None:
        path, _ = QFileDialog.getSaveFileName(
            self, "Daemon config file", self.config_path.text(), "TOML (*.toml)"
        )
        if path:
            self.config_path.setText(path)

    def _browse_work_dir(self) -> None:
        path = QFileDialog.getExistingDirectory(
            self, "Minecraft server directory", self.work_dir.text() or str(Path.home())
        )
        if path:
            self.work_dir.setText(path)

    def _browse_jar(self) -> None:
        path, _ = QFileDialog.getOpenFileName(
            self, "Server jar", self.work_dir.text() or str(Path.home()), "Java archives (*.jar)"
        )
        if path:
            self.jar_path.setText(path)

    # ------------------------------------------------------------------- ids

    def _on_name_edited(self, text: str) -> None:
        current = self.instance_id.text()
        if current == self._last_auto_id or not current:
            self._last_auto_id = slugify_instance_id(text)
            self.instance_id.setText(self._last_auto_id)

    def _on_id_edited(self, _text: str) -> None:
        self._last_auto_id = ""  # user took over; stop auto-filling

    # ------------------------------------------------------------------ submit

    def _collect(self) -> InstanceSpec | None:
        name = self.server_name.text().strip() or "Minecraft"
        instance_id = self.instance_id.text().strip() or slugify_instance_id(name)
        if not INSTANCE_ID_RE.fullmatch(instance_id):
            self.status.setText(
                "Instance id must be 1-64 chars of [a-z0-9_-], not starting or "
                "ending with '-' or '_'."
            )
            return None

        work_dir = self.work_dir.text().strip()
        if not work_dir or not Path(work_dir).expanduser().is_dir():
            self.status.setText("Choose an existing server directory.")
            return None

        jar = self.jar_path.text().strip()
        if jar and not Path(jar).expanduser().is_file():
            self.status.setText("The server jar path does not exist.")
            return None

        extras = self.extra_args.text().split()
        launch_args = (["-jar", str(Path(jar).expanduser())] if jar else []) + extras
        if not launch_args:
            self.status.setText("Provide the server jar and/or launch arguments.")
            return None

        discord_enabled = self.discord_enabled.isChecked()
        webhook = self.webhook.text().strip()
        if discord_enabled and not webhook:
            self.status.setText("Discord is enabled but the webhook URL is empty.")
            return None

        return InstanceSpec(
            instance_id=instance_id,
            name=name,
            work_dir=str(Path(work_dir).expanduser().resolve()),
            java_path=self.java_path.text().strip() or "java",
            jvm_args=[
                line.strip() for line in self.jvm_args.toPlainText().splitlines() if line.strip()
            ],
            launch_args=launch_args,
            autostart=self.autostart.isChecked(),
            probe_port=self.probe_port.value(),
            query_enabled=self.query_enabled.isChecked(),
            discord_enabled=discord_enabled,
            webhook_url=webhook or None,
            discord_interval=self.discord_interval.value(),
            discord_log_lines=self.discord_log_lines.value(),
        )

    def _on_create(self) -> None:
        spec = self._collect()
        if spec is None:
            return

        config_path = Path(self.config_path.text()).expanduser()
        if config_path.exists():
            answer = QMessageBox.question(
                self,
                "Overwrite config?",
                f"{config_path} already exists. Replace it with this setup?",
            )
            if answer != QMessageBox.StandardButton.Yes:
                return

        host = self.daemon_host.text().strip() or "127.0.0.1"
        port = self.daemon_port.value()
        token = self.token.text().strip() or None
        data_dir = config_path.expanduser().resolve().parent / "data"

        self.create_button.setEnabled(False)
        self.status.setText("Writing config and starting the daemon…")

        def work() -> LaunchResult:
            return provision_local_daemon(
                config_path=config_path,
                host=host,
                port=port,
                data_dir=data_dir,
                instance=spec,
                token=token,
                overwrite=True,
            )

        run_async(work, self._on_done, self._on_error)

    def _on_done(self, result: LaunchResult) -> None:
        self.setup_complete.emit(
            {"url": result.url, "token": result.token, "name": result.name}
        )
        self.accept()

    def _on_error(self, exc: Exception) -> None:
        self.create_button.setEnabled(True)
        if isinstance(exc, LaunchError):
            self.status.setText(str(exc))
        else:
            self.status.setText(f"Setup failed: {exc}")
