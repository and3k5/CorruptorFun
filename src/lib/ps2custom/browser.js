// Browser-only helpers for the PS2 customizations: drawing text and pictures
// into the BIOS's 8-bit brightness pictures (see index.js), and previews.

import { DISC, SCE } from './index.js';

function canvas(w, h) {
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  return c;
}

// Brightness from a canvas: white drawn on transparent, so alpha is the value.
function alphaOf(c, scale = 1) {
  const { data } = c.getContext('2d').getImageData(0, 0, c.width, c.height);
  const out = new Uint8Array(c.width * c.height);
  for (let i = 0; i < out.length; i++) out[i] = Math.round(data[i * 4 + 3] * scale);
  return out;
}

// Draws `text` centred in a box, shrinking it to fit the width.
function fitText(g, text, x, y, maxW, px, weight = '600') {
  let size = px;
  do {
    g.font = `${weight} ${size}px "Helvetica Neue", Arial, sans-serif`;
    if (g.measureText(text).width <= maxW) break;
    size -= 1;
  } while (size > 8);
  g.fillText(text, x, y, maxW);
}

/** The intro line drawn as text: a SCE.strip × SCE.rows brightness strip. */
export function textToStrip(text) {
  const c = canvas(SCE.strip, SCE.rows);
  const g = c.getContext('2d');
  g.fillStyle = '#fff';
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  // The intro shows the strip at about half width, so the letters are wide.
  g.save();
  g.scale(2, 1);
  fitText(g, text, SCE.strip / 4, SCE.rows / 2 + 1, SCE.strip / 2 - 8, 25);
  g.restore();
  return alphaOf(c);
}

/** A picture ({ width, height, data: RGBA }) fitted into the strip. */
export function imageToStrip(img) {
  const src = canvas(img.width, img.height);
  src.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(img.data), img.width, img.height), 0, 0);
  // Shown at half width, so the picture is stretched 2× across.
  const s = Math.min(SCE.strip / (img.width * 2), SCE.rows / img.height);
  const w = Math.max(1, Math.round(img.width * 2 * s)), h = Math.max(1, Math.round(img.height * s));
  const c = canvas(SCE.strip, SCE.rows);
  const g = c.getContext('2d');
  g.imageSmoothingEnabled = w < img.width * 4; // pixel art scaled up a lot stays sharp
  g.drawImage(src, Math.round((SCE.strip - w) / 2), Math.round((SCE.rows - h) / 2), w, h);
  const { data } = g.getImageData(0, 0, c.width, c.height);
  const out = new Uint8Array(SCE.strip * SCE.rows);
  for (let i = 0; i < out.length; i++) {
    const [r, gr, b, a] = [data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]];
    out[i] = Math.round(((0.3 * r + 0.59 * gr + 0.11 * b) * a) / 255);
  }
  return out;
}

/** The disc message: two centred lines in a 256 × 128 picture. */
export function discImage(line1, line2) {
  const c = canvas(DISC.width, DISC.height);
  const g = c.getContext('2d');
  g.fillStyle = '#fff';
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  fitText(g, line1, DISC.width / 2, 40, DISC.width - 8, 15, '500');
  fitText(g, line2, DISC.width / 2, 86, DISC.width - 8, 15, '500');
  return alphaOf(c);
}

/** A brightness picture as a data URL, white on black, `sx` × `sy` scaled. */
export function grayToUrl(gray, width, height, sx = 1, sy = 1) {
  const c = canvas(width, height);
  const g = c.getContext('2d');
  const img = g.createImageData(width, height);
  for (let i = 0; i < gray.length; i++) {
    img.data[i * 4] = img.data[i * 4 + 1] = img.data[i * 4 + 2] = gray[i];
    img.data[i * 4 + 3] = 255;
  }
  g.putImageData(img, 0, 0);
  const big = canvas(Math.round(width * sx), Math.round(height * sy));
  const b = big.getContext('2d');
  b.imageSmoothingEnabled = true;
  b.drawImage(c, 0, 0, big.width, big.height);
  return big.toDataURL();
}
