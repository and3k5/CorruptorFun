<script setup>
import { computed, ref, shallowRef, watch } from 'vue';
import { DEFAULT_DISC, DEFAULT_SCE, DISC, SCE, SOUND_RATE, defaultCustom, findSamples, originalDisc, originalSample, originalSce } from '../lib/ps2custom/index.js';
import { discImage, grayToUrl, imageToStrip, textToStrip } from '../lib/ps2custom/browser.js';
import { decodeAudio, decodeImage, playPcm, recordAudio } from '../lib/ps1custom/browser.js';
import { drawingToImage } from '../lib/ps1custom/draw.js';
import LogoDraw from './LogoDraw.vue';

// The PS2 boot customization form: the intro's "Sony Computer
// Entertainment" line (text, a picture or a drawing), the no-disc message,
// and the intro's sound samples. Works on a plain object (see
// ps2custom/index.js) and emits a new one on every change.
const props = defineProps({
  bios: { type: Object, required: true }, // the untouched BIOS
  modelValue: { type: Object, required: true },
  notes: { type: Array, default: () => [] },
  used: { type: Object, default: () => ({}) }, // per sample: seconds kept, cut
});
const emit = defineEmits(['update:modelValue']);
const c = computed(() => props.modelValue);
const update = (patch) => emit('update:modelValue', { ...c.value, ...patch });

// What this BIOS has.
const origSce = computed(() => originalSce(props.bios));
const origDisc = computed(() => originalDisc(props.bios));
const bank = computed(() => findSamples(props.bios));

// Previews: the strip as the intro shows it (half width), and the message.
const scePreview = computed(() => {
  const strip = c.value.sce?.strip || origSce.value;
  return strip ? grayToUrl(strip, SCE.strip, SCE.rows, 0.5, 1.07) : '';
});
const discPreview = computed(() => {
  const img = c.value.disc?.image || origDisc.value.E || Object.values(origDisc.value)[0];
  return img ? grayToUrl(img, DISC.width, DISC.height, 1.5, 1.5) : '';
});

// Text fields edit a draft, applied when the field is left (or on Enter).
const draft = ref({});
watch(
  () => [c.value.sceText, c.value.discText],
  () => (draft.value = { sce: c.value.sceText, disc1: c.value.discText[0], disc2: c.value.discText[1] }),
  { immediate: true },
);

function setSceText() {
  const t = draft.value.sce.trim();
  if (t === c.value.sceText && c.value.sce?.from !== 'picture' && c.value.sce?.from !== 'drawing') return;
  if (!t || t === DEFAULT_SCE) update({ sceText: DEFAULT_SCE, sce: null });
  else update({ sceText: t, sce: { from: 'text', strip: textToStrip(t) } });
}

function setDisc() {
  const lines = [draft.value.disc1.trim(), draft.value.disc2.trim()];
  if (lines[0] === c.value.discText[0] && lines[1] === c.value.discText[1]) return;
  const same = lines[0] === DEFAULT_DISC[0] && lines[1] === DEFAULT_DISC[1];
  update({ discText: lines, disc: same ? null : { image: discImage(lines[0], lines[1]) } });
}

// Picture or drawing for the intro line.
const picError = ref('');
const picBusy = ref(false);
async function onPicture(ev) {
  const file = ev.target.files?.[0];
  ev.target.value = '';
  if (!file) return;
  picBusy.value = true;
  picError.value = '';
  try {
    const img = await decodeImage(file, 1024);
    update({ sce: { from: 'picture', name: file.name, strip: imageToStrip(img) } });
  } catch {
    picError.value = "Couldn't read that picture. PNG, JPEG, GIF, WebP and SVG all work.";
  } finally {
    picBusy.value = false;
  }
}

// Drawing: a grid the shape of the line (it's shown at half width, so a
// grid cell is two texels across).
const DRAW_SIZES = [
  [116, 8],
  [231, 16],
  [462, 32],
];
const drawOpen = ref(false);
const drawKeep = shallowRef(null);
const drawStart = computed(() => c.value.sce?.drawing || drawKeep.value || null);
function closeDraw(d) {
  drawKeep.value = d;
  drawOpen.value = false;
}
function applyDraw(d) {
  const img = drawingToImage(d);
  update({ sce: { from: 'drawing', name: 'Drawing', drawing: d, strip: imageToStrip(img) } });
  drawKeep.value = null;
  drawOpen.value = false;
}

function resetSce() {
  draft.value.sce = DEFAULT_SCE;
  update({ sceText: DEFAULT_SCE, sce: null });
}

