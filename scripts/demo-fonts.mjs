// Generate the showcase's small, licensed ASCII outline subsets. No fonts are loaded
// from a person's machine. Runtime/builds need neither Node nor a network connection.
// Prepare a scratch prefix with @fontsource/geist@5.3.0,
// @fontsource/geist-mono@5.3.0 and opentype.js@2.0.0, then:
// node scripts/demo-fonts.mjs /path/to/scratch/node_modules
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const modules = process.argv[2];
if (!modules) throw new Error('usage: node scripts/demo-fonts.mjs NODE_MODULES_DIRECTORY');
const require = createRequire(path.resolve(modules, '../package.json'));
const opentype = require('opentype.js');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'crates/caretline-cli/assets/typography');
fs.mkdirSync(out, { recursive: true });
const round = n => Math.round(n * 1000000) / 1000000;
for (const family of ['geist', 'geist-mono']) {
  const file = require.resolve(`@fontsource/${family}/files/${family}-latin-500-normal.woff`);
  const data = fs.readFileSync(file);
  const font = opentype.parse(data.buffer.slice(data.byteOffset, data.byteOffset + data.length));
  const glyphs = {};
  for (let code = 32; code < 127; code++) {
    const char = String.fromCharCode(code);
    const commands = font.getPath(char, 0, 0, 1).commands.map(c => {
      const keys = { M: ['x', 'y'], L: ['x', 'y'], Q: ['x1', 'y1', 'x', 'y'], C: ['x1', 'y1', 'x2', 'y2', 'x', 'y'], Z: [] }[c.type];
      if (!keys) throw new Error(`unknown path command ${c.type}`);
      return [c.type, ...keys.map(k => round(c[k]))];
    });
    glyphs[char] = { advance: round(font.charToGlyph(char).advanceWidth / font.unitsPerEm), commands };
  }
  fs.writeFileSync(path.join(out, `${family}.json`), JSON.stringify({
    source: `@fontsource/${family}@5.3.0, latin 500 normal`,
    license: 'SIL Open Font License 1.1; see adjacent license file', glyphs,
  }) + '\n');
  const packageRoot = path.dirname(path.dirname(file));
  fs.copyFileSync(path.join(packageRoot, 'LICENSE'), path.join(out, `${family}.LICENSE.txt`));
}
