// The emulator side of the PS2 drawing worker (ps2-gs.worker.js): the core
// records each frame's GS commands instead of drawing them, and `send` hands
// them over. Frames come back through `onFrame`, in order.

export class GsLink {
  constructor(wasm, wasmUrl, onError) {
    this.wasm = wasm;
    this.inFlight = 0; // frames sent but not drawn yet
    this.pictureDue = false; // a frame sent with render, not back yet
    this.onFrame = null;
    this.waiters = [];
    this.worker = new Worker(new URL('./ps2-gs.worker.js', import.meta.url), { type: 'module' });
    this.worker.onmessage = (ev) => {
      const m = ev.data;
      if (m.type === 'done') {
        this.inFlight--;
        if (m.pixels) {
          this.pictureDue = false;
          this.onFrame?.(m);
        }
        for (const r of this.waiters.splice(0)) r();
      } else if (m.type === 'error') {
        onError(m.message);
      }
    };
    this.worker.onerror = (e) => onError(`The PS2 drawing worker failed: ${e.message || 'unknown error'}`);
    this.worker.postMessage({ type: 'start', wasmUrl });
    wasm.ps2_set_remote_gs(1);
  }

  // Sends the commands the last frame recorded; render asks for its picture.
  send(render) {
    const w = this.wasm;
    const len = w.ps2_gs_cmds_len();
    const regs = w.ps2_display_regs();
    const n = len + 7;
    const buf = new BigUint64Array(n);
    buf.set(new BigUint64Array(w.memory.buffer, regs, 7));
    if (len) buf.set(new BigUint64Array(w.memory.buffer, w.ps2_gs_cmds_ptr(), len), 7);
    w.ps2_gs_cmds_clear();
    this.inFlight++;
    if (render) this.pictureDue = true;
    this.worker.postMessage({ type: 'draw', buf: buf.buffer, n, render }, [buf.buffer]);
  }

  // Resolves once fewer than `max` frames are waiting to be drawn.
  async below(max) {
    while (this.inFlight >= max) await new Promise((r) => this.waiters.push(r));
  }

  stop() {
    this.worker.terminate();
  }
}
