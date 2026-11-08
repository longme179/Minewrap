"""Main window: node list, instance tree, per-instance pages."""

# pyright: basic

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QAction, QBrush, QCloseEvent, QColor
from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QMainWindow,
    QMenu,
    QMessageBox,
    QPushButton,
    QSplitter,
    QStackedWidget,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from mcpanel_gui.apiclient import NodeApi
from mcpanel_gui.node_store import DEFAULT_LOCAL_URL, Node, NodeStore
from mcpanel_gui.session import NodeSession
from mcpanel_gui.viewmodels import (
    CONNECTION_CONNECTED,
    CONNECTION_CONNECTING,
    CONNECTION_INCOMPATIBLE,
    CONNECTION_OFFLINE,
    CONNECTION_UNAUTHORIZED,
    NodeViewModel,
)
from mcpanel_gui.widgets.instance_page import InstancePage
from mcpanel_gui.widgets.local_setup_dialog import LocalSetupDialog

CONNECTION_TEXT = {
    CONNECTION_CONNECTED: "connected",
    CONNECTION_CONNECTING: "connecting…",
    CONNECTION_OFFLINE: "offline",
    CONNECTION_UNAUTHORIZED: "unauthorized",
    CONNECTION_INCOMPATIBLE: "incompatible daemon",
}

STATE_COLORS = {
    "online": "#2e7d32",
    "starting": "#b58900",
    "stopping": "#b58900",
    "unresponsive": "#a83232",
    "crashed": "#c62828",
    "offline": "#6b7280",
    "unknown": "#6b7280",
}

NODE_ROLE = Qt.ItemDataRole.UserRole


class _NodeDialog(QDialog):
    def __init__(
        self,
        parent: QWidget | None = None,
        *,
        node: Node | None = None,
    ) -> None:
        super().__init__(parent)
        editing = node is not None
        self.setWindowTitle("Edit node" if editing else "Add remote node")
        form = QFormLayout(self)
        self.name = QLineEdit(node.name if node is not None else "Remote", self)
        self.url = QLineEdit(node.url if node is not None else "http://", self)
        self.token = QLineEdit(self)
        self.token.setEchoMode(QLineEdit.EchoMode.Password)
        self.token.setPlaceholderText(
            "leave blank to keep the current token" if editing else "Node API token"
        )
        form.addRow("Name", self.name)
        form.addRow("URL", self.url)
        form.addRow("Token", self.token)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel,
            self,
        )
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        form.addRow(buttons)


