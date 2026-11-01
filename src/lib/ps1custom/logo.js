// Turns a picture into a logo model for the licensed screen. The BIOS reads
// the model from the disc (sectors 5-11) and draws it above "PlayStation".
//
// What the BIOS accepts, measured with a US BIOS (SCPH-1001):
// - A TMD with one object, made of lit flat triangles (mode 0x20). It
//   stops at quads, untextured unlit or gradient primitives. Gouraud
//   triangles work but only shade, so flat triangles it is.
// - At most 7 sectors of model (14 KB): it reads no more than that.
// - At most about 600 triangles. More than that overruns a buffer in the BIOS
//   and scribbles over memory. We stay at 590.
// - Colours go through the BIOS's lighting. With the normal we use, a face
//   comes out about 1.125 times as bright as its colour (and then rounded
//   to 5 bits per channel), so we divide that out.
//
// The picture becomes pixel art: a grid of coloured squares, with same
// coloured neighbours merged into rectangles (two triangles each). Black and
// transparent pixels are left out, since the screen is black anyway.

export const MAX_TRIANGLES = 590;
export const MAX_BYTES = 7 * 2048;
// The part of the screen the model may use, in model units. At z = 0 one
// unit is about 0.18 pixels of a 640-wide screen; y = 0 is about 185 pixels
// down, and "PlayStation" starts about 470 units down.
const BOX = { x0: -1300, x1: 1300, y0: -900, y1: 250 };
const LIGHT_GAIN = 1.125;
const NORMALS = [
  [-2365, -2365, -2365], // front: faces the light, so colours come out true
  [0, -4096, 0], // top
  [-4096, 0, 0], // left
  [4096, 0, 0], // right
  [0, 4096, 0], // bottom
];

// Downsamples RGBA pixels to a gw x gh grid by averaging. Returns, per cell,
// [r, g, b] or null when the cell is (mostly) transparent or near black.
function sampleGrid(img, gw, gh, { blackCutoff }) {
  const { width: w, height: h, data } = img;
  const cells = new Array(gw * gh).fill(null);
  for (let gy = 0; gy < gh; gy++) {
    const ya = Math.floor((gy * h) / gh), yb = Math.max(ya + 1, Math.floor(((gy + 1) * h) / gh));
    for (let gx = 0; gx < gw; gx++) {
      const xa = Math.floor((gx * w) / gw), xb = Math.max(xa + 1, Math.floor(((gx + 1) * w) / gw));
      let r = 0, g = 0, b = 0, a = 0, n = 0;
      for (let y = ya; y < yb; y++) {
        for (let x = xa; x < xb; x++) {
          const o = (y * w + x) * 4;
          const al = data[o + 3] / 255;
          r += data[o] * al;
          g += data[o + 1] * al;
          b += data[o + 2] * al;
          a += al;
          n++;
        }
      }
      if (a / n < 0.5) continue;
      const c = [r / a, g / a, b / a];
      if (Math.max(...c) < blackCutoff) continue;
      cells[gy * gw + gx] = c;
    }
  }
  return cells;
}

// Median cut down to at most k colours. Returns the cells remapped to the
// palette (as palette indexes, or -1) and the palette.
function quantize(cells, k) {
  const pts = cells.filter(Boolean);
  if (!pts.length) return { idx: cells.map(() => -1), palette: [] };
  let boxes = [pts];
  while (boxes.length < k) {
    // Split the box with the widest channel range.
    let bi = -1, bc = 0, br = 0;
    boxes.forEach((box, i) => {
      if (box.length < 2) return;
      for (let c = 0; c < 3; c++) {
        let lo = 255, hi = 0;
        for (const p of box) {
          lo = Math.min(lo, p[c]);
          hi = Math.max(hi, p[c]);
        }
        if (hi - lo > br) [bi, bc, br] = [i, c, hi - lo];
      }
    });
    if (bi < 0 || br < 8) break;
    const box = boxes[bi].sort((a, b) => a[bc] - b[bc]);
    const mid = box.length >> 1;
    boxes.splice(bi, 1, box.slice(0, mid), box.slice(mid));
  }
  const palette = boxes.map((box) => [0, 1, 2].map((c) => Math.round(box.reduce((s, p) => s + p[c], 0) / box.length)));
  const idx = cells.map((c) => {
    if (!c) return -1;
    let best = 0, bd = Infinity;
    palette.forEach((p, i) => {
      const d = (p[0] - c[0]) ** 2 + (p[1] - c[1]) ** 2 + (p[2] - c[2]) ** 2;
      if (d < bd) [best, bd] = [i, d];
    });
    return best;
  });
  return { idx, palette };
}

