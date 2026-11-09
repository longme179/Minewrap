"""Contract test: the generated OpenAPI schema exposes exactly the agreed API."""

from __future__ import annotations

from typing import Any, cast

from mcpanel_common.api import CombinedState, ProcessState
from mcpanel_daemon.app import create_app

EXPECTED_REST_PATHS = {
    "/health": {"get"},
    "/api/v1/instances": {"get"},
    "/api/v1/instances/{instance_id}": {"get"},
    "/api/v1/instances/{instance_id}/start": {"post"},
    "/api/v1/instances/{instance_id}/stop": {"post"},
    "/api/v1/instances/{instance_id}/restart": {"post"},
    "/api/v1/instances/{instance_id}/command": {"post"},
    "/api/v1/instances/{instance_id}/logs": {"get"},
    "/api/v1/instances/{instance_id}/config": {"get", "put"},
    "/api/v1/instances/{instance_id}/discord/test": {"post"},
}

# WebSocket routes cannot be represented in OpenAPI 3.0 and are therefore absent
# from openapi.json; the WS contract lives in mcpanel_common.api message models
# (tested in test_api_models.py) and is documented in ARCHITECTURE.md.

EXPECTED_SCHEMAS = {
    "HealthResponse",
    "InstanceSummary",
    "InstanceSnapshot",
    "ProbeInfo",
    "ProcessInfo",
    "LogLine",
    "LogsResponse",
    "ActionResponse",
    "CommandRequest",
    "CommandResponse",
    "DiscordTestResponse",
    "ConfigUpdateRequest",
    "InstanceConfigView",
}


def _openapi() -> dict[str, Any]:
    return create_app().openapi()


def test_every_rest_path_and_method_present() -> None:
    spec = _openapi()
    actual = {
        path: set(ops) for path, ops in spec["paths"].items() if "websocket" not in ops
    }
    assert actual == EXPECTED_REST_PATHS


def test_health_is_unauthenticated_everything_else_requires_bearer() -> None:
    spec = _openapi()
    security_schemes = spec["components"]["securitySchemes"]
    assert "HTTPBearer" in security_schemes
    for path, ops in spec["paths"].items():
        for method, op in ops.items():
            if method == "parameters":
                continue
            if path == "/health":
                assert "security" not in op, "/health must not require auth"
            else:
                assert op.get("security") == [{"HTTPBearer": []}], (
                    f"{method.upper()} {path} must require the bearer token"
                )


def test_required_schemas_present() -> None:
    spec = _openapi()
    names = set(spec["components"]["schemas"])
    assert names >= EXPECTED_SCHEMAS, f"missing: {EXPECTED_SCHEMAS - names}"


def _inline_enum(schema: dict[str, Any], path: list[str]) -> list[str]:
    """Resolve `["ProcessInfo", "properties", "state", "enum"]` through refs."""
    cur: Any = schema["components"]["schemas"]
    for step in path:
        if isinstance(cur, dict) and "$ref" in cur:
            ref = cast(dict[str, Any], cur)["$ref"]
            cur = schema["components"]["schemas"][cast(str, ref).split("/")[-1]]
        assert isinstance(cur, dict), f"cannot resolve {path} at {step}"
        cur = cast(dict[str, Any], cur)[step]
    assert isinstance(cur, list)
    return cast(list[str], cur)


def test_state_enums_in_schema() -> None:
    spec = _openapi()
    process_state = _inline_enum(spec, ["ProcessInfo", "properties", "state", "enum"])
    combined_state = _inline_enum(spec, ["InstanceSummary", "properties", "combined_state", "enum"])
    assert set(process_state) == set(ProcessState.__args__)  # type: ignore[attr-defined]
    assert set(combined_state) == set(CombinedState.__args__)  # type: ignore[attr-defined]


def test_snapshot_separates_process_and_probe() -> None:
    spec = _openapi()
    snapshot = spec["components"]["schemas"]["InstanceSnapshot"]
    required = set(snapshot["required"])
    assert {"process", "probe", "combined_state"} <= required
