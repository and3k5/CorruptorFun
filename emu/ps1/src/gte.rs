//! Geometry Transformation Engine (COP2).
//!
//! Follows the documented fixed-point behaviour (no-$ psx-spx): 44-bit MAC
//! overflow flags, IR saturation, the UNR divider and the colour FIFO.

const UNR_TABLE: [u8; 0x101] = {
    let mut t = [0u8; 0x101];
    let mut i = 0;
    while i < 0x101 {
        let v = ((0x40000 / (i + 0x100) + 1) / 2) as i32 - 0x101;
        t[i] = if v < 0 { 0 } else { v as u8 };
        i += 1;
    }
    t
};

#[derive(Clone)]
pub struct Gte {
    // Data registers
    v: [[i16; 3]; 4], // V0, V1, V2 and (as scratch) IR as a vector
    rgbc: [u8; 4],
    otz: u16,
    ir: [i16; 4], // IR0..IR3
    sxy: [(i16, i16); 3],
    sz: [u16; 4],
    rgb: [[u8; 4]; 3],
    res1: u32,
    mac: [i32; 4], // MAC0..MAC3
    lzcs: u32,
    // Control registers
    rt: [[i16; 3]; 3],
    tr: [i32; 3],
    l: [[i16; 3]; 3],
    bk: [i32; 3],
    lr: [[i16; 3]; 3],
    fc: [i32; 3],
    ofx: i32,
    ofy: i32,
    h: u16,
    dqa: i16,
    dqb: i32,
    zsf3: i16,
    zsf4: i16,
    flag: u32,
}

impl Gte {
    pub fn new() -> Self {
        Gte {
            v: [[0; 3]; 4],
            rgbc: [0; 4],
            otz: 0,
            ir: [0; 4],
            sxy: [(0, 0); 3],
            sz: [0; 4],
            rgb: [[0; 4]; 3],
            res1: 0,
            mac: [0; 4],
            lzcs: 0,
            rt: [[0; 3]; 3],
            tr: [0; 3],
            l: [[0; 3]; 3],
            bk: [0; 3],
            lr: [[0; 3]; 3],
            fc: [0; 3],
            ofx: 0,
            ofy: 0,
            h: 0,
            dqa: 0,
            dqb: 0,
            zsf3: 0,
            zsf4: 0,
            flag: 0,
        }
    }

    fn pack16(a: i16, b: i16) -> u32 {
        (a as u16 as u32) | ((b as u16 as u32) << 16)
    }

    fn orgb(&self) -> u32 {
        let c = |v: i16| -> u32 { ((v as i32) >> 7).clamp(0, 0x1F) as u32 };
        c(self.ir[1]) | (c(self.ir[2]) << 5) | (c(self.ir[3]) << 10)
    }

    pub fn read_data(&self, r: usize) -> u32 {
        match r {
            0 => Self::pack16(self.v[0][0], self.v[0][1]),
            1 => self.v[0][2] as i32 as u32,
            2 => Self::pack16(self.v[1][0], self.v[1][1]),
            3 => self.v[1][2] as i32 as u32,
            4 => Self::pack16(self.v[2][0], self.v[2][1]),
            5 => self.v[2][2] as i32 as u32,
            6 => u32::from_le_bytes(self.rgbc),
            7 => self.otz as u32,
            8..=11 => self.ir[r - 8] as i32 as u32,
            12..=14 => Self::pack16(self.sxy[r - 12].0, self.sxy[r - 12].1),
            15 => Self::pack16(self.sxy[2].0, self.sxy[2].1),
            16..=19 => self.sz[r - 16] as u32,
            20..=22 => u32::from_le_bytes(self.rgb[r - 20]),
            23 => self.res1,
            24..=27 => self.mac[r - 24] as u32,
            28 | 29 => self.orgb(),
            30 => self.lzcs,
            31 => {
                let v = self.lzcs as i32;
                if v < 0 { (!self.lzcs).leading_zeros() } else { self.lzcs.leading_zeros() }
            }
            _ => 0,
        }
    }

