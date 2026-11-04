<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue';
import {
  MAX_COLOURS,
  SIZES,
  cloneDrawing,
  coloursSetting,
  createDrawing,
  drawingToImage,
  floodFill,
  pick,
  resizeDrawing,
  stamp,
  strokeLine,
  visibleColour,
} from '../lib/ps1custom/draw.js';
import { imageToLogo, MAX_TRIANGLES } from '../lib/ps1custom/logo.js';
import { pixelsToUrl } from '../lib/ps1custom/browser.js';

// A full-screen pixel drawing editor for the logo on the licensed screen.
// Works with a mouse, a pen or fingers: one pointer draws, two fingers pinch
// to zoom and drag to pan (and so do Ctrl + wheel and trackpad pinches).
const props = defineProps({
  initial: { type: Object, default: null }, // a drawing (see draw.js)
  depth: { type: Boolean, default: false }, // 3D sides, for the triangle count
  error: { type: String, default: '' },
});
const emit = defineEmits(['apply', 'close']);

const d = shallowRef(props.initial ? cloneDrawing(props.initial) : createDrawing());
const version = ref(0); // bumped on every change to d
const touched = () => version.value++;

const tool = ref('pen'); // pen, eraser, fill, pick, pan
const size = ref(2);
const colour = ref(1);
const TOOLS = [
  { id: 'pen', label: 'Pen', icon: '✎', key: 'b' },
  { id: 'eraser', label: 'Eraser', icon: '⌫', key: 'e' },
  { id: 'fill', label: 'Fill', icon: '◧', key: 'f' },
  { id: 'pick', label: 'Pick colour', icon: '⊙', key: 'i' },
  { id: 'pan', label: 'Move', icon: '✥', key: 'h' },
];

// Undo / redo: snapshots of the whole drawing (they're tiny).
const undoStack = shallowRef([]);
const redoStack = shallowRef([]);
function checkpoint() {
  undoStack.value = [...undoStack.value.slice(-79), cloneDrawing(d.value)];
  redoStack.value = [];
}
function undo() {
  if (!undoStack.value.length) return;
  redoStack.value = [...redoStack.value, cloneDrawing(d.value)];
  d.value = undoStack.value.at(-1);
  undoStack.value = undoStack.value.slice(0, -1);
  touched();
}
function redo() {
  if (!redoStack.value.length) return;
  undoStack.value = [...undoStack.value, cloneDrawing(d.value)];
  d.value = redoStack.value.at(-1);
  redoStack.value = redoStack.value.slice(0, -1);
  touched();
}
function clearAll() {
  if (!d.value.pixels.some(Boolean)) return;
  checkpoint();
  d.value.pixels.fill(0);
  touched();
}

// Size of the grid.
const sizeKey = computed(() => `${d.value.width}x${d.value.height}`);
const sizeOptions = computed(() => {
  const list = SIZES.map(([w, h]) => ({ key: `${w}x${h}`, w, h }));
  if (!list.some((o) => o.key === sizeKey.value)) list.push({ key: sizeKey.value, w: d.value.width, h: d.value.height });
  return list;
});
function setSize(key) {
  const o = sizeOptions.value.find((s) => s.key === key);
  if (!o || key === sizeKey.value) return;
  checkpoint();
  d.value = resizeDrawing(d.value, o.w, o.h);
  touched();
  nextTick(fit);
}

// Colours.
const hex = (c) => '#' + c.map((v) => v.toString(16).padStart(2, '0')).join('');
const fromHex = (s) => [1, 3, 5].map((i) => parseInt(s.slice(i, i + 2), 16));
const swatches = computed(() => (version.value, d.value.palette.map(hex)));
const colourHex = computed(() => swatches.value[colour.value] || '#ffffff');
const darkened = ref(false);
let colourEditing = false;
function setColour(value) {
  const raw = fromHex(value);
  const c = visibleColour(raw);
  darkened.value = c.some((v, i) => v !== raw[i]);
  if (!colourEditing) {
    checkpoint();
    colourEditing = true;
  }
  d.value.palette[colour.value] = c;
  touched();
}
function addColour() {
  if (d.value.palette.length > MAX_COLOURS) return;
  checkpoint();
  d.value.palette.push([...(d.value.palette[colour.value] || [255, 255, 255])]);
  colour.value = d.value.palette.length - 1;
  touched();
}
function choose(i) {
  colour.value = i;
  colourEditing = false;
  darkened.value = false;
  if (tool.value === 'eraser' || tool.value === 'pick' || tool.value === 'pan') tool.value = 'pen';
}

