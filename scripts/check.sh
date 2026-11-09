#!/usr/bin/env bash
# Full check suite (goal §9.1): rustfmt, clippy (deny warnings),
# cargo test, svelte-check, lint, frontend tests. Steps that cannot
# run are SKIPPED with a clear name (not treated as failures), so
# partial toolchains still produce an honest report. Any ran step
# that fails => exit 1.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export PATH="/home/kimhoanglong/.npm-global/bin:$PATH"

skipped=()
failed=()
run() {
  local name="$1"; shift
  if command -v "$1" >/dev/null 2>&1 || [ "$1" = "cargo" ]; then
    echo "== $name"
    if "$@"; then echo "ok: $name"; else failed+=("$name"); echo "FAIL: $name"; fi
  else
    echo "skip: $name ($1 not found)"
    skipped+=("$name")
  fi
}

# rustfmt / clippy are separate cargo subcommands; check both.
if command -v cargo >/dev/null 2>&1 && cargo fmt --version >/dev/null 2>&1; then
  if cargo fmt --all -- --check; then echo "ok: rustfmt"; else failed+=("rustfmt"); fi
else
  echo "skip: rustfmt (rustfmt not installed; 'rustup component add rustfmt')"
  skipped+=("rustfmt")
fi

if command -v cargo >/dev/null 2>&1 && cargo clippy --version >/dev/null 2>&1; then
  if cargo clippy --workspace --all-targets -- -D warnings; then echo "ok: clippy"; else failed+=("clippy"); fi
else
  echo "skip: clippy (cargo-clippy not installed; 'rustup component add clippy')"
  skipped+=("clippy")
fi

run "cargo test" cargo test --workspace
run "svelte-check" sh -c 'cd ui && pnpm run check'
run "lint" sh -c 'cd ui && pnpm lint'
run "frontend tests" sh -c 'cd ui && pnpm test'

echo "== skipped: ${skipped[*]:-none}"
if [ "${#failed[@]}" -gt 0 ]; then
  echo "check.sh FAILED: ${failed[*]}" >&2
  exit 1
fi
echo "check.sh: all ran steps green"
