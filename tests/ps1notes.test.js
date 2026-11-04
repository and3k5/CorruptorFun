import test from 'node:test';
import assert from 'node:assert/strict';
import { encodeAdpcm, patchSamples } from '../src/lib/ps1custom/sound.js';
import { naturalShift, originalSample, playedAs, renderNote, sampleAddresses, sampleUse } from '../src/lib/ps1custom/notes.js';

// A ROM with a tiny VAB: 1 program, 2 tones, 2 samples (the second loops).
function fakeVabRom() {
  const rom = new Uint8Array(65536);
  const o = 0x1000;
  const v = new DataView(rom.buffer);
  rom.set([0x70, 0x42, 0x41, 0x56], o);
  const ps = 1, vs = 2;
  const tones = o + 32 + 2048, table = tones + ps * 512, body = table + 512;
  const a = encodeAdpcm(Array.from({ length: 2800 }, (_, i) => Math.sin(i / 5) * 8000));
  const b = encodeAdpcm(Array.from({ length: 1400 }, (_, i) => Math.sin(i / 3) * 8000), { loop: true });
  a.fill(0, 0, 16); // starts with a silent block, as real samples often do
  v.setUint32(o + 12, body + a.length + b.length - o, true);
  v.setUint16(o + 18, ps, true);
  v.setUint16(o + 22, vs, true);
  [a, b].forEach((s, i) => v.setUint16(table + (i + 1) * 2, s.length / 8, true));
  [1, 2].forEach((vag, t) => {
    rom[tones + t * 32 + 4] = 60;
    v.setUint16(tones + t * 32 + 22, vag, true);
  });
  rom.set(a, body);
  rom.set(b, body + a.length);
  return { rom, tones, body, sizes: [a.length, b.length] };
}

// Sound RAM with the bank's body uploaded at `at`.
function spuRam(rom, body, size, at) {
  const ram = new Uint8Array(512 * 1024);
  ram.set(rom.subarray(body, body + size), at);
  return ram;
}

const note = (start, pitch, sounded, { min = pitch, max = pitch, on = 0, ended = 1 } = {}) => [
  start, pitch, min, max, sounded, Math.floor((sounded * pitch) / 0x1000), on, ended,
];

test('notes: finds the samples in sound RAM', () => {
  const { rom, body, sizes } = fakeVabRom();
  const ram = spuRam(rom, body, sizes[0] + sizes[1], 0x1010);
  assert.deepEqual(sampleAddresses(rom, ram), { 1: 0x1010, 2: 0x1010 + sizes[0] });
  assert.equal(sampleAddresses(rom, new Uint8Array(512 * 1024)), null);
});

test('notes: typical speed, and the length of sound the longest note uses', () => {
  const { rom, body, sizes } = fakeVabRom();
  const ram = spuRam(rom, body, sizes[0] + sizes[1], 0x2000);
  const log = Uint32Array.from([
    ...note(0x2000, 0x3fff, 30), // a set-up blip: ignored
    ...note(0x2000, 0x800, 44100, { min: 0x700 }), // 1 s at half speed: uses 0.5 s
    ...note(0x2000, 0x1000, 4410), // 0.1 s at 1×
    ...note(0x2000, 0x800, 4410),
    ...note(0x2000 + sizes[0], 0x400, 88200), // 2 s at a quarter: 0.5 s
    ...note(0x2000 + sizes[0], 0x200, 441000, { on: 44100, ended: 0 }), // still on at the end: counts 1 s
    ...note(0x9000, 0x1000, 44100), // something else
  ]);
  const use = sampleUse(rom, ram, log, { until: 88200 });
  assert.deepEqual(Object.keys(use), ['1', '2']);
  const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-3, `${a} ≈ ${b}`);
  assert.equal(use[1].count, 3);
  near(use[1].speed, 0.5);
  near(use[1].longest.held, 1);
  near(use[1].longest.reads, 0.5);
  assert.equal(use[1].speedMin, 0x700 / 0x1000);
  assert.equal(use[1].speedMax, 1);
  near(use[2].longest.held, 2);
  near(use[2].speedMin, 0.125);

  // As recorded: an octave up, so the longest note uses all 1 s of the sound.
  assert.equal(naturalShift(use[1]), 12);
  const p = playedAs(use[1], 12);
  near(p.speed, 1);
  near(p.seconds, 1);
  assert.equal(playedAs(use[2], 60).speed, 4, 'the SPU tops out at 4×');
});

test('notes: renders a note at its speed, looping or running out', () => {
  const pcm = Float32Array.from({ length: 4410 }, () => 0.5); // 0.1 s
  assert.equal(renderNote(pcm, 44100, { speed: 0.5, seconds: 1 }).length, 8818, 'runs out after 0.2 s');
  assert.equal(renderNote(pcm, 44100, { speed: 0.5, seconds: 1, loopStart: 0 }).length, 44100);
  assert.equal(renderNote(pcm, 22050, { speed: 1, seconds: 1 }).length, 8818, 'half the rate: twice as long');
  const { rom } = fakeVabRom();
  assert.equal(originalSample(rom, 1).loopStart, -1);
  assert.equal(originalSample(rom, 2).loopStart, 0);
  assert.equal(originalSample(rom, 2).pcm.length, 1400);
});

test('sound: a speed shift lowers the centre note', () => {
  const { rom, tones } = fakeVabRom();
  const pcm = Float32Array.from({ length: 1000 }, (_, i) => Math.sin(i / 10));
  const r = patchSamples(rom, { 1: { pcm, rate: 44100, shift: 12 } });
  assert.equal(r.used[1].rate, 44100);
  assert.equal(r.rom[tones + 4], 48);
  assert.equal(r.rom[tones + 32 + 4], 60);
});