    pub fn write_data(&mut self, r: usize, v: u32) {
        let lo = v as i16;
        let hi = (v >> 16) as i16;
        match r {
            0 => { self.v[0][0] = lo; self.v[0][1] = hi }
            1 => self.v[0][2] = lo,
            2 => { self.v[1][0] = lo; self.v[1][1] = hi }
            3 => self.v[1][2] = lo,
            4 => { self.v[2][0] = lo; self.v[2][1] = hi }
            5 => self.v[2][2] = lo,
            6 => self.rgbc = v.to_le_bytes(),
            7 => self.otz = v as u16,
            8..=11 => self.ir[r - 8] = lo,
            12..=14 => self.sxy[r - 12] = (lo, hi),
            15 => {
                self.sxy[0] = self.sxy[1];
                self.sxy[1] = self.sxy[2];
                self.sxy[2] = (lo, hi);
            }
            16..=19 => self.sz[r - 16] = v as u16,
            20..=22 => self.rgb[r - 20] = v.to_le_bytes(),
            23 => self.res1 = v,
            24..=27 => self.mac[r - 24] = v as i32,
            28 => {
                self.ir[1] = ((v & 0x1F) * 0x80) as i16;
                self.ir[2] = (((v >> 5) & 0x1F) * 0x80) as i16;
                self.ir[3] = (((v >> 10) & 0x1F) * 0x80) as i16;
            }
            30 => self.lzcs = v,
            _ => {}
        }
    }

    fn mat_read(m: &[[i16; 3]; 3], r: usize) -> u32 {
        match r {
            0 => Self::pack16(m[0][0], m[0][1]),
            1 => Self::pack16(m[0][2], m[1][0]),
            2 => Self::pack16(m[1][1], m[1][2]),
            3 => Self::pack16(m[2][0], m[2][1]),
            _ => m[2][2] as i32 as u32,
        }
    }

    fn mat_write(m: &mut [[i16; 3]; 3], r: usize, v: u32) {
        let lo = v as i16;
        let hi = (v >> 16) as i16;
        match r {
            0 => { m[0][0] = lo; m[0][1] = hi }
            1 => { m[0][2] = lo; m[1][0] = hi }
            2 => { m[1][1] = lo; m[1][2] = hi }
            3 => { m[2][0] = lo; m[2][1] = hi }
            _ => m[2][2] = lo,
        }
    }

    pub fn read_ctrl(&self, r: usize) -> u32 {
        match r {
            0..=4 => Self::mat_read(&self.rt, r),
            5..=7 => self.tr[r - 5] as u32,
            8..=12 => Self::mat_read(&self.l, r - 8),
            13..=15 => self.bk[r - 13] as u32,
            16..=20 => Self::mat_read(&self.lr, r - 16),
            21..=23 => self.fc[r - 21] as u32,
            24 => self.ofx as u32,
            25 => self.ofy as u32,
            26 => self.h as i16 as i32 as u32, // hardware sign-extends H on read
            27 => self.dqa as i32 as u32,
            28 => self.dqb as u32,
            29 => self.zsf3 as i32 as u32,
            30 => self.zsf4 as i32 as u32,
            31 => self.flag,
            _ => 0,
        }
    }

    pub fn write_ctrl(&mut self, r: usize, v: u32) {
        match r {
            0..=4 => Self::mat_write(&mut self.rt, r, v),
            5..=7 => self.tr[r - 5] = v as i32,
            8..=12 => Self::mat_write(&mut self.l, r - 8, v),
            13..=15 => self.bk[r - 13] = v as i32,
            16..=20 => Self::mat_write(&mut self.lr, r - 16, v),
            21..=23 => self.fc[r - 21] = v as i32,
            24 => self.ofx = v as i32,
            25 => self.ofy = v as i32,
            26 => self.h = v as u16,
            27 => self.dqa = v as i16,
            28 => self.dqb = v as i32,
            29 => self.zsf3 = v as i16,
            30 => self.zsf4 = v as i16,
            31 => {
                self.flag = v & 0x7FFF_F000;
                self.update_flag_summary();
            }
            _ => {}
        }
    }

    fn update_flag_summary(&mut self) {
        if self.flag & 0x7F87_E000 != 0 {
            self.flag |= 0x8000_0000;
        } else {
            self.flag &= 0x7FFF_FFFF;
        }
    }

    // ---- saturation / flag helpers ----

    /// Checks a MAC1..3 value for 44-bit overflow.
    fn check_mac(&mut self, i: usize, v: i64) -> i64 {
        if v >= 1 << 43 {
            self.flag |= 1 << (31 - i);
        } else if v < -(1 << 43) {
            self.flag |= 1 << (28 - i);
        }
        // Wrap to 44 bits like the hardware accumulator.
        (v << 20) >> 20
    }

