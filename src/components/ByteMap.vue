<script setup>
import { computed, onMounted, ref, watch } from 'vue';

// A strip showing the whole file left to right, with the corruption range
// shaded and each corrupted byte marked.
const props = defineProps({
  size: { type: Number, required: true },
  hits: { type: Array, default: () => [] },
  start: { type: Number, default: 0 },
  end: { type: Number, default: 0 },
  original: { type: Object, default: null },
  corrupted: { type: Object, default: null },
});

const canvas = ref(null);
const COLS = 1024;

function draw() {
  const c = canvas.value;
  if (!c) return;
  const ctx = c.getContext('2d');
  ctx.fillStyle = '#0a0d10';
  ctx.fillRect(0, 0, COLS, c.height);
  if (!props.size) return;
  const x0 = (props.start / props.size) * COLS;
  const x1 = (props.end / props.size) * COLS;
  ctx.fillStyle = 'rgba(53, 208, 127, 0.16)';
  ctx.fillRect(x0, 0, Math.max(1, x1 - x0), c.height);
  const buckets = new Uint32Array(COLS);
  for (const h of props.hits) buckets[Math.floor((h / props.size) * COLS)]++;
  let max = 1;
  for (const b of buckets) max = Math.max(max, b);
  for (let x = 0; x < COLS; x++) {
    if (!buckets[x]) continue;
    const k = 0.35 + 0.65 * (buckets[x] / max);
    ctx.fillStyle = `rgba(255, 79, 123, ${k})`;
    ctx.fillRect(x, 0, 1, c.height);
  }
}

const hex = (n, w = 6) => '0x' + n.toString(16).toUpperCase().padStart(w, '0');
const sample = computed(() =>
  props.hits.slice(0, 6).map((o) => ({
    at: hex(o),
    from: props.original?.[o]?.toString(16).padStart(2, '0'),
    to: props.corrupted?.[o]?.toString(16).padStart(2, '0'),
  })),
);

watch(() => [props.hits, props.start, props.end, props.size], draw);
onMounted(draw);
</script>

<template>
  <div class="map">
    <canvas ref="canvas" :width="COLS" height="28" aria-label="Where the corrupted bytes are" />
    <div class="legend note">
      <span>{{ hits.length.toLocaleString() }} bytes changed</span>
      <span v-for="s in sample" :key="s.at" class="mono">{{ s.at }}: {{ s.from }}→{{ s.to }}</span>
      <span v-if="hits.length > sample.length">…</span>
    </div>
  </div>
</template>

<style scoped>
.map { display: grid; gap: 6px; }
canvas { width: 100%; height: 28px; border-radius: 4px; border: 1px solid var(--line); display: block; }
.legend { display: flex; flex-wrap: wrap; gap: 4px 14px; }
</style>
