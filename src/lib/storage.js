// Keeps the user's original BIOS files in this browser (IndexedDB) so they can
// run new corruptions without picking the file again. Nothing leaves the device.

const DB_NAME = 'corruptorfun';
const STORE = 'bios';
// Boot customizations (text, logo, sounds), by key.
const CUSTOM = 'custom';

function open() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 2);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE, { keyPath: 'id' });
      if (!db.objectStoreNames.contains(CUSTOM)) db.createObjectStore(CUSTOM);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tx(mode, fn, store = STORE) {
  const db = await open();
  try {
    return await new Promise((resolve, reject) => {
      const t = db.transaction(store, mode);
      const result = fn(t.objectStore(store));
      t.oncomplete = () => resolve(result && 'result' in result ? result.result : undefined);
      t.onerror = () => reject(t.error);
      t.onabort = () => reject(t.error);
    });
  } finally {
    db.close();
  }
}

export async function sha1Hex(bytes) {
  const digest = await crypto.subtle.digest('SHA-1', bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

// Stored record: { id (sha1), consoleId, name, size, addedAt, data: ArrayBuffer }
export async function saveBios(consoleId, name, bytes) {
  const id = await sha1Hex(bytes);
  const record = { id, consoleId, name, size: bytes.length, addedAt: Date.now(), data: bytes.slice().buffer };
  await tx('readwrite', (s) => s.put(record));
  return { id, consoleId, name, size: record.size, addedAt: record.addedAt };
}

export async function listBios() {
  const all = (await tx('readonly', (s) => s.getAll())) || [];
  // Don't hand the (possibly huge) buffers around just for a list.
  return all
    .map(({ id, consoleId, name, size, addedAt }) => ({ id, consoleId, name, size, addedAt }))
    .sort((a, b) => b.addedAt - a.addedAt);
}

export async function loadBios(id) {
  const rec = await tx('readonly', (s) => s.get(id));
  return rec ? { ...rec, data: new Uint8Array(rec.data) } : null;
}

export async function deleteBios(id) {
  await tx('readwrite', (s) => s.delete(id));
}

export async function saveCustom(key, value) {
  await tx('readwrite', (s) => s.put(value, key), CUSTOM);
}

export async function loadCustom(key) {
  return (await tx('readonly', (s) => s.get(key), CUSTOM)) ?? null;
}
