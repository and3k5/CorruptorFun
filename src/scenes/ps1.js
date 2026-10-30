// Simulated PlayStation boot: white SCE screen, then either the black
// "licensed" screen leading into a game, or the shell when there's no
// licensed disc. A stylised re-creation, not the real BIOS renderer.
import { W, H, ramp, ease, window_, hsl, glitchText, garbage, postFx, chime } from './common.js';

function regionName(feed) {
  const v = feed.text('version');
  const last = v.trim().slice(-1);
  return { E: 'Europe', A: 'America', J: 'Inc.' }[last] || 'Europe';
}

function diamond(ctx, t, feed, cx, cy) {
  // Four facets that slide together to form the diamond.
  const s = 70;
  const facets = [
    { pts: [[0, -s], [-s, 0], [0, 0]], col: [48, 95, 55], from: [-1, -1] },
    { pts: [[0, -s], [s, 0], [0, 0]], col: [30, 95, 50], from: [1, -1] },
    { pts: [[-s, 0], [0, s], [0, 0]], col: [5, 85, 50], from: [-1, 1] },
    { pts: [[s, 0], [0, s], [0, 0]], col: [350, 70, 45], from: [1, 1] },
  ];
  const k = ease(ramp(t, 0.6, 2.0));
  facets.forEach((f, i) => {
    const n = `ps1.diamond.f${i}`;
    const fly = (1 - k) * 260;
    const ox = f.from[0] * fly + feed.value(`${n}.x`, 0, 90);
    const oy = f.from[1] * fly + feed.value(`${n}.y`, 0, 90);
    const hue = feed.value(`${n}.hue`, f.col[0], 180);
    ctx.fillStyle = hsl(hue, f.col[1], feed.value(`${n}.light`, f.col[2], 40), ramp(t, 0.5, 1.2));
    ctx.beginPath();
    f.pts.forEach(([x, y], j) => {
      const vx = x + feed.value(`${n}.v${j}x`, 0, 120);
      const vy = y + feed.value(`${n}.v${j}y`, 0, 120);
      j ? ctx.lineTo(cx + ox + vx, cy + oy + vy) : ctx.moveTo(cx + ox + vx, cy + oy + vy);
    });
    ctx.closePath();
    ctx.fill();
  });
  // The shine that sweeps across once the logo is assembled.
  const sweep = ramp(t, 2.0, 2.8);
  if (sweep > 0 && sweep < 1) {
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    const x = cx - s + sweep * 2 * s;
    const g = ctx.createLinearGradient(x - 20, 0, x + 20, 0);
    g.addColorStop(0, 'rgba(255,255,255,0)');
    g.addColorStop(0.5, 'rgba(255,255,255,0.7)');
    g.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = g;
    ctx.fillRect(cx - s, cy - s, 2 * s, 2 * s);
    ctx.restore();
  }
}

function sceScreen(ctx, t, feed) {
  const bgL = feed.value('ps1.sce.bg', 100, 60);
  const fade = window_(t, 0, 5.2, 0.5, 0.6);
  ctx.fillStyle = hsl(feed.value('ps1.sce.bghue', 0, 360), feed.glitch('ps1.sce.bghue') * 80, bgL);
  ctx.fillRect(0, 0, W, H);
  diamond(ctx, t, feed, W / 2 + feed.value('ps1.diamond.x', 0, 200), H / 2 - 40 + feed.value('ps1.diamond.y', 0, 150));

  ctx.globalAlpha = ramp(t, 2.0, 2.8);
  ctx.fillStyle = hsl(feed.value('ps1.sce.texthue', 0, 360), feed.glitch('ps1.sce.texthue') * 90, 12);
  ctx.font = `bold ${Math.round(feed.value('ps1.sce.textsize', 26, 30))}px Helvetica, Arial, sans-serif`;
  const words = feed.text('company').replace(/ Inc\.?$/, '');
  glitchText(ctx, words, W / 2, H / 2 + 80, feed.glitch('ps1.sce.text'), feed.bytes('ps1.sce.text'));
  ctx.globalAlpha = 1;

  // Fade from/to black.
  ctx.fillStyle = `rgba(0,0,0,${1 - fade})`;
  ctx.fillRect(0, 0, W, H);
}

function psLogo(ctx, t, feed, cx, cy) {
  // A stylised blocky "P" standing on a flat, isometric "S".
  const rise = ease(ramp(t, 0, 1.2));
  ctx.save();
  ctx.translate(cx, cy);
  ctx.transform(1, feed.value('ps1.logo.skew', 0, 0.8), feed.value('ps1.logo.shear', 0, 0.8), 1, 0, 0);
  const s = feed.value('ps1.logo.scale', 1, 0.8);
  ctx.scale(s, s);

  // S: four coloured strokes on an isometric plane.
  const iso = (x, y) => [(x - y) * 0.9, (x + y) * 0.45];
  const sCols = [[45, 95, 55], [140, 70, 45], [205, 85, 55], [270, 50, 55]];
  const sPath = [[-40, 0, 40, 0], [40, 0, 40, 30], [40, 30, -40, 30], [-40, 30, -40, 60]];
  sPath.forEach(([x1, y1, x2, y2], i) => {
    const n = `ps1.logo.s${i}`;
    const [a, b] = iso(x1 + feed.value(`${n}.a`, 0, 50), y1);
    const [c, d] = iso(x2, y2 + feed.value(`${n}.b`, 0, 50));
    ctx.strokeStyle = hsl(feed.value(`${n}.hue`, sCols[i][0], 160), sCols[i][1], sCols[i][2], rise);
    ctx.lineWidth = feed.value(`${n}.w`, 14, 20);
    ctx.lineCap = 'square';
    ctx.beginPath();
    ctx.moveTo(a + 10, b + 30);
    ctx.lineTo(c + 10, d + 30);
    ctx.stroke();
  });

  // P: a red letter that stands up out of the floor.
  ctx.save();
  ctx.translate(-30, 35);
  ctx.scale(1, rise);
  ctx.fillStyle = hsl(feed.value('ps1.logo.phue', 355, 180), 85, 50);
  ctx.fillRect(-12, -110, 22, 110);
  ctx.beginPath();
  ctx.moveTo(10, -110);
  ctx.lineTo(10 + feed.value('ps1.logo.bowl', 28, 60), -110);
  ctx.quadraticCurveTo(55, -92, 38, -70);
  ctx.lineTo(10, -60);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
  ctx.restore();
}

