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

// What earlier probes taught us, per 4 KB block of the BIOS: how often a
// corruption window over it gave a good result. Every BIOS revision lays its
// data out differently, so this steers later candidates towards the parts
// that matter for this one.
const BLOCK = 0x1000;

export function learn(history, size) {
  const n = Math.ceil(size / BLOCK);
  const stats = { good: new Float64Array(n), tried: new Float64Array(n) };
  for (const c of history) addToStats(stats, c.settings, c.metrics);
  return stats;
}

export function addToStats(stats, settings, metrics) {
  const b0 = Math.floor(settings.start / BLOCK);
  const b1 = Math.max(b0 + 1, Math.ceil(settings.end / BLOCK));
  const share = 1 / (b1 - b0);
  for (let b = b0; b < b1 && b < stats.tried.length; b++) {
    stats.tried[b] += share;
    if (metrics.ok) stats.good[b] += share;
  }
}

// Chance estimate that a corruption over a block gives a good result.
const blockWeight = (stats, b) => (stats ? (stats.good[b] + 0.5) / (stats.tried[b] + 1) : 0.5);

// Settings for one random candidate, drawn from `rand`, steered by `stats`
// from learn() when given.
export function candidateSettings(consoleId, size, rand, stats = null) {
  const areas = AREAS[consoleId] || {};
  const bounds = (area) => {
    const [a0, a1] = areas[area] || [0, size];
    return [Math.min(a0, size - 1), Math.min(a1, size)];
  };
  const areaWeight = (area) => {
    const [s, e] = bounds(area);
    let sum = 0, n = 0;
    for (let b = Math.floor(s / BLOCK); b < Math.ceil(e / BLOCK); b++, n++) sum += blockWeight(stats, b);
    return n ? sum / n : 0.5;
  };
  const weights = RECIPES.map((r) => r.w * areaWeight(r.area));
  let pick = rand() * weights.reduce((a, w) => a + w, 0);
  const recipe = RECIPES[weights.findIndex((w) => (pick -= w) < 0)] || RECIPES[0];
  const between = ([lo, hi]) => lo + Math.floor(rand() * (hi - lo + 1));
  const [areaStart, areaEnd] = bounds(recipe.area);

  // Centre the window on a block, favouring blocks that worked before.
  const first = Math.floor(areaStart / BLOCK);
  const blocks = [];
  for (let b = first; b < Math.ceil(areaEnd / BLOCK); b++) blocks.push(blockWeight(stats, b) ** 2);
  let bp = rand() * blocks.reduce((a, w) => a + w, 0);
  const centreBlock = first + Math.max(0, blocks.findIndex((w) => (bp -= w) < 0));
  const centre = centreBlock * BLOCK + Math.floor(rand() * BLOCK);

  // Window of 5% to all of the area: concentrated hits tend to give coherent
  // glitches rather than noise.
  const span = areaEnd - areaStart;
  const len = Math.max(16, Math.floor(span * Math.exp(Math.log(0.05) * rand())));
  const start = Math.max(areaStart, Math.min(areaEnd - len, (Math.floor((centre - len / 2) / 16) * 16)));
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

// Whether a scored probe is good enough under the user's filters.
// `refM` is the clean boot scored against itself.
export function accepts(m, refM, { picture = false, sound = false } = {}) {
  if (!m.ok) return false;
  // "No black screen": a picture for most of the time the clean boot has one.
  if (picture && m.visual < Math.max(0.15, 0.8 * refM.visual)) return false;
  // "Must have sound": audible for at least half as long as the clean boot.
  if (sound && m.sound < Math.max(0.1, 0.5 * refM.sound)) return false;
  return true;
}

// The best unused cached candidate that passes the filters, or null.
export function pickCached(cache, filters) {
  const ok = cache.candidates.filter((c) => !c.used && accepts(c.metrics, cache.refMetrics, filters));
  ok.sort((a, b) => b.metrics.score - a.metrics.score);
  return ok[0] || null;
}

// Probes new candidates on a small pool of workers until `enough` of them
// pass `accept` (or `maxTries` is reached), adding every probe to `cache`
// ({ ref, refMetrics, candidates }, filled in on first use). Returns how many
// were tried.
export async function easySearch({ bios, consoleId, region, wasmUrl, makeWorker, cache, accept, onProgress, signal, maxTries = 16, enough = 3, frames = 540 }) {
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

    if (!cache.ref) {
      onProgress?.({ phase: 'reference' });
      const checks = await run(workers[0], bios, frames);
      if (signal?.aborted) return 0;
      if (!checks.length) throw new Error('the emulator could not run here');
      cache.ref = checks;
      cache.refMetrics = score(checks, checks, checks.length);
    }
    const refChecks = cache.ref;
    const expected = refChecks.length;
    const giveUpAt = giveUp(refChecks);

    const rand = mulberry32((Math.random() * 2 ** 32) >>> 0);
    const stats = learn(cache.candidates, bios.length);
    let tried = 0;
    let started = 0;
    let found = 0;
    const done = () => signal?.aborted || started >= maxTries || found >= enough;
    await Promise.all(
      workers.map(async (w) => {
        while (!done()) {
          started++;
          const settings = candidateSettings(consoleId, bios.length, rand, stats);
          const checks = await run(w, corrupt(bios, settings).data, giveUpAt);
          if (signal?.aborted) return; // a cut-short probe says nothing
          tried++;
          const metrics = score(checks, refChecks, expected);
          cache.candidates.push({ settings, metrics, used: false });
          addToStats(stats, settings, metrics);
          if (accept(metrics)) found++;
          onProgress?.({ phase: 'search', tried, maxTries, found });
        }
      }),
    );
    return tried;
  } finally {
    signal?.removeEventListener('abort', cancel);
    workers.forEach((w) => w.terminate());
  }
}

// Probe results are kept per BIOS and core build, so later clicks (and
// visits) can reuse them. Only settings and scores are stored; the corrupted
// bytes are rebuilt from the settings.
const CACHE_PREFIX = 'corruptorfun.easy.';

export function loadCache(key) {
  try {
    const raw = JSON.parse(localStorage.getItem(CACHE_PREFIX + key));
    if (raw?.ref) {
      raw.ref = raw.ref.map((c) => ({ loud: c.loud, grid: Uint8Array.from(atob(c.grid), (ch) => ch.charCodeAt(0)) }));
      return raw;
    }
  } catch {
    // fall through to an empty cache
  }
  return { ref: null, refMetrics: null, candidates: [] };
}

export function saveCache(key, cache) {
  try {
    // One BIOS's cache at a time is plenty; drop the others.
    for (let i = localStorage.length - 1; i >= 0; i--) {
      const k = localStorage.key(i);
      if (k?.startsWith(CACHE_PREFIX) && k !== CACHE_PREFIX + key) localStorage.removeItem(k);
    }
    const ref = cache.ref?.map((c) => ({ loud: c.loud, grid: btoa(String.fromCharCode(...c.grid)) }));
    localStorage.setItem(CACHE_PREFIX + key, JSON.stringify({ ...cache, ref, candidates: cache.candidates.slice(-200) }));
  } catch {
    // The cache is a convenience only.
  }
}
