// A small pixel-art drawing for the logo on the licensed screen. The logo is
// pixel art anyway (see logo.js), so the drawing is a grid of palette indexes
// with no antialiasing: every colour you pick is a colour the model gets, and
// same-coloured areas merge into few triangles.
//
// A drawing is { width, height, pixels: Uint8Array (palette index per
// pixel), palette: [[r, g, b], ...] }. Index 0 is the background: black, left
// out of the model, like the screen behind it.

export const MAX_COLOURS = 32; // drawing colours, not counting the background
// Channels below this count as background in logo.js (blackCutoff), so the
// darkest colour we allow is a little above it.
export const MIN_BRIGHT = 32;

// Grid sizes to draw at, about the shape of the room the BIOS gives the logo
// (2600 x 1150 model units). 128 wide is the most logo.js uses.
export const SIZES = [
  [32, 14],
  [48, 21],
  [64, 28],
  [96, 42],
  [128, 56],
];

export const DEFAULT_PALETTE = [
  [0, 0, 0],
  [255, 255, 255],
  [160, 168, 176],
  [232, 40, 48],
  [255, 144, 32],
  [255, 216, 40],
  [56, 200, 88],
  [40, 200, 216],
  [48, 96, 232],
  [144, 72, 216],
  [248, 104, 176],
  [136, 88, 56],
];

export function createDrawing(width = 96, height = 42, palette = DEFAULT_PALETTE) {
  return { width, height, pixels: new Uint8Array(width * height), palette: palette.map((c) => [...c]) };
}

export function cloneDrawing(d) {
  return { width: d.width, height: d.height, pixels: new Uint8Array(d.pixels), palette: d.palette.map((c) => [...c]) };
}

/** Keeps a colour bright enough not to be taken for the background. */
export function visibleColour([r, g, b]) {
  const m = Math.max(r, g, b);
  if (m >= MIN_BRIGHT) return [r, g, b];
  if (m === 0) return [MIN_BRIGHT, MIN_BRIGHT, MIN_BRIGHT];
  const s = MIN_BRIGHT / m;
  return [r, g, b].map((v) => Math.min(255, Math.round(v * s)));
}

/**
 * Stamps a round brush of `size` pixels across, centred on (x, y) in pixel
 * units (fractions allowed). Returns whether any pixel changed.
 */
export function stamp(d, x, y, size, value) {
  const { width: w, height: h, pixels } = d;
  let changed = false;
  if (size <= 1) {
    const px = Math.floor(x), py = Math.floor(y);
    if (px < 0 || py < 0 || px >= w || py >= h) return false;
    const i = py * w + px;
    if (pixels[i] !== value) {
      pixels[i] = value;
      changed = true;
    }
    return changed;
  }
  // Even sizes centre on a pixel corner, odd ones on a pixel centre, so a
  // brush of n is always n pixels across.
  const cx = size % 2 ? Math.floor(x) + 0.5 : Math.round(x);
  const cy = size % 2 ? Math.floor(y) + 0.5 : Math.round(y);
  const r = size / 2;
  const r2 = r * r + 0.01;
  const x0 = Math.max(0, Math.floor(cx - r)), x1 = Math.min(w - 1, Math.ceil(cx + r));
  const y0 = Math.max(0, Math.floor(cy - r)), y1 = Math.min(h - 1, Math.ceil(cy + r));
  for (let py = y0; py <= y1; py++) {
    const dy = py + 0.5 - cy;
    for (let px = x0; px <= x1; px++) {
      const dx = px + 0.5 - cx;
      if (dx * dx + dy * dy > r2) continue;
      const i = py * w + px;
      if (pixels[i] !== value) {
        pixels[i] = value;
        changed = true;
      }
    }
  }
  return changed;
}

/** Stamps the brush along a line, close enough together to leave no gaps. */
export function strokeLine(d, x0, y0, x1, y1, size, value) {
  const len = Math.hypot(x1 - x0, y1 - y0);
  const steps = Math.max(1, Math.ceil(len / 0.5));
  let changed = false;
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    if (stamp(d, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, size, value)) changed = true;
  }
  return changed;
}

