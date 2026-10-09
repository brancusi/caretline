# caretline

A text-editing engine and a terminal text editor. The engine puts Helix's editing model (a
rope, multi-range selections, transactions, an undo tree, grapheme-correct motion, soft wrap)
inside a strict Elm architecture: one serializable `State`, every input a `Msg`, a pure
`update` that returns `Effect`s, and a pure `view` that draws a grid of cells. No I/O and no
terminal code in the engine, so any session can be saved, driven from a script or an agent,
and replayed exactly.

```sh
curl -fsSL https://caretline.app/install.sh | sh -s -- --demo
```

That installs `caretline` and opens `caretline demo`, the welcome: what caretline is, then each
demo as a chapter (a timed showcase, a hands-on tour, an agent co-editing beside you, layers,
animated scenes) and the commands to take away.

From a checkout, try the **self-running interactive showcase** (not released yet):

```sh
./scripts/demo.sh
```

Twelve timed slides demonstrate real edits, multi-carets, shared views, branded overlays,
checked state/replay invariants, Geist typography, live ASCII scenes and fractional-pixel
motion, ending with the original animated warp logo. `←`/`→` choose slides, `Space` pauses, `q` quits. Best in
Ghostty at 100×30; other terminals use cell overlays. See the
[showcase walkthrough](docs/quickstart.md#caretline-demo-showcase-the-interactive-presentation).

Website and docs: **[caretline.app](https://caretline.app)** · crate: [`caretline`](https://crates.io/crates/caretline) · editor: [`caretline-cli`](https://crates.io/crates/caretline-cli)

## What's here

| Path | What it is |
|---|---|
| [`crates/caretline`](crates/caretline) | The engine (crates.io `caretline`). Its [CHANGELOG](crates/caretline/CHANGELOG.md) is the repo's changelog |
| [`crates/caretline-cli`](crates/caretline-cli) | The `caretline` binary: the interactive editor and the headless tools (crates.io `caretline-cli`) |
| [`crates/caretline-mcp`](crates/caretline-mcp) | The `caretline-mcp` binary: an MCP server so agents edit a live editor beside a person |
| [`docs/`](docs/README.md) | The documentation. The site reads these files in place |
| [`site/`](site/README.md) | caretline.app: landing page, docs, brand, the WASM playground and `install.sh` |

## Use it

```toml
[dependencies]
caretline = "0.4"
```

Start with the [overview](docs/README.md), the [quickstart](docs/quickstart.md) and
[embedding](docs/embedding.md). Build from a checkout with `cargo build --workspace` and test
with `cargo test --workspace` (the toolchain is `stable`, pinned in `mise.toml`).

## Contributing

caretline is developed here, and only here. The flow (also in [AGENTS.md](AGENTS.md)):

1. **Every change lands in this repo first** (brancusi/caretline), by pull request. Engine
   bugs found in a host app such as thc are fixed here, never patched inside the host.
2. **Every PR states its CHANGELOG entry.** A change to the engine's public API files
   (`crates/caretline/src`, the CLI's command line, the MCP server) needs a line under
   **Unreleased** in [`crates/caretline/CHANGELOG.md`](crates/caretline/CHANGELOG.md); CI
   fails without one. Internal-only changes carry the `no-changelog` label.
3. **Every merged change gets a docs-and-site pass, by a dedicated subagent.** After a merge, a
   separate agent (or person) whose only job is the docs reads the change and updates
   `docs/`, the site's pages (`site/src`), the playground (rebuild
   `site/public/wasm/caretline.wasm` with `npm run wasm` when the engine changed) and the
   CHANGELOG, then redeploys caretline.app. If nothing needs changing, it says so on the PR.
   The PR template has the checkbox: "docs and site updated, or N/A because …".
4. **Releases follow semver.** Breaking API changes bump the minor version while we're at 0.x
   (0.3 → 0.4), additions and fixes bump the patch. A release is: move **Unreleased** in the
   CHANGELOG under the new version, bump `crates/*/Cargo.toml`, merge, tag `vX.Y.Z` (the
   [release workflow](.github/workflows/release.yml) builds and publishes the binaries that
   `install.sh` serves), `cargo publish` the crates, and redeploy the site.
5. **Hosts pull a published version.** A host such as thc depends on `caretline = "0.4"` from
   crates.io. For a fix that hasn't been released yet, it pins a git revision of this repo
   with `[patch.crates-io] caretline = { git = "https://github.com/brancusi/caretline", rev = "<sha>" }`
   and drops the patch at the next release. A local path patch is fine for development but is
   never committed.

The site deploys from `site/`:
`npm ci && npm run build && npm run scan && npx wrangler deploy --config ./wrangler.jsonc`
(`npm run scan` fails on anything private in the built site; never skip it).

## History

caretline grew inside [thought-control](https://github.com/brancusi/thought-control) (the
`thc` notes app, its first host) and moved here with its full history on 2026-10-07. Releases
up to 0.3.0 were first published there as `caretline-v*`; `install.sh` still falls back to
them.

## License

MIT ([LICENSE-MIT](LICENSE-MIT)). The files vendored from Helix under
`crates/caretline/src/helix` are MPL-2.0 ([LICENSE-MPL-2.0](LICENSE-MPL-2.0)), file by file;
the engine crate is `MIT AND MPL-2.0`.
