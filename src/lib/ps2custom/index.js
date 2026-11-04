// PS2 boot customizations: what can be changed, and applying it.
//
// The PS2 browser program (OSDSYS) takes its pictures and sounds from files
// in the BIOS (see romfs.js), compressed (lz.js):
// - TEXIMAGE/TEXOSCE: the "Sony Computer Entertainment" line of the intro,
//   a 512 × 64 8-bit brightness picture. The intro shows its top half and
//   then its bottom half on one line, the bottom half starting just after where
//   "Sony Computer " ends, so it is really one 924 × 32 strip.
// - TEXIMAGE/TEXOPNG?: "Please insert a PlayStation or PlayStation 2 format
//   disc", one 256 × 128 picture per language (the last letter).
// - SNDIMAGE/SNDBOOTB: the intro's sound samples (SPU ADPCM, as on the PS1),
//   played by the sequence SNDBOOTS through the instruments in SNDBOOTH.

import { archiveFiles, fileData, rebuildArchive, replaceFile } from './romfs.js';
import { compress, decompress } from './lz.js';
import { decodeAdpcm, encodeAdpcm, resample } from '../ps1custom/sound.js';

export const SCE = { width: 512, height: 64, split: 412, strip: 412 + 512, rows: 32 };
export const DISC = { width: 256, height: 128 };
export const DEFAULT_SCE = 'Sony Computer Entertainment';
export const DEFAULT_DISC = ['Please insert a PlayStation', 'or PlayStation 2 format disc.'];
export const SOUND_RATE = 48000; // the SPU2's rate: pitch 0x1000 plays a sample at this rate

export function defaultCustom() {
  return {
    sceText: DEFAULT_SCE,
    sce: null, // { name, strip: Uint8Array(SCE.strip * SCE.rows), from: 'text' | 'picture' | 'drawing', drawing? }
    discText: [...DEFAULT_DISC],
    disc: null, // { image: Uint8Array(256 * 128) } when the text differs
    sounds: [], // per sample: { name, pcm (Float32, -1..1), rate, loop } or null
  };
}

export function biosChanged(c) {
  return !!c.sce || !!c.disc || c.sounds.some(Boolean);
}

/** The 918 × 32 strip folded into the 512 × 64 texture the intro uses. */
export function stripToTexture(strip) {
  const out = new Uint8Array(SCE.width * SCE.height);
  for (let y = 0; y < SCE.rows; y++) {
    for (let x = 0; x < SCE.width; x++) {
      out[y * SCE.width + x] = x < SCE.split ? strip[y * SCE.strip + x] : 0;
      out[(y + SCE.rows) * SCE.width + x] = strip[y * SCE.strip + SCE.split + x] ?? 0;
    }
  }
  return out;
}

/** The texture unfolded into the strip (to show the original). */
export function textureToStrip(tex) {
  const out = new Uint8Array(SCE.strip * SCE.rows);
  for (let y = 0; y < SCE.rows; y++) {
    for (let x = 0; x < SCE.strip; x++) {
      out[y * SCE.strip + x] = x < SCE.split ? tex[y * SCE.width + x] : tex[(y + SCE.rows) * SCE.width + x - SCE.split];
    }
  }
  return out;
}

function texFiles(rom) {
  const arc = fileData(rom, 'TEXIMAGE');
  return arc ? archiveFiles(arc) : null;
}

/** The original SCE line as a strip, or null if this BIOS has none. */
export function originalSce(rom) {
  const f = texFiles(rom)?.TEXOSCE;
  if (!f) return null;
  const tex = decompress(f);
  return tex.length === SCE.width * SCE.height ? textureToStrip(tex) : null;
}

/** Disc message pictures: { E: Uint8Array, J: …, … }. */
export function originalDisc(rom) {
  const files = texFiles(rom) || {};
  const out = {};
  for (const [name, data] of Object.entries(files)) {
    if (/^TEXOPNG.$/.test(name)) out[name[7]] = decompress(data);
  }
  return out;
}

// ------------------------------------------------------------- sounds --

/**
 * The boot sound bank: { body (Uint8Array, unpacked SNDBOOTB), samples:
 * [{ offset, size, seconds }] }. A sample runs until a block with the end
 * flag; one-block samples (silence) are left out.
 */
