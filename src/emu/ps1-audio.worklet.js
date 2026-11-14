// Plays an emulator core's stereo output on the audio thread (PS1: 44.1 kHz,
// PS2: 48 kHz, given as processorOptions.rate). The emulator worker sends
// Int16 chunks straight here over a MessagePort, so a busy main thread can't
// cause gaps.
//
// When the emulator can't keep real time (the PS2 intro on most devices),
// sound arrives slower than it plays. Rather than running dry and
// stuttering, playback follows the speed the sound arrives at (measured
// over about a second), nudged to keep a little sound queued. The tempo
// changes but the pitch doesn't: the sound is time-stretched (WSOLA:
// overlapping windowed grains, each placed where it best continues the
// last one). At full speed the grains line up exactly and the sound passes
// through unchanged.

const PREBUFFER = 0.1; // seconds queued before (re)starting playback
const MAX_QUEUE = 0.35; // seconds; older audio is dropped beyond this
const TARGET = 0.1; // seconds queued that playback steers towards
const AVERAGE = 1; // seconds the arrival speed is averaged over
const MIN_SPEED = 0.3;
const MAX_SPEED = 1.05;

const GRAIN = 2048; // samples per grain (at the output rate)
const HOP = GRAIN / 2; // output samples per grain
const SEEK = 256; // how far (in samples) a grain may move to fit
const RING = 1 << 17; // input held, in samples (a power of two)
const MASK = RING - 1;

// Hann window: at half overlap its copies add up to exactly 1.
const WINDOW = new Float32Array(GRAIN);
for (let i = 0; i < GRAIN; i++) WINDOW[i] = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / GRAIN);