// The view: `scale` screen pixels per drawing pixel, drawing origin at (ox, oy).
const view = ref({ scale: 8, ox: 0, oy: 0 });
const viewport = ref(null);
const canvas = ref(null);
let vw = 0, vh = 0;
const fitScale = () => Math.max(0.5, Math.min((vw - 16) / d.value.width, (vh - 16) / d.value.height));
function fit() {
  const s = fitScale();
  view.value = { scale: s, ox: (vw - d.value.width * s) / 2, oy: (vh - d.value.height * s) / 2 };
}
function zoomAt(factor, sx, sy) {
  const v = view.value;
  const s = Math.min(80, Math.max(fitScale() * 0.5, v.scale * factor));
  const k = s / v.scale;
  view.value = { scale: s, ox: sx - (sx - v.ox) * k, oy: sy - (sy - v.oy) * k };
}
const zoomBy = (f) => zoomAt(f, vw / 2, vh / 2);
const zoomPercent = computed(() => Math.round((view.value.scale / fitScale()) * 100));

// Drawing on the canvas.
const off = document.createElement('canvas');
function render() {
  const cv = canvas.value;
  if (!cv) return;
  const dpr = window.devicePixelRatio || 1;
  if (cv.width !== Math.round(vw * dpr) || cv.height !== Math.round(vh * dpr)) {
    cv.width = Math.round(vw * dpr);
    cv.height = Math.round(vh * dpr);
  }
  const g = cv.getContext('2d');
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, vw, vh);
  const { width: w, height: h } = d.value;
  if (off.width !== w || off.height !== h) {
    off.width = w;
    off.height = h;
  }
  const img = drawingToImage(d.value);
  off.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(img.data.buffer), w, h), 0, 0);
  const { scale: s, ox, oy } = view.value;
  g.fillStyle = '#000';
  g.fillRect(ox, oy, w * s, h * s);
  g.imageSmoothingEnabled = false;
  g.drawImage(off, ox, oy, w * s, h * s);
  if (s >= 6) {
    g.strokeStyle = 'rgba(255,255,255,0.07)';
    g.lineWidth = 1;
    g.beginPath();
    const x0 = Math.max(0, Math.floor(-ox / s)), x1 = Math.min(w, Math.ceil((vw - ox) / s));
    const y0 = Math.max(0, Math.floor(-oy / s)), y1 = Math.min(h, Math.ceil((vh - oy) / s));
    for (let x = x0; x <= x1; x++) {
      const px = Math.round(ox + x * s) + 0.5;
      g.moveTo(px, oy + y0 * s);
      g.lineTo(px, oy + y1 * s);
    }
    for (let y = y0; y <= y1; y++) {
      const py = Math.round(oy + y * s) + 0.5;
      g.moveTo(ox + x0 * s, py);
      g.lineTo(ox + x1 * s, py);
    }
    g.stroke();
  }
  g.strokeStyle = 'rgba(255,255,255,0.25)';
  g.strokeRect(ox - 0.5, oy - 0.5, w * s + 1, h * s + 1);
  // Brush outline under a mouse or pen hovering over the drawing.
  if (hover.value && (tool.value === 'pen' || tool.value === 'eraser')) {
    const n = size.value;
    const { x, y } = hover.value;
    const cx = n % 2 ? Math.floor(x) + 0.5 : Math.round(x);
    const cy = n % 2 ? Math.floor(y) + 0.5 : Math.round(y);
    g.strokeStyle = 'rgba(255,255,255,0.8)';
    g.beginPath();
    g.arc(ox + cx * s, oy + cy * s, Math.max(1.5, (n / 2) * s), 0, Math.PI * 2);
    g.stroke();
  }
}
let frame = 0;
function scheduleRender() {
  if (!frame) frame = requestAnimationFrame(() => ((frame = 0), render()));
}
const hover = ref(null);
watch([version, view, hover, tool, size], scheduleRender);

