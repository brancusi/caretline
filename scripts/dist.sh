#!/bin/sh
# Builds the release binaries: for each target (default: all four), build `caretline`,
# smoke-test it if this machine can run it, and package it the way install.sh expects:
#
#   dist/caretline-<version>-<target>.tar.gz          the binary, licenses and README
#   dist/caretline-<version>-<target>.tar.gz.sha256   its checksum
#
#   scripts/dist.sh                              all four targets
#   scripts/dist.sh x86_64-unknown-linux-musl    just one
#
# A target for this machine's OS builds with cargo; the others cross-build with
# cargo-zigbuild (brew install zig && cargo install --locked cargo-zigbuild). On a Mac that
# is all four. scripts/release.sh and the release workflow both build through this script.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

ALL="aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
targets=${*:-$ALL}

if command -v cargo >/dev/null 2>&1; then cargo=cargo; else cargo="mise exec -- cargo"; fi
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/caretline-cli/Cargo.toml | head -1)
out=${CARGO_TARGET_DIR:-target}
host_os=$(uname -s)
host_arch=$(uname -m)

say() { printf '%s\n' "$*"; }

# Whether this machine can run a binary for target $1 (x86_64 macOS needs Rosetta).
runnable() {
	case "$host_os:$host_arch:$1" in
		Darwin:arm64:aarch64-apple-darwin | Darwin:x86_64:x86_64-apple-darwin) return 0 ;;
		Darwin:arm64:x86_64-apple-darwin) arch -x86_64 /usr/bin/true 2>/dev/null ;;
		Linux:x86_64:x86_64-unknown-linux-musl | Linux:aarch64:aarch64-unknown-linux-musl) return 0 ;;
		*) return 1 ;;
	esac
}

build() {
	case "$host_os:$1" in
		Darwin:*-apple-darwin | Linux:*-linux-*) $cargo build --release --locked -p caretline-cli --target "$1" ;;
		*) $cargo zigbuild --release --locked -p caretline-cli --target "$1" ;;
	esac
}

# The same checks wherever a release is built: the version, and each demo's first frame.
smoke() {
	b=$1
	"$b" --version | grep -qx "caretline $version"
	"$b" demo --help | grep -q 'scenes'
	"$b" demo --snapshot 80x24 | grep -q 'Start here'
	"$b" demo tour --snapshot 80x24 | grep -q 'Welcome to caretline'
	"$b" demo scenes --snapshot 80x24 | grep -q 'C A R E T L I N E'
	"$b" demo agent --headless | grep -q '"ok": true'
}

package() {
	name="caretline-$version-$1"
	rm -rf "dist/$name" "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"
	mkdir -p "dist/$name"
	cp "$2" "dist/$name/caretline"
	cp LICENSE-MIT LICENSE-MPL-2.0 "dist/$name/"
	cp crates/caretline-cli/README.md "dist/$name/README.md"
	tar -C dist -czf "dist/$name.tar.gz" "$name"
	rm -rf "dist/$name"
	(cd dist && if command -v sha256sum >/dev/null 2>&1; then sha256sum "$name.tar.gz"; else shasum -a 256 "$name.tar.gz"; fi >"$name.tar.gz.sha256")
}

for t in $targets; do
	say "== $t"
	build "$t"
	b="$out/$t/release/caretline"
	if runnable "$t"; then
		smoke "$b"
		say "   smoke test passed"
	else
		say "   can't run $t here: built, not smoke-tested ($(file -b "$b" | cut -d, -f1-2))"
	fi
	package "$t" "$b"
	say "   dist/caretline-$version-$t.tar.gz"
done
