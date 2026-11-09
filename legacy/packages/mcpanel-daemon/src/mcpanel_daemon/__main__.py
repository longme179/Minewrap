"""Daemon entry point: ``mcpanel-daemon --config path/to/config.toml``."""

from __future__ import annotations

import argparse
import logging
from pathlib import Path

import uvicorn

from mcpanel_daemon.app import create_app
from mcpanel_daemon.auth import TokenAuth
from mcpanel_daemon.config_store import ConfigError, load_config, resolve_token
from mcpanel_daemon.events import EventBus
from mcpanel_daemon.instances import InstanceManager

logger = logging.getLogger(__name__)


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="mcpanel-daemon",
        description="mcpanel node daemon (Minecraft Java Edition server manager)",
    )
    parser.add_argument("--config", type=Path, required=True, help="path to config.toml")
    parser.add_argument(
        "--log-level",
        default="INFO",
        choices=["DEBUG", "INFO", "WARNING", "ERROR"],
        help="logging level (default: INFO)",
    )
    args = parser.parse_args()

    logging.basicConfig(
        level=args.log_level,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    try:
        config = load_config(args.config)
        token, generated = resolve_token(config)
    except ConfigError as exc:
        logger.error("%s", exc)
        return 2

    if generated:
        # Shown exactly once; afterwards only readable from the 0600 token file.
        print(token, flush=True)
        logger.warning(
            "generated API token written to %s/token (shown once above)",
            config.daemon.data_dir,
        )

    manager = InstanceManager(config, args.config.resolve(), EventBus())
    app = create_app(manager, TokenAuth(token))
    uvicorn.run(
        app,
        host=config.daemon.host,
        port=config.daemon.port,
        log_level=args.log_level.lower(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
