// Plays an emulator core's stereo output on the audio thread (PS1: 44.1 kHz,
// PS2: 48 kHz, given as processorOptions.rate). The emulator worker sends
// Int16 chunks straight here over a MessagePort, so a busy main thread can't
// cause gaps.

const PREBUFFER = 0.06; // seconds queued before (re)starting playback
const MAX_QUEUE = 0.25; // seconds; older audio is dropped beyond this

class Ps1Audio extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.rate = options?.processorOptions?.rate || 44100;
    this.chunks = [];
    this.queued = 0; // stereo frames held in chunks
    this.pos = 0; // fractional read position in chunks[0], in frames
    this.playing = false;
    this.step = this.rate / sampleRate;
    this.port.onmessage = (ev) => {
      if (ev.data.port) ev.data.port.onmessage = (e) => this.push(e.data);
      else if (ev.data === 'reset') this.reset();
    };
  }

  reset() {
    this.chunks = [];
    this.queued = 0;
    this.pos = 0;
    this.playing = false;
  }

  push(buf) {
    const s = new Int16Array(buf);
    if (!s.length) return;
    this.chunks.push(s);
    this.queued += s.length / 2;
    while (this.queued > MAX_QUEUE * this.rate && this.chunks.length > 1) {
      const old = this.chunks.shift();
      this.queued -= old.length / 2;
      this.pos = 0;
    }
  }

  // Sample of channel `ch` at frame `i` past the start of chunks[0], or
  // null when that frame hasn't arrived yet.
  at(i, ch) {
    for (const c of this.chunks) {
      const n = c.length / 2;
      if (i < n) return c[i * 2 + ch];
      i -= n;
    }
    return null;
  }

  process(_inputs, outputs) {
    const [l, r] = outputs[0];
    if (!this.playing && this.queued - this.pos >= PREBUFFER * this.rate) this.playing = true;
    for (let n = 0; n < l.length; n++) {
      l[n] = 0;
      if (r) r[n] = 0;
      if (!this.playing) continue;
      const i = Math.floor(this.pos);
      if (this.at(i + 1, 0) === null) {
        // Ran dry: go quiet and wait for a fresh prebuffer.
        this.playing = false;
        continue;
      }
      const t = this.pos - i;
      const l0 = this.at(i, 0);
      const r0 = this.at(i, 1);
      l[n] = (l0 + (this.at(i + 1, 0) - l0) * t) / 32768;
      if (r) r[n] = (r0 + (this.at(i + 1, 1) - r0) * t) / 32768;
      this.pos += this.step;
      // Drop chunks we've fully played.
      while (this.chunks.length && this.pos >= this.chunks[0].length / 2) {
        const len = this.chunks.shift().length / 2;
        this.pos -= len;
        this.queued -= len;
      }
    }
    return true;
  }
}

registerProcessor('ps1-audio', Ps1Audio);
