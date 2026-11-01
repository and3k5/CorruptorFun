import test from 'node:test';
import assert from 'node:assert/strict';
import { buildTestDisc } from '../src/lib/testdisc.js';

const SECTOR = 2352;
const user = (img, lba) => img.subarray(lba * SECTOR + 24, lba * SECTOR + 24 + 2048);
const text = (b, o, n) => String.fromCharCode(...b.subarray(o, o + n));

test('test disc is an ISO 9660 volume with SYSTEM.CNF and a PS-X EXE', () => {
  const img = buildTestDisc();
  assert.equal(img.length % SECTOR, 0);
  // Sector 16's header says 00:02:16, mode 2.
  assert.deepEqual([...img.subarray(16 * SECTOR + 12, 16 * SECTOR + 16)], [0x00, 0x02, 0x16, 0x02]);
  const pvd = user(img, 16);
  assert.equal(text(pvd, 1, 5), 'CD001');
  const dv = new DataView(pvd.buffer, pvd.byteOffset);
  const rootLba = dv.getUint32(156 + 2, true);

  const files = {};
  const root = user(img, rootLba);
  for (let p = 0; root[p]; p += root[p]) {
    const name = text(root, p + 33, root[p + 32]);
    const rv = new DataView(root.buffer, root.byteOffset + p);
    files[name] = { lba: rv.getUint32(2, true), size: rv.getUint32(10, true) };
  }
  const cnf = files['SYSTEM.CNF;1'];
  assert.ok(cnf, Object.keys(files).join());
  assert.match(text(user(img, cnf.lba), 0, cnf.size), /^BOOT = cdrom:\\PSX\.EXE;1/);
  const exe = files['PSX.EXE;1'];
  assert.equal(text(user(img, exe.lba), 0, 8), 'PS-X EXE');
});

test('test disc carries a license line and a logo model for the licensed screen', () => {
  const us = buildTestDisc('A');
  assert.match(text(user(us, 4), 0, 80), /^ {10}Licensed {2}by {10}Sony Computer Entertainment Amer {2}ica/);
  assert.match(text(user(buildTestDisc('E'), 4), 0, 80), /Euro pe/);
  const logo = new DataView(us.buffer, 5 * SECTOR + 24);
  assert.equal(logo.getUint32(0, true), 0x41); // TMD id
  assert.equal(logo.getUint32(8, true), 1); // one object
  // Primitives follow the object table directly, all lit flat triangles.
  assert.equal(logo.getUint32(12 + 16, true), 28);
  assert.equal(us[5 * SECTOR + 24 + 40 + 3], 0x20);
});
