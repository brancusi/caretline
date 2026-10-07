#!/usr/bin/env bash
# Fails when the engine's public API files changed between two commits without an entry in
# crates/caretline/CHANGELOG.md. Usage: scripts/check-changelog.sh <base> <head>
# LABELS (comma separated) containing no-changelog skips the check, for internal-only changes.
set -euo pipefail
base="$1"; head="${2:-HEAD}"
case ",${LABELS:-}," in *,no-changelog,*) echo "changelog: skipped (no-changelog label)"; exit 0 ;; esac
changed="$(git diff --name-only "$base"..."$head")"
# The public API: the engine's sources (not the vendored Helix internals, not tests), its
# manifest, and the binaries' command lines and protocol.
api="$(echo "$changed" | grep -E '^crates/caretline/(src/|Cargo\.toml$)|^crates/caretline-cli/src/main\.rs$|^crates/caretline-mcp/src/' \
  | grep -vE '^crates/caretline/src/helix/' || true)"
if [[ -z "$api" ]]; then echo "changelog: no public API files changed"; exit 0; fi
if echo "$changed" | grep -qx 'crates/caretline/CHANGELOG.md'; then
  echo "changelog: entry present"; exit 0
fi
echo "These public API files changed without a crates/caretline/CHANGELOG.md entry:" >&2
echo "$api" | sed 's/^/  /' >&2
echo "Add a line under '## Unreleased', or label the PR no-changelog if nothing public changed." >&2
exit 1
