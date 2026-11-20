//! GPU drawing: instead of rasterizing, the GS turns each primitive into
//! vertices plus a block of pipeline state, for a WebGL2 renderer in the
//! drawing worker (`src/emu/ps2-gl.js`) to draw.
//!
//! Frame buffers live on the GPU as "targets", one texture per (FBP, FBW)
//! of a 32/24-bit buffer, with their Z buffers as depth textures. Local
//! memory here stays the reference for everything else (transfers, CLUTs,
//! the other texture formats). The two are kept in step page by page:
//! - a texture whose pages belong to a target is read from the target's
//!   texture on the GPU (the boot's zoom and glow passes);
//! - anything else that reads local memory where a target has drawn
//!   first has the target read back (`readback`): drawing stops there
//!   until the renderer has copied the target's pixels in;
//! - a transfer into a target's pages uploads the target again.
//!
//! Target alpha is stored as As / 128 (so 0x80, "1.0", is full), which is
//! what the GPU's blend factors need.
//!
//! The renderer reads three buffers: `cmds` (u32 words, see the OP_
//! constants), `verts` (VERT_FLOATS f32s a vertex) and `blob` (pixels
//! that commands point into).

use super::*;

pub const OP_TARGET: u32 = 1; // id, w, h, blob offset (w*h RGBA)
pub const OP_TUPLOAD: u32 = 2; // id, x, y, w, h, blob offset
pub const OP_DEPTH: u32 = 3; // id, w, h
pub const OP_TEX: u32 = 4; // id, w, h, blob offset
pub const OP_DRAW: u32 = 5; // STATE_WORDS of state, gl mode, first vertex, count
pub const OP_DISPLAY: u32 = 6; // DISPLAY_WORDS
pub const OP_IMAGE: u32 = 7; // w, h, blob offset: a picture built here
pub const OP_FREE_TARGET: u32 = 8; // id
pub const OP_POINTS: u32 = 9; // id, n, channel mask, blob offset (n * [u16 x, u16 y, RGBA])

pub const VERT_FLOATS: usize = 11; // x, y, depth, r, g, b, a, s, t, q, fog
pub const STATE_WORDS: usize = 32;
pub const DISPLAY_WORDS: usize = 16;

const GL_POINTS: u32 = 0;
const GL_LINES: u32 = 1;
const GL_TRIANGLES: u32 = 4;

const MAX_TARGETS: usize = 16;
const MAX_TEXTURES: usize = 48;
const TARGET_H: u32 = 1024;

/// Blend shapes for the renderer (state word 6).
pub const BL_NONE: u32 = 0;
pub const BL_ADD: u32 = 1; // Cs * C + Cd
pub const BL_LERP: u32 = 2; // (Cs - Cd) * C + Cd
pub const BL_SUB: u32 = 3; // Cd - Cs * C
pub const BL_SRC: u32 = 4; // Cs * C
pub const BL_INVLERP: u32 = 5; // (Cd - Cs) * C + Cs

struct Target {
    fbp: u32,
    fbw: u32,
    w: u32,
    /// Rows drawn so far (pages owned).
    rows: u32,
    /// The GPU holds pixels local memory doesn't have.
    dirty: bool,
    stamp: u32,
}

struct GTex {
    key: (u64, u64, u32, u32),
    stamp: u32,
}

pub struct Gpu {
    pub cmds: Vec<u32>,
    pub verts: Vec<f32>,
    pub blob: Vec<u8>,
    targets: Vec<Option<Target>>,
    depths: Vec<(u32, u32)>, // (zbp, fbw) by depth id
    /// Target id owning each page, or -1.
    owner: Vec<i16>,
    /// Pages a target has drawn on since local memory last had them.
    page_dirty: Vec<bool>,
    texs: Vec<Option<GTex>>,
    stamp: u32,
    /// The open batch: its state and where its OP_DRAW count is.
    state: [u32; STATE_WORDS],
    mode: u32,
    count_at: Option<usize>,
    /// Target id whose pixels drawing waits for (see the module notes).
    pub readback: Option<usize>,
    pub readback_buf: Vec<u8>,
    /// Targets a transfer wrote into, uploaded once it is done.
    pending_upload: Vec<usize>,
    /// A host → local transfer of 32/24-bit pixels goes to the targets it
    /// lands in as single pixels (no readback needed): open OP_POINTS
    /// (where its count is) and which target and format it is for.
    points_at: Option<(usize, usize, u32)>,
    xfer_gpu: bool,
    /// PSMCT32 offset within a page -> (x, y) in the page.
    inv32: Vec<(u8, u8)>,
    pub stats: [u64; 4], // draws, readbacks, texture uploads, target uploads
}

