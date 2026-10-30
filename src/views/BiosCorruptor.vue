<script setup>
import { computed, nextTick, onMounted, reactive, ref, shallowRef, watch } from 'vue';
import { CONSOLES, CONSOLE_LIST, detectConsole, locateStrings } from '../lib/consoles.js';
import { DEFAULT_SETTINGS, ENGINES, clampRange, corrupt } from '../lib/corrupt.js';
import { BiosFeed } from '../lib/feed.js';
import { deleteBios, listBios, loadBios, saveBios } from '../lib/storage.js';
import { makeZip } from '../lib/zip.js';
import { SCENES } from '../scenes/index.js';
import SceneCanvas from '../components/SceneCanvas.vue';
import ByteMap from '../components/ByteMap.vue';
import Ps1Emulator from '../components/Ps1Emulator.vue';

const PREFS_KEY = 'corruptorfun.bios.prefs';

function readPrefs() {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY)) || {};
  } catch {
    return {};
  }
}
const prefs = readPrefs();

const consoleId = ref(prefs.consoleId || 'ps1');
const settings = reactive({ ...DEFAULT_SETTINGS, ...(prefs.settings?.[consoleId.value] || {}) });
const licensed = ref(prefs.licensed ?? true);
const sound = ref(prefs.sound ?? false);
const view = ref(prefs.view || 'sim'); // 'sim' | 'emu'

const library = ref([]);
const current = shallowRef(null); // { id, name, consoleId, data }
const result = shallowRef(null); // { data, hits }
const bootKey = ref(0);
const message = ref('');
const busy = ref(false);
const storageOk = ref(true);
const disc = shallowRef(null);
const discLabel = ref('');
const emu = ref(null);

const cons = computed(() => CONSOLES[consoleId.value]);
const scene = computed(() => SCENES[consoleId.value]);
const forConsole = computed(() => library.value.filter((b) => b.consoleId === consoleId.value));
const strings = computed(() => (current.value ? locateStrings(consoleId.value, current.value.data) : {}));
const feed = computed(() => {
  if (!current.value) return null;
  return new BiosFeed(current.value.data, result.value?.data || current.value.data, strings.value);
});
const range = computed(() => (current.value ? clampRange(settings, current.value.data.length) : { start: 0, end: 0 }));

function savePrefs() {
  const all = readPrefs();
  all.consoleId = consoleId.value;
  all.licensed = licensed.value;
  all.sound = sound.value;
  all.view = view.value;
  all.settings = { ...(all.settings || {}), [consoleId.value]: { ...settings } };
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(all));
  } catch {
    // Preferences are a convenience only.
  }
}
watch([settings, licensed, sound, view], savePrefs, { deep: true });

async function refreshLibrary() {
  try {
    library.value = await listBios();
  } catch {
    storageOk.value = false;
  }
}

async function selectConsole(id) {
  if (id === consoleId.value) return;
  savePrefs();
  consoleId.value = id;
  Object.assign(settings, DEFAULT_SETTINGS, readPrefs().settings?.[id] || {});
  if (!CONSOLES[id].realEmulation) view.value = 'sim';
  current.value = null;
  result.value = null;
  savePrefs();
  const latest = forConsole.value[0];
  if (latest) await useStored(latest.id);
}

async function useStored(id) {
  busy.value = true;
  try {
    const rec = await loadBios(id);
    if (!rec) return;
    current.value = rec;
    runCorruption();
  } finally {
    busy.value = false;
  }
}

async function onFile(ev) {
  const file = ev.target.files?.[0];
  ev.target.value = '';
  if (!file) return;
  busy.value = true;
  message.value = '';
  try {
    const data = new Uint8Array(await file.arrayBuffer());
    const guess = detectConsole(data);
    if (guess && guess !== consoleId.value) {
      message.value = `That looks like a ${CONSOLES[guess].name} file, so I switched to ${CONSOLES[guess].name}.`;
      await selectConsole(guess);
    } else if (!guess || !cons.value.sizes.includes(data.length)) {
      message.value = `Unusual size for a ${cons.value.name} BIOS (${data.length.toLocaleString()} bytes). Using it anyway.`;
    }
    let stored = { id: null };
    try {
      stored = await saveBios(consoleId.value, file.name, data);
      await refreshLibrary();
    } catch {
      storageOk.value = false;
    }
    current.value = { id: stored.id, consoleId: consoleId.value, name: file.name, data };
    runCorruption();
  } finally {
    busy.value = false;
  }
}

