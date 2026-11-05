//! GS drawing: register state, the software rasterizer (points, lines,
//! triangles, sprites with textures, CLUTs, alpha/depth tests and
//! blending), host ↔ local memory transfers and the display read-out.

use crate::gsmem::{self, *};
use crate::gsmem::WORDS;

#[derive(Clone, Copy, Default)]
struct Vertex {
    x: i32, // 12.4 fixed, offset removed
    y: i32,
    z: u32,
    rgba: u32,
    q: f32,
    s: f32,
    t: f32,
    u: i32, // 10.4 fixed
    v: i32,
    f: u32,
}

#[derive(Default)]
struct Transfer {
    active: bool,
    x: u32,
    y: u32,
    bits: u128,
    nbits: u32,
}

pub struct Draw {
    pub vram: Vec<u32>,
    pub regs: [u64; 0x64],
    pub q_bits: u32,
    /// bit 0: SIGNAL, bit 1: FINISH, since the last `Gs::take_irq`.
    pub events: u32,
    pub signal_id: Option<u32>,
    pub label_id: Option<u32>,
    pub prims: u64,
    pub pixels: u64,
    pub stat_tex: [u64; 4],
    /// Don't rasterize (frames nobody will see).
    pub skip: bool,
    queue: [Vertex; 3],
    count: usize,
    /// Triangle fan anchor.
    fan0: Vertex,
    st: (f32, f32),
    clut: [u16; 512],
    cbp: [u32; 2],
    xfer: Transfer,
    /// Write count per 8 KB page of local memory, to notice stale textures.
    page_ver: Vec<u32>,
    clut_gen: u32,
    texcache: Vec<TexEntry>,
    tex_cur: usize,
    tex_stamp: u32,
    /// TEX0 of the texture made current by `prepare_texture`.
    cur_tex0: u64,
    /// One span's colours and depths (triangles).
    span_c: Vec<u32>,
    span_z: Vec<u32>,
}

/// A texture decoded to RGBA, filled in lazily 8x8 texels at a time.
struct TexEntry {
    key: (u64, u64, u32),
    tw: i32,
    th: i32,
    data: Vec<u32>,
    valid: Vec<u64>,
    pages: Vec<(u16, u32)>,
    stamp: u32,
}

type TriKernel = fn(&mut Draw, &Px, Attr, &[u32], u32, u32, usize, &[f64; 9], &[f64; 9], u32, (f64, f64));
type CopyKernel = fn(&mut Draw, &Px, u32, u32, u32, (f32, f32), (u32, u32, u32), u32, u32);
type FlatKernel = fn(&mut Draw, &Px, u32, u32, u32, u32, u32);
const TEXCACHE_SIZE: usize = 12;
/// Textures up to this many texels are decoded whole when used.
const SMALL_TEX: i32 = 256 * 256;

#[inline(always)]
fn bits(v: u64, lo: u32, n: u32) -> u32 {
    ((v >> lo) & ((1u64 << n) - 1)) as u32
}

#[derive(Clone, Copy)]
struct Ctx {
    fbp: u32,
    fbw: u32,
    fpsm: u32,
    fbmsk: u32,
    zbp: u32,
    zpsm: u32,
    zmsk: bool,
    test: u64,
    alpha: u64,
    tex0: u64,
    clamp: u64,
    tex1: u64,
    scissor: [i32; 4],
    ofx: i32,
    ofy: i32,
    fba: bool,
}

/// Per-primitive pixel pipeline settings.
struct Px {
    scissor: [i32; 4],
    scanmsk: u32,
    tme: bool,
    tfx: u32,
    tcc: bool,
    bilinear: bool,
    tw: i32,
    th: i32,
    wu: (u32, i32, i32),
    wv: (u32, i32, i32),
    fge: bool,
    fog: [u32; 3],
    ate: bool,
    atst: u32,
    aref: u32,
    afail: u32,
    date: bool,
    datm: u32,
    zte: bool,
    ztst: u32,
    zpsm: u32,
    zbp: u32,
    zmax: u32,
    zmsk: bool,
    fpsm: u32,
    fbp: u32,
    fbw: u32,
    fbmsk: u32,
    fb16: bool,
    fb24: bool,
    abe: bool,
    pabe: bool,
    blend: [u32; 4],
    /// The blend equation's common shapes (see `out`).
    bmode: u8,
    fix: u32,
    colclamp: bool,
    fba: bool,
    tex0: u64,
    /// Frame and Z buffer layouts.
    fb: Swz,
    zb: Swz,
    /// Texture read straight from memory (0: through the cache, 1: CT32,
    /// 2: CT24) and its layout.
    tdirect: u8,
    tsw: Swz,
    /// CT24 texel alpha: TEXA.TA0 and AEM.
    ta0: u32,
    aem: bool,
}

#[derive(Clone, Copy)]
struct Attr {
    iip: bool,
    tme: bool,
    fge: bool,
    abe: bool,
    fst: bool,
    ctxt: usize,
}

/// The specialised kernel `$k` for a primitive's depth test (`ZT`: 9 off,
/// 0 never, 1 always, 2 >=, 3 >), depth write (`ZW`) and blend (`BM`, as
/// `Px::bmode`; 4 none).
macro_rules! pick_kernel {
    ($p:expr, $k:ident) => {{
        let p = $p;
        macro_rules! blend {
            ($zt:expr, $zw:expr) => {
                match if p.abe { p.bmode } else { 4 } {
                    1 => Draw::$k::<$zt, $zw, 1>,
                    2 => Draw::$k::<$zt, $zw, 2>,
                    4 => Draw::$k::<$zt, $zw, 4>,
                    _ => Draw::$k::<$zt, $zw, 0>,
                }
            };
        }
        let zt = if !p.zte || (p.ztst == 1 && p.zmsk) { 9 } else { p.ztst as u8 };
        let zw = p.zte && !p.zmsk;
        match (zt, zw) {
            (9, _) => blend!(9, false),
            (0, _) => blend!(0, false),
            (1, _) => blend!(1, true),
            (2, false) => blend!(2, false),
            (2, true) => blend!(2, true),
            (_, false) => blend!(3, false),
            (_, true) => blend!(3, true),
        }
    }};
}

impl Draw {
    pub fn new() -> Self {
        Draw {
            vram: vec![0; 1 << 20],
            regs: [0; 0x64],
            q_bits: 0x3F80_0000,
            events: 0,
            signal_id: None,
            label_id: None,
            prims: 0,
            pixels: 0,
            stat_tex: [0; 4],
            skip: false,
            queue: [Vertex::default(); 3],
            count: 0,
            fan0: Vertex::default(),
            st: (0.0, 0.0),
            clut: [0; 512],
            cbp: [0; 2],
            xfer: Transfer::default(),
            page_ver: vec![0; 512],
            clut_gen: 0,
            texcache: Vec::new(),
            tex_cur: usize::MAX,
            tex_stamp: 0,
            cur_tex0: 0,
            span_c: Vec::new(),
            span_z: Vec::new(),
        }
    }

    #[inline(always)]
    fn put(&mut self, psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
        let (i, sh, m) = gsmem::locate(psm, bp, bw, x, y);
        let w = &mut self.vram[i];
        *w = (*w & !(m << sh)) | ((v & m) << sh);
        self.page_ver[i >> 11] = self.page_ver[i >> 11].wrapping_add(1);
    }

    /// Makes the texture in `c.tex0` current in the cache.
    fn prepare_texture(&mut self, c: &Ctx) {
        let t = c.tex0;
        self.cur_tex0 = t;
        let psm = bits(t, 20, 6);
        let pal = matches!(psm, PSMT8 | PSMT8H | PSMT4 | PSMT4HL | PSMT4HH);
        // TBP, TBW, PSM, TW, TH; CLUT fields only for palettized formats.
        let key_t = if pal { t & !(7u64 << 61) & 0x1FFF_FFFF_FFFF_FFFF } else { t & 0x3_FFFF_FFFF };
        let key = (key_t, if pal { 0 } else { self.regs[0x3B] }, if pal { self.clut_gen } else { 0 });
        self.tex_stamp = self.tex_stamp.wrapping_add(1);
        let stamp = self.tex_stamp;
        if let Some(k) = self.texcache.iter().position(|e| e.key == key) {
            let e = &mut self.texcache[k];
            if e.pages.iter().any(|&(p, v)| self.page_ver[p as usize] != v) {
                e.valid.iter_mut().for_each(|b| *b = 0);
                self.stat_tex[3] += 1;
                for pv in e.pages.iter_mut() {
                    pv.1 = self.page_ver[pv.0 as usize];
                }
            }
            e.stamp = stamp;
            self.tex_cur = k;
            self.decode_all();
            return;
        }
        self.stat_tex[2] += 1;
        let tw = 1i32 << bits(t, 26, 4).min(10);
        let th = 1i32 << bits(t, 30, 4).min(10);
        let tbp = bits(t, 0, 14);
        let tbw = bits(t, 14, 6).max(1);
        let (pw, ph) = page_size(psm);
        let bwp = if bpp(psm) <= 8 { (tbw >> 1).max(1) } else { tbw };
        let mut pages = Vec::new();
        for py in 0..(th as u32).div_ceil(ph) {
            for px in 0..(tw as u32).div_ceil(pw) {
                let p = ((tbp / 32 + px + py * bwp) & 511) as u16;
                if !pages.iter().any(|&(q, _)| q == p) {
                    pages.push((p, self.page_ver[p as usize]));
                }
                if tbp % 32 != 0 {
                    let q = (p + 1) & 511;
                    if !pages.iter().any(|&(r, _)| r == q) {
                        pages.push((q, self.page_ver[q as usize]));
                    }
                }
            }
        }
        let blocks = ((tw / 8) * (th / 8)).max(1) as usize;
        let e = TexEntry { key, tw, th, data: vec![0; (tw * th) as usize], valid: vec![0; blocks.div_ceil(64)], pages, stamp };
        if self.texcache.len() < TEXCACHE_SIZE {
            self.texcache.push(e);
            self.tex_cur = self.texcache.len() - 1;
        } else {
            let k = (0..self.texcache.len()).max_by_key(|&k| stamp.wrapping_sub(self.texcache[k].stamp)).unwrap();
            self.texcache[k] = e;
            self.tex_cur = k;
        }
        self.decode_all();
    }

