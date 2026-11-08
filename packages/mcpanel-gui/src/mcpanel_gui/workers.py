"""Run blocking calls off the UI thread (QThreadPool + queued signals)."""

# pyright: basic

from __future__ import annotations

import contextlib
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QObject, QRunnable, QThreadPool, Signal

_keep_alive: set[QRunnable] = set()


class _Signals(QObject):
    done = Signal(object)
    error = Signal(object)


class _Task(QRunnable):
    def __init__(self, fn: Callable[[], Any]) -> None:
        super().__init__()
        self._fn = fn
        self.signals = _Signals()

    def run(self) -> None:
        # The window may be closing; emitting from a deleted signal source
        # raises RuntimeError, which must not escape the worker thread.
        try:
            result = self._fn()
        except Exception as exc:
            with contextlib.suppress(RuntimeError):
                self.signals.error.emit(exc)
        else:
            with contextlib.suppress(RuntimeError):
                self.signals.done.emit(result)
        finally:
            _keep_alive.discard(self)


def run_async(
    fn: Callable[[], Any],
    on_done: Callable[[Any], None],
    on_error: Callable[[Exception], None],
) -> None:
    task = _Task(fn)
    task.signals.done.connect(on_done)
    task.signals.error.connect(on_error)
    _keep_alive.add(task)
    QThreadPool.globalInstance().start(task)
