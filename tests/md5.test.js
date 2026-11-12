import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { md5Hex } from '../src/lib/md5.js';

const enc = (s) => new TextEncoder().encode(s);

test('md5 matches the RFC 1321 test vectors', () => {
  assert.equal(md5Hex(enc('')), 'd41d8cd98f00b204e9800998ecf8427e');
  assert.equal(md5Hex(enc('abc')), '900150983cd24fb0d6963f7d28e17f72');
  assert.equal(md5Hex(enc('message digest')), 'f96b697d7cb7938d525a2f31aaf161d0');
  assert.equal(
    md5Hex(enc('12345678901234567890123456789012345678901234567890123456789012345678901234567890')),
    '57edf4a22be3c955ac49da2e2107b67a',
  );
});

test('md5 matches node for every tail length and a BIOS-sized buffer', () => {
  const ref = (b) => createHash('md5').update(b).digest('hex');
  for (let n = 0; n < 200; n++) {
    const b = new Uint8Array(n).map((_, i) => (i * 31 + n) & 0xff);
    assert.equal(md5Hex(b), ref(b), `length ${n}`);
  }
  const big = new Uint8Array(4 * 1024 * 1024).map((_, i) => (i * 2654435761) >>> 24);
  assert.equal(md5Hex(big), ref(big));
  assert.equal(md5Hex(big.buffer), ref(big));
});
