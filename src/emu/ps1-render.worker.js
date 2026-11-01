// Renders a boot to a video file: runs the PS1 core as fast as the device
// allows (not in real time), draws every frame, and encodes the pictures and
// the SPU's sound with WebCodecs. Every frame lands in the file, however slow
// the device is, so the video never stutters or drifts out of sync.

import {
  Output,
  BufferTarget,
  Mp4OutputFormat,
  WebMOutputFormat,
  CanvasSource,
  AudioSampleSource,
  AudioSample,
  QUALITY_HIGH,
  canEncodeVideo,
  canEncodeAudio,
  getFirstEncodableVideoCodec,
  getFirstEncodableAudioCodec,
} from 'mediabunny';

const RATE = 44100; // the core's stereo output rate

let cancelled = false;

// MP4 (H.264 + AAC) plays everywhere, so it comes first; WebM (VP9/VP8 +
// Opus) is for browsers that can't encode those (Firefox on some systems).
async function pickFormat(width, height) {
  const size = { width, height };
  if ((await canEncodeVideo('avc', size)) && (await canEncodeAudio('aac', { numberOfChannels: 2, sampleRate: RATE }))) {
    return { format: new Mp4OutputFormat({ fastStart: 'in-memory' }), video: 'avc', audio: 'aac', ext: 'mp4' };
  }
  const video = await getFirstEncodableVideoCodec(['vp9', 'vp8', 'av1'], size);
  if (video) {
    const audio = await getFirstEncodableAudioCodec(['opus', 'vorbis'], { numberOfChannels: 2, sampleRate: RATE });
    return { format: new WebMOutputFormat(), video, audio, ext: 'webm' };
  }
  const avc = (await canEncodeVideo('avc', size)) ? 'avc' : null;
  if (avc) return { format: new Mp4OutputFormat({ fastStart: 'in-memory' }), video: avc, audio: null, ext: 'mp4' };
  return null;
}

async function render(msg) {
  const { width, height, seconds, runGame } = msg;
  if (typeof VideoEncoder === 'undefined') throw new Error('This browser cannot encode video (no WebCodecs). Try a recent Chrome, Edge, Firefox or Safari.');
  const picked = await pickFormat(width, height);
  if (!picked) throw new Error(`This browser cannot encode video at ${width}×${height}. Try a smaller size.`);

  const { instance } = await WebAssembly.instantiateStreaming(fetch(msg.wasmUrl), {});
  const wasm = instance.exports;
  const copyIn = (bytes) => {
    const ptr = wasm.alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer).set(bytes, ptr);
    return ptr;
  };
  const bios = new Uint8Array(msg.bios);
  wasm.ps1_init(copyIn(bios), bios.length);
  if (msg.disc) {
    const disc = new Uint8Array(msg.disc);
    wasm.ps1_set_disc(copyIn(disc), disc.length);
  }
  wasm.ps1_set_region(msg.region.charCodeAt(0));
  wasm.ps1_set_render(1);

  // The core's picture changes size during the boot (and between modes), so
  // each frame is stretched onto a fixed 4:3 canvas, pixels kept sharp.
  const out = new OffscreenCanvas(width, height);
  const ctx = out.getContext('2d', { alpha: false });
  ctx.imageSmoothingEnabled = false;
  let frame = new OffscreenCanvas(320, 240);
  let frameCtx = frame.getContext('2d');

  const output = new Output({ format: picked.format, target: new BufferTarget() });
  const video = new CanvasSource(out, { codec: picked.video, bitrate: QUALITY_HIGH, keyFrameInterval: 2 });
  output.addVideoTrack(video);
  let audio = null;
  if (picked.audio) {
    audio = new AudioSampleSource({ codec: picked.audio, bitrate: QUALITY_HIGH });
    output.addAudioTrack(audio);
  }
  await output.start();

  let t = 0; // seconds of video so far
  let samples = 0; // stereo frames of audio so far
  let n = 0;
  let reason = 'length';
  let lastProgress = 0;
  try {
    while (t < seconds) {
      if (cancelled) {
        await output.cancel();
        postMessage({ type: 'cancelled' });
        return;
      }
      wasm.ps1_run_frame();
      const dt = 1 / (wasm.ps1_is_pal() === 1 ? 50 : 60);

      const w = wasm.ps1_frame_width();
      const h = wasm.ps1_frame_height();
      if (frame.width !== w || frame.height !== h) {
        frame = new OffscreenCanvas(w, h);
        frameCtx = frame.getContext('2d');
      }
      const px = new Uint8ClampedArray(wasm.memory.buffer, wasm.ps1_frame_ptr(), w * h * 4);
      frameCtx.putImageData(new ImageData(px.slice(), w, h), 0, 0);
      ctx.drawImage(frame, 0, 0, width, height);
      await video.add(t, dt);

      const len = wasm.ps1_audio_len();
      if (len) {
        const pcm = new Int16Array(wasm.memory.buffer, wasm.ps1_audio_ptr(), len).slice();
        wasm.ps1_audio_clear();
        if (audio) {
          const sample = new AudioSample({ data: pcm, format: 's16', numberOfChannels: 2, sampleRate: RATE, timestamp: samples / RATE });
          await audio.add(sample);
          sample.close();
        }
        samples += len / 2;
      }

      t += dt;
      n++;
      if (!runGame && wasm.ps1_game_started()) {
        reason = 'hand-off';
        break;
      }
      const now = performance.now();
      if (now - lastProgress > 150) {
        lastProgress = now;
        postMessage({ type: 'progress', seconds: t, of: seconds, frames: n });
      }
    }
  } catch (e) {
    await output.cancel().catch(() => {});
    throw new Error(`The emulator crashed ${t.toFixed(1)} s in: ${e.message}`);
  }
  postMessage({ type: 'progress', seconds: t, of: seconds, frames: n, finishing: true });
  await output.finalize();
  const buffer = output.target.buffer;
  postMessage({ type: 'done', buffer, mime: await output.getMimeType(), ext: picked.ext, seconds: t, frames: n, reason, sound: !!audio }, [buffer]);
}

onmessage = (ev) => {
  const msg = ev.data;
  if (msg.type === 'render') {
    cancelled = false;
    render(msg).catch((e) => postMessage({ type: 'error', message: e.message }));
  } else if (msg.type === 'cancel') {
    cancelled = true;
  }
};
