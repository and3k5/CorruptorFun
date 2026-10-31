//! GPU: command processing, software rasterizer and video timing.

pub const VRAM_W: usize = 1024;
pub const VRAM_H: usize = 512;

#[derive(Default)]
pub struct TickEvents {
    pub vblank_start: bool,
    pub hblanks: u32,
    pub frame_done: bool,
}

#[derive(Clone, Copy, Default)]
struct Vertex {
    x: i32,
    y: i32,
    r: i32,
    g: i32,
    b: i32,
    u: i32,
    v: i32,
}

#[derive(PartialEq)]
enum Transfer {
    None,
    ToVram { x: u32, y: u32, w: u32, h: u32, i: u32 },
    FromVram { x: u32, y: u32, w: u32, h: u32, i: u32 },
}

const DITHER: [[i32; 4]; 4] = [[-4, 0, -3, 1], [2, -2, 3, -1], [-3, 1, -4, 0], [3, -1, 2, -2]];

pub struct Gpu {
    pub vram: Vec<u16>,
    fifo: Vec<u32>,
    expected: usize,
    polyline: bool,
    transfer: Transfer,
    read_latch: u32,
    // E1..E6 state
    texpage: u32,
    tex_window: u32,
    tw: (u32, u32, u32, u32), // mask x, mask y, off x, off y
    area_x1: i32,
    area_y1: i32,
    area_x2: i32,
    area_y2: i32,
    off_x: i32,
    off_y: i32,
    set_mask: bool,
    check_mask: bool,
    rect_flip_x: bool,
    rect_flip_y: bool,
    // GP1 state
    pub display_disabled: bool,
    dma_dir: u32,
    disp_x: u32,
    disp_y: u32,
    h_range: (u32, u32),
    v_range: (u32, u32),
    disp_mode: u32,
    irq: bool,
    // Timing
    line: u32,
    line_cycles: u32,
    field: bool,
    in_vblank: bool,
    pub frames: u64,
}

impl Gpu {
    pub fn new() -> Self {
        let mut g = Gpu {
            vram: vec![0; VRAM_W * VRAM_H],
            fifo: Vec::with_capacity(16),
            expected: 0,
            polyline: false,
            transfer: Transfer::None,
            read_latch: 0,
            texpage: 0,
            tex_window: 0,
            tw: (0, 0, 0, 0),
            area_x1: 0,
            area_y1: 0,
            area_x2: 0,
            area_y2: 0,
            off_x: 0,
            off_y: 0,
            set_mask: false,
            check_mask: false,
            rect_flip_x: false,
            rect_flip_y: false,
            display_disabled: true,
            dma_dir: 0,
            disp_x: 0,
            disp_y: 0,
            h_range: (0x200, 0xC00),
            v_range: (0x10, 0x100),
            disp_mode: 0,
            irq: false,
            line: 0,
            line_cycles: 0,
            field: false,
            in_vblank: false,
            frames: 0,
        };
        g.reset();
        g
    }

    fn reset(&mut self) {
        self.fifo.clear();
        self.expected = 0;
        self.transfer = Transfer::None;
        self.texpage = 0;
        self.tex_window = 0;
        self.tw = (0, 0, 0, 0);
        self.area_x1 = 0;
        self.area_y1 = 0;
        self.area_x2 = 0;
        self.area_y2 = 0;
        self.off_x = 0;
        self.off_y = 0;
        self.set_mask = false;
        self.check_mask = false;
        self.display_disabled = true;
        self.dma_dir = 0;
        self.disp_x = 0;
        self.disp_y = 0;
        self.h_range = (0x200, 0xC00);
        self.v_range = (0x10, 0x100);
        self.disp_mode = 0;
        self.irq = false;
    }

    pub fn is_pal(&self) -> bool {
        self.disp_mode & 8 != 0
    }

    fn lines_per_frame(&self) -> u32 {
        if self.is_pal() { 314 } else { 263 }
    }

    fn cycles_per_line(&self) -> u32 {
        if self.is_pal() { 2169 } else { 2172 }
    }

