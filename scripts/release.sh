#!/bin/sh
# Releases caretline from this machine, start to finish:
#
#   scripts/release.sh 0.4.3              the whole release
#   scripts/release.sh 0.4.3 --dry-run    steps 1–4 only: nothing is pushed or published
#
#   1. check   on main, clean, level with origin; the version is new; every entry has Try it
#   2. bump    caretline-cli to the version; its CHANGELOG's Unreleased becomes the version,
#              and every `--ref main` Try-it line becomes `--version X.Y.Z`
#   3. test    fmt, the workspace tests, preflight
#   4. build   the four binaries into dist/ (scripts/dist.sh)
#   5. land    a release PR, merged; the tag vX.Y.Z pushed
#   6. ship    the GitHub release with the binaries (what install.sh reads), every crate whose
#              version isn't on crates.io yet, and the site
#
# GitHub CI is not a step. The tag starts .github/workflows/release.yml, which rebuilds each
# target on its own runner in the background and reports if one fails; nothing waits for it.
# Bump the engine or the other crates in the PR that changes them: step 6 publishes any
# version crates.io doesn't have, in dependency order.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

say() { printf '%s\n' "$*"; }
step() { printf '\n\033[1m%s\033[0m\n' "$*"; }
die() { printf 'release: %s\n' "$*" >&2; exit 1; }

v=${1:-}
dry=0
[ "${2:-}" = "--dry-run" ] && dry=1
printf '%s' "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || die "usage: scripts/release.sh X.Y.Z [--dry-run]"
if command -v cargo >/dev/null 2>&1; then cargo=cargo; else cargo="mise exec -- cargo"; fi
cl=crates/caretline-cli/CHANGELOG.md

step "1. check"
[ "$(git branch --show-current)" = main ] || die "not on main"
[ -z "$(git status --porcelain)" ] || die "the working tree isn't clean"
git fetch -q origin
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main isn't level with origin/main"
git rev-parse -q --verify "refs/tags/v$v" >/dev/null && die "v$v is already tagged"
grep -q '^## Unreleased' "$cl" || die "$cl has no Unreleased section"
scripts/check-try.sh >/dev/null || die "every Unreleased entry needs a Try it: line"
say "   main at $(git rev-parse --short HEAD), releasing $v"

step "2. bump"
git switch -q -c "release/$v"
sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$v\"/" crates/caretline-cli/Cargo.toml
rm crates/caretline-cli/Cargo.toml.bak
today=$(date +%Y-%m-%d)
awk -v h="## $v — $today" '!done && /^## Unreleased/ { print; print ""; print h; done=1; next } { print }' "$cl" >"$cl.new"
mv "$cl.new" "$cl"
# Try-it lines built main from source until now; from here they install this release.
for f in crates/*/CHANGELOG.md; do
	sed -i.bak "s/install\.sh | sh -s -- --ref main --/install.sh | sh -s -- --version $v --/" "$f"
	rm "$f.bak"
done
$cargo metadata -q --format-version 1 >/dev/null # Cargo.lock takes the new version
say "   caretline-cli $v, CHANGELOG dated $today"

step "3. test"
$cargo fmt --all --check
$cargo test -q --workspace --locked
git add crates/caretline-cli/Cargo.toml crates/*/CHANGELOG.md Cargo.lock
scripts/preflight.sh --staged

step "4. build"
scripts/dist.sh

if [ "$dry" = 1 ]; then
	say ""
	say "dry run: nothing pushed or published; the bump is undone. dist/ has the binaries."
	git restore --staged . && git checkout -q -- . && git switch -q main && git branch -q -D "release/$v"
	exit 0
fi

step "5. land"
git commit -q -m "release: caretline-cli $v"
git push -q -u origin "release/$v"
gh pr create --base main --head "release/$v" --label no-changelog \
	--title "release: caretline-cli $v" --body "Release $v, made by scripts/release.sh."
gh pr merge "release/$v" --merge --delete-branch
git switch -q main
git pull -q --ff-only origin main
git tag -a "v$v" -m "caretline-cli $v"
git push -q origin "v$v"
say "   tagged v$v at $(git rev-parse --short HEAD)"

step "6. ship"
notes=$(awk -v h="## $v " 'index($0, h) == 1 { on=1; next } on && /^## / { exit } on { print }' "$cl")
gh release create "v$v" dist/caretline-"$v"-*.tar.gz dist/caretline-"$v"-*.tar.gz.sha256 \
	--title "caretline $v" --notes "$notes" --latest --verify-tag
say "   GitHub release v$v published"
for crate in caretline caretline-layers caretline-tour caretline-mcp caretline-cli; do
	cv=$(sed -n 's/^version = "\(.*\)"/\1/p' "crates/$crate/Cargo.toml" | head -1)
	code=$(curl -s -o /dev/null -w '%{http_code}' -A 'caretline release.sh' "https://crates.io/api/v1/crates/$crate/$cv")
	if [ "$code" = 200 ]; then
		say "   $crate $cv: already on crates.io"
	else
		$cargo publish -q -p "$crate"
		say "   $crate $cv: published"
	fi
done
(cd site && npm ci --silent && npm run build && npm run scan && npx wrangler deploy --config ./wrangler.jsonc)
say ""
say "released $v: https://github.com/brancusi/caretline/releases/tag/v$v"