async function remove(id) {
  await deleteBios(id);
  if (current.value?.id === id) {
    current.value = null;
    result.value = null;
  }
  await refreshLibrary();
}

function runCorruption() {
  if (!current.value) return;
  result.value = corrupt(current.value.data, settings);
  bootKey.value++;
  // Wait for the emulator to receive the new bytes before rebooting it.
  if (view.value === 'emu') nextTick(() => emu.value?.boot());
}

function reroll() {
  settings.seed = Math.floor(Math.random() * 1e9);
  runCorruption();
}

function resetToClean() {
  result.value = null;
  bootKey.value++;
}

function applyRegion(r) {
  settings.start = r.start;
  settings.end = r.end;
}

// Accepts decimal or 0x-prefixed hex, blank = end of file.
function parseOffset(v) {
  const s = String(v).trim();
  if (!s) return null;
  const n = s.toLowerCase().startsWith('0x') ? parseInt(s, 16) : parseInt(s, 10);
  return Number.isFinite(n) ? n : null;
}
const hex = (n) => (n == null ? '' : '0x' + n.toString(16).toUpperCase());
const startText = computed({ get: () => hex(settings.start), set: (v) => (settings.start = parseOffset(v) ?? 0) });
const endText = computed({ get: () => hex(settings.end), set: (v) => (settings.end = parseOffset(v)) });