const toDrawing = (e) => {
  const r = canvas.value.getBoundingClientRect();
  const { scale: s, ox, oy } = view.value;
  return { x: (e.clientX - r.left - ox) / s, y: (e.clientY - r.top - oy) / s };
};
const toScreen = (e) => {
  const r = canvas.value.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
};

// Pointer handling: one pointer draws (or pans), two pinch and pan. A
// stroke that turns out to be the start of a pinch is taken back.
const pointers = new Map();
let mode = null; // 'draw', 'pan', 'pinch', 'wait' (fingers lifting after a pinch)
let last = null;
let strokeBase = null;
let pinch = null;
let spaceDown = false;
let downAt = null; // where a stroke started, on screen

function startPinch() {
  const [a, b] = [...pointers.values()];
  pinch = { d0: Math.hypot(a.x - b.x, a.y - b.y) || 1, m0: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, v0: { ...view.value } };
}

function onDown(e) {
  if (e.pointerType === 'mouse' && e.button !== 0 && e.button !== 1) return;
  e.preventDefault();
  canvas.value.setPointerCapture(e.pointerId);
  pointers.set(e.pointerId, toScreen(e));
  if (pointers.size === 2) {
    if (mode === 'draw' && strokeBase) {
      // That was a pinch, not a dot: put the drawing back.
      d.value = strokeBase;
      undoStack.value = undoStack.value.slice(0, -1);
      touched();
    }
    strokeBase = null;
    mode = 'pinch';
    startPinch();
    return;
  }
  if (pointers.size > 2 || mode) return;
  hover.value = null;
  const p = toDrawing(e);
  if (tool.value === 'pan' || e.button === 1 || spaceDown) {
    mode = 'pan';
    last = toScreen(e);
    return;
  }
  if (tool.value === 'pick') {
    const c = pick(d.value, p.x, p.y);
    if (c > 0) choose(c);
    else if (c === 0) tool.value = 'eraser';
    mode = 'wait';
    return;
  }
  if (tool.value === 'fill') {
    const before = cloneDrawing(d.value);
    if (floodFill(d.value, p.x, p.y, colour.value)) {
      undoStack.value = [...undoStack.value.slice(-79), before];
      redoStack.value = [];
      touched();
    }
    mode = 'wait';
    return;
  }
  // Pen or eraser.
  strokeBase = cloneDrawing(d.value);
  checkpoint();
  mode = 'draw';
  last = p;
  downAt = toScreen(e);
  stamp(d.value, p.x, p.y, size.value, tool.value === 'eraser' ? 0 : colour.value);
  touched();
}

function onMove(e) {
  if (!pointers.has(e.pointerId)) {
    if (e.pointerType !== 'touch') hover.value = toDrawing(e);
    return;
  }
  pointers.set(e.pointerId, toScreen(e));
  if (mode === 'pinch' && pointers.size >= 2 && pinch) {
    const [a, b] = [...pointers.values()];
    const dist = Math.hypot(a.x - b.x, a.y - b.y);
    const m = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
    const { v0, d0, m0 } = pinch;
    const s = Math.min(80, Math.max(fitScale() * 0.5, (v0.scale * dist) / d0));
    // Keep the drawing point that was under the fingers' midpoint under it.
    const px = (m0.x - v0.ox) / v0.scale, py = (m0.y - v0.oy) / v0.scale;
    view.value = { scale: s, ox: m.x - px * s, oy: m.y - py * s };
  } else if (mode === 'pan') {
    const p = toScreen(e);
    view.value = { ...view.value, ox: view.value.ox + p.x - last.x, oy: view.value.oy + p.y - last.y };
    last = p;
  } else if (mode === 'draw') {
    const value = tool.value === 'eraser' ? 0 : colour.value;
    const events = e.getCoalescedEvents?.() || [e];
    let changed = false;
    for (const ev of events.length ? events : [e]) {
      const p = toDrawing(ev);
      if (strokeLine(d.value, last.x, last.y, p.x, p.y, size.value, value)) changed = true;
      last = p;
    }
    if (changed) touched();
    // Past the first few pixels it's a real stroke: a second finger now
    // starts a pinch but doesn't take the stroke back.
    if (strokeBase && downAt) {
      const p = toScreen(e);
      if (Math.hypot(p.x - downAt.x, p.y - downAt.y) > 24) strokeBase = null;
    }
  }
  if (e.pointerType !== 'touch') hover.value = toDrawing(e);
}

