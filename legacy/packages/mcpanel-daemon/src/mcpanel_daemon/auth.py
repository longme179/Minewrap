"""Bearer-token auth with constant-time comparison."""

from __future__ import annotations

import hmac


class TokenAuth:
    def __init__(self, token: str) -> None:
        self._expected = token.encode("utf-8")

    def verify_token(self, token: str | None) -> bool:
        if not token:
            return False
        return hmac.compare_digest(token.encode("utf-8"), self._expected)

    def verify_header(self, header: str | None) -> bool:
        if not header:
            return False
        scheme, _, credentials = header.partition(" ")
        if scheme.lower() != "bearer" or not credentials.strip():
            return False
        return self.verify_token(credentials.strip())
