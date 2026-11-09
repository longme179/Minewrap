"""Background sockets: events stream (per node) and console stream (per page).

All Qt signals are emitted from worker threads; Qt queues them to the receiver's
thread, so widgets always update on the main thread.
"""

# pyright: basic

from __future__ import annotations

import contextlib
import json
import logging
import threading
from collections.abc import Callable

from mcpanel_common.api import (
    ConsoleCommandResult,
    ConsoleError,
    ConsoleLogMessage,
    ConsoleResync,
    ConsoleSnapshotMessage,
    EventsHelloOk,
    StateChangeEvent,
    server_console_adapter,
    server_event_adapter,
)
from PySide6.QtCore import QObject, Signal
from websockets.exceptions import ConnectionClosed, InvalidStatus
from websockets.sync.client import ClientConnection

from mcpanel_gui.apiclient import (
    ApiError,
    IncompatibleVersionError,
    NodeApi,
    UnauthorizedError,
)
from mcpanel_gui.node_store import Node
from mcpanel_gui.viewmodels import (
    CONNECTION_CONNECTED,
    CONNECTION_CONNECTING,
    CONNECTION_INCOMPATIBLE,
    CONNECTION_OFFLINE,
    CONNECTION_UNAUTHORIZED,
)

logger = logging.getLogger(__name__)

MAX_BACKOFF_SECONDS = 60.0


def _http_status(exc: Exception) -> int | None:
    response = getattr(exc, "response", None)
    return getattr(response, "status_code", None)


