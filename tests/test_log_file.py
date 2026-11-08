"""RotatingLogFile: appends, rotation, backups."""

from __future__ import annotations

from pathlib import Path

from mcpanel_daemon.log_file import RotatingLogFile


def test_writes_lines_and_creates_parent_dirs(tmp_path: Path) -> None:
    path = tmp_path / "nested" / "server.log"
    log = RotatingLogFile(path, max_bytes=1_000_000, backups=3)
    log.write("first line")
    log.write("second line")
    log.close()
    assert path.read_text() == "first line\nsecond line\n"


def test_rotation_moves_files_and_keeps_backup_count(tmp_path: Path) -> None:
    path = tmp_path / "server.log"
    log = RotatingLogFile(path, max_bytes=40, backups=2)
    for i in range(10):
        log.write(f"line {i:02d} xxxxx")  # 14 bytes + newline
    log.close()

    assert path.exists()
    assert path.with_name("server.log.1").exists()
    assert path.with_name("server.log.2").exists()
    assert not path.with_name("server.log.3").exists()


def test_zero_backups_recreates_after_rotation(tmp_path: Path) -> None:
    path = tmp_path / "server.log"
    log = RotatingLogFile(path, max_bytes=30, backups=0)
    for i in range(5):
        log.write(f"line {i} xxxxxxxx")
    log.close()

    assert not path.with_name("server.log.1").exists()
    assert path.exists()  # recreated by the last write after rotation