// Sounds.
const soundError = ref('');
const soundBusy = ref(-1);
const fmt = (s) => `${s.toFixed(2)} s`;

function playOriginal(i) {
  playPcm(originalSample(bank.value, i), SOUND_RATE);
}

const rec = ref({ i: -1, seconds: 0, sound: null, level: 0, stop: null });
const recIdle = { i: -1, seconds: 0, sound: null, level: 0, stop: null };
const canRecord = typeof window !== 'undefined' && !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== 'undefined';
const room = (i) => bank.value.samples[i].seconds;
function recText(i) {
  const r = rec.value;
  if (r.sound === null) return 'Listening… start the sound when you are ready.';
  const left = room(i) - r.sound;
  return left > 0 ? `${left.toFixed(1)} s left of ${room(i).toFixed(1)} s` : `${(-left).toFixed(1)} s longer than fits`;
}
const recFill = (i) => (rec.value.sound === null ? 0 : Math.min(100, (rec.value.sound / room(i)) * 100));

async function toggleRecord(i) {
  if (rec.value.i === i) {
    rec.value.stop?.();
    return;
  }
  if (rec.value.i >= 0) return;
  soundError.value = '';
  try {
    const r = await recordAudio({ onTick: (t) => (rec.value = { ...rec.value, ...t }), autoStop: () => room(i) });
    rec.value = { ...recIdle, i, stop: r.stop };
    const blob = await r.done;
    rec.value = { ...recIdle };
    await useSound(i, blob, `Recording ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`);
  } catch (e) {
    rec.value = { ...recIdle };
    soundError.value = e?.name === 'NotAllowedError' ? "The browser wasn't allowed to use the microphone." : `Couldn't record: ${e?.message || e}`;
  }
}

async function onSound(i, ev) {
  const file = ev.target.files?.[0];
  ev.target.value = '';
  if (file) await useSound(i, file, file.name);
}

async function useSound(i, file, name) {
  soundError.value = '';
  soundBusy.value = i;
  try {
    const { pcm, rate } = await decodeAudio(file);
    if (!pcm.length) throw new Error('silent');
    setSound(i, { name, pcm, rate, loop: false });
  } catch {
    soundError.value = `Couldn't use ${name}: it's silent, or this browser can't play that format (WAV, MP3, OGG and FLAC work).`;
  } finally {
    soundBusy.value = -1;
  }
}
function setSound(i, value) {
  const sounds = [...c.value.sounds];
  sounds[i] = value;
  update({ sounds });
}
function usedText(i) {
  const u = props.used[i];
  if (!u) return '';
  return u.cut ? `Stored: the first ${fmt(u.seconds)} of ${fmt(u.of)} (no room for more).` : `Stored: all ${fmt(u.seconds)}.`;
}

function resetAll() {
  update(defaultCustom());
  picError.value = '';
  soundError.value = '';
}
</script>

