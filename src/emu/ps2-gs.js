// The emulator side of the PS2 drawing worker (ps2-gs.worker.js): the core
// records each frame's GS commands instead of drawing them, and `send` hands
// them over. Frames come back through `onFrame`, in order. Where the drawing
// worker can't start (no workers inside workers, a load error, no answer),
// `GsLink.open` gives null and the core keeps drawing by itself.

const READY_MS = 5000;

export class GsLink {
  static async open(wasm, wasmUrl, onError) {
    let link;
    try {
      link = new GsLink(wasm, wasmUrl, onError);
    } catch {
      return null;
    }
    if (await link.ready) {
      link.started = true;
      wasm.ps2_set_remote_gs(1);
      return link;
    }
    link.stop();
    return null;
  }

  constructor(wasm, wasmUrl, onError) {
    this.wasm = wasm;
    this.inFlight = 0; // frames sent but not drawn yet
    this.onFrame = null;
    this.waiters = [];
    this.started = false;
    let answer;
    this.ready = new Promise((r) => (answer = r));
    const timer = setTimeout(() => answer(false), READY_MS);
    // Errors before the start mean falling back, after it a failed run.
    const failed = (message) => {
      clearTimeout(timer);
      if (this.started) onError(message);
      else answer(false);
    };
    this.worker = new Worker(new URL('./ps2-gs.worker.js', import.meta.url), { type: 'module' });
    this.worker.onmessage = (ev) => {
      const m = ev.data;
      if (m.type === 'ready') {
        clearTimeout(timer);
        answer(true);
      } else if (m.type === 'done') {
        this.inFlight--;
        if (m.pixels) this.onFrame?.(m);
        for (const r of this.waiters.splice(0)) r();
      } else if (m.type === 'error') {
        failed(m.message);
      }
    };
    this.worker.onerror = (e) => {
      e.preventDefault();
      failed(`The PS2 drawing worker failed: ${e.message || 'unknown error'}`);
    };
    this.worker.postMessage({ type: 'start', wasmUrl });
  }

  // Sends the commands the last frame recorded; render asks for its picture
  // (which the drawing worker skips when it is behind, unless every).
  send(render, every = false) {
    const w = this.wasm;
    const len = w.ps2_gs_cmds_len();
    const regs = w.ps2_display_regs();
    const n = len + 7;
    const buf = new BigUint64Array(n);
    buf.set(new BigUint64Array(w.memory.buffer, regs, 7));
    if (len) buf.set(new BigUint64Array(w.memory.buffer, w.ps2_gs_cmds_ptr(), len), 7);
    w.ps2_gs_cmds_clear();
    this.inFlight++;
    this.worker.postMessage({ type: 'draw', buf: buf.buffer, n, render, every }, [buf.buffer]);
  }

  // Resolves once fewer than `max` frames are waiting to be drawn.
  async below(max) {
    while (this.inFlight >= max) await new Promise((r) => this.waiters.push(r));
  }

  stop() {
    this.worker.terminate();
  }
}