    pub fn dot_divider(&self) -> u32 {
        if self.disp_mode & 0x40 != 0 {
            7
        } else {
            [10, 8, 5, 4][(self.disp_mode & 3) as usize]
        }
    }

    pub fn tick(&mut self, cycles: u32) -> TickEvents {
        let mut ev = TickEvents::default();
        self.line_cycles += cycles;
        let cpl = self.cycles_per_line();
        while self.line_cycles >= cpl {
            self.line_cycles -= cpl;
            self.line += 1;
            ev.hblanks += 1;
            let vblank_line = if self.is_pal() { 288 } else { 240 };
            if self.line == vblank_line {
                self.in_vblank = true;
                ev.vblank_start = true;
            }
            if self.line >= self.lines_per_frame() {
                self.line = 0;
                self.in_vblank = false;
                self.field = !self.field;
                self.frames += 1;
                ev.frame_done = true;
            }
        }
        ev
    }

    pub fn status(&self) -> u32 {
        let mut s = self.texpage & 0x7FF;
        s |= (self.set_mask as u32) << 11;
        s |= (self.check_mask as u32) << 12;
        let interlaced = self.disp_mode & 0x20 != 0;
        s |= ((!interlaced || self.field) as u32) << 13;
        s |= ((self.texpage >> 11) & 1) << 15;
        s |= ((self.disp_mode >> 6) & 1) << 16;
        s |= (self.disp_mode & 3) << 17;
        s |= ((self.disp_mode >> 2) & 1) << 19;
        s |= ((self.disp_mode >> 3) & 1) << 20;
        s |= ((self.disp_mode >> 4) & 1) << 21;
        s |= ((self.disp_mode >> 5) & 1) << 22;
        s |= (self.display_disabled as u32) << 23;
        s |= (self.irq as u32) << 24;
        let ready_cmd = 1;
        let ready_read = matches!(self.transfer, Transfer::FromVram { .. }) as u32;
        let ready_dma = 1;
        s |= ready_cmd << 26 | ready_read << 27 | ready_dma << 28;
        s |= self.dma_dir << 29;
        let dreq = match self.dma_dir {
            0 => 0,
            1 => 1,
            2 => ready_dma,
            _ => ready_read,
        };
        s |= dreq << 25;
        // Even/odd line flag; low during vblank.
        let odd = if interlaced { self.field } else { self.line & 1 != 0 };
        s |= ((odd && !self.in_vblank) as u32) << 31;
        s
    }

    pub fn read_data(&mut self) -> u32 {
        if let Transfer::FromVram { x, y, w, h, i } = self.transfer {
            let mut out = 0u32;
            let mut i = i;
            for k in 0..2 {
                let px = (x + i % w) as usize & 1023;
                let py = (y + i / w) as usize & 511;
                out |= (self.vram[py * VRAM_W + px] as u32) << (16 * k);
                i += 1;
            }
            if i >= w * h {
                self.transfer = Transfer::None;
            } else {
                self.transfer = Transfer::FromVram { x, y, w, h, i };
            }
            self.read_latch = out;
        }
        self.read_latch
    }

    /// Returns true if the GPU IRQ was raised.
    pub fn gp1(&mut self, v: u32) -> bool {
        match v >> 24 {
            0x00 => self.reset(),
            0x01 => {
                self.fifo.clear();
                self.expected = 0;
                self.transfer = Transfer::None;
            }
            0x02 => self.irq = false,
            0x03 => self.display_disabled = v & 1 != 0,
            0x04 => self.dma_dir = v & 3,
            0x05 => {
                self.disp_x = v & 0x3FE;
                self.disp_y = (v >> 10) & 0x1FF;
            }
            0x06 => self.h_range = (v & 0xFFF, (v >> 12) & 0xFFF),
            0x07 => self.v_range = (v & 0x3FF, (v >> 10) & 0x3FF),
            0x08 => self.disp_mode = v & 0xFF,
            0x10..=0x1F => {
                self.read_latch = match v & 7 {
                    2 => self.tex_window,
                    3 => (self.area_x1 as u32) | ((self.area_y1 as u32) << 10),
                    4 => (self.area_x2 as u32) | ((self.area_y2 as u32) << 10),
                    5 => ((self.off_x as u32) & 0x7FF) | (((self.off_y as u32) & 0x7FF) << 11),
                    7 => 2,
                    _ => self.read_latch,
                };
            }
            _ => {}
        }
        false
    }