export function findSamples(rom) {
  const arc = fileData(rom, 'SNDIMAGE');
  const f = arc && archiveFiles(arc).SNDBOOTB;
  if (!f) return null;
  const body = decompress(f);
  const samples = [];
  let start = 0;
  for (let o = 0; o + 16 <= body.length; o += 16) {
    if (body[o + 1] & 1) {
      const size = o + 16 - start;
      if (size > 64) samples.push({ offset: start, size, seconds: ((size / 16) * 28) / SOUND_RATE, loops: (body[o + 1] & 2) !== 0 });
      start = o + 16;
    }
  }
  return { body, samples };
}

/** The original sample `i` as Int16 PCM at SOUND_RATE. */
export function originalSample(bank, i) {
  const s = bank.samples[i];
  return decodeAdpcm(bank.body.subarray(s.offset, s.offset + s.size));
}

// Encodes a sound into a sample's slot: same size, so the instruments that
// point at the samples stay right. Longer sounds are cut (with a short
// fade), shorter ones end early.
function fitSample(bank, i, sound) {
  const s = bank.samples[i];
  const room = (s.size / 16) * 28;
  let pcm = resample(sound.pcm, sound.rate, SOUND_RATE);
  const cut = pcm.length > room;
  if (cut) {
    pcm = pcm.slice(0, room);
    const fade = Math.min(pcm.length, Math.round(SOUND_RATE * 0.02));
    for (let k = 0; k < fade; k++) pcm[pcm.length - 1 - k] *= k / fade;
  }
  // As loud as the sample it replaces.
  let oldPeak = 0, peak = 0;
  for (const x of originalSample(bank, i)) oldPeak = Math.max(oldPeak, Math.abs(x));
  for (const x of pcm) peak = Math.max(peak, Math.abs(x));
  const gain = peak ? (oldPeak || 24000) / peak : 0;
  const enc = encodeAdpcm(Float32Array.from(pcm, (x) => x * gain), { loop: !!sound.loop });
  const out = new Uint8Array(s.size);
  out.set(enc.subarray(0, s.size));
  if (enc.length < s.size) {
    // Silent end blocks after the sound (the first one ends it).
    for (let o = enc.length; o < s.size; o += 16) out[o + 1] = 0;
    out[s.size - 16 + 1] = 1;
  }
  return { data: out, seconds: pcm.length / SOUND_RATE, cut, of: sound.pcm.length / sound.rate };
}

// ------------------------------------------------------------- apply --

/**
 * Applies the customizations to a copy of the BIOS. Returns { rom, notes,
 * used } (used: per replaced sample, seconds kept and whether it was cut).
 */
export function customizeBios(rom, c) {
  if (!biosChanged(c)) return { rom, notes: [], used: {} };
  let out = rom;
  const notes = [];
  const used = {};
  try {
    const tex = {};
    if (c.sce) tex.TEXOSCE = compress(stripToTexture(c.sce.strip));
    if (c.disc) {
      const packed = compress(c.disc.image);
      for (const k of Object.keys(originalDisc(rom))) tex[`TEXOPNG${k}`] = packed;
    }
    if (Object.keys(tex).length) out = replaceFile(out, 'TEXIMAGE', rebuildArchive(fileData(out, 'TEXIMAGE'), tex));

    if (c.sounds.some(Boolean)) {
      const bank = findSamples(rom);
      if (!bank) {
        notes.push("Couldn't find the boot sounds in this BIOS.");
      } else {
        const body = new Uint8Array(bank.body);
        c.sounds.forEach((s, i) => {
          if (!s || !bank.samples[i]) return;
          const r = fitSample(bank, i, s);
          body.set(r.data, bank.samples[i].offset);
          used[i] = r;
          if (r.cut) notes.push(`Sound ${i + 1}: kept the first ${r.seconds.toFixed(2)} s of ${r.of.toFixed(2)} s.`);
        });
        out = replaceFile(out, 'SNDIMAGE', rebuildArchive(fileData(out, 'SNDIMAGE'), { SNDBOOTB: compress(body) }));
      }
    }
  } catch (e) {
    notes.push(e.short ? `${e.message} Try a simpler picture or a shorter sound.` : e.message);
    return { rom, notes, used: {} };
  }
  return { rom: out, notes, used };
}