// Greedy meshing: merges same-coloured cells into rectangles.
function rectangles(idx, gw, gh) {
  const done = new Uint8Array(gw * gh);
  const rects = [];
  for (let y = 0; y < gh; y++) {
    for (let x = 0; x < gw; x++) {
      const c = idx[y * gw + x];
      if (c < 0 || done[y * gw + x]) continue;
      let w = 1;
      while (x + w < gw && idx[y * gw + x + w] === c && !done[y * gw + x + w]) w++;
      let h = 1;
      grow: while (y + h < gh) {
        for (let i = 0; i < w; i++) if (idx[(y + h) * gw + x + i] !== c || done[(y + h) * gw + x + i]) break grow;
        h++;
      }
      for (let yy = y; yy < y + h; yy++) for (let xx = x; xx < x + w; xx++) done[yy * gw + xx] = 1;
      rects.push({ x, y, w, h, c });
    }
  }
  return rects;
}

// Side walls along the outline, merged into runs, for a bit of 3D.
function sideRuns(idx, gw, gh) {
  const at = (x, y) => (x < 0 || y < 0 || x >= gw || y >= gh ? -1 : idx[y * gw + x]);
  const runs = [];
  // Top and bottom edges run along rows; left and right along columns.
  for (const [dir, dx, dy] of [
    ['top', 0, -1],
    ['bottom', 0, 1],
  ]) {
    for (let y = 0; y < gh; y++) {
      for (let x = 0; x < gw; ) {
        const c = at(x, y);
        if (c < 0 || at(x + dx, y + dy) >= 0) {
          x++;
          continue;
        }
        let n = 1;
        while (at(x + n, y) === c && at(x + n + dx, y + dy) < 0) n++;
        runs.push({ dir, x, y, n, c });
        x += n;
      }
    }
  }
  for (const [dir, dx, dy] of [
    ['left', -1, 0],
    ['right', 1, 0],
  ]) {
    for (let x = 0; x < gw; x++) {
      for (let y = 0; y < gh; ) {
        const c = at(x, y);
        if (c < 0 || at(x + dx, y + dy) >= 0) {
          y++;
          continue;
        }
        let n = 1;
        while (at(x, y + n) === c && at(x + dx, y + n + dy) < 0) n++;
        runs.push({ dir, x, y, n, c });
        y += n;
      }
    }
  }
  return runs;
}

function buildTmd(rects, runs, palette, idx, gw, gh, depth) {
  const cell = Math.min((BOX.x1 - BOX.x0) / gw, (BOX.y1 - BOX.y0) / gh);
  const ox = Math.round(-(gw * cell) / 2);
  const oy = Math.round((BOX.y0 + BOX.y1) / 2 - (gh * cell) / 2);
  const D = depth ? Math.round(Math.min(140, Math.max(50, cell))) : 0;
  const zf = -D / 2, zb = D / 2;
  const X = (gx) => Math.round(ox + gx * cell);
  const Y = (gy) => Math.round(oy + gy * cell);

  const verts = [];
  const index = new Map();
  const vert = (x, y, z) => {
    const k = `${x},${y},${z}`;
    if (!index.has(k)) {
      index.set(k, verts.length);
      verts.push([x, y, z]);
    }
    return index.get(k);
  };
  const material = (rgb, shade = 1) => rgb.map((v) => Math.min(255, Math.round((v * shade) / LIGHT_GAIN)));
  const tris = [];
  const quad = ([a, b, c, d], normal, colour) => tris.push([a, b, c, normal, colour], [a, c, d, normal, colour]);

  // Sides first, so the fronts are drawn over them.
  for (const r of runs) {
    const col = material(palette[r.c], 0.6);
    if (r.dir === 'top') {
      const [x0, x1, y] = [X(r.x), X(r.x + r.n), Y(r.y)];
      quad([vert(x0, y, zb), vert(x1, y, zb), vert(x1, y, zf), vert(x0, y, zf)], 1, col);
    } else if (r.dir === 'bottom') {
      const [x0, x1, y] = [X(r.x), X(r.x + r.n), Y(r.y + 1)];
      quad([vert(x0, y, zf), vert(x1, y, zf), vert(x1, y, zb), vert(x0, y, zb)], 4, col);
    } else if (r.dir === 'left') {
      const [x, y0, y1] = [X(r.x), Y(r.y), Y(r.y + r.n)];
      quad([vert(x, y0, zb), vert(x, y0, zf), vert(x, y1, zf), vert(x, y1, zb)], 2, col);
    } else {
      const [x, y0, y1] = [X(r.x + 1), Y(r.y), Y(r.y + r.n)];
      quad([vert(x, y0, zf), vert(x, y0, zb), vert(x, y1, zb), vert(x, y1, zf)], 3, col);
    }
  }
  // Rectangles that meet another in the middle of its side (a T-junction)
  // can leave a 1-pixel crack after projection. Where a rectangle has a
  // neighbour to the right or below, stretch it a little under that
  // neighbour (drawn later, so it ends up on top).
  const PAD = 8;
  const filled = (x, y) => x >= 0 && y >= 0 && x < gw && y < gh && idx[y * gw + x] >= 0;
  for (const r of rects) {
    let right = false, below = false;
    for (let y = r.y; y < r.y + r.h && !right; y++) right = filled(r.x + r.w, y);
    for (let x = r.x; x < r.x + r.w && !below; x++) below = filled(x, r.y + r.h);
    const [x0, x1, y0, y1] = [X(r.x), X(r.x + r.w) + (right ? PAD : 0), Y(r.y), Y(r.y + r.h) + (below ? PAD : 0)];
    quad([vert(x0, y0, zf), vert(x1, y0, zf), vert(x1, y1, zf), vert(x0, y1, zf)], 0, material(palette[r.c]));
  }
  return encodeTmd(tris, verts, depth ? NORMALS : NORMALS.slice(0, 1));
}

