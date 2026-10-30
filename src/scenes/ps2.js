// Simulated PlayStation 2 boot: the dark field of glowing towers, the white
// flash into the SCE screen, then either the PS2 logo (licensed disc) or the
// browser menu. A stylised re-creation, not the real OSDSYS renderer.
import { W, H, ramp, ease, window_, hsl, glitchText, garbage, postFx, chime, mulberryLite } from './common.js';

const GRID = 11;
const stock = mulberryLite(2000);
const TOWERS = [];
for (let i = 0; i < GRID; i++) {
  for (let j = 0; j < GRID; j++) {
    TOWERS.push({ x: (i - GRID / 2) * 60, z: j * 60 + 120, h: 20 + stock() * stock() * 220, group: (i * GRID + j) % 16 });
  }
}
const PARTICLES = Array.from({ length: 140 }, () => ({ x: (stock() - 0.5) * 900, y: stock() * 400, z: 80 + stock() * 700, s: 0.5 + stock() * 2 }));

function project(x, y, z, cam) {
  const dz = z - cam.z;
  if (dz < 10) return null;
  const f = cam.fov / dz;
  return [W / 2 + (x - cam.x) * f, H * 0.62 - (y - cam.y) * f, f];
}

function towers(ctx, t, feed) {
  ctx.fillStyle = hsl(feed.value('ps2.sky.hue', 225, 180), 60, feed.value('ps2.sky.l', 3, 25));
  ctx.fillRect(0, 0, W, H);
  const fade = window_(t, 0, 6.2, 1.2, 0.4);
  const cam = {
    x: Math.sin(t * 0.3) * 40 + feed.value('ps2.cam.x', 0, 300),
    y: 160 + feed.value('ps2.cam.y', 0, 200) - t * 6,
    z: -200 + t * feed.value('ps2.cam.speed', 55, 120),
    fov: feed.value('ps2.cam.fov', 380, 300),
  };
  const hue = feed.value('ps2.tower.hue', 215, 180);
  const list = TOWERS
    .map((tw) => ({ ...tw, h: tw.h * feed.value(`ps2.tower.g${tw.group}`, 1, 2.5) * ease(ramp(t, 0.5 + tw.group * 0.08, 2.5 + tw.group * 0.08)) }))
    .sort((a, b) => b.z - a.z);
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  for (const tw of list) {
    const base = project(tw.x - 18, 0, tw.z, cam);
    const top = project(tw.x + 18, tw.h, tw.z, cam);
    if (!base || !top) continue;
    const depth = Math.max(0, 1 - (tw.z - cam.z) / 900);
    const g = ctx.createLinearGradient(0, base[1], 0, top[1]);
    g.addColorStop(0, hsl(hue, 70, 45, 0.05 * fade * depth));
    g.addColorStop(1, hsl(hue + 20, 90, 70, 0.55 * fade * depth));
    ctx.fillStyle = g;
    ctx.fillRect(base[0], top[1], top[0] - base[0], base[1] - top[1]);
    // The bright cap on each tower.
    ctx.fillStyle = hsl(hue + 30, 100, 85, 0.6 * fade * depth);
    ctx.fillRect(base[0], top[1] - 2, top[0] - base[0], 3);
  }
  // Drifting light particles.
  const pc = feed.value('ps2.particles.n', 1, 1);
  for (let i = 0; i < PARTICLES.length * Math.max(0.1, pc); i++) {
    const p = PARTICLES[i % PARTICLES.length];
    const y = (p.y + t * 20 * p.s) % 400;
    const q = project(p.x, y, p.z, cam);
    if (!q) continue;
    ctx.fillStyle = hsl(feed.value('ps2.particles.hue', 200, 180), 80, 85, 0.7 * fade);
    const r = Math.max(0.5, q[2] * 2.5 * p.s);
    ctx.fillRect(q[0], q[1], r, r);
  }
  ctx.restore();

  // White flash at the end of the tower sequence.
  const flash = ramp(t, 5.4, 6.2) * (1 - ramp(t, 6.6, 7.4));
  if (flash > 0) {
    ctx.fillStyle = hsl(feed.value('ps2.flash.hue', 0, 360), feed.glitch('ps2.flash.hue') * 100, 100, flash);
    ctx.fillRect(0, 0, W, H);
  }
}

function sceText(ctx, t, feed) {
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, W, H);
  const a = window_(t, 0, 3.6, 0.8, 0.8);
  ctx.fillStyle = hsl(feed.value('ps2.sce.hue', 0, 360), feed.glitch('ps2.sce.hue') * 90, 92, a);
  ctx.font = `${Math.round(feed.value('ps2.sce.size', 24, 26))}px Helvetica, Arial, sans-serif`;
  ctx.shadowColor = hsl(210, 100, 70, a);
  ctx.shadowBlur = 18;
  const name = feed.text('company');
  glitchText(ctx, name, W / 2 + feed.value('ps2.sce.x', 0, 200), H / 2 + feed.value('ps2.sce.y', 0, 160), feed.glitch('ps2.sce.text'), feed.bytes('ps2.sce.text'));
  ctx.shadowBlur = 0;
}