/** Fills the area of one colour around (x, y), 4-connected. */
export function floodFill(d, x, y, value) {
  const { width: w, height: h, pixels } = d;
  x = Math.floor(x);
  y = Math.floor(y);
  if (x < 0 || y < 0 || x >= w || y >= h) return false;
  const from = pixels[y * w + x];
  if (from === value) return false;
  const stack = [y * w + x];
  while (stack.length) {
    const i = stack.pop();
    if (pixels[i] !== from) continue;
    pixels[i] = value;
    const px = i % w;
    if (px > 0) stack.push(i - 1);
    if (px < w - 1) stack.push(i + 1);
    if (i >= w) stack.push(i - w);
    if (i < w * (h - 1)) stack.push(i + w);
  }
  return true;
}

/** The palette index at (x, y), or -1 outside. */
export function pick(d, x, y) {
  x = Math.floor(x);
  y = Math.floor(y);
  if (x < 0 || y < 0 || x >= d.width || y >= d.height) return -1;
  return d.pixels[y * d.width + x];
}

/** A copy at another size, nearest neighbour. */
export function resizeDrawing(d, width, height) {
  const out = { width, height, pixels: new Uint8Array(width * height), palette: d.palette.map((c) => [...c]) };
  for (let y = 0; y < height; y++) {
    const sy = Math.min(d.height - 1, Math.floor(((y + 0.5) * d.height) / height));
    for (let x = 0; x < width; x++) {
      const sx = Math.min(d.width - 1, Math.floor(((x + 0.5) * d.width) / width));
      out.pixels[y * width + x] = d.pixels[sy * d.width + sx];
    }
  }
  return out;
}

/** RGBA pixels for logo.js and for showing: background is transparent. */
export function drawingToImage(d) {
  const data = new Uint8Array(d.width * d.height * 4);
  for (let i = 0; i < d.pixels.length; i++) {
    const c = d.pixels[i];
    if (!c) continue;
    const [r, g, b] = d.palette[c];
    data.set([r, g, b, 255], i * 4);
  }
  return { width: d.width, height: d.height, data };
}

/** How many palette colours the drawing uses (not counting the background). */
export function coloursUsed(d) {
  const seen = new Set();
  for (const c of d.pixels) if (c) seen.add(c);
  return seen.size;
}

/**
 * Turns RGBA pixels (such as a logo's preview) into a drawing. Transparent
 * and near black pixels become background; past MAX_COLOURS, colours go to
 * the nearest one kept.
 */
export function imageToDrawing(img, { blackCutoff = 24 } = {}) {
  const { width, height, data } = img;
  const pixels = new Uint8Array(width * height);
  const palette = [[0, 0, 0]];
  const index = new Map();
  const nearest = (r, g, b) => {
    let best = 1, bd = Infinity;
    for (let i = 1; i < palette.length; i++) {
      const p = palette[i];
      const dd = (p[0] - r) ** 2 + (p[1] - g) ** 2 + (p[2] - b) ** 2;
      if (dd < bd) [best, bd] = [i, dd];
    }
    return best;
  };
  for (let i = 0; i < width * height; i++) {
    const o = i * 4;
    if (data[o + 3] < 128) continue;
    const r = data[o], g = data[o + 1], b = data[o + 2];
    if (Math.max(r, g, b) < blackCutoff) continue;
    const k = (r << 16) | (g << 8) | b;
    let c = index.get(k);
    if (c === undefined) {
      if (palette.length <= MAX_COLOURS) {
        c = palette.length;
        palette.push(visibleColour([r, g, b]));
      } else c = nearest(r, g, b);
      index.set(k, c);
    }
    pixels[i] = c;
  }
  // Leave a few colours to draw with when the picture had only a couple.
  for (const p of DEFAULT_PALETTE.slice(1)) {
    if (palette.length > MAX_COLOURS || palette.length >= DEFAULT_PALETTE.length) break;
    if (!palette.some((q) => q[0] === p[0] && q[1] === p[1] && q[2] === p[2])) palette.push([...p]);
  }
  return { width, height, pixels, palette };
}

/** The smallest "Colours" setting of logo.js that keeps every colour. */
export function coloursSetting(d) {
  const n = coloursUsed(d);
  return [2, 4, 8, 16, 32].find((k) => k >= n) || 32;
}
