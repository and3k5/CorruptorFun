// The license text on the licensed screen ("Licensed by / Sony Computer
// Entertainment America") comes from the disc (sector 4), and the BIOS draws
// it with its own font. It doesn't lay the text out itself, though: it cuts
// the string into 16-character pieces and draws each one at a fixed spot.
// Real discs pad their text with spaces so the pieces join up, which is why
// they read "Amer  ica". This file does that padding for any text.
//
// Measured with a US BIOS (SCPH-1001), in a 640-pixel-wide screen:
// - Line 1 is characters 0-31: pieces at x = 224 and x = 336.
// - Line 2 is characters 32-63: pieces at x = 157 and x = 345.
// - If the string is exactly 70 characters long, line 2 has a third piece of
//   6 characters, and the pieces move to x = 121, 309 and 485. That's the
//   layout of the American and European discs.

// How far the BIOS font moves on after each character, in pixels.
const NARROW = { 1: 8, I: 8, J: 12, i: 8, j: 8, l: 8, r: 8, m: 16, v: 16, w: 16 };
export function advance(ch) {
  if (ch === ' ') return 4;
  if (ch in NARROW) return NARROW[ch];
  if (ch >= 'a' && ch <= 'z') return 12;
  return 16;
}

const LINE1 = { starts: [224, 336], caps: [16, 16] };
const LINE2_SHORT = { starts: [157, 345], caps: [16, 16] };
const LINE2_LONG = { starts: [121, 309, 485], caps: [16, 16, 6] };
const CENTRE = 324; // where the real license text is centred
const SPACE = 4;

// Printable ASCII only; the BIOS shows "\" as a yen sign and "~" as an overline.
export function cleanText(s) {
  return String(s ?? '')
    .replace(/[^\x20-\x7e]/g, '?')
    .replace(/\s+/g, ' ')
    .trim();
}

const width = (s) => [...s].reduce((w, c) => w + advance(c), 0);

// Finds the best way to put `text` into the pieces of one line: centred, and
// with each piece starting where the previous one ended. Returns the piece
// strings (padded to their size) and how far off it is, or null if the text
// doesn't fit.
export function layoutLine(text, { starts, caps }) {
  const n = text.length;
  if (!n) return { pieces: caps.map((c) => ' '.repeat(c)), score: 0, width: 0 };
  // Cumulative widths, so the end of text[a..b) is quick to get.
  const cum = [0];
  for (const c of text) cum.push(cum[cum.length - 1] + advance(c));
  let best = null;

  // Tries to place text[i..] in piece j, which starts drawing at x.
  function place(j, i, x, pieces, gaps, x0) {
    if (i === n) {
      const score = Math.abs((x0 + x) / 2 - CENTRE) + gaps;
      if (!best || score < best.score) {
        const out = [...pieces];
        while (out.length < caps.length) out.push('');
        best = { pieces: out.map((p, k) => p.padEnd(caps[k], ' ')), score, width: x - x0, x0 };
      }
      return;
    }
    if (j >= caps.length) return;
    const lead = (x - starts[j]) / SPACE;
    if (lead < 0 || lead >= caps[j]) return;
    const room = caps[j] - lead;
    for (let k = 1; k <= Math.min(room, n - i); k++) {
      const end = x + cum[i + k] - cum[i];
      const piece = ' '.repeat(lead) + text.slice(i, i + k);
      if (i + k === n) {
        place(j + 1, n, end, [...pieces, piece], gaps, x0);
        continue;
      }
      const next = starts[j + 1];
      if (next === undefined) continue;
      if (end >= next) {
        place(j + 1, i + k, end, [...pieces, piece], gaps, x0);
      } else {
        // The next piece starts further right, leaving a gap. Fine at a space
        // (it just looks like a wider space), bad in the middle of a word.
        const atSpace = text[i + k - 1] === ' ' || text[i + k] === ' ';
        place(j + 1, i + k, next, [...pieces, piece], gaps + (atSpace ? next - end : 1000 + next - end), x0);
      }
    }
  }

  for (let j = 0; j < caps.length; j++) {
    for (let lead = 0; lead < caps[j]; lead++) {
      const x = starts[j] + lead * SPACE;
      place(j, 0, x, Array(j).fill(''), 0, x);
    }
  }
  return best;
}

// Builds the license sector text from two lines. Returns { text, warnings }.
// `text` is what goes at the start of sector 4 (64 or 70 characters).
export function licenseText(line1, line2) {
  const a = cleanText(line1);
  const b = cleanText(line2);
  const warnings = [];
  // The font's space is narrow (4 pixels), so words look run together.
  // Real discs use two spaces between words; do the same when it fits.
  const better = (x, y) => (!x ? y : !y ? x : y.score < x.score ? y : x);
  const fit = (text, layouts) => {
    let best = null;
    for (const t of [text.replace(/ /g, '  '), text]) {
      for (const l of layouts) best = better(best, layoutLine(t, l));
      if (best && best.score < 1000) return best;
    }
    return best;
  };
  const l1 = fit(a, [LINE1]);
  if (!l1) warnings.push(`Line 1 is too long (at most about ${LINE1.caps[0] + LINE1.caps[1]} letters, and less for wide ones).`);
  const l2 = fit(b, [LINE2_SHORT, LINE2_LONG]);
  if (!l2) warnings.push('Line 2 is too long (at most 38 letters, and less for wide ones).');
  for (const [l, name] of [
    [l1, 'Line 1'],
    [l2, 'Line 2'],
  ]) {
    if (l && l.score >= 1000) warnings.push(`${name} has a gap inside a word: the BIOS draws it in 16-letter pieces at fixed spots. Try adding or removing a space.`);
  }
  const p1 = l1 ? l1.pieces.join('') : ' '.repeat(32);
  const p2 = l2 ? l2.pieces.join('') : ' '.repeat(32);
  return { text: p1 + p2, warnings };
}

export const DEFAULT_LICENSE = {
  A: ['Licensed by', 'Sony Computer Entertainment America'],
  E: ['Licensed by', 'Sony Computer Entertainment Europe'],
  I: ['Licensed by', 'Sony Computer Entertainment Inc.'],
};
