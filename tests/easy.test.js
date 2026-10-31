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

test('filters: no black screen and must have sound', async () => {
  const { accepts } = await import('../src/lib/easy.js');
  const ref = run(30, grid(60), 0.01);
  const refM = score(ref, ref, 30);
  const silent = score(run(30, grid(140)), ref, 30);
  const halfBlack = score([...run(10, grid(140), 0.01), ...run(20, grid(0), 0.01)], ref, 30);
  assert.equal(accepts(silent, refM, {}), true);
  assert.equal(accepts(silent, refM, { sound: true }), false);
  assert.equal(accepts(halfBlack, refM, {}), true);
  assert.equal(accepts(halfBlack, refM, { picture: true }), false);
});

test('cached picks skip used and filtered-out candidates', async () => {
  const { pickCached } = await import('../src/lib/easy.js');
  const ref = run(30, grid(60), 0.01);
  const cache = { ref, refMetrics: score(ref, ref, 30), candidates: [] };
  const add = (checks, used = false) => cache.candidates.push({ settings: { seed: cache.candidates.length }, metrics: score(checks, ref, 30), used });
  add(run(30, grid(140), 0.01), true); // best, but used
  add(run(30, grid(140))); // silent
  add(run(30, grid(0))); // dead
  assert.equal(pickCached(cache, {}).settings.seed, 1);
  assert.equal(pickCached(cache, { sound: true }), null);
});
