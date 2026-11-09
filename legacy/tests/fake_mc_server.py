#!/usr/bin/env python3
"""Fake Minecraft Java server for process-manager tests.

Prints realistic console lines (boot, done-line, a login line containing an IP
that Phase 4 redaction tests consume), reads commands from stdin, and exits
cleanly on ``stop``. Never used against a real server.

Modes:
  --crash during-start   exit 1 before the done line
  --crash after-done     exit 1 shortly after the done line
  --ignore-stop          ignore the ``stop`` stdin command
  --ignore-sigterm       ignore SIGTERM (forces SIGKILL escalation)
"""

from __future__ import annotations

import argparse
import signal
import sys
import time
from datetime import datetime


def _ts() -> str:
    return datetime.now().strftime("[%H:%M:%S]")


def _log(message: str, thread: str = "Server thread/INFO") -> None:
    print(f"{_ts()} [{thread}]: {message}", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description="fake Minecraft server")
    parser.add_argument("--done-delay", type=float, default=0.05)
    parser.add_argument("--crash", choices=["none", "during-start", "after-done"], default="none")
    parser.add_argument("--crash-delay", type=float, default=0.1)
    parser.add_argument("--ignore-stop", action="store_true")
    parser.add_argument("--ignore-sigterm", action="store_true")
    args = parser.parse_args()

    if args.ignore_sigterm:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)

    print(f"{_ts()} [main/INFO]: Starting minecraft server version 1.21.4", flush=True)
    print(f"{_ts()} [main/WARN]: [FakeServer] test mode", flush=True)

    if args.crash == "during-start":
        time.sleep(args.crash_delay)
        _log("Encountered an unexpected exception", "Server thread/ERROR")
        return 1

    _log('Preparing level "world"')
    time.sleep(args.done_delay)
    _log('Done (1.234s)! For help, type "help" or "?"')
    print(
        f"{_ts()} [User Authenticator #1/INFO]: UUID of player Alice is "
        "069a79f4-44e9-4726-a5be-fca90e38aaf5",
        flush=True,
    )
    _log("Alice[/192.168.50.7:54321] logged in with entity id 42 at (10.5, 64.0, -3.25)")
    print(f"{_ts()} [Server thread/INFO]: <Alice> hello world", flush=True)

    if args.crash == "after-done":
        time.sleep(args.crash_delay)
        _log("Simulated crash", "Server thread/ERROR")
        return 1

    for raw in sys.stdin:
        command = raw.strip()
        if not command:
            continue
        if command == "stop" and not args.ignore_stop:
            _log("Stopping the server")
            _log("Stopping server")
            return 0
        if command == "crash":
            _log("Simulated crash by command", "Server thread/ERROR")
            return 1
        _log(f"Executing command: {command}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
