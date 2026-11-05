// The compression of the PS2 BIOS's TEXIMAGE / SNDIMAGE / ICOIMAGE files, as
// the browser program (OSDSYS) unpacks it:
//
//   u32 LE  unpacked size
//   then groups: u32 BE flags, then up to 30 tokens. Flag bits 31..2 say,
//   token by token, literal byte (0) or copy (1). The low 2 bits, n, set the
//   copy format: a big-endian u16 whose low 14-n bits are the distance - 1
//   and whose high 2+n bits are the length - 3.
//
// Unpacking stops when the size is reached.

export function decompress(src) {
  const size = (src[0] | (src[1] << 8) | (src[2] << 16) | (src[3] << 24)) >>> 0;
  const out = new Uint8Array(size);
  let p = 4, o = 0, count = 0, flags = 0, mask = 0, shift = 0;
  while (o < size) {
    if (count === 0) {
      if (p + 4 > src.length) throw new Error('Compressed data ends early.');
      flags = ((src[p] << 24) | (src[p + 1] << 16) | (src[p + 2] << 8) | src[p + 3]) >>> 0;
      p += 4;
      const n = flags & 3;
      mask = 0x3fff >> n;
      shift = 14 - n;
      count = 30;
    }
    if (flags & 0x80000000) {
      const w = (src[p] << 8) | src[p + 1];
      p += 2;
      const dist = (w & mask) + 1;
      const len = (w >> shift) + 3;
      if (dist > o) throw new Error('Bad copy in compressed data.');
      for (let k = 0; k < len && o < size; k++, o++) out[o] = out[o - dist];
    } else {
      out[o++] = src[p++];
    }
    count--;
    flags = (flags << 1) >>> 0;
  }
  return out;
}

const HASH_BITS = 15;
const CHAIN = 48; // match candidates tried per position

/** Packs `data` in the same format. */
export function compress(data) {
  const n = data.length;
  const out = [];
  const push32le = (v) => out.push(v & 255, (v >> 8) & 255, (v >> 16) & 255, (v >>> 24) & 255);
  push32le(n);
  // Hash chains over 3-byte prefixes.
  const head = new Int32Array(1 << HASH_BITS).fill(-1);
  const prev = new Int32Array(n).fill(-1);
  const hash = (i) => (((data[i] << 10) ^ (data[i + 1] << 5) ^ data[i + 2]) * 2654435761) >>> (32 - HASH_BITS);
  let inserted = 0;
  const insertUpTo = (i) => {
    for (; inserted < i && inserted + 2 < n; inserted++) {
      const h = hash(inserted);
      prev[inserted] = head[h];
      head[h] = inserted;
    }
  };
  // Longest match at i within distance maxDist, length up to maxLen.
  const match = (i, maxDist, maxLen) => {
    if (i + 2 >= n) return [0, 0];
    insertUpTo(i);
    let best = 0, bestDist = 0;
    let c = head[hash(i)];
    for (let tries = 0; c >= 0 && tries < CHAIN; tries++, c = prev[c]) {
      const d = i - c;
      if (d <= 0) continue; // inserted by a trial further ahead
      if (d > maxDist) break;
      let l = 0;
      const lim = Math.min(maxLen, n - i);
      while (l < lim && data[c + l] === data[i + l]) l++;
      if (l > best) {
        best = l;
        bestDist = d;
        if (l === lim) break;
      }
    }
    // Runs: the byte just before repeats (distance 1, not in the chain yet).
    if (i > 0 && best < maxLen) {
      let l = 0;
      const lim = Math.min(maxLen, n - i);
      while (l < lim && data[i - 1 + l] === data[i + l]) l++;
      if (l > best) {
        best = l;
        bestDist = 1;
      }
    }
    return best >= 3 ? [best, bestDist] : [0, 0];
  };

  let i = 0;
  while (i < n) {
    // Try each copy format for this group and keep the one that covers the
    // most input per output byte.
    let pick = null;
    for (let fmt = 0; fmt < 4; fmt++) {
      const maxDist = (0x3fff >> fmt) + 1;
      const maxLen = (1 << (2 + fmt)) - 1 + 3;
      let j = i, bytes = 4, flags = 0;
      const toks = [];
      for (let t = 0; t < 30 && j < n; t++) {
        const [len, dist] = match(j, maxDist, maxLen);
        if (len) {
          flags |= 1 << (31 - t);
          toks.push([1, ((len - 3) << (14 - fmt)) | (dist - 1)]);
          j += len;
          bytes += 2;
        } else {
          toks.push([0, data[j]]);
          j += 1;
          bytes += 1;
        }
      }
      const score = (j - i) / bytes;
      if (!pick || score > pick.score) pick = { score, flags: (flags | fmt) >>> 0, toks, j };
    }
    const f = pick.flags;
    out.push((f >>> 24) & 255, (f >> 16) & 255, (f >> 8) & 255, f & 255);
    for (const [kind, v] of pick.toks) {
      if (kind) out.push((v >> 8) & 255, v & 255);
      else out.push(v);
    }
    i = pick.j;
  }
  return Uint8Array.from(out);
}
