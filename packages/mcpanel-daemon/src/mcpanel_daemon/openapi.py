"""OpenAPI schema generation.

Phase 0 artefact: the FastAPI app is a *contract skeleton*. Route handlers are
stubs (501) until Phase 3 — only the paths, parameters, auth requirements and
response models are real. Run ``python -m mcpanel_daemon.openapi`` to dump the
schema (committed at docs/openapi.json).
"""

from __future__ import annotations

import json
from typing import Annotated, NoReturn

from fastapi import APIRouter, Depends, FastAPI, HTTPException, Query, WebSocket
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from mcpanel_common.api import (
    ActionResponse,
    CommandRequest,
    CommandResponse,
    ConfigResponse,
    ConfigUpdateRequest,
    DiscordTestResponse,
    HealthResponse,
    InstanceSnapshot,
    InstanceSummary,
    LogsResponse,
)
from mcpanel_common.version import API_VERSION, __version__

_bearer = HTTPBearer(auto_error=False)


def _require_auth(
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(_bearer)],
) -> None:
    """Phase 0 stub: presence check only; constant-time compare lands in Phase 3."""
    if credentials is None:
        raise HTTPException(status_code=401, detail="missing bearer token")


def _stub() -> NoReturn:
    raise HTTPException(status_code=501, detail="not implemented yet")


def create_app() -> FastAPI:
    app = FastAPI(
        title="mcpanel daemon",
        version=__version__,
        description="Node daemon API for managing Minecraft Java Edition servers.",
    )

    @app.get("/health", response_model=HealthResponse, tags=["health"])
    async def health() -> HealthResponse:
        return HealthResponse(status="ok", api_version=API_VERSION, daemon_version=__version__)

    api = APIRouter(prefix="/api/v1", dependencies=[Depends(_require_auth)])

    @api.get("/instances", response_model=list[InstanceSummary], tags=["instances"])
    async def list_instances() -> list[InstanceSummary]:
        _stub()

    @api.get(
        "/instances/{instance_id}",
        response_model=InstanceSnapshot,
        tags=["instances"],
    )
    async def get_snapshot(instance_id: str) -> InstanceSnapshot:
        _stub()

    @api.post(
        "/instances/{instance_id}/start",
        response_model=ActionResponse,
        tags=["instances"],
    )
    async def start_instance(instance_id: str) -> ActionResponse:
        _stub()

    @api.post(
        "/instances/{instance_id}/stop",
        response_model=ActionResponse,
        tags=["instances"],
    )
    async def stop_instance(instance_id: str) -> ActionResponse:
        _stub()

    @api.post(
        "/instances/{instance_id}/restart",
        response_model=ActionResponse,
        tags=["instances"],
    )
    async def restart_instance(instance_id: str) -> ActionResponse:
        _stub()

    @api.post(
        "/instances/{instance_id}/command",
        response_model=CommandResponse,
        tags=["instances"],
    )
    async def send_command(instance_id: str, body: CommandRequest) -> CommandResponse:
        _stub()

    @api.get("/instances/{instance_id}/logs", response_model=LogsResponse, tags=["instances"])
    async def get_logs(
        instance_id: str,
        limit: Annotated[int, Query(ge=1, le=2000)] = 100,
        since_seq: Annotated[int, Query(ge=0)] = 0,
    ) -> LogsResponse:
        _stub()

    @api.get(
        "/instances/{instance_id}/config",
        response_model=ConfigResponse,
        tags=["config"],
    )
    async def get_config(instance_id: str) -> ConfigResponse:
        _stub()

    @api.put(
        "/instances/{instance_id}/config",
        response_model=ConfigResponse,
        tags=["config"],
    )
    async def update_config(instance_id: str, body: ConfigUpdateRequest) -> ConfigResponse:
        _stub()

    @api.post(
        "/instances/{instance_id}/discord/test",
        response_model=DiscordTestResponse,
        tags=["discord"],
    )
    async def discord_test(instance_id: str) -> DiscordTestResponse:
        _stub()

    @api.websocket("/instances/{instance_id}/console")
    async def console_ws(websocket: WebSocket, instance_id: str) -> None:
        await websocket.close(code=1000)

    @api.websocket("/events")
    async def events_ws(websocket: WebSocket) -> None:
        await websocket.close(code=1000)

    app.include_router(api)
    return app


def main() -> None:
    print(json.dumps(create_app().openapi(), indent=2))


if __name__ == "__main__":
    main()
