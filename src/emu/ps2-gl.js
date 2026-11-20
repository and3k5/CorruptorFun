// The PS2 GS drawn with WebGL2 on an OffscreenCanvas, in the drawing
// worker (ps2-gs.worker.js). The core (emu/ps2/src/gsdraw/gpu.rs) turns the
// GS's primitives into commands, vertices and pixel data in its memory;
// `run` carries them out. Frame buffers ("targets") are textures, Z buffers
// depth textures, and the picture is drawn into the canvas, from which
// `picture` takes an ImageBitmap.
//
// Target alpha is stored as As / 128 (0x80 is full): see gpu.rs.

const OP_TARGET = 1;
const OP_TUPLOAD = 2;
const OP_DEPTH = 3;
const OP_TEX = 4;
const OP_DRAW = 5;
const OP_DISPLAY = 6;
const OP_IMAGE = 7;
const OP_FREE_TARGET = 8;
const OP_POINTS = 9;
const STATE_WORDS = 32;
const DISPLAY_WORDS = 16;
const VERT_BYTES = 11 * 4;
const NONE = 0xffffffff;

const BL_NONE = 0;
const BL_ADD = 1;
const BL_LERP = 2;
const BL_SUB = 3;
const BL_SRC = 4;
const BL_INVLERP = 5;

const DRAW_VS = `#version 300 es
layout(location = 0) in vec2 aPos;
layout(location = 1) in float aZ;
layout(location = 2) in vec4 aCol;
layout(location = 3) in vec3 aSTQ;
layout(location = 4) in float aFog;
uniform vec2 uSize;
out vec4 vCol;
flat out vec4 vFlat;
out vec3 vSTQ;
out float vZ;
out float vFog;
void main() {
  vCol = aCol;
  vFlat = aCol;
  vSTQ = aSTQ;
  vZ = aZ;
  vFog = aFog;
  // The GS takes a pixel when its corner is inside, GL when its centre is.
  gl_Position = vec4((aPos + 0.5) / uSize * 2.0 - 1.0, 0.0, 1.0);
  gl_PointSize = 1.0;
}`;

