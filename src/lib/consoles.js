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
    realEmulation: true,
    licensed: true,
    regions: [
      { label: 'Whole BIOS', start: 0, end: null },
      { label: 'Kernel', start: 0x00000, end: 0x10000 },
      { label: 'Boot intro & shell', start: 0x18000, end: 0x64000 },
      { label: 'Font area', start: 0x66000, end: 0x80000 },
    ],
    strings: [
      { key: 'company', needles: ['Sony Computer Entertainment Inc'], fallback: 'Sony Computer Entertainment' },
      { key: 'version', needles: ['System ROM Version'], fallback: 'System ROM Version' },
      { key: 'licensed', needles: ['Licensed by'], fallback: 'Licensed by' },
    ],
  },
  ps2: {
    id: 'ps2',
    name: 'PlayStation 2',
    short: 'PS2',
    sizes: [4 * MB, 8 * MB],
    fileHint: 'e.g. SCPH-39001.bin / SCPH-70012.bin (4 MB)',
    realEmulation: false,
    licensed: true,
    regions: [
      { label: 'Whole BIOS', start: 0, end: null },
      { label: 'Reset & kernel', start: 0x00000, end: 0x40000 },
      { label: 'First 1 MB (OSD modules)', start: 0x40000, end: 0x100000 },
      { label: 'Last 3 MB', start: 0x100000, end: null },
    ],
    strings: [
      { key: 'company', needles: ['Sony Computer Entertainment'], fallback: 'Sony Computer Entertainment' },
      { key: 'osd', needles: ['OSDSYS'], fallback: 'OSDSYS' },
      { key: 'browser', needles: ['Browser'], fallback: 'Browser' },
      { key: 'config', needles: ['System Configuration'], fallback: 'System Configuration' },
    ],
  },
  xbox: {
    id: 'xbox',
    name: 'Xbox',
    short: 'Xbox',
    sizes: [256 * KB, 512 * KB, 1 * MB],
    fileHint: 'a 256 KB or 1 MB Xbox BIOS image',
    realEmulation: false,
    licensed: false,
    regions: [
      { label: 'Whole BIOS', start: 0, end: null },
      { label: 'First quarter', start: 0, end: 64 * KB },
      { label: 'Middle', start: 64 * KB, end: 192 * KB },
      { label: 'Last 64 KB', start: 192 * KB, end: 256 * KB },
    ],
    strings: [],
  },
  x360: {
    id: 'x360',
    name: 'Xbox 360',
    short: '360',
    sizes: [16 * MB, 64 * MB, 256 * MB, 512 * MB],
    fileHint: 'a NAND dump (16 MB, 64 MB, ...). Any file works as a data source',
    realEmulation: false,
    licensed: false,
    regions: [
      { label: 'Whole file', start: 0, end: null },
      { label: 'First 1 MB (bootloaders)', start: 0, end: 1 * MB },
      { label: 'Rest of the flash', start: 1 * MB, end: null },
    ],
    strings: [],
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
  if (size === 256 * KB || size === 1 * MB) return 'xbox';
  if (size >= 16 * MB) return 'x360';
  if (size === 512 * KB) return 'ps1';
  return null;
}

// Locate the strings a scene wants to draw so we can re-read them from the
// corrupted copy later.
export function locateStrings(consoleId, original) {
  const out = {};
  for (const s of CONSOLES[consoleId].strings) {
    let found = null;
    for (const needle of s.needles) {
      const at = includesAscii(original, needle);
      if (at >= 0) {
        // Grow forward to the end of the printable run so we pick up the full text.
        let end = at;
        while (end < original.length && end - at < 64 && original[end] >= 0x20 && original[end] < 0x7f) end++;
        found = { offset: at, length: end - at };
        break;
      }
    }
    out[s.key] = { ...found, fallback: s.fallback };
  }
  return out;
}
