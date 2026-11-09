"""Log hygiene before anything leaves the daemon (Discord).

Order: strip ANSI colour codes, redact the Minecraft login form
``Alice[/1.2.3.4:5678]``, redact bare IPv4/IPv6 addresses, apply user-defined
regexes, then truncate long lines.

Known limitation (documented in docs/discord.md): exotic IPv6 spellings may
slip through; operators can add their own ``redact_regexes``.
"""

from __future__ import annotations

import re

ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")

_LOGIN_FORM_RE = re.compile(r"\[/(?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?\]")
_IPV4_RE = re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])")
# IPv6: require a hex letter in the address (or the ::/::1 forms) so plain
# "12:34:56" log timestamps are not redacted. Exotic all-decimal IPv6 spellings
# are a documented limitation.
_IPV6_RE = re.compile(
    r"(?<![0-9A-Za-z:])(?=[0-9A-Fa-f:]*[A-Fa-f])(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}"
    r"(?![0-9A-Za-z:])"
    r"|(?<![0-9A-Za-z:])::1?(?![0-9A-Za-z:])"
)

REDACTED = "[redacted]"
MAX_LINE_CHARS = 300


def strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text)


def compile_patterns(patterns: list[str]) -> list[re.Pattern[str]]:
    return [re.compile(pattern) for pattern in patterns]


def redact_line(
    line: str,
    extra_patterns: list[re.Pattern[str]] | None = None,
    max_chars: int = MAX_LINE_CHARS,
) -> str:
    text = strip_ansi(line)
    text = _LOGIN_FORM_RE.sub("[/redacted]", text)
    text = _IPV4_RE.sub(REDACTED, text)
    text = _IPV6_RE.sub(REDACTED, text)
    for pattern in extra_patterns or []:
        text = pattern.sub(REDACTED, text)
    text = text.replace("\r", " ").replace("\t", "  ").strip()
    if max_chars > 0 and len(text) > max_chars:
        text = text[: max_chars - 1] + "…"
    return text
