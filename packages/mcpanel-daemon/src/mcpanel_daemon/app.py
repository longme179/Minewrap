"""FastAPI application wiring: REST routes, WS endpoints, auth, lifespan."""

from __future__ import annotations

import logging
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from typing import Annotated

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

from mcpanel_daemon.auth import TokenAuth
from mcpanel_daemon.instances import InstanceManager, ManagedInstance, UnknownInstanceError
from mcpanel_daemon.process_manager import ProcessError
from mcpanel_daemon.ws import handle_console, handle_events

logger = logging.getLogger(__name__)

_bearer = HTTPBearer(auto_error=False)


def create_app(
    manager: InstanceManager | None = None,
    auth: TokenAuth | None = None,
) -> FastAPI:
    """Build the daemon app.

    ``manager``/``auth`` are None only for OpenAPI schema generation; the
    generated schema is identical to the runtime one (handlers are not called).
    """

    @asynccontextmanager
    async def lifespan(_app: FastAPI) -> AsyncGenerator[None, None]:
        if manager is not None:
            await manager.start()
        try:
            yield
        finally:
            if manager is not None:
                await manager.stop()

    app = FastAPI(
        title="mcpanel daemon",
        version=__version__,
        description="Node daemon API for managing Minecraft Java Edition servers.",
        lifespan=lifespan,
    )

    def require_auth(
        credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(_bearer)],
    ) -> None:
        if auth is None or credentials is None or not auth.verify_token(credentials.credentials):
            raise HTTPException(
                status_code=401,
                detail="invalid or missing bearer token",
                headers={"WWW-Authenticate": "Bearer"},
            )

    def get_manager() -> InstanceManager:
        if manager is None:
            raise HTTPException(status_code=503, detail="daemon not initialized")
        return manager

    def instance_or_404(instance_id: str) -> ManagedInstance:
        try:
            return get_manager().require(instance_id)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc

    @app.get("/health", response_model=HealthResponse, tags=["health"])
    async def health() -> HealthResponse:
        return HealthResponse(status="ok", api_version=API_VERSION, daemon_version=__version__)

    api = APIRouter(prefix="/api/v1", dependencies=[Depends(require_auth)])

    @api.get("/instances", response_model=list[InstanceSummary], tags=["instances"])
    async def list_instances() -> list[InstanceSummary]:
        return get_manager().summaries()

    @api.get(
        "/instances/{instance_id}",
        response_model=InstanceSnapshot,
        tags=["instances"],
    )
    async def get_snapshot(instance_id: str) -> InstanceSnapshot:
        return instance_or_404(instance_id).snapshot()

    @api.post(
        "/instances/{instance_id}/start",
        response_model=ActionResponse,
        tags=["instances"],
    )
    async def start_instance(instance_id: str) -> ActionResponse:
        try:
            get_manager().request_start(instance_id)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc
        except ProcessError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        return ActionResponse(ok=True)

    @api.post(
        "/instances/{instance_id}/stop",
        response_model=ActionResponse,
        tags=["instances"],
    )
    async def stop_instance(instance_id: str) -> ActionResponse:
        try:
            get_manager().request_stop(instance_id)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc
        except ProcessError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        return ActionResponse(ok=True)

    @api.post(
        "/instances/{instance_id}/restart",
        response_model=ActionResponse,
        tags=["instances"],
    )
    async def restart_instance(instance_id: str) -> ActionResponse:
        try:
            get_manager().request_restart(instance_id)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc
        except ProcessError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        return ActionResponse(ok=True)

    @api.post(
        "/instances/{instance_id}/command",
        response_model=CommandResponse,
        tags=["instances"],
    )
    async def send_command(instance_id: str, body: CommandRequest) -> CommandResponse:
        try:
            await get_manager().send_command(instance_id, body.command)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc
        except ProcessError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        return CommandResponse(accepted=True)

    @api.get("/instances/{instance_id}/logs", response_model=LogsResponse, tags=["instances"])
    async def get_logs(
        instance_id: str,
        limit: Annotated[int, Query(ge=1, le=2000)] = 100,
        since_seq: Annotated[int, Query(ge=0)] = 0,
    ) -> LogsResponse:
        try:
            return get_manager().logs(instance_id, limit, since_seq)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc

    @api.get(
        "/instances/{instance_id}/config",
        response_model=ConfigResponse,
        tags=["config"],
    )
    async def get_config(instance_id: str) -> ConfigResponse:
        instance_or_404(instance_id)
        return get_manager().config_view(instance_id)

    @api.put(
        "/instances/{instance_id}/config",
        response_model=ConfigResponse,
        tags=["config"],
    )
    async def update_config(instance_id: str, body: ConfigUpdateRequest) -> ConfigResponse:
        try:
            return await get_manager().update_config(instance_id, body)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc

    @api.post(
        "/instances/{instance_id}/discord/test",
        response_model=DiscordTestResponse,
        tags=["discord"],
    )
    async def discord_test(instance_id: str) -> DiscordTestResponse:
        try:
            await get_manager().discord_test(instance_id)
        except UnknownInstanceError as exc:
            raise HTTPException(
                status_code=404, detail=f"unknown instance: {instance_id}"
            ) from exc
        except NotImplementedError as exc:
            raise HTTPException(status_code=501, detail=str(exc)) from exc
        return DiscordTestResponse(ok=True)

    @api.websocket("/instances/{instance_id}/console")
    async def console_ws(websocket: WebSocket, instance_id: str) -> None:
        await handle_console(websocket, instance_id, manager)

    @api.websocket("/events")
    async def events_ws(websocket: WebSocket) -> None:
        await handle_events(websocket, manager)

    app.include_router(api)
    return app
