"""Shared pytest fixtures."""

from __future__ import annotations

import os

# Widgets must never need a display in tests.
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
