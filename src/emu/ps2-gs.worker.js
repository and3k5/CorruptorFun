// Draws a PS2 boot's pictures: a second instance of the PS2 core whose GS
// replays the drawing commands the emulator worker records, so drawing one
// frame runs at the same time as emulating the next.
//
// Messages in: { type: 'start', wasmUrl } (answered with { type: 'ready' }), then { type: 'draw', buf, n, render, every }
// with buf holding the 7 display registers and the frame's commands (n u64s).
// Each draw is answered with { type: 'done' }, carrying the picture
// ({ w, h, pixels }) when render was set, unless newer draws are already
// waiting (then at least every other picture is built); every asks for
// all of them.

let wasm = null;
let chain = Promise.resolve();
let queued = 0; // draws received but not done
let skipped = 0; // pictures skipped since the last one built

async function start(url) {
  const { instance } = await WebAssembly.instantiateStreaming(fetch(url), {});
  wasm = instance.exports;
  postMessage({ type: 'ready' });
}

function draw(m) {
  const ptr = wasm.ps2_gs_input(m.n);
  new BigUint64Array(wasm.memory.buffer, ptr, m.n).set(new BigUint64Array(m.buf, 0, m.n));
  wasm.ps2_gs_run(m.n);
  queued--;
  // When behind, build only the newest picture (or every other one).
  if (!m.render || (!m.every && queued > 0 && skipped < 1)) {
    if (m.render) skipped++;
    postMessage({ type: 'done' });
    return;
  }
  skipped = 0;
  wasm.ps2_gs_render();
  const w = wasm.ps2_gs_frame_width();
  const h = wasm.ps2_gs_frame_height();
  const p = wasm.ps2_gs_frame_ptr();
  const pixels = new Uint8Array(wasm.memory.buffer, p, w * h * 4).slice();
  postMessage({ type: 'done', w, h, pixels: pixels.buffer }, [pixels.buffer]);
}

// Messages are handled strictly in order, the first one (start) included.
onmessage = (ev) => {
  const m = ev.data;
  if (m.type === 'draw') queued++;
  chain = chain
    .then(() => (m.type === 'start' ? start(m.wasmUrl) : m.type === 'draw' ? draw(m) : null))
    .catch((e) => {
      postMessage({ type: 'error', message: `The PS2 drawing crashed: ${e.message}` });
      chain = new Promise(() => {}); // stop here
    });
};