    fn cmd_len(cmd: u32) -> usize {
        let op = cmd >> 24;
        match op {
            0x02 => 3,
            0x20..=0x3F => {
                let quad = op & 0x08 != 0;
                let tex = op & 0x04 != 0;
                let shaded = op & 0x10 != 0;
                let n = if quad { 4 } else { 3 };
                let per = 1 + tex as usize + shaded as usize;
                // First vertex shares its colour with the command word.
                n * per + 1 - shaded as usize
            }
            0x40..=0x5F => {
                let shaded = op & 0x10 != 0;
                if op & 0x08 != 0 {
                    usize::MAX // polyline, terminated by 0x5xxx5xxx
                } else if shaded {
                    4
                } else {
                    3
                }
            }
            0x60..=0x7F => {
                let tex = op & 0x04 != 0;
                let var = (op >> 3) & 3 == 0;
                2 + tex as usize + var as usize
            }
            0x80..=0x9F => 4,
            0xA0..=0xBF => 3,
            0xC0..=0xDF => 3,
            _ => 1,
        }
    }

    pub fn gp0(&mut self, v: u32) {
        if let Transfer::ToVram { x, y, w, h, i } = self.transfer {
            let mut i = i;
            for k in 0..2 {
                if i < w * h {
                    let px = (x + i % w) as usize & 1023;
                    let py = (y + i / w) as usize & 511;
                    let idx = py * VRAM_W + px;
                    if !(self.check_mask && self.vram[idx] & 0x8000 != 0) {
                        let mut c = (v >> (16 * k)) as u16;
                        if self.set_mask {
                            c |= 0x8000;
                        }
                        self.vram[idx] = c;
                    }
                    i += 1;
                }
            }
            self.transfer = if i >= w * h { Transfer::None } else { Transfer::ToVram { x, y, w, h, i } };
            return;
        }

        if self.fifo.is_empty() {
            self.expected = Self::cmd_len(v);
            self.polyline = self.expected == usize::MAX;
        }
        self.fifo.push(v);

        if self.polyline {
            let shaded = self.fifo[0] & 0x1000_0000 != 0;
            let n = self.fifo.len();
            // Terminator is only checked where a vertex word could start.
            let at_vertex = if shaded { n >= 4 && n % 2 == 1 } else { n >= 4 };
            if (at_vertex && v & 0xF000_F000 == 0x5000_5000) || n > 512 {
                self.fifo.pop();
                self.draw_polyline();
                self.fifo.clear();
            }
            return;
        }
        if self.fifo.len() >= self.expected {
            let cmd = std::mem::take(&mut self.fifo);
            self.execute(&cmd);
            self.fifo = cmd;
            self.fifo.clear();
        }
    }