class NodeSession(QObject):
    # (node_id, state) — node id is carried in the payload so slots can be
    # bound methods (Qt queues cross-thread deliveries reliably).
    connection_changed = Signal(str, str)
    summaries_changed = Signal(str, list)
    snapshot_received = Signal(str, object)

    def __init__(
        self,
        node: Node,
        token: str | None,
        api_factory: Callable[[str, str], NodeApi] = NodeApi,
    ) -> None:
        super().__init__()
        self.node = node
        self.api = api_factory(node.url, token or "")
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self._ws: ClientConnection | None = None
        self._lock = threading.Lock()
        self._state = ""

    def start(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop_event.clear()
        self._thread = threading.Thread(
            target=self._run, daemon=True, name=f"mcpanel-events-{self.node.id}"
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop_event.set()
        with self._lock:
            ws = self._ws
        if ws is not None:
            with contextlib.suppress(Exception):
                ws.close()

    # ---------------------------------------------------------------- internals

    def _set_state(self, state: str) -> None:
        if state != self._state:
            self._state = state
            self.connection_changed.emit(self.node.id, state)

    def _sleep(self, seconds: float) -> None:
        self._stop_event.wait(seconds)

    def _run(self) -> None:
        backoff = 1.0
        while not self._stop_event.is_set():
            self._set_state(CONNECTION_CONNECTING)
            try:
                self.api.check_compatible()
            except IncompatibleVersionError:
                self._set_state(CONNECTION_INCOMPATIBLE)
                return
            except UnauthorizedError:
                self._set_state(CONNECTION_UNAUTHORIZED)
                self._sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECONDS)
                continue
            except ApiError:
                self._set_state(CONNECTION_OFFLINE)
                self._sleep(backoff)
                backoff = min(backoff * 2, MAX_BACKOFF_SECONDS)
                continue

            self._set_state(CONNECTION_CONNECTED)
            backoff = 1.0
            try:
                with self.api.connect_events() as ws:
                    with self._lock:
                        self._ws = ws
                    while not self._stop_event.is_set():
                        raw = ws.recv()
                        message = server_event_adapter.validate_json(raw)
                        if isinstance(message, EventsHelloOk):
                            self.summaries_changed.emit(self.node.id, list(message.instances))
                        elif isinstance(message, StateChangeEvent):
                            self.snapshot_received.emit(self.node.id, message.snapshot)
            except InvalidStatus as exc:
                status = _http_status(exc)
                self._set_state(
                    CONNECTION_UNAUTHORIZED
                    if status == 401
                    else CONNECTION_OFFLINE
                )
            except (ConnectionClosed, OSError, TimeoutError):
                pass
            except Exception:
                logger.exception("events socket failed for %s", self.node.id)
            finally:
                with self._lock:
                    self._ws = None
            if self._stop_event.is_set():
                break
            self._set_state(CONNECTION_OFFLINE)
            self._sleep(backoff)
            backoff = min(backoff * 2, MAX_BACKOFF_SECONDS)
        self._set_state(CONNECTION_OFFLINE)


class ConsoleStream(QObject):
    line_received = Signal(object)
    snapshot_received = Signal(object)
    command_result = Signal(bool, object)
    connection_changed = Signal(str)

    def __init__(self, api_provider: Callable[[], NodeApi], instance_id: str) -> None:
        super().__init__()
        self._api_provider = api_provider
        self._instance_id = instance_id
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self._ws: ClientConnection | None = None
        self._lock = threading.Lock()
        self._last_seq: int | None = None
        self._state = ""

    def start(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop_event.clear()
        self._thread = threading.Thread(
            target=self._run,
            daemon=True,
            name=f"mcpanel-console-{self._instance_id}",
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop_event.set()
        with self._lock:
            ws = self._ws
        if ws is not None:
            with contextlib.suppress(Exception):
                ws.close()
        thread = self._thread
        if thread is not None and thread.is_alive() and thread is not threading.current_thread():
            thread.join(timeout=2.0)
        self._thread = None

    def send_command(self, command: str) -> None:
        with self._lock:
            ws = self._ws
        if ws is None:
            self.command_result.emit(False, "console is not connected")
            return
        try:
            ws.send(json.dumps({"type": "command", "command": command}))
        except Exception as exc:
            self.command_result.emit(False, f"send failed: {type(exc).__name__}")

    def _set_state(self, state: str) -> None:
        if state != self._state:
            self._state = state
            self.connection_changed.emit(state)

    def _sleep(self, seconds: float) -> None:
        self._stop_event.wait(seconds)

    def _run(self) -> None:
        backoff = 1.0
        while not self._stop_event.is_set():
            self._set_state(CONNECTION_CONNECTING)
            try:
                api = self._api_provider()
                with api.connect_console(
                    self._instance_id, since_seq=self._last_seq
                ) as ws:
                    with self._lock:
                        self._ws = ws
                    self._set_state(CONNECTION_CONNECTED)
                    backoff = 1.0
                    while not self._stop_event.is_set():
                        raw = ws.recv()
                        message = server_console_adapter.validate_json(raw)
                        if isinstance(message, ConsoleLogMessage):
                            self._last_seq = message.line.seq
                            self.line_received.emit(message.line)
                        elif isinstance(message, ConsoleSnapshotMessage):
                            self.snapshot_received.emit(message.snapshot)
                        elif isinstance(message, ConsoleResync):
                            # Slow-consumer gap: reconnect with hello(since_seq)
                            # to re-backfill the missing lines.
                            break
                        elif isinstance(message, ConsoleCommandResult):
                            self.command_result.emit(message.accepted, message.error)
                        elif isinstance(message, ConsoleError):
                            self.command_result.emit(False, message.error)
            except InvalidStatus as exc:
                status = _http_status(exc)
                self._set_state(
                    CONNECTION_UNAUTHORIZED if status == 401 else CONNECTION_OFFLINE
                )
            except (ConnectionClosed, OSError, TimeoutError):
                pass
            except Exception:
                logger.exception(
                    "console socket failed for %s/%s",
                    self._instance_id,
                    self._instance_id,
                )
            finally:
                with self._lock:
                    self._ws = None
            if self._stop_event.is_set():
                break
            self._set_state(CONNECTION_OFFLINE)
            self._sleep(backoff)
            backoff = min(backoff * 2, MAX_BACKOFF_SECONDS)
        self._set_state(CONNECTION_OFFLINE)
