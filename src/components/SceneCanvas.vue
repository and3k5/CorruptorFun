<script setup>
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { W, H } from '../scenes/common.js';

const props = defineProps({
  scene: { type: Object, required: true },
  feed: { type: Object, required: true },
  licensed: { type: Boolean, default: false },
  sound: { type: Boolean, default: false },
  // Bump to restart the boot from the top.
  bootKey: { type: Number, default: 0 },
});

const canvas = ref(null);
const progress = ref(0);
const playing = ref(true);
let raf = 0;
let start = 0;
let pausedAt = 0;
let audio = null;

function duration() {
  return props.scene.duration({ licensed: props.licensed });
}

function restart() {
  start = performance.now();
  pausedAt = 0;
  playing.value = true;
  if (props.sound) {
    try {
      audio = audio || new AudioContext();
      audio.resume();
      props.scene.sound?.(audio, props.feed);
    } catch {
      // Audio is a nice-to-have.
    }
  }
}

function frame(now) {
  raf = requestAnimationFrame(frame);
  const ctx = canvas.value?.getContext('2d');
  if (!ctx) return;
  const total = duration();
  let t = playing.value ? (now - start) / 1000 : pausedAt;
  // Hold on the last frame once the boot is over.
  t = Math.min(t, total);
  if (playing.value && t >= total) {
    playing.value = false;
    pausedAt = total;
  }
  // A corrupted "timing" slice makes the whole boot stutter like a sick GPU.
  const freeze = props.feed.glitch(`${props.scene.id}.timing.stutter`);
  if (freeze) {
    const fps = 2 + (1 - freeze) * 22;
    t = Math.floor(t * fps) / fps;
  }
  const speed = props.feed.value(`${props.scene.id}.timing.speed`, 1, 0.6);
  progress.value = t / total;
  ctx.save();
  ctx.font = '16px sans-serif';
  ctx.textBaseline = 'alphabetic';
  props.scene.draw(ctx, t * Math.max(0.2, speed), props.feed, { licensed: props.licensed });
  ctx.restore();
}

function togglePlay() {
  if (playing.value) {
    pausedAt = Math.min(duration(), (performance.now() - start) / 1000);
    playing.value = false;
  } else if (pausedAt >= duration()) {
    restart();
  } else {
    start = performance.now() - pausedAt * 1000;
    playing.value = true;
  }
}

function seek(ev) {
  const rect = ev.currentTarget.getBoundingClientRect();
  const f = Math.min(1, Math.max(0, (ev.clientX - rect.left) / rect.width));
  pausedAt = f * duration();
  start = performance.now() - pausedAt * 1000;
}

function snapshot() {
  const a = document.createElement('a');
  a.href = canvas.value.toDataURL('image/png');
  a.download = `${props.scene.id}-corrupted.png`;
  a.click();
}

watch(() => [props.bootKey, props.licensed], restart);

onMounted(() => {
  restart();
  raf = requestAnimationFrame(frame);
});
onBeforeUnmount(() => {
  cancelAnimationFrame(raf);
  audio?.close();
});

defineExpose({ restart });
</script>

<template>
  <div class="scene">
    <canvas ref="canvas" :width="W" :height="H" aria-label="Simulated boot sequence" />
    <div class="controls">
      <button class="small" @click="togglePlay">{{ playing ? 'Pause' : 'Play' }}</button>
      <button class="small" @click="restart">Restart</button>
      <div class="track" @click="seek"><div class="fill" :style="{ width: progress * 100 + '%' }" /></div>
      <button class="small" @click="snapshot">Save PNG</button>
    </div>
  </div>
</template>

<style scoped>
.scene { display: grid; gap: 8px; }
canvas { width: 100%; height: auto; aspect-ratio: 4 / 3; background: #000; border-radius: 8px; image-rendering: pixelated; display: block; }
.controls { display: flex; gap: 8px; align-items: center; }
.track { flex: 1; height: 8px; background: var(--panel-2); border-radius: 4px; cursor: pointer; overflow: hidden; }
.fill { height: 100%; background: var(--accent); }
</style>
