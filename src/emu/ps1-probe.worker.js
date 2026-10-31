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
    }
  } catch (e) {
    postMessage({ type: 'result', id: msg.id, checks: [], error: e.message });
  }
};
