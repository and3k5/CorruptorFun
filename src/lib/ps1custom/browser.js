// Browser-only helpers for the boot customizations: decoding pictures and
// sounds the user picks, and drawing letters the BIOS pictures don't have.

// Fonts close to the originals: a heavy slab serif for SONY, a plain
// grotesque for COMPUTER ENTERTAINMENT and PlayStation.
const FONTS = {
  serif: '900 {px}px "Clarendon", "Rockwell Extra Bold", "Rockwell", "Georgia", serif',
  sans: '400 {px}px "Helvetica Neue", Helvetica, Arial, sans-serif',
};

let ctx = null;
function context(w, h) {
  if (!ctx) ctx = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
  ctx.canvas.width = w;
  ctx.canvas.height = h;
  ctx.clearRect(0, 0, w, h);
  return ctx;
}

// Ink width of one character drawn at size px.
function inkBox(ch, font, px, h, baseline) {
  const c = context(px * 3, h);
  c.font = font.replace('{px}', px);
  c.fillStyle = '#fff';
  c.textBaseline = 'alphabetic';
  c.fillText(ch, px, baseline);
  const { data, width } = c.getImageData(0, 0, c.canvas.width, h);
  let lo = width, hi = -1;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < width; x++) {
      if (data[(y * width + x) * 4 + 3] > 40) {
        lo = Math.min(lo, x);
        hi = Math.max(hi, x);
      }
    }
  }
  return { lo, hi, data, width };
}

/**
 * Draws a character the BIOS picture doesn't have, matching the line's cap
 * height and letter width. Signature as wordmark.js's compose() expects.
 */
export function canvasFallback(ch, { height, capTop, baseline, style, ref }) {
  const font = FONTS[style] || FONTS.sans;
  const capH = Math.max(4, baseline - capTop);
  // Size the font so its capitals are as tall as the original's.
  const c = context(400, 400);
  c.font = font.replace('{px}', 200);
  const m = c.measureText('H');
  const px = Math.max(4, Math.round((200 * capH) / (m.actualBoundingBoxAscent || 140)));
  // Stretch sideways so a letter the original has comes out as wide.
  const r = inkBox(ref.ch, font, px, height, baseline);
  const stretch = r.hi >= r.lo ? ref.w / (r.hi - r.lo + 1) : 1;
  const g = inkBox(ch, font, px, height, baseline);
  if (g.hi < g.lo) return null;
  const w0 = g.hi - g.lo + 1;
  const w = Math.max(1, Math.round(w0 * stretch));
  const cov = new Float32Array(w * height);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < w; x++) {
      const sx = g.lo + Math.min(w0 - 1, Math.floor(x / stretch));
      cov[y * w + x] = g.data[(y * g.width + sx) * 4 + 3] / 255;
    }
  }
  return { w, h: height, cov };
}

/** Decodes any picture the browser can show into { width, height, data }. */
export async function decodeImage(file, maxSide = 256) {
  const bmp = await createImageBitmap(file);
  const s = Math.min(1, maxSide / Math.max(bmp.width, bmp.height));
  const w = Math.max(1, Math.round(bmp.width * s)), h = Math.max(1, Math.round(bmp.height * s));
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  const g = c.getContext('2d');
  g.drawImage(bmp, 0, 0, w, h);
  const { data } = g.getImageData(0, 0, w, h);
  return { width: w, height: h, data: new Uint8Array(data.buffer) };
}

/** Draws RGBA pixels into a data URL, scaled up crisply, for previews. */
export function pixelsToUrl({ width, height, data }, scale = 4) {
  const c = document.createElement('canvas');
  c.width = width;
  c.height = height;
  c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(data), width, height), 0, 0);
  const big = document.createElement('canvas');
  big.width = width * scale;
  big.height = height * scale;
  const g = big.getContext('2d');
  g.imageSmoothingEnabled = false;
  g.drawImage(c, 0, 0, big.width, big.height);
  return big.toDataURL();
}

// The most a sample could ever use: all of the sound bank at the lowest rate.
const MAX_SECONDS = 7;

/**
 * Decodes any sound the browser can play (WAV, MP3, OGG, FLAC, AAC, the
 * sound of a video…) to mono at 44.1 kHz. Returns { pcm, rate, seconds }.
 */
export async function decodeAudio(file) {
  const bytes = await file.arrayBuffer();
  const Ctx = window.OfflineAudioContext || window.webkitOfflineAudioContext;
  const probe = new Ctx(1, 1, 44100);
  const buf = await probe.decodeAudioData(bytes);
  const seconds = buf.duration;
  const len = Math.min(buf.length, Math.ceil(buf.sampleRate * MAX_SECONDS));
  // Mix down and resample in one go.
  const off = new Ctx(1, Math.ceil((len / buf.sampleRate) * 44100), 44100);
  const src = off.createBufferSource();
  src.buffer = buf;
  src.connect(off.destination);
  src.start();
  const out = await off.startRendering();
  // Trim silence at the start, so the note starts when the BIOS plays it.
  const pcm = out.getChannelData(0);
  let start = 0;
  while (start < pcm.length - 1 && Math.abs(pcm[start]) < 0.01) start++;
  return { pcm: pcm.slice(start), rate: 44100, seconds };
}

let player = null;
/** Plays mono samples, stopping whatever was playing before. */
export function playPcm(pcm, rate) {
  player?.stop();
  const ctx = new AudioContext();
  const buf = ctx.createBuffer(1, pcm.length, rate);
  buf.copyToChannel(pcm instanceof Float32Array ? pcm : Float32Array.from(pcm, (s) => s / 32768), 0);
  const src = ctx.createBufferSource();
  src.buffer = buf;
  src.connect(ctx.destination);
  src.onended = () => ctx.close();
  src.start();
  player = { stop: () => { try { src.stop(); } catch { /* already stopped */ } } };
}