    fn set_mac(&mut self, i: usize, v: i64, shift: u32) -> i64 {
        let v = self.check_mac(i, v);
        let r = v >> shift;
        self.mac[i] = r as i32;
        r
    }

    fn set_mac0(&mut self, v: i64) -> i64 {
        if v > 0x7FFF_FFFF {
            self.flag |= 1 << 16;
        } else if v < -0x8000_0000 {
            self.flag |= 1 << 15;
        }
        self.mac[0] = v as i32;
        v
    }

    fn set_ir(&mut self, i: usize, v: i64, lm: bool) {
        let min = if lm { 0 } else { -0x8000 };
        let c = if v > 0x7FFF {
            self.flag |= 1 << (25 - i);
            0x7FFF
        } else if v < min {
            self.flag |= 1 << (25 - i);
            min
        } else {
            v
        };
        self.ir[i] = c as i16;
    }

    fn set_ir0(&mut self, v: i64) {
        let c = if v > 0x1000 {
            self.flag |= 1 << 12;
            0x1000
        } else if v < 0 {
            self.flag |= 1 << 12;
            0
        } else {
            v
        };
        self.ir[0] = c as i16;
    }

    fn set_mac_ir(&mut self, i: usize, v: i64, shift: u32, lm: bool) {
        let r = self.set_mac(i, v, shift);
        self.set_ir(i, r, lm);
    }

    fn push_sz(&mut self, v: i64) {
        let c = if v > 0xFFFF {
            self.flag |= 1 << 18;
            0xFFFF
        } else if v < 0 {
            self.flag |= 1 << 18;
            0
        } else {
            v
        };
        self.sz[0] = self.sz[1];
        self.sz[1] = self.sz[2];
        self.sz[2] = self.sz[3];
        self.sz[3] = c as u16;
    }

    fn push_sxy(&mut self, x: i64, y: i64) {
        let cx = if x > 0x3FF { self.flag |= 1 << 14; 0x3FF } else if x < -0x400 { self.flag |= 1 << 14; -0x400 } else { x };
        let cy = if y > 0x3FF { self.flag |= 1 << 13; 0x3FF } else if y < -0x400 { self.flag |= 1 << 13; -0x400 } else { y };
        self.sxy[0] = self.sxy[1];
        self.sxy[1] = self.sxy[2];
        self.sxy[2] = (cx as i16, cy as i16);
    }

    fn push_color(&mut self) {
        let mut c = [0u8; 4];
        for i in 0..3 {
            let v = self.mac[i + 1] >> 4;
            c[i] = if v > 0xFF {
                self.flag |= 1 << (21 - i);
                0xFF
            } else if v < 0 {
                self.flag |= 1 << (21 - i);
                0
            } else {
                v as u8
            };
        }
        c[3] = self.rgbc[3];
        self.rgb[0] = self.rgb[1];
        self.rgb[1] = self.rgb[2];
        self.rgb[2] = c;
    }

    fn divide(&mut self) -> i64 {
        let h = self.h as u32;
        let sz3 = self.sz[3] as u32;
        if h < sz3 * 2 {
            let z = (sz3 as u16).leading_zeros();
            let n = (h as u64) << z;
            let d = (sz3 << z) as u64;
            let u = UNR_TABLE[((d - 0x7FC0) >> 7) as usize] as u64 + 0x101;
            let d = (0x2000080 - d * u) >> 8;
            let d = (0x0000080 + d * u) >> 8;
            ((n * d + 0x8000) >> 16).min(0x1FFFF) as i64
        } else {
            self.flag |= 1 << 17;
            0x1FFFF
        }
    }

    // ---- commands ----

