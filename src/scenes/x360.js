// Simulated Xbox 360 boot: the green orb with white light ribbons swirling
// around it, resolving into the wordmark. A stylised re-creation; the real
// animation lives in the console's encrypted flash.
import { W, H, ramp, ease, window_, hsl, glitchText, garbage, postFx, chime, mulberryLite } from './common.js';

const rnd = mulberryLite(360);
const SPARKS = Array.from({ length: 70 }, () => ({ a: rnd() * Math.PI * 2, r: 90 + rnd() * 160, s: 0.3 + rnd() * 1.2 }));

function orb(ctx, t, feed, cx, cy) {
  const grow = ease(ramp(t, 0.4, 2.4));
  const R = feed.value('x360.orb.r', 78, 70) * grow * (1 + Math.sin(t * 4) * 0.015 * (1 + feed.glitch('x360.orb.pulse') * 20));
  if (R <= 1) return;
  const hue = feed.value('x360.orb.hue', 95, 200);
  // Outer glow.
  const glow = ctx.createRadialGradient(cx, cy, R * 0.8, cx, cy, R * 2.4);
  glow.addColorStop(0, hsl(hue, 90, 60, 0.35));
  glow.addColorStop(1, hsl(hue, 90, 60, 0));
  ctx.fillStyle = glow;
  ctx.fillRect(cx - R * 2.5, cy - R * 2.5, R * 5, R * 5);
  // The orb itself, squashed if its slice is corrupted.
  ctx.save();
  ctx.translate(cx, cy);
  ctx.scale(feed.value('x360.orb.sx', 1, 0.7), feed.value('x360.orb.sy', 1, 0.7));
  const g = ctx.createRadialGradient(-R * 0.35, -R * 0.4, R * 0.05, 0, 0, R);
  g.addColorStop(0, hsl(hue - 10, 100, 92));
  g.addColorStop(0.35, hsl(hue, 85, 58));
  g.addColorStop(1, hsl(hue + 10, 90, 24));
  ctx.fillStyle = g;
  ctx.beginPath();
  ctx.arc(0, 0, R, 0, Math.PI * 2);
  ctx.fill();
  // The silver "X" swoosh across the orb.
  const xa = ramp(t, 3.6, 4.4);
  if (xa) {
    ctx.strokeStyle = hsl(0, 0, feed.value('x360.x.l', 96, 60), xa);
    ctx.lineWidth = feed.value('x360.x.w', R * 0.16, 20);
    ctx.lineCap = 'round';
    const k = R * 0.62;
    for (const s of [1, -1]) {
      ctx.beginPath();
      ctx.moveTo(-k, -k * s);
      ctx.quadraticCurveTo(feed.value(`x360.x.bend${s}`, 0, 50), feed.value(`x360.x.mid${s}`, 0, 50), k, k * s);
      ctx.stroke();
    }
  }
  ctx.restore();
}

function rings(ctx, t, feed, cx, cy) {
  const a = window_(t, 0.8, 4.6, 0.6, 0.8);
  if (!a) return;
  ctx.save();
  ctx.globalCompositeOperation = feed.glitch('x360.rings.blend') ? 'difference' : 'source-over';
  for (let i = 0; i < 3; i++) {
    const n = `x360.ring${i}`;
    const tighten = ease(ramp(t, 1.0, 4.2));
    const rx = (300 - tighten * 170) * feed.value(`${n}.rx`, 1, 0.8);
    const ry = rx * feed.value(`${n}.ry`, 0.28 + i * 0.08, 0.5);
    const rot = t * feed.value(`${n}.spin`, 0.9 + i * 0.4, 3) + i * 1.1;
    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(feed.value(`${n}.tilt`, -0.35 + i * 0.3, 1.5));
    // Draw the ring as a fading arc so it reads as a moving ribbon.
    for (let k = 0; k < 24; k++) {
      const u0 = rot + (k / 24) * Math.PI * 1.4;
      ctx.beginPath();
      ctx.ellipse(0, 0, rx, Math.abs(ry), 0, u0, u0 + 0.08);
      ctx.strokeStyle = hsl(feed.value(`${n}.hue`, 100, 200), feed.glitch(`${n}.hue`) * 90 + 10, 70 + k, (k / 24) * a);
      ctx.lineWidth = feed.value(`${n}.w`, 5, 10);
      ctx.stroke();
    }
    ctx.restore();
  }
  ctx.restore();
  // Sparks thrown off the rings.
  ctx.fillStyle = hsl(90, 90, 60, a * 0.8);
  for (const s of SPARKS) {
    const ang = s.a + t * s.s * feed.value('x360.sparks.spin', 1, 3);
    const rr = s.r * (1 - ease(ramp(t, 1, 4.4)) * 0.5);
    ctx.fillRect(cx + Math.cos(ang) * rr, cy + Math.sin(ang) * rr * 0.4, 2, 2);
  }
}

export default {
  id: 'x360',
  duration: () => 9,
  sound(audio, feed) {
    const p = feed.value('x360.sound.pitch', 1, 0.5);
    chime(audio, [
      { freq: 196 * p, at: 0.4, len: 3.4, vol: 0.06, type: 'triangle' },
      { freq: 294 * p, at: 1.2, len: 3.0, vol: 0.05 },
      { freq: 392 * p * feed.value('x360.sound.ding', 1, 0.4), at: 3.6, len: 3.0, vol: 0.08 },
      { freq: 587 * p, at: 3.7, len: 3.0, vol: 0.05 },
    ]);
  },
  draw(ctx, t, feed) {
    const cx = W / 2 + feed.value('x360.center.x', 0, 200);
    const cy = H / 2 - 30 + feed.value('x360.center.y', 0, 150);
    const bg = ctx.createRadialGradient(W / 2, H / 2, 40, W / 2, H / 2, 480);
    const bgL = feed.value('x360.bg.l', 97, 70);
    bg.addColorStop(0, hsl(feed.value('x360.bg.hue', 0, 360), feed.glitch('x360.bg.hue') * 80, bgL));
    bg.addColorStop(1, hsl(0, 0, bgL - 22));
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, W, H);
    rings(ctx, t, feed, cx, cy);
    orb(ctx, t, feed, cx, cy);

    const ta = ramp(t, 4.4, 5.2);
    if (ta) {
      ctx.fillStyle = hsl(0, 0, feed.value('x360.word.l', 35, 50), ta);
      ctx.font = `${Math.round(feed.value('x360.word.size', 40, 40))}px "Segoe UI", Arial, sans-serif`;
      glitchText(ctx, 'XBOX 360', W / 2, cy + 150, feed.glitch('x360.word.text'), feed.bytes('x360.word.text'));
    }
    const corrupt = feed.glitch('x360.rrod');
    if (corrupt && t > 5.5) garbage(ctx, feed.bytes('x360.rrod', 4096), 0, 0, W, H * corrupt * 0.4, 5);

    // Fade in from black.
    ctx.fillStyle = `rgba(0,0,0,${1 - ramp(t, 0, 0.6)})`;
    ctx.fillRect(0, 0, W, H);
    postFx(ctx, t, feed, 'x360');
  },
};