impl Gpu {
    pub fn new() -> Self {
        Gpu {
            cmds: Vec::new(),
            verts: Vec::new(),
            blob: Vec::new(),
            targets: (0..MAX_TARGETS).map(|_| None).collect(),
            depths: Vec::new(),
            owner: vec![-1; 512],
            page_dirty: vec![false; 512],
            texs: (0..MAX_TEXTURES).map(|_| None).collect(),
            stamp: 0,
            state: [0; STATE_WORDS],
            mode: 0,
            count_at: None,
            readback: None,
            readback_buf: Vec::new(),
            pending_upload: Vec::new(),
            points_at: None,
            xfer_gpu: false,
            inv32: {
                let mut t = vec![(0u8, 0u8); 2048];
                for y in 0..32 {
                    for x in 0..64 {
                        t[gsmem::locate(PSMCT32, 0, 1, x, y).0] = (x as u8, y as u8);
                    }
                }
                t
            },
            stats: [0; 4],
        }
    }

    /// After the renderer has run the buffers.
    pub fn clear(&mut self) {
        self.cmds.clear();
        self.verts.clear();
        self.blob.clear();
        self.count_at = None;
    }

    fn close_batch(&mut self) {
        self.count_at = None;
        self.points_at = None;
    }

    fn target_pages(t: &Target) -> impl Iterator<Item = usize> + '_ {
        let rows = t.rows.div_ceil(32);
        let bw = t.fbw.max(1);
        (0..rows).flat_map(move |r| (0..bw).map(move |c| ((t.fbp / 32 + r * bw + c) & 511) as usize))
    }

    fn own(&mut self, id: usize) {
        let t = self.targets[id].as_ref().unwrap();
        let pages: Vec<usize> = Self::target_pages(t).collect();
        for p in pages {
            self.owner[p] = id as i16;
        }
    }
}

/// The pages a buffer of format `psm` at `bp` (blocks), `bw` (64-pixel
/// units) covers over rows y0..=y1 (whole rows of pages).
fn buffer_pages(psm: u32, bp: u32, bw: u32, y0: u32, y1: u32) -> Vec<usize> {
    let (_, ph) = page_size(psm);
    let bwp = if bpp(psm) <= 8 { (bw >> 1).max(1) } else { bw.max(1) };
    let mut v = Vec::new();
    for r in y0 / ph..=y1 / ph {
        for c in 0..bwp {
            v.push(((bp / 32 + r * bwp + c) & 511) as usize);
        }
        if bp % 32 != 0 {
            v.push(((bp / 32 + r * bwp + bwp) & 511) as usize);
        }
    }
    v
}

fn is32(psm: u32) -> bool {
    matches!(psm, PSMCT32 | PSMCT24)
}

impl Draw {
    pub fn enable_gpu(&mut self) {
        self.gpu = Some(Box::new(Gpu::new()));
    }

    /// Drawing waits for a readback (`Gpu::readback`).
    pub fn stalled(&self) -> bool {
        self.gpu.as_ref().is_some_and(|g| g.readback.is_some())
    }

    /// Checks that local memory is up to date for `pages`; if a target
    /// drew there, asks for its readback and returns false.
    fn gpu_need_cpu(&mut self, pages: &[usize]) -> bool {
        let g = self.gpu.as_mut().unwrap();
        for &p in pages {
            let o = g.owner[p & 511];
            if o >= 0 && g.page_dirty[p & 511] && g.targets[o as usize].as_ref().is_some_and(|t| t.dirty) {
                g.readback = Some(o as usize);
                g.stats[1] += 1;
                return false;
            }
        }
        true
    }

