"""PyInstaller entry point for the daemon binary."""

from __future__ import annotations

from mcpanel_daemon.__main__ import main

if __name__ == "__main__":
    raise SystemExit(main())
