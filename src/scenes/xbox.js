// Simulated original Xbox boot: green energy ribbons spiral in, flash, and
// settle into the X emblem and wordmark. A stylised re-creation; the real
// animation is rendered by code inside the (encrypted) BIOS.
import { W, H, ramp, ease, window_, hsl, glitchText, garbage, postFx, chime } from './common.js';

const RIBBONS = 7;

function ribbons(ctx, t, feed, cx, cy) {
  const pull = ease(ramp(t, 0.3, 3.4));
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  ctx.lineCap = 'round';
  for (let r = 0; r < RIBBONS; r++) {
    const n = `xbox.ribbon${r}`;
    const hue = feed.value(`${n}.hue`, 110, 200);
    const spin = feed.value(`${n}.spin`, 1.6, 4);
    const wob = feed.value(`${n}.wobble`, 0, 1.5);
    const r0 = 420 * (1 - pull) + 70 + feed.value(`${n}.radius`, 0, 160);
    ctx.beginPath();
    for (let k = 0; k <= 60; k++) {
      const u = k / 60;
      const ang = (r / RIBBONS) * Math.PI * 2 + t * spin + u * 2.6;
      const rad = r0 * (1 - u * 0.65) * (1 + Math.sin(u * 9 + t * 3) * wob * 0.2);
      const x = cx + Math.cos(ang) * rad;
      const y = cy + Math.sin(ang) * rad * feed.value(`${n}.flat`, 0.75, 0.6);
      k ? ctx.lineTo(x, y) : ctx.moveTo(x, y);
    }
    const a = window_(t, 0.2, 4.4, 0.8, 0.6);
    ctx.strokeStyle = hsl(hue, 90, 55, 0.35 * a);
    ctx.lineWidth = feed.value(`${n}.width`, 10, 18);
    ctx.stroke();
    ctx.strokeStyle = hsl(hue + 20, 100, 85, 0.5 * a);
    ctx.lineWidth = 2;
    ctx.stroke();
  }
  ctx.restore();
}

function emblem(ctx, t, feed, cx, cy) {
  const a = ramp(t, 3.8, 4.6);
  if (!a) return;
  const R = feed.value('xbox.emblem.r', 70, 60);
  // Glowing sphere.
  const g = ctx.createRadialGradient(cx - R * 0.3, cy - R * 0.3, R * 0.1, cx, cy, R);
  g.addColorStop(0, hsl(feed.value('xbox.emblem.hue', 105, 160), 70, 75, a));
  g.addColorStop(0.7, hsl(feed.value('xbox.emblem.hue', 105, 160), 85, 35, a));
  g.addColorStop(1, hsl(feed.value('xbox.emblem.hue', 105, 160), 90, 15, a));
  ctx.fillStyle = g;
  ctx.beginPath();
  ctx.arc(cx, cy, R, 0, Math.PI * 2);
  ctx.fill();
  // The X: two crossing strokes that bow outwards.
  ctx.strokeStyle = hsl(0, 0, feed.value('xbox.emblem.xl', 8, 80), a);
  ctx.lineWidth = feed.value('xbox.emblem.xw', R * 0.28, 30);
  ctx.lineCap = 'butt';
  const k = R * 0.75;
  for (const s of [1, -1]) {
    ctx.beginPath();
    ctx.moveTo(cx - k, cy - k * s);
    ctx.quadraticCurveTo(cx + feed.value(`xbox.emblem.bend${s}`, 0, 60), cy, cx + k, cy + k * s);
    ctx.stroke();
  }
}

export default {
  id: 'xbox',
  duration: () => 9,
  sound(audio, feed) {
    const p = feed.value('xbox.sound.pitch', 1, 0.5);
    chime(audio, [
      { freq: 73 * p, at: 0.2, len: 4, vol: 0.14, type: 'sawtooth' },
      { freq: 146 * p, at: 1.5, len: 2.8, vol: 0.05, type: 'square' },
      { freq: 587 * p * feed.value('xbox.sound.hit', 1, 0.5), at: 3.9, len: 2.5, vol: 0.07 },
      { freq: 880 * p, at: 4.0, len: 2.5, vol: 0.04 },
    ]);
  },
  draw(ctx, t, feed) {
    const cx = W / 2 + feed.value('xbox.center.x', 0, 200);
    const cy = H / 2 - 30 + feed.value('xbox.center.y', 0, 150);
    ctx.fillStyle = hsl(feed.value('xbox.bg.hue', 120, 180), 60, feed.value('xbox.bg.l', 3, 25));
    ctx.fillRect(0, 0, W, H);

    ribbons(ctx, t, feed, cx, cy);

    // Flash as the energy collapses into the emblem.
    const flash = ramp(t, 3.4, 3.9) * (1 - ramp(t, 3.9, 4.8));
    if (flash) {
      const fg = ctx.createRadialGradient(cx, cy, 0, cx, cy, 380);
      fg.addColorStop(0, hsl(100, 100, 92, flash));
      fg.addColorStop(1, hsl(120, 100, 40, 0));
      ctx.fillStyle = fg;
      ctx.fillRect(0, 0, W, H);
    }

    emblem(ctx, t, feed, cx, cy);

    const ta = ramp(t, 4.6, 5.4);
    if (ta) {
      ctx.fillStyle = hsl(feed.value('xbox.word.hue', 95, 160), 70, 60, ta);
      ctx.font = `bold ${Math.round(feed.value('xbox.word.size', 52, 50))}px "Arial Black", Arial, sans-serif`;
      const letters = feed.glitch('xbox.word.letters') ? 'XBOX'.split('').map((c, i) => (feed.rand('xbox.word.letters', i) < 0.5 ? c : 'OXBX'[i])).join('') : 'XBOX';
      glitchText(ctx, letters.split('').join(' '), W / 2, cy + 140, feed.glitch('xbox.word.text'), feed.bytes('xbox.word.text'));
      ctx.fillStyle = hsl(0, 0, 80, ta * 0.8);
      ctx.font = '14px Arial, sans-serif';
      glitchText(ctx, 'Microsoft', W / 2, cy + 170, feed.glitch('xbox.word.ms'), feed.bytes('xbox.word.ms'));
    }

    // A hung 2BL on real hardware tends to leave green garbage on screen.
    const hang = feed.glitch('xbox.hang');
    if (hang && t > 5) garbage(ctx, feed.bytes('xbox.hang', 4096), 0, H * (1 - hang * 0.6), W, H * hang * 0.6, 6, 'green');

    postFx(ctx, t, feed, 'xbox');
  },
};
