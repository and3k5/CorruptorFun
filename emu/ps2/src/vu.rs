//! Vector units VU0 and VU1: 32 four-float registers, 16 integer
//! registers, and microprograms of 64-bit instruction pairs (an "upper"
//! FMAC op and a "lower" op). VU0 also runs single upper ops on behalf of
//! the EE (COP2 macro mode). VU1 sends its GIF packets to the GS with
//! XGKICK. Programs run to completion when started.

use crate::hw::Hw;

#[derive(Clone)]
pub struct Vu {
    pub vf: [[f32; 4]; 32],
    pub vi: [u16; 16],
    pub acc: [f32; 4],
    pub q: f32,
    pub p: f32,
    pub i: f32,
    pub r: u32,
    pub mac: u32,
    pub status: u32,
    pub clip: u32,
    pub tpc: u32,
    pub cmsar: u32,
    pub runs: u32,
    pub steps: u64,
    pub unknown: std::collections::BTreeMap<String, u32>,
}

impl Default for Vu {
    fn default() -> Self {
        let mut vf = [[0.0; 4]; 32];
        vf[0] = [0.0, 0.0, 0.0, 1.0];
        Vu { vf, vi: [0; 16], acc: [0.0; 4], q: 0.0, p: 0.0, i: 0.0, r: 0, mac: 0, status: 0, clip: 0, tpc: 0, cmsar: 0, runs: 0, steps: 0, unknown: Default::default() }
    }
}

#[inline(always)]
fn clamp(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        f32::from_bits(0x7F7F_FFFF | (x.to_bits() & 0x8000_0000))
    }
}

/// Denormals read as zero.
#[inline(always)]
fn rd(x: f32) -> f32 {
    if x.to_bits() & 0x7F80_0000 == 0 { f32::from_bits(x.to_bits() & 0x8000_0000) } else { x }
}

const MAX_STEPS: u64 = 4_000_000;

impl Vu {
    pub fn vf_bits(&self, r: usize) -> [u32; 4] {
        self.vf[r].map(|f| f.to_bits())
    }

    pub fn set_vf_bits(&mut self, r: usize, v: [u32; 4]) {
        if r != 0 {
            self.vf[r] = v.map(f32::from_bits);
        }
    }

    fn set_vi(&mut self, r: usize, v: u32) {
        if r != 0 {
            self.vi[r & 15] = v as u16;
        }
    }

    /// Writes `res` to vf[fd] for the components in `dest` (bit 3 = x)
    /// and updates the MAC/status flags.
    fn store(&mut self, fd: usize, dest: u32, res: [f32; 4], to_acc: bool) {
        let mut mac = 0u32;
        for k in 0..4 {
            if dest & (8 >> k) == 0 {
                continue;
            }
            let v = clamp(res[k]);
            if v == 0.0 {
                mac |= 1 << (3 - k);
            }
            if v.is_sign_negative() && v != 0.0 {
                mac |= 1 << (7 - k);
            }
            if to_acc {
                self.acc[k] = v;
            } else if fd != 0 {
                self.vf[fd][k] = v;
            }
        }
        self.mac = mac;
        let z = (mac & 0xF != 0) as u32;
        let s = (mac & 0xF0 != 0) as u32;
        self.status = (self.status & !0x3) | z | s << 1 | (self.status & 0xFC0) | z << 6 | s << 7;
    }

