// The words on the boot screens that the BIOS draws from pictures in its ROM:
// "SONY", "COMPUTER ENTERTAINMENT" on the white intro screen, and
// "PlayStation" on the licensed screen. Each is a 16-colour TIM picture
// stored as is in the ROM. To change the text we rebuild the picture,
// reusing the letters cut out of the original, so it stays in the same font.
// Letters the original doesn't have are taken from the other line (scaled)
// or, failing that, drawn with a similar system font by a `fallback`
// function the caller passes in (the browser uses a canvas).
//
// Everything is read from the user's own BIOS at run time; no Sony pictures
// are part of this code.

// Pictures we know, by size in pixels. `lines` are the words in the original,
// from top to bottom; `mark` means the last shape is a ® or ™ that we keep.
export const WORDMARKS = {
  sony: { label: 'SONY', w: 240, h: 48, lines: ['SONY'], caps: true, style: 'serif' },
  sce: { label: 'COMPUTER / ENTERTAINMENT', w: 240, h: 60, lines: ['COMPUTER', 'ENTERTAINMENT'], caps: true, style: 'sans' },
  playstation: { label: 'PlayStation', w: 200, h: 40, lines: ['PlayStation'], mark: true, style: 'sans' },
};

// Finds 4-bit TIMs (with a 16-colour CLUT) in a ROM image.
export function findTims(rom) {
  const v = new DataView(rom.buffer, rom.byteOffset, rom.byteLength);
  const out = [];
  for (let o = 0; o + 64 < rom.length; o += 4) {
    if (v.getUint32(o, true) !== 0x10 || v.getUint32(o + 4, true) !== 8) continue;
    const clutLen = v.getUint32(o + 8, true);
    if (clutLen !== 44) continue; // one 16-colour palette
    const p = o + 8 + clutLen;
    if (p + 12 > rom.length) continue;
    const len = v.getUint32(p, true);
    const wHalf = v.getUint16(p + 8, true);
    const h = v.getUint16(p + 10, true);
    if (!wHalf || !h || len !== 12 + wHalf * 2 * h || p + len > rom.length) continue;
    const clut = [];
    for (let i = 0; i < 16; i++) clut.push(v.getUint16(o + 20 + i * 2, true));
    out.push({ offset: o, clut, pixels: p + 12, w: wHalf * 4, h, stride: wHalf * 2 });
  }
  return out;
}

export function readIndexes(rom, tim) {
  const px = new Uint8Array(tim.w * tim.h);
  for (let y = 0; y < tim.h; y++) {
    for (let x = 0; x < tim.w; x++) {
      const b = rom[tim.pixels + y * tim.stride + (x >> 1)];
      px[y * tim.w + x] = (b >> ((x & 1) * 4)) & 15;
    }
  }
  return px;
}

export function writeIndexes(rom, tim, px) {
  for (let y = 0; y < tim.h; y++) {
    for (let x = 0; x < tim.w; x += 2) {
      rom[tim.pixels + y * tim.stride + (x >> 1)] = px[y * tim.w + x] | (px[y * tim.w + x + 1] << 4);
    }
  }
}

const rgb = (c) => [(c & 31) << 3, ((c >> 5) & 31) << 3, ((c >> 10) & 31) << 3];

// How much "ink" each palette entry is, from 0 (background) to 1 (ink): the
// letters are one colour, anti-aliased towards the screen behind them.
function inkLevels(clut, px, bgIndex) {
  const counts = new Array(16).fill(0);
  for (const i of px) counts[i]++;
  const solid = clut.map((c, i) => (i !== bgIndex && c !== 0 ? counts[i] : -1));
  const ink = rgb(clut[solid.indexOf(Math.max(...solid))]);
  // The edge colour: the used entry furthest from the ink.
  let far = ink, fd = -1;
  clut.forEach((c, i) => {
    if (i === bgIndex || c === 0 || !counts[i]) return;
    const q = rgb(c);
    const d = (q[0] - ink[0]) ** 2 + (q[1] - ink[1]) ** 2 + (q[2] - ink[2]) ** 2;
    if (d > fd) [far, fd] = [q, d];
  });
  const span = [0, 1, 2].map((k) => ink[k] - far[k]);
  const len2 = span.reduce((s, x) => s + x * x, 0) || 1;
  return clut.map((c, i) => {
    if (i === bgIndex || c === 0) return 0;
    const q = rgb(c);
    const t = [0, 1, 2].reduce((s, k) => s + (q[k] - far[k]) * span[k], 0) / len2;
    // Even the faintest edge colour is some ink.
    return 0.15 + 0.85 * Math.min(1, Math.max(0, t));
  });
}