<template>
  <section class="panel custom">
    <h3>Customize the boot</h3>
    <p class="note">Changes go into a copy of your BIOS, so corruptions start from your customized boot. Nothing is uploaded.</p>

    <details open>
      <summary>Intro text</summary>
      <div v-if="origSce" class="form">
        <img :src="scePreview" alt="The intro line as the boot shows it" class="preview strip" />
        <label>
          Text
          <input v-model="draft.sce" maxlength="60" @keydown.enter="$event.target.blur()" @change="setSceText" />
        </label>
        <div class="row">
          <label class="button small file">
            {{ picBusy ? 'Reading…' : 'Use a picture' }}
            <input type="file" accept="image/*" @change="onPicture" />
          </label>
          <button class="small" @click="drawOpen = true">{{ c.sce?.from === 'drawing' ? 'Edit drawing' : 'Draw one' }}</button>
          <button v-if="c.sce" class="small" @click="resetSce">Back to the original</button>
        </div>
        <p v-if="picError" class="note warn">{{ picError }}</p>
        <LogoDraw
          v-if="drawOpen"
          :initial="drawStart"
          :sizes="DRAW_SIZES"
          simple
          title="Draw the intro line"
          @apply="applyDraw"
          @close="closeDraw"
        />
        <p class="note">
          The line under the cubes in the intro is a picture in your BIOS: one long, short strip, glowing white on the
          blue. Your text is drawn into it with a font like Sony's. A picture or drawing is fitted into the strip in
          shades of grey (black shows the background). It fades in and out like the original.
        </p>
      </div>
      <p v-else class="note warn">Couldn't find the intro picture in this BIOS.</p>
    </details>

    <details open>
      <summary>No-disc message</summary>
      <div class="form">
        <img v-if="discPreview" :src="discPreview" alt="The message" class="preview" />
        <label>Line 1 <input v-model="draft.disc1" maxlength="48" @keydown.enter="$event.target.blur()" @change="setDisc" /></label>
        <label>Line 2 <input v-model="draft.disc2" maxlength="48" @keydown.enter="$event.target.blur()" @change="setDisc" /></label>
        <p class="note">
          Shown when the console starts a disc it can't play. The BIOS has one picture of it per language; yours replaces
          them all.
        </p>
      </div>
    </details>

    <details open>
      <summary>Intro sounds</summary>
      <div v-if="bank" class="form">
        <div v-for="(s, i) in bank.samples" :key="i" class="sound">
          <span class="label">Sound {{ i + 1 }}: {{ fmt(s.seconds) }}{{ s.loops ? ', loops' : '' }}</span>
          <div class="row">
            <button class="small" title="The original sample, as stored" @click="playOriginal(i)">▶ Original</button>
            <label class="button small file">
              {{ soundBusy === i ? 'Decoding…' : c.sounds[i] ? 'Change' : 'Replace' }}
              <input type="file" accept="audio/*,video/*,.wav,.mp3,.ogg,.oga,.flac,.m4a,.aac,.opus,.webm" @change="onSound(i, $event)" />
            </label>
            <button v-if="canRecord" class="small" :class="{ recording: rec.i === i }" :disabled="rec.i >= 0 && rec.i !== i" @click="toggleRecord(i)">
              {{ rec.i === i ? '■ Stop' : '● Record' }}
            </button>
          </div>
          <div v-if="rec.i === i" class="recbox" role="status">
            <div class="meter"><span :style="{ width: Math.min(100, rec.level * 140) + '%' }" /></div>
            <div class="progress"><span :style="{ width: recFill(i) + '%' }" /></div>
            <span class="note mono">{{ recText(i) }}</span>
          </div>
          <template v-if="c.sounds[i] && rec.i !== i">
            <div class="row">
              <span class="mono note name" :title="c.sounds[i].name">{{ c.sounds[i].name }}</span>
              <button class="small" @click="playPcm(c.sounds[i].pcm, c.sounds[i].rate)">▶ Yours</button>
              <button class="small danger" aria-label="Back to the original sample" @click="setSound(i, null)">✕</button>
            </div>
            <label class="check"><input type="checkbox" :checked="c.sounds[i].loop" @change="setSound(i, { ...c.sounds[i], loop: $event.target.checked })" /> Loop while the note is held</label>
            <p class="note">{{ usedText(i) }}</p>
          </template>
        </div>
        <p v-if="soundError" class="note warn">{{ soundError }}</p>
        <p class="note">
          The intro's music is a sequence of notes played on these samples, like instruments, so your sound plays at the
          pitches and fades of the original music. Each sound takes the place of the one it replaces and gets the same
          length (shown); longer sounds are cut, and recording stops there. Most of the short ones loop in the original:
          tick Loop for a sound that should keep going while the note is held. Any format your browser can play works.
        </p>
      </div>
      <p v-else class="note warn">Couldn't find the intro sounds in this BIOS.</p>
    </details>

    <p v-for="n in notes" :key="n" class="note warn">{{ n }}</p>
    <button class="small" @click="resetAll">Reset everything</button>
  </section>
</template>

<style scoped>
.custom details { border-top: 1px solid var(--line); padding-top: 10px; margin-bottom: 10px; }
.custom summary { cursor: pointer; margin-bottom: 8px; font-weight: 600; }
.form { display: grid; gap: 8px; }
.form p { margin: 0; }
.check { display: flex; gap: 8px; align-items: center; }
.label { font-size: 0.85em; color: var(--text); }
.sound { display: grid; gap: 6px; padding-bottom: 6px; }
.preview { width: 100%; object-fit: contain; background: #000; border-radius: 6px; }
.preview.strip { image-rendering: auto; }
.file { position: relative; overflow: hidden; }
.file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
.recording { border-color: var(--accent-2); color: var(--accent-2); }
.recbox { display: grid; gap: 4px; }
.meter, .progress { height: 6px; border-radius: 3px; background: var(--line); overflow: hidden; }
.meter { height: 3px; }
.meter span, .progress span { display: block; height: 100%; background: var(--accent); transition: width 50ms linear; }
.name { max-width: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
