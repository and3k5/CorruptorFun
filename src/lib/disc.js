// Turns the files a user picks for a PS1 disc into one raw data track
// (2352-byte sectors), which is what the emulator's CD-ROM reads.

const RAW = 2352;

const bcd = (n) => ((Math.floor(n / 10) << 4) | (n % 10));

// Wraps 2048-byte ISO sectors in Mode 2 Form 1 raw sectors.
function isoToRaw(iso) {
  const n = iso.length / 2048;
  const out = new Uint8Array(n * RAW);
  for (let i = 0; i < n; i++) {
    const o = i * RAW;
    out.fill(0xff, o + 1, o + 11);
    const lba = i + 150;
    out[o + 12] = bcd(Math.floor(lba / 4500));
    out[o + 13] = bcd(Math.floor(lba / 75) % 60);
    out[o + 14] = bcd(lba % 75);
    out[o + 15] = 2;
    out.set(iso.subarray(i * 2048, i * 2048 + 2048), o + 24);
  }
  return out;
}

const base = (name) => name.split(/[\\/]/).pop().toLowerCase();

// files: FileList or File[] -> { data: Uint8Array, label: string }
export async function loadDisc(files) {
  const list = [...files];
  const cue = list.find((f) => /\.cue$/i.test(f.name));
  let track = null;
  let end = null;
  if (cue) {
    const text = await cue.text();
    let file = null;
    let first = null;
    for (const line of text.split(/\r?\n/)) {
      const m = line.trim().match(/^FILE\s+"?(.+?)"?\s+\w+$/i);
      if (m) file = m[1];
      const t = line.trim().match(/^TRACK\s+(\d+)\s+(\S+)/i);
      if (t && !first && /MODE/i.test(t[2])) first = { file, num: +t[1] };
      else if (t && first && file === first.file && end == null) {
        end = 'next-track-same-file';
      }
      const idx = line.trim().match(/^INDEX\s+01\s+(\d+):(\d+):(\d+)/i);
      if (idx && first && end === 'next-track-same-file') {
        end = ((+idx[1] * 60 + +idx[2]) * 75 + +idx[3]) * RAW;
      }
    }
    if (!first) throw new Error('The .cue file has no data track.');
    track = list.find((f) => base(f.name) === base(first.file));
    if (!track) throw new Error(`Also pick "${first.file}" (the .cue points to it).`);
  } else {
    track = list.find((f) => /\.(bin|img|iso)$/i.test(f.name)) || list[0];
  }
  let data = new Uint8Array(await track.arrayBuffer());
  if (typeof end === 'number') data = data.subarray(0, end);
  if (data.length % RAW !== 0 && data.length % 2048 === 0) data = isoToRaw(data);
  if (data.length < RAW * 16) throw new Error('That disc image is too small to be a PlayStation disc.');
  return { data, label: track.name };
}
