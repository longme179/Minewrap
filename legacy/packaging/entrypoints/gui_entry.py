"""PyInstaller entry point for the GUI binary."""

from __future__ import annotations

from mcpanel_gui.__main__ import main

if __name__ == "__main__":
    raise SystemExit(main())
