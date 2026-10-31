// "Easy corrupt": try a batch of corruptions headlessly and keep one that
// still shows or plays something, and differs from the clean boot.

import { DEFAULT_SETTINGS, corrupt, mulberry32 } from './corrupt.js';

// Where and how to corrupt, by weight. Code hits mostly crash the BIOS, so
// most recipes go for the intro and shell area (graphics, models, sound data)
// and the font, with only the odd light touch to the kernel.
const RECIPES = [
  { w: 3, area: 'shell', engine: 'bitflip', count: [1, 30] },
  { w: 2, area: 'shell', engine: 'random', count: [1, 12] },
  { w: 2, area: 'shell', engine: 'increment', count: [4, 60], amount: [1, 4] },
  { w: 2, area: 'shell', engine: 'smear', count: [1, 8], amount: [2, 24] },
  { w: 1, area: 'shell', engine: 'shift', count: [1, 4], amount: [8, 128] },
  { w: 1, area: 'shell', engine: 'xor', count: [1, 16], amount: [1, 255] },
  { w: 1, area: 'font', engine: 'bitflip', count: [100, 2000] },
  { w: 1, area: 'font', engine: 'random', count: [50, 500] },
  { w: 0.5, area: 'kernel', engine: 'bitflip', count: [1, 3] },
];

const AREAS = {
  ps1: { kernel: [0x00000, 0x10000], shell: [0x18000, 0x64000], font: [0x66000, 0x80000] },
};

// Settings for one random candidate, drawn from `rand`.
export function candidateSettings(consoleId, size, rand) {
  const total = RECIPES.reduce((a, r) => a + r.w, 0);
  let pick = rand() * total;
  const recipe = RECIPES.find((r) => (pick -= r.w) < 0) || RECIPES[0];
  const between = ([lo, hi]) => lo + Math.floor(rand() * (hi - lo + 1));
  const [a0, a1] = (AREAS[consoleId] || {})[recipe.area] || [0, size];
  const areaStart = Math.min(a0, size - 1);
  const areaEnd = Math.min(a1, size);
  // Focus on a window of the area (5% to all of it): concentrated hits tend
  // to give coherent glitches rather than noise.
  const span = areaEnd - areaStart;
  const len = Math.max(16, Math.floor(span * Math.exp(Math.log(0.05) * rand())));
  const start = areaStart + (Math.floor((rand() * (span - len + 1)) / 16) * 16);
  return {
    ...DEFAULT_SETTINGS,
    engine: recipe.engine,
    seed: Math.floor(rand() * 1e9),
    start,
    end: Math.min(areaEnd, start + len),
    mode: 'count',
    count: between(recipe.count),
    amount: recipe.amount ? between(recipe.amount) : 1,
  };
}

const lit = (grid) => {
  let n = 0;
  for (let i = 0; i < grid.length; i += 3) if (Math.max(grid[i], grid[i + 1], grid[i + 2]) > 8) n++;
  return n / (grid.length / 3);
};
const gridDiff = (a, b) => {
  let d = 0;
  for (let i = 0; i < a.length; i++) d += Math.abs(a[i] - b[i]);
  return d / a.length / 255;
};
const LOUD = 0.002;

// Rates a probe against the clean boot. `expected` is how many checks a full
// run has, so a run that died early counts its missing checks as dead air.
export function score(checks, ref, expected) {
  const n = Math.max(expected, 1);
  let vis = 0, snd = 0, mov = 0, dif = 0, sdif = 0;
  checks.forEach((c, i) => {
    const r = ref[i];
    if (lit(c.grid) > 0.02) vis++;
    if (c.loud > LOUD) snd++;
    if (i > 0 && gridDiff(c.grid, checks[i - 1].grid) > 0.01) mov++;
    dif += r ? gridDiff(c.grid, r.grid) : lit(c.grid);
    if (!r || (c.loud > LOUD) !== (r.loud > LOUD)) sdif++;
  });
  const m = { visual: vis / n, sound: snd / n, motion: mov / n, different: dif / n, soundDifferent: sdif / n };
  m.alive = m.visual >= 0.15 || m.sound >= 0.1;
  m.peculiar = m.different >= 0.02 || m.soundDifferent >= 0.15;
  m.ok = m.alive && m.peculiar;
  m.score =
    m.visual + 0.8 * m.sound + 0.6 * m.motion + 3 * Math.min(m.different, 0.3) + 0.5 * m.soundDifferent +
    (m.visual >= 0.15 && m.sound >= 0.1 ? 0.5 : 0);
  return m;
}

// Runs probes on a small pool of workers. Resolves to { best, tried, passed,
// ref }, where best is { settings, data, hits, metrics } or null, and ref is
// the clean boot's probe, to pass back in next time.
export async function easyCorrupt({ bios, consoleId, region, wasmUrl, makeWorker, ref, onProgress, signal, maxTries = 16, enough = 3, frames = 300 }) {
  const giveUp = (refChecks) => {
    const first = refChecks.findIndex((c) => lit(c.grid) > 0.02 || c.loud > LOUD);
    return Math.max(150, (first < 0 ? 15 : first + 1) * 10 + 90);
  };
  const poolSize = Math.max(1, Math.min(4, (navigator.hardwareConcurrency || 2) - 1));
  const workers = [];
  let nextId = 0;
  const waiting = new Map();
  const run = (w, bytes, giveUpAt) =>
    new Promise((resolve) => {
      const id = nextId++;
      waiting.set(id, resolve);
      const copy = bytes.slice().buffer;
      w.postMessage({ type: 'probe', id, bios: copy, region, frames, giveUpAt }, [copy]);
    });
  const cancel = () => {
    for (const resolve of waiting.values()) resolve([]);
    waiting.clear();
  };
  signal?.addEventListener('abort', cancel);
  try {
    for (let i = 0; i < poolSize; i++) {
      const w = makeWorker();
      w.onmessage = (ev) => {
        if (ev.data.type === 'result') {
          waiting.get(ev.data.id)?.(ev.data.checks);
          waiting.delete(ev.data.id);
        }
      };
      w.postMessage({ type: 'init', wasmUrl });
      workers.push(w);
    }

    const rand = mulberry32((Math.random() * 2 ** 32) >>> 0);
    let refChecks = ref?.checks;
    if (!refChecks) {
      onProgress?.({ phase: 'reference' });
      refChecks = await run(workers[0], bios, frames);
      if (!refChecks.length) {
        if (signal?.aborted) return { best: null, tried: 0, passed: 0, ref: null };
        throw new Error('the emulator could not run here');
      }
    }
    const expected = refChecks.length || frames / 10;
    const giveUpAt = giveUp(refChecks);

    const results = [];
    let tried = 0;
    let started = 0;
    const passed = () => results.filter((r) => r.metrics.ok).length;
    const done = () => signal?.aborted || started >= maxTries || passed() >= enough;
    await Promise.all(
      workers.map(async (w) => {
        while (!done()) {
          started++;
          const settings = candidateSettings(consoleId, bios.length, rand);
          const { data, hits } = corrupt(bios, settings);
          const checks = await run(w, data, giveUpAt);
          tried++;
          results.push({ settings, data, hits, metrics: score(checks, refChecks, expected) });
          onProgress?.({ phase: 'search', tried, maxTries, passed: passed() });
        }
      }),
    );
    results.sort((a, b) => b.metrics.ok - a.metrics.ok || b.metrics.score - a.metrics.score);
    return { best: results[0] || null, tried, passed: passed(), ref: { checks: refChecks } };
  } finally {
    signal?.removeEventListener('abort', cancel);
    workers.forEach((w) => w.terminate());
  }
}
