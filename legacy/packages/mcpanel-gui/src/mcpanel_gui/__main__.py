"""GUI entry point: ``mcpanel-gui``."""

# pyright: basic

from __future__ import annotations

import sys

from PySide6.QtWidgets import QApplication

from mcpanel_gui.widgets.main_window import MainWindow


def main() -> int:
    app = QApplication(sys.argv)
    app.setApplicationName("mcpanel")
    app.setOrganizationName("mcpanel")
    window = MainWindow()
    window.resize(1200, 800)
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
