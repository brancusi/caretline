#!/bin/sh
# Start the self-running showcase from any working directory. No agent required.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if ! command -v cargo >/dev/null 2>&1; then
    printf '%s\n' 'The showcase needs a Rust toolchain (cargo on PATH).' >&2
    exit 1
fi
exec cargo run --manifest-path "$root/Cargo.toml" -p caretline-cli -- demo showcase "$@"