const DRAW_FS = `#version 300 es
precision highp float;
precision highp int;
in vec4 vCol;
flat in vec4 vFlat;
in vec3 vSTQ;
in float vZ;
in float vFog;
uniform sampler2D uTex;
uniform int uTexKind;      // 0 none, 1 a target, 2 an uploaded texture
uniform ivec2 uTexSize;    // TW, TH
uniform ivec2 uTexReal;    // the GL texture's size
uniform int uRel;          // target: the texture's first page past the target's
uniform int uTbw;
uniform int uFbw;
uniform ivec2 uWrap;
uniform ivec2 uRegU;
uniform ivec2 uRegV;
uniform int uTfx;
uniform bool uTcc;
uniform bool uBil;
uniform int uTAlpha;       // 0 as uploaded, 1 target (A / 128), 2 target CT24 (TEXA)
uniform float uTA0;
uniform bool uAem;
uniform bool uFst;
uniform bool uIip;
uniform bool uFge;
uniform vec3 uFogCol;
uniform int uAtst;
uniform float uAref;
uniform int uShape;
uniform int uCf;           // 0 As, 1 Ad, 2 FIX
uniform float uFix;
uniform bool uFba;
out vec4 oCol;

int wrapc(int c, int size, int mode, ivec2 reg) {
  if (mode == 0) return c & (size - 1);
  if (mode == 1) return clamp(c, 0, size - 1);
  if (mode == 2) return clamp(c, reg.x, reg.y);
  return (c & reg.x) | reg.y;
}

vec4 fetch(int u, int v) {
  u = wrapc(u, uTexSize.x, uWrap.x, uRegU);
  v = wrapc(v, uTexSize.y, uWrap.y, uRegV);
  ivec2 p = ivec2(u, v);
  if (uTexKind == 1) {
    // The texel's page, found in the target.
    int page = uRel + (v >> 5) * uTbw + (u >> 6);
    int row = page / uFbw;
    p = ivec2((page - row * uFbw) * 64 + (u & 63), row * 32 + (v & 31));
  }
  if (p.x < 0 || p.y < 0 || p.x >= uTexReal.x || p.y >= uTexReal.y) return vec4(0.0);
  vec4 t = floor(texelFetch(uTex, p, 0) * 255.0 + 0.5);
  if (uTAlpha == 1) t.a = floor(t.a * 128.0 / 255.0 + 0.5);
  else if (uTAlpha == 2) t.a = (uAem && t.r + t.g + t.b == 0.0) ? 0.0 : uTA0;
  return t;
}

vec4 sampleTex(vec2 uv) {
  if (uBil) {
    // In 1/16 texels, as the GS.
    vec2 q = floor(uv * 16.0) / 16.0 - 0.5;
    ivec2 i = ivec2(floor(q));
    vec2 f = q - floor(q);
    vec4 a = fetch(i.x, i.y);
    vec4 b = fetch(i.x + 1, i.y);
    vec4 c = fetch(i.x, i.y + 1);
    vec4 d = fetch(i.x + 1, i.y + 1);
    return floor(mix(mix(a, b, f.x), mix(c, d, f.x), f.y));
  }
  ivec2 i = ivec2(floor(uv));
  return fetch(i.x, i.y);
}

void main() {
  vec4 cf = floor(uIip ? vCol : vFlat);
  vec4 c = cf;
  if (uTexKind != 0) {
    vec2 uv;
    if (uFst || vSTQ.z < 0.0) uv = vSTQ.xy;
    else uv = vSTQ.xy / (vSTQ.z == 0.0 ? 1.0 : vSTQ.z) * vec2(uTexSize);
    vec4 t = sampleTex(uv);
    if (uTfx == 0) {
      c.rgb = min(floor(t.rgb * cf.rgb / 128.0), 255.0);
      if (uTcc) c.a = min(floor(t.a * cf.a / 128.0), 255.0);
    } else if (uTfx == 1) {
      c.rgb = t.rgb;
      if (uTcc) c.a = t.a;
    } else {
      c.rgb = min(floor(t.rgb * cf.rgb / 128.0) + cf.a, 255.0);
      if (uTcc) c.a = uTfx == 2 ? min(t.a + cf.a, 255.0) : t.a;
    }
  }
  // Alpha test (a failing pixel is dropped).
  bool pass = true;
  if (uAtst == 0) pass = false;
  else if (uAtst == 2) pass = c.a < uAref;
  else if (uAtst == 3) pass = c.a <= uAref;
  else if (uAtst == 4) pass = c.a == uAref;
  else if (uAtst == 5) pass = c.a >= uAref;
  else if (uAtst == 6) pass = c.a > uAref;
  else if (uAtst == 7) pass = c.a != uAref;
  if (!pass) discard;
  if (uFge) c.rgb = floor((c.rgb * vFog + uFogCol * (255.0 - vFog)) / 255.0);
  float store = uFba ? 1.0 : min(c.a, 128.0) / 128.0;
  float k = uCf == 0 ? c.a : uFix;
  vec3 rgb = c.rgb;
  if ((uShape == ${BL_ADD} || uShape == ${BL_SUB} || uShape == ${BL_SRC}) && uCf != 1) {
    rgb = uCf == 3 ? vec3(0.0) : min(floor(c.rgb * k / 128.0), 255.0);
  }
  float a = store;
  if ((uShape == ${BL_LERP} || uShape == ${BL_INVLERP}) && uCf == 0) a = min(c.a / 128.0, 1.0);
  oCol = vec4(rgb / 255.0, a);
  gl_FragDepth = vZ;
}`;

