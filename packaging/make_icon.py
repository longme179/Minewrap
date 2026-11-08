"""Render the AppImage icon (256x256 PNG) with Qt, headless.

Usage: uv run python packaging/make_icon.py <output.png>
"""

# pyright: basic

from __future__ import annotations

import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QRectF, Qt
from PySide6.QtGui import QColor, QFont, QGuiApplication, QPainter, QPixmap


def main() -> int:
    output = sys.argv[1] if len(sys.argv) > 1 else "mcpanel.png"
    app = QGuiApplication([])
    pixmap = QPixmap(256, 256)
    pixmap.fill(QColor("#111827"))

    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    painter.setPen(Qt.PenStyle.NoPen)
    painter.setBrush(QColor("#57F287"))
    painter.drawRoundedRect(QRectF(24, 24, 208, 208), 44, 44)

    font = QFont()
    font.setPixelSize(104)
    font.setBold(True)
    painter.setFont(font)
    painter.setPen(QColor("#111827"))
    painter.drawText(QRectF(24, 24, 208, 208), Qt.AlignmentFlag.AlignCenter, "MC")
    painter.end()

    if not pixmap.save(output, "PNG"):
        print(f"failed to write {output}", file=sys.stderr)
        return 1
    app.quit()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
