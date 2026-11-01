// PS1 boot customizations: what the user can change, and applying it.
// - In the BIOS: the SONY / COMPUTER / ENTERTAINMENT and PlayStation text
//   (wordmark.js) and the boot sound samples (sound.js).
// - On the test disc: the license text (license.js) and the logo (logo.js).

import { patchWordmarks } from './wordmark.js';
import { patchSamples } from './sound.js';
import { licenseText } from './license.js';

export function defaultCustom() {
  return {
    texts: { sony: 'SONY', computer: 'COMPUTER', entertainment: 'ENTERTAINMENT', playstation: 'PlayStation' },
    license: null, // [line1, line2], or null for the region's own
    logo: null, // { name, tmd, preview, triangles, bytes, source, opts }
    sounds: [null, null, null], // { name, pcm, rate, loop, seconds }
  };
}

export function biosChanged(c) {
  const d = defaultCustom().texts;
  return Object.keys(d).some((k) => c.texts[k] !== d[k]) || c.sounds.some(Boolean);
}

/** Applies the BIOS-side customizations to a copy of the ROM. */
export function customizeBios(rom, c, { fallback } = {}) {
  if (!biosChanged(c)) return { rom, notes: [], used: {} };
  const t = c.texts;
  const w = patchWordmarks(
    rom,
    { sony: [t.sony], sce: [t.computer, t.entertainment], playstation: [t.playstation] },
    { fallback },
  );
  const sounds = {};
  c.sounds.forEach((s, i) => {
    if (s) sounds[i + 1] = s;
  });
  const s = patchSamples(w.rom, sounds);
  return { rom: s.rom, notes: [...w.notes, ...s.notes], used: s.used };
}

/** Options for buildTestDisc(). */
export function discOptions(c) {
  const opts = {};
  if (c.license) opts.license = licenseText(c.license[0], c.license[1]).text;
  if (c.logo) opts.logo = c.logo.tmd;
  return opts;
}
