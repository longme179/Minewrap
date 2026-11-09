"""HTTP/WebSocket client for one daemon node.

Pure Python (no Qt): safe to call from worker threads and easy to test.
"""

# pyright: basic

from __future__ import annotations

import json
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any

import httpx
from mcpanel_common.api import (
    ActionResponse,
    CommandResponse,
    ConfigResponse,
    ConfigUpdateRequest,
    DiscordTestResponse,
    HealthResponse,
    InstanceSnapshot,
    InstanceSummary,
    LogsResponse,
)
from mcpanel_common.version import API_VERSION
from websockets.sync.client import ClientConnection, connect


class ApiError(Exception):
    def __init__(self, message: str, status_code: int | None = None) -> None:
        super().__init__(message)
        self.status_code = status_code


class UnauthorizedError(ApiError):
    def __init__(self, message: str = "unauthorized: check the node token") -> None:
        super().__init__(message, 401)


class IncompatibleVersionError(Exception):
    def __init__(self, client_api: str, daemon_api: str) -> None:
        super().__init__(
            f"incompatible daemon API {daemon_api} (client speaks {client_api})"
        )
        self.client_api = client_api
        self.daemon_api = daemon_api


def api_major(version: str) -> str:
    return version.split(".", 1)[0]


class NodeApi:
    def __init__(
        self,
        base_url: str,
        token: str,
        *,
        timeout: float = 10.0,
        client: httpx.Client | None = None,
    ) -> None:
        self._base = base_url.rstrip("/")
        self._token = token
        self._client = client if client is not None else httpx.Client(timeout=timeout)

    def close(self) -> None:
        self._client.close()

    # ------------------------------------------------------------------ REST

    def _request(self, method: str, path: str, **kwargs: Any) -> Any:
        try:
            response = self._client.request(
                method,
                f"{self._base}{path}",
                headers={"Authorization": f"Bearer {self._token}"},
                **kwargs,
            )
        except httpx.HTTPError as exc:
            raise ApiError(f"cannot reach node: {type(exc).__name__}") from exc
        if response.status_code == 401:
            raise UnauthorizedError()
        if response.status_code >= 400:
            detail = f"HTTP {response.status_code}"
            try:
                payload = response.json()
                if isinstance(payload, dict) and "detail" in payload:
                    detail = str(payload["detail"])
            except ValueError:
                pass
            raise ApiError(detail, response.status_code)
        if not response.content:
            return None
        return response.json()

    def health(self) -> HealthResponse:
        return HealthResponse.model_validate(self._request("GET", "/health"))

    def check_compatible(self) -> HealthResponse:
        health = self.health()
        if api_major(health.api_version) != api_major(API_VERSION):
            raise IncompatibleVersionError(API_VERSION, health.api_version)
        return health

    def list_instances(self) -> list[InstanceSummary]:
        payload = self._request("GET", "/api/v1/instances")
        return [InstanceSummary.model_validate(item) for item in payload]

    def get_snapshot(self, instance_id: str) -> InstanceSnapshot:
        return InstanceSnapshot.model_validate(
            self._request("GET", f"/api/v1/instances/{instance_id}")
        )

    def start(self, instance_id: str) -> ActionResponse:
        return ActionResponse.model_validate(
            self._request("POST", f"/api/v1/instances/{instance_id}/start")
        )

    def stop(self, instance_id: str) -> ActionResponse:
        return ActionResponse.model_validate(
            self._request("POST", f"/api/v1/instances/{instance_id}/stop")
        )

    def restart(self, instance_id: str) -> ActionResponse:
        return ActionResponse.model_validate(
            self._request("POST", f"/api/v1/instances/{instance_id}/restart")
        )

    def send_command(self, instance_id: str, command: str) -> CommandResponse:
        return CommandResponse.model_validate(
            self._request(
                "POST",
                f"/api/v1/instances/{instance_id}/command",
                json={"command": command},
            )
        )

    def get_logs(
        self, instance_id: str, limit: int = 500, since_seq: int = 0
    ) -> LogsResponse:
        return LogsResponse.model_validate(
            self._request(
                "GET",
                f"/api/v1/instances/{instance_id}/logs",
                params={"limit": limit, "since_seq": since_seq},
            )
        )

    def get_config(self, instance_id: str) -> ConfigResponse:
        return ConfigResponse.model_validate(
            self._request("GET", f"/api/v1/instances/{instance_id}/config")
        )

    def update_config(
        self, instance_id: str, body: ConfigUpdateRequest
    ) -> ConfigResponse:
        payload = body.model_dump(mode="python", exclude={"discord": {"webhook_url"}})
        if body.discord.webhook_url is not None:
            payload["discord"]["webhook_url"] = body.discord.webhook_url.get_secret_value()
        return ConfigResponse.model_validate(
            self._request(
                "PUT", f"/api/v1/instances/{instance_id}/config", json=payload
            )
        )

    def discord_test(self, instance_id: str) -> DiscordTestResponse:
        return DiscordTestResponse.model_validate(
            self._request("POST", f"/api/v1/instances/{instance_id}/discord/test")
        )

    # ------------------------------------------------------------- WebSocket

    def _ws_url(self, path: str) -> str:
        if self._base.startswith("https://"):
            return "wss://" + self._base[len("https://") :] + path
        if self._base.startswith("http://"):
            return "ws://" + self._base[len("http://") :] + path
        raise ApiError("node URL must use http:// or https://")

    @contextmanager
    def connect_console(
        self, instance_id: str, since_seq: int | None = None
    ) -> Iterator[ClientConnection]:
        url = self._ws_url(f"/api/v1/instances/{instance_id}/console")
        with connect(
            url,
            additional_headers={"Authorization": f"Bearer {self._token}"},
            open_timeout=10,
        ) as ws:
            if since_seq is not None:
                ws.send(json.dumps({"type": "hello", "since_seq": since_seq}))
            yield ws

    @contextmanager
    def connect_events(self) -> Iterator[ClientConnection]:
        with connect(
            self._ws_url("/api/v1/events"),
            additional_headers={"Authorization": f"Bearer {self._token}"},
            open_timeout=10,
        ) as ws:
            yield ws
