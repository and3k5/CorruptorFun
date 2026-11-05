// The PS2 BIOS's file system. ROM0 starts with a file table (ROMDIR): one
// 16-byte entry per file (10-byte name, 2-byte EXTINFO size, 4-byte size),
// beginning with the RESET entry, and the files follow one another in that
// order, each padded to 16 bytes. Entries named "-" are padding that keeps
// later files at fixed addresses. TEXIMAGE, SNDIMAGE and ICOIMAGE are
// archives in the same format, and their files are compressed (see lz).
//
// Growing a file moves every later file up and takes the room from the
// next padding entry, so the files that must stay put (behind it) do.

const ALIGN = (n) => (n + 15) & ~15;

function readName(rom, at) {
  let s = '';
  for (let i = 0; i < 10 && rom[at + i]; i++) s += String.fromCharCode(rom[at + i]);
  return s;
}

function u32(rom, at) {
  return (rom[at] | (rom[at + 1] << 8) | (rom[at + 2] << 16) | (rom[at + 3] << 24)) >>> 0;
}

function findReset(rom, limit) {
  outer: for (let i = 0; i + 16 <= Math.min(limit, rom.length); i += 16) {
    for (let k = 0; k < 5; k++) if (rom[i + k] !== 'RESET'.charCodeAt(k)) continue outer;
    if (rom[i + 5] === 0) return i;
  }
  return -1;
}

/**
 * The file table of a ROM image or archive: [{ name, offset, size, entry }]
 * (entry: where its table entry sits). Null if there is none.
 */
export function readDir(rom, limit = 0x10000) {
  const start = findReset(rom, limit);
  if (start < 0) return null;
  const files = [];
  let off = 0;
  for (let e = start; e + 16 <= rom.length; e += 16) {
    const name = readName(rom, e);
    const size = u32(rom, e + 12);
    if (!name && !size) break;
    files.push({ name, offset: off, size, entry: e });
    off += ALIGN(size);
    if (files.length > 1024) return null;
  }
  return files;
}

export function findFile(rom, name) {
  return readDir(rom)?.find((f) => f.name === name) || null;
}

export function fileData(rom, name) {
  const f = findFile(rom, name);
  return f ? rom.subarray(f.offset, f.offset + f.size) : null;
}

/**
 * A copy of `rom` with file `name` replaced by `data`. Later files move to
 * make room, which comes out of the next padding entry. Throws if it can't
 * fit, with `.short` set to the bytes missing.
 */
export function replaceFile(rom, name, data) {
  const files = readDir(rom);
  const i = files?.findIndex((f) => f.name === name) ?? -1;
  if (i < 0) throw new Error(`No ${name} in this BIOS.`);
  const f = files[i];
  const delta = ALIGN(data.length) - ALIGN(f.size);
  const out = new Uint8Array(rom.length);
  const dv = new DataView(out.buffer);
  if (delta === 0) {
    out.set(rom);
    out.fill(0, f.offset, f.offset + ALIGN(f.size));
    out.set(data, f.offset);
    dv.setUint32(f.entry + 12, data.length, true);
    return out;
  }
  // The padding that absorbs the change: the first one after this file,
  // which must have room when growing.
  const p = files.findIndex((g, k) => k > i && g.name === '-');
  if (p < 0) throw Object.assign(new Error(`No room after ${name} in this BIOS.`), { short: delta });
  const pad = files[p];
  if (pad.size - delta < 0) throw Object.assign(new Error(`${name} is ${delta - pad.size} bytes too big to fit.`), { short: delta - pad.size });
  out.set(rom);
  // Files between this one and the padding move by delta.
  const moveFrom = f.offset + ALIGN(f.size);
  const moveTo = pad.offset; // end of the moved range
  out.set(rom.subarray(moveFrom, moveTo), moveFrom + delta);
  out.fill(0, f.offset, f.offset + ALIGN(data.length));
  out.set(data, f.offset);
  // The padding shrinks (or grows) from its start, so its end stays put.
  const padStart = pad.offset + delta;
  out.fill(0xFF, padStart, pad.offset + ALIGN(pad.size));
  if (delta < 0) {
    // Gap left between the moved files and the old padding start.
    out.fill(0xFF, padStart, pad.offset);
  }
  dv.setUint32(f.entry + 12, data.length, true);
  dv.setUint32(pad.entry + 12, pad.size - delta, true);
  return out;
}

/**
 * Builds an archive (TEXIMAGE-style) from the old one with some files
 * swapped: `changes` maps file name to new bytes. The table itself is the
 * archive's ROMDIR file.
 */
export function rebuildArchive(arc, changes) {
  const files = readDir(arc, arc.length);
  if (!files) throw new Error('Not an archive.');
  let total = 0;
  const parts = files.map((f) => {
    const d = changes[f.name] ?? arc.subarray(f.offset, f.offset + f.size);
    total += ALIGN(d.length);
    return d;
  });
  const out = new Uint8Array(total);
  const dv = new DataView(out.buffer);
  let off = 0;
  files.forEach((f, k) => {
    out.set(parts[k], off);
    off += ALIGN(parts[k].length);
  });
  // Sizes in the table (which is one of the files, so rewrite in place).
  const dir = files.find((f) => f.name === 'ROMDIR');
  const dirAt = files.slice(0, files.indexOf(dir)).reduce((s, g, k) => s + ALIGN(parts[k].length), 0);
  files.forEach((f, k) => dv.setUint32(dirAt + (f.entry - dir.offset) + 12, parts[k].length, true));
  // The last file needn't be padded.
  const last = parts[parts.length - 1].length;
  return out.subarray(0, total - ALIGN(last) + last);
}

/** Every file in an archive, by name. */
export function archiveFiles(arc) {
  const files = readDir(arc, arc.length) || [];
  return Object.fromEntries(files.map((f) => [f.name, arc.subarray(f.offset, f.offset + f.size)]));
}
