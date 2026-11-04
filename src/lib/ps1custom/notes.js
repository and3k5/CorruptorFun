// How the BIOS plays each boot sample, worked out from a headless boot: the
// SPU core logs every note (see emu/ps1/src/spu.rs), and we match the notes'
// start addresses to the samples of the VAB in sound RAM.
//
// What matters for a replacement sound is how fast the BIOS plays it and for
// how long. At pitch 0x1000 the SPU reads 44,100 samples a second; the boot
// jingles play most notes lower than that, so the sound is slowed down (and
// deepened), and a sound shorter than what the longest note reads runs out
// before the note ends.

import { decodeAdpcm, findVab } from './sound.js';

const RATE = 44100;
const FIELDS = 8; // u32s per logged note

/** Where each VAB sample (1-based index) starts in sound RAM, or null. */
export function sampleAddresses(rom, spuRam, vab = findVab(rom)) {
  if (!vab) return null;
  // Find the bank's body by a distinctive piece of one sample (many samples
  // start with a silent block, so skip those).
  for (const s of vab.samples) {
    for (let at = s.offset; at + 64 <= s.offset + s.size; at += 16) {
      const key = rom.subarray(at, at + 64);
      if (key.every((b, k) => k % 16 < 2 || b === 0)) continue; // silent block
      const found = indexOf(spuRam, key);
      if (found < 0) return null;
      const base = found - (at - vab.body);
      if (base < 0) return null;
      const out = {};
      vab.samples.forEach((x) => (out[x.index] = base + (x.offset - vab.body)));
      return out;
    }
  }
  return null;
}

function indexOf(hay, needle) {
  const first = needle[0];
  for (let i = 0; i + needle.length <= hay.length; i += 8) {
    if (hay[i] !== first) continue;
    let k = 1;
    while (k < needle.length && hay[i + k] === needle[k]) k++;
    if (k === needle.length) return i;
  }
  return -1;
}

/**
 * Sums up the logged notes per sample. `notes` is the core's flat Uint32Array
 * log; notes still sounding at `until` (SPU clock: the end of the boot
 * sound) count up to then. Returns, per sample index:
 * { count, speed, speedMin, speedMax, longest: { speed, held, reads } }
 * where speed is the typical (median) note's speed (1 = as recorded), and
 * `longest` is the note that reads the most of the sound: its speed, how
 * long it sounds and how much sound it reads (both in seconds).
 */
export function sampleUse(rom, spuRam, notes, { until = null } = {}) {
  const vab = findVab(rom);
  const addr = sampleAddresses(rom, spuRam, vab);
  if (!addr) return null;
  const byAddr = new Map(Object.entries(addr).map(([i, a]) => [a, +i]));
  const per = {};
  for (let o = 0; o + FIELDS <= notes.length; o += FIELDS) {
    const i = byAddr.get(notes[o]);
    let sounded = notes[o + 4];
    if (until !== null) sounded = Math.min(sounded, Math.max(0, until - notes[o + 6]));
    // Skip blips (the BIOS keys every voice on briefly while setting up).
    if (!i || sounded < RATE / 50) continue;
    const speed = notes[o + 5] / notes[o + 4];
    (per[i] ||= []).push({ speed, held: sounded / RATE, reads: (sounded * speed) / RATE, lo: notes[o + 2] / 0x1000, hi: notes[o + 3] / 0x1000 });
  }
  const out = {};
  for (const [i, list] of Object.entries(per)) {
    const speeds = list.map((n) => n.speed).sort((a, b) => a - b);
    const longest = list.reduce((a, b) => (b.reads > a.reads ? b : a));
    out[i] = {
      count: list.length,
      speed: speeds[Math.floor((speeds.length - 1) / 2)],
      speedMin: Math.min(...list.map((n) => n.lo)),
      speedMax: Math.max(...list.map((n) => n.hi)),
      longest: { speed: longest.speed, held: longest.held, reads: longest.reads },
    };
  }
  return out;
}

/** The speed factor a shift of `semitones` gives. */
export const shiftFactor = (semitones = 0) => 2 ** (semitones / 12);

// The SPU's pitch tops out at 4×.
const cap = (x) => Math.min(4, x);

/**
 * How a sample plays once replaced, with the replacement's pitch shift:
 * the typical speed and range, and the longest note's speed, duration and
 * the length of sound it uses (`seconds`: the length to aim for).
 */
export function playedAs(use, shift = 0) {
  if (!use) return null;
  const f = shiftFactor(shift);
  const l = use.longest;
  return {
    speed: cap(use.speed * f),
    speedMin: cap(use.speedMin * f),
    speedMax: cap(use.speedMax * f),
    noteSpeed: cap(l.speed * f),
    held: l.held,
    seconds: l.held * cap(l.speed * f),
  };
}

/**
 * The shift that makes the typical note play closest to 1×, in whole
 * octaves so the jingle stays in tune with the other samples.
 */
export function naturalShift(use) {
  return use ? 12 * Math.round(-Math.log2(use.speed)) : 0;
}

/**
 * The original sample from the ROM as float samples, with its loop start
 * (sample index, or -1 if it doesn't loop).
 */
export function originalSample(rom, i, vab = findVab(rom)) {
  const s = vab?.samples[i - 1];
  if (!s) return null;
  const bytes = rom.subarray(s.offset, s.offset + s.size);
  const ints = decodeAdpcm(bytes);
  let loopStart = -1;
  let end = ints.length;
  for (let b = 0; b < bytes.length; b += 16) {
    const flags = bytes[b + 1];
    if (flags & 4) loopStart = (b / 16) * 28;
    if (flags & 1) {
      end = (b / 16 + 1) * 28;
      if (!(flags & 2)) loopStart = -1;
      break;
    }
  }
  return { pcm: Float32Array.from(ints.subarray(0, end), (x) => x / 32768), rate: RATE, loopStart };
}

/**
 * Renders a sound the way a note plays it: at `speed`, for `seconds` (or
 * until it runs out, unless it loops back to `loopStart`), with a short fade
 * at the end. Returns float samples at 44.1 kHz.
 */
export function renderNote(pcm, rate, { speed = 1, seconds, loopStart = -1 }) {
  const step = (speed * rate) / RATE;
  const max = Math.round((seconds ?? pcm.length / rate / speed) * RATE);
  const out = new Float32Array(max);
  let pos = 0, n = 0;
  for (; n < max; n++) {
    if (pos >= pcm.length - 1) {
      if (loopStart < 0 || loopStart >= pcm.length - 1) break;
      pos = loopStart + (pos - (pcm.length - 1));
    }
    const k = Math.floor(pos), t = pos - k;
    out[n] = pcm[k] * (1 - t) + pcm[k + 1] * t;
    pos += step;
  }
  const fade = Math.min(n, Math.round(RATE * 0.05));
  for (let k = 0; k < fade; k++) out[n - 1 - k] *= k / fade;
  return out.subarray(0, n);
}
