// A tiny homemade PlayStation data disc, built in memory: an ISO 9660 volume
// with SYSTEM.CNF and a minimal PS-X EXE that just loops. It contains no Sony
// data. It's enough for a BIOS to recognise a disc and walk the licensed boot
// path up to starting the program, which is where our emulation stops.

const SECTOR = 2352;
const SECTORS = 300;
const LBA = { pvd: 16, term: 17, pathL: 18, pathM: 19, root: 20, cnf: 21, exe: 22 };
const EXE_ADDR = 0x80010000;

const bcd = (v) => ((v / 10) << 4) | v % 10;

function both32(view, off, v) {
  view.setUint32(off, v, true);
  view.setUint32(off + 4, v, false);
}
function both16(view, off, v) {
  view.setUint16(off, v, true);
  view.setUint16(off + 2, v, false);
}
function ascii(buf, off, text, len = text.length) {
  for (let i = 0; i < len; i++) buf[off + i] = i < text.length ? text.charCodeAt(i) : 0x20;
}

// 1995-01-01 00:00:00, as a 7-byte directory record date.
const DIR_DATE = [95, 1, 1, 0, 0, 0, 0];

function dirRecord(name, lba, size, isDir) {
  const nameLen = name.length;
  const len = 33 + nameLen + ((33 + nameLen) % 2);
  const r = new Uint8Array(len);
  const v = new DataView(r.buffer);
  r[0] = len;
  both32(v, 2, lba);
  both32(v, 10, size);
  r.set(DIR_DATE, 18);
  r[25] = isDir ? 2 : 0;
  both16(v, 28, 1);
  r[32] = nameLen;
  if (typeof name === 'string') ascii(r, 33, name);
  else r.set(name, 33);
  return r;
}

function exeFile() {
  const exe = new Uint8Array(2048 + 2048);
  const v = new DataView(exe.buffer);
  ascii(exe, 0, 'PS-X EXE');
  v.setUint32(0x10, EXE_ADDR, true); // pc
  v.setUint32(0x18, EXE_ADDR, true); // load address
  v.setUint32(0x1c, 2048, true); // size
  v.setUint32(0x30, 0x801ffff0, true); // stack
  ascii(exe, 0x4c, 'CorruptorFun test disc');
  // Program: loop forever (j self; nop).
  v.setUint32(2048, 0x08000000 | ((EXE_ADDR & 0x0fffffff) >>> 2), true);
  return exe;
}

export function buildTestDisc() {
  const img = new Uint8Array(SECTORS * SECTOR);
  const data = (lba) => img.subarray(lba * SECTOR + 24, lba * SECTOR + 24 + 2048);

  // Headers for every sector: sync, MSF address, mode 2, form 1 subheader.
  const fileEnds = new Set([LBA.pvd, LBA.term, LBA.pathL, LBA.pathM, LBA.root, LBA.cnf, LBA.exe + 1]);
  for (let lba = 0; lba < SECTORS; lba++) {
    const o = lba * SECTOR;
    img.fill(0xff, o + 1, o + 11);
    const a = lba + 150;
    img[o + 12] = bcd(Math.floor(a / 4500));
    img[o + 13] = bcd(Math.floor(a / 75) % 60);
    img[o + 14] = bcd(a % 75);
    img[o + 15] = 2;
    const submode = fileEnds.has(lba) ? 0x89 : 0x08;
    img[o + 18] = submode;
    img[o + 22] = submode;
  }

  const cnf = new TextEncoder().encode('BOOT = cdrom:\\PSX.EXE;1\r\nTCB = 4\r\nEVENT = 10\r\nSTACK = 801FFFF0\r\n');
  const exe = exeFile();
  data(LBA.cnf).set(cnf);
  data(LBA.exe).set(exe.subarray(0, 2048));
  data(LBA.exe + 1).set(exe.subarray(2048));

  // Root directory.
  const root = data(LBA.root);
  let p = 0;
  for (const rec of [
    dirRecord(new Uint8Array([0]), LBA.root, 2048, true),
    dirRecord(new Uint8Array([1]), LBA.root, 2048, true),
    dirRecord('PSX.EXE;1', LBA.exe, exe.length, false),
    dirRecord('SYSTEM.CNF;1', LBA.cnf, cnf.length, false),
  ]) {
    root.set(rec, p);
    p += rec.length;
  }

  // Path tables (one entry: the root), little- and big-endian.
  for (const [lba, le] of [
    [LBA.pathL, true],
    [LBA.pathM, false],
  ]) {
    const t = data(lba);
    const v = new DataView(t.buffer, t.byteOffset, 2048);
    t[0] = 1;
    v.setUint32(2, LBA.root, le);
    v.setUint16(6, 1, le);
  }

  // Primary volume descriptor and terminator.
  const pvd = data(LBA.pvd);
  const pv = new DataView(pvd.buffer, pvd.byteOffset, 2048);
  pvd[0] = 1;
  ascii(pvd, 1, 'CD001');
  pvd[6] = 1;
  ascii(pvd, 8, 'PLAYSTATION', 32);
  ascii(pvd, 40, 'CORRUPTORFUN', 32);
  both32(pv, 80, SECTORS);
  both16(pv, 120, 1);
  both16(pv, 124, 1);
  both16(pv, 128, 2048);
  both32(pv, 132, 10); // path table size
  pv.setUint32(140, LBA.pathL, true);
  pv.setUint32(148, LBA.pathM, false);
  pvd.set(dirRecord(new Uint8Array([0]), LBA.root, 2048, true), 156);
  ascii(pvd, 190, 'CORRUPTORFUN', 128);
  ascii(pvd, 813, '1995010100000000', 16);
  pvd[881] = 1;
  const term = data(LBA.term);
  term[0] = 255;
  ascii(term, 1, 'CD001');
  term[6] = 1;

  return img;
}
