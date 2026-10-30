// Byte corruption engines. Everything is deterministic for a given seed so a
// corruption can be shared or replayed by its settings alone.

export function mulberry32(seed) {
  let a = seed >>> 0;
  return function rand() {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const ENGINES = {
  bitflip: { label: 'Bit flip', help: 'Flip one random bit in each hit byte.' },
  random: { label: 'Random byte', help: 'Replace each hit byte with a random value.' },
  increment: { label: 'Nudge', help: 'Add or subtract the amount to each hit byte.' },
  xor: { label: 'XOR mask', help: 'XOR each hit byte with the amount.' },
  smear: { label: 'Smear', help: 'Copy a run of bytes forward over its neighbours.' },
  shift: { label: 'Shift block', help: 'Copy a block from somewhere else in the range.' },
  zero: { label: 'Zero out', help: 'Set each hit byte to 0x00.' },
};

export const DEFAULT_SETTINGS = {
  engine: 'bitflip',
  seed: 1337,
  start: 0,
  end: null, // null = end of file
  mode: 'count', // 'count' | 'every'
  count: 64,
  every: 4096,
  amount: 1, // nudge amount / xor mask / block length for smear and shift
};

export function clampRange(settings, size) {
  const start = Math.max(0, Math.min(size - 1, Math.floor(settings.start || 0)));
  const rawEnd = settings.end == null || settings.end === '' ? size : Math.floor(settings.end);
  const end = Math.max(start + 1, Math.min(size, rawEnd));
  return { start, end };
}

// Returns { data, hits } where hits is a sorted list of byte offsets that changed.
export function corrupt(source, settings) {
  const s = { ...DEFAULT_SETTINGS, ...settings };
  const data = new Uint8Array(source);
  const size = data.length;
  if (!size) return { data, hits: [] };
  const { start, end } = clampRange(s, size);
  const span = end - start;
  const rand = mulberry32(s.seed);
  const amount = Math.max(1, Math.floor(s.amount || 1));

  const targets = [];
  if (s.mode === 'every') {
    const step = Math.max(1, Math.floor(s.every || 1));
    // Random phase so different seeds hit different bytes in "every" mode too.
    for (let o = start + Math.floor(rand() * step); o < end; o += step) targets.push(o);
  } else {
    const n = Math.max(0, Math.min(1_000_000, Math.floor(s.count || 0)));
    for (let i = 0; i < n; i++) targets.push(start + Math.floor(rand() * span));
  }

  for (const o of targets) {
    switch (s.engine) {
      case 'bitflip':
        data[o] ^= 1 << Math.floor(rand() * 8);
        break;
      case 'random':
        data[o] = Math.floor(rand() * 256);
        break;
      case 'increment':
        data[o] = (data[o] + (rand() < 0.5 ? amount : -amount)) & 0xff;
        break;
      case 'xor':
        data[o] ^= amount & 0xff;
        break;
      case 'zero':
        data[o] = 0;
        break;
      case 'smear': {
        const v = data[o];
        const len = Math.min(amount, end - o);
        for (let k = 1; k < len; k++) data[o + k] = v;
        break;
      }
      case 'shift': {
        const len = Math.min(amount, end - o);
        const from = start + Math.floor(rand() * Math.max(1, span - len));
        data.copyWithin(o, from, from + len);
        break;
      }
      default:
        throw new Error(`Unknown engine: ${s.engine}`);
    }
  }

  const hits = [];
  for (let i = start; i < end; i++) if (data[i] !== source[i]) hits.push(i);
  return { data, hits };
}