const POINTS_VS = `#version 300 es
layout(location = 0) in vec2 aPos;
layout(location = 1) in vec4 aCol;
uniform vec2 uSize;
out vec4 vCol;
void main() {
  vCol = aCol;
  gl_Position = vec4((aPos + 0.5) / uSize * 2.0 - 1.0, 0.0, 1.0);
  gl_PointSize = 1.0;
}`;

const POINTS_FS = `#version 300 es
precision highp float;
in vec4 vCol;
out vec4 oCol;
void main() { oCol = vCol; }`;

const QUAD_VS = `#version 300 es
void main() {
  vec2 p = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
  gl_Position = vec4(p, 0.0, 1.0);
}`;

// The display circuits (as Draw::display in gsdraw.rs), top row first.
const DISPLAY_FS = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D uSrc0;
uniform sampler2D uSrc1;
uniform ivec2 uOff0;
uniform ivec2 uOff1;
uniform int uFlags;   // 1: circuit 1 on, 2: circuit 2 on, 4: field mode, 8: MMOD
uniform float uAlp;
uniform vec3 uBg;
uniform int uH;
out vec4 oCol;
vec4 rd(sampler2D s, ivec2 p) {
  ivec2 n = textureSize(s, 0);
  if (p.x >= n.x || p.y >= n.y) return vec4(0.0);
  return floor(texelFetch(s, p, 0) * 255.0 + 0.5);
}
void main() {
  int x = int(gl_FragCoord.x);
  int y = uH - 1 - int(gl_FragCoord.y);
  if ((uFlags & 4) != 0) y >>= 1;
  vec3 c = uBg;
  if ((uFlags & 2) != 0) c = rd(uSrc1, ivec2(x, y) + uOff1).rgb;
  if ((uFlags & 1) != 0) {
    vec4 c1 = rd(uSrc0, ivec2(x, y) + uOff0);
    float a1 = floor(c1.a * 128.0 / 255.0 + 0.5);
    float a = min((uFlags & 8) != 0 ? uAlp : min(a1, 128.0) * 2.0, 255.0);
    c = (uFlags & 2) != 0 ? floor((c1.rgb * a + c * (255.0 - a)) / 255.0) : c1.rgb;
  }
  oCol = vec4(c / 255.0, 1.0);
}`;

const IMAGE_FS = `#version 300 es
precision highp float;
uniform sampler2D uImg;
uniform int uH;
out vec4 oCol;
void main() {
  oCol = vec4(texelFetch(uImg, ivec2(int(gl_FragCoord.x), uH - 1 - int(gl_FragCoord.y)), 0).rgb, 1.0);
}`;

function program(gl, vs, fs) {
  const p = gl.createProgram();
  for (const [type, src] of [
    [gl.VERTEX_SHADER, vs],
    [gl.FRAGMENT_SHADER, fs],
  ]) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(`PS2 GPU shader: ${gl.getShaderInfoLog(s)}`);
    gl.attachShader(p, s);
  }
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(`PS2 GPU shader: ${gl.getProgramInfoLog(p)}`);
  const u = {};
  const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
  for (let i = 0; i < n; i++) {
    const name = gl.getActiveUniform(p, i).name;
    u[name] = gl.getUniformLocation(p, name);
  }
  return { p, u };
}

export class GsGl {
  // Throws where WebGL2 isn't there, or (unless forced) runs on the CPU:
  // the caller then draws in software.
  constructor(wasm, force = false) {
    if (typeof OffscreenCanvas === 'undefined') throw new Error('no OffscreenCanvas');
    this.wasm = wasm;
    this.canvas = new OffscreenCanvas(640, 448);
    const gl = this.canvas.getContext('webgl2', { antialias: false, alpha: false, depth: false, preserveDrawingBuffer: false, premultipliedAlpha: false });
    if (!gl) throw new Error('no WebGL2');
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    const renderer = String(gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER));
    if (!force && /swiftshader|llvmpipe|softpipe|software|basic render/i.test(renderer)) throw new Error(`WebGL2 runs on the CPU here (${renderer})`);
    this.gl = gl;
    gl.disable(gl.DITHER);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.pixelStorei(gl.PACK_ALIGNMENT, 1);
    this.draw = program(gl, DRAW_VS, DRAW_FS);
    this.points = program(gl, POINTS_VS, POINTS_FS);
    this.display = program(gl, QUAD_VS, DISPLAY_FS);
    this.image = program(gl, QUAD_VS, IMAGE_FS);
    this.targets = []; // { tex, w, h }
    this.depths = []; // { tex, w, h }
    this.texs = []; // { tex, w, h }
    this.fbos = new Map(); // "target/depth" -> framebuffer
    this.scratch = null; // copy of a target that draws from itself
    this.imageTex = null;
    this.width = 640;
    this.height = 448;
    this.vbuf = gl.createBuffer();
    this.vao = gl.createVertexArray();
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbuf);
    const attrs = [
      [0, 2, 0],
      [1, 1, 8],
      [2, 4, 12],
      [3, 3, 28],
      [4, 1, 40],
    ];
    for (const [loc, size, off] of attrs) {
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, gl.FLOAT, false, VERT_BYTES, off);
    }
    this.pbuf = gl.createBuffer();
    this.pvao = gl.createVertexArray();
    gl.bindVertexArray(this.pvao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.pbuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.UNSIGNED_SHORT, false, 8, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.UNSIGNED_BYTE, true, 8, 4);
    this.qvao = gl.createVertexArray();
    gl.bindVertexArray(null);
  }

  texture(w, h, data) {
    const gl = this.gl;
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
    return t;
  }

  fbo(target, depth) {
    const key = `${target}/${depth}`;
    let f = this.fbos.get(key);
    if (f) return f;
    const gl = this.gl;
    f = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, f);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.targets[target].tex, 0);
    if (depth !== NONE) gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, this.depths[depth].tex, 0);
    this.fbos.set(key, f);
    return f;
  }

  dropTarget(id) {
    const t = this.targets[id];
    if (!t) return;
    for (const [k, f] of this.fbos) {
      if (k.startsWith(`${id}/`)) {
        this.gl.deleteFramebuffer(f);
        this.fbos.delete(k);
      }
    }
    this.gl.deleteTexture(t.tex);
    this.targets[id] = null;
  }

  // Carries out what the core has queued, then empties its buffers.
  run() {
    const w = this.wasm;
    const gl = this.gl;
    const n = w.ps2_gs_gpu_cmds_len();
    if (!n) return;
    const buf = w.memory.buffer;
    const cmds = new Uint32Array(buf, w.ps2_gs_gpu_cmds_ptr(), n);
    const blob = new Uint8Array(buf, w.ps2_gs_gpu_blob_ptr());
    const nv = w.ps2_gs_gpu_verts_len();
    if (nv) {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.vbuf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(buf, w.ps2_gs_gpu_verts_ptr(), nv), gl.STREAM_DRAW);
    }
    let i = 0;
    while (i < n) {
      const op = cmds[i++];
      if (op === OP_TARGET) {
        const [id, tw, th, off] = cmds.subarray(i, i + 4);
        i += 4;
        this.dropTarget(id);
        this.targets[id] = { tex: this.texture(tw, th, blob.subarray(off, off + tw * th * 4)), w: tw, h: th };
      } else if (op === OP_TUPLOAD) {
        const [id, x, y, tw, th, off] = cmds.subarray(i, i + 6);
        i += 6;
        gl.bindTexture(gl.TEXTURE_2D, this.targets[id].tex);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, x, y, tw, th, gl.RGBA, gl.UNSIGNED_BYTE, blob.subarray(off, off + tw * th * 4));
      } else if (op === OP_DEPTH) {
        const [id, dw, dh] = cmds.subarray(i, i + 3);
        i += 3;
        const t = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT32F, dw, dh, 0, gl.DEPTH_COMPONENT, gl.FLOAT, null);
        this.depths[id] = { tex: t, w: dw, h: dh };
      } else if (op === OP_TEX) {
        const [id, tw, th, off] = cmds.subarray(i, i + 4);
        i += 4;
        if (this.texs[id]) gl.deleteTexture(this.texs[id].tex);
        this.texs[id] = { tex: this.texture(tw, th, blob.subarray(off, off + tw * th * 4)), w: tw, h: th };
      } else if (op === OP_DRAW) {
        const s = cmds.subarray(i, i + STATE_WORDS);
        i += STATE_WORDS;
        const mode = cmds[i++];
        const first = cmds[i++];
        const count = cmds[i++];
        if (count) this.drawBatch(s, mode, first, count);
      } else if (op === OP_POINTS) {
        const [id, np, mask, off] = cmds.subarray(i, i + 4);
        i += 4;
        if (np) this.drawPoints(id, np, mask, blob.subarray(off, off + np * 8));
      } else if (op === OP_DISPLAY) {
        this.drawDisplay(cmds.subarray(i, i + DISPLAY_WORDS));
        i += DISPLAY_WORDS;
      } else if (op === OP_IMAGE) {
        const [iw, ih, off] = cmds.subarray(i, i + 3);
        i += 3;
        this.drawImage(iw, ih, blob.subarray(off, off + iw * ih * 4));
      } else if (op === OP_FREE_TARGET) {
        this.dropTarget(cmds[i++]);
      } else {
        throw new Error(`PS2 GPU: unknown command ${op}`);
      }
    }
    w.ps2_gs_gpu_clear();
  }

  bindTarget(s0, depth) {
    const gl = this.gl;
    const t = this.targets[s0];
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo(s0, depth));
    gl.viewport(0, 0, t.w, t.h);
    return t;
  }

  drawBatch(s, mode, first, count) {
    const gl = this.gl;
    const { p, u } = this.draw;
    const t = this.bindTarget(s[0], s[1]);
    gl.useProgram(p);
    gl.bindVertexArray(this.vao);
    gl.uniform2f(u.uSize, t.w, t.h);
    // Scissor.
    gl.enable(gl.SCISSOR_TEST);
    const x0 = Math.min(s[2], t.w);
    const y0 = Math.min(s[3], t.h);
    gl.scissor(x0, y0, Math.max(0, Math.min(s[4], t.w) - x0), Math.max(0, Math.min(s[5], t.h) - y0));
    // Depth.
    if (s[9] === 9 || s[1] === NONE) {
      gl.disable(gl.DEPTH_TEST);
    } else {
      gl.enable(gl.DEPTH_TEST);
      gl.depthFunc([gl.NEVER, gl.ALWAYS, gl.GEQUAL, gl.GREATER][s[9]]);
      gl.depthMask(!!s[10]);
    }
    // Frame buffer mask: channels whose bits are all masked.
    const m = s[11];
    gl.colorMask((m & 0xff) !== 0xff, ((m >> 8) & 0xff) !== 0xff, ((m >> 16) & 0xff) !== 0xff, m >>> 24 !== 0xff);
    // Blending.
    const shape = s[6];
    const cf = s[7];
    const fix = s[8];
    this.blend(shape, cf, fix);
    // Texture.
    const kind = s[15];
    gl.uniform1i(u.uTexKind, kind);
    if (kind) {
      let tex;
      if (kind === 1) {
        tex = this.targets[s[16]];
        if (s[16] === s[0]) tex = this.copyOf(s[16]);
      } else {
        tex = this.texs[s[16]];
      }
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, tex.tex);
      gl.uniform1i(u.uTex, 0);
      gl.uniform2i(u.uTexSize, s[17], s[18]);
      gl.uniform2i(u.uTexReal, tex.w, tex.h);
      gl.uniform1i(u.uRel, s[19]);
      gl.uniform1i(u.uTbw, s[29]);
      gl.uniform1i(u.uFbw, s[30]);
      gl.uniform2i(u.uWrap, s[20] & 15, (s[20] >> 4) & 15);
      gl.uniform2i(u.uRegU, s[21] & 0xffff, s[21] >>> 16);
      gl.uniform2i(u.uRegV, s[22] & 0xffff, s[22] >>> 16);
      gl.uniform1i(u.uTfx, s[23] & 15);
      gl.uniform1i(u.uTcc, (s[23] >> 4) & 1);
      gl.uniform1i(u.uBil, (s[23] >> 5) & 1);
      gl.uniform1i(u.uTAlpha, (s[23] >> 8) & 3);
      gl.uniform1f(u.uTA0, s[24] & 0xff);
      gl.uniform1i(u.uAem, (s[24] >> 8) & 1);
    }
    const f = s[25];
    gl.uniform1i(u.uFst, f & 1);
    gl.uniform1i(u.uIip, (f >> 1) & 1);
    gl.uniform1i(u.uFge, (f >> 2) & 1);
    gl.uniform1i(u.uFba, (f >> 3) & 1);
    gl.uniform3f(u.uFogCol, s[26] & 0xff, (s[26] >> 8) & 0xff, (s[26] >> 16) & 0xff);
    gl.uniform1i(u.uAtst, s[12]);
    gl.uniform1f(u.uAref, s[13]);
    gl.uniform1i(u.uShape, shape);
    gl.uniform1i(u.uCf, cf);
    gl.uniform1f(u.uFix, fix);
    gl.drawArrays(mode, first, count);
  }

  blend(shape, cf, fix) {
    const gl = this.gl;
    if (shape === BL_NONE || (shape === BL_SRC && cf !== 1)) {
      gl.disable(gl.BLEND);
      return;
    }
    gl.enable(gl.BLEND);
    // The colour's factor C: As (via the shader's alpha), Ad, or FIX.
    const k = Math.min(fix / 128, 1);
    gl.blendColor(k, k, k, 0);
    const C = [gl.SRC_ALPHA, gl.DST_ALPHA, gl.CONSTANT_COLOR][cf] ?? gl.SRC_ALPHA;
    const notC = [gl.ONE_MINUS_SRC_ALPHA, gl.ONE_MINUS_DST_ALPHA, gl.ONE_MINUS_CONSTANT_COLOR][cf] ?? gl.ONE_MINUS_SRC_ALPHA;
    gl.blendEquationSeparate(shape === BL_SUB ? gl.FUNC_REVERSE_SUBTRACT : gl.FUNC_ADD, gl.FUNC_ADD);
    // ADD, SUB and SRC have the colour already scaled by C in the shader,
    // except for C = Ad.
    const scaled = cf === 1 ? gl.DST_ALPHA : gl.ONE;
    if (shape === BL_ADD || shape === BL_SUB) gl.blendFuncSeparate(scaled, gl.ONE, gl.ONE, gl.ZERO);
    else if (shape === BL_SRC) gl.blendFuncSeparate(gl.DST_ALPHA, gl.ZERO, gl.ONE, gl.ZERO);
    else if (shape === BL_LERP) gl.blendFuncSeparate(C, notC, gl.ONE, gl.ZERO);
    else gl.blendFuncSeparate(notC, C, gl.ONE, gl.ZERO);
  }

  // A copy of a target, for drawing that reads the target it draws to.
  copyOf(id) {
    const gl = this.gl;
    const t = this.targets[id];
    if (!this.scratch || this.scratch.w !== t.w || this.scratch.h !== t.h) {
      if (this.scratch) gl.deleteTexture(this.scratch.tex);
      this.scratch = { tex: this.texture(t.w, t.h, null), w: t.w, h: t.h };
    }
    gl.bindTexture(gl.TEXTURE_2D, this.scratch.tex);
    gl.copyTexSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 0, 0, t.w, t.h);
    return this.scratch;
  }

  drawPoints(id, n, mask, data) {
    const gl = this.gl;
    const t = this.bindTarget(id, NONE);
    gl.useProgram(this.points.p);
    gl.uniform2f(this.points.u.uSize, t.w, t.h);
    gl.bindVertexArray(this.pvao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.pbuf);
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STREAM_DRAW);
    gl.disable(gl.SCISSOR_TEST);
    gl.disable(gl.DEPTH_TEST);
    gl.disable(gl.BLEND);
    gl.colorMask(true, true, true, mask >>> 24 !== 0);
    gl.drawArrays(gl.POINTS, 0, n);
  }

  screen(w, h) {
    const gl = this.gl;
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    this.width = w;
    this.height = h;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, w, h);
    gl.disable(gl.SCISSOR_TEST);
    gl.disable(gl.DEPTH_TEST);
    gl.disable(gl.BLEND);
    gl.colorMask(true, true, true, true);
    gl.bindVertexArray(this.qvao);
  }

  drawDisplay(d) {
    const gl = this.gl;
    const [w, h, flags, alp, s0, x0, y0, s1, x1, y1, bg] = d;
    this.screen(w, h);
    const { p, u } = this.display;
    gl.useProgram(p);
    const any = this.targets[s0 !== NONE ? s0 : s1];
    for (const [unit, id] of [
      [0, s0],
      [1, s1],
    ]) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, (this.targets[id] || any)?.tex ?? null); // none: nothing is shown
    }
    gl.uniform1i(u.uSrc0, 0);
    gl.uniform1i(u.uSrc1, 1);
    gl.uniform2i(u.uOff0, x0, y0);
    gl.uniform2i(u.uOff1, x1, y1);
    gl.uniform1i(u.uFlags, flags);
    gl.uniform1f(u.uAlp, alp);
    gl.uniform3f(u.uBg, bg & 0xff, (bg >> 8) & 0xff, (bg >> 16) & 0xff);
    gl.uniform1i(u.uH, h);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.activeTexture(gl.TEXTURE0);
  }

  drawImage(w, h, data) {
    const gl = this.gl;
    if (this.imageTex) gl.deleteTexture(this.imageTex);
    this.imageTex = this.texture(w, h, data);
    this.screen(w, h);
    gl.useProgram(this.image.p);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.imageTex);
    gl.uniform1i(this.image.u.uImg, 0);
    gl.uniform1i(this.image.u.uH, h);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  // Copies a target's pixels into the core, when it asks (see gpu.rs).
  readback() {
    const w = this.wasm;
    const id = w.ps2_gs_gpu_readback();
    if (id < 0) return false;
    const gl = this.gl;
    const t = this.targets[id];
    const ptr = w.ps2_gs_gpu_readback_buf();
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo(id, NONE));
    gl.readPixels(0, 0, t.w, t.h, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(w.memory.buffer, ptr, t.w * t.h * 4));
    w.ps2_gs_gpu_readback_done();
    return true;
  }

  // Draws input already copied into the core (`ps2_gs_input`, n u64s).
  frame(n) {
    const w = this.wasm;
    const pairs = Math.max(0, (n - 7) >> 1);
    let at = 0;
    for (;;) {
      at = w.ps2_gs_gpu_run(at, n);
      this.run();
      if (at >= pairs || !this.readback()) break;
    }
  }

  // The picture, as an ImageBitmap.
  picture() {
    const w = this.wasm;
    for (;;) {
      const waits = w.ps2_gs_gpu_display();
      this.run();
      if (!waits || !this.readback()) break;
    }
    return { bitmap: this.canvas.transferToImageBitmap(), w: this.width, h: this.height };
  }
}