    fn execute(&mut self, c: &[u32]) {
        let op = c[0] >> 24;
        match op {
            0x00 | 0x01 | 0x03..=0x1E => {}
            0x1F => self.irq = true,
            0x02 => self.fill(c),
            0x20..=0x3F => self.polygon(c),
            0x40..=0x5F => self.line_cmd(c),
            0x60..=0x7F => self.rect(c),
            0x80..=0x9F => self.vram_copy(c),
            0xA0..=0xBF => {
                let (x, y, w, h) = Self::xywh(c[1], c[2]);
                self.transfer = Transfer::ToVram { x, y, w, h, i: 0 };
            }
            0xC0..=0xDF => {
                let (x, y, w, h) = Self::xywh(c[1], c[2]);
                self.transfer = Transfer::FromVram { x, y, w, h, i: 0 };
            }
            0xE1 => {
                self.texpage = (self.texpage & !0x9FF) | (c[0] & 0x9FF);
                // Keep bit 11 (texture disable) as well.
                self.texpage = (self.texpage & !0x800) | (c[0] & 0x800);
                self.rect_flip_x = c[0] & (1 << 12) != 0;
                self.rect_flip_y = c[0] & (1 << 13) != 0;
            }
            0xE2 => {
                self.tex_window = c[0] & 0xF_FFFF;
                let v = c[0];
                self.tw = ((v & 0x1F) * 8, ((v >> 5) & 0x1F) * 8, ((v >> 10) & 0x1F) * 8, ((v >> 15) & 0x1F) * 8);
            }
            0xE3 => {
                self.area_x1 = (c[0] & 0x3FF) as i32;
                self.area_y1 = ((c[0] >> 10) & 0x1FF) as i32;
            }
            0xE4 => {
                self.area_x2 = (c[0] & 0x3FF) as i32;
                self.area_y2 = ((c[0] >> 10) & 0x1FF) as i32;
            }
            0xE5 => {
                self.off_x = ((c[0] << 21) as i32) >> 21;
                self.off_y = (((c[0] >> 11) << 21) as i32) >> 21;
            }
            0xE6 => {
                self.set_mask = c[0] & 1 != 0;
                self.check_mask = c[0] & 2 != 0;
            }
            _ => {}
        }
    }

    fn xywh(a: u32, b: u32) -> (u32, u32, u32, u32) {
        let x = a & 0x3FF;
        let y = (a >> 16) & 0x1FF;
        let w = ((b & 0xFFFF).wrapping_sub(1) & 0x3FF) + 1;
        let h = (((b >> 16) & 0xFFFF).wrapping_sub(1) & 0x1FF) + 1;
        (x, y, w, h)
    }

    fn fill(&mut self, c: &[u32]) {
        let col = Self::to15(c[0]);
        let x0 = c[1] & 0x3F0;
        let y0 = (c[1] >> 16) & 0x1FF;
        let w = ((c[2] & 0x3FF) + 0xF) & !0xF;
        let h = (c[2] >> 16) & 0x1FF;
        for y in 0..h {
            let row = ((y0 + y) & 511) as usize * VRAM_W;
            if x0 + w <= VRAM_W as u32 {
                self.vram[row + x0 as usize..row + (x0 + w) as usize].fill(col);
            } else {
                for x in 0..w {
                    self.vram[row + ((x0 + x) & 1023) as usize] = col;
                }
            }
        }
    }

    fn vram_copy(&mut self, c: &[u32]) {
        let sx = c[1] & 0x3FF;
        let sy = (c[1] >> 16) & 0x1FF;
        let dx = c[2] & 0x3FF;
        let dy = (c[2] >> 16) & 0x1FF;
        let (_, _, w, h) = Self::xywh(0, c[3]);
        for y in 0..h {
            for x in 0..w {
                let s = (((sy + y) & 511) as usize) * VRAM_W + ((sx + x) & 1023) as usize;
                let d = (((dy + y) & 511) as usize) * VRAM_W + ((dx + x) & 1023) as usize;
                if self.check_mask && self.vram[d] & 0x8000 != 0 {
                    continue;
                }
                let mut v = self.vram[s];
                if self.set_mask {
                    v |= 0x8000;
                }
                self.vram[d] = v;
            }
        }
    }

    fn to15(c: u32) -> u16 {
        let r = (c >> 3) & 0x1F;
        let g = (c >> 11) & 0x1F;
        let b = (c >> 19) & 0x1F;
        (r | (g << 5) | (b << 10)) as u16
    }

    fn vertex(&self, w: u32) -> (i32, i32) {
        let x = ((w << 21) as i32) >> 21;
        let y = (((w >> 16) << 21) as i32) >> 21;
        (x + self.off_x, y + self.off_y)
    }

