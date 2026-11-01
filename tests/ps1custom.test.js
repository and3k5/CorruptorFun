import test from 'node:test';
import assert from 'node:assert/strict';
import { licenseText, layoutLine, advance } from '../src/lib/ps1custom/license.js';
import { imageToLogo, MAX_TRIANGLES, MAX_BYTES } from '../src/lib/ps1custom/logo.js';
import { decodeAdpcm, encodeAdpcm, findVab, patchSamples } from '../src/lib/ps1custom/sound.js';
import { availableLetters, findWordmarks, patchWordmarks } from '../src/lib/ps1custom/wordmark.js';

const width = (s) => [...s].reduce((w, c) => w + advance(c), 0);

test('license text: pieces join up where the BIOS draws them', () => {
  const { text, warnings } = licenseText('Licensed by', 'Sony Computer Entertainment America');
  assert.deepEqual(warnings, []);
  // Long layout (70 characters): line 2 pieces at 121, 309 and 485.
  assert.equal(text.length, 70);
  const pieces = [text.slice(32, 48), text.slice(48, 64), text.slice(64)];
  let x = 121 + width(pieces[0].match(/^ */)[0]);
  const starts = [121, 309, 485];
  pieces.forEach((p, i) => {
    const lead = p.match(/^ */)[0].length;
    if (p.trim()) assert.equal(starts[i] + lead * 4, x, `piece ${i} starts where the last ended`);
    x += width(p.trimEnd()) - lead * 4;
  });
  assert.equal(text.slice(32).replace(/ +/g, ''), 'SonyComputerEntertainmentAmerica');
});

test('license text: short text centres, too long text warns', () => {
  const l = layoutLine('Hi', { starts: [224, 336], caps: [16, 16] });
  assert.ok(l && l.score < 40);
  assert.equal(licenseText('x', 'y'.repeat(60)).warnings.length, 1);
  assert.ok([64, 70].includes(licenseText('Hello', 'World').text.length));
});

function picture(w, h, paint) {
  const data = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) data.set(paint(x, y), (y * w + x) * 4);
  return { width: w, height: h, data };
}

test('logo: a flat picture becomes a small model of lit flat triangles', () => {
  // Left half red, right half transparent.
  const img = picture(16, 8, (x) => (x < 8 ? [255, 0, 0, 255] : [0, 0, 0, 0]));
  const r = imageToLogo(img);
  assert.ok(r.triangles <= 4, `merged into one rectangle, got ${r.triangles}`);
  const v = new DataView(r.tmd.buffer);
  assert.equal(v.getUint32(0, true), 0x41);
  assert.equal(r.tmd[12 + 28 + 7], 0x20); // mode byte
  // Colour is divided by the BIOS's light gain.
  assert.equal(r.tmd[12 + 28 + 4], Math.round(255 / 1.125));
});

test('logo: a busy picture is fitted to the BIOS limits', () => {
  let seed = 1;
  const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) & 255;
  const img = picture(128, 64, () => [rnd(), rnd(), rnd(), 255]);
  const r = imageToLogo(img, { depth: true });
  assert.ok(r.triangles <= MAX_TRIANGLES);
  assert.ok(r.bytes <= MAX_BYTES);
  assert.ok(r.gw >= 4);
});

test('ADPCM: encode and decode keep a sine wave', () => {
  const pcm = Array.from({ length: 2800 }, (_, i) => Math.round(Math.sin(i / 7) * 20000));
  const enc = encodeAdpcm(pcm, { loop: true });
  assert.equal(enc.length, 100 * 16);
  assert.equal(enc[1] & 4, 4); // loop start
  assert.equal(enc[enc.length - 15] & 3, 3); // end + repeat
  const dec = decodeAdpcm(enc);
  let err = 0, pow = 0;
  pcm.forEach((s, i) => {
    err += (s - dec[i]) ** 2;
    pow += s * s;
  });
  assert.ok(10 * Math.log10(pow / err) > 30);
});

