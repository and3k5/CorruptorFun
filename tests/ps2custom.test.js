import test from 'node:test';
import assert from 'node:assert/strict';
import { compress, decompress } from '../src/lib/ps2custom/lz.js';
import { archiveFiles, fileData, readDir, rebuildArchive, replaceFile } from '../src/lib/ps2custom/romfs.js';
import { SCE, stripToTexture, textureToStrip } from '../src/lib/ps2custom/index.js';

// All data here is made up: no BIOS is needed (or included).

const ALIGN = (n) => (n + 15) & ~15;

// A file table image: RESET, ROMDIR (the table), then the given files.
function image(files, total) {
  const all = [{ name: 'RESET', data: new Uint8Array(32).fill(0xaa) }, { name: 'ROMDIR', data: null }, ...files];
  const dirSize = (all.length + 1) * 16;
  all[1].data = new Uint8Array(dirSize);
  const size = total ?? all.reduce((s, f) => s + ALIGN(f.data.length), 0);
  const out = new Uint8Array(size).fill(0);
  let off = 0;
  const dirOff = ALIGN(all[0].data.length);
  all.forEach((f, k) => {
    if (f.name !== 'ROMDIR') out.set(f.data, off);
    const e = dirOff + k * 16;
    for (let i = 0; i < f.name.length; i++) out[e + i] = f.name.charCodeAt(i);
    new DataView(out.buffer).setUint32(e + 12, f.data.length, true);
    off += ALIGN(f.data.length);
  });
  return out;
}

function noise(n, seed) {
  const out = new Uint8Array(n);
  let x = seed;
  for (let i = 0; i < n; i++) {
    x = (Math.imul(x, 1103515245) + 12345) >>> 0;
    // Mostly runs and repeats, like the pictures, with some noise.
    out[i] = i % 97 < 60 ? (i >> 5) & 255 : x >>> 24;
  }
  return out;
}

test('lz: round trip of runs, repeats, noise and edge sizes', () => {
  for (const data of [new Uint8Array(0), Uint8Array.of(7), Uint8Array.of(1, 2, 3), new Uint8Array(5000), noise(40000, 1), noise(70000, 9)]) {
    const packed = compress(data);
    assert.deepEqual(decompress(packed), data);
  }
  // Runs pack small.
  assert.ok(compress(new Uint8Array(32768)).length < 32768 / 8);
});

test('romfs: unchanged archive rebuilds byte for byte', () => {
  const arc = image([
    { name: 'A', data: noise(100, 2) },
    { name: 'B', data: noise(37, 3) },
  ]);
  const trimmed = arc.subarray(0, arc.length - ALIGN(37) + 37);
  assert.deepEqual(rebuildArchive(trimmed, {}), trimmed);
  const changed = rebuildArchive(trimmed, { A: noise(300, 4) });
  const files = archiveFiles(changed);
  assert.deepEqual(files.A, noise(300, 4));
  assert.deepEqual(files.B, noise(37, 3));
});

test('romfs: a grown file takes room from the next padding, later files stay put', () => {
  const rom = image([
    { name: 'TEX', data: noise(200, 5) },
    { name: 'MID', data: noise(50, 6) },
    { name: '-', data: new Uint8Array(512).fill(0xff) },
    { name: 'FIXED', data: noise(64, 7) },
  ]);
  const before = readDir(rom).find((f) => f.name === 'FIXED').offset;
  const out = replaceFile(rom, 'TEX', noise(400, 8));
  assert.equal(out.length, rom.length);
  assert.deepEqual(fileData(out, 'TEX'), noise(400, 8));
  assert.deepEqual(fileData(out, 'MID'), noise(50, 6));
  const fixed = readDir(out).find((f) => f.name === 'FIXED');
  assert.equal(fixed.offset, before);
  assert.deepEqual(fileData(out, 'FIXED'), noise(64, 7));
  // Shrinking works too, and too big says by how much.
  assert.deepEqual(fileData(replaceFile(rom, 'TEX', noise(20, 9)), 'MID'), noise(50, 6));
  assert.throws(() => replaceFile(rom, 'TEX', new Uint8Array(2000)), (e) => e.short > 0);
});

test('SCE strip folds into the texture and back', () => {
  const strip = noise(SCE.strip * SCE.rows, 11);
  const tex = stripToTexture(strip);
  assert.equal(tex.length, SCE.width * SCE.height);
  assert.deepEqual(textureToStrip(tex), strip);
});
