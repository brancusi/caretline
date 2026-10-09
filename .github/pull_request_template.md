## What changed

<!-- One or two sentences. Link the issue if there is one. -->

## Try it

<!-- One command anyone can paste to see this change (AGENTS.md, the flow), e.g.
`curl -fsSL https://caretline.app/install.sh | sh -s -- --ref main -- doctor --keys`
or: none (internal: why) -->

Try it: ``

## Checklist

- [ ] Tests pass locally (`cargo test --workspace`)
- [ ] The CHANGELOG entry under **Unreleased** ends with the same `Try it:` line (`scripts/check-try.sh`)
- [ ] `crates/caretline/CHANGELOG.md` has an entry under **Unreleased** (or the PR is labelled `no-changelog`: nothing public changed)
- [ ] Docs and site updated by the docs-and-site pass (`docs/`, `site/` pages, the playground's `site/public/wasm/caretline.wasm`, CHANGELOG), or N/A because: <!-- why -->
- [ ] Hosts affected (thc or others) are named here, with the version or rev they should pull
