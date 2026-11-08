"""Offscreen window smoke tests (no display required)."""

# pyright: basic
# PySide6 ships no type stubs, so this file uses basic mode.

from __future__ import annotations

import time
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


def test_multiple_nodes_are_listed_and_persisted(qapp: QApplication, tmp_path: Path) -> None:
    store = NodeStore(tmp_path, force_file_tokens=True)
    alpha = store.add("Alpha", "http://127.0.0.1:8080")
    beta = store.add("Beta", "https://mc.example.com")

    window = MainWindow(store, auto_connect=False)
    listed = window._node_items  # pyright: ignore[reportPrivateUsage]
    assert set(listed) == {alpha.id, beta.id}

    store.update(alpha.id, name="Alpha 2", url="http://127.0.0.1:9090")
    reloaded = NodeStore(tmp_path, force_file_tokens=True)
    edited = reloaded.get(alpha.id)
    assert edited is not None
    assert edited.name == "Alpha 2"
    assert edited.url == "http://127.0.0.1:9090"


def test_local_setup_dialog_constructs(qapp: QApplication) -> None:
    from mcpanel_gui.widgets.local_setup_dialog import LocalSetupDialog, slugify_instance_id

    dialog = LocalSetupDialog()
    assert dialog.windowTitle() == "Set up a local daemon"
    assert dialog.instance_id.text() == "minecraft"
    assert dialog.daemon_port.value() == 8080
    assert slugify_instance_id("My Server!") == "my-server"
    dialog.close()


def test_local_setup_dialog_requires_server_jar(
    qapp: QApplication, tmp_path: Path
) -> None:
    from mcpanel_gui.widgets.local_setup_dialog import LocalSetupDialog

    dialog = LocalSetupDialog()
    dialog.work_dir.setText(str(tmp_path))
    dialog.extra_args.setText("nogui")
    # 'nogui' alone would make java treat it as the main class.
    collected = dialog._collect()  # pyright: ignore[reportPrivateUsage]
    assert collected is None
    assert "server jar" in dialog.status.text().lower()

    jar = tmp_path / "server.jar"
    jar.write_bytes(b"")
    dialog.jar_path.setText(str(jar))
    collected = dialog._collect()  # pyright: ignore[reportPrivateUsage]
    assert collected is not None
    assert collected.launch_args == ["-jar", str(jar), "nogui"]
    dialog.close()


def test_selecting_instance_opens_page(qapp: QApplication, tmp_path: Path) -> None:
    """Regression: clicking an instance must replace the placeholder pane."""
    from mcpanel_common.api import InstanceSummary
    from mcpanel_gui.apiclient import NodeApi
    from mcpanel_gui.widgets.instance_page import InstancePage

    store = NodeStore(tmp_path, force_file_tokens=True)
    node = store.add("Local", "http://127.0.0.1:9")
    window = MainWindow(
        store,
        api_factory=lambda url, token: NodeApi(url, token, timeout=0.2),
        auto_connect=False,
    )
    window._on_summaries(  # pyright: ignore[reportPrivateUsage]
        node.id,
        [InstanceSummary(id="minecraft", name="Minecraft", combined_state="online")],
    )
    item = window._instance_items[(node.id, "minecraft")]  # pyright: ignore[reportPrivateUsage]
    window._tree.setCurrentItem(item)
    qapp.processEvents()
    assert isinstance(window._stack.currentWidget(), InstancePage)  # pyright: ignore[reportPrivateUsage]
    window.close()


def test_first_run_opens_local_setup(
    qapp: QApplication, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    opened: list[bool] = []

    class _FakeSignals:
        def connect(self, _slot: object) -> None:
            pass

    class _FakeDialog:
        def __init__(self, _parent: object = None) -> None:
            self.setup_complete = _FakeSignals()
            opened.append(True)

        def exec(self) -> int:
            return 0

    monkeypatch.setattr(
        "mcpanel_gui.widgets.main_window.LocalSetupDialog", _FakeDialog
    )
    store = NodeStore(tmp_path, force_file_tokens=True)
    window = MainWindow(store, auto_connect=True)
    deadline = time.time() + 2
    while time.time() < deadline and not opened:
        qapp.processEvents()
        time.sleep(0.01)
    assert opened
    window.close()