// A ROM with a tiny VAB: 1 program, 2 tones, 2 samples.
function fakeVabRom() {
  const rom = new Uint8Array(65536);
  const o = 0x1000;
  const v = new DataView(rom.buffer);
  rom.set([0x70, 0x42, 0x41, 0x56], o);
  const ps = 1, vs = 2;
  const tones = o + 32 + 2048, table = tones + ps * 512, body = table + 512;
  const sizes = [1600, 800];
  v.setUint32(o + 12, body + 2400 - o, true);
  v.setUint16(o + 18, ps, true);
  v.setUint16(o + 22, vs, true);
  sizes.forEach((s, i) => v.setUint16(table + (i + 1) * 2, s / 8, true));
  [1, 2].forEach((vag, t) => {
    rom[tones + t * 32 + 4] = 60; // centre note
    v.setUint16(tones + t * 32 + 22, vag, true);
  });
  const tone = Array.from({ length: 2800 }, (_, i) => Math.sin(i / 5) * 8000);
  rom.set(encodeAdpcm(tone.slice(0, 2800)).subarray(0, 1600), body);
  rom.set(encodeAdpcm(tone.slice(0, 1400)), body + 1600);
  return { rom, tones };
}

test('sound: replacing a sample rewrites the bank and retunes long sounds', () => {
  const { rom, tones } = fakeVabRom();
  const vab = findVab(rom);
  assert.equal(vab.samples.length, 2);
  // 1 s at 44.1 kHz is far more than the 1600 bytes there are: it has to go
  // down to 11 kHz and be cut.
  const pcm = Float32Array.from({ length: 44100 }, (_, i) => Math.sin(i / 10));
  const r = patchSamples(rom, { 1: { pcm, rate: 44100 } });
  assert.equal(r.used[1].rate, 11025);
  assert.ok(r.used[1].cut);
  const after = findVab(r.rom);
  assert.ok(after.samples[0].size <= 1600);
  assert.equal(after.samples[1].offset, after.samples[0].offset + after.samples[0].size);
  assert.deepEqual(r.rom.subarray(after.samples[1].offset, after.samples[1].offset + 800), rom.subarray(vab.samples[1].offset, vab.samples[1].offset + 800));
  assert.equal(r.rom[tones + 4], 60 + 24, 'two octaves up for a quarter of the rate');
  assert.equal(r.rom[tones + 32 + 4], 60, 'the other sample keeps its tuning');
});

// A ROM with a fake "PlayStation" picture: 11 letter blobs and a mark.
function fakeWordmarkRom() {
  const rom = new Uint8Array(32768);
  const v = new DataView(rom.buffer);
  const o = 0x100;
  v.setUint32(o, 0x10, true);
  v.setUint32(o + 4, 8, true);
  v.setUint32(o + 8, 44, true);
  // Palette: 0 = white ink ... 15 = transparent.
  for (let i = 0; i < 16; i++) v.setUint16(o + 20 + i * 2, i === 15 ? 0 : 0x7fff - i * 0x0842, true);
  const p = o + 8 + 44;
  const w = 200, h = 40;
  v.setUint32(p, 12 + (w / 2) * h, true);
  v.setUint16(p + 8, w / 4, true);
  v.setUint16(p + 10, h, true);
  const px = new Uint8Array(w * h).fill(15);
  let x = 2;
  [10, 4, 9, 10, 9, 6, 9, 6, 4, 10, 9, 8].forEach((bw, k) => {
    const top = k === 11 ? 2 : 4, bottom = k === 11 ? 10 : 34;
    for (let y = top; y < bottom; y++) for (let i = 0; i < bw; i++) px[y * w + x + i] = 0;
    x += bw + 3;
  });
  for (let y = 0; y < h; y++) for (let i = 0; i < w; i += 2) rom[p + 12 + y * (w / 2) + i / 2] = px[y * w + i] | (px[y * w + i + 1] << 4);
  return rom;
}

test('wordmarks: finds the letters and rebuilds the picture from them', () => {
  const rom = fakeWordmarkRom();
  const found = findWordmarks(rom);
  assert.equal(found.playstation?.length, 1);
  assert.deepEqual(availableLetters(found.playstation[0]), ['PSailnoty']);
  assert.deepEqual(patchWordmarks(rom, { playstation: ['PlayStation'] }).rom, rom, 'same text, same bytes');
  const r = patchWordmarks(rom, { playstation: ['Sony'] });
  assert.notDeepEqual(r.rom, rom);
  assert.deepEqual(r.notes, []); // S, o, n and y are all there
  const missing = patchWordmarks(rom, { playstation: ['Wii'] });
  assert.match(missing.notes.join(' '), /left out W/);
});
