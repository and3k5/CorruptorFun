// Runs the PS1 WebAssembly core off the main thread. A corrupted BIOS can't
// freeze the page this way, and the page can always kill a boot by
// terminating the worker.
//
// Frames are paced against the wall clock: if the device falls behind, the
// worker runs extra frames without building their pictures to catch up, so
// the boot (and its sound) keeps real speed instead of slowing down.

const MAX_CATCH_UP = 4; // frames per tick before we give up and slow down

let wasm = null;
let mem = () => new Uint8Array(wasm.memory.buffer);
let timer = 0;
let running = false;
let audioPort = null;
let nextAt = 0;

function copyIn(bytes) {
  const ptr = wasm.alloc(bytes.length);
  mem().set(bytes, ptr);
  return ptr;
}

function flushAudio() {
  const n = wasm.ps1_audio_len();
  if (!n) return;
  const samples = new Int16Array(wasm.memory.buffer, wasm.ps1_audio_ptr(), n).slice();
  wasm.ps1_audio_clear();
  if (audioPort) audioPort.postMessage(samples.buffer, [samples.buffer]);
}

function tick() {
  if (!running) return;
  const frameMs = 1000 / (wasm.ps1_is_pal() === 1 ? 50 : 60);
  const now = performance.now();
  let due = Math.floor((now - nextAt) / frameMs) + 1;
  if (due > MAX_CATCH_UP) {
    // Too far behind to catch up: accept the slowdown.
    nextAt = now;
    due = MAX_CATCH_UP;
  }
  try {
    for (let i = 0; i < due; i++) {
      wasm.ps1_set_render(i === due - 1 ? 1 : 0);
      wasm.ps1_run_frame();
      flushAudio();
      nextAt += frameMs;
      if (wasm.ps1_game_started()) break;
    }
  } catch (e) {
    running = false;
    postMessage({ type: 'error', message: `The emulator crashed: ${e.message}` });
    return;
  }
  const w = wasm.ps1_frame_width();
  const h = wasm.ps1_frame_height();
  const pixels = mem().slice(wasm.ps1_frame_ptr(), wasm.ps1_frame_ptr() + w * h * 4);
  postMessage({ type: 'frame', w, h, pixels: pixels.buffer }, [pixels.buffer]);

  if (wasm.ps1_game_started()) {
    running = false;
    postMessage({ type: 'game-started' });
    return;
  }
  timer = setTimeout(tick, Math.max(0, nextAt - performance.now()));
}

onmessage = async (ev) => {
  const msg = ev.data;
  if (msg.type === 'start') {
    try {
      audioPort = msg.audioPort || null;
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
      nextAt = performance.now();
      tick();
    } catch (e) {
      postMessage({ type: 'error', message: `Could not start the emulator: ${e.message}` });
    }
  } else if (msg.type === 'buttons' && wasm) {
    wasm.ps1_set_buttons(msg.mask);
  } else if (msg.type === 'report') {
    let text = 'The emulator has not started.';
    if (wasm) {
      wasm.ps1_report();
      text = new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.ps1_report_ptr(), wasm.ps1_report_len()));
    }
    postMessage({ type: 'report', text });
  } else if (msg.type === 'stop') {
    running = false;
    clearTimeout(timer);
  }
};
