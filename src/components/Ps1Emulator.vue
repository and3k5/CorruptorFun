<script setup>
import { onBeforeUnmount, ref } from 'vue';
import { makeZip } from '../lib/zip.js';

// Runs the corrupted PS1 BIOS for real, inside an EmulatorJS (pcsx_rearmed)
// frame. Without a disc the BIOS plays its intro and drops into the memory
// card / CD player shell; with the user's own game disc it takes the licensed
// path into the game.
const props = defineProps({
  bios: { type: Object, required: true }, // corrupted Uint8Array
  disc: { type: Object, default: null }, // optional user File
});

const frame = ref(null);
const frameKey = ref(0);
const status = ref('Press Boot to run the corrupted BIOS.');
const running = ref(false);

// pcsx_rearmed looks for these names in its system folder.
const BIOS_NAMES = ['scph5501.bin', 'scph1001.bin', 'scph7001.bin', 'scph101.bin'];

// A 4 second silent audio CD: enough for the BIOS to see a disc that isn't a
// game, so it plays the intro and opens the shell.
function silentAudioCd() {
  const bin = new Uint8Array(300 * 2352);
  const cue = new TextEncoder().encode('FILE "nodisc.bin" BINARY\r\n  TRACK 01 AUDIO\r\n    INDEX 01 00:00:00\r\n');
  return makeZip([
    { name: 'nodisc.cue', data: cue },
    { name: 'nodisc.bin', data: bin },
  ]);
}

let pending = null;
function onMessage(ev) {
  // The frame is sandboxed with an opaque origin, so match it by reference.
  if (!frame.value || ev.source !== frame.value.contentWindow) return;
  if (ev.data?.type === 'ps1-ready' && pending) {
    frame.value.contentWindow.postMessage(pending, '*');
    pending = null;
  } else if (ev.data?.type === 'ps1-status') {
    status.value = ev.data.text;
  }
}
window.addEventListener('message', onMessage);
onBeforeUnmount(() => window.removeEventListener('message', onMessage));

function boot() {
  const biosZip = makeZip(BIOS_NAMES.map((name) => ({ name, data: props.bios })));
  pending = { type: 'ps1-boot', biosZip, disc: props.disc || silentAudioCd() };
  status.value = 'Starting…';
  running.value = true;
  // A fresh frame per boot: EmulatorJS keeps its state in globals.
  frameKey.value++;
}

function stop() {
  running.value = false;
  pending = null;
  status.value = 'Stopped.';
}

// VITE_EJS_LOCAL points at a locally served EmulatorJS for offline testing.
const query = new URLSearchParams();
if (import.meta.env.VITE_EJS_LOCAL) query.set('ejs', import.meta.env.VITE_EJS_LOCAL);
if (import.meta.env.VITE_EJS_DEBUG) query.set('debug', '1');
const src = `${import.meta.env.BASE_URL}emu/ps1.html${query.size ? `?${query}` : ''}`;
defineExpose({ boot });
</script>

<template>
  <div class="emu">
    <div class="screen">
      <iframe v-if="running" :key="frameKey" ref="frame" :src="src" title="PS1 emulator" sandbox="allow-scripts allow-pointer-lock" allow="autoplay; gamepad; fullscreen; screen-wake-lock" />
      <div v-else class="idle note">{{ status }}</div>
    </div>
    <div class="row">
      <button class="primary" @click="boot">{{ running ? 'Reboot' : 'Boot' }}</button>
      <button v-if="running" @click="stop">Stop</button>
      <span class="note">{{ running ? status : '' }}</span>
    </div>
  </div>
</template>

<style scoped>
.emu { display: grid; gap: 8px; }
.screen { aspect-ratio: 4 / 3; background: #000; border-radius: 8px; overflow: hidden; display: grid; }
iframe { width: 100%; height: 100%; border: 0; }
.idle { place-self: center; padding: 16px; text-align: center; }
</style>
