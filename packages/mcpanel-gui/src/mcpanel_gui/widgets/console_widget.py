"""Console pane: auto-scrolling output, pause-on-scroll-up, command history."""

# pyright: basic

from __future__ import annotations

from typing import Any

from mcpanel_common.api import LogLine
from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QFontDatabase, QKeyEvent
from PySide6.QtWidgets import (
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QVBoxLayout,
    QWidget,
)

from mcpanel_gui.viewmodels import ConsoleModel


class _HistoryLineEdit(QLineEdit):
    def __init__(self, model: ConsoleModel, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._model = model

    def keyPressEvent(self, event: QKeyEvent) -> None:
        if event.key() == Qt.Key.Key_Up:
            text = self._model.history_previous()
            if text is not None:
                self.setText(text)
                self.setCursorPosition(len(text))
                return
        elif event.key() == Qt.Key.Key_Down:
            text = self._model.history_next()
            if text is not None:
                self.setText(text)
                return
        super().keyPressEvent(event)


class ConsoleWidget(QWidget):
    command_submitted = Signal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.model = ConsoleModel()
        self._output = QPlainTextEdit(self)
        self._output.setReadOnly(True)
        self._output.setMaximumBlockCount(20000)
        self._output.setFont(QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont))

        self._input = _HistoryLineEdit(self.model, self)
        self._input.setPlaceholderText("Type a command and press Enter")
        self._input.setEnabled(False)
        self._input.returnPressed.connect(self._submit)

        self._paused_label = QLabel("Auto-scroll paused — scroll to the bottom to resume.", self)
        self._paused_label.setStyleSheet("color: #b58900;")
        self._paused_label.hide()

        layout = QVBoxLayout(self)
        layout.addWidget(self._output, 1)
        input_row = QHBoxLayout()
        input_row.addWidget(QLabel(">", self))
        input_row.addWidget(self._input, 1)
        layout.addLayout(input_row)
        layout.addWidget(self._paused_label)

        self.model.line_added.connect(self._append_line)
        self._output.verticalScrollBar().valueChanged.connect(self._update_paused)

    def append_line(self, line: LogLine) -> None:
        self.model.add_line(line)

    def append_lines(self, lines: list[LogLine]) -> None:
        for line in lines:
            self.model.add_line(line)

    def set_input_enabled(self, enabled: bool) -> None:
        self._input.setEnabled(enabled)

    def clear(self) -> None:
        self._output.clear()
        self.model.clear()

    def _append_line(self, line: Any) -> None:
        scrollbar = self._output.verticalScrollBar()
        at_bottom = scrollbar.value() >= scrollbar.maximum() - 4
        timestamp = line.ts.astimezone().strftime("%H:%M:%S")
        self._output.appendPlainText(f"{timestamp} {line.line}")
        if at_bottom:
            scrollbar.setValue(scrollbar.maximum())

    def _update_paused(self, _value: int) -> None:
        scrollbar = self._output.verticalScrollBar()
        self._paused_label.setVisible(scrollbar.value() < scrollbar.maximum() - 4)

    def _submit(self) -> None:
        text = self._input.text()
        if not text.strip():
            return
        self.model.add_command(text)
        self._input.clear()
        self.command_submitted.emit(text)