function nearestIndex(levels, bgIndex, a) {
  if (a < 0.08) return bgIndex;
  let best = bgIndex, bd = Infinity;
  levels.forEach((l, i) => {
    if (i === bgIndex || l === 0) return;
    const d = Math.abs(l - a);
    if (d < bd) [best, bd] = [i, d];
  });
  return best;
}

// Splits the picture into rows of text, and each row into letter shapes.
function segment(px, w, h, bgIndex) {
  const ink = (x, y) => px[y * w + x] !== bgIndex;
  const bands = [];
  for (let y = 0; y < h; ) {
    let any = false;
    for (let x = 0; x < w && !any; x++) any = ink(x, y);
    if (!any) {
      y++;
      continue;
    }
    const y0 = y;
    for (; y < h; y++) {
      let row = false;
      for (let x = 0; x < w && !row; x++) row = ink(x, y);
      if (!row) break;
    }
    bands.push({ y0, y1: y });
  }
  for (const b of bands) {
    const colInk = [];
    for (let x = 0; x < w; x++) {
      let n = 0;
      for (let y = b.y0; y < b.y1; y++) n += ink(x, y) ? 1 : 0;
      colInk.push(n);
    }
    b.colInk = colInk;
    b.segs = [];
    for (let x = 0; x < w; ) {
      if (!colInk[x]) {
        x++;
        continue;
      }
      const x0 = x;
      while (x < w && colInk[x]) x++;
      b.segs.push({ x0, x1: x });
    }
  }
  return bands;
}

// Letters that touch come out as one shape; split the widest shapes at their
// thinnest column until there's one shape per letter.
function splitTo(band, count) {
  const segs = band.segs;
  while (segs.length < count) {
    let wi = 0;
    segs.forEach((s, i) => {
      if (s.x1 - s.x0 > segs[wi].x1 - segs[wi].x0) wi = i;
    });
    const s = segs[wi];
    const a = s.x0 + Math.floor((s.x1 - s.x0) * 0.25), b = s.x1 - Math.floor((s.x1 - s.x0) * 0.25);
    let cut = a, least = Infinity;
    for (let x = a; x < b; x++) {
      if (band.colInk[x] < least) [cut, least] = [x, band.colInk[x]];
    }
    segs.splice(wi, 1, { x0: s.x0, x1: cut, joined: s.joined }, { x0: cut, x1: s.x1, joined: true });
  }
}

// Coverage image (0..1 per pixel) of a rectangle of the picture.
// `mask(x, y)` (picture coordinates), if given, says which pixels belong.
function cutCoverage(px, w, levels, x0, x1, y0, y1, mask) {
  const cw = x1 - x0, ch = y1 - y0;
  const cov = new Float32Array(cw * ch);
  for (let y = 0; y < ch; y++) {
    for (let x = 0; x < cw; x++) {
      if (!mask || mask(x0 + x, y0 + y)) cov[y * cw + x] = levels[px[(y0 + y) * w + x0 + x]];
    }
  }
  return { w: cw, h: ch, cov };
}

