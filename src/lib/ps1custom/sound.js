// The boot sounds are played from a VAB (a bank of instruments) in the BIOS
// ROM: three ADPCM samples, played at the pitches of the boot jingles. We can
// swap the samples for any sound, re-encoded as SPU ADPCM. The BIOS still
// plays its own melody and envelopes with them, so your sound becomes the
// instrument.
//
// Limits: the samples have to fit where the old ones were (about 42 KB in
// all, 1.7 seconds at 44.1 kHz). We record longer sounds at a lower rate
// (22.05 or 11.025 kHz, and tell the BIOS so through the instrument's
// centre note, so they still play at the right pitch), and cut what still
// doesn't fit.

const FILTERS = [
  [0, 0],
  [60, 0],
  [115, -52],
  [98, -55],
  [122, -60],
];

export const RATES = [44100, 22050, 11025];

// What each sample does in the US BIOS (SCPH-1001); other BIOS versions
// likely use them the same way.
export const SAMPLE_ROLES = [
  'Main tone (both jingles)',
  'Licensed screen chime',
  'Sony intro swell',
];

/** Finds and reads the VAB. Returns null if there isn't one. */
export function findVab(rom) {
  const magic = [0x70, 0x42, 0x41, 0x56]; // "pBAV"
  for (let o = 0; o + 32 < rom.length; o += 4) {
    if (rom[o] !== magic[0] || rom[o + 1] !== magic[1] || rom[o + 2] !== magic[2] || rom[o + 3] !== magic[3]) continue;
    const v = new DataView(rom.buffer, rom.byteOffset + o);
    const fsize = v.getUint32(12, true);
    const ps = v.getUint16(18, true);
    const vs = v.getUint16(22, true);
    if (!ps || ps > 128 || !vs || vs > 254 || o + fsize > rom.length) continue;
    const tones = o + 32 + 128 * 16;
    const table = tones + ps * 16 * 32;
    const body = table + 512;
    const sizes = [];
    for (let i = 1; i <= vs; i++) sizes.push(v.getUint16(table - o + i * 2, true) * 8);
    if (body + sizes.reduce((a, b) => a + b, 0) > o + fsize) continue;
    const samples = [];
    let p = body;
    sizes.forEach((size, i) => {
      samples.push({ index: i + 1, offset: p, size, loop: hasLoop(rom, p, size) });
      p += size;
    });
    // Instruments (tones) that play each sample, so we can retune them.
    const toneList = [];
    for (let t = 0; t < ps * 16; t++) {
      const to = tones + t * 32;
      const vag = rom[to + 22] | (rom[to + 23] << 8);
      if (vag >= 1 && vag <= vs) toneList.push({ offset: to, vag });
    }
    return { offset: o, fsize, ps, vs, table, body, end: o + fsize, samples, tones: toneList };
  }
  return null;
}

function hasLoop(rom, off, size) {
  for (let p = off; p < off + size; p += 16) if (rom[p + 1] & 1) return (rom[p + 1] & 2) !== 0;
  return false;
}

/** Decodes SPU ADPCM to 16-bit samples. */
export function decodeAdpcm(bytes) {
  const out = new Int16Array((bytes.length / 16) * 28);
  let s1 = 0, s2 = 0, n = 0;
  for (let b = 0; b + 16 <= bytes.length; b += 16) {
    const shift = bytes[b] & 15, f = Math.min(4, bytes[b] >> 4);
    for (let i = 0; i < 28; i++) {
      let s = ((bytes[b + 2 + (i >> 1)] >> ((i & 1) * 4)) & 15) << 12;
      if (s & 0x8000) s -= 0x10000;
      s = (s >> shift) + Math.floor((s1 * FILTERS[f][0] + s2 * FILTERS[f][1] + 32) / 64);
      s = Math.max(-32768, Math.min(32767, s));
      out[n++] = s;
      s2 = s1;
      s1 = s;
    }
  }
  return out;
}

// Encodes one block of 28 samples, trying every filter and shift.
function encodeBlock(pcm, at, state, out, o) {
  let best = null;
  for (let f = 0; f < 5; f++) {
    // Pick the smallest shift whose range covers the prediction error.
    let s1 = state[0], s2 = state[1], peak = 0;
    for (let i = 0; i < 28; i++) {
      const x = pcm[at + i] ?? 0;
      const pred = (s1 * FILTERS[f][0] + s2 * FILTERS[f][1]) / 64;
      peak = Math.max(peak, Math.abs(x - pred));
      s2 = s1;
      s1 = x;
    }
    let shift = 12;
    while (shift > 0 && peak > (7 << (12 - shift)) * 1) shift--;
    for (const sh of [shift, Math.max(0, shift - 1)]) {
      let p1 = state[0], p2 = state[1], err = 0;
      const nib = new Uint8Array(28);
      for (let i = 0; i < 28; i++) {
        const x = pcm[at + i] ?? 0;
        const pred = Math.floor((p1 * FILTERS[f][0] + p2 * FILTERS[f][1] + 32) / 64);
        let q = Math.round(((x - pred) * (1 << sh)) / 4096);
        q = Math.max(-8, Math.min(7, q));
        nib[i] = q & 15;
        let s = ((q << 12) >> sh) + pred;
        s = Math.max(-32768, Math.min(32767, s));
        err += (x - s) ** 2;
        p2 = p1;
        p1 = s;
      }
      if (!best || err < best.err) best = { err, f, sh, nib, p1, p2 };
    }
  }
  out[o] = (best.f << 4) | best.sh;
  for (let i = 0; i < 14; i++) out[o + 2 + i] = best.nib[i * 2] | (best.nib[i * 2 + 1] << 4);
  state[0] = best.p1;
  state[1] = best.p2;
}