// Writes a TMD with one object whose primitives follow the object table
// directly, all lit flat triangles: colour, then normal and vertex indexes.
export function encodeTmd(tris, verts, normals) {
  const OBJ = 28;
  const vertTop = OBJ + tris.length * 16;
  const normTop = vertTop + verts.length * 8;
  const tmd = new Uint8Array(12 + normTop + normals.length * 8);
  const v = new DataView(tmd.buffer);
  v.setUint32(0, 0x41, true); // TMD id
  v.setUint32(4, 0, true); // offsets are relative to the object table
  v.setUint32(8, 1, true); // one object
  [vertTop, verts.length, normTop, normals.length, OBJ, tris.length, 0].forEach((x, i) => v.setUint32(12 + i * 4, x, true));
  tris.forEach(([a, b, c, n, [r, g, bl]], i) => {
    const o = 12 + OBJ + i * 16;
    v.setUint32(o, 0x20000304, true); // mode 0x20, flag 0, ilen 3, olen 4
    tmd.set([r, g, bl, 0x20], o + 4);
    v.setUint16(o + 8, n, true);
    v.setUint16(o + 10, a, true);
    v.setUint16(o + 12, b, true);
    v.setUint16(o + 14, c, true);
  });
  const put = (top, list) => list.forEach((p, i) => p.forEach((x, k) => v.setInt16(12 + top + i * 8 + k * 2, x, true)));
  put(vertTop, verts);
  put(normTop, normals);
  return tmd;
}

// Counts what a model at grid width gw would cost, and builds it.
function attempt(img, gw, opts) {
  const aspect = img.height / img.width;
  const gh = Math.max(1, Math.round(gw * aspect));
  const cells = sampleGrid(img, gw, gh, opts);
  const { idx, palette } = quantize(cells, opts.colours);
  const rects = rectangles(idx, gw, gh);
  const runs = opts.depth ? sideRuns(idx, gw, gh) : [];
  const triangles = 2 * (rects.length + runs.length);
  return { gw, gh, idx, palette, rects, runs, triangles };
}

/**
 * Builds a logo model from an image ({ width, height, data: RGBA bytes }).
 * Options: colours (palette size, default 16), depth (3D sides, default
 * false), detail (grid width; default: the most that fits), blackCutoff
 * (channels below this count as background, default 24).
 * Returns { tmd, triangles, bytes, gw, gh, preview } or throws if even the
 * coarsest grid doesn't fit. `preview` is a gw x gh RGBA image of what the
 * model shows.
 */
export function imageToLogo(img, opts = {}) {
  const o = { colours: 16, depth: false, blackCutoff: 24, ...opts };
  let pick = null;
  let built = null;
  const fits = (a) => {
    if (a.triangles > MAX_TRIANGLES) return null;
    const tmd = buildTmd(a.rects, a.runs, a.palette, a.idx, a.gw, a.gh, o.depth);
    return tmd.length <= MAX_BYTES ? tmd : null;
  };
  if (o.detail) {
    pick = attempt(img, o.detail, o);
    built = fits(pick);
  } else {
    // Start from the finest grid and get coarser until it fits.
    const maxW = Math.min(128, img.width);
    for (let gw = maxW; gw >= 4; gw = gw > 48 ? gw - 4 : gw - 1) {
      const a = attempt(img, gw, o);
      const tmd = fits(a);
      if (tmd) {
        pick = a;
        built = tmd;
        break;
      }
    }
  }
  if (!built) {
    const tris = pick ? pick.triangles : 'too many';
    throw new Error(`That picture needs ${tris} triangles at this detail; the BIOS can draw about ${MAX_TRIANGLES}. Try less detail or fewer colours.`);
  }
  const preview = new Uint8ClampedArray(pick.gw * pick.gh * 4);
  pick.idx.forEach((c, i) => {
    if (c < 0) return;
    preview.set([...pick.palette[c], 255], i * 4);
  });
  return { tmd: built, triangles: pick.triangles, bytes: built.length, gw: pick.gw, gh: pick.gh, preview };
}