function ps2Logo(ctx, t, feed) {
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, W, H);
  const a = window_(t, 0, 4.6, 1.0, 0.8);
  ctx.save();
  ctx.translate(W / 2 + feed.value('ps2.logo.x', 0, 200), H / 2 + feed.value('ps2.logo.y', 0, 150));
  ctx.rotate(feed.value('ps2.logo.rot', 0, 0.6));
  const sc = feed.value('ps2.logo.scale', 1, 0.7);
  ctx.scale(sc, sc);
  ctx.font = `italic bold 44px Helvetica, Arial, sans-serif`;
  const g = ctx.createLinearGradient(-200, 0, 200, 0);
  g.addColorStop(0, hsl(feed.value('ps2.logo.hue', 220, 160), 90, 60, a));
  g.addColorStop(1, hsl(feed.value('ps2.logo.hue2', 200, 160), 100, 85, a));
  ctx.fillStyle = g;
  glitchText(ctx, 'PlayStation', -20, 10, feed.glitch('ps2.logo.word'), feed.bytes('ps2.logo.word'));
  ctx.font = `bold ${Math.round(feed.value('ps2.logo.two', 64, 60))}px Helvetica, Arial, sans-serif`;
  ctx.fillText('2', 160, 18);
  ctx.restore();
  // The sweeping light band under the logo.
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  const x = ((t * 0.5) % 1.4 - 0.2) * W;
  const lg = ctx.createRadialGradient(x, H / 2 + 50, 0, x, H / 2 + 50, 160);
  lg.addColorStop(0, hsl(210, 100, 70, 0.35 * a));
  lg.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = lg;
  ctx.fillRect(0, H / 2 + 30, W, 40);
  ctx.restore();
  if (feed.glitch('ps2.logo.vram')) garbage(ctx, feed.bytes('ps2.logo.vram', 2048), 0, H - 60, W, 60 * feed.glitch('ps2.logo.vram'), 4, 'mono');
}

function browser(ctx, t, feed) {
  const g = ctx.createRadialGradient(W / 2, H / 2, 40, W / 2, H / 2, 420);
  g.addColorStop(0, hsl(feed.value('ps2.menu.hue', 215, 160), 40, 22));
  g.addColorStop(1, hsl(feed.value('ps2.menu.hue', 215, 160), 60, 4));
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, W, H);
  const a = ramp(t, 0, 1);
  // Slowly orbiting translucent cubes in the background.
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  for (let i = 0; i < 8; i++) {
    const ang = t * 0.25 + (i / 8) * Math.PI * 2 + feed.value(`ps2.menu.orbit${i}`, 0, 3);
    const r = 170 + feed.value(`ps2.menu.r${i}`, 0, 200);
    const x = W / 2 + Math.cos(ang) * r;
    const y = H / 2 + Math.sin(ang) * r * 0.35;
    const s = 26 + Math.sin(ang) * 10;
    ctx.fillStyle = hsl(205, 80, 70, 0.15 * a);
    ctx.fillRect(x - s / 2, y - s / 2, s, s);
  }
  ctx.restore();
  const items = [feed.text('browser'), feed.text('config')];
  items.forEach((label, i) => {
    const n = `ps2.menu.item${i}`;
    const sel = i === Math.floor(t / 3) % 2;
    ctx.fillStyle = sel ? hsl(200, 90, 85, a) : hsl(210, 30, 70, a * 0.7);
    ctx.font = `${sel ? 'bold ' : ''}26px Helvetica, Arial, sans-serif`;
    glitchText(ctx, label, W / 2 + feed.value(`${n}.x`, 0, 200), H / 2 - 20 + i * 60 + feed.value(`${n}.y`, 0, 120), feed.glitch(`${n}.text`), feed.bytes(`${n}.text`));
  });
}

export default {
  id: 'ps2',
  duration: (opts) => (opts.licensed ? 16.5 : 17),
  sound(audio, feed) {
    const p = feed.value('ps2.sound.pitch', 1, 0.5);
    chime(audio, [
      { freq: 55 * p, at: 0.4, len: 6, vol: 0.12, type: 'sawtooth' },
      { freq: 110 * p, at: 1.4, len: 5, vol: 0.04, type: 'triangle' },
      { freq: 880 * p * feed.value('ps2.sound.shine', 1, 0.4), at: 6.2, len: 2.5, vol: 0.05 },
      { freq: 1320 * p, at: 6.3, len: 2.2, vol: 0.03 },
    ]);
  },
  draw(ctx, t, feed, opts) {
    if (t < 7.4) towers(ctx, t, feed);
    else if (t < 11.4) sceText(ctx, t - 7.4, feed);
    else if (opts.licensed) ps2Logo(ctx, t - 11.6, feed);
    else browser(ctx, t - 11.4, feed);
    postFx(ctx, t, feed, 'ps2');
  },
};
