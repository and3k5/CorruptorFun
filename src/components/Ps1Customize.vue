<script setup>
import { computed, ref, shallowRef, watch } from 'vue';
import { defaultCustom } from '../lib/ps1custom/index.js';
import { findWordmarks, availableLetters } from '../lib/ps1custom/wordmark.js';
import { findVab, SAMPLE_ROLES } from '../lib/ps1custom/sound.js';
import { imageToLogo, MAX_BYTES, MAX_TRIANGLES } from '../lib/ps1custom/logo.js';
import { DEFAULT_LICENSE, licenseText } from '../lib/ps1custom/license.js';
import { decodeAudio, decodeImage, pixelsToUrl, playPcm, recordAudio } from '../lib/ps1custom/browser.js';

// The boot customization form. Works on a plain object (see
// ps1custom/index.js) and emits a new one on every change.
const props = defineProps({
  bios: { type: Object, required: true }, // the untouched BIOS
  region: { type: String, default: 'A' },
  modelValue: { type: Object, required: true },
  notes: { type: Array, default: () => [] },
  used: { type: Object, default: () => ({}) }, // per sample: rate, seconds, cut
  testDisc: { type: Boolean, default: false },
});
const emit = defineEmits(['update:modelValue']);

const c = computed(() => props.modelValue);
const update = (patch) => emit('update:modelValue', { ...c.value, ...patch });
const license = computed(() => c.value.license || DEFAULT_LICENSE[props.region] || DEFAULT_LICENSE.A);

// What the user's BIOS has, so we can say which letters are original.
const marks = computed(() => findWordmarks(props.bios));
const letters = computed(() => {
  const m = marks.value;
  const has = (k) => (m[k]?.[0] ? availableLetters(m[k][0]) : null);
  return { sony: has('sony'), sce: has('sce'), playstation: has('playstation') };
});
const vab = computed(() => findVab(props.bios));

// Text fields edit a draft, applied when the field is left (or on Enter), so
// the BIOS isn't rebuilt and rebooted on every key.
const draft = ref({});
watch(
  () => [c.value.texts, c.value.license, props.region],
  () => {
    draft.value = { ...c.value.texts, license1: license.value[0], license2: license.value[1] };
  },
  { immediate: true },
);

function setText(key) {
  if (draft.value[key] === c.value.texts[key]) return;
  update({ texts: { ...c.value.texts, [key]: draft.value[key] } });
}

// License text (test disc).
const licenseWarnings = computed(() => (c.value.license ? licenseText(...c.value.license).warnings : []));
function setLicense() {
  const next = [draft.value.license1, draft.value.license2];
  if (next[0] === license.value[0] && next[1] === license.value[1]) return;
  update({ license: next });
}

// Logo.
const logoError = ref('');
const logoBusy = ref(false);
const logoOpts = computed(() => c.value.logo?.opts || { colours: 16, depth: false, detail: 0 });
const logoPreview = computed(() => (c.value.logo ? pixelsToUrl(c.value.logo.preview, Math.max(1, Math.floor(240 / c.value.logo.preview.width))) : ''));

function buildLogo(source, name, opts) {
  logoError.value = '';
  try {
    const r = imageToLogo(source, { colours: opts.colours, depth: opts.depth, detail: opts.detail || undefined });
    update({
      logo: {
        name,
        source,
        opts,
        tmd: r.tmd,
        triangles: r.triangles,
        bytes: r.bytes,
        preview: { width: r.gw, height: r.gh, data: new Uint8Array(r.preview.buffer) },
      },
    });
  } catch (e) {
    logoError.value = e.message;
  }
}

async function onLogo(ev) {
  const file = ev.target.files?.[0];
  ev.target.value = '';
  if (!file) return;
  logoBusy.value = true;
  try {
    const src = await decodeImage(file);
    buildLogo(src, file.name, { ...logoOpts.value, detail: 0 });
  } catch {
    logoError.value = "Couldn't read that picture. PNG, JPEG, GIF, WebP and SVG all work.";
  } finally {
    logoBusy.value = false;
  }
}

function setLogoOpt(key, value) {
  if (!c.value.logo) return;
  buildLogo(c.value.logo.source, c.value.logo.name, { ...logoOpts.value, [key]: value });
}

// Sounds.
const soundError = ref('');
const soundBusy = ref(-1);
// Recording from the microphone: which sample, how long so far, and how to stop.
const rec = ref({ i: -1, seconds: 0, stop: null });
const canRecord = typeof window !== 'undefined' && !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== 'undefined';