// Letters split by splitTo() overlap a little (the T's bar over the A). Give
// each connected blob of ink to the letter its centre falls in, so a letter
// cut out doesn't carry bits of its neighbour, and fix up the letter's extent.
function separate(px, w, bgIndex, band) {
  const segs = band.segs;
  for (let a = 0; a < segs.length; ) {
    let b = a + 1;
    while (b < segs.length && segs[b].joined) b++;
    if (b - a > 1) {
      const X0 = segs[a].x0, X1 = segs[b - 1].x1, H = band.y1 - band.y0, W = X1 - X0;
      const label = new Int32Array(W * H).fill(-1);
      const owner = [];
      for (let y = 0; y < H; y++) {
        for (let x = 0; x < W; x++) {
          if (label[y * W + x] >= 0 || px[(band.y0 + y) * w + X0 + x] === bgIndex) continue;
          // Flood fill one blob (8-connected) and find its centre.
          const id = owner.length;
          const stack = [[x, y]];
          label[y * W + x] = id;
          let sx = 0, n = 0;
          while (stack.length) {
            const [cx, cy] = stack.pop();
            sx += cx;
            n++;
            for (let dy = -1; dy <= 1; dy++) {
              for (let dx = -1; dx <= 1; dx++) {
                const nx = cx + dx, ny = cy + dy;
                if (nx < 0 || ny < 0 || nx >= W || ny >= H || label[ny * W + nx] >= 0) continue;
                if (px[(band.y0 + ny) * w + X0 + nx] === bgIndex) continue;
                label[ny * W + nx] = id;
                stack.push([nx, ny]);
              }
            }
          }
          const cx = X0 + sx / n;
          let k = a;
          while (k < b - 1 && cx >= segs[k].x1) k++;
          owner.push(k);
        }
      }
      for (let k = a; k < b; k++) {
        let lo = Infinity, hi = -Infinity;
        for (let y = 0; y < H; y++) {
          for (let x = 0; x < W; x++) {
            const id = label[y * W + x];
            if (id >= 0 && owner[id] === k) {
              lo = Math.min(lo, x);
              hi = Math.max(hi, x);
            }
          }
        }
        if (lo > hi) continue; // nothing ended up here; keep the plain cut
        const kk = k;
        segs[k].mask = (x, y) => {
          const id = label[(y - band.y0) * W + x - X0];
          return id >= 0 && owner[id] === kk;
        };
        segs[k].x0 = X0 + lo;
        segs[k].x1 = X0 + hi + 1;
      }
    }
    a = b;
  }
}

// Box/bilinear resample of a coverage image.
function resample(g, nw, nh) {
  nw = Math.max(1, Math.round(nw));
  nh = Math.max(1, Math.round(nh));
  if (nw === g.w && nh === g.h) return g;
  const cov = new Float32Array(nw * nh);
  const sx = g.w / nw, sy = g.h / nh;
  for (let y = 0; y < nh; y++) {
    for (let x = 0; x < nw; x++) {
      // Average the source area this pixel covers (at least one sample).
      const fx0 = x * sx, fx1 = (x + 1) * sx, fy0 = y * sy, fy1 = (y + 1) * sy;
      let s = 0, n = 0;
      for (let yy = Math.floor(fy0); yy < Math.max(Math.floor(fy0) + 1, Math.ceil(fy1)); yy++) {
        for (let xx = Math.floor(fx0); xx < Math.max(Math.floor(fx0) + 1, Math.ceil(fx1)); xx++) {
          const wy = Math.min(fy1, yy + 1) - Math.max(fy0, yy) || 1;
          const wx = Math.min(fx1, xx + 1) - Math.max(fx0, xx) || 1;
          s += g.cov[Math.min(g.h - 1, yy) * g.w + Math.min(g.w - 1, xx)] * wx * wy;
          n += wx * wy;
        }
      }
      cov[y * nw + x] = s / n;
    }
  }
  return { w: nw, h: nh, cov };
}

