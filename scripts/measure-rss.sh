#!/usr/bin/env bash
# Sample RSS for the Minewrap Tauri process + WebKit helpers.
# Usage: measure-rss.sh <pid> [seconds]
set -euo pipefail
PID="${1:?usage: measure-rss.sh <main-pid> [seconds]}"
SECONDS_TO_SAMPLE="${2:-10}"
echo "Sampling $PID for ${SECONDS_TO_SAMPLE}s ..."
for i in $(seq 1 "$SECONDS_TO_SAMPLE"); do
  vm=$(grep VmRSS "/proc/$PID/status" 2>/dev/null | awk '{print $2}' || echo "?")
  echo "$(date +%H:%M:%S) main: ${vm} kB"
  # WebKit helpers are children of the main process.
  for child in $(pgrep -P "$PID" 2>/dev/null || true); do
    cvm=$(grep VmRSS "/proc/$child/status" 2>/dev/null | awk '{print $2}' || echo "?")
    echo "  helper $child: ${cvm} kB"
  done
  sleep 1
done
echo "Budget: main < 30 MB (30720 kB). See ARCHITECTURE.md."
