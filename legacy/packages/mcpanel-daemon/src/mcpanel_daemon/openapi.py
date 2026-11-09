"""OpenAPI schema dump: ``python -m mcpanel_daemon.openapi``.

The schema is generated from the real app (no manager/auth needed; handlers are
not called). WebSocket routes are not representable in OpenAPI 3.0 — their
contract lives in ``mcpanel_common.api`` message models.
"""

from __future__ import annotations

import json

from mcpanel_daemon.app import create_app


def main() -> None:
    print(json.dumps(create_app().openapi(), indent=2))


if __name__ == "__main__":
    main()
