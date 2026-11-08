"""WebSocket endpoints: live console streaming and instance state events."""

from __future__ import annotations

import asyncio
import logging
from typing import cast

from fastapi import WebSocket, WebSocketDisconnect
from mcpanel_common.api import (
    ConsoleCaughtUp,
    ConsoleCommandMessage,
    ConsoleCommandResult,
    ConsoleError,
    ConsoleLogMessage,
    ConsoleResync,
    ConsoleSnapshotMessage,
    EventsHelloOk,
    ProcessState,
    StateChangeEvent,
    client_console_adapter,
)
from pydantic import BaseModel, ValidationError

from mcpanel_daemon.events import WILDCARD, Event, LogLineEvent, StateChange
from mcpanel_daemon.instances import InstanceManager, ManagedInstance, UnknownInstanceError
from mcpanel_daemon.process_manager import ProcessError

logger = logging.getLogger(__name__)


async def _send(websocket: WebSocket, message: BaseModel) -> None:
    await websocket.send_text(message.model_dump_json())


async def handle_console(
    websocket: WebSocket,
    instance_id: str,
    manager: InstanceManager | None,
) -> None:
    await websocket.accept()
    if manager is None:
        await _send(websocket, ConsoleError(type="error", error="daemon not initialized"))
        await websocket.close()
        return
    instance = manager.get(instance_id)
    if instance is None:
        await _send(
            websocket,
            ConsoleError(type="error", error=f"unknown instance: {instance_id}"),
        )
        await websocket.close()
        return

    queue = manager.bus.subscribe(instance_id)
    receive_task = asyncio.create_task(websocket.receive_text())
    queue_task = asyncio.create_task(queue.get())
    last_sent = 0
    try:
        last_sent = await _send_backfill(websocket, manager, instance, since_seq=None)
        while True:
            done, _ = await asyncio.wait(
                {receive_task, queue_task}, return_when=asyncio.FIRST_COMPLETED
            )
            if receive_task in done:
                try:
                    raw = receive_task.result()
                except WebSocketDisconnect:
                    break
                receive_task = asyncio.create_task(websocket.receive_text())
                last_sent = await _handle_client_text(
                    websocket, manager, instance, raw, last_sent
                )
            if queue_task in done:
                event = queue_task.result()
                queue_task = asyncio.create_task(queue.get())
                last_sent = await _handle_event(websocket, manager, instance, event, last_sent)
    except WebSocketDisconnect:
        pass
    finally:
        receive_task.cancel()
        queue_task.cancel()
        await asyncio.gather(receive_task, queue_task, return_exceptions=True)
        manager.bus.unsubscribe(instance_id, queue)


async def _send_backfill(
    websocket: WebSocket,
    manager: InstanceManager,
    instance: ManagedInstance,
    since_seq: int | None,
) -> int:
    lines = manager.backfill(instance.config.id, since_seq)
    for line in lines:
        await _send(websocket, ConsoleLogMessage(type="log", line=line))
    last_seq = instance.buffer.last_seq or 0
    await _send(websocket, ConsoleCaughtUp(type="caught_up", seq=last_seq))
    return last_seq


async def _handle_client_text(
    websocket: WebSocket,
    manager: InstanceManager,
    instance: ManagedInstance,
    raw: str,
    last_sent: int,
) -> int:
    try:
        message = client_console_adapter.validate_json(raw)
    except ValidationError:
        await _send(websocket, ConsoleError(type="error", error="invalid client message"))
        return last_sent
    if isinstance(message, ConsoleCommandMessage):
        try:
            await instance.process.send_command(message.command)
        except ProcessError as exc:
            await _send(
                websocket,
                ConsoleCommandResult(type="command_result", accepted=False, error=str(exc)),
            )
        else:
            await _send(
                websocket, ConsoleCommandResult(type="command_result", accepted=True)
            )
        return last_sent
    return max(last_sent, await _send_backfill(websocket, manager, instance, message.since_seq))


async def _handle_event(
    websocket: WebSocket,
    manager: InstanceManager,
    instance: ManagedInstance,
    event: Event,
    last_sent: int,
) -> int:
    if isinstance(event, LogLineEvent):
        if event.line.seq <= last_sent:
            return last_sent
        if event.line.seq > last_sent + 1:
            await _send(
                websocket,
                ConsoleResync(
                    type="resync", from_seq=last_sent + 1, to_seq=event.line.seq - 1
                ),
            )
        await _send(websocket, ConsoleLogMessage(type="log", line=event.line))
        return event.line.seq
    if isinstance(event, StateChange):
        try:
            snapshot = manager.snapshot(event.instance_id)
        except UnknownInstanceError:  # pragma: no cover - instances are fixed at startup
            return last_sent
        await _send(websocket, ConsoleSnapshotMessage(type="snapshot", snapshot=snapshot))
    return last_sent


async def handle_events(
    websocket: WebSocket,
    manager: InstanceManager | None,
) -> None:
    await websocket.accept()
    if manager is None:
        await _send(websocket, ConsoleError(type="error", error="daemon not initialized"))
        await websocket.close()
        return

    await _send(websocket, EventsHelloOk(type="hello_ok", instances=manager.summaries()))
    queue = manager.bus.subscribe(WILDCARD)
    receive_task = asyncio.create_task(websocket.receive_text())
    queue_task = asyncio.create_task(queue.get())
    try:
        while True:
            done, _ = await asyncio.wait(
                {receive_task, queue_task}, return_when=asyncio.FIRST_COMPLETED
            )
            if receive_task in done:
                try:
                    receive_task.result()
                except WebSocketDisconnect:
                    break
                receive_task = asyncio.create_task(websocket.receive_text())
            if queue_task in done:
                event = queue_task.result()
                queue_task = asyncio.create_task(queue.get())
                if isinstance(event, StateChange):
                    try:
                        snapshot = manager.snapshot(event.instance_id)
                    except UnknownInstanceError:  # pragma: no cover
                        continue
                    await _send(
                        websocket,
                        StateChangeEvent(
                            type="state_change",
                            instance_id=event.instance_id,
                            old_state=cast(ProcessState, event.old_state),
                            new_state=cast(ProcessState, event.new_state),
                            reason=event.reason,
                            snapshot=snapshot,
                        ),
                    )
    except WebSocketDisconnect:
        pass
    finally:
        receive_task.cancel()
        queue_task.cancel()
        await asyncio.gather(receive_task, queue_task, return_exceptions=True)
        manager.bus.unsubscribe(WILDCARD, queue)
