import { test } from 'node:test';
import assert from 'node:assert/strict';
import { corrupt, ENGINES } from '../src/lib/corrupt.js';
import { detectConsole, ps1Region } from '../src/lib/consoles.js';

const source = Uint8Array.from({ length: 65536 }, (_, i) => (i * 31) & 0xff);

test('every engine is deterministic and leaves the source alone', () => {
  for (const engine of Object.keys(ENGINES)) {
    const copy = source.slice();
    const a = corrupt(source, { engine, seed: 42, count: 50, amount: 3 });
    const b = corrupt(source, { engine, seed: 42, count: 50, amount: 3 });
    assert.deepEqual(a.data, b.data, engine);
    assert.deepEqual(source, copy, engine);
    assert.ok(a.hits.length > 0, `${engine} changed something`);
  }
});

test('corruption stays inside the range', () => {
  const { hits } = corrupt(source, { engine: 'random', seed: 7, start: 1000, end: 2000, count: 500 });
  assert.ok(hits.every((h) => h >= 1000 && h < 2000));
  const smear = corrupt(source, { engine: 'smear', seed: 7, start: 1000, end: 2000, count: 50, amount: 400 });
  assert.ok(smear.hits.every((h) => h >= 1000 && h < 2000));
});

test('every-N mode spaces hits', () => {
  const { hits } = corrupt(source, { engine: 'xor', amount: 0xff, mode: 'every', every: 1000, seed: 1 });
  assert.ok(hits.length === 65 || hits.length === 66);
  for (let i = 1; i < hits.length; i++) assert.equal(hits[i] - hits[i - 1], 1000);
});

test('console detection by size and content', () => {
  const ps1 = new Uint8Array(512 * 1024);
  ps1.set(new TextEncoder().encode('Sony Computer Entertainment Inc.'), 100);
  assert.equal(detectConsole(ps1), 'ps1');
  assert.equal(detectConsole(new Uint8Array(4 * 1024 * 1024)), 'ps2');
  assert.equal(detectConsole(new Uint8Array(256 * 1024)), null);
});

test('PS1 region comes from the BIOS version string', () => {
  const bios = new Uint8Array(1024);
  bios.set(new TextEncoder().encode('System ROM Version 4.1 12/16/97 A'), 100);
  assert.equal(ps1Region(bios), 'A');
  assert.equal(ps1Region(new Uint8Array(1024)), 'E');
  // An earlier version string without a region doesn't hide the real one.
  const two = new Uint8Array(1024);
  two.set(new TextEncoder().encode('System ROM Version 1.0'), 10);
  two.set(new TextEncoder().encode('System ROM Version 2.2 12/04/95 J'), 200);
  assert.equal(ps1Region(two), 'I');
});
