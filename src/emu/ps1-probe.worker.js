// Boots a BIOS headlessly as fast as the device allows and reports what it
// showed and played, so "Easy corrupt" can tell a fun corruption from a
// black screen. Every probe gets a fresh instance of the core.

const CHECK_EVERY = 10; // frames between samples
const GRID_W = 16;
const GRID_H = 12;

let module = null; // Promise of the compiled core

// Mean colour of each grid cell of the current frame, as bytes.
function sample(wasm) {
  wasm.ps1_render();
  const w = wasm.ps1_frame_width();
  const h = wasm.ps1_frame_height();
  const px = new Uint8Array(wasm.memory.buffer, wasm.ps1_frame_ptr(), w * h * 4);
  const out = new Uint8Array(GRID_W * GRID_H * 3);
  for (let gy = 0; gy < GRID_H; gy++) {
    for (let gx = 0; gx < GRID_W; gx++) {
      let r = 0, g = 0, b = 0, n = 0;
      const y0 = Math.floor((gy * h) / GRID_H), y1 = Math.floor(((gy + 1) * h) / GRID_H);
      const x0 = Math.floor((gx * w) / GRID_W), x1 = Math.floor(((gx + 1) * w) / GRID_W);
      // Every other pixel is plenty for a cell average.
      for (let y = y0; y < y1; y += 2) {
        for (let x = x0; x < x1; x += 2) {
          const o = (y * w + x) * 4;
          r += px[o];
          g += px[o + 1];
          b += px[o + 2];
          n++;
        }
      }
      const c = (gy * GRID_W + gx) * 3;
      out[c] = r / n;
      out[c + 1] = g / n;
      out[c + 2] = b / n;
    }
  }
  return out;
}

function loudness(wasm) {
  const n = wasm.ps1_audio_len();
  if (!n) return 0;
  const s = new Int16Array(wasm.memory.buffer, wasm.ps1_audio_ptr(), n);
  let sum = 0;
  for (let i = 0; i < n; i++) sum += Math.abs(s[i]);
  wasm.ps1_audio_clear();
  return sum / n / 32768;
}

async function probe({ bios, region, frames, giveUpAt }) {
  const { exports: wasm } = await WebAssembly.instantiate(await module, {});
  const bytes = new Uint8Array(bios);
  const ptr = wasm.alloc(bytes.length);
  new Uint8Array(wasm.memory.buffer).set(bytes, ptr);
  wasm.ps1_init(ptr, bytes.length);
  wasm.ps1_set_region(region.charCodeAt(0));
  wasm.ps1_set_render(0);
  const checks = [];
  let loud = 0;
  let seen = false;
  for (let f = 1; f <= frames; f++) {
    wasm.ps1_run_frame();
    loud = Math.max(loud, loudness(wasm));
    if (f % CHECK_EVERY === 0) {
      const grid = sample(wasm);
      checks.push({ grid, loud });
      if (loud > 0.002 || grid.some((v) => v > 8)) seen = true;
      loud = 0;
      // Nothing on screen or speaker this long: call it dead and move on.
      if (!seen && f >= giveUpAt) break;
    }
    if (wasm.ps1_game_started()) break;
  }
  return checks;
}

// Boots until the game starts (or `frames`) and returns the notes the SPU
// played, the sound RAM and when the boot sound ends (SPU clock), to see
// how the BIOS plays each sample.
async function notes({ bios, disc, region, frames }) {
  const { exports: wasm } = await WebAssembly.instantiate(await module, {});
  const copyIn = (bytes) => {
    const ptr = wasm.alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer).set(bytes, ptr);
    return ptr;
  };
  const b = new Uint8Array(bios);
  wasm.ps1_init(copyIn(b), b.length);
  if (disc) {
    const d = new Uint8Array(disc);
    wasm.ps1_set_disc(copyIn(d), d.length);
  }
  wasm.ps1_set_region(region.charCodeAt(0));
  wasm.ps1_set_render(0);
  // Notes may ring on after the hand-off to the game; a game would soon
  // stop them, so a second after it is where the boot sound ends.
  let until = null;
  for (let f = 0, end = frames; f < end; f++) {
    wasm.ps1_run_frame();
    wasm.ps1_audio_clear();
    if (until === null && wasm.ps1_game_started()) {
      until = wasm.ps1_spu_clock() + 44100;
      end = Math.min(frames, f + 62);
    }
  }
  const log = new Uint32Array(wasm.memory.buffer, wasm.ps1_notes_ptr(), wasm.ps1_notes_len() * 8).slice();
  const ram = new Uint8Array(wasm.memory.buffer, wasm.ps1_spu_ram_ptr(), 512 * 1024).slice();
  return { log, ram, until };
}

onmessage = async (ev) => {
  const msg = ev.data;
  try {
    if (msg.type === 'init') {
      // Probes may arrive before compiling finishes; they wait on this.
      module = WebAssembly.compileStreaming(fetch(msg.wasmUrl));
      module.catch(() => {}); // reported by the probes that need it
    } else if (msg.type === 'probe') {
      const checks = await probe(msg);
      postMessage({ type: 'result', id: msg.id, checks });
    } else if (msg.type === 'notes') {
      const r = await notes(msg);
      postMessage({ type: 'notes', id: msg.id, ...r }, [r.log.buffer, r.ram.buffer]);
    }
  } catch (e) {
    postMessage({ type: msg.type === 'notes' ? 'notes' : 'result', id: msg.id, checks: [], error: e.message });
  }
};