class MainWindow(QMainWindow):
    def __init__(
        self,
        node_store: NodeStore | None = None,
        *,
        api_factory: Callable[[str, str], NodeApi] = NodeApi,
        auto_connect: bool = True,
    ) -> None:
        super().__init__()
        self.setWindowTitle("mcpanel")
        self._store = node_store if node_store is not None else NodeStore()
        self._api_factory = api_factory
        self._auto_connect = auto_connect
        self._sessions: dict[str, NodeSession] = {}
        self._views: dict[str, NodeViewModel] = {}
        self._node_items: dict[str, QTreeWidgetItem] = {}
        self._instance_items: dict[tuple[str, str], QTreeWidgetItem] = {}
        self._pages: dict[tuple[str, str], InstancePage] = {}
        self._prompted: set[str] = set()

        toolbar = self.addToolBar("Main")
        toolbar.setMovable(False)
        setup = QAction("Set up local daemon…", self)
        add_local = QAction("Add local node", self)
        add_remote = QAction("Add remote node", self)
        edit = QAction("Edit node", self)
        reconnect = QAction("Reconnect", self)
        remove = QAction("Remove node", self)
        setup.triggered.connect(self._open_local_setup)
        add_local.triggered.connect(self._add_local_node)
        add_remote.triggered.connect(self._add_remote_node)
        edit.triggered.connect(self._edit_selected)
        reconnect.triggered.connect(self._reconnect_selected)
        remove.triggered.connect(self._remove_selected)
        toolbar.addAction(setup)
        toolbar.addSeparator()
        toolbar.addAction(add_local)
        toolbar.addAction(add_remote)
        toolbar.addSeparator()
        toolbar.addAction(edit)
        toolbar.addAction(reconnect)
        toolbar.addAction(remove)

        self._tree = QTreeWidget(self)
        self._tree.setColumnCount(2)
        self._tree.setHeaderLabels(["Node / Instance", "State"])
        self._tree.setColumnWidth(0, 260)
        self._tree.currentItemChanged.connect(self._on_selection)
        self._tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self._tree.customContextMenuRequested.connect(self._show_context_menu)

        self._stack = QStackedWidget(self)
        placeholder = QWidget(self)
        placeholder_layout = QVBoxLayout(placeholder)
        placeholder_layout.addStretch(1)
        placeholder_text = QLabel(
            "Add a node to get started.\n\n"
            "New to mcpanel? Let the GUI create a daemon config and start the "
            "daemon for you — or add an existing node with its URL and token."
        )
        placeholder_text.setAlignment(Qt.AlignmentFlag.AlignCenter)
        placeholder_text.setStyleSheet("color: #6b7280; padding: 20px;")
        setup_button = QPushButton("Set up a local daemon…", placeholder)
        setup_button.clicked.connect(self._open_local_setup)
        placeholder_layout.addWidget(placeholder_text)
        placeholder_layout.addWidget(
            setup_button, alignment=Qt.AlignmentFlag.AlignHCenter
        )
        placeholder_layout.addStretch(1)
        self._stack.addWidget(placeholder)

        splitter = QSplitter(self)
        splitter.addWidget(self._tree)
        splitter.addWidget(self._stack)
        splitter.setStretchFactor(0, 0)
        splitter.setStretchFactor(1, 1)
        splitter.setSizes([320, 900])
        self.setCentralWidget(splitter)
        self.statusBar().showMessage("Ready")

        for node in self._store.nodes:
            self._add_node_item(node)
            if auto_connect:
                self._start_session(node)

    # -------------------------------------------------------------- node list

    def _add_node_item(self, node: Node) -> None:
        item = QTreeWidgetItem([node.name, CONNECTION_TEXT[CONNECTION_OFFLINE]])
        item.setData(0, NODE_ROLE, ("node", node.id))
        item.setToolTip(0, node.url)
        self._tree.addTopLevelItem(item)
        self._node_items[node.id] = item
        self._views[node.id] = NodeViewModel(node.id, node.name, node.url, parent=self)

    def _start_session(self, node: Node) -> None:
        session = NodeSession(node, self._store.get_token(node.id), self._api_factory)
        session.connection_changed.connect(self._on_connection)
        session.summaries_changed.connect(self._on_summaries)
        session.snapshot_received.connect(self._on_snapshot)
        self._sessions[node.id] = session
        session.start()

    def _restart_session(self, node_id: str) -> None:
        session = self._sessions.pop(node_id, None)
        if session is not None:
            session.stop()
        node = self._store.get(node_id)
        if node is not None:
            self._start_session(node)

    def _api_provider(self, node_id: str, fallback: Node) -> Callable[[], NodeApi]:
        def provider() -> NodeApi:
            session = self._sessions.get(node_id)
            if session is not None:
                return session.api
            return self._api_factory(fallback.url, self._store.get_token(node_id) or "")

        return provider

    # ---------------------------------------------------------------- signals

    def _on_connection(self, node_id: str, state: str) -> None:
        view = self._views.get(node_id)
        if view is not None:
            view.set_connection(state)
        item = self._node_items.get(node_id)
        if item is not None:
            item.setText(1, CONNECTION_TEXT.get(state, state))
            color = {
                CONNECTION_CONNECTED: "#2e7d32",
                CONNECTION_CONNECTING: "#b58900",
                CONNECTION_OFFLINE: "#6b7280",
                CONNECTION_UNAUTHORIZED: "#c62828",
                CONNECTION_INCOMPATIBLE: "#a83232",
            }.get(state, "#6b7280")
            item.setForeground(1, QBrush(QColor(color)))
        if state == CONNECTION_CONNECTED:
            self._prompted.discard(node_id)
            self.statusBar().showMessage("Connected")
        elif state == CONNECTION_UNAUTHORIZED:
            self.statusBar().showMessage("Unauthorized — paste the node token")
            self._prompt_token(node_id)
        elif state == CONNECTION_INCOMPATIBLE:
            self.statusBar().showMessage(
                "Daemon API version is incompatible with this client"
            )
        elif state == CONNECTION_OFFLINE:
            self.statusBar().showMessage("Node offline — retrying")

    def _prompt_token(self, node_id: str) -> None:
        if node_id in self._prompted:
            return
        node = self._store.get(node_id)
        if node is None:
            return
        self._prompted.add(node_id)
        token, accepted = QInputDialog.getText(
            self,
            "Node token",
            f"Paste the API token for “{node.name}” ({node.url}):",
            QLineEdit.EchoMode.Password,
        )
        if accepted and token.strip():
            self._store.set_token(node_id, token.strip())
            self._restart_session(node_id)

    def _on_summaries(self, node_id: str, summaries: Any) -> None:
        view = self._views.get(node_id)
        if view is None:
            return
        view.apply_summaries(summaries)
        node_item = self._node_items.get(node_id)
        if node_item is None:
            return

        for key in list(self._instance_items):
            if key[0] == node_id and key[1] not in view.instances:
                item = self._instance_items.pop(key)
                index = node_item.indexOfChild(item)
                if index >= 0:
                    node_item.takeChild(index)
                page = self._pages.pop(key, None)
                if page is not None:
                    page.shutdown()
                    self._stack.removeWidget(page)
                    page.deleteLater()

        for instance_id, instance_view in view.instances.items():
            key = (node_id, instance_id)
            item = self._instance_items.get(key)
            if item is None:
                item = QTreeWidgetItem([instance_view.name, instance_view.combined_state])
                item.setData(
                    0, NODE_ROLE, ("instance", node_id, instance_id, instance_view.name)
                )
                node_item.addChild(item)
                self._instance_items[key] = item
            item.setText(0, instance_view.name)
            item.setText(1, instance_view.combined_state)
            item.setForeground(
                1,
                QBrush(
                    QColor(STATE_COLORS.get(instance_view.combined_state, "#6b7280"))
                ),
            )
        node_item.setExpanded(True)

    def _on_snapshot(self, node_id: str, snapshot: Any) -> None:
        view = self._views.get(node_id)
        if view is not None:
            instance_view = view.get(snapshot.id)
            if instance_view is not None:
                instance_view.update_snapshot(snapshot)
                item = self._instance_items.get((node_id, snapshot.id))
                if item is not None:
                    item.setText(0, snapshot.name)
                    item.setText(1, snapshot.combined_state)
                    item.setForeground(
                        1,
                        QBrush(
                            QColor(STATE_COLORS.get(snapshot.combined_state, "#6b7280"))
                        ),
                    )
        page = self._pages.get((node_id, snapshot.id))
        if page is not None:
            page.handle_snapshot(snapshot)

    # ------------------------------------------------------------- navigation

    def _on_selection(self, current: QTreeWidgetItem | None, _previous: Any) -> None:
        if current is None:
            self._stack.setCurrentIndex(0)
            return
        data = current.data(0, NODE_ROLE)
        if data is not None and data[0] == "instance":
            _, node_id, instance_id, name = data
            self._stack.setCurrentWidget(self._ensure_page(node_id, instance_id, name))
        else:
            self._stack.setCurrentIndex(0)

    def _ensure_page(self, node_id: str, instance_id: str, name: str) -> InstancePage:
        key = (node_id, instance_id)
        page = self._pages.get(key)
        if page is not None:
            return page
        node = self._store.get(node_id)
        fallback = node or Node(id=node_id, name="Node", url=DEFAULT_LOCAL_URL)
        page = InstancePage(self._api_provider(node_id, fallback), instance_id, name)
        self._pages[key] = page
        self._stack.addWidget(page)
        return page

    # ---------------------------------------------------------------- actions

    def _open_local_setup(self) -> None:
        dialog = LocalSetupDialog(self)
        dialog.setup_complete.connect(self._on_local_setup)
        dialog.exec()

    def _on_local_setup(self, result: Any) -> None:
        url = str(result["url"])
        token = str(result["token"])
        name = str(result["name"])

        existing = next((node for node in self._store.nodes if node.url == url), None)
        if existing is None:
            node = self._store.add(name, url)
            self._add_node_item(node)
        else:
            node = existing
        self._store.set_token(node.id, token)
        if node.id in self._sessions:
            self._restart_session(node.id)
        elif self._auto_connect:
            self._start_session(node)
        self.statusBar().showMessage(f"Local daemon ready at {url}")

    def _add_local_node(self) -> None:
        node = self._store.add("Local", DEFAULT_LOCAL_URL)
        self._add_node_item(node)
        if self._auto_connect:
            self._start_session(node)

    def _add_remote_node(self) -> None:
        dialog = _NodeDialog(self)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            node = self._store.add(dialog.name.text().strip() or "Remote", dialog.url.text())
        except ValueError as exc:
            QMessageBox.warning(self, "Invalid node URL", str(exc))
            return
        if dialog.token.text().strip():
            self._store.set_token(node.id, dialog.token.text().strip())
        self._add_node_item(node)
        if self._auto_connect:
            self._start_session(node)

    def _edit_selected(self) -> None:
        node_id = self._selected_node_id()
        if node_id is None:
            return
        node = self._store.get(node_id)
        if node is None:
            return
        dialog = _NodeDialog(self, node=node)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            self._store.update(node.id, name=dialog.name.text(), url=dialog.url.text())
        except ValueError as exc:
            QMessageBox.warning(self, "Invalid node URL", str(exc))
            return
        token = dialog.token.text().strip()
        if token:
            self._store.set_token(node.id, token)
        item = self._node_items.get(node.id)
        if item is not None:
            item.setText(0, node.name)
            item.setToolTip(0, node.url)
        view = self._views.get(node.id)
        if view is not None:
            view.name = node.name
            view.url = node.url
        self._restart_session(node.id)

    def _selected_node_id(self) -> str | None:
        current = self._tree.currentItem()
        if current is None:
            return None
        data = current.data(0, NODE_ROLE)
        if data is None:
            return None
        return str(data[1])

    def _reconnect_selected(self) -> None:
        node_id = self._selected_node_id()
        if node_id is not None:
            self._restart_session(node_id)

    def _remove_selected(self) -> None:
        node_id = self._selected_node_id()
        if node_id is None:
            return
        node = self._store.get(node_id)
        if node is None:
            return
        answer = QMessageBox.question(
            self,
            "Remove node",
            f"Remove “{node.name}” from this client? The daemon keeps running.",
        )
        if answer != QMessageBox.StandardButton.Yes:
            return
        session = self._sessions.pop(node_id, None)
        if session is not None:
            session.stop()
        for key in list(self._pages):
            if key[0] == node_id:
                page = self._pages.pop(key)
                page.shutdown()
                self._stack.removeWidget(page)
                page.deleteLater()
        for key in list(self._instance_items):
            if key[0] == node_id:
                del self._instance_items[key]
        item = self._node_items.pop(node_id, None)
        if item is not None:
            index = self._tree.indexOfTopLevelItem(item)
            if index >= 0:
                self._tree.takeTopLevelItem(index)
        self._views.pop(node_id, None)
        self._store.remove(node_id)
        self._stack.setCurrentIndex(0)

    def _show_context_menu(self, position: Any) -> None:
        node_id = self._selected_node_id()
        if node_id is None:
            return
        menu = QMenu(self)
        edit = menu.addAction("Edit node…")
        reconnect = menu.addAction("Reconnect")
        remove = menu.addAction("Remove node")
        chosen = menu.exec(self._tree.viewport().mapToGlobal(position))
        if chosen is edit:
            self._edit_selected()
        elif chosen is reconnect:
            self._restart_session(node_id)
        elif chosen is remove:
            self._remove_selected()

    def closeEvent(self, event: QCloseEvent) -> None:
        for session in self._sessions.values():
            session.stop()
        for page in self._pages.values():
            page.shutdown()
        super().closeEvent(event)
