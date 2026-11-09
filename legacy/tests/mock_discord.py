"""Minimal mock Discord webhook server for tests (stdlib only).

# pyright: basic
"""

from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, format: str, *args: Any) -> None:
        pass

    def _handle(self, method: str) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b""
        payload = json.loads(raw) if raw else None
        server: _Server = self.server  # type: ignore[assignment]
        server.requests.append({"method": method, "path": self.path, "payload": payload})
        status, body, headers = server.next_response()
        data = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for key, value in headers.items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self) -> None:
        self._handle("POST")

    def do_PATCH(self) -> None:
        self._handle("PATCH")


class _Server(ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int]) -> None:
        super().__init__(address, _Handler)
        self.requests: list[dict[str, Any]] = []
        self.responses: list[tuple[int, dict[str, Any], dict[str, str]]] = []

    def next_response(self) -> tuple[int, dict[str, Any], dict[str, str]]:
        if self.responses:
            return self.responses.pop(0)
        return 200, {"id": "msg-1"}, {}


class MockDiscord:
    def __init__(self) -> None:
        self._server = _Server(("127.0.0.1", 0))
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def webhook_url(self) -> str:
        host, port = self._server.server_address[0], self._server.server_address[1]
        return f"http://{host}:{port}/api/webhooks/123/test-token"

    @property
    def requests(self) -> list[dict[str, Any]]:
        return self._server.requests

    def queue_response(
        self,
        status: int,
        body: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
    ) -> None:
        self._server.responses.append((status, body or {}, headers or {}))

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()