function onUp(e) {
  if (!pointers.has(e.pointerId)) return;
  pointers.delete(e.pointerId);
  if (mode === 'pinch') {
    if (pointers.size >= 2) startPinch();
    else mode = pointers.size ? 'wait' : null;
    return;
  }
  if (pointers.size === 0) {
    if (mode === 'draw') {
      // Nothing changed after all: drop the undo step.
      const base = undoStack.value.at(-1);
      if (base && base.pixels.every((v, i) => v === d.value.pixels[i])) undoStack.value = undoStack.value.slice(0, -1);
    }
    mode = null;
    strokeBase = null;
    downAt = null;
  }
}

function onLeave(e) {
  if (e.pointerType !== 'touch') hover.value = null;
}

function onWheel(e) {
  const p = toScreen(e);
  if (e.ctrlKey || e.metaKey) {
    // Ctrl + wheel, and trackpad pinches (which arrive as Ctrl + wheel).
    e.preventDefault();
    zoomAt(Math.exp(-e.deltaY * (e.deltaMode ? 0.05 : 0.01)), p.x, p.y);
  } else {
    e.preventDefault();
    const k = e.deltaMode ? 16 : 1;
    if (e.shiftKey && !e.deltaX) view.value = { ...view.value, ox: view.value.ox - e.deltaY * k };
    else view.value = { ...view.value, ox: view.value.ox - e.deltaX * k, oy: view.value.oy - e.deltaY * k };
  }
}

function onKey(e) {
  if (e.target instanceof HTMLInputElement && e.target.type !== 'range' && e.target.type !== 'checkbox') return;
  if (e.target instanceof HTMLSelectElement) return;
  const mod = e.ctrlKey || e.metaKey;
  const k = e.key.toLowerCase();
  if (mod && k === 'z') {
    e.preventDefault();
    e.shiftKey ? redo() : undo();
  } else if (mod && k === 'y') {
    e.preventDefault();
    redo();
  } else if (mod) return;
  else if (e.key === 'Escape') emit('close', cloneDrawing(d.value));
  else if (e.key === ' ') {
    spaceDown = true;
    e.preventDefault();
  } else if (e.key === '[') size.value = Math.max(1, size.value - 1);
  else if (e.key === ']') size.value = Math.min(16, size.value + 1);
  else if (e.key === '+' || e.key === '=') zoomBy(1.25);
  else if (e.key === '-') zoomBy(0.8);
  else if (e.key === '0') fit();
  else {
    const t = TOOLS.find((t) => t.key === k);
    if (t) tool.value = t.id;
  }
}
function onKeyUp(e) {
  if (e.key === ' ') spaceDown = false;
}

let ro = null;
onMounted(() => {
  ro = new ResizeObserver(() => {
    const r = viewport.value.getBoundingClientRect();
    const first = !vw;
    const was = vw ? fitScale() : 0;
    vw = r.width;
    vh = r.height;
    if (first || Math.abs(view.value.scale - was) < 0.01) fit();
    scheduleRender();
  });
  ro.observe(viewport.value);
  window.addEventListener('keydown', onKey);
  window.addEventListener('keyup', onKeyUp);
  document.documentElement.style.overflow = 'hidden';
});
onBeforeUnmount(() => {
  ro?.disconnect();
  window.removeEventListener('keydown', onKey);
  window.removeEventListener('keyup', onKeyUp);
  document.documentElement.style.overflow = '';
  cancelAnimationFrame(frame);
});