async function toggleRecord(i) {
  if (rec.value.i === i) {
    rec.value.stop?.();
    return;
  }
  if (rec.value.i >= 0) return;
  soundError.value = '';
  try {
    const r = await recordAudio({ onTick: (s) => (rec.value = { ...rec.value, seconds: s }) });
    rec.value = { i, seconds: 0, stop: r.stop };
    const blob = await r.done;
    rec.value = { i: -1, seconds: 0, stop: null };
    await useSound(i, blob, `Recording ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`);
  } catch (e) {
    rec.value = { i: -1, seconds: 0, stop: null };
    soundError.value =
      e?.name === 'NotAllowedError' ? 'The browser wasn\'t allowed to use the microphone.' : `Couldn't record: ${e?.message || e}`;
  }
}

async function onSound(i, ev) {
  const file = ev.target.files?.[0];
  ev.target.value = '';
  if (!file) return;
  await useSound(i, file, file.name);
}

async function useSound(i, file, name) {
  soundError.value = '';
  soundBusy.value = i;
  try {
    const { pcm, rate, seconds } = await decodeAudio(file);
    const sounds = [...c.value.sounds];
    if (!pcm.length) throw new Error('silent');
    sounds[i] = { name, pcm, rate, seconds, loop: false };
    update({ sounds });
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
  const u = props.used[i + 1];
  if (!u) return '';
  const khz = `${{ 44100: '44.1', 22050: '22', 11025: '11' }[u.rate] || u.rate / 1000} kHz`;
  return u.cut ? `Plays the first ${u.seconds.toFixed(2)} s of ${u.of.toFixed(2)} s, at ${khz}.` : `Plays all ${u.seconds.toFixed(2)} s, at ${khz}.`;
}

function resetAll() {
  update(defaultCustom());
  logoError.value = '';
  soundError.value = '';
}
</script>

<template>
  <section class="panel custom">
    <h3>Customize the boot</h3>
    <p class="note">
      Changes go into a copy of your BIOS (and the test disc), so corruptions start from your customized boot. Nothing is
      uploaded.
    </p>

    <details open>
      <summary>Sony intro text</summary>
      <div class="form">
        <label>SONY <input v-model="draft.sony" maxlength="12" @keydown.enter="$event.target.blur()" @change="setText('sony')" /></label>
        <label>COMPUTER <input v-model="draft.computer" maxlength="20" @keydown.enter="$event.target.blur()" @change="setText('computer')" /></label>
        <label>ENTERTAINMENT <input v-model="draft.entertainment" maxlength="24" @keydown.enter="$event.target.blur()" @change="setText('entertainment')" /></label>
        <p class="note">
          These are pictures in your BIOS. New text is put together from the letters in them
          <template v-if="letters.sony && letters.sce">
            (<span class="mono">{{ letters.sony[0] }}</span> in SONY, <span class="mono">{{ letters.sce[0] }}</span> and
            <span class="mono">{{ letters.sce[1] }}</span> below)</template
          >, so it's the real font. A letter from the other line is resized to fit; any other letter is drawn with a similar
          font, which won't match exactly. Long words are squeezed to fit the picture. Capitals only.
        </p>
        <p v-if="!letters.sony || !letters.sce" class="note warn">Couldn't find these pictures in this BIOS, so they can't be changed.</p>
      </div>
    </details>

    <details open>
      <summary>Licensed screen</summary>
      <div class="form">
        <label>PlayStation <input v-model="draft.playstation" maxlength="20" @keydown.enter="$event.target.blur()" @change="setText('playstation')" /></label>
        <p class="note">
          Also a picture in your BIOS<template v-if="letters.playstation">, with the letters <span class="mono">{{ letters.playstation[0] }}</span></template>.
          The ™ stays.
        </p>

        <template v-if="testDisc">
          <label>License text, line 1 <input v-model="draft.license1" maxlength="40" @keydown.enter="$event.target.blur()" @change="setLicense" /></label>
          <label>Line 2 <input v-model="draft.license2" maxlength="48" @keydown.enter="$event.target.blur()" @change="setLicense" /></label>
          <p v-for="w in licenseWarnings" :key="w" class="note warn">{{ w }}</p>
          <p class="note">
            This text is on the disc, and the BIOS draws it with its own font, so any letters work (\ shows as ¥). The BIOS
            draws it in 16-letter pieces at fixed spots, so about 30 letters fit per line.
          </p>

          <div class="logo">
            <span class="label">Logo above "PlayStation"</span>
            <div class="row">
              <label class="button small file">
                {{ logoBusy ? 'Reading…' : c.logo ? 'Change picture' : 'Use a picture' }}
                <input type="file" accept="image/*" @change="onLogo" />
              </label>
              <button v-if="c.logo" class="small" @click="update({ logo: null })">Back to the CF logo</button>
            </div>
            <template v-if="c.logo">
              <img :src="logoPreview" alt="How the logo will look" class="preview" />
              <div class="two">
                <label>
                  Colours
                  <select :value="logoOpts.colours" @change="setLogoOpt('colours', +$event.target.value)">
                    <option v-for="n in [2, 4, 8, 16, 32]" :key="n" :value="n">{{ n }}</option>
                  </select>
                </label>
                <label>
                  Detail
                  <select :value="logoOpts.detail" @change="setLogoOpt('detail', +$event.target.value)">
                    <option :value="0">Most that fits</option>
                    <option v-for="n in [8, 12, 16, 24, 32, 48, 64]" :key="n" :value="n">{{ n }} squares wide</option>
                  </select>
                </label>
              </div>
              <label class="check"><input type="checkbox" :checked="logoOpts.depth" @change="setLogoOpt('depth', $event.target.checked)" /> 3D sides</label>
              <p class="note">
                {{ c.logo.preview.width }}×{{ c.logo.preview.height }} squares, {{ c.logo.triangles }} of {{ MAX_TRIANGLES }} triangles,
                {{ (c.logo.bytes / 1024).toFixed(1) }} of {{ MAX_BYTES / 1024 }} KB.
              </p>
            </template>
            <p v-if="logoError" class="note warn">{{ logoError }}</p>
            <p class="note">
              The BIOS draws the logo as a 3D model, made only of flat-coloured triangles: about 590 at most (more overruns
              its memory), in 14 KB. So your picture becomes pixel art. Squares of the same colour are merged, so logos and
              pixel art with a few flat colours keep the most detail; photos get blocky. Black and transparent parts are left
              out.
            </p>
          </div>
        </template>
        <p v-else class="note">The license text and the logo come from the disc: put the test disc in to change them.</p>
      </div>
    </details>

    <details open>
      <summary>Boot sounds</summary>
      <div v-if="vab" class="form">
        <div v-for="(s, i) in vab.samples" :key="i" class="sound">
          <span class="label">Sample {{ i + 1 }}: {{ SAMPLE_ROLES[i] || 'other' }}</span>
          <div class="row">
            <label class="button small file">
              {{ soundBusy === i ? 'Decoding…' : c.sounds[i] ? 'Change' : 'Replace' }}
              <input type="file" accept="audio/*,video/*,.wav,.mp3,.ogg,.oga,.flac,.m4a,.aac,.opus,.webm" @change="onSound(i, $event)" />
            </label>
            <button
              v-if="canRecord"
              class="small"
              :class="{ recording: rec.i === i }"
              :disabled="rec.i >= 0 && rec.i !== i"
              @click="toggleRecord(i)"
            >
              {{ rec.i === i ? `■ Stop ${rec.seconds.toFixed(1)} s` : '● Record' }}
            </button>
            <template v-if="c.sounds[i] && rec.i !== i">
              <span class="mono note name" :title="c.sounds[i].name">{{ c.sounds[i].name }}</span>
              <button class="small" aria-label="Play" @click="playPcm(c.sounds[i].pcm, c.sounds[i].rate)">▶</button>
              <button class="small danger" aria-label="Back to the original sample" @click="setSound(i, null)">✕</button>
            </template>
          </div>
          <template v-if="c.sounds[i]">
            <label class="check"><input type="checkbox" :checked="c.sounds[i].loop" @change="setSound(i, { ...c.sounds[i], loop: $event.target.checked })" /> Loop while the note is held</label>
            <p class="note">{{ usedText(i) }}</p>
          </template>
        </div>
        <p v-if="soundError" class="note warn">{{ soundError }}</p>
        <p class="note">
          Any format your browser can play, or record one with your microphone (up to 7 s; silence at the start is
          trimmed). The BIOS plays your sound as an instrument: at the pitches and with the fade of
          its own jingle. All replaced samples share the room the originals took (about 42 KB, 1.7 s at 44.1 kHz), so longer
          sounds are stored at 22 or 11 kHz, and cut if they still don't fit.
        </p>
      </div>
      <p v-else class="note warn">Couldn't find the sound bank in this BIOS.</p>
    </details>

    <p v-for="n in notes" :key="n" class="note">{{ n }}</p>
    <button class="small" @click="resetAll">Reset everything</button>
  </section>
</template>

<style scoped>
.custom details { border-top: 1px solid var(--line); padding-top: 10px; margin-bottom: 10px; }
.custom summary { cursor: pointer; margin-bottom: 8px; font-weight: 600; }
.form { display: grid; gap: 8px; }
.form p { margin: 0; }
.two { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.check { display: flex; gap: 8px; align-items: center; }
.label { font-size: 0.85em; color: var(--text); }
.logo, .sound { display: grid; gap: 6px; }
.sound { padding-bottom: 6px; }
.preview { width: 100%; max-height: 160px; object-fit: contain; image-rendering: pixelated; background: #000; border-radius: 6px; }
.file { position: relative; overflow: hidden; }
.file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
.recording { border-color: var(--accent-2); color: var(--accent-2); }
.name { max-width: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