function download() {
  if (!result.value || !current.value) return;
  const base = current.value.name.replace(/\.[^.]+$/, '');
  const ext = current.value.name.match(/\.[^.]+$/)?.[0] || '.bin';
  const blob = new Blob([result.value.data], { type: 'application/octet-stream' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = `${base}.corrupt-${settings.engine}-${settings.seed}${ext}`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

async function onDisc(ev) {
  const files = [...(ev.target.files || [])];
  ev.target.value = '';
  if (!files.length) return;
  if (files.length === 1) {
    disc.value = files[0];
  } else {
    // A .cue with its .bin tracks: hand them over together as one zip.
    const entries = await Promise.all(files.map(async (f) => ({ name: f.name, data: new Uint8Array(await f.arrayBuffer()) })));
    disc.value = makeZip(entries);
  }
  discLabel.value = files.map((f) => f.name).join(', ');
}

function clearDisc() {
  disc.value = null;
  discLabel.value = '';
}

const fmtSize = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(n % 1048576 ? 1 : 0)} MB` : `${Math.round(n / 1024)} KB`);

onMounted(async () => {
  await refreshLibrary();
  const latest = forConsole.value[0];
  if (latest) await useStored(latest.id);
});
</script>

<template>
  <h1>BIOS corruptor</h1>
  <p class="muted lead">
    Load your own BIOS dump, corrupt it, and watch the boot sequence go wrong. Files stay in this browser:
    they are kept in local storage so you can keep corrupting without picking them again.
  </p>

  <nav class="tabs" aria-label="Console">
    <button v-for="c in CONSOLE_LIST" :key="c.id" :class="{ active: c.id === consoleId }" @click="selectConsole(c.id)">
      {{ c.name }}
    </button>
  </nav>

  <div class="layout">
    <section class="stage panel">
      <div class="row stage-head">
        <h2>{{ cons.name }} boot</h2>
        <div v-if="cons.realEmulation" class="seg" role="tablist">
          <button :class="{ active: view === 'sim' }" @click="view = 'sim'">Simulated</button>
          <button :class="{ active: view === 'emu' }" @click="view = 'emu'">Real BIOS (emulated)</button>
        </div>
      </div>

      <template v-if="!current">
        <div class="empty">
          <p>Load a {{ cons.name }} BIOS to start.</p>
          <p class="note">{{ cons.fileHint }}</p>
          <label class="button primary file">
            Choose BIOS file
            <input type="file" @change="onFile" />
          </label>
        </div>
      </template>

      <template v-else-if="view === 'emu' && cons.realEmulation">
        <Ps1Emulator ref="emu" :bios="result?.data || current.data" :disc="disc" />
        <div class="disc row">
          <span class="note">Disc:</span>
          <span v-if="disc" class="mono note">{{ discLabel }}</span>
          <span v-else class="note">none (the BIOS opens its shell). Add your own game image to see the licensed screen.</span>
          <label class="button small file">
            {{ disc ? 'Change' : 'Use my disc image' }}
            <input type="file" multiple accept=".bin,.cue,.img,.pbp,.chd,.iso,.ccd,.m3u" @change="onDisc" />
          </label>
          <button v-if="disc" class="small" @click="clearDisc">Remove</button>
        </div>
        <p class="note">
          Runs the real corrupted BIOS in pcsx_rearmed via EmulatorJS (loaded from jsDelivr on first use). The disc image is read
          locally and is not stored.
        </p>
        <p class="note warn">
          A badly corrupted BIOS can lock up the emulator, and with it this tab. If that happens, reload the page: your BIOS
          and settings are kept.
        </p>
      </template>

      <template v-else-if="feed">
        <SceneCanvas :scene="scene" :feed="feed" :licensed="licensed" :sound="sound" :boot-key="bootKey" />
        <div class="row opts">
          <label v-if="cons.licensed" class="check"><input v-model="licensed" type="checkbox" /> Licensed disc inserted</label>
          <label class="check"><input v-model="sound" type="checkbox" /> Sound</label>
        </div>
        <p class="note">
          Simulated: a re-creation of the boot animation. Every part of it is wired to its own slice of your BIOS, and reads the
          corrupted bytes there, so where you corrupt decides what breaks.
          <template v-if="cons.realEmulation">Switch to "Real BIOS" to run the actual code.</template>
        </p>
      </template>

      <ByteMap
        v-if="current"
        :size="current.data.length"
        :hits="result?.hits || []"
        :start="range.start"
        :end="range.end"
        :original="current.data"
        :corrupted="result?.data"
      />
    </section>

    <aside class="side">
      <section class="panel">
        <h3>Your {{ cons.short }} files</h3>
        <p v-if="!storageOk" class="note warn">This browser blocked local storage, so files won't be remembered.</p>
        <ul v-if="forConsole.length" class="lib">
          <li v-for="b in forConsole" :key="b.id" :class="{ on: current?.id === b.id }">
            <button class="link" :title="b.id" @click="useStored(b.id)">{{ b.name }}</button>
            <span class="note">{{ fmtSize(b.size) }}</span>
            <button class="small danger" :aria-label="`Forget ${b.name}`" @click="remove(b.id)">✕</button>
          </li>
        </ul>
        <p v-else class="note">Nothing stored yet.</p>
        <label class="button small file">
          Add BIOS file
          <input type="file" @change="onFile" />
        </label>
        <p v-if="message" class="note warn msg">{{ message }}</p>
      </section>

      <section class="panel">
        <h3>Corruption</h3>
        <div class="form">
          <label>
            Engine
            <select v-model="settings.engine">
              <option v-for="(e, id) in ENGINES" :key="id" :value="id">{{ e.label }}</option>
            </select>
            <span class="note">{{ ENGINES[settings.engine].help }}</span>
          </label>
          <label v-if="['increment', 'xor', 'smear', 'shift'].includes(settings.engine)">
            {{ settings.engine === 'xor' ? 'Mask' : settings.engine === 'increment' ? 'Amount' : 'Block length' }}
            <input v-model.number="settings.amount" type="number" min="1" max="65535" />
          </label>
          <div class="two">
            <label>
              Hits
              <select v-model="settings.mode">
                <option value="count">Random count</option>
                <option value="every">Every Nth byte</option>
              </select>
            </label>
            <label v-if="settings.mode === 'count'">
              Count
              <input v-model.number="settings.count" type="number" min="0" max="1000000" />
            </label>
            <label v-else>
              Every N
              <input v-model.number="settings.every" type="number" min="1" />
            </label>
          </div>
          <div class="two">
            <label>Start <input v-model.lazy="startText" class="mono" placeholder="0x0" /></label>
            <label>End <input v-model.lazy="endText" class="mono" placeholder="end of file" /></label>
          </div>
          <div class="row regions">
            <button v-for="r in cons.regions" :key="r.label" class="small" @click="applyRegion(r)">{{ r.label }}</button>
          </div>
          <label>Seed <input v-model.number="settings.seed" type="number" /></label>
        </div>
        <div class="row actions">
          <button class="primary" :disabled="!current || busy" @click="runCorruption">Corrupt &amp; boot</button>
          <button :disabled="!current || busy" @click="reroll">New seed</button>
          <button :disabled="!result" @click="resetToClean">Clean boot</button>
          <button :disabled="!result" @click="download">Download corrupted BIOS</button>
        </div>
        <p class="note">Every corruption starts from your untouched original, so you can keep iterating.</p>
      </section>
    </aside>
  </div>

  <section class="panel about">
    <h3>How each console is rendered</h3>
    <ul class="note">
      <li><strong>PlayStation:</strong> the real BIOS runs in an emulator, so corruption changes the actual code. A simulated mode is there too.</li>
      <li>
        <strong>PlayStation 2, Xbox, Xbox 360:</strong> running these boot ROMs needs a full console emulator that doesn't exist
        for the browser in a usable form. The boot is re-created instead and driven by your file's bytes. Download the corrupted
        BIOS to try it for real in PCSX2 or xemu.
      </li>
      <li><strong>Xbox 360:</strong> there's no standalone BIOS; a NAND dump (or any file) is used as the data source.</li>
    </ul>
  </section>
</template>

<style scoped>
.lead { max-width: 70ch; }
.tabs { display: flex; flex-wrap: wrap; gap: 6px; margin: 16px 0; }
.tabs button.active, .seg button.active { border-color: var(--accent); color: var(--accent); }
.layout { display: grid; grid-template-columns: minmax(0, 1fr) 340px; gap: 16px; align-items: start; }
@media (max-width: 900px) { .layout { grid-template-columns: minmax(0, 1fr); } }
.stage { display: grid; gap: 12px; }
.stage-head { justify-content: space-between; }
.stage-head h2 { margin: 0; }
.seg { display: inline-flex; gap: 4px; }
.seg button { padding: 4px 10px; font-size: 0.85em; }
.empty { aspect-ratio: 4 / 3; display: grid; place-content: center; text-align: center; background: #000; border-radius: 8px; padding: 16px; }
.file { position: relative; overflow: hidden; justify-self: center; }
.file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
.side { display: grid; gap: 16px; }
.lib { list-style: none; padding: 0; margin: 0 0 10px; display: grid; gap: 4px; }
.lib li { display: grid; grid-template-columns: 1fr auto auto; gap: 8px; align-items: center; padding: 4px 6px; border-radius: 6px; }
.lib li.on { background: var(--panel-2); }
.link { background: none; border: 0; padding: 0; color: var(--text); text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: block; }
.link:hover { color: var(--accent); }
.form { display: grid; gap: 10px; margin-bottom: 12px; }
.two { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.regions { gap: 4px; }
.actions { margin-bottom: 8px; }
.check { display: inline-flex; flex-direction: row; gap: 6px; align-items: center; color: var(--text); font-size: 0.9em; }
.opts { gap: 16px; }
.msg { margin: 8px 0 0; }
.disc { gap: 8px; }
.about { margin-top: 16px; }
.about ul { margin: 0; padding-left: 18px; }
</style>
