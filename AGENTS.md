# AGENTS.md

Guidance for AI agents (and people) working on caretline. `CLAUDE.md` points here.

## The repo

- `crates/caretline`: the engine. Pure: no I/O, no terminal, no async, no host crate.
  `tests/scope.rs` fails if its sources name a host concept (task, status, journal, vault,
  thc, …): what a line *means* belongs to the host, added through `Host` commands, input rules,
  mark payloads and decorations (docs/embedding.md).
- `crates/caretline-cli`: the `caretline` binary (editor, demos, headless tools, the socket).
- `crates/caretline-mcp`: the MCP server.
- `docs/`: the documentation, read in place by the site. Links to code use `../crates/…`.
- `site/`: caretline.app (Astro, Cloudflare Worker `caretline`), the WASM playground in
  `site/wasm`, and `site/public/install.sh`.
- Build and test: `cargo build --workspace`, `cargo test --workspace` (`mise exec -- cargo …`
  if cargo isn't on PATH). Behaviour changes come with goldens (docs/testing.md).

## The flow: every change, in order

1. **Land it here first.** Every change to the engine, the CLI, the MCP server, the docs or
   the site is a PR to brancusi/caretline. Never patch caretline inside a host repo (such as
   brancusi/thought-control); an engine bug found there is fixed here, then pulled in.
2. **Write the CHANGELOG line in the same PR.** Public API changes go under **Unreleased** in
   `crates/caretline/CHANGELOG.md` (CI's `changelog` job enforces it for the API files;
   `no-changelog` label for internal-only changes).
3. **After every merge, run a docs-and-site pass with a dedicated subagent.** Spawn a separate
   agent whose only task is documentation for that merge. Give it the PR (number and diff). It:
   - updates `docs/*.md` where behaviour, API, keys, messages, protocol or CLI changed;
   - updates the site's pages in `site/src` (the landing page's claims and code samples, the
     docs sidebar in `site/src/lib/docs.ts` if a page was added);
   - rebuilds the playground when the engine changed (`cd site && npm run wasm`, needs
     `rustup target add wasm32-unknown-unknown`) and commits `site/public/wasm/caretline.wasm`;
   - checks the CHANGELOG entry reads well;
   - builds, scans and redeploys the site:
     `cd site && npm ci && npm run build && npm run scan && npx wrangler deploy --config ./wrangler.jsonc`;
   - opens its own PR (or says "no docs change needed" on the merged PR) and links it.
   The pass is not optional and not folded into the code PR's author: a fresh agent reads the
   change the way a reader of the docs would.
4. **Release with semver.** At 0.x, a breaking change bumps the minor (0.3 → 0.4); additions
   and fixes bump the patch. Steps: move Unreleased under the version in the CHANGELOG, bump
   the versions in `crates/*/Cargo.toml` (and `caretline-cli`'s dependency on `caretline`),
   merge, tag `vX.Y.Z` and push the tag (`.github/workflows/release.yml` builds the four
   targets and publishes the release the installer reads), `cargo publish -p caretline`, then
   `-p caretline-cli`, then redeploy the site. Releasing is the maintainer's call.
5. **Hosts pull from here.** A host depends on a published version (`caretline = "0.3"`). A fix
   still in flight is pinned by git revision in the host's workspace `Cargo.toml`:

   ```toml
   [patch.crates-io]
   caretline = { git = "https://github.com/brancusi/caretline", rev = "<full sha>" }
   ```

   and the patch is removed once a release contains the fix. While developing against a local
   checkout, `caretline = { path = "../caretline/crates/caretline" }` under `[patch.crates-io]`
   is fine but never committed.

## Rules

- **No orphaned work.** Work lives on a branch pushed to this repo with a PR, or it doesn't
  exist. Don't leave caretline commits on a host repo's branches.
- **No private material** in the repo or the site: home paths, user names, emails, internal
  repo names or claude.ai links. `site/scripts/scan.sh` checks the built site; check commits
  by eye.
- **Keep the engine standalone:** no new dependency on a terminal, async runtime or host crate
  (CI checks `cargo tree -p caretline`).
