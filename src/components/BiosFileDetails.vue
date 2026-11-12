<script setup>
// Details for one stored BIOS file: its hashes and version, plus downloading
// the untouched original again and deleting it from this browser.
import { ref, watch } from 'vue';
import { CONSOLES, ps1Version, ps2Version } from '../lib/consoles.js';
import { loadBios } from '../lib/storage.js';
import { md5Hex } from '../lib/md5.js';

const props = defineProps({ entry: { type: Object, required: true } });
const emit = defineEmits(['delete']);

// MD5 per file id, kept while the page is open.
const md5s = new Map();

const md5 = ref('');
const version = ref('');
const error = ref('');
const confirming = ref(false);
const copied = ref('');
let data = null;

watch(
  () => props.entry.id,
  async (id) => {
    md5.value = md5s.get(id) || '';
    version.value = '';
    error.value = '';
    confirming.value = false;
    data = null;
    try {
      const rec = await loadBios(id);
      if (id !== props.entry.id) return;
      if (!rec) {
        error.value = 'This file is no longer stored.';
        return;
      }
      data = rec.data;
      version.value = (rec.consoleId === 'ps2' ? ps2Version(data) : ps1Version(data)) || '';
      if (!md5.value) {
        // Let the panel paint before hashing a few MB.
        await new Promise((r) => setTimeout(r));
        const h = md5Hex(data);
        md5s.set(id, h);
        if (id === props.entry.id) md5.value = h;
      }
    } catch {
      error.value = "Couldn't read this file from browser storage.";
    }
  },
  { immediate: true },
);

function download() {
  if (!data) return;
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([data], { type: 'application/octet-stream' }));
  a.download = props.entry.name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

async function copy(what, text) {
  try {
    await navigator.clipboard.writeText(text);
    copied.value = what;
    setTimeout(() => copied.value === what && (copied.value = ''), 1500);
  } catch {
    // Clipboard blocked; the text is selectable anyway.
  }
}

const added = (t) => new Date(t).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
</script>

<template>
  <div class="details">
    <dl>
      <dt>Console</dt>
      <dd>{{ CONSOLES[entry.consoleId]?.name || entry.consoleId }}</dd>
      <dt>Size</dt>
      <dd>{{ entry.size.toLocaleString() }} bytes</dd>
      <template v-if="version">
        <dt>Version</dt>
        <dd>{{ version }}</dd>
      </template>
      <dt>Stored</dt>
      <dd>{{ added(entry.addedAt) }}</dd>
      <dt>MD5</dt>
      <dd class="hash">
        <code>{{ md5 || 'computing…' }}</code>
        <button v-if="md5" class="small" @click="copy('md5', md5)">{{ copied === 'md5' ? 'Copied' : 'Copy' }}</button>
      </dd>
      <dt>SHA-1</dt>
      <dd class="hash">
        <code>{{ entry.id }}</code>
        <button class="small" @click="copy('sha1', entry.id)">{{ copied === 'sha1' ? 'Copied' : 'Copy' }}</button>
      </dd>
    </dl>
    <p v-if="error" class="note warn">{{ error }}</p>
    <div class="row">
      <button class="small" :disabled="!data" @click="download">Download original</button>
      <button v-if="!confirming" class="small danger" @click="confirming = true">Delete from this browser</button>
      <template v-else>
        <button class="small danger" @click="emit('delete', entry.id)">Yes, delete</button>
        <button class="small" @click="confirming = false">Keep</button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.details { padding: 8px 6px 10px; display: grid; gap: 8px; font-size: 0.85em; }
dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 10px; margin: 0; }
dt { color: var(--muted); }
dd { margin: 0; min-width: 0; }
.hash { display: flex; gap: 6px; align-items: center; }
.hash code { overflow-wrap: anywhere; user-select: all; }
</style>
