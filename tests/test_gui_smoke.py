"""Offscreen window smoke tests (no display required)."""

from __future__ import annotations

from pathlib import Path

import pytest
from mcpanel_gui.node_store import NodeStore
from mcpanel_gui.widgets.main_window import MainWindow
from PySide6.QtWidgets import QApplication


@pytest.fixture(scope="module")
def qapp() -> QApplication:
    app = QApplication.instance()
    if app is None:
        app = QApplication([])
    assert isinstance(app, QApplication)
    return app


def test_main_window_smoke(qapp: QApplication, tmp_path: Path) -> None:
    store = NodeStore(tmp_path, force_file_tokens=True)
    window = MainWindow(store, auto_connect=False)
    window.show()
    assert window.windowTitle() == "mcpanel"
    assert window.centralWidget() is not None
    window.close()


def test_add_local_node_is_one_click(qapp: QApplication, tmp_path: Path) -> None:
    store = NodeStore(tmp_path, force_file_tokens=True)
    window = MainWindow(store, auto_connect=False)
    window._add_local_node()  # pyright: ignore[reportPrivateUsage] - the one-click action
    nodes = store.nodes
    assert len(nodes) == 1
    assert nodes[0].name == "Local"
    assert nodes[0].url == "http://127.0.0.1:8080"
    window.close()
