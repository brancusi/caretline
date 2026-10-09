#!/bin/sh
# Every change says how to see it: each entry under **Unreleased** in a crate's CHANGELOG
# ends with a `Try it:` line, a one-line command anyone can paste (AGENTS.md, the flow):
#
#   Try it: `curl -fsSL https://caretline.app/install.sh | sh -s -- --ref main -- doctor --keys`
#
# `--ref main` builds the change from source until it's released; scripts/release.sh then
# rewrites it to `--version X.Y.Z`. A change nobody can see (a refactor, a build fix) says
# so: `Try it: none (internal: …)`.
#
#   scripts/check-try.sh          checks every crates/*/CHANGELOG.md
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bad=0
for f in "$root"/crates/*/CHANGELOG.md; do
	out=$(awk '
		/^## Unreleased/ { on = 1; next }
		on && /^## / { exit }
		!on { next }
		/^- / { if (entry != "" && !tried) print entry; entry = $0; tried = 0 }
		/^### / { if (entry != "" && !tried) print entry; entry = ""; tried = 0 }
		/Try it:/ { tried = 1 }
		END { if (entry != "" && !tried) print entry }
	' "$f")
	if [ -n "$out" ]; then
		printf '%s: entries without a "Try it:" line:\n%s\n\n' "${f#"$root"/}" "$out" >&2
		bad=1
	fi
done
[ "$bad" = 0 ] || {
	printf 'check-try: add a `Try it:` command to each entry (see scripts/check-try.sh)\n' >&2
	exit 1
}
echo "check-try: ok"
