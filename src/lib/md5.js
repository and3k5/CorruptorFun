// MD5 of a byte array, as lowercase hex. Web Crypto has no MD5, but it's the
// hash BIOS lists (emulator docs, redump, No-Intro) use to name known dumps.

const S = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16,
  23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];
const K = new Int32Array(64);
for (let i = 0; i < 64; i++) K[i] = Math.floor(Math.abs(Math.sin(i + 1)) * 2 ** 32) | 0;

function blocks(state, bytes, end) {
  const M = new Int32Array(16);
  let [a0, b0, c0, d0] = state;
  for (let off = 0; off < end; off += 64) {
    for (let j = 0; j < 16; j++) {
      const p = off + j * 4;
      M[j] = bytes[p] | (bytes[p + 1] << 8) | (bytes[p + 2] << 16) | (bytes[p + 3] << 24);
    }
    let a = a0, b = b0, c = c0, d = d0;
    for (let i = 0; i < 64; i++) {
      let f, g;
      if (i < 16) (f = (b & c) | (~b & d)), (g = i);
      else if (i < 32) (f = (d & b) | (~d & c)), (g = (5 * i + 1) & 15);
      else if (i < 48) (f = b ^ c ^ d), (g = (3 * i + 5) & 15);
      else (f = c ^ (b | ~d)), (g = (7 * i) & 15);
      const t = d;
      d = c;
      c = b;
      const x = (a + f + K[i] + M[g]) | 0;
      b = (b + ((x << S[i]) | (x >>> (32 - S[i])))) | 0;
      a = t;
    }
    a0 = (a0 + a) | 0;
    b0 = (b0 + b) | 0;
    c0 = (c0 + c) | 0;
    d0 = (d0 + d) | 0;
  }
  state[0] = a0;
  state[1] = b0;
  state[2] = c0;
  state[3] = d0;
}

export function md5Hex(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const state = [0x67452301, 0xefcdab89 | 0, 0x98badcfe | 0, 0x10325476];
  const whole = bytes.length & ~63;
  blocks(state, bytes, whole);
  // Last partial block, the 0x80 marker and the bit length.
  const rest = bytes.length - whole;
  const tail = new Uint8Array(rest < 56 ? 64 : 128);
  tail.set(bytes.subarray(whole));
  tail[rest] = 0x80;
  const bits = bytes.length * 8;
  const at = tail.length - 8;
  for (let i = 0; i < 4; i++) tail[at + i] = (bits >>> (8 * i)) & 0xff;
  const hi = Math.floor(bytes.length / 0x20000000);
  for (let i = 0; i < 4; i++) tail[at + 4 + i] = (hi >>> (8 * i)) & 0xff;
  blocks(state, tail, tail.length);
  let out = '';
  for (const w of state) for (let i = 0; i < 4; i++) out += ((w >>> (8 * i)) & 0xff).toString(16).padStart(2, '0');
  return out;
}
