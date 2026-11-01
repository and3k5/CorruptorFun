<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, shallowRef, watch } from 'vue';
import { CONSOLES, CONSOLE_LIST, detectConsole, ps1Region } from '../lib/consoles.js';
import { DEFAULT_SETTINGS, ENGINES, clampRange, corrupt } from '../lib/corrupt.js';
import { deleteBios, listBios, loadBios, saveBios } from '../lib/storage.js';
import { loadDisc } from '../lib/disc.js';
import { buildTestDisc } from '../lib/testdisc.js';
import testProgramUrl from '../assets/testdisc.exe?url';
import { accepts, easySearch, loadCache, pickCached, saveCache } from '../lib/easy.js';
import wasmUrl from '../wasm/ps1.wasm?url';
import ByteMap from '../components/ByteMap.vue';
import Ps1Screen from '../components/Ps1Screen.vue';

const PREFS_KEY = 'corruptorfun.bios.prefs';

function readPrefs() {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY)) || {};
  } catch {
    return {};
  }
}
const prefs = readPrefs();

const consoleId = ref(CONSOLES[prefs.consoleId] ? prefs.consoleId : 'ps1');
const settings = reactive({ ...DEFAULT_SETTINGS, ...(prefs.settings?.[consoleId.value] || {}) });

const library = ref([]);
const current = shallowRef(null); // { id, name, consoleId, data }
const result = shallowRef(null); // { data, hits }
const message = ref('');
const busy = ref(false);
const storageOk = ref(true);
const disc = shallowRef(null);
const discLabel = ref('');
const discIsTest = ref(false);
const discError = ref('');
const screen = ref(null);
const mini = ref(!!prefs.mini);
const advancedOpen = ref(prefs.advancedOpen ?? true);
const easy = reactive({ running: false, text: '', abort: null, saved: 0 });
const easyFilters = reactive({ picture: prefs.easyFilters?.picture ?? true, sound: prefs.easyFilters?.sound ?? false });
// Probe results for the current BIOS, reused across clicks and visits. The
// key includes the core build, since a different core boots differently.
const coreTag = wasmUrl.split('/').pop();
let easyCache = null;
let easyCacheKey = '';

const cons = computed(() => CONSOLES[consoleId.value]);
const forConsole = computed(() => library.value.filter((b) => b.consoleId === consoleId.value));
const range = computed(() => (current.value ? clampRange(settings, current.value.data.length) : { start: 0, end: 0 }));
const region = computed(() => (current.value && consoleId.value === 'ps1' ? ps1Region(current.value.data) : 'E'));
const bootBytes = computed(() => result.value?.data || current.value?.data);