    /// Before a register write: false when it has to wait for a readback.
    pub(super) fn gpu_pre(&mut self, r: usize, v: u64) -> bool {
        match r {
            0x04 | 0x05 | 0x0C | 0x0D => {
                // A kick that will draw from a texture not on the GPU.
                let a = self.attr();
                if !a.tme || r >= 0x0C {
                    return true;
                }
                let c = self.ctx(a.ctxt);
                if self.gpu_tex_target(c.tex0).is_some() {
                    return true;
                }
                let pages = self.tex_pages(c.tex0);
                self.gpu_need_cpu(&pages)
            }
            0x06 | 0x07 | 0x16 | 0x17 => {
                // A CLUT load reads local memory.
                let t = if r >= 0x16 { (self.regs[r - 0x10] & !((0x3Fu64 << 20) | (0x7FF_FFFFu64 << 37))) | (v & ((0x3Fu64 << 20) | (0x7FF_FFFFu64 << 37))) } else { v };
                let psm = bits(t, 20, 6);
                if !matches!(psm, PSMT8 | PSMT8H | PSMT4 | PSMT4HL | PSMT4HH) || bits(t, 61, 3) == 0 {
                    return true;
                }
                let cbp = bits(t, 37, 14);
                let pages = buffer_pages(PSMCT32, cbp, 1, 0, 31);
                self.gpu_need_cpu(&pages)
            }
            0x53 => {
                let dir = v & 3;
                self.gpu.as_mut().unwrap().xfer_gpu = false;
                if dir == 0 && is32(bits(self.regs[0x50], 56, 6)) {
                    // Pixel by pixel to the GPU as well (`gpu_xfer_pixel`).
                    self.gpu.as_mut().unwrap().xfer_gpu = true;
                    return true;
                }
                let buf = self.regs[0x50];
                let pos = self.regs[0x51];
                let reg = self.regs[0x52];
                let h = bits(reg, 32, 12).max(1);
                let (dbp, dbw, dpsm) = (bits(buf, 32, 14), bits(buf, 48, 6), bits(buf, 56, 6));
                let dy = bits(pos, 48, 11);
                let mut pages = buffer_pages(dpsm, dbp, dbw, dy, (dy + h - 1).min(2047));
                if dir == 2 {
                    let (sbp, sbw, spsm) = (bits(buf, 0, 14), bits(buf, 16, 6), bits(buf, 24, 6));
                    let sy = bits(pos, 16, 11);
                    pages.extend(buffer_pages(spsm, sbp, sbw, sy, (sy + h - 1).min(2047)));
                }
                if dir == 1 {
                    // Local → host: the EE reads local memory.
                    let (sbp, sbw, spsm) = (bits(buf, 0, 14), bits(buf, 16, 6), bits(buf, 24, 6));
                    let sy = bits(pos, 16, 11);
                    pages = buffer_pages(spsm, sbp, sbw, sy, (sy + h - 1).min(2047));
                }
                if !self.gpu_need_cpu(&pages) {
                    return false;
                }
                if dir == 0 || dir == 2 {
                    // Once written, targets there take the new pixels.
                    let g = self.gpu.as_mut().unwrap();
                    for &p in &pages {
                        let o = g.owner[p & 511];
                        if o >= 0 && !g.pending_upload.contains(&(o as usize)) {
                            g.pending_upload.push(o as usize);
                        }
                    }
                }
                true
            }
            _ => true,
        }
    }

    /// A transferred pixel was written at word `i` of local memory: if a
    /// target owns that page, it gets the pixel too.
    pub(super) fn gpu_xfer_pixel(&mut self, i: usize, psm: u32) {
        let g = self.gpu.as_mut().unwrap();
        if !g.xfer_gpu {
            return;
        }
        let page = (i >> 11) & 511;
        let o = g.owner[page];
        if o < 0 {
            return;
        }
        let id = o as usize;
        let t = g.targets[id].as_ref().unwrap();
        let rel = (page as u32).wrapping_sub(t.fbp / 32) & 511;
        if t.fbp % 32 != 0 {
            return;
        }
        let (row, col) = (rel / t.fbw, rel % t.fbw);
        let (px, py) = g.inv32[i & 2047];
        let (x, y) = (col * 64 + px as u32, row * 32 + py as u32);
        let w = self.vram[i];
        let a = ((w >> 24).min(128) * 255 + 64) / 128;
        let mask = if psm == PSMCT24 { 0x00FF_FFFF } else { 0xFFFF_FFFF };
        let g = self.gpu.as_mut().unwrap();
        if g.points_at.is_none_or(|(_, tid, m)| tid != id || m != mask) {
            g.count_at = None;
            g.cmds.extend_from_slice(&[OP_POINTS, id as u32, 0, mask, g.blob.len() as u32]);
            g.points_at = Some((g.cmds.len() - 3, id, mask));
        }
        let at = g.points_at.unwrap().0;
        g.cmds[at] += 1;
        g.blob.extend_from_slice(&(x as u16).to_le_bytes());
        g.blob.extend_from_slice(&(y as u16).to_le_bytes());
        g.blob.extend_from_slice(&[w as u8, (w >> 8) as u8, (w >> 16) as u8, a as u8]);
    }

