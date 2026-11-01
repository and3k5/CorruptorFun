// What we know about each console's boot ROM. Offsets in `regions` are rough
// landmarks to make corrupting interesting parts easier; they are not exact
// for every revision.

const KB = 1024;
const MB = 1024 * KB;

export const CONSOLES = {
  ps1: {
    id: 'ps1',
    name: 'PlayStation',
    short: 'PS1',
    sizes: [512 * KB],
    fileHint: 'e.g. SCPH1001.BIN / SCPH5501.BIN / SCPH7502.BIN (512 KB)',
    regions: [
      { label: 'Whole BIOS', start: 0, end: null },
      { label: 'Kernel', start: 0x00000, end: 0x10000 },
      { label: 'Boot intro & shell', start: 0x18000, end: 0x64000 },
      { label: 'Font area', start: 0x66000, end: 0x80000 },
    ],
  },
  ps2: {
    id: 'ps2',
    name: 'PlayStation 2',
    short: 'PS2',
    sizes: [4 * MB, 8 * MB],
    fileHint: 'e.g. SCPH-39001.bin / SCPH-70012.bin (4 MB)',
    regions: [
      { label: 'Whole BIOS', start: 0, end: null },
      { label: 'Reset & kernel', start: 0x00000, end: 0x40000 },
      { label: 'First 1 MB (OSD modules)', start: 0x40000, end: 0x100000 },
      { label: 'Last 3 MB', start: 0x100000, end: null },
    ],
  },
};

export const CONSOLE_LIST = Object.values(CONSOLES);

function includesAscii(bytes, text, limit = bytes.length) {
  const first = text.charCodeAt(0);
  outer: for (let i = 0, n = Math.min(limit, bytes.length) - text.length; i <= n; i++) {
    if (bytes[i] !== first) continue;
    for (let k = 1; k < text.length; k++) if (bytes[i + k] !== text.charCodeAt(k)) continue outer;
    return i;
  }
  return -1;
}

export { includesAscii as findAscii };

// Best guess at which console a file is for. Returns a console id or null.
export function detectConsole(bytes) {
  const size = bytes.length;
  if (size === 512 * KB && includesAscii(bytes, 'Sony Computer Entertainment') >= 0) return 'ps1';
  if ((size === 4 * MB || size === 8 * MB) && includesAscii(bytes, 'RESET', 0x10000) >= 0) return 'ps2';
  if (size === 4 * MB) return 'ps2';
  if (size === 512 * KB) return 'ps1';
  return null;
}

// Region letter of a PS1 BIOS from its version string, e.g.
// "System ROM Version 4.1 12/16/97 E". The CD-ROM answers GetID with the
// matching SCEx code so a licensed disc boots. Returns 'A', 'E' or 'I'.
export function ps1Region(bios) {
  // Some BIOSes hold more than one version string (e.g. "System ROM Version
  // 1.0" without a region, then "... 2.2 12/04/95 A"), so check each one.
  let from = 0;
  for (;;) {
    const at = includesAscii(bios.subarray(from), 'System ROM Version');
    if (at < 0) break;
    let end = from + at;
    while (end < bios.length && bios[end] >= 0x20 && bios[end] < 0x7f) end++;
    const last = String.fromCharCode(bios[end - 1]);
    if (last === 'A') return 'A';
    if (last === 'J') return 'I';
    if (last === 'E') return 'E';
    from = end;
  }
  return 'E';
}