    /// Small textures are decoded whole up front, so drawing can read the
    /// decoded texels without checking (see `Px::tdirect` 3).
    fn decode_all(&mut self) {
        let e = &self.texcache[self.tex_cur];
        let (tw, th) = (e.tw, e.th);
        if tw * th > SMALL_TEX {
            return;
        }
        let t = self.cur_tex0;
        let bw = (tw >> 3).max(1);
        for by in 0..(th >> 3).max(1) {
            for bx in 0..bw {
                let blk = (by * bw + bx) as usize;
                if self.texcache[self.tex_cur].valid[blk >> 6] >> (blk & 63) & 1 == 0 {
                    self.fill_block(t, bx * 8, by * 8, blk);
                }
            }
        }
    }

    fn ctx(&self, i: usize) -> Ctx {
        let frame = self.regs[0x4C + i];
        let zbuf = self.regs[0x4E + i];
        let sc = self.regs[0x40 + i];
        let off = self.regs[0x18 + i];
        Ctx {
            fbp: bits(frame, 0, 9) * 32,
            fbw: bits(frame, 16, 6),
            fpsm: bits(frame, 24, 6),
            fbmsk: (frame >> 32) as u32,
            zbp: bits(zbuf, 0, 9) * 32,
            zpsm: bits(zbuf, 24, 4) | 0x30,
            zmsk: zbuf >> 32 & 1 != 0,
            test: self.regs[0x47 + i],
            alpha: self.regs[0x42 + i],
            tex0: self.regs[0x06 + i],
            clamp: self.regs[0x08 + i],
            tex1: self.regs[0x14 + i],
            scissor: [bits(sc, 0, 11) as i32, bits(sc, 16, 11) as i32, bits(sc, 32, 11) as i32, bits(sc, 48, 11) as i32],
            ofx: bits(off, 0, 16) as i32,
            ofy: bits(off, 32, 16) as i32,
            fba: self.regs[0x4A + i] & 1 != 0,
        }
    }

    fn attr(&self) -> Attr {
        let prim = self.regs[0x00];
        let a = if self.regs[0x1A] & 1 != 0 { prim } else { self.regs[0x1B] };
        Attr {
            iip: a >> 3 & 1 != 0,
            tme: a >> 4 & 1 != 0,
            fge: a >> 5 & 1 != 0,
            abe: a >> 6 & 1 != 0 || a >> 7 & 1 != 0,
            fst: a >> 8 & 1 != 0,
            ctxt: (a >> 9 & 1) as usize,
        }
    }

    pub fn write_reg(&mut self, r: u32, v: u64) {
        let r = r as usize;
        match r {
            0x00 => {
                self.regs[0] = v;
                self.count = 0;
            }
            0x01 => self.regs[1] = v,
            0x02 => {
                self.regs[2] = v;
                self.st = (f32::from_bits(v as u32), f32::from_bits((v >> 32) as u32));
            }
            0x04 | 0x05 | 0x0C | 0x0D => self.vertex(v, r == 0x04 || r == 0x0C, r < 0x0C),
            0x06 | 0x07 => {
                self.regs[r] = v;
                self.clut_load(v, r - 6);
            }
            0x16 | 0x17 => {
                // TEX2: replaces PSM and the CLUT fields of TEX0.
                let i = r - 0x16;
                let keep = !((0x3Fu64 << 20) | (0x7FF_FFFFu64 << 37));
                let t = (self.regs[0x06 + i] & keep) | (v & !keep);
                self.regs[0x06 + i] = t;
                self.regs[r] = v;
                self.clut_load(t, i);
            }
            0x53 => {
                self.regs[r] = v;
                self.start_transfer();
            }
            0x54 => self.hwreg(v),
            0x60 => {
                self.events |= 1;
                self.signal_id = Some(v as u32);
            }
            0x61 => self.events |= 2,
            0x62 => self.label_id = Some(v as u32),
            _ => {
                if r < self.regs.len() {
                    self.regs[r] = v;
                }
            }
        }
    }

    // ---------------------------------------------------------- vertices --

    fn vertex(&mut self, v: u64, fog: bool, kick: bool) {
        let a = self.attr();
        let c = self.ctx(a.ctxt);
        let rgbaq = self.regs[1];
        let uv = self.regs[3];
        let vx = Vertex {
            x: bits(v, 0, 16) as i32 - c.ofx,
            y: bits(v, 16, 16) as i32 - c.ofy,
            z: if fog { bits(v, 32, 24) } else { (v >> 32) as u32 },
            f: if fog { bits(v, 56, 8) } else { bits(self.regs[0x0A], 56, 8) },
            rgba: rgbaq as u32,
            q: f32::from_bits((rgbaq >> 32) as u32),
            s: self.st.0,
            t: self.st.1,
            u: bits(uv, 0, 14) as i32,
            v: bits(uv, 16, 14) as i32,
        };
        let prim = self.regs[0] & 7;
        let need = match prim {
            0 => 1,
            1 | 2 | 6 => 2,
            7 => return,
            _ => 3,
        };
        if prim == 5 && self.count == 0 {
            self.fan0 = vx;
        }
        if self.count < need {
            self.queue[self.count] = vx;
            self.count += 1;
        } else {
            match prim {
                2 => {
                    self.queue[0] = self.queue[1];
                    self.queue[1] = vx;
                }
                4 => {
                    self.queue[0] = self.queue[1];
                    self.queue[1] = self.queue[2];
                    self.queue[2] = vx;
                }
                5 => {
                    self.queue[0] = self.fan0;
                    self.queue[1] = self.queue[2];
                    self.queue[2] = vx;
                }
                _ => {
                    self.queue[0] = vx;
                    self.count = 1;
                }
            }
        }
        if self.count < need {
            return;
        }
        if kick && !self.skip {
            self.prims += 1;
            if a.tme && !matches!(bits(c.tex0, 20, 6), PSMCT32 | PSMCT24) {
                self.prepare_texture(&c);
            }
            let q = self.queue;
            match prim {
                0 => self.point(&q[0], a, &c),
                1 | 2 => self.line(&q[0], &q[1], a, &c),
                6 => self.sprite(&q[0], &q[1], a, &c),
                _ => self.triangle(&q[0], &q[1], &q[2], a, &c),
            }
        }
        if matches!(prim, 0 | 1 | 3 | 6) {
            self.count = 0;
        }
    }

    // ------------------------------------------------------------ pixels --

    /// Everything a primitive's pixels need, worked out once per primitive.
    fn pix_state(&self, a: Attr, c: &Ctx) -> Px {
        let t = c.tex0;
        let cl = c.clamp;
        let test = c.test;
        let fc = self.regs[0x3D];
        let al = c.alpha;
        let fpsm = c.fpsm;
        let zpsm = c.zpsm;
        let (sa, sb, sd) = (bits(al, 0, 2), bits(al, 2, 2), bits(al, 6, 2));
        let sc = bits(al, 4, 2);
        Px {
            scissor: c.scissor,
            scanmsk: bits(self.regs[0x22], 0, 2),
            tme: a.tme,
            tfx: bits(t, 35, 2),
            tcc: t >> 34 & 1 != 0,
            bilinear: c.tex1 >> 5 & 1 != 0,
            tw: 1i32 << bits(t, 26, 4).min(10),
            th: 1i32 << bits(t, 30, 4).min(10),
            wu: (bits(cl, 0, 2), bits(cl, 4, 10) as i32, bits(cl, 14, 10) as i32),
            wv: (bits(cl, 2, 2), bits(cl, 24, 10) as i32, bits(cl, 34, 10) as i32),
            fge: a.fge,
            fog: [bits(fc, 0, 8), bits(fc, 8, 8), bits(fc, 16, 8)],
            ate: test & 1 != 0,
            atst: bits(test, 1, 3),
            aref: bits(test, 4, 8),
            afail: bits(test, 12, 2),
            date: test >> 14 & 1 != 0,
            datm: (test >> 15 & 1) as u32,
            zte: test >> 16 & 1 != 0,
            ztst: bits(test, 17, 2),
            zpsm,
            zbp: c.zbp,
            zmax: match bpp(zpsm) {
                16 => 0xFFFF,
                24 => 0xFF_FFFF,
                _ => 0xFFFF_FFFF,
            },
            zmsk: c.zmsk,
            fpsm,
            fbp: c.fbp,
            fbw: c.fbw,
            fbmsk: c.fbmsk,
            fb16: bpp(fpsm) == 16,
            fb24: fpsm == PSMCT24,
            abe: a.abe,
            pabe: self.regs[0x49] & 1 != 0,
            blend: [sa, sb, sc, sd],
            bmode: match (sa, sb, sd) {
                (0, 2, 1) => 1, // Cs * C + Cd
                (0, 1, 1) => 2, // (Cs - Cd) * C + Cd
                (2, 0, 1) => 3, // Cd - Cs * C
                _ => 0,
            },
            fix: bits(al, 32, 8),
            colclamp: self.regs[0x46] & 1 != 0,
            fba: c.fba,
            tex0: t,
            fb: Swz::new(fpsm, c.fbp, c.fbw),
            zb: Swz::new(zpsm, c.zbp, c.fbw),
            tdirect: match bits(t, 20, 6) {
                PSMCT32 => 1,
                PSMCT24 => 2,
                _ if (1i32 << bits(t, 26, 4).min(10)) * (1i32 << bits(t, 30, 4).min(10)) <= SMALL_TEX => 3,
                _ => 0,
            },
            tsw: Swz::new(bits(t, 20, 6), bits(t, 0, 14), bits(t, 14, 6)),
            ta0: bits(self.regs[0x3B], 0, 8),
            aem: self.regs[0x3B] >> 15 & 1 != 0,
        }
    }