    fn polygon(&mut self, c: &[u32]) {
        let op = c[0] >> 24;
        let quad = op & 0x08 != 0;
        let tex = op & 0x04 != 0;
        let shaded = op & 0x10 != 0;
        let semi = op & 0x02 != 0;
        let raw = op & 0x01 != 0;
        let n = if quad { 4 } else { 3 };
        let mut vs = [Vertex::default(); 4];
        let mut idx = 1;
        let mut clut = 0u32;
        let mut page = self.texpage;
        for i in 0..n {
            let color = if i > 0 && shaded {
                idx += 1;
                c[idx - 1]
            } else {
                c[0]
            };
            let (x, y) = self.vertex(c[idx]);
            idx += 1;
            let (mut u, mut v) = (0, 0);
            if tex {
                let t = c[idx];
                idx += 1;
                u = (t & 0xFF) as i32;
                v = ((t >> 8) & 0xFF) as i32;
                if i == 0 {
                    clut = t >> 16;
                }
                if i == 1 {
                    page = t >> 16;
                }
            }
            vs[i] = Vertex { x, y, r: (color & 0xFF) as i32, g: ((color >> 8) & 0xFF) as i32, b: ((color >> 16) & 0xFF) as i32, u, v };
        }
        if tex {
            // Textured polygons update the draw mode's texpage bits.
            self.texpage = (self.texpage & !0x9FF) | (page & 0x9FF);
        }
        let st = Shade { tex, shaded, semi, raw, clut, page: if tex { page } else { self.texpage } };
        self.triangle(vs[0], vs[1], vs[2], &st);
        if quad {
            self.triangle(vs[1], vs[2], vs[3], &st);
        }
    }

    fn triangle(&mut self, a: Vertex, b: Vertex, c: Vertex, st: &Shade) {
        // Reject polygons larger than the hardware allows.
        let minx = a.x.min(b.x).min(c.x);
        let maxx = a.x.max(b.x).max(c.x);
        let miny = a.y.min(b.y).min(c.y);
        let maxy = a.y.max(b.y).max(c.y);
        if maxx - minx >= 1024 || maxy - miny >= 512 {
            return;
        }
        let area = (b.x - a.x) as i64 * (c.y - a.y) as i64 - (c.x - a.x) as i64 * (b.y - a.y) as i64;
        if area == 0 {
            return;
        }
        let (a, b, c, area) = if area < 0 { (a, c, b, -area) } else { (a, b, c, area) };

        let x0 = minx.max(self.area_x1);
        let x1 = maxx.min(self.area_x2);
        let y0 = miny.max(self.area_y1);
        let y1 = maxy.min(self.area_y2);
        if x0 > x1 || y0 > y1 {
            return;
        }

        // Edge functions with a top-left rule, stepped incrementally. Each
        // edge value is linear in x and y, and so is every attribute's
        // weighted sum, so a row only needs additions.
        let edge = |p: &Vertex, q: &Vertex, x: i32, y: i32| -> i32 { (q.x - p.x) * (y - p.y) - (q.y - p.y) * (x - p.x) };
        let bias = |p: &Vertex, q: &Vertex| -> i32 {
            let top_left = (q.y < p.y) || (q.y == p.y && q.x > p.x);
            if top_left { 0 } else { -1 }
        };
        let bias0 = bias(&b, &c);
        let bias1 = bias(&c, &a);
        let bias2 = bias(&a, &b);
        let (dx0, dx1, dx2) = (b.y - c.y, c.y - a.y, a.y - b.y);

        // Attributes as 16.16 fixed point: value at the row start (exact),
        // plus a per-pixel step.
        let attrs: [(i32, i32, i32); 5] = [(a.r, b.r, c.r), (a.g, b.g, c.g), (a.b, b.b, c.b), (a.u, b.u, c.u), (a.v, b.v, c.v)];
        let n_attr = if st.tex { 5 } else if st.shaded { 3 } else { 0 };
        let mut step = [0i64; 5];
        for (k, &(va, vb, vc)) in attrs.iter().enumerate().take(n_attr) {
            let num = dx0 as i64 * va as i64 + dx1 as i64 * vb as i64 + dx2 as i64 * vc as i64;
            step[k] = (num << 16) / area;
        }

        let dither = self.texpage & (1 << 9) != 0 && (st.shaded || (st.tex && !st.raw));
        for y in y0..=y1 {
            // Solve each edge for the row's covered span instead of testing
            // every pixel of the bounding box.
            let mut lo = x0;
            let mut hi = x1;
            for (w, d) in [(edge(&b, &c, x0, y) + bias0, dx0), (edge(&c, &a, x0, y) + bias1, dx1), (edge(&a, &b, x0, y) + bias2, dx2)] {
                if d > 0 {
                    lo = lo.max(x0 - w.div_euclid(d));
                } else if d < 0 {
                    hi = hi.min(x0 + w.div_euclid(-d));
                } else if w < 0 {
                    hi = lo - 1;
                }
            }
            if lo > hi {
                continue;
            }
            let (w0, w1, w2) = (edge(&b, &c, lo, y), edge(&c, &a, lo, y), edge(&a, &b, lo, y));
            let mut acc = [0i64; 5];
            for (k, &(va, vb, vc)) in attrs.iter().enumerate().take(n_attr) {
                let num = w0 as i64 * va as i64 + w1 as i64 * vb as i64 + w2 as i64 * vc as i64;
                acc[k] = (num << 16) / area;
            }
            for x in lo..=hi {
                let at = |k: usize| (acc[k] >> 16) as i32;
                let (r, g, bb) = if st.shaded { (at(0), at(1), at(2)) } else { (a.r, a.g, a.b) };
                let (u, v) = if st.tex { (at(3), at(4)) } else { (0, 0) };
                self.plot(x, y, r, g, bb, u, v, st, dither);
                for k in 0..5 {
                    acc[k] += step[k];
                }
            }
        }
    }

