"""TokenAuth: bearer parsing and constant-time comparison."""

from __future__ import annotations

from mcpanel_daemon.auth import TokenAuth


def test_verify_token() -> None:
    auth = TokenAuth("s3cret")
    assert auth.verify_token("s3cret") is True
    assert auth.verify_token("wrong") is False
    assert auth.verify_token("") is False
    assert auth.verify_token(None) is False


def test_verify_header() -> None:
    auth = TokenAuth("s3cret")
    assert auth.verify_header("Bearer s3cret") is True
    assert auth.verify_header("bearer s3cret") is True  # scheme is case-insensitive
    assert auth.verify_header("Bearer  s3cret ") is True  # tolerant of whitespace
    assert auth.verify_header("Basic s3cret") is False
    assert auth.verify_header("Bearer wrong") is False
    assert auth.verify_header("Bearer") is False
    assert auth.verify_header("") is False
    assert auth.verify_header(None) is False