    /// Marks the pages a primitive may have written (for the texture cache).
    fn touch(&mut self, p: &Px, x0: i32, y0: i32, x1: i32, y1: i32) {
        let x1 = x1.max(x0);
        let y1 = y1.max(y0);
        let mut mark = |psm: u32, bp: u32, bw: u32| {
            let (pw, ph) = page_size(psm);
            let bwp = if bpp(psm) <= 8 { (bw >> 1).max(1) } else { bw.max(1) };
            for py in (y0 as u32 & 2047) / ph..=(y1 as u32 & 2047) / ph {
                for px in (x0 as u32 & 2047) / pw..=(x1 as u32 & 2047) / pw {
                    let pg = (bp / 32 + px + py * bwp) as usize;
                    self.page_ver[pg & 511] = self.page_ver[pg & 511].wrapping_add(1);
                    if bp % 32 != 0 {
                        self.page_ver[(pg + 1) & 511] = self.page_ver[(pg + 1) & 511].wrapping_add(1);
                    }
                }
            }
        };
        mark(p.fpsm, p.fbp, p.fbw);
        if p.zte && !p.zmsk {
            mark(p.zpsm, p.zbp, p.fbw);
        }
    }

    /// A texel by integer coordinates (already wrapped), from the cache.
    /// A CT32 / CT24 texel straight from memory; `row` is `tsw.row(v)`.
    #[inline(always)]
    fn fetch_direct(&self, p: &Px, row: u32, u: u32, v: u32) -> u32 {
        let raw = self.vram[p.tsw.word(row.wrapping_add(p.tsw.col(u, v))).0];
        if p.tdirect == 1 {
            raw
        } else {
            let raw = raw & 0xFF_FFFF;
            raw | (if p.aem && raw == 0 { 0 } else { p.ta0 }) << 24
        }
    }

    #[inline(always)]
    fn texel_px(&mut self, p: &Px, u: i32, v: i32) -> u32 {
        match p.tdirect {
            1 => self.vram[p.tsw.at(u as u32 & 2047, v as u32 & 2047).0],
            2 => {
                let raw = self.vram[p.tsw.at(u as u32 & 2047, v as u32 & 2047).0] & 0xFF_FFFF;
                let a = if p.aem && raw == 0 { 0 } else { p.ta0 };
                raw | a << 24
            }
            _ => self.texel(p.tex0, u, v),
        }
    }

    #[inline(always)]
    fn texel(&mut self, tex0: u64, u: i32, v: i32) -> u32 {
        if self.tex_cur < self.texcache.len() {
            let e = &self.texcache[self.tex_cur];
            if u >= 0 && v >= 0 && u < e.tw && v < e.th {
                let i = (v * e.tw + u) as usize;
                let blk = ((v >> 3) * (e.tw >> 3).max(1) + (u >> 3)) as usize;
                if e.valid[blk >> 6] >> (blk & 63) & 1 != 0 {
                    return e.data[i];
                }
                self.fill_block(tex0, u, v, blk);
                return self.texcache[self.tex_cur].data[i];
            }
        }
        self.decode_texel(tex0, u, v)
    }

    #[cold]
    fn fill_block(&mut self, tex0: u64, u: i32, v: i32, blk: usize) {
        let e = &self.texcache[self.tex_cur];
        let (bx, by, tw) = (u & !7, v & !7, e.tw);
        let (w8, h8) = (8.min(e.tw), 8.min(e.th));
        for yy in by..by + h8 {
            for xx in bx..bx + w8 {
                let px = self.decode_texel(tex0, xx, yy);
                self.texcache[self.tex_cur].data[(yy * tw + xx) as usize] = px;
            }
        }
        self.texcache[self.tex_cur].valid[blk >> 6] |= 1 << (blk & 63);
    }

    fn decode_texel(&mut self, t: u64, u: i32, v: i32) -> u32 {
        self.stat_tex[1] += 1;
        let tbp = bits(t, 0, 14);
        let tbw = bits(t, 14, 6);
        let psm = bits(t, 20, 6);
        let raw = gsmem::read(&self.vram, psm, tbp, tbw, u as u32 & 2047, v as u32 & 2047);
        let texa = self.regs[0x3B];
        match psm {
            PSMCT32 => raw,
            PSMCT24 => {
                let aem = texa >> 15 & 1 != 0;
                let a = if aem && raw == 0 { 0 } else { bits(texa, 0, 8) };
                raw | a << 24
            }
            PSMCT16 | PSMCT16S => c16_to_32(raw, texa),
            PSMT8 | PSMT8H | PSMT4 | PSMT4HL | PSMT4HH => {
                let csa = bits(t, 56, 5);
                let cpsm = bits(t, 51, 4);
                let idx = raw as usize;
                let eight = matches!(psm, PSMT8 | PSMT8H);
                if cpsm == 0 {
                    let i = if eight { idx } else { (csa as usize & 15) * 16 + idx };
                    self.clut[i & 255] as u32 | (self.clut[(i & 255) + 256] as u32) << 16
                } else {
                    let i = if eight { idx } else { csa as usize * 16 + idx };
                    c16_to_32(self.clut[i & 511] as u32, texa)
                }
            }
            _ => raw,
        }
    }

    /// A texel of a wholly decoded texture (`data`), by wrapped coordinates.
    #[inline(always)]
    fn dec_texel(&mut self, p: &Px, data: &[u32], u: i32, v: i32) -> u32 {
        if (u as u32) < p.tw as u32 && (v as u32) < p.th as u32 {
            data[(v * p.tw + u) as usize]
        } else {
            self.decode_texel(p.tex0, u, v)
        }
    }

    /// `sample` for a wholly decoded texture.
    #[inline(always)]
    fn sample_dec(&mut self, p: &Px, data: &[u32], u16x: i32, v16x: i32) -> u32 {
        let wu = |x: i32| wrap(x, p.tw, p.wu.0, p.wu.1, p.wu.2);
        let wv = |y: i32| wrap(y, p.th, p.wv.0, p.wv.1, p.wv.2);
        if !p.bilinear {
            return self.dec_texel(p, data, wu(u16x >> 4), wv(v16x >> 4));
        }
        let u = u16x - 8;
        let v = v16x - 8;
        let (u0, v0) = (u >> 4, v >> 4);
        let (fu, fv) = ((u & 15) as u32, (v & 15) as u32);
        let (ua, va) = (wu(u0), wv(v0));
        let a = self.dec_texel(p, data, ua, va);
        if fu == 0 && fv == 0 {
            return a;
        }
        let (ub, vb) = (wu(u0 + 1), wv(v0 + 1));
        let b = self.dec_texel(p, data, ub, va);
        let cc = self.dec_texel(p, data, ua, vb);
        let d = self.dec_texel(p, data, ub, vb);
        bilerp(a, b, cc, d, fu, fv)
    }

    /// Texel for (u, v) in 1/16 texel units, nearest or bilinear.
    #[inline(always)]
    fn sample(&mut self, p: &Px, u16x: i32, v16x: i32) -> u32 {
        let wu = |x: i32| wrap(x, p.tw, p.wu.0, p.wu.1, p.wu.2);
        let wv = |y: i32| wrap(y, p.th, p.wv.0, p.wv.1, p.wv.2);
        if !p.bilinear {
            return self.texel_px(p, wu(u16x >> 4), wv(v16x >> 4));
        }
        let u = u16x - 8;
        let v = v16x - 8;
        let (u0, v0) = (u >> 4, v >> 4);
        let (fu, fv) = ((u & 15) as u32, (v & 15) as u32);
        let (ua, ub, va, vb) = (wu(u0), wu(u0 + 1), wv(v0), wv(v0 + 1));
        let a = self.texel_px(p, ua, va);
        if fu == 0 && fv == 0 {
            return a;
        }
        let b = self.texel_px(p, ub, va);
        let cc = self.texel_px(p, ua, vb);
        let d = self.texel_px(p, ub, vb);
        bilerp(a, b, cc, d, fu, fv)
    }

    #[inline(always)]
    fn pixel(&mut self, p: &Px, x: i32, y: i32, z: u32, rgba: u32, tex: (i32, i32), fog: u32) {
        if x < p.scissor[0] || x > p.scissor[1] || y < p.scissor[2] || y > p.scissor[3] {
            return;
        }
        if (p.scanmsk == 2 && y & 1 == 0) || (p.scanmsk == 3 && y & 1 == 1) {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        let c = self.shade(p, rgba, tex, fog);
        self.out(p, x, y, p.fb.row(y), p.zb.row(y), z, c);
        self.pixels += 1;
    }

    /// Texture and fog: the colour a pixel brings to the tests and blending.
    #[inline(always)]
    fn shade(&mut self, p: &Px, rgba: u32, tex: (i32, i32), fog: u32) -> u32 {
        if !p.tme {
            return if p.fge { self.fog(p, rgba, fog) } else { rgba };
        }
        let t = self.sample(p, tex.0, tex.1);
        let c = Self::modulate(p, t, rgba);
        if p.fge { self.fog(p, c, fog) } else { c }
    }

    #[inline(always)]
    fn fog(&self, p: &Px, c: u32, f: u32) -> u32 {
        let (r, g, b) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF);
        let r = (f * r + (255 - f) * p.fog[0]) >> 8;
        let g = (f * g + (255 - f) * p.fog[1]) >> 8;
        let b = (f * b + (255 - f) * p.fog[2]) >> 8;
        r | g << 8 | b << 16 | (c & 0xFF00_0000)
    }