/**
 * Encodes 16-bit-range samples (any numeric array) as SPU ADPCM. The first
 * block marks the loop start; the last block ends the sound, or jumps back to
 * the start when `loop` is set.
 */
export function encodeAdpcm(pcm, { loop = false } = {}) {
  const blocks = Math.max(1, Math.ceil(pcm.length / 28));
  const out = new Uint8Array(blocks * 16);
  const state = [0, 0];
  for (let b = 0; b < blocks; b++) encodeBlock(pcm, b * 28, state, out, b * 16);
  out[1] |= 4;
  out[(blocks - 1) * 16 + 1] |= loop ? 3 : 1;
  return out;
}

// Linear resampling of mono float samples.
export function resample(src, from, to) {
  if (from === to) return src;
  const n = Math.max(1, Math.floor((src.length * to) / from));
  const out = new Float32Array(n);
  const step = from / to;
  for (let i = 0; i < n; i++) {
    const x = i * step, k = Math.floor(x), t = x - k;
    out[i] = (src[k] ?? 0) * (1 - t) + (src[k + 1] ?? src[k] ?? 0) * t;
  }
  return out;
}

/**
 * Replaces samples in a copy of the BIOS. `sounds` maps sample index (1..)
 * to { pcm: Float32Array (-1..1, mono), rate, loop }. Returns
 * { rom, notes, used } where `used` says, per replaced sample, the rate,
 * seconds kept and whether it was cut.
 */
export function patchSamples(rom, sounds) {
  const out = new Uint8Array(rom);
  const vab = findVab(rom);
  const notes = [];
  if (!vab) return { rom: out, notes: ["Couldn't find the boot sound bank in this BIOS."], used: {} };
  const keys = Object.keys(sounds)
    .map(Number)
    .filter((i) => i >= 1 && i <= vab.vs && sounds[i]);
  if (!keys.length) return { rom: out, notes, used: {} };

  // Replaced samples share the room their originals took.
  const pool = keys.reduce((s, i) => s + vab.samples[i - 1].size, 0);
  const seconds = (i) => sounds[i].pcm.length / sounds[i].rate;
  const bytesAt = (i, rate) => Math.ceil((seconds(i) * rate) / 28) * 16;
  const totalSec = keys.reduce((s, i) => s + seconds(i), 0) || 1;
  const used = {};
  const encoded = {};
  for (const i of keys) {
    const share = Math.floor((pool * seconds(i)) / totalSec / 16) * 16 || 16;
    // At full rate if everything fits, else the best rate that fits the share.
    const fitsAll = keys.reduce((s, k) => s + bytesAt(k, 44100), 0) <= pool;
    const rate = fitsAll ? 44100 : RATES.find((r) => bytesAt(i, r) <= share) || RATES[RATES.length - 1];
    let pcm = resample(sounds[i].pcm, sounds[i].rate, rate);
    const room = fitsAll ? Infinity : Math.floor(share / 16) * 28;
    const cut = pcm.length > room;
    if (cut) {
      pcm = pcm.slice(0, room);
      // Fade out the last 20 ms so the cut doesn't click.
      const fade = Math.min(pcm.length, Math.round(rate * 0.02));
      for (let k = 0; k < fade; k++) pcm[pcm.length - 1 - k] *= k / fade;
    }
    // As loud as the sample it replaces.
    const old = decodeAdpcm(rom.subarray(vab.samples[i - 1].offset, vab.samples[i - 1].offset + vab.samples[i - 1].size));
    let oldPeak = 0, peak = 0;
    for (const s of old) oldPeak = Math.max(oldPeak, Math.abs(s));
    for (const s of pcm) peak = Math.max(peak, Math.abs(s));
    const gain = peak ? (oldPeak || 24000) / peak : 0;
    const ints = Float32Array.from(pcm, (s) => s * gain);
    encoded[i] = encodeAdpcm(ints, { loop: !!sounds[i].loop });
    used[i] = { rate, seconds: pcm.length / rate, cut, of: seconds(i) };
  }

  // Lay the samples out again, in order, and rewrite the size table.
  let p = vab.body;
  const v = new DataView(out.buffer);
  vab.samples.forEach((s, k) => {
    const i = k + 1;
    const data = encoded[i] || rom.subarray(s.offset, s.offset + s.size);
    out.set(data, p);
    v.setUint16(vab.table + i * 2, data.length / 8, true);
    p += data.length;
  });
  out.fill(0, p, vab.end);

  // Retune the instruments: a sample at half the rate needs a centre note an
  // octave up to sound at the same pitch.
  for (const t of vab.tones) {
    if (!encoded[t.vag]) continue;
    const octaves = Math.round(Math.log2(44100 / used[t.vag].rate));
    out[t.offset + 4] = Math.min(127, rom[t.offset + 4] + 12 * octaves);
  }
  for (const i of keys) {
    if (used[i].cut) notes.push(`Sample ${i}: kept the first ${used[i].seconds.toFixed(2)} s of ${used[i].of.toFixed(2)} s.`);
  }
  return { rom: out, notes, used };
}