    pub fn command(&mut self, cmd: u32) {
        self.flag = 0;
        let sf = if cmd & (1 << 19) != 0 { 12 } else { 0 };
        let lm = cmd & (1 << 10) != 0;
        match cmd & 0x3F {
            0x01 => self.rtp(0, sf, lm, true),
            0x06 => self.nclip(),
            0x0C => self.op(sf, lm),
            0x10 => self.dpcs(sf, lm, false),
            0x11 => self.intpl(sf, lm),
            0x12 => self.mvmva(cmd, sf, lm),
            0x13 => self.ncd(0, sf, lm),
            0x14 => self.cdp(sf, lm),
            0x16 => { self.ncd(0, sf, lm); self.ncd(1, sf, lm); self.ncd(2, sf, lm) }
            0x1B => self.ncc(0, sf, lm),
            0x1C => self.cc(sf, lm),
            0x1E => self.nc(0, sf, lm),
            0x20 => { self.nc(0, sf, lm); self.nc(1, sf, lm); self.nc(2, sf, lm) }
            0x28 => self.sqr(sf, lm),
            0x29 => self.dcpl(sf, lm),
            0x2A => { self.dpcs(sf, lm, true); self.dpcs(sf, lm, true); self.dpcs(sf, lm, true) }
            0x2D => self.avsz3(),
            0x2E => self.avsz4(),
            0x30 => { self.rtp(0, sf, lm, false); self.rtp(1, sf, lm, false); self.rtp(2, sf, lm, true) }
            0x3D => self.gpf(sf, lm),
            0x3E => self.gpl(sf, lm),
            0x3F => { self.ncc(0, sf, lm); self.ncc(1, sf, lm); self.ncc(2, sf, lm) }
            _ => {}
        }
        self.update_flag_summary();
    }

    fn rtp(&mut self, idx: usize, sf: u32, lm: bool, last: bool) {
        let v = self.v[idx];
        let mut z_raw = 0i64;
        for i in 0..3 {
            let m = self.rt[i];
            let mut acc = (self.tr[i] as i64) << 12;
            acc = self.check_mac(i + 1, acc + m[0] as i64 * v[0] as i64);
            acc = self.check_mac(i + 1, acc + m[1] as i64 * v[1] as i64);
            acc = acc + m[2] as i64 * v[2] as i64;
            let r = self.set_mac(i + 1, acc, sf);
            if i == 2 {
                z_raw = acc;
                // IR3 saturation flag is based on the >>12 value even when sf=0.
                let saved = self.flag;
                self.set_ir(3, r, lm);
                let ir3 = self.ir[3];
                self.flag = saved;
                let probe = acc >> 12;
                if probe > 0x7FFF || probe < -0x8000 {
                    self.flag |= 1 << 22;
                }
                self.ir[3] = ir3;
            } else {
                self.set_ir(i + 1, r, lm);
            }
        }
        self.push_sz(z_raw >> 12);
        let div = self.divide();
        let x = self.set_mac0(div * self.ir[1] as i64 + self.ofx as i64);
        let y = self.set_mac0(div * self.ir[2] as i64 + self.ofy as i64);
        self.push_sxy(x >> 16, y >> 16);
        if last {
            let d = self.set_mac0(div * self.dqa as i64 + self.dqb as i64);
            self.set_ir0(d >> 12);
        }
    }

    fn nclip(&mut self) {
        let [(x0, y0), (x1, y1), (x2, y2)] = self.sxy;
        let (x0, y0, x1, y1, x2, y2) = (x0 as i64, y0 as i64, x1 as i64, y1 as i64, x2 as i64, y2 as i64);
        self.set_mac0(x0 * y1 + x1 * y2 + x2 * y0 - x0 * y2 - x1 * y0 - x2 * y1);
    }

    fn op(&mut self, sf: u32, lm: bool) {
        let (d1, d2, d3) = (self.rt[0][0] as i64, self.rt[1][1] as i64, self.rt[2][2] as i64);
        let (i1, i2, i3) = (self.ir[1] as i64, self.ir[2] as i64, self.ir[3] as i64);
        self.set_mac_ir(1, i3 * d2 - i2 * d3, sf, lm);
        self.set_mac_ir(2, i1 * d3 - i3 * d1, sf, lm);
        self.set_mac_ir(3, i2 * d1 - i1 * d2, sf, lm);
    }

    /// MAC = (base<<12) + M*v, with overflow checks after each add.
    fn mat_vec(&mut self, m: [[i16; 3]; 3], v: [i16; 3], base: [i32; 3], sf: u32, lm: bool) {
        for i in 0..3 {
            let mut acc = (base[i] as i64) << 12;
            acc = self.check_mac(i + 1, acc + m[i][0] as i64 * v[0] as i64);
            acc = self.check_mac(i + 1, acc + m[i][1] as i64 * v[1] as i64);
            acc += m[i][2] as i64 * v[2] as i64;
            self.set_mac_ir(i + 1, acc, sf, lm);
        }
    }

