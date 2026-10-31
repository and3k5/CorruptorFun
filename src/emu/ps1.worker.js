// Runs the PS1 WebAssembly core off the main thread. A corrupted BIOS can't
// freeze the page this way, and the page can always kill a boot by
// terminating the worker.

let wasm = null;
let mem = () => new Uint8Array(wasm.memory.buffer);
let timer = 0;
let running = false;

function copyIn(bytes) {
  const ptr = wasm.alloc(bytes.length);
  mem().set(bytes, ptr);
  return ptr;
}

function loop() {
  if (!running) return;
  const pal = wasm.ps1_is_pal() === 1;
  const frameMs = 1000 / (pal ? 50 : 60);
  const t0 = performance.now();
  try {
    wasm.ps1_run_frame();
  } catch (e) {
    running = false;
    postMessage({ type: 'error', message: `The emulator crashed: ${e.message}` });
    return;
  }
  const w = wasm.ps1_frame_width();
  const h = wasm.ps1_frame_height();
  const pixels = mem().slice(wasm.ps1_frame_ptr(), wasm.ps1_frame_ptr() + w * h * 4);
  postMessage({ type: 'frame', w, h, pixels: pixels.buffer }, [pixels.buffer]);

  const n = wasm.ps1_audio_len();
  if (n) {
    const ptr = wasm.ps1_audio_ptr();
    const samples = new Int16Array(wasm.memory.buffer, ptr, n).slice();
    wasm.ps1_audio_clear();
    postMessage({ type: 'audio', samples: samples.buffer }, [samples.buffer]);
  }
  if (wasm.ps1_game_started()) {
    running = false;
    postMessage({ type: 'game-started' });
    return;
  }
  const spent = performance.now() - t0;
  timer = setTimeout(loop, Math.max(0, frameMs - spent));
}

onmessage = async (ev) => {
  const msg = ev.data;
  if (msg.type === 'start') {
    try {
      const { instance } = await WebAssembly.instantiateStreaming(fetch(msg.wasmUrl), {});
      wasm = instance.exports;
      const bios = new Uint8Array(msg.bios);
      wasm.ps1_init(copyIn(bios), bios.length);
      if (msg.disc) {
        const disc = new Uint8Array(msg.disc);
        wasm.ps1_set_disc(copyIn(disc), disc.length);
      }
      wasm.ps1_set_region(msg.region.charCodeAt(0));
      running = true;
      loop();
    } catch (e) {
      postMessage({ type: 'error', message: `Could not start the emulator: ${e.message}` });
    }
  } else if (msg.type === 'buttons' && wasm) {
    wasm.ps1_set_buttons(msg.mask);
  } else if (msg.type === 'stop') {
    running = false;
    clearTimeout(timer);
  }
};