    fn texel(&self, u: i32, v: i32, page: u32, clut: u32) -> u16 {
        let (mx, my, ox, oy) = self.tw;
        let u = ((u as u32 & 0xFF) & !mx) | (ox & mx);
        let v = ((v as u32 & 0xFF) & !my) | (oy & my);
        let bx = (page & 0xF) * 64;
        let by = ((page >> 4) & 1) * 256;
        let depth = (page >> 7) & 3;
        let cx = (clut & 0x3F) * 16;
        let cy = (clut >> 6) & 0x1FF;
        let row = ((by + v) & 511) as usize * VRAM_W;
        match depth {
            0 => {
                let w = self.vram[row + ((bx + u / 4) & 1023) as usize];
                let i = (w >> ((u & 3) * 4)) & 0xF;
                self.vram[cy as usize * VRAM_W + ((cx + i as u32) & 1023) as usize]
            }
            1 => {
                let w = self.vram[row + ((bx + u / 2) & 1023) as usize];
                let i = (w >> ((u & 1) * 8)) & 0xFF;
                self.vram[cy as usize * VRAM_W + ((cx + i as u32) & 1023) as usize]
            }
            _ => self.vram[row + ((bx + u) & 1023) as usize],
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn plot(&mut self, x: i32, y: i32, r: i32, g: i32, b: i32, u: i32, v: i32, st: &Shade, dither: bool) {
        let idx = (y as usize & 511) * VRAM_W + (x as usize & 1023);
        if self.check_mask && self.vram[idx] & 0x8000 != 0 {
            return;
        }
        let mut transparent_pixel = st.semi;
        let (mut cr, mut cg, mut cb);
        if st.tex {
            let t = self.texel(u, v, st.page, st.clut);
            if t == 0 {
                return;
            }
            let tr = (t & 0x1F) as i32;
            let tg = ((t >> 5) & 0x1F) as i32;
            let tb = ((t >> 10) & 0x1F) as i32;
            transparent_pixel = st.semi && t & 0x8000 != 0;
            if st.raw {
                cr = tr << 3;
                cg = tg << 3;
                cb = tb << 3;
            } else {
                cr = (tr << 3) * r / 128;
                cg = (tg << 3) * g / 128;
                cb = (tb << 3) * b / 128;
            }
            if self.set_mask || t & 0x8000 != 0 {
                // mask bit handled below
            }
        } else {
            cr = r;
            cg = g;
            cb = b;
        }
        if dither {
            let d = DITHER[(y & 3) as usize][(x & 3) as usize];
            cr += d;
            cg += d;
            cb += d;
        }
        let mut pr = (cr.clamp(0, 255) >> 3) as u16;
        let mut pg = (cg.clamp(0, 255) >> 3) as u16;
        let mut pb = (cb.clamp(0, 255) >> 3) as u16;
        if transparent_pixel {
            let mode = if st.tex { (st.page >> 5) & 3 } else { (self.texpage >> 5) & 3 };
            let bg = self.vram[idx];
            let br = (bg & 0x1F) as i32;
            let bgc = ((bg >> 5) & 0x1F) as i32;
            let bb = ((bg >> 10) & 0x1F) as i32;
            let blend = |b: i32, f: i32| -> u16 {
                let v = match mode {
                    0 => (b + f) / 2,
                    1 => b + f,
                    2 => b - f,
                    _ => b + f / 4,
                };
                v.clamp(0, 31) as u16
            };
            pr = blend(br, pr as i32);
            pg = blend(bgc, pg as i32);
            pb = blend(bb, pb as i32);
        }
        let mut out = pr | (pg << 5) | (pb << 10);
        if self.set_mask {
            out |= 0x8000;
        }
        self.vram[idx] = out;
    }

    fn rect(&mut self, c: &[u32]) {
        let op = c[0] >> 24;
        let tex = op & 0x04 != 0;
        let semi = op & 0x02 != 0;
        let raw = op & 0x01 != 0;
        let (x, y) = self.vertex(c[1]);
        let mut i = 2;
        let (mut u0, mut v0, mut clut) = (0, 0, 0);
        if tex {
            u0 = (c[i] & 0xFF) as i32;
            v0 = ((c[i] >> 8) & 0xFF) as i32;
            clut = c[i] >> 16;
            i += 1;
        }
        let (w, h) = match (op >> 3) & 3 {
            0 => ((c[i] & 0x3FF) as i32, ((c[i] >> 16) & 0x1FF) as i32),
            1 => (1, 1),
            2 => (8, 8),
            _ => (16, 16),
        };
        let st = Shade { tex, shaded: false, semi, raw, clut, page: self.texpage };
        let (r, g, b) = ((c[0] & 0xFF) as i32, ((c[0] >> 8) & 0xFF) as i32, ((c[0] >> 16) & 0xFF) as i32);
        let ys = y.max(self.area_y1);
        let ye = (y + h - 1).min(self.area_y2);
        let xs = x.max(self.area_x1);
        let xe = (x + w - 1).min(self.area_x2);
        for py in ys..=ye {
            for px in xs..=xe {
                let du = px - x;
                let dv = py - y;
                let u = if self.rect_flip_x { u0 - du } else { u0 + du };
                let v = if self.rect_flip_y { v0 - dv } else { v0 + dv };
                self.plot(px, py, r, g, b, u, v, &st, false);
            }
        }
    }

    fn line_cmd(&mut self, c: &[u32]) {
        let shaded = c[0] & 0x1000_0000 != 0;
        let (x0, y0) = self.vertex(c[1]);
        let (col1, p1) = if shaded { (c[2], c[3]) } else { (c[0], c[2]) };
        let (x1, y1) = self.vertex(p1);
        self.draw_line(x0, y0, c[0], x1, y1, col1, shaded, c[0] & 0x0200_0000 != 0);
    }

    fn draw_polyline(&mut self) {
        let c = std::mem::take(&mut self.fifo);
        let shaded = c[0] & 0x1000_0000 != 0;
        let semi = c[0] & 0x0200_0000 != 0;
        let mut pts = Vec::new();
        if shaded {
            pts.push((c[0], c[1]));
            let mut i = 2;
            while i + 1 < c.len() {
                pts.push((c[i], c[i + 1]));
                i += 2;
            }
        } else {
            for w in &c[1..] {
                pts.push((c[0], *w));
            }
        }
        for pair in pts.windows(2) {
            let (x0, y0) = self.vertex(pair[0].1);
            let (x1, y1) = self.vertex(pair[1].1);
            self.draw_line(x0, y0, pair[0].0, x1, y1, pair[1].0, shaded, semi);
        }
        self.fifo = c;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line(&mut self, x0: i32, y0: i32, c0: u32, x1: i32, y1: i32, c1: u32, shaded: bool, semi: bool) {
        if (x1 - x0).abs() >= 1024 || (y1 - y0).abs() >= 512 {
            return;
        }
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let st = Shade { tex: false, shaded, semi, raw: false, clut: 0, page: self.texpage };
        let dither = self.texpage & (1 << 9) != 0 && shaded;
        let ch = |c: u32, s: u32| ((c >> s) & 0xFF) as i32;
        for i in 0..=steps {
            let x = x0 + (x1 - x0) * i / steps;
            let y = y0 + (y1 - y0) * i / steps;
            if x < self.area_x1 || x > self.area_x2 || y < self.area_y1 || y > self.area_y2 {
                continue;
            }
            let (r, g, b) = if shaded {
                (
                    ch(c0, 0) + (ch(c1, 0) - ch(c0, 0)) * i / steps,
                    ch(c0, 8) + (ch(c1, 8) - ch(c0, 8)) * i / steps,
                    ch(c0, 16) + (ch(c1, 16) - ch(c0, 16)) * i / steps,
                )
            } else {
                (ch(c0, 0), ch(c0, 8), ch(c0, 16))
            };
            self.plot(x, y, r, g, b, 0, 0, &st, dither);
        }
    }

    // ---- output ----

    pub fn display_size(&self) -> (u32, u32) {
        let w = if self.disp_mode & 0x40 != 0 { 368 } else { [256, 320, 512, 640][(self.disp_mode & 3) as usize] };
        let lines = self.v_range.1.saturating_sub(self.v_range.0).clamp(1, 288);
        let interlaced = self.disp_mode & 0x24 == 0x24;
        (w, if interlaced { lines * 2 } else { lines })
    }

    /// Renders the visible display area to RGBA.
    pub fn render(&self, out: &mut Vec<u8>) -> (u32, u32) {
        let (w, h) = self.display_size();
        out.resize((w * h * 4) as usize, 0);
        if self.display_disabled {
            out.iter_mut().for_each(|b| *b = 0);
            for p in out.chunks_exact_mut(4) {
                p[3] = 255;
            }
            return (w, h);
        }
        let depth24 = self.disp_mode & 0x10 != 0;
        for y in 0..h {
            let vy = ((self.disp_y + y) & 511) as usize;
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                if depth24 {
                    let bx = (self.disp_x as usize) * 2 + x as usize * 3;
                    let byte = |i: usize| -> u8 {
                        let px = self.vram[vy * VRAM_W + ((i / 2) & 1023)];
                        if i & 1 == 0 { px as u8 } else { (px >> 8) as u8 }
                    };
                    out[o] = byte(bx);
                    out[o + 1] = byte(bx + 1);
                    out[o + 2] = byte(bx + 2);
                } else {
                    let px = self.vram[vy * VRAM_W + ((self.disp_x + x) as usize & 1023)];
                    out[o] = ((px & 0x1F) << 3) as u8 | ((px & 0x1F) >> 2) as u8;
                    out[o + 1] = (((px >> 5) & 0x1F) << 3) as u8 | (((px >> 5) & 0x1F) >> 2) as u8;
                    out[o + 2] = (((px >> 10) & 0x1F) << 3) as u8 | (((px >> 10) & 0x1F) >> 2) as u8;
                }
                out[o + 3] = 255;
            }
        }
        (w, h)
    }
}

struct Shade {
    tex: bool,
    shaded: bool,
    semi: bool,
    raw: bool,
    clut: u32,
    page: u32,
}
