// Shared drawing helpers for the simulated boot scenes. Scenes draw onto a
// 640x480 canvas; `t` is seconds since the boot started.

export const W = 640;
export const H = 480;

export const clamp = (v, a = 0, b = 1) => Math.max(a, Math.min(b, v));
export const ease = (t) => t * t * (3 - 2 * t);
// 0 before a, ramps to 1 by b.
export const ramp = (t, a, b) => clamp((t - a) / (b - a));
// Fades in over [a, a+fin] and out over [b-fout, b].
export const window_ = (t, a, b, fin = 0.5, fout = 0.5) => Math.min(ramp(t, a, a + fin), 1 - ramp(t, b - fout, b));

export function hsl(h, s, l, a = 1) {
  return `hsla(${h}, ${s}%, ${l}%, ${a})`;
}

// Text that falls apart as `g` (0..1) rises: characters get swapped for
// neighbours from the raw bytes and individual glyphs jitter.
export function glitchText(ctx, text, x, y, g, bytes, opts = {}) {
  const { align = 'center', jitter = 6 } = opts;
  if (!g) {
    ctx.textAlign = align;
    ctx.fillText(text, x, y);
    return;
  }
  const chars = [...text];
  ctx.textAlign = 'left';
  const total = ctx.measureText(text).width;
  let cx = align === 'center' ? x - total / 2 : align === 'right' ? x - total : x;
  chars.forEach((c, i) => {
    const b = bytes.length ? bytes[(i * 7) % bytes.length] : 0;
    let ch = c;
    if ((b / 255) < g * 0.5) ch = String.fromCharCode(0x21 + (b % 94));
    const dy = ((b & 15) - 7.5) / 7.5 * jitter * g;
    ctx.fillText(ch, cx, y + dy);
    cx += ctx.measureText(c).width;
  });
}

// Paint bytes as a block of pixels, like garbage in video memory.
export function garbage(ctx, bytes, x, y, w, h, cell = 4, palette = 'rgb') {
  if (!bytes.length) return;
  const cols = Math.max(1, Math.floor(w / cell));
  const rows = Math.max(1, Math.floor(h / cell));
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const i = ((r * cols + c) * 3) % bytes.length;
      const a = bytes[i], b = bytes[(i + 1) % bytes.length], d = bytes[(i + 2) % bytes.length];
      ctx.fillStyle = palette === 'mono' ? `rgb(${a},${a},${a})` : palette === 'green' ? `rgb(${a >> 2},${b},${d >> 2})` : `rgb(${a},${b},${d})`;
      ctx.fillRect(x + c * cell, y + r * cell, cell, cell);
    }
  }
}

// Post effects that every scene can opt into. Each one is bound to its own
// slice of the BIOS, so it only appears when that slice is corrupted.
export function postFx(ctx, t, feed, prefix) {
  const canvas = ctx.canvas;

  // Horizontal tearing of scanline bands.
  const tear = feed.glitch(`${prefix}.post.tear`);
  if (tear) {
    const bands = 2 + Math.floor(tear * 14);
    for (let i = 0; i < bands; i++) {
      const y = Math.floor(((feed.rand(`${prefix}.post.tear`, i) + t * 0.05 * (i % 3)) % 1) * H);
      const h = 2 + Math.floor(feed.rand(`${prefix}.post.tear`, i + 99) * 30 * tear);
      const dx = Math.round((feed.rand(`${prefix}.post.tear`, i + 7) - 0.5) * 120 * tear);
      ctx.drawImage(canvas, 0, y, W, h, dx, y, W, h);
    }
  }

  // Blocks of VRAM-like garbage.
  const blocks = feed.glitch(`${prefix}.post.vram`);
  if (blocks) {
    const n = 1 + Math.floor(blocks * 6);
    for (let i = 0; i < n; i++) {
      const bw = 16 + feed.rand(`${prefix}.post.vram`, i) * 160 * blocks;
      const bh = 8 + feed.rand(`${prefix}.post.vram`, i + 5) * 90 * blocks;
      const x = feed.rand(`${prefix}.post.vram`, i + 11) * (W - bw);
      const y = feed.rand(`${prefix}.post.vram`, i + 17) * (H - bh);
      // Flicker in and out instead of sitting still.
      if (Math.sin(t * (2 + i) + i) > -0.2) garbage(ctx, feed.bytes(`${prefix}.post.vram`, 2048).subarray(i * 97), x, y, bw, bh, 4);
    }
  }

  // Wrong colour depth / swapped channels.
  const swap = feed.glitch(`${prefix}.post.swap`);
  if (swap > 0.3) {
    ctx.save();
    ctx.globalCompositeOperation = 'difference';
    ctx.fillStyle = hsl(feed.rand(`${prefix}.post.swap`) * 360, 80, 50, swap * 0.6);
    ctx.fillRect(0, 0, W, H);
    ctx.restore();
  }

  // Frozen frame: freeze the picture for a moment, like a hung GPU.
  return feed.glitch(`${prefix}.post.freeze`);
}

// A tiny WebAudio chime used by the scenes. Pitch and timing are nudged by
// corruption just like the visuals.
export function chime(audio, notes) {
  if (!audio) return;
  const now = audio.currentTime;
  for (const n of notes) {
    const osc = audio.createOscillator();
    const gain = audio.createGain();
    osc.type = n.type || 'sine';
    osc.frequency.value = n.freq;
    gain.gain.setValueAtTime(0, now + n.at);
    gain.gain.linearRampToValueAtTime(n.vol ?? 0.15, now + n.at + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + n.at + n.len);
    osc.connect(gain).connect(audio.destination);
    osc.start(now + n.at);
    osc.stop(now + n.at + n.len + 0.05);
  }
}

// Fixed pseudo random numbers for scene layout (towers, stars, ...). These are
// the "stock" values, independent of the BIOS.
export function mulberryLite(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), a | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