    /// Texture function: the texel `t` combined with the vertex colour.
    #[inline(always)]
    fn modulate(p: &Px, t: u32, rgba: u32) -> u32 {
        let mut r = rgba & 0xFF;
        let mut g = (rgba >> 8) & 0xFF;
        let mut b = (rgba >> 16) & 0xFF;
        let mut al = (rgba >> 24) & 0xFF;
        {
            let (tr, tg, tb, ta) = (t & 0xFF, (t >> 8) & 0xFF, (t >> 16) & 0xFF, t >> 24);
            match p.tfx {
                0 => {
                    r = ((tr * r) >> 7).min(255);
                    g = ((tg * g) >> 7).min(255);
                    b = ((tb * b) >> 7).min(255);
                    if p.tcc {
                        al = ((ta * al) >> 7).min(255);
                    }
                }
                1 => {
                    r = tr;
                    g = tg;
                    b = tb;
                    if p.tcc {
                        al = ta;
                    }
                }
                m => {
                    r = (((tr * r) >> 7) + al).min(255);
                    g = (((tg * g) >> 7) + al).min(255);
                    b = (((tb * b) >> 7) + al).min(255);
                    if p.tcc {
                        al = if m == 2 { (ta + al).min(255) } else { ta };
                    }
                }
            }
        }
        r | g << 8 | b << 16 | al << 24
    }