// Reads one of the known pictures: its letters, line positions and spacing.
export function analyse(rom, tim, kind) {
  const def = WORDMARKS[kind];
  const px = readIndexes(rom, tim);
  // The background is whatever fills the corners.
  const corners = [px[0], px[tim.w - 1], px[(tim.h - 1) * tim.w], px[tim.h * tim.w - 1]];
  const bgIndex = corners.sort((a, b) => corners.filter((c) => c === b).length - corners.filter((c) => c === a).length)[0];
  const levels = inkLevels(tim.clut, px, bgIndex);
  const bands = segment(px, tim.w, tim.h, bgIndex);
  if (bands.length !== def.lines.length) throw new Error(`unexpected layout in the ${def.label} picture`);
  const lines = bands.map((band, i) => {
    const word = def.lines[i];
    const want = word.length + (def.mark ? 1 : 0);
    if (band.segs.length > want && def.mark) {
      // A mark in several pieces: join the trailing ones.
      const tail = band.segs.splice(want - 1);
      band.segs.push({ x0: tail[0].x0, x1: tail[tail.length - 1].x1 });
    }
    if (band.segs.length > want) throw new Error(`unexpected letters in the ${def.label} picture`);
    splitTo(band, want);
    separate(px, tim.w, bgIndex, band);
    const glyphs = {};
    const gaps = [];
    // Letter pairs that overlap in the original (like "TA"): how far apart.
    const touching = new Map();
    band.segs.forEach((s, k) => {
      if (k > 0 && s.joined) touching.set(word[k - 1] + word[k], s.x0 - band.segs[k - 1].x1);
      else if (k > 0) gaps.push(s.x0 - band.segs[k - 1].x1);
      if (k < word.length && !glyphs[word[k]]) glyphs[word[k]] = cutCoverage(px, tim.w, levels, s.x0, s.x1, band.y0, band.y1, s.mask);
    });
    const mark = def.mark ? cutCoverage(px, tim.w, levels, band.segs[want - 1].x0, band.segs[want - 1].x1, band.y0, band.y1) : null;
    const markGap = def.mark ? gaps.pop() : 0;
    gaps.sort((a, b) => a - b);
    // Where capitals start and end, for drawing missing letters to match.
    const capChar = Object.keys(glyphs).find((c) => /[A-Z]/.test(c)) || Object.keys(glyphs)[0];
    const cap = glyphs[capChar];
    let capTop = cap.h, baseline = 0;
    for (let y = 0; y < cap.h; y++) {
      for (let x = 0; x < cap.w; x++) {
        if (cap.cov[y * cap.w + x] > 0.5) {
          capTop = Math.min(capTop, y);
          baseline = Math.max(baseline, y + 1);
        }
      }
    }
    const ref = { ch: capChar, w: cap.w };
    return { y0: band.y0, y1: band.y1, glyphs, gap: gaps[gaps.length >> 1] ?? 2, mark, markGap, capTop, baseline, ref, touching, word, original: px.slice(band.y0 * tim.w, band.y1 * tim.w) };
  });
  return { tim, bgIndex, levels, lines, def };
}

// The letters the original has, per line, for showing in the UI.
export function availableLetters(info) {
  return info.lines.map((l) => Object.keys(l.glyphs).sort().join(''));
}

