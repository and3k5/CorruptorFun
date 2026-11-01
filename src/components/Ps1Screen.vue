<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue';
import wasmUrl from '../wasm/ps1.wasm?url';
import workletUrl from '../emu/ps1-audio.worklet.js?url&no-inline';
import { findAscii } from '../lib/consoles.js';

// Runs the corrupted BIOS in our own PS1 core (Rust → WebAssembly) inside a
// worker, draws its frames and plays its audio.
const props = defineProps({
  bios: { type: Object, required: true }, // corrupted Uint8Array
  disc: { type: Object, default: null }, // raw data track Uint8Array
  region: { type: String, default: 'E' },
  // Keep running after the BIOS starts the disc's program (our test disc's
  // demo). Real games stop there: only the boot is emulated.
  runGame: { type: Boolean, default: false },
  // Float a small copy of the screen in the corner while it's scrolled away.
  mini: { type: Boolean, default: false },
});

const canvas = ref(null);
const holder = ref(null);
const status = ref('Press Boot to run the BIOS.');
const running = ref(false);
const muted = ref(false);
const offscreen = ref(false);
const report = ref(''); // debug report text once requested
const reportNote = ref('');
let worker = null;
let audio = null; // { ctx, node, gain } once the worklet is loaded
let audioReady = null;
let lastFrame = 0;
let watchdog = 0;
let pending = null; // newest frame not yet drawn
let raf = 0;
let observer = null;

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

function draw() {
  raf = 0;
  const c = canvas.value;
  const f = pending;
  pending = null;
  if (!c || !f) return;
  if (c.width !== f.w || c.height !== f.h) {
    c.width = f.w;
    c.height = f.h;
  }
  c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(f.pixels), f.w, f.h), 0, 0);
}

// Loads the audio worklet once. Must start from a click so the browser lets
// the page make sound.
function setupAudio() {
  if (audioReady) return audioReady;
  audioReady = (async () => {
    try {
      const ctx = new AudioContext({ sampleRate: 44100, latencyHint: 'interactive' });
      ctx.resume();
      await ctx.audioWorklet.addModule(workletUrl);
      const node = new AudioWorkletNode(ctx, 'ps1-audio', { outputChannelCount: [2] });
      const gain = ctx.createGain();
      gain.gain.value = muted.value ? 0 : 1;
      node.connect(gain).connect(ctx.destination);
      audio = { ctx, node, gain };
    } catch {
      audio = null; // no sound, but the boot still runs
    }
    return audio;
  })();
  return audioReady;
}

// Ends the run but keeps the worker, so its debug report can still be read.
function halt(text) {
  running.value = false;
  clearInterval(watchdog);
  status.value = text;
  worker?.postMessage({ type: 'stop' });
}

function stop(text = 'Stopped.') {
  worker?.terminate();
  worker = null;
  halt(text);
}

function reportHeader() {
  const at = findAscii(props.bios, 'System ROM Version');
  let version = 'unknown';
  if (at >= 0) {
    let end = at;
    while (end < props.bios.length && props.bios[end] >= 0x20 && props.bios[end] < 0x7f) end++;
    version = String.fromCharCode(...props.bios.subarray(at, end));
  }
  return `BIOS: ${version}\nRegion: ${props.region}, disc: ${props.disc ? `${props.disc.length} bytes` : 'none'}`;
}

function showReport() {
  if (!worker) {
    report.value = 'Boot first, then get the report while it runs or after it stops.';
    return;
  }
  reportNote.value = '';
  worker.postMessage({ type: 'report' });
}

async function copyReport() {
  try {
    await navigator.clipboard.writeText(report.value);
    reportNote.value = 'Copied.';
  } catch {
    reportNote.value = 'Copying was blocked; select the text and copy it, or download it.';
  }
}

function downloadReport() {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([report.value], { type: 'text/plain' }));
  a.download = 'ps1-debug-report.txt';
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