class Ps1Audio extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.rate = options?.processorOptions?.rate || 44100;
    this.ratio = this.rate / sampleRate; // input samples per output sample
    this.inL = new Float32Array(RING);
    this.inR = new Float32Array(RING);
    this.out = [new Float32Array(HOP), new Float32Array(HOP)]; // the hop being played
    this.tail = [new Float32Array(HOP), new Float32Array(HOP)]; // last grain's second half
    this.reset();
    this.port.onmessage = (ev) => {
      if (ev.data.port) ev.data.port.onmessage = (e) => this.push(e.data);
      else if (ev.data === 'reset') this.reset();
    };
  }

  reset() {
    this.written = 0; // samples written to the ring, ever (output rate)
    this.read = 0; // where the next grain naturally starts
    this.outPos = HOP; // position in `out`; HOP means a new grain is due
    this.prev = undefined; // where the last grain started
    this.carry = [0, 0]; // last input sample, for resampling across chunks
    this.frac = 0; // resampling position past the carried sample
    this.tail[0].fill(0);
    this.tail[1].fill(0);
    this.playing = false;
    this.speed = 1;
    this.arrival = 1; // how fast sound arrives, in real time (averaged)
    this.got = 0; // input frames received since the last block
  }

  // Resamples a chunk to the output rate into the ring.
  push(buf) {
    const s = new Int16Array(buf);
    const n = s.length / 2;
    if (!n) return;
    this.got += n;
    let p = this.frac;
    let [l0, r0] = this.carry;
    // Position p is past the carried sample (index -1 of this chunk).
    while (p < n) {
      const i = Math.floor(p) - 1; // -1 is the carried sample
      const t = p - Math.floor(p);
      const la = i < 0 ? l0 : s[i * 2] / 32768;
      const ra = i < 0 ? r0 : s[i * 2 + 1] / 32768;
      const lb = s[(i + 1) * 2] / 32768;
      const rb = s[(i + 1) * 2 + 1] / 32768;
      const w = this.written & MASK;
      this.inL[w] = la + (lb - la) * t;
      this.inR[w] = ra + (rb - ra) * t;
      this.written++;
      p += this.ratio;
    }
    this.frac = p - n;
    this.carry = [s[(n - 1) * 2] / 32768, s[(n - 1) * 2 + 1] / 32768];
    // Too much queued (a burst after a stall): skip ahead.
    const max = MAX_QUEUE * sampleRate;
    if (this.written - this.read > max) {
      this.read = this.written - Math.round(TARGET * sampleRate);
      this.prev = undefined;
    }
  }

  // Similarity of the input at a and b over the overlap (mono, sparse).
  match(a, b) {
    const L = this.inL;
    const R = this.inR;
    let s = 0;
    for (let i = 0; i < HOP; i += 4) {
      const x = L[(a + i) & MASK] + R[(a + i) & MASK];
      const y = L[(b + i) & MASK] + R[(b + i) & MASK];
      s += x * y;
    }
    return s;
  }

  // Places the next grain: returns false when not enough input is in.
  grain() {
    const natural = this.read; // where the last grain's sound continues
    // Where the grain belongs on the stretched timeline; the search below
    // only shifts it, so the shifts don't add up.
    this.ana = this.prev === undefined ? natural : this.ana + HOP * this.speed;
    const nominal = Math.round(this.ana);
    if (nominal + SEEK + GRAIN > this.written) return false;
    let start = nominal;
    if (this.prev !== undefined && nominal !== natural) {
      let best = -Infinity;
      for (let d = -SEEK; d <= SEEK; d += 2) {
        const c = nominal + d;
        if (c < this.written - RING + GRAIN) continue;
        const v = this.match(c, natural);
        if (v > best) {
          best = v;
          start = c;
        }
      }
    }
    for (let ch = 0; ch < 2; ch++) {
      const src = ch ? this.inR : this.inL;
      const out = this.out[ch];
      const tail = this.tail[ch];
      for (let i = 0; i < HOP; i++) {
        out[i] = tail[i] + src[(start + i) & MASK] * WINDOW[i];
        tail[i] = src[(start + HOP + i) & MASK] * WINDOW[HOP + i];
      }
    }
    this.prev = start;
    this.read = start + HOP;
    this.outPos = 0;
    return true;
  }

  process(_inputs, outputs) {
    const [l, r] = outputs[0];
    // Seconds queued beyond what the next grain needs.
    const level = (this.written - this.read - HOP - SEEK) / sampleRate;
    const block = l.length / sampleRate; // seconds this call plays
    // Arrival speed: input received per input played, averaged.
    this.arrival += (this.got / this.rate / block - this.arrival) * Math.min(1, block / AVERAGE);
    this.got = 0;
    if (!this.playing && level >= PREBUFFER) {
      this.playing = true;
      this.prev = undefined; // start afresh where the sound is
      this.tail[0].fill(0);
      this.tail[1].fill(0);
      this.outPos = HOP;
    }
    // Play at the arrival speed, a little faster or slower (2% at most) to
    // keep the queue near the target, and clearly slower when it is about
    // to run dry (while the average catches up).
    const off = Math.max(-1, Math.min(1, (level - TARGET) / TARGET));
    const nearDry = Math.max(0, 0.5 - level / TARGET); // 0 until half the target
    const want = Math.max(MIN_SPEED, Math.min(MAX_SPEED, this.arrival * (1 + off * 0.02 - nearDry * 0.6)));
    this.speed += (want - this.speed) * 0.05;
    // Close to real speed with the queue near the target, play exactly at
    // it: the grains then line up and the sound is untouched.
    if (Math.abs(this.speed - 1) < 0.004 && Math.abs(off) < 0.5) this.speed = 1;
    for (let n = 0; n < l.length; n++) {
      if (this.playing && this.outPos >= HOP && !this.grain()) {
        // Ran dry: go quiet and wait for a fresh prebuffer.
        this.playing = false;
      }
      if (!this.playing) {
        l[n] = 0;
        if (r) r[n] = 0;
        continue;
      }
      l[n] = this.out[0][this.outPos];
      if (r) r[n] = this.out[1][this.outPos];
      this.outPos++;
    }
    return true;
  }
}

registerProcessor('ps1-audio', Ps1Audio);
