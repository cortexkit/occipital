// Node's WHATWG URL is an independent browser-serialization oracle, not the
// source of any expected string. This tool checks the committed references.
import { readFileSync } from 'node:fs';

const vectors = JSON.parse(readFileSync(new URL('../tests/vectors/urls.json', import.meta.url), 'utf8'));
const expand = value => typeof value === 'string'
  ? value : value.prefix + value.unit.repeat(value.count) + value.suffix;
let count = 0;
for (const vector of vectors.addresses) {
  if (vector.canonical === undefined) continue;
  const expected = expand(vector.whatwg);
  // Only the canonical URL is opened. WHATWG need not serialize the original
  // input identically (for example it preserves lowercase percent escapes).
  const actual = new URL(expand(vector.canonical)).href;
  if (actual !== expected) throw new Error(`${vector.name}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
  count++;
}
console.log(`${process.version}: checked ${count} accepted vectors against WHATWG (canonical fixed points)`);