    fn ir_vec(&self) -> [i16; 3] {
        [self.ir[1], self.ir[2], self.ir[3]]
    }

    fn mvmva(&mut self, cmd: u32, sf: u32, lm: bool) {
        let mx = (cmd >> 17) & 3;
        let vi = ((cmd >> 15) & 3) as usize;
        let cv = (cmd >> 13) & 3;
        let m = match mx {
            0 => self.rt,
            1 => self.l,
            2 => self.lr,
            _ => {
                // "Garbage" matrix documented for mx=3.
                let r = -((self.rgbc[0] as i16) << 4);
                [[r, (self.rgbc[0] as i16) << 4, self.ir[0]], [self.rt[0][2]; 3], [self.rt[1][1]; 3]]
            }
        };
        let v = if vi == 3 { self.ir_vec() } else { self.v[vi] };
        match cv {
            0 => self.mat_vec(m, v, self.tr, sf, lm),
            1 => self.mat_vec(m, v, self.bk, sf, lm),
            2 => {
                // Far colour as translation is bugged: the first column only
                // affects flags, the result uses the last two columns.
                for i in 0..3 {
                    let first = ((self.fc[i] as i64) << 12) + m[i][0] as i64 * v[0] as i64;
                    let f = self.check_mac(i + 1, first);
                    let saved = self.ir[i + 1];
                    self.set_ir(i + 1, f >> sf, false);
                    self.ir[i + 1] = saved;
                    let mut acc = m[i][1] as i64 * v[1] as i64;
                    acc = self.check_mac(i + 1, acc);
                    acc += m[i][2] as i64 * v[2] as i64;
                    self.set_mac_ir(i + 1, acc, sf, lm);
                }
            }
            _ => self.mat_vec(m, v, [0; 3], sf, lm),
        }
    }

    fn rgb_times_ir(&mut self, sf: u32, lm: bool) {
        let c = self.rgbc;
        for i in 0..3 {
            let v = ((c[i] as i64) << 4) * self.ir[i + 1] as i64;
            self.set_mac_ir(i + 1, v, sf, lm);
        }
    }

    /// Depth-cue interpolation of MAC1..3 (unshifted inputs) toward FC.
    fn interpolate(&mut self, inp: [i64; 3], sf: u32, lm: bool) {
        for i in 0..3 {
            self.set_mac_ir(i + 1, ((self.fc[i] as i64) << 12) - inp[i], sf, false);
        }
        for i in 0..3 {
            let v = self.ir[i + 1] as i64 * self.ir[0] as i64 + inp[i];
            self.set_mac_ir(i + 1, v, sf, lm);
        }
    }

    fn light(&mut self, idx: usize, sf: u32, lm: bool) {
        let v = self.v[idx];
        self.mat_vec(self.l, v, [0; 3], sf, lm);
        let ir = self.ir_vec();
        self.mat_vec(self.lr, ir, self.bk, sf, lm);
    }

    fn nc(&mut self, idx: usize, sf: u32, lm: bool) {
        self.light(idx, sf, lm);
        self.push_color();
    }

    fn ncc(&mut self, idx: usize, sf: u32, lm: bool) {
        self.light(idx, sf, lm);
        self.rgb_times_ir(sf, lm);
        self.push_color();
    }

    fn ncd(&mut self, idx: usize, sf: u32, lm: bool) {
        self.light(idx, sf, lm);
        let c = self.rgbc;
        let inp = [0, 1, 2].map(|i| ((c[i] as i64) << 4) * self.ir[i + 1] as i64);
        self.interpolate(inp, sf, lm);
        self.push_color();
    }

    fn cc(&mut self, sf: u32, lm: bool) {
        let ir = self.ir_vec();
        self.mat_vec(self.lr, ir, self.bk, sf, lm);
        self.rgb_times_ir(sf, lm);
        self.push_color();
    }

    fn cdp(&mut self, sf: u32, lm: bool) {
        let ir = self.ir_vec();
        self.mat_vec(self.lr, ir, self.bk, sf, lm);
        let c = self.rgbc;
        let inp = [0, 1, 2].map(|i| ((c[i] as i64) << 4) * self.ir[i + 1] as i64);
        self.interpolate(inp, sf, lm);
        self.push_color();
    }