    /// Tests, blending and the write for one pixel of colour `c`. `frow`
    /// and `zrow` are the buffers' row addresses (`Swz::row(y)`).
    #[inline(always)]
    fn out(&mut self, p: &Px, x: u32, y: u32, frow: u32, zrow: u32, z: u32, c: u32) {
        let (mut r, mut g, mut b, mut al) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF, c >> 24);
        // Alpha test.
        let mut write_fb = true;
        let mut write_z = !p.zmsk;
        let mut fbmask = p.fbmsk;
        if p.ate {
            let aref = p.aref;
            let pass = match p.atst {
                0 => false,
                1 => true,
                2 => al < aref,
                3 => al <= aref,
                4 => al == aref,
                5 => al >= aref,
                6 => al > aref,
                _ => al != aref,
            };
            if !pass {
                match p.afail {
                    0 => return,
                    1 => write_z = false,
                    2 => write_fb = false,
                    _ => {
                        write_z = false;
                        fbmask |= 0xFF00_0000;
                    }
                }
            }
        }
        let (fi, fsh, fm) = p.fb.word(frow.wrapping_add(p.fb.col(x, y)));
        let dst = (self.vram[fi] >> fsh) & fm;
        if p.date {
            let bit = if p.fb16 { dst >> 15 & 1 } else { dst >> 31 & 1 };
            if bit != p.datm {
                return;
            }
        }
        if p.zte && (p.ztst != 1 || write_z) {
            let z = z.min(p.zmax);
            let (zi, zsh, zm) = p.zb.word(zrow.wrapping_add(p.zb.col(x, y)));
            if p.ztst != 1 {
                let dz = (self.vram[zi] >> zsh) & zm;
                let pass = match p.ztst {
                    0 => false,
                    2 => z >= dz,
                    _ => z > dz,
                };
                if !pass {
                    return;
                }
            }
            if write_z {
                let w = &mut self.vram[zi];
                *w = (*w & !(zm << zsh)) | ((z & zm) << zsh);
            }
        }
        if !write_fb {
            return;
        }
        let d = if p.fb16 { c16_to_32(dst, 0x80 | 0x80 << 32) } else if p.fb24 { dst | 0x8000_0000 } else { dst };
        if p.abe && !(p.pabe && al & 0x80 == 0) {
            let (dr, dg, db, da) = (d & 0xFF, (d >> 8) & 0xFF, (d >> 16) & 0xFF, d >> 24);
            let [sa, sb, sc, sd] = p.blend;
            let cf = match sc {
                0 => al,
                1 => da,
                _ => p.fix,
            } as i32;
            let (sr, sg, sb_, dr, dg, db) = (r as i32, g as i32, b as i32, dr as i32, dg as i32, db as i32);
            let (nr, ng, nb) = match p.bmode {
                1 => ((sr * cf >> 7) + dr, (sg * cf >> 7) + dg, (sb_ * cf >> 7) + db),
                2 => (((sr - dr) * cf >> 7) + dr, ((sg - dg) * cf >> 7) + dg, ((sb_ - db) * cf >> 7) + db),
                3 => ((-sr * cf >> 7) + dr, (-sg * cf >> 7) + dg, (-sb_ * cf >> 7) + db),
                _ => {
                    let sel = |s: u32, cs: i32, cd: i32| match s {
                        0 => cs,
                        1 => cd,
                        _ => 0,
                    };
                    let mix = |cs: i32, cd: i32| ((sel(sa, cs, cd) - sel(sb, cs, cd)) * cf >> 7) + sel(sd, cs, cd);
                    (mix(sr, dr), mix(sg, dg), mix(sb_, db))
                }
            };
            if p.colclamp {
                r = nr.clamp(0, 255) as u32;
                g = ng.clamp(0, 255) as u32;
                b = nb.clamp(0, 255) as u32;
            } else {
                r = (nr & 0xFF) as u32;
                g = (ng & 0xFF) as u32;
                b = (nb & 0xFF) as u32;
            }
        }
        if p.fba {
            al |= 0x80;
        }
        let mut out = r | g << 8 | b << 16 | al << 24;
        if p.fb16 {
            out = (r >> 3) | (g >> 3) << 5 | (b >> 3) << 10 | (al >> 7) << 15;
            fbmask = (fbmask >> 3 & 0x1F) | (fbmask >> 11 & 0x1F) << 5 | (fbmask >> 19 & 0x1F) << 10 | (fbmask >> 31) << 15;
        }
        if fbmask != 0 {
            out = (out & !fbmask) | (dst & fbmask);
        }
        let w = &mut self.vram[fi];
        *w = (*w & !(fm << fsh)) | ((out & fm) << fsh);
    }

    // -------------------------------------------------------- primitives --

    fn tex_coords(&self, v: &Vertex, a: Attr, c: &Ctx) -> (f32, f32) {
        if a.fst {
            (v.u as f32, v.v as f32)
        } else {
            let tw = (1u32 << bits(c.tex0, 26, 4).min(10)) as f32;
            let th = (1u32 << bits(c.tex0, 30, 4).min(10)) as f32;
            let q = if v.q == 0.0 { 1.0 } else { v.q };
            (v.s / q * tw * 16.0, v.t / q * th * 16.0)
        }
    }

    fn point(&mut self, v: &Vertex, a: Attr, c: &Ctx) {
        let p = self.pix_state(a, c);
        let (px, py) = ((v.x + 8) >> 4, (v.y + 8) >> 4);
        self.touch(&p, px.max(0), py.max(0), px.max(0), py.max(0));
        let (u, w) = if a.tme { self.tex_coords(v, a, c) } else { (0.0, 0.0) };
        self.pixel(&p, (v.x + 8) >> 4, (v.y + 8) >> 4, v.z, v.rgba, (u as i32, w as i32), v.f);
    }

    fn line(&mut self, v0: &Vertex, v1: &Vertex, a: Attr, c: &Ctx) {
        let p = self.pix_state(a, c);
        let (x0, y0) = (v0.x as f32 / 16.0, v0.y as f32 / 16.0);
        let (x1, y1) = (v1.x as f32 / 16.0, v1.y as f32 / 16.0);
        let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        let (lx, ly) = (x0.min(x1).floor().max(0.0) as i32, y0.min(y1).floor().max(0.0) as i32);
        self.touch(&p, lx, ly, x0.max(x1).ceil().max(0.0) as i32 + 1, y0.max(y1).ceil().max(0.0) as i32 + 1);
        let (t0, t1) = (self.tex_coords(v0, a, c), self.tex_coords(v1, a, c));
        for i in 0..n {
            let t = i as f32 / n as f32;
            let x = (x0 + (x1 - x0) * t).round() as i32;
            let y = (y0 + (y1 - y0) * t).round() as i32;
            let col = if a.iip { lerp_rgba(v0.rgba, v1.rgba, t) } else { v1.rgba };
            let z = (v0.z as f64 + (v1.z as f64 - v0.z as f64) * t as f64) as u32;
            let tex = ((t0.0 + (t1.0 - t0.0) * t) as i32, (t0.1 + (t1.1 - t0.1) * t) as i32);
            self.pixel(&p, x, y, z, col, tex, v1.f);
        }
    }

    fn sprite(&mut self, v0: &Vertex, v1: &Vertex, a: Attr, c: &Ctx) {
        let (xa, xb) = if v0.x <= v1.x { (v0, v1) } else { (v1, v0) };
        let (ya, yb) = if v0.y <= v1.y { (v0, v1) } else { (v1, v0) };
        let x0 = ((xa.x + 15) >> 4).max(c.scissor[0]);
        let x1 = ((xb.x + 15) >> 4).min(c.scissor[1] + 1);
        let y0 = ((ya.y + 15) >> 4).max(c.scissor[2]);
        let y1 = ((yb.y + 15) >> 4).min(c.scissor[3] + 1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let p = self.pix_state(a, c);
        self.touch(&p, x0, y0, x1 - 1, y1 - 1);
        if !a.tme && self.fill(&p, x0, x1, y0, y1, v1) {
            return;
        }
        let (t0, t1) = (self.tex_coords(v0, a, c), self.tex_coords(v1, a, c));
        let dx = (v1.x - v0.x) as f32;
        let dy = (v1.y - v0.y) as f32;
        // Texture coordinate per pixel step, in 1/16 texels.
        let du = if dx != 0.0 { (t1.0 - t0.0) * 16.0 / dx } else { 0.0 };
        let dv = if dy != 0.0 { (t1.1 - t0.1) * 16.0 / dy } else { 0.0 };
        let u_at = |x: i32| t0.0 + ((x << 4) - v0.x) as f32 * du / 16.0;
        let (z, rgba, fog) = (v1.z, v1.rgba, v1.f);
        let fast_out = Self::simple_out(&p);
        let zc = z.min(p.zmax);
        // A 32/24-bit texture whose span of texels (and their neighbours)
        // stays inside it: the specialised row kernel. The first texel is
        // exact, the last one gets a margin for the rounding along the row.
        let (ua0, ua1) = (u_at(x0), u_at(x1 - 1));
        let (lo, hi) = if du >= 0.0 { (ua0, ua1 + 32.0) } else { (ua1 - 32.0, ua0) };
        let texel = |u: f32| if p.bilinear { (u as i32 - 8) >> 4 } else { u as i32 >> 4 };
        let copy_k = if a.tme
            && matches!(p.tdirect, 1 | 2)
            && !p.fge
            && fast_out
            && lo >= 0.0
            && hi < 32768.0
            && texel(lo) >= 0
            && texel(hi) + 1 < p.tw
        {
            Some(pick_kernel!(&p, copy_k) as CopyKernel)
        } else {
            None
        };
        let flat_k = if !a.tme && !p.fge && fast_out { Some(pick_kernel!(&p, flat_k) as FlatKernel) } else { None };
        for y in y0..y1 {
            if (p.scanmsk == 2 && y & 1 == 0) || (p.scanmsk == 3 && y & 1 == 1) {
                continue;
            }
            let tv = (t0.1 + ((y << 4) - v0.y) as f32 * dv / 16.0) as i32;
            let mut u = u_at(x0);
            let (yu, frow, zrow) = (y as u32, p.fb.row(y as u32), p.zb.row(y as u32));
            self.pixels += (x1 - x0) as u64;
            if let Some(k) = flat_k {
                k(self, &p, yu, x0 as u32, (x1 - x0) as u32, rgba, zc);
                continue;
            }
            if !a.tme {
                let c = if p.fge { self.fog(&p, rgba, fog) } else { rgba };
                for x in x0 as u32..x1 as u32 {
                    self.out(&p, x, yu, frow, zrow, z, c);
                }
                continue;
            }
            if let Some(k) = copy_k {
                // No wrapping along the row (see `copy_k` above).
                let (v0r, fv) = if p.bilinear { ((tv - 8) >> 4, ((tv - 8) & 15) as u32) } else { (tv >> 4, 0) };
                let va = wrap(v0r, p.th, p.wv.0, p.wv.1, p.wv.2) as u32 & 2047;
                let vb = wrap(v0r + 1, p.th, p.wv.0, p.wv.1, p.wv.2) as u32 & 2047;
                k(self, &p, yu, x0 as u32, (x1 - x0) as u32, (u, du), (va, vb, fv), rgba, zc);
                continue;
            }
            if p.tdirect != 0 && !p.fge {
                // Texture rows fixed for the whole span.
                let (v0r, fv) = if p.bilinear { ((tv - 8) >> 4, ((tv - 8) & 15) as u32) } else { (tv >> 4, 0) };
                let va = wrap(v0r, p.th, p.wv.0, p.wv.1, p.wv.2) as u32 & 2047;
                let vb = wrap(v0r + 1, p.th, p.wv.0, p.wv.1, p.wv.2) as u32 & 2047;
                let (ra, rb) = (p.tsw.row(va), p.tsw.row(vb));
                for x in x0 as u32..x1 as u32 {
                    let uu = u as i32;
                    let (u0r, fu) = if p.bilinear { ((uu - 8) >> 4, ((uu - 8) & 15) as u32) } else { (uu >> 4, 0) };
                    let ua = wrap(u0r, p.tw, p.wu.0, p.wu.1, p.wu.2) as u32 & 2047;
                    let ta = self.fetch_direct(&p, ra, ua, va);
                    let t = if fu == 0 && fv == 0 {
                        ta
                    } else {
                        let ub = wrap(u0r + 1, p.tw, p.wu.0, p.wu.1, p.wu.2) as u32 & 2047;
                        let (tb, td) = if fu == 0 { (ta, 0) } else { (self.fetch_direct(&p, ra, ub, va), 1) };
                        let tc = if fv == 0 { ta } else { self.fetch_direct(&p, rb, ua, vb) };
                        let td = if fu == 0 { tc } else if fv == 0 { tb } else { let _ = td; self.fetch_direct(&p, rb, ub, vb) };
                        bilerp(ta, tb, tc, td, fu, fv)
                    };
                    let c = Self::modulate(&p, t, rgba);
                    if fast_out {
                        if let Some(za) = self.ztest_fast(&p, x, yu, zrow, zc) {
                            self.write_fast(&p, x, yu, frow, za, zc, c);
                        }
                    } else {
                        self.out(&p, x, yu, frow, zrow, z, c);
                    }
                    u += du;
                }
                self.stat_tex[0] += (x1 - x0) as u64;
                continue;
            }
            for x in x0 as u32..x1 as u32 {
                let c = self.shade(&p, rgba, (u as i32, tv), fog);
                self.out(&p, x, yu, frow, zrow, z, c);
                u += du;
            }
        }
    }

    /// Untextured row of a sprite (see `flat_kernel`): colour c and depth z
    /// on pixels x0.. of row y.
    fn flat_k<const ZT: u8, const ZW: bool, const BM: u8>(&mut self, p: &Px, y: u32, x0: u32, n: u32, c: u32, z: u32) {
        let (frow, ftab) = (p.fb.row(y), p.fb.tab_row32(y));
        let (zrow, ztab) = (p.zb.row(y), p.zb.tab_row32(y));
        let zmask = p.zb.mask;
        let vram: &mut [u32; WORDS] = self.vram.as_mut_slice().try_into().expect("4 MB of VRAM");
        for x in x0..x0 + n {
            let za = if ZT != 9 { addr32(zrow, ztab, x) } else { 0 };
            if ztest_k::<ZT>(vram, za, z, zmask) {
                write_k::<ZW, BM>(vram, p, addr32(frow, ftab, x), za, z, c);
            }
        }
    }

    /// One row of a sprite textured from a 32/24-bit texture: u (in 1/16
    /// texels, stepping by du, kept inside the texture by the caller) along
    /// texture rows (va, vb, fraction fv), modulated by `rgba`, to pixels
    /// x0.. of row y.
    #[allow(clippy::too_many_arguments)]
    fn copy_k<const ZT: u8, const ZW: bool, const BM: u8>(
        &mut self, p: &Px, y: u32, x0: u32, n: u32, (mut uf, du): (f32, f32), (va, vb, fv): (u32, u32, u32), rgba: u32, z: u32,
    ) {
        let bil = p.bilinear;
        let (frow, ftab) = (p.fb.row(y), p.fb.tab_row32(y));
        let (zrow, ztab) = (p.zb.row(y), p.zb.tab_row32(y));
        let (ra, ta) = (p.tsw.row(va), p.tsw.tab_row32(va));
        let (rb, tb) = (p.tsw.row(vb), p.tsw.tab_row32(vb));
        let zmask = p.zb.mask;
        let ct24 = p.tdirect == 2;
        let (aem, ta0) = (p.aem, p.ta0);
        // Modulating by 0x80 (or decal) keeps the texel's colour.
        let keep = p.tfx == 1 || (p.tfx == 0 && rgba & 0xFF_FFFF == 0x80_8080);
        let vram: &mut [u32; WORDS] = self.vram.as_mut_slice().try_into().expect("4 MB of VRAM");
        let fetch = |vram: &[u32; WORDS], row: u32, tab: &[u16; 64], u: u32| -> u32 {
            let raw = vram[addr32(row, tab, u)];
            if ct24 {
                let raw = raw & 0xFF_FFFF;
                raw | (if aem && raw == 0 { 0 } else { ta0 }) << 24
            } else {
                raw
            }
        };
        for i in 0..n {
            let x = x0 + i;
            let za = if ZT != 9 { addr32(zrow, ztab, x) } else { 0 };
            let uu = uf as i32;
            uf += du;
            if !ztest_k::<ZT>(vram, za, z, zmask) {
                continue;
            }
            let (u, fu) = if bil { (((uu - 8) >> 4) as u32, ((uu - 8) & 15) as u32) } else { ((uu >> 4) as u32, 0) };
            let t00 = fetch(vram, ra, ta, u);
            let t = if fu == 0 && fv == 0 {
                t00
            } else if fu == 0 {
                let t01 = fetch(vram, rb, tb, u);
                bilerp(t00, t00, t01, t01, 0, fv)
            } else {
                let t10 = fetch(vram, ra, ta, u + 1);
                let (t01, t11) = if fv == 0 { (t00, t10) } else { (fetch(vram, rb, tb, u), fetch(vram, rb, tb, u + 1)) };
                bilerp(t00, t10, t01, t11, fu, fv)
            };
            let c = if keep {
                let al = rgba >> 24;
                let al = if p.tcc { if p.tfx == 1 { t >> 24 } else { ((t >> 24) * al >> 7).min(255) } } else { al };
                (t & 0xFF_FFFF) | al << 24
            } else {
                Self::modulate(p, t, rgba)
            };
            write_k::<ZW, BM>(vram, p, addr32(frow, ftab, x), za, z, c);
        }
        self.stat_tex[0] += n as u64;
    }

    /// One span of the common textured triangle (see `fast` in `triangle`),
    /// made for one depth test `ZT`, depth write `ZW` and blend `BM` (as
    /// `Px::bmode`, 4 for none): bilinear from a wholly decoded texture,
    /// colours in 16.16 fixed point, 32-bit frame and depth buffers.
    #[allow(clippy::too_many_arguments)]
    fn tri_span_k<const ZT: u8, const ZW: bool, const BM: u8>(
        &mut self, p: &Px, a: Attr, dec: &[u32], y: u32, x0: u32, n: usize, at: &[f64; 9], step: &[f64; 9], flat: u32, tsz: (f64, f64),
    ) {
        let fx = |v: f64| f2i(v * 65536.0);
        let (mut cr, mut cg, mut cb, mut ca) = (fx(at[2]), fx(at[3]), fx(at[4]), fx(at[5]));
        let (dr, dg, db, da) = (fx(step[2]), fx(step[3]), fx(step[4]), fx(step[5]));
        let (mut z, mut s, mut t, mut q) = (at[0], at[6], at[7], at[8]);
        let (dz, ds, dt, dq) = (step[0], step[6], step[7], step[8]);
        let twl = p.tw.trailing_zeros();
        let (tw, th) = (p.tw, p.th);
        let (wu, wv) = (p.wu.0, p.wv.0);
        let wrapc = |c: i32, size: i32, mode: u32| if mode == 0 { c & (size - 1) } else { c.clamp(0, size - 1) };
        let ch = |v: i32| (v >> 16).clamp(0, 255) as u32;
        // Both buffers have one pixel per word (64x32 pages).
        let (frow, ftab) = (p.fb.row(y), p.fb.tab_row32(y));
        let (zrow, ztab) = (p.zb.row(y), p.zb.tab_row32(y));
        let (zmask, zmax) = (p.zb.mask, p.zmax);
        let vram: &mut [u32; WORDS] = self.vram.as_mut_slice().try_into().expect("4 MB of VRAM");
        let dec_at = |i: usize| -> u32 {
            // SAFETY: the wrapped coordinates are inside the texture, whose
            // decoded size is tw * th (checked in `triangle`).
            unsafe { *dec.get_unchecked(i) }
        };
        let (iip, fst) = (a.iip, a.fst);
        // Affine mapping (q the same all along, as for sprites drawn as
        // triangles): one division for the span.
        let r_span = if q == 0.0 { 1.0 } else { 1.0 / q };
        let affine = dq == 0.0;
        for i in 0..n as u32 {
            let x = x0 + i;
            let (mut zi, mut za) = (0, 0);
            if ZT != 9 {
                zi = f2u(z).min(zmax);
                za = addr32(zrow, ztab, x);
            }
            if ztest_k::<ZT>(vram, za, zi, zmask) {
                let (su, sv) = if fst {
                    (f2i(s), f2i(t))
                } else {
                    let r = if affine { r_span } else { 1.0 / if q == 0.0 { 1.0 } else { q } };
                    (f2i(s * r * tsz.0), f2i(t * r * tsz.1))
                };
                let (u, v) = (su - 8, sv - 8);
                let (u0, v0) = (u >> 4, v >> 4);
                let (ua, ub) = (wrapc(u0, tw, wu) as usize, wrapc(u0 + 1, tw, wu) as usize);
                let (ra, rb) = ((wrapc(v0, th, wv) << twl) as usize, (wrapc(v0 + 1, th, wv) << twl) as usize);
                let tex = bilerp(dec_at(ra | ua), dec_at(ra | ub), dec_at(rb | ua), dec_at(rb | ub), (u & 15) as u32, (v & 15) as u32);
                let col = if iip { ch(cr) | ch(cg) << 8 | ch(cb) << 16 | ch(ca) << 24 } else { flat };
                let c = Self::modulate(p, tex, col);
                write_k::<ZW, BM>(vram, p, addr32(frow, ftab, x), za, zi, c);
            }
            z += dz;
            q += dq;
            s += ds;
            t += dt;
            cr = cr.wrapping_add(dr);
            cg = cg.wrapping_add(dg);
            cb = cb.wrapping_add(db);
            ca = ca.wrapping_add(da);
        }
    }

    /// Z test for the fast paths: None if the pixel fails, else where its
    /// depth goes (word, shift, mask; mask 0 when Z isn't written).
    #[inline(always)]
    fn ztest_fast(&self, p: &Px, x: u32, y: u32, zrow: u32, z: u32) -> Option<(usize, u32, u32)> {
        if !p.zte || (p.ztst == 1 && p.zmsk) {
            return Some((0, 0, 0));
        }
        let (zi, zsh, zm) = p.zb.word(zrow.wrapping_add(p.zb.col(x, y)));
        if p.ztst != 1 {
            let dz = (self.vram[zi] >> zsh) & zm;
            let pass = match p.ztst {
                0 => false,
                2 => z >= dz,
                _ => z > dz,
            };
            if !pass {
                return None;
            }
        }
        Some((zi, zsh, if p.zmsk { 0 } else { zm }))
    }

    /// Z write, blending and the frame buffer write for the fast paths (no
    /// alpha or destination alpha test, no FBMSK, a 32/24-bit frame buffer).
    #[inline(always)]
    fn write_fast(&mut self, p: &Px, x: u32, y: u32, frow: u32, za: (usize, u32, u32), z: u32, c: u32) {
        if za.2 != 0 {
            let w = &mut self.vram[za.0];
            *w = (*w & !(za.2 << za.1)) | ((z & za.2) << za.1);
        }
        let (fi, _, fm) = p.fb.word(frow.wrapping_add(p.fb.col(x, y)));
        let dst = self.vram[fi];
        let mut al = c >> 24;
        let mut rgb = c & 0xFF_FFFF;
        if p.abe && !(p.pabe && al & 0x80 == 0) {
            let d = if p.fb24 { dst | 0x8000_0000 } else { dst };
            let cf = match p.blend[2] {
                0 => al,
                1 => d >> 24,
                _ => p.fix,
            } as i32;
            let (sr, sg, sb) = ((c & 0xFF) as i32, (c >> 8 & 0xFF) as i32, (c >> 16 & 0xFF) as i32);
            let (er, eg, eb) = ((d & 0xFF) as i32, (d >> 8 & 0xFF) as i32, (d >> 16 & 0xFF) as i32);
            let (nr, ng, nb) = match p.bmode {
                1 => ((sr * cf >> 7) + er, (sg * cf >> 7) + eg, (sb * cf >> 7) + eb),
                2 => (((sr - er) * cf >> 7) + er, ((sg - eg) * cf >> 7) + eg, ((sb - eb) * cf >> 7) + eb),
                3 => ((-sr * cf >> 7) + er, (-sg * cf >> 7) + eg, (-sb * cf >> 7) + eb),
                _ => {
                    let [sa, sbs, _, sd] = p.blend;
                    let sel = |k: u32, cs: i32, cd: i32| match k {
                        0 => cs,
                        1 => cd,
                        _ => 0,
                    };
                    let mix = |cs: i32, cd: i32| ((sel(sa, cs, cd) - sel(sbs, cs, cd)) * cf >> 7) + sel(sd, cs, cd);
                    (mix(sr, er), mix(sg, eg), mix(sb, eb))
                }
            };
            rgb = if p.colclamp {
                nr.clamp(0, 255) as u32 | (ng.clamp(0, 255) as u32) << 8 | (nb.clamp(0, 255) as u32) << 16
            } else {
                (nr & 0xFF) as u32 | ((ng & 0xFF) as u32) << 8 | ((nb & 0xFF) as u32) << 16
            };
        }
        if p.fba {
            al |= 0x80;
        }
        self.vram[fi] = (dst & !fm) | ((rgb | al << 24) & fm);
    }

    /// The fast paths' conditions on the tests and buffers.
    fn simple_out(p: &Px) -> bool {
        !p.ate && !p.date && p.fbmsk == 0 && p.fb.whole_word() && p.fb.shift == 0 && (!p.zte || p.zb.whole_word()) && p.scanmsk < 2
    }

    /// Untextured sprite that writes its colour (and Z) unconditionally:
    /// straight stores. Returns false when the state needs the full pipeline.
    fn fill(&mut self, p: &Px, x0: i32, x1: i32, y0: i32, y1: i32, v: &Vertex) -> bool {
        let simple = !p.fge
            && !(p.abe && !(p.pabe && (v.rgba >> 31) == 0))
            && (!p.ate || p.atst == 1)
            && !p.date
            && (!p.zte || p.ztst == 1)
            && p.fbmsk == 0
            && p.fb.whole_word()
            && p.fb.shift == 0
            && p.scanmsk < 2;
        if !simple {
            return false;
        }
        let mut col = v.rgba;
        if p.fba {
            col |= 0x8000_0000;
        }
        let fm = p.fb.mask;
        let wz = p.zte && !p.zmsk;
        let z = v.z.min(p.zmax);
        for y in y0..y1 {
            let (y, frow) = (y as u32, p.fb.row(y as u32));
            for x in x0 as u32..x1 as u32 {
                let (i, _, _) = p.fb.word(frow.wrapping_add(p.fb.col(x, y)));
                let w = &mut self.vram[i];
                *w = (*w & !fm) | (col & fm);
            }
            if wz {
                let zrow = p.zb.row(y);
                for x in x0 as u32..x1 as u32 {
                    let (i, sh, m) = p.zb.word(zrow.wrapping_add(p.zb.col(x, y)));
                    let w = &mut self.vram[i];
                    *w = (*w & !(m << sh)) | ((z & m) << sh);
                }
            }
        }
        self.pixels += ((x1 - x0) * (y1 - y0)) as u64;
        true
    }

    fn triangle(&mut self, v0: &Vertex, v1: &Vertex, v2: &Vertex, a: Attr, c: &Ctx) {
        let (ax, ay) = (v0.x as i64, v0.y as i64);
        let (bx, by) = (v1.x as i64, v1.y as i64);
        let (cx, cy) = (v2.x as i64, v2.y as i64);
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area == 0 {
            return;
        }
        let minx = ((ax.min(bx).min(cx) + 15) >> 4).max(c.scissor[0] as i64);
        let maxx = (ax.max(bx).max(cx) >> 4).min(c.scissor[1] as i64);
        let miny = ((ay.min(by).min(cy) + 15) >> 4).max(c.scissor[2] as i64);
        let maxy = (ay.max(by).max(cy) >> 4).min(c.scissor[3] as i64);
        if minx > maxx || miny > maxy {
            return;
        }
        let p = self.pix_state(a, c);
        self.touch(&p, minx as i32, miny as i32, maxx as i32, maxy as i32);
        let sign = if area > 0 { 1 } else { -1 };
        let inv = 1.0 / area as f64 * sign as f64;
        let (tw, th) = ((1u32 << bits(c.tex0, 26, 4).min(10)) as f64 * 16.0, (1u32 << bits(c.tex0, 30, 4).min(10)) as f64 * 16.0);
        let bias = |x0: i64, y0: i64, x1: i64, y1: i64| -> i64 {
            let (dx, dy) = (x1 - x0, y1 - y0);
            let top_left = if sign > 0 { dy < 0 || (dy == 0 && dx > 0) } else { dy > 0 || (dy == 0 && dx < 0) };
            if top_left { 0 } else { -1 }
        };
        let b = [bias(bx, by, cx, cy), bias(cx, cy, ax, ay), bias(ax, ay, bx, by)];
        // Edge functions step by these per pixel (16 subpixels).
        let e = [-(cy - by) * 16 * sign, -(ay - cy) * 16 * sign, -(by - ay) * 16 * sign];
        // Attributes: z, fog, r, g, b, a, s, t, q. Each is a plane
        // base + l0 * d0 + l1 * d1 over the barycentrics (l2 = 1 - l0 - l1).
        const N: usize = 9;
        let vals = |v: &Vertex| -> [f64; N] {
            let (s, t, q) = if a.fst { (v.u as f64, v.v as f64, 1.0) } else { (v.s as f64, v.t as f64, v.q as f64) };
            let ch = |sh: u32| ((v.rgba >> sh) & 0xFF) as f64;
            [v.z as f64, v.f as f64, ch(0), ch(8), ch(16), ch(24), s, t, q]
        };
        let (x0, x1, x2) = (vals(v0), vals(v1), vals(v2));
        let mut base = [0.0; N];
        let mut d0 = [0.0; N];
        let mut d1 = [0.0; N];
        let mut step = [0.0; N];
        for k in 0..N {
            base[k] = x2[k];
            d0[k] = x0[k] - x2[k];
            d1[k] = x1[k] - x2[k];
            step[k] = (e[0] as f64 * inv) * d0[k] + (e[1] as f64 * inv) * d1[k];
        }
        let wmax = maxx - minx;
        // A wholly decoded texture is read from here while drawing.
        let dec = if a.tme && p.tdirect == 3 && self.tex_cur < self.texcache.len() {
            std::mem::take(&mut self.texcache[self.tex_cur].data)
        } else {
            Vec::new()
        };
        // The common case: bilinear from a small (wholly decoded) texture,
        // simple tests, a 32-bit frame buffer and 32/24-bit Z.
        let fast = a.tme
            && !dec.is_empty()
            && p.bilinear
            && !p.fge
            && dec.len() == (p.tw * p.th) as usize
            && p.wu.0 < 2
            && p.wv.0 < 2
            && Self::simple_out(&p);
        let kernel = if fast { Some(pick_kernel!(&p, tri_span_k) as TriKernel) } else { None };
        // Gouraud shading of one colour is flat shading.
        let ka = Attr { iip: a.iip && !(v0.rgba == v1.rgba && v1.rgba == v2.rgba), ..a };
        let mut span_c = std::mem::take(&mut self.span_c);
        let mut span_z = std::mem::take(&mut self.span_z);
        span_c.resize(2048, 0);
        span_z.resize(2048, 0);
        for py in miny..=maxy {
            let y = py << 4;
            let x = minx << 4;
            let w = [
                ((cx - bx) * (y - by) - (cy - by) * (x - bx)) * sign,
                ((ax - cx) * (y - cy) - (ay - cy) * (x - cx)) * sign,
                ((bx - ax) * (y - ay) - (by - ay) * (x - ax)) * sign,
            ];
            // The span where all three edges pass: k in [k0, k1].
            let (mut k0, mut k1) = (0i64, wmax);
            for i in 0..3 {
                let v = w[i] + b[i];
                let ei = e[i];
                if ei > 0 {
                    if v < 0 {
                        k0 = k0.max((-v + ei - 1) / ei);
                    }
                } else if ei < 0 {
                    if v < 0 {
                        k1 = -1;
                    } else {
                        k1 = k1.min(v / -ei);
                    }
                } else if v < 0 {
                    k1 = -1;
                }
            }
            if k0 > k1 {
                continue;
            }
            let l0 = (w[0] + k0 * e[0]) as f64 * inv;
            let l1 = (w[1] + k0 * e[1]) as f64 * inv;
            let mut at = [0.0; N];
            for k in 0..N {
                at[k] = base[k] + l0 * d0[k] + l1 * d1[k];
            }
            let (yu, frow, zrow) = (py as u32, p.fb.row(py as u32), p.zb.row(py as u32));
            if (p.scanmsk == 2 && py & 1 == 0) || (p.scanmsk == 3 && py & 1 == 1) {
                continue;
            }
            // Colours and depths of the span first, then the tests and writes.
            let n = (k1 - k0 + 1) as usize;
            if let Some(kernel) = kernel {
                kernel(self, &p, ka, &dec, py as u32, (minx + k0) as u32, n, &at, &step, v2.rgba, (tw, th));
                self.pixels += n as u64;
                continue;
            }
            for i in 0..n {
                let z = f2u(at[0]);
                let fog = if p.fge { f2u(at[1]).min(255) } else { 0 };
                let col = if a.iip {
                    f2c(at[2]) | f2c(at[3]) << 8 | f2c(at[4]) << 16 | f2c(at[5]) << 24
                } else {
                    v2.rgba
                };
                let tex = if !a.tme {
                    (0, 0)
                } else if a.fst {
                    (f2i(at[6]), f2i(at[7]))
                } else {
                    let q = if at[8] == 0.0 { 1.0 } else { at[8] };
                    let r = 1.0 / q;
                    (f2i(at[6] * r * tw), f2i(at[7] * r * th))
                };
                let c = if a.tme && !dec.is_empty() {
                    let t = self.sample_dec(&p, &dec, tex.0, tex.1);
                    let c = Self::modulate(&p, t, col);
                    if p.fge { self.fog(&p, c, fog) } else { c }
                } else {
                    self.shade(&p, col, tex, fog)
                };
                span_c[i] = c;
                span_z[i] = z;
                for k in 0..N {
                    at[k] += step[k];
                }
            }
            let x0 = (minx + k0) as u32;
            for i in 0..n {
                self.out(&p, x0 + i as u32, yu, frow, zrow, span_z[i], span_c[i]);
            }
            self.pixels += n as u64;
        }
        if !dec.is_empty() {
            self.texcache[self.tex_cur].data = dec;
        }
        self.span_c = span_c;
        self.span_z = span_z;
    }

    // ------------------------------------------------------------- CLUT --

    fn clut_load(&mut self, tex0: u64, ctx: usize) {
        let psm = bits(tex0, 20, 6);
        if !matches!(psm, PSMT8 | PSMT8H | PSMT4 | PSMT4HL | PSMT4HH) {
            return;
        }
        let cbp = bits(tex0, 37, 14);
        let cld = bits(tex0, 61, 3);
        let load = match cld {
            0 => false,
            1 => true,
            2 => {
                self.cbp[0] = cbp;
                true
            }
            3 => {
                self.cbp[1] = cbp;
                true
            }
            4 => {
                let l = self.cbp[0] != cbp;
                self.cbp[0] = cbp;
                l
            }
            5 => {
                let l = self.cbp[1] != cbp;
                self.cbp[1] = cbp;
                l
            }
            _ => false,
        };
        let _ = ctx;
        if !load {
            return;
        }
        self.clut_gen = self.clut_gen.wrapping_add(1);
        let cpsm = bits(tex0, 51, 4);
        let csm = bits(tex0, 55, 1);
        let csa = bits(tex0, 56, 5) as usize;
        let eight = matches!(psm, PSMT8 | PSMT8H);
        let n = if eight { 256 } else { 16 };
        let texclut = self.regs[0x1C];
        for i in 0..n {
            let (x, y) = if csm == 1 {
                (bits(texclut, 6, 6) * 16 + i as u32, bits(texclut, 12, 10))
            } else if eight {
                let j = (i & 0xE7) | ((i & 0x08) << 1) | ((i & 0x10) >> 1);
                ((j & 15) as u32, (j >> 4) as u32)
            } else {
                ((i & 7) as u32, (i >> 3) as u32)
            };
            let bw = if csm == 1 { bits(texclut, 0, 6) } else { 1 };
            if cpsm == 0 {
                let c = gsmem::read(&self.vram, PSMCT32, cbp, bw, x, y);
                let slot = if eight { i } else { (csa & 15) * 16 + i };
                self.clut[slot & 255] = c as u16;
                self.clut[(slot & 255) + 256] = (c >> 16) as u16;
            } else {
                let c = gsmem::read(&self.vram, if cpsm == 2 { PSMCT16 } else { PSMCT16S }, cbp, bw, x, y);
                let slot = if eight { i } else { csa * 16 + i };
                self.clut[slot & 511] = c as u16;
            }
        }
    }

    // --------------------------------------------------------- transfers --

    fn start_transfer(&mut self) {
        let dir = self.regs[0x53] & 3;
        let pos = self.regs[0x51];
        let reg = self.regs[0x52];
        let buf = self.regs[0x50];
        match dir {
            0 => {
                self.xfer = Transfer { active: true, x: 0, y: 0, bits: 0, nbits: 0 };
            }
            2 => {
                // Local → local copy.
                let (sbp, sbw, spsm) = (bits(buf, 0, 14), bits(buf, 16, 6), bits(buf, 24, 6));
                let (dbp, dbw, dpsm) = (bits(buf, 32, 14), bits(buf, 48, 6), bits(buf, 56, 6));
                let (sx, sy, dx, dy) = (bits(pos, 0, 11), bits(pos, 16, 11), bits(pos, 32, 11), bits(pos, 48, 11));
                let (w, h) = (bits(reg, 0, 12), bits(reg, 32, 12));
                let mut tmp = Vec::with_capacity((w * h) as usize);
                for y in 0..h {
                    for x in 0..w {
                        tmp.push(gsmem::read(&self.vram, spsm, sbp, sbw, (sx + x) & 2047, (sy + y) & 2047));
                    }
                }
                let mut i = 0;
                for y in 0..h {
                    for x in 0..w {
                        self.put(dpsm, dbp, dbw, (dx + x) & 2047, (dy + y) & 2047, tmp[i]);
                        i += 1;
                    }
                }
            }
            _ => self.xfer.active = false,
        }
    }

    /// 64 bits of image data for a host → local transfer.
    pub fn hwreg(&mut self, v: u64) {
        if !self.xfer.active {
            return;
        }
        let buf = self.regs[0x50];
        let (dbp, dbw, dpsm) = (bits(buf, 32, 14), bits(buf, 48, 6), bits(buf, 56, 6));
        let pos = self.regs[0x51];
        let reg = self.regs[0x52];
        let (dx, dy) = (bits(pos, 32, 11), bits(pos, 48, 11));
        let (w, h) = (bits(reg, 0, 12).max(1), bits(reg, 32, 12));
        let bpp = bpp(dpsm);
        self.xfer.bits |= (v as u128) << self.xfer.nbits;
        self.xfer.nbits += 64;
        while self.xfer.nbits >= bpp {
            let t = &mut self.xfer;
            let px = (t.bits & ((1u128 << bpp) - 1)) as u32;
            t.bits >>= bpp;
            t.nbits -= bpp;
            if t.y >= h {
                t.active = false;
                return;
            }
            let (x, y) = ((dx + t.x) & 2047, (dy + t.y) & 2047);
            self.put(dpsm, dbp, dbw, x, y, px);
            let t = &mut self.xfer;
            t.x += 1;
            if t.x >= w {
                t.x = 0;
                t.y += 1;
                if t.y >= h {
                    t.active = false;
                    return;
                }
            }
        }
    }

    // ----------------------------------------------------------- display --

    pub fn display(&self, pmode: u64, smode2: u64, dispfb: &[u64; 2], display: &[u64; 2], bg: u64, out: &mut Vec<u8>) -> (u32, u32) {
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
        let w = w.min(1024);
        let h = h.min(1024);
        out.clear();
        out.resize((w * h * 4) as usize, 0);
        let read = |i: usize, x: u32, y: u32| -> u32 {
            let f = dispfb[i];
            let fbp = bits(f, 0, 9) * 32;
            let fbw = bits(f, 9, 6);
            let psm = bits(f, 15, 5);
            let (dbx, dby) = (bits(f, 32, 11), bits(f, 43, 11));
            let p = gsmem::read(&self.vram, psm, fbp, fbw, (x + dbx) & 2047, (y + dby) & 2047);
            if bpp(psm) == 16 { c16_to_32(p, 0x80 | 0x80 << 32) } else { p }
        };
        let alp = bits(pmode, 8, 8);
        let mmod = pmode >> 5 & 1 != 0;
        // Interlaced field mode: the buffer holds one field, each line shown twice.
        let field = smode2 & 3 == 3;
        for y in 0..h {
            let oy = y;
            let fy = if field { y >> 1 } else { y };
            for x in 0..w {
                let y = fy;
                let mut c = bg as u32 & 0xFF_FFFF;
                if en[1] {
                    c = read(1, x, y);
                }
                if en[0] {
                    let c1 = read(0, x, y);
                    let a = if mmod { alp } else { (c1 >> 24).min(128) * 2 }.min(255);
                    if en[1] {
                        let mix = |s: u32| (((c1 >> s) & 0xFF) * a + ((c >> s) & 0xFF) * (255 - a)) / 255;
                        c = mix(0) | mix(8) << 8 | mix(16) << 16;
                    } else {
                        c = c1;
                    }
                }
                let o = ((oy * w + x) * 4) as usize;
                out[o] = c as u8;
                out[o + 1] = (c >> 8) as u8;
                out[o + 2] = (c >> 16) as u8;
                out[o + 3] = 255;
            }
        }
        (w, h)
    }
}