    /// Uploads targets that a finished transfer wrote into.
    fn gpu_flush_uploads(&mut self) {
        if self.xfer.active || self.gpu.as_ref().unwrap().pending_upload.is_empty() {
            return;
        }
        let ids = std::mem::take(&mut self.gpu.as_mut().unwrap().pending_upload);
        for id in ids {
            if self.gpu.as_ref().unwrap().targets[id].is_some() {
                self.gpu_upload_target(id);
            }
        }
    }

    /// Sends a target's pixels from local memory (all its rows).
    fn gpu_upload_target(&mut self, id: usize) {
        let (fbp, fbw, w) = {
            let t = self.gpu.as_ref().unwrap().targets[id].as_ref().unwrap();
            (t.fbp, t.fbw, t.w)
        };
        let off = self.target_pixels(fbp, fbw, w, TARGET_H);
        let g = self.gpu.as_mut().unwrap();
        g.close_batch();
        g.cmds.extend_from_slice(&[OP_TUPLOAD, id as u32, 0, 0, w, TARGET_H, off]);
        g.targets[id].as_mut().unwrap().dirty = false;
        for p in 0..512 {
            if g.owner[p] == id as i16 {
                g.page_dirty[p] = false;
            }
        }
        g.stats[3] += 1;
    }

    /// A buffer's pixels from local memory into the blob, alpha as stored
    /// on the GPU (As / 128). Returns the blob offset.
    fn target_pixels(&mut self, fbp: u32, fbw: u32, w: u32, h: u32) -> u32 {
        let g = self.gpu.as_mut().unwrap();
        let off = g.blob.len();
        g.blob.reserve((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let p = gsmem::read(&self.vram, PSMCT32, fbp, fbw, x, y);
                let a = ((p >> 24).min(128) * 255 + 64) / 128;
                g.blob.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8, a as u8]);
            }
        }
        off as u32
    }

    /// Takes a target's pixels read back from the GPU (`readback_buf`,
    /// bottom row first as GL gives them, i.e. row 0 first here).
    pub fn gpu_readback_done(&mut self) {
        let Some(id) = self.gpu.as_ref().unwrap().readback else { return };
        let buf = std::mem::take(&mut self.gpu.as_mut().unwrap().readback_buf);
        let (fbp, fbw, w, rows) = {
            let t = self.gpu.as_ref().unwrap().targets[id].as_ref().unwrap();
            (t.fbp, t.fbw, t.w, t.rows)
        };
        for y in 0..rows.min(TARGET_H) {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if i + 3 >= buf.len() {
                    break;
                }
                let a = (buf[i + 3] as u32 * 128 + 127) / 255;
                let v = buf[i] as u32 | (buf[i + 1] as u32) << 8 | (buf[i + 2] as u32) << 16 | a << 24;
                self.put(PSMCT32, fbp, fbw, x, y, v);
            }
        }
        let g = self.gpu.as_mut().unwrap();
        g.readback_buf = buf;
        g.targets[id].as_mut().unwrap().dirty = false;
        for p in 0..512 {
            if g.owner[p] == id as i16 {
                g.page_dirty[p] = false;
            }
        }
        g.readback = None;
    }

    /// The size of a target in pixels, for the readback buffer.
    pub fn gpu_target_size(&self, id: usize) -> (u32, u32) {
        self.gpu.as_ref().unwrap().targets[id].as_ref().map(|t| (t.w, TARGET_H)).unwrap_or((0, 0))
    }

    /// The target for a frame buffer, made (from local memory) if new.
    fn gpu_target(&mut self, fbp: u32, fbw: u32, rows: u32) -> usize {
        let fbw = fbw.max(1);
        let g = self.gpu.as_mut().unwrap();
        g.stamp = g.stamp.wrapping_add(1);
        let stamp = g.stamp;
        if let Some(id) = g.targets.iter().position(|t| t.as_ref().is_some_and(|t| t.fbp == fbp && t.fbw == fbw)) {
            let t = g.targets[id].as_mut().unwrap();
            t.stamp = stamp;
            if rows > t.rows {
                t.rows = rows.min(TARGET_H);
                g.own(id);
            }
            return id;
        }
        // A new target. Older ones it overlaps are read back and dropped.
        let pages = buffer_pages(PSMCT32, fbp, fbw, 0, rows.saturating_sub(1).min(TARGET_H - 1));
        let id = match g.targets.iter().position(|t| t.is_none()) {
            Some(i) => i,
            None => (0..MAX_TARGETS).max_by_key(|&i| stamp.wrapping_sub(g.targets[i].as_ref().unwrap().stamp)).unwrap(),
        };
        let _ = pages;
        if let Some(old) = g.targets[id].take() {
            // Evicted: its pages fall back to local memory (stale if dirty;
            // with 16 targets the boot never gets here).
            let _ = old;
            for o in g.owner.iter_mut() {
                if *o == id as i16 {
                    *o = -1;
                }
            }
            g.cmds.extend_from_slice(&[OP_FREE_TARGET, id as u32]);
        }
        let w = fbw * 64;
        let off = self.target_pixels(fbp, fbw, w, TARGET_H);
        let g = self.gpu.as_mut().unwrap();
        g.close_batch();
        g.cmds.extend_from_slice(&[OP_TARGET, id as u32, w, TARGET_H, off]);
        g.targets[id] = Some(Target { fbp, fbw, w, rows: rows.min(TARGET_H), dirty: false, stamp });
        g.own(id);
        g.stats[3] += 1;
        id
    }

    fn gpu_depth(&mut self, zbp: u32, fbw: u32) -> usize {
        let g = self.gpu.as_mut().unwrap();
        if let Some(i) = g.depths.iter().position(|&d| d == (zbp, fbw)) {
            return i;
        }
        g.depths.push((zbp, fbw));
        let id = g.depths.len() - 1;
        g.close_batch();
        g.cmds.extend_from_slice(&[OP_DEPTH, id as u32, fbw.max(1) * 64, TARGET_H]);
        id
    }

    /// The target a 32/24-bit texture lies in, with the texture's first page counted from the target's. The
    /// renderer finds each texel in the target by its page, so the texture
    /// may have another width than the target.
    fn gpu_tex_target(&self, tex0: u64) -> Option<(usize, u32)> {
        let psm = bits(tex0, 20, 6);
        if !is32(psm) {
            return None;
        }
        let tbp = bits(tex0, 0, 14);
        if tbp % 32 != 0 {
            return None;
        }
        let g = self.gpu.as_ref().unwrap();
        let pages = [(tbp / 32) as usize & 511];
        // Its first page in a target. Texels past the target's rows read
        // what the target's texture holds there (local memory as of when it
        // was made), even where another target is: textures as tall as a
        // power of two often reach past a buffer, in parts never shown.
        let o = g.owner[pages[0]];
        if o < 0 {
            return None;
        }
        let t = g.targets[o as usize].as_ref()?;
        if t.fbp % 32 != 0 {
            return None;
        }
        Some((o as usize, (tbp / 32).wrapping_sub(t.fbp / 32) & 511))
    }

    fn tex_pages(&self, tex0: u64) -> Vec<usize> {
        let psm = bits(tex0, 20, 6);
        let th = 1u32 << bits(tex0, 30, 4).min(10);
        buffer_pages(psm, bits(tex0, 0, 14), bits(tex0, 14, 6).max(1), 0, th - 1)
    }

    /// An uploaded texture for `tex0` (decoded here), cached.
    fn gpu_texture(&mut self, tex0: u64) -> usize {
        let psm = bits(tex0, 20, 6);
        let pal = matches!(psm, PSMT8 | PSMT8H | PSMT4 | PSMT4HL | PSMT4HH);
        let key_t = if pal { tex0 & 0x1FFF_FFFF_FFFF_FFFF & !(7u64 << 61) } else { tex0 & 0x3_FFFF_FFFF };
        let ver = self.tex_pages(tex0).iter().fold(0u32, |h, &p| h.wrapping_mul(31).wrapping_add(self.page_ver[p]));
        // Palettes are keyed by their colours: the boot loads its CLUTs
        // again every frame.
        let clut = if pal { self.clut.iter().fold(0x811C_9DC5u32, |h, &c| (h ^ c as u32).wrapping_mul(0x0100_0193)) } else { 0 };
        let key = (key_t, self.regs[0x3B], clut, ver);
        let g = self.gpu.as_mut().unwrap();
        g.stamp = g.stamp.wrapping_add(1);
        let stamp = g.stamp;
        if let Some(i) = g.texs.iter().position(|t| t.as_ref().is_some_and(|t| t.key == key)) {
            g.texs[i].as_mut().unwrap().stamp = stamp;
            return i;
        }
        let id = match g.texs.iter().position(|t| t.is_none()) {
            Some(i) => i,
            None => (0..MAX_TEXTURES).max_by_key(|&i| stamp.wrapping_sub(g.texs[i].as_ref().unwrap().stamp)).unwrap(),
        };
        g.texs[id] = Some(GTex { key, stamp });
        let tw = 1i32 << bits(tex0, 26, 4).min(10);
        let th = 1i32 << bits(tex0, 30, 4).min(10);
        let mut px = Vec::with_capacity((tw * th * 4) as usize);
        for v in 0..th {
            for u in 0..tw {
                let c = self.decode_texel(tex0, u, v);
                px.extend_from_slice(&c.to_le_bytes());
            }
        }
        let g = self.gpu.as_mut().unwrap();
        let off = g.blob.len() as u32;
        g.blob.extend_from_slice(&px);
        g.close_batch();
        g.cmds.extend_from_slice(&[OP_TEX, id as u32, tw as u32, th as u32, off]);
        g.stats[2] += 1;
        id
    }

    /// Draws a primitive (see `vertex`): queue entries `q`, prim type.
    /// False when the GPU doesn't draw it (a 16-bit frame buffer): then it
    /// is drawn here, into local memory.
    pub(super) fn gpu_prim(&mut self, prim: u64, q: &[Vertex; 3], a: Attr, c: &Ctx) -> bool {
        self.gpu_flush_uploads();
        let p = self.pix_state(a, c);
        // Rows the primitive can reach (for page ownership).
        let rows = (c.scissor[3] + 1).clamp(1, TARGET_H as i32) as u32;
        if !is32(p.fpsm) {
            // 16-bit frame buffers aren't drawn on the GPU (the boot has
            // none).
            return false;
        }
        let tid = self.gpu_target(p.fbp, p.fbw, rows);
        let zid = if p.zte { Some(self.gpu_depth(p.zbp, p.fbw)) } else { None };
        // Texture.
        let (tkind, tex_id, trow, talpha) = if a.tme {
            match self.gpu_tex_target(c.tex0) {
                Some((id, row)) => (1, id as u32, row, if bits(c.tex0, 20, 6) == PSMCT24 { 2 } else { 1 }),
                None => (2, self.gpu_texture(c.tex0) as u32, 0, 0),
            }
        } else {
            (0, 0, 0, 0)
        };
        let mut s = [0u32; STATE_WORDS];
        s[0] = tid as u32;
        s[1] = zid.map(|z| z as u32).unwrap_or(u32::MAX);
        s[2] = p.scissor[0].max(0) as u32;
        s[3] = p.scissor[2].max(0) as u32;
        s[4] = (p.scissor[1] + 1).max(0) as u32;
        s[5] = (p.scissor[3] + 1).max(0) as u32;
        // Blend: (A - B) * C + D, A/B/D: 0 Cs, 1 Cd, 2 zero; C: 0 As, 1 Ad, 2 FIX.
        let [ba, bb, bc, bd] = p.blend;
        let (shape, cf) = if !p.abe {
            (BL_NONE, 0)
        } else {
            match (ba, bb, bd) {
                (0, 2, 1) => (BL_ADD, bc),
                (0, 1, 1) => (BL_LERP, bc),
                (2, 0, 1) => (BL_SUB, bc),
                (0, 2, 2) => (BL_SRC, bc),
                (1, 0, 0) => (BL_INVLERP, bc),
                (0, 0, _) | (1, 1, _) | (2, 2, _) => {
                    // A - B is zero: just D.
                    if bd == 0 { (BL_NONE, 0) } else { (BL_ADD, 3) }
                }
                (1, 2, 1) => (BL_ADD, 9), // Cd * C + Cd: approximated
                _ => (BL_LERP, bc),
            }
        };
        s[6] = shape;
        s[7] = cf;
        s[8] = p.fix;
        s[9] = if p.zte { p.ztst } else { 9 };
        s[10] = (p.zte && !p.zmsk) as u32;
        s[11] = p.fbmsk;
        s[12] = if p.ate { p.atst } else { 1 };
        s[13] = p.aref;
        s[14] = p.afail;
        s[15] = tkind;
        s[16] = tex_id;
        s[17] = p.tw as u32;
        s[18] = p.th as u32;
        s[19] = trow;
        s[29] = bits(c.tex0, 14, 6).max(1);
        s[30] = p.fbw.max(1);
        if tkind == 1 {
            s[30] = self.gpu.as_ref().unwrap().targets[tex_id as usize].as_ref().unwrap().fbw;
        }
        s[20] = p.wu.0 | p.wv.0 << 4;
        s[21] = (p.wu.1 as u32) | (p.wu.2 as u32) << 16;
        s[22] = (p.wv.1 as u32) | (p.wv.2 as u32) << 16;
        s[23] = p.tfx | (p.tcc as u32) << 4 | (p.bilinear as u32) << 5 | talpha << 8;
        s[24] = p.ta0 | (p.aem as u32) << 8 | bits(self.regs[0x3B], 32, 8) << 16;
        s[25] = (a.fst as u32) | (a.iip as u32) << 1 | (p.fge as u32) << 2 | (p.fba as u32) << 3 | (p.colclamp as u32) << 4;
        s[26] = p.fog[0] | p.fog[1] << 8 | p.fog[2] << 16;
        s[27] = if p.date { 1 | p.datm << 1 } else { 0 };
        s[28] = p.scanmsk;
        let mode = match prim {
            0 => GL_POINTS,
            1 | 2 => GL_LINES,
            _ => GL_TRIANGLES,
        };
        // Vertices.
        let zs = 1.0 / 4294967296.0f64;
        let (tw, th) = (p.tw as f32, p.th as f32);
        let g = self.gpu.as_mut().unwrap();
        let push = |g: &mut Gpu, x: f32, y: f32, v: &Vertex, col: u32, z: u32, st: (f32, f32, f32)| {
            let z = (z.min(p.zmax) as f64 * zs) as f32;
            g.verts.extend_from_slice(&[
                x,
                y,
                z,
                (col & 0xFF) as f32,
                (col >> 8 & 0xFF) as f32,
                (col >> 16 & 0xFF) as f32,
                (col >> 24) as f32,
                st.0,
                st.1,
                st.2,
                v.f as f32,
            ]);
        };
        let stq = |v: &Vertex| -> (f32, f32, f32) {
            if a.fst {
                (v.u as f32 / 16.0, v.v as f32 / 16.0, 1.0)
            } else {
                (v.s, v.t, v.q)
            }
        };
        if g.count_at.is_none() || g.state != s || g.mode != mode {
            g.state = s;
            g.mode = mode;
            g.cmds.push(OP_DRAW);
            g.cmds.extend_from_slice(&s);
            g.cmds.push(mode);
            g.cmds.push((g.verts.len() / VERT_FLOATS) as u32);
            g.count_at = Some(g.cmds.len());
            g.cmds.push(0);
            g.stats[0] += 1;
        }
        let before = g.verts.len();
        let fx = |v: &Vertex| (v.x as f32 / 16.0, v.y as f32 / 16.0);
        match prim {
            0 => {
                let (x, y) = fx(&q[0]);
                push(g, x, y, &q[0], q[0].rgba, q[0].z, stq(&q[0]));
            }
            1 | 2 => {
                for v in &q[..2] {
                    let (x, y) = fx(v);
                    push(g, x, y, v, v.rgba, v.z, stq(v));
                }
            }
            6 => {
                // Sprite: two triangles; colour, depth and fog of the
                // second vertex, texture corners from both.
                let (v0, v1) = (&q[0], &q[1]);
                let (x0, y0) = fx(v0);
                let (x1, y1) = fx(v1);
                let (t0, t1) = if a.fst {
                    ((v0.u as f32 / 16.0, v0.v as f32 / 16.0), (v1.u as f32 / 16.0, v1.v as f32 / 16.0))
                } else {
                    let d = |v: &Vertex| {
                        let q = if v.q == 0.0 { 1.0 } else { v.q };
                        (v.s / q * tw, v.t / q * th)
                    };
                    (d(v0), d(v1))
                };
                // In texels with q = 1, as with FST.
                let c = [(x0, y0, t0.0, t0.1), (x1, y0, t1.0, t0.1), (x0, y1, t0.0, t1.1), (x1, y0, t1.0, t0.1), (x1, y1, t1.0, t1.1), (x0, y1, t0.0, t1.1)];
                for (x, y, u, w) in c {
                    push(g, x, y, v1, v1.rgba, v1.z, (u, w, -1.0));
                }
            }
            _ => {
                for v in q.iter() {
                    let (x, y) = fx(v);
                    push(g, x, y, v, v.rgba, v.z, stq(v));
                }
            }
        }
        let n = ((g.verts.len() - before) / VERT_FLOATS) as u32;
        let at = g.count_at.unwrap();
        g.cmds[at] += n;
        // The target now has pixels local memory lacks, on the pages the
        // primitive's box (inside the scissor) covers.
        let t = g.targets[tid].as_mut().unwrap();
        t.dirty = true;
        let (fbp, fbw) = (t.fbp, t.fbw);
        let k = match prim {
            0 => 1,
            1 | 2 | 6 => 2,
            _ => 3,
        };
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for v in &q[..k] {
            x0 = x0.min(v.x >> 4);
            y0 = y0.min(v.y >> 4);
            x1 = x1.max((v.x + 15) >> 4);
            y1 = y1.max((v.y + 15) >> 4);
        }
        let x0 = x0.max(p.scissor[0]).max(0) as u32;
        let y0 = y0.max(p.scissor[2]).max(0) as u32;
        let x1 = x1.min(p.scissor[1]).min(fbw as i32 * 64 - 1);
        let y1 = y1.min(p.scissor[3]).min(TARGET_H as i32 - 1);
        if x1 >= x0 as i32 && y1 >= y0 as i32 {
            for r in y0 / 32..=(y1 as u32) / 32 {
                for c in x0 / 64..=(x1 as u32) / 64 {
                    g.page_dirty[((fbp / 32 + r * fbw + c) & 511) as usize] = true;
                }
            }
        }
        true
    }

    /// The picture: drawn from the targets on the GPU when the displayed
    /// buffers are targets, otherwise built here (after any readback).
    /// False when it has to wait for a readback.
    pub fn gpu_display(&mut self, pmode: u64, smode2: u64, dispfb: &[u64; 2], display: &[u64; 2], bg: u64) -> bool {
        self.gpu_flush_uploads();
        let en = [pmode & 1 != 0, pmode & 2 != 0];
        let main = if en[1] { 1 } else { 0 };
        let d = display[main];
        let magh = bits(d, 23, 4) + 1;
        let magv = bits(d, 27, 2) + 1;
        let mut w = (bits(d, 32, 12) + 1) / magh;
        let mut h = (bits(d, 44, 11) + 1) / magv;
        if !en[0] && !en[1] || w < 16 || h < 16 {
            w = 640;
            h = 448;
        }
        let (w, h) = (w.min(1024), h.min(1024));
        let mut src = [u32::MAX; 2];
        let mut all = true;
        for i in 0..2 {
            if !en[i] {
                continue;
            }
            let f = dispfb[i];
            let (fbp, fbw, psm) = (bits(f, 0, 9) * 32, bits(f, 9, 6), bits(f, 15, 5));
            let g = self.gpu.as_ref().unwrap();
            match g.targets.iter().position(|t| t.as_ref().is_some_and(|t| t.fbp == fbp && t.fbw == fbw.max(1))) {
                Some(id) if is32(psm) => src[i] = id as u32,
                _ => all = false,
            }
        }
        if all {
            let g = self.gpu.as_mut().unwrap();
            g.close_batch();
            let field = (smode2 & 3 == 3) as u32;
            g.cmds.push(OP_DISPLAY);
            let words: [u32; DISPLAY_WORDS] = [
                w,
                h,
                en[0] as u32 | (en[1] as u32) << 1 | field << 2 | ((pmode >> 5 & 1) as u32) << 3,
                bits(pmode, 8, 8),
                src[0],
                bits(dispfb[0], 32, 11),
                bits(dispfb[0], 43, 11),
                src[1],
                bits(dispfb[1], 32, 11),
                bits(dispfb[1], 43, 11),
                bg as u32 & 0xFF_FFFF,
                0,
                0,
                0,
                0,
                0,
            ];
            g.cmds.extend_from_slice(&words);
            return true;
        }
        // Built here from local memory.
        let mut pages = Vec::new();
        for i in 0..2 {
            if en[i] {
                let f = dispfb[i];
                pages.extend(buffer_pages(bits(f, 15, 5), bits(f, 0, 9) * 32, bits(f, 9, 6), 0, 1023));
            }
        }
        if !self.gpu_need_cpu(&pages) {
            return false;
        }
        let mut out = Vec::new();
        let (w, h) = self.display(pmode, smode2, dispfb, display, bg, &mut out);
        let g = self.gpu.as_mut().unwrap();
        g.close_batch();
        let off = g.blob.len() as u32;
        g.blob.extend_from_slice(&out);
        g.cmds.extend_from_slice(&[OP_IMAGE, w, h, off]);
        true
    }
}