// What the BIOS will get: the model logo.js builds, counted after each change
// (shortly after, so long strokes don't stall).
const result = shallowRef(null);
let timer = 0;
watch(
  [version, () => props.depth],
  () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const img = drawingToImage(d.value);
      if (!img.data.some(Boolean)) return (result.value = { empty: true });
      try {
        const r = imageToLogo(img, { colours: coloursSetting(d.value), depth: props.depth });
        result.value = { ...r, url: pixelsToUrl({ width: r.gw, height: r.gh, data: r.preview }, 2) };
      } catch (e) {
        result.value = { error: e.message };
      }
    }, mode === 'draw' ? 250 : 60);
  },
  { immediate: true },
);
const status = computed(() => {
  const r = result.value;
  if (!r) return '';
  if (r.empty) return 'Empty: the CF logo stays until you draw something.';
  if (r.error) return r.error;
  const coarser = r.gw < d.value.width;
  return `${r.triangles} of ${MAX_TRIANGLES} triangles` + (coarser ? `. Too detailed for ${d.value.width}×${d.value.height}, so it becomes ${r.gw}×${r.gh} squares (see the preview).` : `, at ${r.gw}×${r.gh} squares.`);
});
const fill = computed(() => (result.value?.triangles ? Math.min(100, (result.value.triangles / MAX_TRIANGLES) * 100) : 0));

function apply() {
  emit('apply', cloneDrawing(d.value));
}
</script>

<template>
  <Teleport to="body">
    <div class="draw" role="dialog" aria-modal="true" aria-label="Draw the logo">
      <header class="bar">
        <strong class="title">Draw the logo</strong>
        <div class="row">
          <button class="small" :disabled="!undoStack.length" title="Undo (Ctrl+Z)" aria-label="Undo" @click="undo">↶</button>
          <button class="small" :disabled="!redoStack.length" title="Redo (Ctrl+Shift+Z)" aria-label="Redo" @click="redo">↷</button>
          <button class="small danger" title="Clear the drawing" @click="clearAll">Clear</button>
          <button class="small" title="Close (Esc). Your drawing is kept until you reload." @click="emit('close', cloneDrawing(d))">Cancel</button>
          <button class="small primary" :disabled="!result || result.empty || !!result.error" @click="apply">Use drawing</button>
        </div>
      </header>

      <div class="bar tools">
        <div class="seg" role="radiogroup" aria-label="Tool">
          <button
            v-for="t in TOOLS"
            :key="t.id"
            class="small"
            :class="{ on: tool === t.id }"
            role="radio"
            :aria-checked="tool === t.id"
            :title="`${t.label} (${t.key.toUpperCase()})`"
            @click="tool = t.id"
          >
            <span aria-hidden="true">{{ t.icon }}</span><span class="tl">{{ t.label }}</span>
          </button>
        </div>
        <label class="inline" title="Stroke width ([ and ])">
          Width
          <input v-model.number="size" type="range" min="1" max="16" />
          <span class="mono num">{{ size }}</span>
        </label>
        <div class="row zoom">
          <button class="small" aria-label="Zoom out" title="Zoom out (-)" @click="zoomBy(0.8)">−</button>
          <button class="small mono" title="Fit (0)" @click="fit">{{ zoomPercent }}%</button>
          <button class="small" aria-label="Zoom in" title="Zoom in (+)" @click="zoomBy(1.25)">+</button>
        </div>
        <label class="inline">
          Grid
          <select :value="sizeKey" @change="setSize($event.target.value)">
            <option v-for="o in sizeOptions" :key="o.key" :value="o.key">{{ o.w }}×{{ o.h }}</option>
          </select>
        </label>
      </div>

      <div class="bar palette">
        <button
          v-for="(c, i) in swatches.slice(1)"
          :key="i"
          class="swatch"
          :class="{ on: colour === i + 1 && tool !== 'eraser' }"
          :style="{ background: c }"
          :aria-label="`Colour ${c}`"
          @click="choose(i + 1)"
        />
        <button v-if="swatches.length <= MAX_COLOURS" class="swatch add" title="Add a colour" aria-label="Add a colour" @click="addColour">+</button>
        <label class="edit" title="Change this colour (everything drawn in it changes too)">
          <span class="swatch big" :style="{ background: colourHex }" />
          <span>Edit</span>
          <input type="color" :value="colourHex" @input="setColour($event.target.value)" @change="colourEditing = false" />
        </label>
      </div>

      <div
        ref="viewport"
        class="viewport"
        :class="['t-' + tool]"
        @contextmenu.prevent
      >
        <canvas
          ref="canvas"
          @pointerdown="onDown"
          @pointermove="onMove"
          @pointerup="onUp"
          @pointercancel="onUp"
          @pointerleave="onLeave"
          @wheel="onWheel"
        />
      </div>

      <footer class="bar foot">
        <img v-if="result?.url" :src="result.url" alt="What the BIOS will draw" class="thumb" />
        <div class="info">
          <div class="meter"><span :style="{ width: fill + '%' }" :class="{ over: fill > 90 }" /></div>
          <span class="note" :class="{ warn: result?.error || (result?.gw && result.gw < d.width) }">{{ status }}</span>
          <span v-if="darkened" class="note warn">Black is the background, so very dark colours are lifted a little to stay in the logo.</span>
          <span v-if="error" class="note warn">{{ error }}</span>
          <span class="note hint">One finger or the mouse draws. Pinch or Ctrl + wheel zooms; two fingers, the wheel, Space + drag or the middle button move.</span>
        </div>
      </footer>
    </div>
  </Teleport>