    fn dpcs(&mut self, sf: u32, lm: bool, fifo: bool) {
        let c = if fifo { self.rgb[0] } else { self.rgbc };
        let inp = [0, 1, 2].map(|i| (c[i] as i64) << 16);
        self.interpolate(inp, sf, lm);
        self.push_color();
    }

    fn intpl(&mut self, sf: u32, lm: bool) {
        let inp = [1, 2, 3].map(|i| (self.ir[i] as i64) << 12);
        self.interpolate(inp, sf, lm);
        self.push_color();
    }

    fn dcpl(&mut self, sf: u32, lm: bool) {
        let c = self.rgbc;
        let inp = [0, 1, 2].map(|i| ((c[i] as i64) << 4) * self.ir[i + 1] as i64);
        self.interpolate(inp, sf, lm);
        self.push_color();
    }

    fn sqr(&mut self, sf: u32, lm: bool) {
        for i in 1..4 {
            let v = self.ir[i] as i64;
            self.set_mac_ir(i, v * v, sf, lm);
        }
    }

    fn avsz3(&mut self) {
        let s = self.sz[1] as i64 + self.sz[2] as i64 + self.sz[3] as i64;
        let v = self.set_mac0(self.zsf3 as i64 * s);
        self.set_otz(v >> 12);
    }

    fn avsz4(&mut self) {
        let s = self.sz[0] as i64 + self.sz[1] as i64 + self.sz[2] as i64 + self.sz[3] as i64;
        let v = self.set_mac0(self.zsf4 as i64 * s);
        self.set_otz(v >> 12);
    }

    fn set_otz(&mut self, v: i64) {
        self.otz = if v > 0xFFFF {
            self.flag |= 1 << 18;
            0xFFFF
        } else if v < 0 {
            self.flag |= 1 << 18;
            0
        } else {
            v as u16
        };
    }

    fn gpf(&mut self, sf: u32, lm: bool) {
        for i in 1..4 {
            let v = self.ir[0] as i64 * self.ir[i] as i64;
            self.set_mac_ir(i, v, sf, lm);
        }
        self.push_color();
    }

    fn gpl(&mut self, sf: u32, lm: bool) {
        for i in 1..4 {
            let v = ((self.mac[i] as i64) << sf) + self.ir[0] as i64 * self.ir[i] as i64;
            self.set_mac_ir(i, v, sf, lm);
        }
        self.push_color();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unr_table_matches_known_values() {
        assert_eq!(UNR_TABLE[0], 0xFF);
        assert_eq!(UNR_TABLE[0x100], 0x00);
    }

    #[test]
    fn rtps_projects_a_point() {
        let mut g = Gte::new();
        // Identity rotation (1.0 = 0x1000), no translation.
        g.write_ctrl(0, 0x1000);
        g.write_ctrl(2, 0x1000);
        g.write_ctrl(4, 0x1000);
        g.write_ctrl(24, 160 << 16); // OFX
        g.write_ctrl(25, 120 << 16); // OFY
        g.write_ctrl(26, 200); // H
        g.write_data(0, (10u32 << 16) | 20); // VX=20, VY=10
        g.write_data(1, 400); // VZ
        g.command(0x0018_0001); // RTPS sf=1
        let sxy = g.read_data(14);
        let sx = sxy as i16;
        let sy = (sxy >> 16) as i16;
        // x' = 160 + 20 * 200/400 = 170, y' = 120 + 10 * 0.5 = 125
        assert_eq!((sx, sy), (170, 125));
        assert_eq!(g.read_data(19), 400);
        assert_eq!(g.read_ctrl(31) & 0x8000_0000, 0);
    }

    #[test]
    fn nclip_sign() {
        let mut g = Gte::new();
        g.write_data(12, 0);
        g.write_data(13, 10);
        g.write_data(14, 10 << 16);
        g.command(0x06);
        assert!((g.read_data(24) as i32) > 0);
    }

    #[test]
    fn lzcr_counts_leading_bits() {
        let mut g = Gte::new();
        g.write_data(30, 0x0000_FFFF);
        assert_eq!(g.read_data(31), 16);
        g.write_data(30, 0xFFF0_0000);
        assert_eq!(g.read_data(31), 12);
    }
}
