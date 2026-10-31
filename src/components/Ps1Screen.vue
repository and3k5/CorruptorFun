<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue';
import wasmUrl from '../wasm/ps1.wasm?url';

// Runs the corrupted BIOS in our own PS1 core (Rust → WebAssembly) inside a
// worker, draws its frames and plays its audio.
const props = defineProps({
  bios: { type: Object, required: true }, // corrupted Uint8Array
  disc: { type: Object, default: null }, // raw data track Uint8Array
  region: { type: String, default: 'E' },
});

const canvas = ref(null);
const status = ref('Press Boot to run the BIOS.');
const running = ref(false);
const muted = ref(false);
let worker = null;
let audio = null;
let gain = null;
let nextAudio = 0;
let lastFrame = 0;
let watchdog = 0;

// Keyboard → PS1 pad bits (active high).
const KEYS = {
  Enter: 1 << 3, ShiftRight: 1 << 0, Backspace: 1 << 0,
  ArrowUp: 1 << 4, ArrowRight: 1 << 5, ArrowDown: 1 << 6, ArrowLeft: 1 << 7,
  KeyQ: 1 << 10, KeyW: 1 << 11, KeyE: 1 << 8, KeyR: 1 << 9,
  KeyS: 1 << 12, KeyX: 1 << 13, KeyZ: 1 << 14, KeyA: 1 << 15,
};
let buttons = 0;

function onKey(ev) {
  const bit = KEYS[ev.code];
  if (!bit || !running.value) return;
  ev.preventDefault();
  buttons = ev.type === 'keydown' ? buttons | bit : buttons & ~bit;
  worker?.postMessage({ type: 'buttons', mask: buttons });
}

function draw(w, h, pixels) {
  const c = canvas.value;
  if (!c) return;
  if (c.width !== w || c.height !== h) {
    c.width = w;
    c.height = h;
  }
  c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(pixels), w, h), 0, 0);
}

function play(buf) {
  if (!audio || muted.value) return;
  const s = new Int16Array(buf);
  const frames = s.length / 2;
  if (!frames) return;
  const b = audio.createBuffer(2, frames, 44100);
  const l = b.getChannelData(0);
  const r = b.getChannelData(1);
  for (let i = 0; i < frames; i++) {
    l[i] = s[2 * i] / 32768;
    r[i] = s[2 * i + 1] / 32768;
  }
  const now = audio.currentTime;
  // Keep latency bounded: resync if we've fallen behind or run far ahead.
  if (nextAudio < now + 0.02 || nextAudio > now + 0.3) nextAudio = now + 0.05;
  const src = audio.createBufferSource();
  src.buffer = b;
  src.connect(gain);
  src.start(nextAudio);
  nextAudio += b.duration;
}

function stop(text = 'Stopped.') {
  worker?.terminate();
  worker = null;
  running.value = false;
  clearInterval(watchdog);
  status.value = text;
}

function boot() {
  stop();
  try {
    audio = audio || new AudioContext({ sampleRate: 44100 });
    gain = gain || audio.createGain();
    gain.connect(audio.destination);
    audio.resume();
  } catch {
    audio = null;
  }
  worker = new Worker(new URL('../emu/ps1.worker.js', import.meta.url), { type: 'module' });
  worker.onmessage = (ev) => {
    const m = ev.data;
    if (m.type === 'frame') {
      lastFrame = performance.now();
      draw(m.w, m.h, m.pixels);
    } else if (m.type === 'audio') {
      play(m.samples);
    } else if (m.type === 'game-started') {
      stop('The BIOS handed over to the game here. Only the boot is emulated, so it stops.');
    } else if (m.type === 'error') {
      stop(m.message);
    }
  };
  // Copies, so the stored original stays untouched and we can transfer.
  const bios = props.bios.slice().buffer;
  const disc = props.disc ? props.disc.slice().buffer : null;
  worker.postMessage({ type: 'start', wasmUrl: new URL(wasmUrl, location.href).href, bios, disc, region: props.region }, disc ? [bios, disc] : [bios]);
  running.value = true;
  status.value = props.disc ? 'Booting with your disc…' : 'Booting with no disc…';
  lastFrame = performance.now();
  watchdog = setInterval(() => {
    if (running.value && performance.now() - lastFrame > 4000) {
      stop('The emulator stopped responding. Try another corruption.');
    }
  }, 1000);
}

function toggleMute() {
  muted.value = !muted.value;
  if (gain) gain.gain.value = muted.value ? 0 : 1;
}

function snapshot() {
  const a = document.createElement('a');
  a.href = canvas.value.toDataURL('image/png');
  a.download = 'ps1-boot.png';
  a.click();
}

onMounted(() => {
  window.addEventListener('keydown', onKey);
  window.addEventListener('keyup', onKey);
});
onBeforeUnmount(() => {
  stop();
  window.removeEventListener('keydown', onKey);
  window.removeEventListener('keyup', onKey);
  audio?.close();
});

defineExpose({ boot });
</script>

<template>
  <div class="emu">
    <div class="screen">
      <canvas ref="canvas" width="320" height="240" aria-label="PlayStation output" />
      <div v-if="!running" class="overlay note">{{ status }}</div>
    </div>
    <div class="row">
      <button class="primary" @click="boot">{{ running ? 'Reboot' : 'Boot' }}</button>
      <button v-if="running" @click="stop()">Stop</button>
      <button @click="toggleMute">{{ muted ? 'Unmute' : 'Mute' }}</button>
      <button @click="snapshot">Save PNG</button>
      <span v-if="running" class="note">{{ status }}</span>
    </div>
    <p class="note keys">
      Pad: arrows, Enter = Start, Backspace = Select, Z = ✕, X = ○, A = □, S = △, Q/W = L1/R1, E/R = L2/R2.
    </p>
  </div>
</template>

<style scoped>
.emu { display: grid; gap: 8px; }
.screen { position: relative; aspect-ratio: 4 / 3; background: #000; border-radius: 8px; overflow: hidden; }
canvas { width: 100%; height: 100%; display: block; image-rendering: pixelated; }
.overlay { position: absolute; inset: 0; display: grid; place-items: center; text-align: center; padding: 16px; background: rgba(0, 0, 0, 0.55); }
.keys { margin: 0; }
</style>
