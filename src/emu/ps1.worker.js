// Runs an emulator WebAssembly core (PS1 or PS2, whose exports share names
// apart from their ps1_ / ps2_ prefix) off the main thread. A corrupted BIOS can't
// freeze the page this way, and the page can always kill a boot by
// terminating the worker.
//
// Frames are paced against the wall clock: if the device falls behind, the
// worker runs extra frames without building their pictures to catch up, so
// the boot (and its sound) keeps real speed instead of slowing down.
//
// The PS2 core draws in a worker of its own (see ps2-gs.js), a frame behind
// the emulation, so the two run side by side.

import { GsLink } from './ps2-gs.js';

const MAX_CATCH_UP = 4; // frames per tick before we give up and slow down
const MAX_GS_AHEAD = 3; // PS2: frames the emulation may run ahead of the drawing

let wasm = null;
let mem = () => new Uint8Array(wasm.memory.buffer);
let timer = 0;
let running = false;
let audioPort = null;
let nextAt = 0;
let runGame = false; // keep going after the hand-off (test disc)
let handedOver = false;
let core = 'ps1';
let gs = null; // PS2: the drawing worker
let frames = 0; // emulated so far
// The core's export `name`, or a stand-in for one this core doesn't have.
const fn = (name) => wasm[`${core}_${name}`] || (() => 0);

function copyIn(bytes) {
  const ptr = wasm.alloc(bytes.length);
  mem().set(bytes, ptr);
  return ptr;
}

function flushAudio() {
  const n = fn('audio_len')();
  if (!n) return;
  const samples = new Int16Array(wasm.memory.buffer, fn('audio_ptr')(), n).slice();
  fn('audio_clear')();
  if (audioPort) audioPort.postMessage(samples.buffer, [samples.buffer]);
}

function tick() {
  if (!running) return;
  const frameMs = 1000 / (fn('is_pal')() === 1 ? 50 : 60);
  const now = performance.now();
  let due = Math.floor((now - nextAt) / frameMs) + 1;
  if (due > MAX_CATCH_UP) {
    // Too far behind to catch up: accept the slowdown.
    nextAt = now;
    due = MAX_CATCH_UP;
  }
  let waitForGs = false;
  try {
    for (let i = 0; i < due; i++) {
      if (gs && gs.inFlight >= MAX_GS_AHEAD) {
        // The drawing is behind: wait for it (it wakes us).
        waitForGs = true;
        break;
      }
      if (!gs) fn('set_render')(i === due - 1 ? 1 : 0);
      fn('run_frame')();
      frames++;
      flushAudio();
      // A picture whenever the last one asked for is back.
      if (gs) gs.send(!gs.pictureDue);
      nextAt += frameMs;
      if (!runGame && fn('game_started')()) break;
    }
  } catch (e) {
    fail(`The emulator crashed: ${e.message}`);
    return;
  }
  if (!gs) {
    const w = fn('frame_width')();
    const h = fn('frame_height')();
    const pixels = mem().slice(fn('frame_ptr')(), fn('frame_ptr')() + w * h * 4);
    postMessage({ type: 'frame', w, h, frames, pixels: pixels.buffer }, [pixels.buffer]);
  }

  if (fn('game_started')() && !handedOver) {
    handedOver = true;
    postMessage({ type: 'game-started' });
    if (!runGame) {
      running = false;
      return;
    }
  }
  if (waitForGs) {
    gs.below(MAX_GS_AHEAD).then(() => {
      if (running) tick();
    });
    return;
  }
  timer = setTimeout(tick, Math.max(0, nextAt - performance.now()));
}

function fail(message) {
  running = false;
  clearTimeout(timer);
  postMessage({ type: 'error', message });
}

onmessage = async (ev) => {
  const msg = ev.data;
  if (msg.type === 'start') {
    try {
      audioPort = msg.audioPort || null;
      runGame = !!msg.runGame;
      core = msg.core || 'ps1';
      const { instance } = await WebAssembly.instantiateStreaming(fetch(msg.wasmUrl), {});
      wasm = instance.exports;
      const bios = new Uint8Array(msg.bios);
      fn('init')(copyIn(bios), bios.length);
      if (msg.disc) {
        const disc = new Uint8Array(msg.disc);
        fn('set_disc')(copyIn(disc), disc.length);
      }
      fn('set_region')(msg.region.charCodeAt(0));
      if (core === 'ps2') {
        gs = new GsLink(wasm, msg.wasmUrl, fail);
        gs.onFrame = (m) => postMessage({ type: 'frame', w: m.w, h: m.h, frames, pixels: m.pixels }, [m.pixels]);
      }
      running = true;
      nextAt = performance.now();
      tick();
    } catch (e) {
      postMessage({ type: 'error', message: `Could not start the emulator: ${e.message}` });
    }
  } else if (msg.type === 'buttons' && wasm) {
    fn('set_buttons')(msg.mask);
  } else if (msg.type === 'report') {
    let text = 'The emulator has not started.';
    if (wasm) {
      fn('report')();
      text = new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, fn('report_ptr')(), fn('report_len')()));
    }
    postMessage({ type: 'report', text });
  } else if (msg.type === 'stop') {
    running = false;
    clearTimeout(timer);
    gs?.stop();
    gs = null;
  }
};
