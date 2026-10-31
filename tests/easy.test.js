import test from 'node:test';
import assert from 'node:assert/strict';
import { candidateSettings, score } from '../src/lib/easy.js';
import { mulberry32 } from '../src/lib/corrupt.js';

const grid = (v) => new Uint8Array(16 * 12 * 3).fill(v);
const run = (n, g, loud = 0) => Array.from({ length: n }, () => ({ grid: g, loud }));

test('candidates stay inside the BIOS and use known engines', () => {
  const rand = mulberry32(7);
  for (let i = 0; i < 500; i++) {
    const s = candidateSettings('ps1', 512 * 1024, rand);
    assert.ok(s.start >= 0 && s.end <= 512 * 1024 && s.start < s.end, JSON.stringify(s));
    assert.ok(s.count >= 1);
    assert.match(s.engine, /^(bitflip|random|increment|smear|shift|xor)$/);
  }
});

test('a black, silent boot is rejected', () => {
  const ref = run(36, grid(60), 0.01);
  const m = score(run(36, grid(0)), ref, 36);
  assert.equal(m.alive, false);
  assert.equal(m.ok, false);
});

test('a boot identical to the clean one is not peculiar', () => {
  const ref = run(36, grid(60), 0.01);
  const m = score(run(36, grid(60), 0.01), ref, 36);
  assert.equal(m.alive, true);
  assert.equal(m.ok, false);
});

test('a boot that shows something different passes', () => {
  const ref = run(36, grid(60), 0.01);
  const m = score(run(36, grid(140), 0.01), ref, 36);
  assert.equal(m.ok, true);
});

test('a boot that died early counts the missing time as dead', () => {
  const ref = run(36, grid(60), 0.01);
  const m = score(run(3, grid(140)), ref, 36);
  assert.equal(m.alive, false);
});
