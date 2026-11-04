//! GS drawing: register state, the software rasterizer (points, lines,
//! triangles, sprites with textures, CLUTs, alpha/depth tests and
//! blending), host ↔ local memory transfers and the display read-out.

use crate::gsmem::{self, *};

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
    queue: [Vertex; 3],
    count: usize,
    /// Triangle fan anchor.
    fan0: Vertex,
    st: (f32, f32),
    clut: [u16; 512],
    cbp: [u32; 2],
    xfer: Transfer,
}

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

#[derive(Clone, Copy)]
struct Attr {
    iip: bool,
    tme: bool,
    fge: bool,
    abe: bool,
    fst: bool,
    ctxt: usize,
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
            queue: [Vertex::default(); 3],
            count: 0,
            fan0: Vertex::default(),
            st: (0.0, 0.0),
            clut: [0; 512],
            cbp: [0; 2],
            xfer: Transfer::default(),
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
        if kick {
            self.prims += 1;
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

    #[inline]
    fn tex_color(&self, c: &Ctx, mut u: i32, mut v: i32) -> u32 {
        let t = c.tex0;
        let tbp = bits(t, 0, 14);
        let tbw = bits(t, 14, 6);
        let psm = bits(t, 20, 6);
        let tw = 1i32 << bits(t, 26, 4).min(10);
        let th = 1i32 << bits(t, 30, 4).min(10);
        let cl = c.clamp;
        u = wrap(u, tw, bits(cl, 0, 2), bits(cl, 4, 10) as i32, bits(cl, 14, 10) as i32);
        v = wrap(v, th, bits(cl, 2, 2), bits(cl, 24, 10) as i32, bits(cl, 34, 10) as i32);
        let raw = gsmem::read(&self.vram, psm, tbp, tbw, u as u32, v as u32);
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

    /// Texel for (u, v) in 1/16 texel units, nearest or bilinear.
    fn sample(&self, c: &Ctx, u16x: i32, v16x: i32) -> u32 {
        let bilinear = c.tex1 >> 5 & 1 != 0;
        if !bilinear {
            return self.tex_color(c, u16x >> 4, v16x >> 4);
        }
        let u = u16x - 8;
        let v = v16x - 8;
        let (u0, v0) = (u >> 4, v >> 4);
        let (fu, fv) = ((u & 15) as u32, (v & 15) as u32);
        let a = self.tex_color(c, u0, v0);
        let b = self.tex_color(c, u0 + 1, v0);
        let cc = self.tex_color(c, u0, v0 + 1);
        let d = self.tex_color(c, u0 + 1, v0 + 1);
        let mut out = 0;
        for sh in [0, 8, 16, 24] {
            let ch = |p: u32| (p >> sh) & 0xFF;
            let top = ch(a) * (16 - fu) + ch(b) * fu;
            let bot = ch(cc) * (16 - fu) + ch(d) * fu;
            let val = (top * (16 - fv) + bot * fv) >> 8;
            out |= val.min(255) << sh;
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn pixel(&mut self, x: i32, y: i32, z: u32, rgba: u32, tex: Option<(i32, i32)>, fog: u32, a: Attr, c: &Ctx) {
        if x < c.scissor[0] || x > c.scissor[1] || y < c.scissor[2] || y > c.scissor[3] {
            return;
        }
        let msk = bits(self.regs[0x22], 0, 2);
        if (msk == 2 && y & 1 == 0) || (msk == 3 && y & 1 == 1) {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        let mut r = rgba & 0xFF;
        let mut g = (rgba >> 8) & 0xFF;
        let mut b = (rgba >> 16) & 0xFF;
        let mut al = (rgba >> 24) & 0xFF;
        if let Some((u, v)) = tex {
            let t = self.sample(c, u, v);
            let (tr, tg, tb, ta) = (t & 0xFF, (t >> 8) & 0xFF, (t >> 16) & 0xFF, t >> 24);
            let tcc = c.tex0 >> 34 & 1 != 0;
            match bits(c.tex0, 35, 2) {
                0 => {
                    r = (tr * r) >> 7;
                    g = (tg * g) >> 7;
                    b = (tb * b) >> 7;
                    if tcc {
                        al = (ta * al) >> 7;
                    }
                }
                1 => {
                    r = tr;
                    g = tg;
                    b = tb;
                    if tcc {
                        al = ta;
                    }
                }
                m => {
                    r = ((tr * r) >> 7) + al;
                    g = ((tg * g) >> 7) + al;
                    b = ((tb * b) >> 7) + al;
                    if tcc {
                        al = if m == 2 { ta + al } else { ta };
                    }
                }
            }
            r = r.min(255);
            g = g.min(255);
            b = b.min(255);
            al = al.min(255);
        }
        if a.fge {
            let fc = self.regs[0x3D];
            let f = fog;
            r = (f * r + (255 - f) * bits(fc, 0, 8)) >> 8;
            g = (f * g + (255 - f) * bits(fc, 8, 8)) >> 8;
            b = (f * b + (255 - f) * bits(fc, 16, 8)) >> 8;
        }
        // Alpha test.
        let test = c.test;
        let mut write_fb = true;
        let mut write_z = !c.zmsk;
        let mut fbmask = c.fbmsk;
        if test & 1 != 0 {
            let aref = bits(test, 4, 8);
            let pass = match bits(test, 1, 3) {
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
                match bits(test, 12, 2) {
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
        let fpsm = c.fpsm;
        let dst = gsmem::read(&self.vram, fpsm, c.fbp, c.fbw, x, y);
        // Destination alpha test.
        if test >> 14 & 1 != 0 {
            let bit = if bpp(fpsm) == 16 { dst >> 15 & 1 } else { dst >> 31 & 1 };
            if bit != (test >> 15 & 1) as u32 {
                return;
            }
        }
        // Depth test.
        if test >> 16 & 1 != 0 {
            let zpsm = c.zpsm;
            let zmax = match bpp(zpsm) {
                16 => 0xFFFF,
                24 => 0xFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
            let z = z.min(zmax);
            let zt = bits(test, 17, 2);
            if zt != 1 {
                let dz = gsmem::read(&self.vram, zpsm, c.zbp, c.fbw, x, y);
                let pass = match zt {
                    0 => false,
                    2 => z >= dz,
                    _ => z > dz,
                };
                if !pass {
                    return;
                }
            }
            if write_z {
                gsmem::write(&mut self.vram, zpsm, c.zbp, c.fbw, x, y, z);
            }
        }
        if !write_fb {
            return;
        }
        let d = if bpp(fpsm) == 16 { c16_to_32(dst, 0x80 | 0x80 << 32) } else if fpsm == PSMCT24 { dst | 0x8000_0000 } else { dst };
        let (dr, dg, db, da) = (d & 0xFF, (d >> 8) & 0xFF, (d >> 16) & 0xFF, d >> 24);
        if a.abe && !(self.regs[0x49] & 1 != 0 && al & 0x80 == 0) {
            let al_ = c.alpha;
            let sel = |s: u32, cs: u32, cd: u32| match s {
                0 => cs as i32,
                1 => cd as i32,
                _ => 0,
            };
            let cf = match bits(al_, 4, 2) {
                0 => al,
                1 => da,
                _ => bits(al_, 32, 8),
            } as i32;
            let (sa, sb, sd) = (bits(al_, 0, 2), bits(al_, 2, 2), bits(al_, 6, 2));
            let mix = |cs: u32, cd: u32| ((sel(sa, cs, cd) - sel(sb, cs, cd)) * cf >> 7) + sel(sd, cs, cd);
            let (nr, ng, nb) = (mix(r, dr), mix(g, dg), mix(b, db));
            let clamp = self.regs[0x46] & 1 != 0;
            let fix = |v: i32| if clamp { v.clamp(0, 255) as u32 } else { (v & 0xFF) as u32 };
            r = fix(nr);
            g = fix(ng);
            b = fix(nb);
        }
        if c.fba {
            al |= 0x80;
        }
        let mut out = r | g << 8 | b << 16 | al << 24;
        if bpp(fpsm) == 16 {
            out = (r >> 3) | (g >> 3) << 5 | (b >> 3) << 10 | (al >> 7) << 15;
            fbmask = (fbmask >> 3 & 0x1F) | (fbmask >> 11 & 0x1F) << 5 | (fbmask >> 19 & 0x1F) << 10 | (fbmask >> 31) << 15;
        }
        if fbmask != 0 {
            out = (out & !fbmask) | (dst & fbmask);
        }
        gsmem::write(&mut self.vram, fpsm, c.fbp, c.fbw, x, y, out);
        self.pixels += 1;
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
        let tex = if a.tme {
            let (u, w) = self.tex_coords(v, a, c);
            Some((u as i32, w as i32))
        } else {
            None
        };
        self.pixel((v.x + 8) >> 4, (v.y + 8) >> 4, v.z, v.rgba, tex, v.f, a, c);
    }

    fn line(&mut self, v0: &Vertex, v1: &Vertex, a: Attr, c: &Ctx) {
        let (x0, y0) = (v0.x as f32 / 16.0, v0.y as f32 / 16.0);
        let (x1, y1) = (v1.x as f32 / 16.0, v1.y as f32 / 16.0);
        let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        let (t0, t1) = (self.tex_coords(v0, a, c), self.tex_coords(v1, a, c));
        for i in 0..n {
            let t = i as f32 / n as f32;
            let x = (x0 + (x1 - x0) * t).round() as i32;
            let y = (y0 + (y1 - y0) * t).round() as i32;
            let col = if a.iip { lerp_rgba(v0.rgba, v1.rgba, t) } else { v1.rgba };
            let z = (v0.z as f64 + (v1.z as f64 - v0.z as f64) * t as f64) as u32;
            let tex = if a.tme { Some(((t0.0 + (t1.0 - t0.0) * t) as i32, (t0.1 + (t1.1 - t0.1) * t) as i32)) } else { None };
            self.pixel(x, y, z, col, tex, v1.f, a, c);
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
        let (t0, t1) = (self.tex_coords(v0, a, c), self.tex_coords(v1, a, c));
        let dx = (v1.x - v0.x) as f32;
        let dy = (v1.y - v0.y) as f32;
        for y in y0..y1 {
            let ty = if dy != 0.0 { ((y << 4) - v0.y) as f32 / dy } else { 0.0 };
            let tv = t0.1 + (t1.1 - t0.1) * ty;
            for x in x0..x1 {
                let tex = if a.tme {
                    let tx = if dx != 0.0 { ((x << 4) - v0.x) as f32 / dx } else { 0.0 };
                    Some(((t0.0 + (t1.0 - t0.0) * tx) as i32, tv as i32))
                } else {
                    None
                };
                self.pixel(x, y, v1.z, v1.rgba, tex, v1.f, a, c);
            }
        }
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
        let sign = if area > 0 { 1 } else { -1 };
        let inv = 1.0 / area as f64;
        // Per-vertex attributes to interpolate.
        let tc: [(f32, f32, f32); 3] = [v0, v1, v2].map(|v| if a.fst { (v.u as f32, v.v as f32, 1.0) } else { (v.s, v.t, v.q) });
        let (tw, th) = ((1u32 << bits(c.tex0, 26, 4).min(10)) as f64 * 16.0, (1u32 << bits(c.tex0, 30, 4).min(10)) as f64 * 16.0);
        // Top-left rule bias per edge.
        let bias = |x0: i64, y0: i64, x1: i64, y1: i64| -> i64 {
            let (dx, dy) = (x1 - x0, y1 - y0);
            let top_left = if sign > 0 { dy < 0 || (dy == 0 && dx > 0) } else { dy > 0 || (dy == 0 && dx < 0) };
            if top_left { 0 } else { -1 }
        };
        let b0 = bias(bx, by, cx, cy);
        let b1 = bias(cx, cy, ax, ay);
        let b2 = bias(ax, ay, bx, by);
        for py in miny..=maxy {
            let y = py << 4;
            for px in minx..=maxx {
                let x = px << 4;
                let w0 = ((cx - bx) * (y - by) - (cy - by) * (x - bx)) * sign;
                let w1 = ((ax - cx) * (y - cy) - (ay - cy) * (x - cx)) * sign;
                let w2 = ((bx - ax) * (y - ay) - (by - ay) * (x - ax)) * sign;
                if w0 + b0 < 0 || w1 + b1 < 0 || w2 + b2 < 0 {
                    continue;
                }
                let l0 = w0 as f64 * inv * sign as f64;
                let l1 = w1 as f64 * inv * sign as f64;
                let l2 = 1.0 - l0 - l1;
                let z = (v0.z as f64 * l0 + v1.z as f64 * l1 + v2.z as f64 * l2).max(0.0) as u32;
                let col = if a.iip { bary_rgba(v0.rgba, v1.rgba, v2.rgba, l0, l1, l2) } else { v2.rgba };
                let fog = (v0.f as f64 * l0 + v1.f as f64 * l1 + v2.f as f64 * l2) as u32;
                let tex = if a.tme {
                    let s = tc[0].0 as f64 * l0 + tc[1].0 as f64 * l1 + tc[2].0 as f64 * l2;
                    let t = tc[0].1 as f64 * l0 + tc[1].1 as f64 * l1 + tc[2].1 as f64 * l2;
                    if a.fst {
                        Some((s as i32, t as i32))
                    } else {
                        let q = tc[0].2 as f64 * l0 + tc[1].2 as f64 * l1 + tc[2].2 as f64 * l2;
                        let q = if q == 0.0 { 1.0 } else { q };
                        Some(((s / q * tw) as i32, (t / q * th) as i32))
                    }
                } else {
                    None
                };
                self.pixel(px as i32, py as i32, z, col, tex, fog.min(255), a, c);
            }
        }
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
                        gsmem::write(&mut self.vram, dpsm, dbp, dbw, (dx + x) & 2047, (dy + y) & 2047, tmp[i]);
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
            gsmem::write(&mut self.vram, dpsm, dbp, dbw, x, y, px);
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

    pub fn display(&self, pmode: u64, dispfb: &[u64; 2], display: &[u64; 2], bg: u64, out: &mut Vec<u8>) -> (u32, u32) {
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
        for y in 0..h {
            for x in 0..w {
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
                let o = ((y * w + x) * 4) as usize;
                out[o] = c as u8;
                out[o + 1] = (c >> 8) as u8;
                out[o + 2] = (c >> 16) as u8;
                out[o + 3] = 255;
            }
        }
        (w, h)
    }
}

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

fn bary_rgba(a: u32, b: u32, c: u32, l0: f64, l1: f64, l2: f64) -> u32 {
    let mut o = 0;
    for sh in [0, 8, 16, 24] {
        let v = ((a >> sh) & 0xFF) as f64 * l0 + ((b >> sh) & 0xFF) as f64 * l1 + ((c >> sh) & 0xFF) as f64 * l2;
        o |= (v.clamp(0.0, 255.0) as u32) << sh;
    }
    o
}