/// Float to integer conversions for the rasterizer, truncating like `as`
/// but without its saturation checks; NaN (from corrupt data) becomes the
/// low bound, since f64::max/min return the number.
#[inline(always)]
fn f2i(x: f64) -> i32 {
    // SAFETY: the value is finite and within i32 after the clamp.
    unsafe { x.max(-1.0e9).min(1.0e9).to_int_unchecked::<i32>() }
}

#[inline(always)]
fn f2u(x: f64) -> u32 {
    // SAFETY: as above, within u32.
    unsafe { x.max(0.0).min(4294967295.0).to_int_unchecked::<u32>() }
}

/// A colour channel: clamped to 0..=255.
#[inline(always)]
fn f2c(x: f64) -> u32 {
    // SAFETY: as above.
    unsafe { x.max(0.0).min(255.0).to_int_unchecked::<u32>() }
}

/// Depth test of the specialised kernels (`ZT` as in `Draw::tri_kernel`).
#[inline(always)]
fn ztest_k<const ZT: u8>(vram: &[u32; WORDS], za: usize, zi: u32, zmask: u32) -> bool {
    match ZT {
        0 => false,
        2 => zi >= vram[za] & zmask,
        3 => zi > vram[za] & zmask,
        _ => true,
    }
}

/// Depth write, blending (`BM` as `Px::bmode`, 4 for none) and the frame
/// buffer write of the specialised kernels: no alpha or destination alpha
/// test, no FBMSK, one pixel per word.
#[inline(always)]
fn write_k<const ZW: bool, const BM: u8>(vram: &mut [u32; WORDS], p: &Px, fi: usize, za: usize, zi: u32, c: u32) {
    if ZW {
        let m = p.zb.mask;
        vram[za] = (vram[za] & !m) | (zi & m);
    }
    let fmask = p.fb.mask;
    let dst = vram[fi];
    let al = c >> 24;
    let mut rgb = c & 0xFF_FFFF;
    if BM != 4 && !(p.pabe && al & 0x80 == 0) {
        if BM == 1 && rgb == 0 {
            // Adding nothing (the dark parts of glows).
            rgb = dst & 0xFF_FFFF;
        } else {
            let d = dst | if p.fb24 { 0x8000_0000 } else { 0 };
            let cf = match p.blend[2] {
                0 => al,
                1 => d >> 24,
                _ => p.fix,
            } as i32;
            let (sr, sg, sb) = ((c & 0xFF) as i32, (c >> 8 & 0xFF) as i32, (c >> 16 & 0xFF) as i32);
            let (er, eg, eb) = ((d & 0xFF) as i32, (d >> 8 & 0xFF) as i32, (d >> 16 & 0xFF) as i32);
            let (nr, ng, nb) = match BM {
                1 => ((sr * cf >> 7) + er, (sg * cf >> 7) + eg, (sb * cf >> 7) + eb),
                2 => (((sr - er) * cf >> 7) + er, ((sg - eg) * cf >> 7) + eg, ((sb - eb) * cf >> 7) + eb),
                _ => {
                    let [sa, sbs, _, sd] = p.blend;
                    let sel = |k: u32, cs: i32, cd: i32| match k {
                        0 => cs,
                        1 => cd,
                        _ => 0,
                    };
                    let mix = |cs: i32, cd: i32| ((sel(sa, cs, cd) - sel(sbs, cs, cd)) * cf >> 7) + sel(sd, cs, cd);
                    (mix(sr, er), mix(sg, eg), mix(sb, eb))
                }
            };
            rgb = if p.colclamp {
                nr.clamp(0, 255) as u32 | (ng.clamp(0, 255) as u32) << 8 | (nb.clamp(0, 255) as u32) << 16
            } else {
                (nr & 0xFF) as u32 | ((ng & 0xFF) as u32) << 8 | ((nb & 0xFF) as u32) << 16
            };
        }
    }
    let fba = if p.fba { 0x8000_0000 } else { 0 };
    vram[fi] = (dst & !fmask) | ((rgb | al << 24 | fba) & fmask);
}

