// Browser-only helpers for the boot customizations: decoding pictures and
// sounds the user picks, recording, measuring how the BIOS plays its sounds,
// and drawing letters the BIOS pictures don't have.

import wasmUrl from '../../wasm/ps1.wasm?url';
import { buildTestDisc } from '../testdisc.js';
import { sampleUse } from './notes.js';

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

/**
 * Records from the microphone. Returns { stop(), done } where `done`
 * resolves to the recording as a Blob once stopped (or after maxSeconds).
 *
 * The silence before you start is trimmed later (see decodeAudio), so the
 * recording also listens for when the sound starts. `onTick` gets
 * { seconds, sound, level }: time recorded, time since the sound started
 * (null before), and the input level (0..1). `autoStop()` is asked on every
 * tick for how long after the sound starts to stop (null: don't).
 */
export async function recordAudio({ maxSeconds = MAX_SECONDS, onTick, autoStop } = {}) {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
  });
  const rec = new MediaRecorder(stream);
  const chunks = [];
  rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  // Level meter, and the moment the sound starts.
  let ac = null, analyser = null, buf = null;
  try {
    ac = new AudioContext();
    analyser = ac.createAnalyser();
    analyser.fftSize = 2048;
    ac.createMediaStreamSource(stream).connect(analyser);
    buf = new Float32Array(analyser.fftSize);
  } catch {
    analyser = null; // no meter; the time still counts
  }
  const started = performance.now();
  let soundAt = analyser ? null : started;
  const timer = setInterval(() => {
    const now = performance.now();
    let level = 0;
    if (analyser) {
      analyser.getFloatTimeDomainData(buf);
      for (const x of buf) level = Math.max(level, Math.abs(x));
      // The same threshold decodeAudio trims at, a bit above it against noise.
      if (soundAt === null && level > 0.02) soundAt = now - (buf.length / ac.sampleRate) * 500;
    }
    const seconds = (now - started) / 1000;
    const sound = soundAt === null ? null : Math.max(0, (now - soundAt) / 1000);
    onTick?.({ seconds, sound, level });
    const after = autoStop?.();
    if (seconds >= maxSeconds || (after && sound !== null && sound >= after)) stop();
  }, 50);
  const done = new Promise((resolve) => {
    rec.onstop = () => {
      clearInterval(timer);
      stream.getTracks().forEach((t) => t.stop());
      ac?.close();
      resolve(new Blob(chunks, { type: rec.mimeType }));
    };
  });
  function stop() {
    if (rec.state !== 'inactive') rec.stop();
  }
  rec.start();
  return { stop, done };
}

const measured = new WeakMap();
/**
 * Boots the untouched BIOS headlessly with the test disc (so the licensed
 * screen chime plays too) and works out how it plays each boot sample; see
 * notes.js. Resolves to sampleUse()'s result, or null. Cached per BIOS.
 */
export function measureSamples(bios, region) {
  const key = bios;
  const hit = measured.get(key);
  if (hit?.region === region) return hit.promise;
  const promise = (async () => {
    const disc = buildTestDisc(region); // with a program that just loops
    const w = new Worker(new URL('../../emu/ps1-probe.worker.js', import.meta.url), { type: 'module' });
    try {
      const r = await new Promise((resolve, reject) => {
        w.onmessage = (ev) => (ev.data.error ? reject(new Error(ev.data.error)) : resolve(ev.data));
        w.onerror = (e) => reject(new Error(e.message || 'worker failed'));
        w.postMessage({ type: 'init', wasmUrl: new URL(wasmUrl, location.href).href });
        const b = bios.slice().buffer;
        const d = disc.slice().buffer;
        w.postMessage({ type: 'notes', id: 0, bios: b, disc: d, region, frames: 60 * 40 }, [b, d]);
      });
      return sampleUse(bios, r.ram, r.log, { until: r.until });
    } finally {
      w.terminate();
    }
  })();
  promise.catch(() => measured.delete(key));
  measured.set(key, { region, promise });
  return promise;
}
