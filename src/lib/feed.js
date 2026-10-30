// The bridge between a BIOS file and the simulated boot scenes.
//
// Every visual parameter of a scene ("orb.radius", "logo.hue", ...) is bound
// to its own slice of the BIOS. The scene reads the corrupted copy of that
// slice and compares it to the original: identical bytes give the stock
// animation, and every changed byte pushes the parameter away from stock.
// So *where* you corrupt decides *what* breaks, and the same file + settings
// always break the same way.

function hashString(str) {
  let h = 2166136261;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

// Each parameter watches roughly 1/96th of the file, so ~100 hits spread
// across the whole BIOS touch about every parameter once.
const sliceSize = (size) => Math.max(512, Math.floor(size / 96));

export class BiosFeed {
  constructor(original, corrupted, strings = {}) {
    this.original = original;
    this.corrupted = corrupted;
    this.size = corrupted.length;
    this.strings = strings;
    this.cache = new Map();
  }

  // The byte range a parameter is bound to.
  slice(name) {
    const span = Math.min(sliceSize(this.size), this.size);
    const start = this.size > span ? hashString(name) % (this.size - span) : 0;
    return [start, start + span];
  }

  stats(name) {
    let st = this.cache.get(name);
    if (st) return st;
    const [a, b] = this.slice(name);
    let changed = 0;
    let signed = 0;
    let xor = 0;
    for (let i = a; i < b; i++) {
      const o = this.original[i];
      const c = this.corrupted[i];
      if (o !== c) {
        changed++;
        signed += ((c - o) << 24) >> 24; // wrap to -128..127
        xor = (xor * 31 + (o ^ c) + i) >>> 0;
      }
    }
    st = { changed, signed, xor, start: a, end: b };
    this.cache.set(name, st);
    return st;
  }

  // 0 when the slice is untouched, approaching 1 the more of it is corrupted.
  glitch(name) {
    const { changed } = this.stats(name);
    return changed === 0 ? 0 : 1 - Math.exp(-changed / 3);
  }

  // A signed offset in [-1, 1] derived from how the bytes changed.
  delta(name) {
    const { changed, signed } = this.stats(name);
    if (!changed) return 0;
    return Math.max(-1, Math.min(1, signed / (changed * 64)));
  }

  // stock value, nudged by up to `range` when the slice is corrupted.
  value(name, stock, range) {
    const g = this.glitch(name);
    if (!g) return stock;
    const d = this.delta(name) || (this.rand(name) * 2 - 1);
    return stock + d * range * (0.35 + g);
  }

  // A stable pseudo random number in [0, 1) for a corrupted parameter.
  rand(name, salt = 0) {
    const { xor } = this.stats(name);
    let x = (xor ^ hashString(name + salt)) >>> 0;
    x ^= x << 13; x >>>= 0;
    x ^= x >>> 17;
    x ^= x << 5; x >>>= 0;
    return x / 4294967296;
  }

  // Raw corrupted bytes for a parameter, used as "garbage texture" data.
  bytes(name, length = 256) {
    const [a] = this.slice(name);
    return this.corrupted.subarray(a, Math.min(this.size, a + length));
  }

  // Text that lives in the BIOS, read back from the corrupted copy so damaged
  // bytes show up as damaged characters.
  text(key, override) {
    const s = this.strings[key];
    if (!s || s.offset == null) {
      if (override) return override;
      return s ? s.fallback : key;
    }
    let out = '';
    for (let i = 0; i < s.length; i++) {
      const c = this.corrupted[s.offset + i];
      out += c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : String.fromCharCode(0x2580 + (c % 32));
    }
    return out;
  }

  // Overall fraction of all bytes changed, for a global "how wrecked" knob.
  get total() {
    if (this._total != null) return this._total;
    let n = 0;
    // Sample for big files to keep this cheap.
    const step = Math.max(1, Math.floor(this.size / 2_000_000));
    for (let i = 0; i < this.size; i += step) if (this.original[i] !== this.corrupted[i]) n++;
    this._total = (n * step) / this.size;
    return this._total;
  }
}
