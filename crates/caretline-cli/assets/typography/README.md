# Showcase typography outlines

`geist.json` and `geist-mono.json` are small ASCII outline subsets of the site's Geist and
Geist Mono faces (Latin, weight 500, normal), from `@fontsource/geist@5.3.0` and
`@fontsource/geist-mono@5.3.0`. The original font software is by the Geist Project Authors:
https://github.com/vercel/geist-font. The adjacent SIL Open Font License files apply to these
outlines. The surrounding Rust code is MIT licensed. The standalone binary also embeds
both complete licenses: `caretline demo showcase --font-licenses` prints them.

The showcase draws the outlines with tiny-skia, so no runtime font installation, Node,
platform-specific font discovery or network fetch is needed. It does not change Ghostty's
terminal font. These are illustrative vector text samples, not a general Unicode text
shaping implementation.

To regenerate, install those pinned packages and `opentype.js@2.0.0` in a scratch directory,
then run `node scripts/demo-fonts.mjs /path/to/scratch/node_modules` from the repository.
The generator writes deterministic, text-only JSON and copies the upstream license notices.
