"""Append-only log file with size-based rotation.

Writes are small and local; they run on the event loop thread deliberately
(a daemon logging a Minecraft console is low-volume). No shell, text mode.
"""

from __future__ import annotations

from pathlib import Path
from typing import TextIO


class RotatingLogFile:
    def __init__(self, path: Path, max_bytes: int, backups: int) -> None:
        if backups < 0:
            raise ValueError("backups must be >= 0")
        self._path = path
        self._max_bytes = max_bytes
        self._backups = backups
        self._file: TextIO | None = None
        self._size = path.stat().st_size if path.exists() else 0

    def write(self, line: str) -> None:
        text = line + "\n"
        handle = self._file_handle()
        handle.write(text)
        handle.flush()
        self._size += len(text.encode("utf-8"))
        if self._size >= self._max_bytes:
            self._rotate()

    def close(self) -> None:
        if self._file is not None:
            self._file.close()
            self._file = None

    def _file_handle(self) -> TextIO:
        if self._file is None:
            self._path.parent.mkdir(parents=True, exist_ok=True)
            self._file = self._path.open("a", encoding="utf-8", errors="replace")
        return self._file

    def _rotate(self) -> None:
        self.close()
        if self._backups == 0:
            self._path.unlink(missing_ok=True)
            self._size = 0
            return
        oldest = self._path.with_name(f"{self._path.name}.{self._backups}")
        oldest.unlink(missing_ok=True)
        for index in range(self._backups - 1, 0, -1):
            src = self._path.with_name(f"{self._path.name}.{index}")
            if src.exists():
                src.replace(self._path.with_name(f"{self._path.name}.{index + 1}"))
        if self._path.exists():
            self._path.replace(self._path.with_name(f"{self._path.name}.1"))
        self._size = 0