/// Word of pixel x in a row of a one-pixel-per-word buffer (see
/// `Swz::tab_row32`).
#[inline(always)]
fn addr32(row: u32, tab: &[u16; 64], x: u32) -> usize {
    (row.wrapping_add((x >> 6) << 11).wrapping_add(tab[(x & 63) as usize] as u32)) as usize & (WORDS - 1)
}

/// Bilinear blend of four texels by 1/16 fractions (two channels at a
/// time in 0x00FF00FF lanes).
#[inline(always)]
fn bilerp(a: u32, b: u32, cc: u32, d: u32, fu: u32, fv: u32) -> u32 {
    if a == b && a == cc && a == d {
        return a;
    }
    let w00 = ((16 - fu) * (16 - fv)) as u64;
    let w10 = (fu * (16 - fv)) as u64;
    let w01 = ((16 - fu) * fv) as u64;
    let w11 = (fu * fv) as u64;
    // Channels in 16-bit lanes: weights sum to 256, so no lane overflows.
    let x = |p: u32| {
        let p = p as u64;
        let p = (p | p << 16) & 0x0000_FFFF_0000_FFFF;
        (p | p << 8) & 0x00FF_00FF_00FF_00FF
    };
    let s = ((x(a) * w00 + x(b) * w10 + x(cc) * w01 + x(d) * w11) >> 8) & 0x00FF_00FF_00FF_00FF;
    let s = (s | s >> 8) & 0x0000_FFFF_0000_FFFF;
    (s | s >> 16) as u32
}

#[inline(always)]
fn wrap(c: i32, size: i32, mode: u32, min: i32, max: i32) -> i32 {
    match mode {
        0 => c & (size - 1),
        1 => c.clamp(0, size - 1),
        2 => c.clamp(min, max),
        _ => (c & min) | max,
    }
}

fn c16_to_32(p: u32, texa: u64) -> u32 {
    let r = (p & 0x1F) << 3;
    let g = ((p >> 5) & 0x1F) << 3;
    let b = ((p >> 10) & 0x1F) << 3;
    let aem = texa >> 15 & 1 != 0;
    let a = if p & 0x8000 != 0 {
        bits(texa, 32, 8)
    } else if aem && p & 0x7FFF == 0 {
        0
    } else {
        bits(texa, 0, 8)
    };
    r | g << 8 | b << 16 | a << 24
}

fn lerp_rgba(a: u32, b: u32, t: f32) -> u32 {
    let mut o = 0;
    for sh in [0, 8, 16, 24] {
        let x = ((a >> sh) & 0xFF) as f32;
        let y = ((b >> sh) & 0xFF) as f32;
        o |= ((x + (y - x) * t) as u32 & 0xFF) << sh;
    }
    o
}