function licensedScreen(ctx, t, feed) {
  ctx.fillStyle = hsl(0, 0, feed.value('ps1.lic.bg', 0, 40));
  ctx.fillRect(0, 0, W, H);
  const a = window_(t, 0, 5.2, 0.6, 0.8);
  ctx.globalAlpha = a;
  psLogo(ctx, t, feed, W / 2, H / 2 - 50);
  ctx.fillStyle = hsl(0, 0, 100, a);
  ctx.font = `${Math.round(feed.value('ps1.lic.size', 20, 20))}px Helvetica, Arial, sans-serif`;
  const lic = feed.text('licensed');
  glitchText(ctx, lic, W / 2, H / 2 + 90, feed.glitch('ps1.lic.text'), feed.bytes('ps1.lic.text'));
  const company = feed.text('company').replace(/ Inc\.?$/, '');
  glitchText(ctx, `${company} ${regionName(feed)}`, W / 2, H / 2 + 118, feed.glitch('ps1.lic.text2'), feed.bytes('ps1.lic.text2'));
  ctx.globalAlpha = 1;
}

function gameScreen(ctx, t, feed) {
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, W, H);
  // What a crashed boot looks like when the BIOS jumps into garbage.
  const crash = feed.glitch('ps1.game.crash');
  if (crash) garbage(ctx, feed.bytes('ps1.game.crash', 4096), 0, 0, W, H * crash, 8);
  ctx.fillStyle = 'rgba(255,255,255,0.35)';
  ctx.font = '14px ui-monospace, monospace';
  ctx.textAlign = 'center';
  ctx.globalAlpha = ramp(t, 0.5, 1.5);
  ctx.fillText('(the game would start here)', W / 2, H - 30);
  ctx.globalAlpha = 1;
}

function shell(ctx, t, feed) {
  // Memory card / CD player shell for when there's no licensed disc.
  const g = ctx.createLinearGradient(0, 0, 0, H);
  g.addColorStop(0, hsl(feed.value('ps1.shell.top', 225, 160), 60, 18));
  g.addColorStop(1, hsl(feed.value('ps1.shell.bottom', 230, 160), 70, 6));
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, W, H);
  const a = ramp(t, 0, 0.8);
  const buttons = ['MEMORY CARD', 'CD PLAYER'];
  buttons.forEach((label, i) => {
    const n = `ps1.shell.b${i}`;
    const bx = W / 2 - 110 + feed.value(`${n}.x`, 0, 160);
    const by = 170 + i * 90 + feed.value(`${n}.y`, 0, 120);
    const selected = i === Math.floor(t / 2.5) % 2;
    ctx.fillStyle = selected ? hsl(45, 80, 60, a) : hsl(220, 30, 70, a * 0.5);
    ctx.fillRect(bx, by, 220 + feed.value(`${n}.w`, 0, 200), 54);
    ctx.fillStyle = hsl(220, 60, 10, a);
    ctx.font = 'bold 20px Helvetica, Arial, sans-serif';
    glitchText(ctx, label, bx + 110, by + 34, feed.glitch(`${n}.label`), feed.bytes(`${n}.label`));
  });
  ctx.fillStyle = hsl(0, 0, 100, 0.4 * a);
  ctx.font = '12px ui-monospace, monospace';
  glitchText(ctx, feed.text('version'), W / 2, H - 24, feed.glitch('ps1.shell.ver'), feed.bytes('ps1.shell.ver'));
}

export default {
  id: 'ps1',
  duration: (opts) => (opts.licensed ? 13 : 12),
  sound(audio, feed) {
    // The familiar two-part swell, detuned by corruption.
    const p = feed.value('ps1.sound.pitch', 1, 0.6);
    chime(audio, [
      { freq: 220 * p, at: 0.6, len: 2.4, vol: 0.08, type: 'triangle' },
      { freq: 330 * p, at: 0.9, len: 2.2, vol: 0.06 },
      { freq: 440 * p * feed.value('ps1.sound.harm', 1, 0.3), at: 1.9, len: 2.6, vol: 0.07 },
      { freq: 660 * p, at: 2.1, len: 2.6, vol: 0.04 },
    ]);
  },
  draw(ctx, t, feed, opts) {
    if (t < 5.2) sceScreen(ctx, t, feed);
    else if (opts.licensed) {
      if (t < 10.6) licensedScreen(ctx, t - 5.4, feed);
      else gameScreen(ctx, t - 10.6, feed);
    } else shell(ctx, t - 5.2, feed);
    postFx(ctx, t, feed, 'ps1');
  },
};