</template>

<style scoped>
.draw {
  position: fixed;
  inset: 0;
  z-index: 100;
  background: var(--bg);
  display: flex;
  flex-direction: column;
  padding-bottom: env(safe-area-inset-bottom);
}
.bar { display: flex; flex-wrap: wrap; gap: 8px 12px; align-items: center; padding: 8px 12px; border-bottom: 1px solid var(--line); background: var(--panel); }
header.bar { justify-content: space-between; padding-top: max(8px, env(safe-area-inset-top)); }
.title { font-size: 0.95em; }
.seg { display: inline-flex; gap: 4px; flex-wrap: wrap; }
.seg button.on { border-color: var(--accent); color: var(--accent); }
.tl { margin-left: 2px; }
.inline { display: inline-flex; align-items: center; gap: 6px; }
.inline input[type='range'] { width: 110px; accent-color: var(--accent); padding: 0; }
.num { width: 2ch; text-align: right; color: var(--text); }
.zoom button.mono { min-width: 58px; justify-content: center; }
.palette { gap: 6px; }
.swatch {
  width: 28px; height: 28px; padding: 0; border-radius: 6px; border: 2px solid var(--line);
  display: inline-flex; align-items: center; justify-content: center; flex: none;
}
.swatch.on { border-color: var(--text); box-shadow: 0 0 0 2px var(--accent); }
.swatch.add { background: var(--panel-2); color: var(--muted); font-size: 1.1em; }
.edit { position: relative; display: inline-flex; flex-direction: row; align-items: center; gap: 6px; margin-left: 6px; cursor: pointer; }
.edit .big { width: 22px; height: 22px; border-radius: 50%; border: 2px solid var(--line); }
.edit input { position: absolute; inset: 0; opacity: 0; cursor: pointer; width: 100%; height: 100%; padding: 0; }
.viewport { flex: 1; min-height: 0; position: relative; overflow: hidden; background: repeating-conic-gradient(#14181c 0 25%, #101316 0 50%) 0 0 / 24px 24px; }
.viewport canvas { position: absolute; inset: 0; width: 100%; height: 100%; touch-action: none; display: block; cursor: crosshair; }
.t-pan canvas { cursor: grab; }
.t-pick canvas, .t-fill canvas { cursor: cell; }
.foot { border-bottom: 0; border-top: 1px solid var(--line); align-items: flex-start; flex-wrap: nowrap; }
.thumb { height: 56px; max-width: 128px; object-fit: contain; image-rendering: pixelated; background: #000; border-radius: 4px; border: 1px solid var(--line); flex: none; }
.info { display: grid; gap: 3px; min-width: 0; flex: 1; }
.info .note { margin: 0; }
.meter { height: 5px; border-radius: 3px; background: var(--line); overflow: hidden; max-width: 320px; }
.meter span { display: block; height: 100%; background: var(--accent); }
.meter span.over { background: var(--warn); }
@media (max-width: 640px) {
  .tl { display: none; }
  .hint { display: none; }
  .bar { padding: 6px 10px; }
  .inline input[type='range'] { width: 90px; }
}
@media (hover: none) {
  .hint { display: none; }
}
</style>