function savePrefs() {
  const all = readPrefs();
  all.consoleId = consoleId.value;
  all.settings = { ...(all.settings || {}), [consoleId.value]: { ...settings } };
  all.mini = mini.value;
  all.advancedOpen = advancedOpen.value;
  all.easyFilters = { ...easyFilters };
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(all));
  } catch {
    // Preferences are a convenience only.
  }
}
watch(settings, savePrefs, { deep: true });
watch([mini, advancedOpen], savePrefs);
watch(easyFilters, () => {
  savePrefs();
  countSaved();
});

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
    result.value = null;
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
      message.value = `That looks like a ${CONSOLES[guess].name} BIOS, so I switched to ${CONSOLES[guess].name}.`;
      await selectConsole(guess);
    } else if (!cons.value.sizes.includes(data.length)) {
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
    result.value = null;
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

function bootNow() {
  // Wait for the screen to receive the new bytes before rebooting it.
  nextTick(() => screen.value?.boot());
}

function runCorruption() {
  if (!current.value) return;
  result.value = corrupt(current.value.data, settings);
  bootNow();
}

function reroll() {
  settings.seed = Math.floor(Math.random() * 1e9);
  runCorruption();
}

const hex4 = (n) => '0x' + n.toString(16).toUpperCase().padStart(5, '0');

function easyCacheNow() {
  const key = `${current.value.id || current.value.name}:${region.value}:${coreTag}`;
  if (key !== easyCacheKey) {
    easyCache = loadCache(key);
    easyCacheKey = key;
  }
  return easyCache;
}

function countSaved() {
  const c = current.value && consoleId.value === 'ps1' ? easyCacheNow() : null;
  easy.saved = c?.refMetrics ? c.candidates.filter((x) => !x.used && accepts(x.metrics, c.refMetrics, easyFilters)).length : 0;
}
watch(current, countSaved);

function describe(c) {
  const what = `${ENGINES[c.settings.engine].label} × ${c.settings.count} in ${hex4(c.settings.start)}–${hex4(c.settings.end)}`;
  const m = c.metrics;
  const shows = [m.visual >= 0.15 && 'picture', m.sound >= 0.1 && 'sound'].filter(Boolean).join(' and ');
  return `${what}${shows ? ` (${shows})` : ''}`;
}

function useCandidate(c) {
  c.used = true;
  saveCache(easyCacheKey, easyCache);
  countSaved();
  Object.assign(settings, c.settings);
  result.value = corrupt(current.value.data, c.settings);
  bootNow();
}

async function runEasy() {
  if (!current.value || easy.running) return;
  const cache = easyCacheNow();
  const filters = { ...easyFilters };
  const cached = pickCached(cache, filters);
  if (cached) {
    useCandidate(cached);
    easy.text = `Picked ${describe(cached)} from earlier searches. Its settings are in Advanced below.`;
    return;
  }

  const ctl = new AbortController();
  Object.assign(easy, { running: true, abort: ctl, text: cache.ref ? 'Searching…' : 'Booting the clean BIOS once to learn what it looks like…' });
  screen.value?.stop?.();
  try {
    const tried = await easySearch({
      bios: current.value.data,
      consoleId: consoleId.value,
      region: region.value,
      wasmUrl: new URL(wasmUrl, location.href).href,
      makeWorker: () => new Worker(new URL('../emu/ps1-probe.worker.js', import.meta.url), { type: 'module' }),
      cache,
      accept: (m) => accepts(m, cache.refMetrics, filters),
      signal: ctl.signal,
      onProgress: (p) => {
        if (p.phase === 'search') easy.text = `Tried ${p.tried} of up to ${p.maxTries} corruptions, ${p.found} fit…`;
      },
    });
    saveCache(easyCacheKey, cache);
    if (ctl.signal.aborted) {
      easy.text = 'Stopped. What was tested so far is saved.';
      return;
    }
    const pick = pickCached(cache, filters);
    if (pick) {
      useCandidate(pick);
      easy.text = `Picked ${describe(pick)} after ${tried} tries. Its settings are in Advanced below.`;
      return;
    }
    // Nothing fits the filters: fall back to the liveliest one not used yet.
    const rest = cache.candidates.filter((c) => !c.used).sort((a, b) => b.metrics.score - a.metrics.score);
    if (!rest.length) {
      easy.text = 'Could not test any corruption.';
      return;
    }
    useCandidate(rest[0]);
    easy.text = `None of ${tried} tries fit the boxes ticked above, so here's the liveliest: ${describe(rest[0])}. Try again for another batch.`;
  } catch (e) {
    easy.text = `Easy corrupt failed: ${e.message}`;
  } finally {
    easy.running = false;
    easy.abort = null;
    countSaved();
  }
}

function stopEasy() {
  easy.abort?.abort();
}

onBeforeUnmount(stopEasy);

function cleanBoot() {
  result.value = null;
  bootNow();
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
  const files = ev.target.files;
  discError.value = '';
  if (!files?.length) return;
  try {
    const d = await loadDisc(files);
    disc.value = d.data;
    discLabel.value = d.label;
    discIsTest.value = false;
  } catch (e) {
    discError.value = e.message;
  } finally {
    ev.target.value = '';
  }
}

async function useTestDisc() {
  discError.value = '';
  try {
    const res = await fetch(testProgramUrl);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    disc.value = buildTestDisc(region.value, new Uint8Array(await res.arrayBuffer()));
    discLabel.value = 'Built-in test disc (homemade, with a little demo instead of a game)';
    discIsTest.value = true;
  } catch (e) {
    discError.value = `Could not load the test disc: ${e.message}`;
  }
}

function clearDisc() {
  disc.value = null;
  discLabel.value = '';
  discIsTest.value = false;
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
    Load your own BIOS dump, corrupt it, and boot it in our own emulator, running right here in your browser. BIOS files are
    kept in this browser so you can keep corrupting the untouched original.
  </p>

  <nav class="tabs" aria-label="Console">
    <button v-for="c in CONSOLE_LIST" :key="c.id" :class="{ active: c.id === consoleId }" @click="selectConsole(c.id)">
      {{ c.name }}
    </button>
  </nav>

  <div class="layout">
    <section class="stage panel">
      <h2>{{ cons.name }} boot</h2>

      <div v-if="!current" class="empty">
        <p>Load a {{ cons.name }} BIOS to start.</p>
        <p class="note">{{ cons.fileHint }}</p>
        <label class="button primary file">
          Choose BIOS file
          <input type="file" @change="onFile" />
        </label>
      </div>

      <template v-else-if="consoleId === 'ps1'">
        <Ps1Screen ref="screen" :bios="bootBytes" :disc="disc" :run-game="discIsTest" :region="region" :mini="mini" />
        <div class="disc row">
          <span class="note">Disc:</span>
          <span v-if="disc" class="mono note">{{ discLabel }}</span>
          <span v-else class="note">none, so the BIOS opens its memory card / CD player menu. For the licensed screen, add a game image or use the built-in test disc.</span>
          <label class="button small file">
            {{ disc ? 'Change' : 'Use my disc image' }}
            <input type="file" multiple accept=".bin,.cue,.img,.iso" @change="onDisc" />
          </label>
          <button v-if="!disc" class="small" @click="useTestDisc">Use test disc</button>
          <button v-if="disc" class="small" @click="clearDisc">Remove</button>
        </div>
        <p v-if="discError" class="note warn">{{ discError }}</p>
        <label class="check note">
          <input v-model="mini" type="checkbox" />
          Keep a mini screen in the corner while I scroll down to the options
        </label>
        <p class="note">
          Pick the .cue and its .bin together, or a single .bin. The image is read locally and never stored. Emulation stops
          when the BIOS hands over to the game.
        </p>
      </template>

      <div v-else class="empty">
        <p>The PlayStation 2 core is still being built.</p>
        <p class="note">You can already store and corrupt PS2 BIOS files here, and download the result.</p>
      </div>

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
        <div v-if="consoleId === 'ps1'" class="easy">
          <div class="row">
            <button class="primary big" :disabled="!current || busy || easy.running" @click="runEasy">
              {{ easy.running ? 'Searching…' : 'Easy corrupt' }}
            </button>
            <button v-if="easy.running" @click="stopEasy">Stop</button>
          </div>
          <div class="row filters">
            <label class="check note"><input v-model="easyFilters.picture" type="checkbox" /> No black screen</label>
            <label class="check note"><input v-model="easyFilters.sound" type="checkbox" /> Must have sound</label>
          </div>
          <p class="note">
            {{ easy.text || 'Tries a batch of corruptions in the background and boots one that still shows or plays something, and looks different from the clean boot.' }}
            <template v-if="easy.saved && !easy.running"> {{ easy.saved }} more found {{ easy.saved === 1 ? 'is' : 'are' }} saved, so the next {{ easy.saved === 1 ? 'click is' : 'clicks are' }} instant.</template>
          </p>
        </div>
        <details :open="advancedOpen" @toggle="advancedOpen = $event.target.open">
          <summary>Advanced</summary>
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
          </div>
        </details>
        <div class="row actions">
          <button :disabled="!current" @click="cleanBoot">Clean boot</button>
          <button :disabled="!result" @click="download">Download corrupted BIOS</button>
        </div>
        <p class="note">Every corruption starts from your untouched original.</p>
      </section>
    </aside>
  </div>
</template>

<style scoped>
.lead { max-width: 70ch; }
.tabs { display: flex; flex-wrap: wrap; gap: 6px; margin: 16px 0; }
.tabs button.active { border-color: var(--accent); color: var(--accent); }
.layout { display: grid; grid-template-columns: minmax(0, 1fr) 340px; gap: 16px; align-items: start; }
@media (max-width: 900px) { .layout { grid-template-columns: minmax(0, 1fr); } }
.stage { display: grid; gap: 12px; }
.stage h2 { margin: 0; }
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
.msg { margin: 8px 0 0; }
.disc { gap: 8px; }
.check { display: flex; gap: 8px; align-items: center; }
.easy { display: grid; gap: 6px; margin-bottom: 12px; }
.filters { gap: 14px; }
.easy .big { font-size: 1.05rem; padding: 10px 18px; }
details { margin-bottom: 12px; border-top: 1px solid var(--line, #333); padding-top: 10px; }
summary { cursor: pointer; margin-bottom: 10px; font-weight: 600; }
</style>