    /// One upper (FMAC) instruction. Used by both micro and macro mode.
    pub fn upper(&mut self, op: u32) {
        let dest = (op >> 21) & 0xF;
        let ft = ((op >> 16) & 31) as usize;
        let fs = ((op >> 11) & 31) as usize;
        let fd = ((op >> 6) & 31) as usize;
        let f = op & 0x3F;
        let a = self.vf[fs].map(rd);
        let b = self.vf[ft].map(rd);
        let acc = self.acc;
        let bc = (op & 3) as usize;
        let bcv = [b[bc]; 4];
        let qv = [self.q; 4];
        let iv = [self.i; 4];
        let add = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|k| x[k] + y[k]);
        let sub = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|k| x[k] - y[k]);
        let mul = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|k| x[k] * y[k]);
        let max = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|k| x[k].max(y[k]));
        let min = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|k| x[k].min(y[k]));
        match f {
            0x00..=0x03 => self.store(fd, dest, add(a, bcv), false),
            0x04..=0x07 => self.store(fd, dest, sub(a, bcv), false),
            0x08..=0x0B => self.store(fd, dest, add(acc, mul(a, bcv)), false),
            0x0C..=0x0F => self.store(fd, dest, sub(acc, mul(a, bcv)), false),
            0x10..=0x13 => self.store(fd, dest, max(a, bcv), false),
            0x14..=0x17 => self.store(fd, dest, min(a, bcv), false),
            0x18..=0x1B => self.store(fd, dest, mul(a, bcv), false),
            0x1C => self.store(fd, dest, mul(a, qv), false),
            0x1D => self.store(fd, dest, max(a, iv), false),
            0x1E => self.store(fd, dest, mul(a, iv), false),
            0x1F => self.store(fd, dest, min(a, iv), false),
            0x20 => self.store(fd, dest, add(a, qv), false),
            0x21 => self.store(fd, dest, add(acc, mul(a, qv)), false),
            0x22 => self.store(fd, dest, add(a, iv), false),
            0x23 => self.store(fd, dest, add(acc, mul(a, iv)), false),
            0x24 => self.store(fd, dest, sub(a, qv), false),
            0x25 => self.store(fd, dest, sub(acc, mul(a, qv)), false),
            0x26 => self.store(fd, dest, sub(a, iv), false),
            0x27 => self.store(fd, dest, sub(acc, mul(a, iv)), false),
            0x28 => self.store(fd, dest, add(a, b), false),
            0x29 => self.store(fd, dest, add(acc, mul(a, b)), false),
            0x2A => self.store(fd, dest, mul(a, b), false),
            0x2B => self.store(fd, dest, max(a, b), false),
            0x2C => self.store(fd, dest, sub(a, b), false),
            0x2D => self.store(fd, dest, sub(acc, mul(a, b)), false),
            0x2E => {
                // OPMSUB: fd = acc - fs × ft (cross product, second half)
                let r = [acc[0] - a[1] * b[2], acc[1] - a[2] * b[0], acc[2] - a[0] * b[1], 0.0];
                self.store(fd, dest & 0xE, r, false)
            }
            0x2F => self.store(fd, dest, min(a, b), false),
            0x3C..=0x3F => {
                let sub_op = ((op >> 6) & 0x1F) << 2 | (op & 3);
                match sub_op {
                    0x00..=0x03 => self.store(0, dest, add(a, bcv), true),
                    0x04..=0x07 => self.store(0, dest, sub(a, bcv), true),
                    0x08..=0x0B => self.store(0, dest, add(acc, mul(a, bcv)), true),
                    0x0C..=0x0F => self.store(0, dest, sub(acc, mul(a, bcv)), true),
                    0x10..=0x13 => {
                        let sc = [1.0, 16.0, 4096.0, 32768.0][(sub_op & 3) as usize];
                        let src = self.vf[fs];
                        let r = src.map(|x| x.to_bits() as i32 as f32 / sc);
                        for k in 0..4 {
                            if dest & (8 >> k) != 0 && ft != 0 {
                                self.vf[ft][k] = r[k];
                            }
                        }
                    }
                    0x14..=0x17 => {
                        let sc = [1.0, 16.0, 4096.0, 32768.0][(sub_op & 3) as usize];
                        let src = self.vf[fs];
                        for k in 0..4 {
                            if dest & (8 >> k) != 0 && ft != 0 {
                                let v = (src[k] * sc).clamp(-2147483648.0, 2147483647.0) as i32;
                                self.vf[ft][k] = f32::from_bits(v as u32);
                            }
                        }
                    }
                    0x18..=0x1B => self.store(0, dest, mul(a, bcv), true),
                    0x1C => self.store(0, dest, mul(a, qv), true),
                    0x1D => {
                        for k in 0..4 {
                            if dest & (8 >> k) != 0 && ft != 0 {
                                self.vf[ft][k] = a[k].abs();
                            }
                        }
                    }
                    0x1E => self.store(0, dest, mul(a, iv), true),
                    0x1F => {
                        // CLIP: compare fs.xyz with ±|ft.w|.
                        let w = b[3].abs();
                        let mut c = 0;
                        if a[0] > w { c |= 1 }
                        if a[0] < -w { c |= 2 }
                        if a[1] > w { c |= 4 }
                        if a[1] < -w { c |= 8 }
                        if a[2] > w { c |= 16 }
                        if a[2] < -w { c |= 32 }
                        self.clip = ((self.clip << 6) | c) & 0xFF_FFFF;
                    }
                    0x20 => self.store(0, dest, add(a, qv), true),
                    0x21 => self.store(0, dest, add(acc, mul(a, qv)), true),
                    0x22 => self.store(0, dest, add(a, iv), true),
                    0x23 => self.store(0, dest, add(acc, mul(a, iv)), true),
                    0x24 => self.store(0, dest, sub(a, qv), true),
                    0x25 => self.store(0, dest, sub(acc, mul(a, qv)), true),
                    0x26 => self.store(0, dest, sub(a, iv), true),
                    0x27 => self.store(0, dest, sub(acc, mul(a, iv)), true),
                    0x28 => self.store(0, dest, add(a, b), true),
                    0x29 => self.store(0, dest, add(acc, mul(a, b)), true),
                    0x2A => self.store(0, dest, mul(a, b), true),
                    0x2C => self.store(0, dest, sub(a, b), true),
                    0x2D => self.store(0, dest, sub(acc, mul(a, b)), true),
                    0x2E => {
                        // OPMULA: acc = fs × ft (cross product, first half)
                        let r = [a[1] * b[2], a[2] * b[0], a[0] * b[1], 0.0];
                        self.store(0, dest & 0xE, r, true)
                    }
                    0x2F => {}
                    _ => *self.unknown.entry(format!("upper special {sub_op:02X}")).or_insert(0) += 1,
                }
            }
            _ => *self.unknown.entry(format!("upper {f:02X}")).or_insert(0) += 1,
        }
    }

    fn comp(&self, r: usize, f: u32) -> f32 {
        rd(self.vf[r][f as usize])
    }

    /// Lower-instruction ops shared with macro mode (MOVE, DIV, …).
    /// Returns false when the op isn't one of them.
    fn common_lower(&mut self, sub_op: u32, op: u32, mem: &mut [u8]) -> bool {
        let dest = (op >> 21) & 0xF;
        let ft = ((op >> 16) & 31) as usize;
        let fs = ((op >> 11) & 31) as usize;
        let fsf = (op >> 21) & 3;
        let ftf = (op >> 23) & 3;
        let mask = mem.len() - 1;
        let ld = |m: &[u8], a: usize| -> [u32; 4] {
            let a = (a * 16) & mask;
            [0, 1, 2, 3].map(|k| u32::from_le_bytes(m[a + k * 4..a + k * 4 + 4].try_into().unwrap()))
        };
        match sub_op {
            0x30 => {
                // MOVE
                let v = self.vf[fs];
                for k in 0..4 {
                    if dest & (8 >> k) != 0 && ft != 0 {
                        self.vf[ft][k] = v[k];
                    }
                }
            }
            0x31 => {
                // MR32
                let v = self.vf[fs];
                let r = [v[1], v[2], v[3], v[0]];
                for k in 0..4 {
                    if dest & (8 >> k) != 0 && ft != 0 {
                        self.vf[ft][k] = r[k];
                    }
                }
            }
            0x34..=0x37 => {
                // LQI / SQI / LQD / SQD
                let (vr, store) = match sub_op {
                    0x34 => (fs, false),
                    0x35 => (ft, true),
                    0x36 => (fs, false),
                    _ => (ft, true),
                };
                let is = vr & 15;
                if sub_op >= 0x36 {
                    let v = self.vi[is].wrapping_sub(1);
                    self.set_vi(is, v as u32);
                }
                let addr = self.vi[is] as usize;
                if store {
                    let a = (addr * 16) & mask;
                    let v = self.vf_bits(fs);
                    for k in 0..4 {
                        if dest & (8 >> k) != 0 {
                            mem[a + k * 4..a + k * 4 + 4].copy_from_slice(&v[k].to_le_bytes());
                        }
                    }
                } else {
                    let v = ld(mem, addr);
                    for k in 0..4 {
                        if dest & (8 >> k) != 0 && ft != 0 {
                            self.vf[ft][k] = f32::from_bits(v[k]);
                        }
                    }
                }
                if sub_op < 0x36 {
                    let v = self.vi[is].wrapping_add(1);
                    self.set_vi(is, v as u32);
                }
            }
            0x38 => {
                let d = self.comp(ft, ftf);
                let n = self.comp(fs, fsf);
                self.q = if d == 0.0 { f32::from_bits(0x7F7F_FFFF | ((n.to_bits() ^ d.to_bits()) & 0x8000_0000)) } else { clamp(n / d) };
            }
            0x39 => self.q = self.comp(ft, ftf).abs().sqrt(),
            0x3A => {
                let d = self.comp(ft, ftf).abs().sqrt();
                let n = self.comp(fs, fsf);
                self.q = if d == 0.0 { f32::from_bits(0x7F7F_FFFF | (n.to_bits() & 0x8000_0000)) } else { clamp(n / d) };
            }
            0x3B => {}
            0x3C => {
                let v = self.vf[fs][fsf as usize].to_bits();
                self.set_vi(ft & 15, v & 0xFFFF);
            }
            0x3D => {
                let v = self.vi[fs & 15] as i16 as i32 as u32;
                for k in 0..4 {
                    if dest & (8 >> k) != 0 && ft != 0 {
                        self.vf[ft][k] = f32::from_bits(v);
                    }
                }
            }
            0x3E | 0x3F => {
                // ILWR / ISWR
                let (it, is) = (ft & 15, fs & 15);
                let a = (self.vi[is] as usize * 16) & mask;
                for k in 0..4 {
                    if dest & (8 >> k) == 0 {
                        continue;
                    }
                    if sub_op == 0x3E {
                        let v = u32::from_le_bytes(mem[a + k * 4..a + k * 4 + 4].try_into().unwrap());
                        self.set_vi(it, v & 0xFFFF);
                        break;
                    } else {
                        let v = self.vi[it] as u32;
                        mem[a + k * 4..a + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
            0x40 | 0x41 => {
                if sub_op == 0x40 {
                    // RNEXT: xorshift-style step of the 23-bit generator.
                    let x = self.r;
                    let bit = ((x >> 4) ^ (x >> 22)) & 1;
                    self.r = ((x << 1) | bit) & 0x7F_FFFF;
                }
                let v = f32::from_bits(0x3F80_0000 | (self.r & 0x7F_FFFF));
                for k in 0..4 {
                    if dest & (8 >> k) != 0 && ft != 0 {
                        self.vf[ft][k] = v;
                    }
                }
            }
            0x42 => self.r = self.vf[fs][fsf as usize].to_bits() & 0x7F_FFFF,
            0x43 => self.r ^= self.vf[fs][fsf as usize].to_bits() & 0x7F_FFFF,
            _ => return false,
        }
        true
    }
}

/// COP2 macro instruction from the EE (VU0).
pub fn macro_op(hw: &mut Hw, i: u32) {
    let f = i & 0x3F;
    if f >= 0x3C {
        let sub_op = ((i >> 6) & 0x1F) << 2 | (i & 3);
        if sub_op >= 0x30 {
            let mut vu = std::mem::take(&mut hw.vu0);
            let ok = vu.common_lower(sub_op, i, &mut hw.vu0_data);
            if !ok {
                *vu.unknown.entry(format!("macro {sub_op:02X}")).or_insert(0) += 1;
            }
            hw.vu0 = vu;
            return;
        }
        hw.vu0.upper(i);
        return;
    }
    match f {
        0x30..=0x35 => {
            let it = ((i >> 16) & 15) as usize;
            let is = ((i >> 11) & 15) as usize;
            let id = ((i >> 6) & 15) as usize;
            let vi = hw.vu0.vi;
            let v = match f {
                0x30 => vi[is].wrapping_add(vi[it]) as u32,
                0x31 => vi[is].wrapping_sub(vi[it]) as u32,
                0x32 => (vi[is] as i32 + (((i >> 6) as i32) << 27 >> 27)) as u32,
                0x34 => (vi[is] & vi[it]) as u32,
                _ => (vi[is] | vi[it]) as u32,
            };
            let dst = if f == 0x32 { it } else { id };
            hw.vu0.set_vi(dst, v);
        }
        0x38 => run(hw, 0, Some(((i >> 6) & 0x7FFF) * 8)),
        0x39 => {
            let a = hw.vu0.cmsar;
            run(hw, 0, Some(a * 8));
        }
        _ => hw.vu0.upper(i),
    }
}

fn reset(v: &mut Vu) {
    let (runs, steps, unknown) = (v.runs, v.steps, std::mem::take(&mut v.unknown));
    *v = Vu { runs, steps, unknown, ..Default::default() };
}

/// CFC2: integer and control registers of VU0.
pub fn read_ctrl(hw: &Hw, r: usize) -> u32 {
    let v = &hw.vu0;
    match r {
        0..=15 => v.vi[r] as u32,
        16 => v.status,
        17 => v.mac,
        18 => v.clip,
        20 => v.r,
        21 => v.i.to_bits(),
        22 => v.q.to_bits(),
        26 => v.tpc,
        27 => v.cmsar,
        28 => 0,
        29 => 0, // VPU-STAT: both VUs idle
        31 => hw.vu1.cmsar,
        _ => 0,
    }
}

/// CTC2.
pub fn write_ctrl(hw: &mut Hw, r: usize, val: u32) {
    let v = &mut hw.vu0;
    match r {
        0 => {}
        1..=15 => v.vi[r] = val as u16,
        16 => v.status = val & 0xFC0 | (v.status & 0x3F),
        18 => v.clip = val & 0xFF_FFFF,
        20 => v.r = val & 0x7F_FFFF,
        21 => v.i = f32::from_bits(val),
        22 => v.q = f32::from_bits(val),
        27 => v.cmsar = val & 0xFFFF,
        28 => {
            // FBRST: reset bits clear the VU state.
            if val & 2 != 0 { reset(&mut hw.vu0); }
            if val & 0x200 != 0 { reset(&mut hw.vu1); }
        }
        31 => {
            hw.vu1.cmsar = val & 0xFFFF;
            run(hw, 1, Some((val & 0xFFFF) * 8));
        }
        _ => {}
    }
}

/// Starts a microprogram (MSCAL/MSCNT from VIF, VCALLMS, CMSAR1).
pub fn start(hw: &mut Hw, which: usize, addr: Option<u32>) {
    run(hw, which, addr);
}

fn run(hw: &mut Hw, which: usize, addr: Option<u32>) {
    let mut vu = std::mem::take(if which == 1 { &mut hw.vu1 } else { &mut hw.vu0 });
    vu.runs += 1;
    let mut pc = addr.unwrap_or(vu.tpc * 8);
    let code_mask = if which == 1 { 0x3FFF } else { 0xFFF };
    let mut branch: Option<u32> = None;
    let mut end_after = false;
    let mut steps = 0u64;
    loop {
        let a = (pc & code_mask) as usize;
        let code = if which == 1 { &hw.vu1_code } else { &hw.vu0_code };
        let lower = u32::from_le_bytes(code[a..a + 4].try_into().unwrap());
        let upper = u32::from_le_bytes(code[a + 4..a + 8].try_into().unwrap());
        let next = pc.wrapping_add(8);
        let pending = branch.take();
        // Upper first, but the lower reads registers from before it.
        let snapshot = vu.vf;
        vu.upper(upper & 0x07FF_FFFF);
        if upper & 0x8000_0000 != 0 {
            vu.i = f32::from_bits(lower);
        } else {
            let new_vf = vu.vf;
            vu.vf = snapshot;
            let upper_dest = ((upper >> 6) & 31) as usize;
            if let Some(t) = lower_op(hw, which, &mut vu, lower, pc) {
                branch = Some(t);
            }
            // Merge: the upper result wins on its own destination.
            let after_lower = vu.vf;
            vu.vf = after_lower;
            for r in 1..32 {
                if new_vf[r] != snapshot[r] && (r == upper_dest || after_lower[r] == snapshot[r]) {
                    vu.vf[r] = new_vf[r];
                }
            }
        }
        vu.vf[0] = [0.0, 0.0, 0.0, 1.0];
        steps += 1;
        if end_after || steps > MAX_STEPS {
            vu.tpc = next / 8;
            break;
        }
        if upper & 0x4000_0000 != 0 {
            end_after = true;
        }
        pc = pending.unwrap_or(next);
    }
    vu.steps += steps;
    if which == 1 {
        hw.vu1 = vu;
    } else {
        hw.vu0 = vu;
    }
}

/// Executes a lower instruction. Returns a branch target, if any.
fn lower_op(hw: &mut Hw, which: usize, vu: &mut Vu, op: u32, pc: u32) -> Option<u32> {
    let it = ((op >> 16) & 15) as usize;
    let is = ((op >> 11) & 15) as usize;
    let dest = (op >> 21) & 0xF;
    let imm11 = ((op & 0x7FF) as i32) << 21 >> 21;
    let mem_mask = if which == 1 { 0x3FFF } else { 0xFFF };
    let target = |imm: i32| (pc as i32 + 8 + imm * 8) as u32;
    let top = op >> 25;
    if top == 0x40 {
        let f = op & 0x3F;
        match f {
            0x30 | 0x31 | 0x34 | 0x35 => {
                let id = ((op >> 6) & 15) as usize;
                let (a, b) = (vu.vi[is], vu.vi[it]);
                let v = match f {
                    0x30 => a.wrapping_add(b),
                    0x31 => a.wrapping_sub(b),
                    0x34 => a & b,
                    _ => a | b,
                };
                vu.set_vi(id, v as u32);
            }
            0x32 => {
                let imm5 = (((op >> 6) & 31) as i32) << 27 >> 27;
                let v = (vu.vi[is] as i32 + imm5) as u32;
                vu.set_vi(it, v);
            }
            0x3C..=0x3F => {
                let sub_op = ((op >> 6) & 0x1F) << 2 | (op & 3);
                let mem = if which == 1 { &mut hw.vu1_data } else { &mut hw.vu0_data };
                if vu.common_lower(sub_op, op, mem) {
                    return None;
                }
                let fs = ((op >> 11) & 31) as usize;
                let ft = ((op >> 16) & 31) as usize;
                let fsf = ((op >> 21) & 3) as usize;
                let v = vu.vf[fs].map(rd);
                match sub_op {
                    0x64 => {
                        // MFP
                        let p = vu.p;
                        for k in 0..4 {
                            if dest & (8 >> k) != 0 && ft != 0 {
                                vu.vf[ft][k] = p;
                            }
                        }
                    }
                    0x68 => vu.set_vi(it, if which == 1 { hw.vif1.top } else { 0 }),
                    0x69 => vu.set_vi(it, if which == 1 { hw.vif1.itop } else { hw.vif0.itop }),
                    0x6C => xgkick(hw, vu.vi[is] as usize),
                    0x70 => vu.p = v[0] * v[0] + v[1] * v[1] + v[2] * v[2],
                    0x71 => {
                        let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                        vu.p = if s == 0.0 { 0.0 } else { 1.0 / s };
                    }
                    0x72 => vu.p = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(),
                    0x73 => {
                        let s = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                        vu.p = if s == 0.0 { 0.0 } else { 1.0 / s };
                    }
                    0x74 => vu.p = v[1].atan2(v[0]),
                    0x75 => vu.p = v[2].atan2(v[0]),
                    0x76 => vu.p = v[0] + v[1] + v[2] + v[3],
                    0x78 => vu.p = v[fsf].abs().sqrt(),
                    0x79 => {
                        let s = v[fsf].abs().sqrt();
                        vu.p = if s == 0.0 { 0.0 } else { 1.0 / s };
                    }
                    0x7A => vu.p = if v[fsf] == 0.0 { 0.0 } else { 1.0 / v[fsf] },
                    0x7B => {}
                    0x7C => vu.p = v[fsf].sin(),
                    0x7D => vu.p = v[fsf].atan(),
                    0x7E => vu.p = (-v[fsf]).exp(),
                    _ => *vu.unknown.entry(format!("lower special {sub_op:02X}")).or_insert(0) += 1,
                }
            }
            _ => *vu.unknown.entry(format!("lower {f:02X}")).or_insert(0) += 1,
        }
        return None;
    }
    let mem = if which == 1 { &mut hw.vu1_data } else { &mut hw.vu0_data };
    let rd32 = |m: &[u8], a: usize| u32::from_le_bytes(m[a..a + 4].try_into().unwrap());
    match top {
        0x00 | 0x01 => {
            // LQ / SQ
            let ft = ((op >> 16) & 31) as usize;
            let fs = ((op >> 11) & 31) as usize;
            let base = if top == 0 { is } else { it };
            let a = (((vu.vi[base] as i32 + imm11) as usize) * 16) & mem_mask;
            for k in 0..4 {
                if dest & (8 >> k) == 0 {
                    continue;
                }
                if top == 0 {
                    if ft != 0 {
                        vu.vf[ft][k] = f32::from_bits(rd32(mem, a + k * 4));
                    }
                } else {
                    mem[a + k * 4..a + k * 4 + 4].copy_from_slice(&vu.vf[fs][k].to_bits().to_le_bytes());
                }
            }
        }
        0x04 | 0x05 => {
            // ILW / ISW
            let a = (((vu.vi[is] as i32 + imm11) as usize) * 16) & mem_mask;
            for k in 0..4 {
                if dest & (8 >> k) == 0 {
                    continue;
                }
                if top == 4 {
                    let v = rd32(mem, a + k * 4);
                    vu.set_vi(it, v & 0xFFFF);
                    break;
                } else {
                    mem[a + k * 4..a + k * 4 + 4].copy_from_slice(&(vu.vi[it] as u32).to_le_bytes());
                }
            }
        }
        0x08 | 0x09 => {
            let imm15 = (op & 0x7FF) | ((op >> 10) & 0x7800);
            let v = if top == 8 { vu.vi[is].wrapping_add(imm15 as u16) } else { vu.vi[is].wrapping_sub(imm15 as u16) };
            vu.set_vi(it, v as u32);
        }
        0x10 => vu.vi[1] = ((vu.clip & 0xFF_FFFF) == (op & 0xFF_FFFF)) as u16,
        0x11 => vu.clip = op & 0xFF_FFFF,
        0x12 => vu.vi[1] = (vu.clip & op & 0xFF_FFFF != 0) as u16,
        0x13 => vu.vi[1] = ((vu.clip | op) & 0xFF_FFFF == 0xFF_FFFF) as u16,
        0x14..=0x17 => {
            let imm12 = (op & 0x7FF) | ((op >> 10) & 0x800);
            match top {
                0x14 => vu.set_vi(it, (vu.status & 0xFFF == imm12) as u32),
                0x15 => vu.status = (vu.status & 0x3F) | (imm12 & 0xFC0),
                0x16 => vu.set_vi(it, vu.status & imm12),
                _ => vu.set_vi(it, vu.status | imm12),
            }
        }
        0x18 => vu.set_vi(it, (vu.mac & 0xFFFF == vu.vi[is] as u32) as u32),
        0x1A => vu.set_vi(it, vu.vi[is] as u32 & vu.mac),
        0x1B => vu.set_vi(it, vu.vi[is] as u32 | vu.mac),
        0x1C => vu.set_vi(it, vu.clip & 0xFFF),
        0x20 => return Some(target(imm11)),
        0x21 => {
            vu.set_vi(it, (pc + 16) / 8);
            return Some(target(imm11));
        }
        0x24 => return Some(vu.vi[is] as u32 * 8),
        0x25 => {
            let t = vu.vi[is] as u32 * 8;
            vu.set_vi(it, (pc + 16) / 8);
            return Some(t);
        }
        0x28..=0x2F => {
            let a = vu.vi[is] as i16;
            let b = vu.vi[it] as i16;
            let taken = match top {
                0x28 => a == b,
                0x29 => a != b,
                0x2C => a < 0,
                0x2D => a > 0,
                0x2E => a <= 0,
                0x2F => a >= 0,
                _ => false,
            };
            if taken {
                return Some(target(imm11));
            }
        }
        _ => *vu.unknown.entry(format!("lower op {top:02X}")).or_insert(0) += 1,
    }
    None
}

/// Sends the GIF packet at VU1 data address `qaddr` (in quadwords) to the
/// GS through PATH1, up to the tag with EOP set.
fn xgkick(hw: &mut Hw, mut qaddr: usize) {
    for _ in 0..1024 {
        let rdq = |m: &[u8], q: usize| -> u128 {
            let a = (q * 16) & 0x3FFF;
            u128::from_le_bytes(m[a..a + 16].try_into().unwrap())
        };
        let tag = rdq(&hw.vu1_data, qaddr);
        let t = tag as u64;
        let nloop = (t & 0x7FFF) as usize;
        let eop = t >> 15 & 1 != 0;
        let flg = (t >> 58) & 3;
        let nreg = { let n = ((t >> 60) & 0xF) as usize; if n == 0 { 16 } else { n } };
        let size = match flg {
            0 => nloop * nreg,
            1 => (nloop * nreg).div_ceil(2),
            _ => nloop,
        };
        hw.gs.gif_path_write(1, tag);
        for k in 1..=size {
            let q = rdq(&hw.vu1_data, qaddr + k);
            hw.gs.gif_path_write(1, q);
        }
        qaddr += size + 1;
        if eop {
            break;
        }
    }
}