// Lays out new text for every line and returns the new pixel indexes plus a
// list of letters that had to come from elsewhere.
export function compose(info, texts, { fallback } = {}) {
  const { tim, def, lines, levels, bgIndex } = info;
  const W = tim.w;
  const out = new Uint8Array(tim.w * tim.h).fill(bgIndex);
  const scaled = new Set();
  const drawn = new Set();
  const missing = new Set();

  lines.forEach((line, li) => {
    let text = String(texts[li] ?? def.lines[li]);
    if (def.caps) text = text.toUpperCase();
    if (text === line.word) {
      // Unchanged: keep the original pixels.
      out.set(line.original, line.y0 * W);
      return;
    }
    const lh = line.y1 - line.y0;
    // Pick a shape for every character.
    const shapes = [];
    let prev = '';
    for (const ch of text) {
      const pair = prev + ch;
      prev = ch;
      if (ch === ' ') {
        shapes.push({ space: Math.round(lh * 0.35) });
        continue;
      }
      let g = line.glyphs[ch];
      if (!g) {
        // Same font, other line: scale it to this line's height.
        const other = lines.find((l) => l.glyphs[ch]);
        if (other) {
          const src = other.glyphs[ch];
          g = resample(src, (src.w * lh) / src.h, lh);
          scaled.add(ch);
        }
      }
      if (!g && fallback) {
        g = fallback(ch, { height: lh, capTop: line.capTop, baseline: line.baseline, style: def.style, ref: line.ref });
        if (g) drawn.add(ch);
      }
      if (!g) {
        missing.add(ch);
        continue;
      }
      shapes.push(line.touching.has(pair) ? { ...g, touch: line.touching.get(pair) } : g);
    }
    // Total width, then squeeze to fit if needed.
    const markW = line.mark ? line.mark.w + line.markGap : 0;
    let total = 0;
    const gapBefore = (s, k) => (!k ? 0 : s.touch !== undefined ? s.touch : line.gap);
    shapes.forEach((s, k) => (total += (s.space ?? s.w) + gapBefore(s, k)));
    const room = W - markW;
    const scale = total > room ? room / total : 1;
    let x = Math.round((W - (total * scale + markW)) / 2);
    const put = (g, x0) => {
      for (let y = 0; y < g.h && y < lh; y++) {
        for (let xx = 0; xx < g.w; xx++) {
          const X = x0 + xx;
          if (X < 0 || X >= W) continue;
          const a = g.cov[y * g.w + xx];
          if (a < 0.08) continue;
          const o = (line.y0 + y) * W + X;
          // Where letters overlap, keep the stronger ink.
          if (levels[out[o]] < a) out[o] = nearestIndex(levels, bgIndex, a);
        }
      }
    };
    shapes.forEach((s, k) => {
      x += gapBefore(s, k) * scale;
      if (s.space) {
        x += s.space * scale;
        return;
      }
      const g = scale < 1 ? resample(s, s.w * scale, s.h) : s;
      put(g, Math.round(x));
      x += g.w;
    });
    if (line.mark) put(line.mark, Math.round(x + line.markGap));
  });
  return { px: out, scaled: [...scaled], drawn: [...drawn], missing: [...missing] };
}

/**
 * Finds the known pictures in a BIOS. Returns { kind: [info, ...] } where
 * each info is from analyse(); a kind can have several pictures (the
 * PlayStation text comes with ® and with ™).
 */
export function findWordmarks(rom) {
  const found = {};
  for (const tim of findTims(rom)) {
    for (const [kind, def] of Object.entries(WORDMARKS)) {
      if (tim.w !== def.w || tim.h !== def.h) continue;
      try {
        (found[kind] ||= []).push(analyse(rom, tim, kind));
      } catch {
        // Same size but not the picture we know; leave it alone.
      }
    }
  }
  return found;
}

/**
 * Writes new text into a copy of the BIOS. `texts` is { sony: ['SONY'],
 * sce: ['COMPUTER', 'ENTERTAINMENT'], playstation: ['PlayStation'] }, any of
 * them optional. Returns { rom, notes }.
 */
export function patchWordmarks(rom, texts, opts = {}) {
  const out = new Uint8Array(rom);
  const found = findWordmarks(rom);
  const notes = [];
  for (const [kind, lines] of Object.entries(texts)) {
    if (!lines) continue;
    const def = WORDMARKS[kind];
    if (lines.every((t, i) => t === def.lines[i])) continue;
    if (!found[kind]?.length) {
      notes.push(`Couldn't find the ${def.label} picture in this BIOS.`);
      continue;
    }
    for (const info of found[kind]) {
      const r = compose(info, lines, opts);
      writeIndexes(out, info.tim, r.px);
      if (r.missing.length) notes.push(`${def.label}: left out ${r.missing.join(' ')} (no letter for it).`);
      if (r.scaled.length) notes.push(`${def.label}: ${r.scaled.join(' ')} taken from the other line and resized.`);
      if (r.drawn.length) notes.push(`${def.label}: ${r.drawn.join(' ')} not in the original, drawn with a similar font.`);
    }
  }
  return { rom: out, notes: [...new Set(notes)] };
}