async function boot() {
  stop();
  const ready = setupAudio();
  audio?.ctx.resume();
  // Copies, so the stored original stays untouched and we can transfer.
  const bios = props.bios.slice().buffer;
  const disc = props.disc ? props.disc.slice().buffer : null;
  const w = new Worker(new URL('../emu/ps1.worker.js', import.meta.url), { type: 'module' });
  worker = w;
  running.value = true;
  status.value = props.disc ? 'Booting with your disc…' : 'Booting with no disc…';
  await ready;
  if (worker !== w) return; // stopped or rebooted meanwhile
  audio?.ctx.resume();
  w.onmessage = (ev) => {
    const m = ev.data;
    if (m.type === 'frame') {
      lastFrame = performance.now();
      pending = m;
      raf = raf || requestAnimationFrame(draw);
    } else if (m.type === 'game-started') {
      if (props.runGame) status.value = "The boot finished, and the test disc's demo is running.";
      else halt('The BIOS handed over to the game here. Only the boot is emulated, so it stops.');
    } else if (m.type === 'error') {
      halt(m.message);
    } else if (m.type === 'report') {
      report.value = `${reportHeader()}\n${m.text}`;
    }
  };
  // The worker posts audio straight to the audio thread.
  let audioPort = null;
  if (audio) {
    const ch = new MessageChannel();
    audio.node.port.postMessage('reset');
    audio.node.port.postMessage({ port: ch.port1 }, [ch.port1]);
    audioPort = ch.port2;
  }
  const transfer = [bios];
  if (disc) transfer.push(disc);
  if (audioPort) transfer.push(audioPort);
  w.postMessage({ type: 'start', wasmUrl: new URL(wasmUrl, location.href).href, bios, disc, region: props.region, runGame: props.runGame, audioPort }, transfer);
  lastFrame = performance.now();
  watchdog = setInterval(() => {
    if (running.value && performance.now() - lastFrame > 4000) {
      stop('The emulator stopped responding. Try another corruption.');
    }
  }, 1000);
}

function toggleMute() {
  muted.value = !muted.value;
  if (audio) audio.gain.gain.value = muted.value ? 0 : 1;
}

function backToScreen() {
  holder.value?.scrollIntoView({ behavior: 'smooth', block: 'center' });
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
  observer = new IntersectionObserver(([e]) => (offscreen.value = e.intersectionRatio < 0.35), { threshold: [0, 0.35, 1] });
  observer.observe(holder.value);
});
onBeforeUnmount(() => {
  stop();
  observer?.disconnect();
  cancelAnimationFrame(raf);
  window.removeEventListener('keydown', onKey);
  window.removeEventListener('keyup', onKey);
  audio?.ctx.close();
});

defineExpose({ boot, stop });
</script>

<template>
  <div class="emu">
    <div ref="holder" class="holder">
      <div class="screen" :class="{ mini: mini && running && offscreen }">
        <canvas ref="canvas" width="320" height="240" aria-label="PlayStation output" />
        <div v-if="!running" class="overlay note">{{ status }}</div>
        <button v-if="mini && running && offscreen" class="back" aria-label="Scroll back to the screen" @click="backToScreen" />
      </div>
    </div>
    <div class="row">
      <button class="primary" @click="boot">{{ running ? 'Reboot' : 'Boot' }}</button>
      <button v-if="running" @click="halt('Stopped.')">Stop</button>
      <button @click="toggleMute">{{ muted ? 'Unmute' : 'Mute' }}</button>
      <button @click="snapshot">Save PNG</button>
      <button @click="showReport">Debug report</button>
      <span v-if="running" class="note">{{ status }}</span>
    </div>
    <div v-if="report" class="report">
      <textarea readonly :value="report" rows="10" aria-label="Debug report" />
      <div class="row">
        <button class="small" @click="copyReport">Copy</button>
        <button class="small" @click="downloadReport">Download .txt</button>
        <button class="small" @click="showReport">Refresh</button>
        <button class="small" @click="report = ''">Close</button>
        <span class="note">{{ reportNote }}</span>
      </div>
    </div>
    <p class="note keys">
      Pad: arrows, Enter = Start, Backspace = Select, Z = ✕, X = ○, A = □, S = △, Q/W = L1/R1, E/R = L2/R2.
    </p>
  </div>
</template>

<style scoped>
.emu { display: grid; gap: 8px; }
.holder { aspect-ratio: 4 / 3; }
.screen { position: relative; width: 100%; height: 100%; background: #000; border-radius: 8px; overflow: hidden; }
.screen.mini {
  position: fixed; z-index: 50; right: 12px; bottom: 12px; width: min(46vw, 280px); height: auto; aspect-ratio: 4 / 3;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.6); outline: 1px solid var(--line, #333);
}
.back { position: absolute; inset: 0; background: none; border: 0; padding: 0; cursor: pointer; }
canvas { width: 100%; height: 100%; display: block; image-rendering: pixelated; }
.overlay { position: absolute; inset: 0; display: grid; place-items: center; text-align: center; padding: 16px; background: rgba(0, 0, 0, 0.55); }
.keys { margin: 0; }
.report { display: grid; gap: 6px; }
.report textarea { width: 100%; box-sizing: border-box; font: 12px/1.4 var(--mono); background: var(--panel-2); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px; }
</style>
