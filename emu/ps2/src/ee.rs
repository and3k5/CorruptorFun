//! Emotion Engine CPU: MIPS R5900 interpreter (MIPS III with 128-bit
//! registers and the MMI multimedia instructions), COP0, the FPU (COP1)
//! and the VU0 register interface (COP2).

use std::collections::BTreeMap;

use crate::hw::Hw;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exc {
    Int = 0,
    AdEL = 4,
    AdES = 5,
    Sys = 8,
    Bp = 9,
    RI = 10,
    CpU = 11,
    Ov = 12,
    Tr = 13,
}

pub struct Ee {
    /// 128-bit GPRs as [low 64, high 64].
    pub r: [[u64; 2]; 32],
    pub pc: u32,
    pub next_pc: u32,
    cur_pc: u32,
    branch: bool,
    in_delay: bool,
    pub hi: [u64; 2],
    pub lo: [u64; 2],
    pub sa: u32,
    // COP0
    pub status: u32,
    pub cause: u32,
    pub epc: u32,
    pub error_epc: u32,
    pub bad_vaddr: u32,
    pub count: u32,
    pub compare: u32,
    pub cop0: [u32; 32],
    pub tlb: [[u32; 4]; 48],
    // FPU
    pub f: [u32; 32],
    pub acc: f32,
    pub fcr31: u32,
    // VU0 registers (macro mode)
    pub vf: [[u32; 4]; 32],
    pub vi: [u32; 32],
    pub cycles: u64,
    pub trace: Vec<String>,
    pub syscalls: BTreeMap<i32, u32>,
    /// Opcodes we don't emulate yet: (name, count), shown in the report.
    pub unimpl: BTreeMap<String, u32>,
    pub exec_entry: Option<u32>,
    pub exceptions: u32,
    /// Recent jump targets, for debugging hangs.
    pub hist: [u32; 64],
    pub hist_i: usize,
}

#[inline(always)]
fn sx32(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// PS2 floats have no infinities or NaNs: they clamp to the largest value.
#[inline(always)]
fn clampf(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else if x.is_nan() {
        f32::from_bits(0x7F7F_FFFF | (x.to_bits() & 0x8000_0000))
    } else {
        f32::from_bits(0x7F7F_FFFF | (x.to_bits() & 0x8000_0000))
    }
}

#[inline(always)]
fn fbits(u: u32) -> f32 {
    // Denormals read as zero.
    if u & 0x7F80_0000 == 0 {
        f32::from_bits(u & 0x8000_0000)
    } else {
        f32::from_bits(u)
    }
}

impl Ee {
    pub fn new() -> Self {
        Ee {
            r: [[0; 2]; 32],
            pc: 0xBFC0_0000,
            next_pc: 0xBFC0_0004,
            cur_pc: 0xBFC0_0000,
            branch: false,
            in_delay: false,
            hi: [0; 2],
            lo: [0; 2],
            sa: 0,
            status: 0x0040_0004,
            cause: 0,
            epc: 0,
            error_epc: 0,
            bad_vaddr: 0,
            count: 0,
            compare: 0,
            cop0: [0; 32],
            tlb: [[0; 4]; 48],
            f: [0; 32],
            acc: 0.0,
            fcr31: 0,
            vf: [[0; 4]; 32],
            vi: [0; 32],
            cycles: 0,
            trace: Vec::new(),
            syscalls: BTreeMap::new(),
            unimpl: BTreeMap::new(),
            exec_entry: None,
            exceptions: 0,
            hist: [0; 64],
            hist_i: 0,
        }
    }

    #[inline(always)]
    fn g(&self, i: usize) -> u64 {
        self.r[i][0]
    }
    #[inline(always)]
    fn g32(&self, i: usize) -> u32 {
        self.r[i][0] as u32
    }
    #[inline(always)]
    fn s(&mut self, i: usize, v: u64) {
        if i != 0 {
            self.r[i][0] = v;
        }
    }
    #[inline(always)]
    fn q(&self, i: usize) -> u128 {
        self.r[i][0] as u128 | (self.r[i][1] as u128) << 64
    }
    #[inline(always)]
    fn sq(&mut self, i: usize, v: u128) {
        if i != 0 {
            self.r[i] = [v as u64, (v >> 64) as u64];
        }
    }

    fn note(&mut self, what: &str) {
        *self.unimpl.entry(what.to_string()).or_insert(0) += 1;
    }

    /// External interrupt lines: INT0 (INTC) → IP2, INT1 (DMAC) → IP3.
    pub fn set_int_lines(&mut self, int0: bool, int1: bool) {
        self.cause = (self.cause & !0xC00) | (int0 as u32) << 10 | (int1 as u32) << 11;
    }

    pub fn add_count(&mut self, cycles: u32) {
        let before = self.count;
        self.count = self.count.wrapping_add(cycles);
        let d = self.compare.wrapping_sub(before);
        if d != 0 && d <= cycles {
            self.cause |= 1 << 15;
        }
    }

    fn interrupts_on(&self) -> bool {
        self.status & 0x0001_0001 == 0x0001_0001 && self.status & 0x6 == 0
    }

    /// Runs one instruction; returns the cycles it took.
    pub fn step(&mut self, hw: &mut Hw) -> u32 {
        if self.interrupts_on() && self.cause & self.status & 0x8C00 != 0 {
            self.cur_pc = self.pc;
            // In a delay slot, EPC points at the branch so it runs again.
            self.in_delay = self.branch;
            self.exception(Exc::Int);
            return 2;
        }
        self.cur_pc = self.pc;
        self.in_delay = self.branch;
        self.branch = false;
        if self.cur_pc & 3 != 0 {
            self.bad_vaddr = self.cur_pc;
            self.exception(Exc::AdEL);
            return 2;
        }
        let instr = hw.ee_read32(self.cur_pc);
        self.pc = self.next_pc;
        self.next_pc = self.next_pc.wrapping_add(4);
        self.exec(instr, hw);
        2
    }

    fn exception(&mut self, e: Exc) {
        self.exceptions += 1;
        if !matches!(e, Exc::Int | Exc::Sys) && self.trace.len() < 64 {
            self.trace.push(format!("EE exception {:?} at {:08X} (bad addr {:08X})", e, self.cur_pc, self.bad_vaddr));
        }
        let bev = self.status & (1 << 22) != 0;
        if self.status & 2 == 0 {
            // EPC points at the branch when the exception hit its delay slot.
            if self.in_delay {
                self.epc = self.cur_pc.wrapping_sub(4);
                self.cause |= 1 << 31;
            } else {
                self.epc = self.cur_pc;
                self.cause &= !(1 << 31);
            }
        }
        self.cause = (self.cause & !0x7C) | ((e as u32) << 2);
        self.status |= 2;
        let off = if e == Exc::Int { 0x200 } else { 0x180 };
        let vec = if bev { 0xBFC0_0200 + off } else { 0x8000_0000 + off };
        self.pc = vec;
        self.next_pc = vec + 4;
        self.branch = false;
        self.in_delay = false;
    }

    #[inline(always)]
    fn jump(&mut self, t: u32) {
        if self.hist[self.hist_i.wrapping_sub(1) & 63] != self.cur_pc {
            self.hist[self.hist_i & 63] = self.cur_pc;
            self.hist_i = (self.hist_i + 1) & 63;
        }
        self.next_pc = t;
        self.branch = true;
    }

    #[inline(always)]
    fn br(&mut self, cond: bool, imm: u32) {
        self.branch = true;
        if cond {
            self.next_pc = self.pc.wrapping_add((imm as i16 as i32 as u32) << 2);
        }
    }

    /// Branch likely: the delay slot only runs if the branch is taken.
    #[inline(always)]
    fn brl(&mut self, cond: bool, imm: u32) {
        if cond {
            self.branch = true;
            self.next_pc = self.pc.wrapping_add((imm as i16 as i32 as u32) << 2);
        } else {
            self.pc = self.next_pc;
            self.next_pc = self.pc.wrapping_add(4);
        }
    }

    fn exec(&mut self, i: u32, hw: &mut Hw) {
        let op = i >> 26;
        let rs = ((i >> 21) & 31) as usize;
        let rt = ((i >> 16) & 31) as usize;
        let rd = ((i >> 11) & 31) as usize;
        let sh = (i >> 6) & 31;
        let imm = i & 0xFFFF;
        let se = imm as i16 as i64 as u64;
        match op {
            0x00 => self.special(i, hw, rs, rt, rd, sh),
            0x01 => {
                let v = self.g(rs) as i64;
                match rt {
                    0x00 => self.br(v < 0, imm),
                    0x01 => self.br(v >= 0, imm),
                    0x02 => self.brl(v < 0, imm),
                    0x03 => self.brl(v >= 0, imm),
                    0x08..=0x0E => {} // trap immediates: never taken in practice
                    0x10 => { let ra = self.next_pc; self.s(31, sx32(ra)); self.br(v < 0, imm) }
                    0x11 => { let ra = self.next_pc; self.s(31, sx32(ra)); self.br(v >= 0, imm) }
                    0x12 => { let ra = self.next_pc; self.s(31, sx32(ra)); self.brl(v < 0, imm) }
                    0x13 => { let ra = self.next_pc; self.s(31, sx32(ra)); self.brl(v >= 0, imm) }
                    0x18 => self.sa = (((self.g32(rs) & 0xF) ^ (imm & 0xF)) * 8) & 0x7F,
                    0x19 => self.sa = (((self.g32(rs) & 0x7) ^ (imm & 0x7)) * 16) & 0x7F,
                    _ => self.exception(Exc::RI),
                }
            }
            0x02 => self.jump((self.pc & 0xF000_0000) | ((i & 0x03FF_FFFF) << 2)),
            0x03 => {
                let ra = self.next_pc;
                self.s(31, sx32(ra));
                self.jump((self.pc & 0xF000_0000) | ((i & 0x03FF_FFFF) << 2));
            }
            0x04 => self.br(self.g(rs) == self.g(rt), imm),
            0x05 => self.br(self.g(rs) != self.g(rt), imm),
            0x06 => self.br(self.g(rs) as i64 <= 0, imm),
            0x07 => self.br(self.g(rs) as i64 > 0, imm),
            0x08 => match (self.g32(rs) as i32).checked_add(imm as i16 as i32) {
                Some(v) => self.s(rt, v as i64 as u64),
                None => self.exception(Exc::Ov),
            },
            0x09 => self.s(rt, sx32(self.g32(rs).wrapping_add(se as u32))),
            0x0A => self.s(rt, ((self.g(rs) as i64) < se as i64) as u64),
            0x0B => self.s(rt, (self.g(rs) < se) as u64),
            0x0C => self.s(rt, self.g(rs) & imm as u64),
            0x0D => self.s(rt, self.g(rs) | imm as u64),
            0x0E => self.s(rt, self.g(rs) ^ imm as u64),
            0x0F => self.s(rt, sx32(imm << 16)),
            0x10 => self.cop0_op(i, hw, rs, rt, rd),
            0x11 => self.cop1_op(i, rs, rt, rd),
            0x12 => self.cop2_op(i, rs, rt, rd),
            0x14 => self.brl(self.g(rs) == self.g(rt), imm),
            0x15 => self.brl(self.g(rs) != self.g(rt), imm),
            0x16 => self.brl(self.g(rs) as i64 <= 0, imm),
            0x17 => self.brl(self.g(rs) as i64 > 0, imm),
            0x18 => match (self.g(rs) as i64).checked_add(se as i64) {
                Some(v) => self.s(rt, v as u64),
                None => self.exception(Exc::Ov),
            },
            0x19 => self.s(rt, self.g(rs).wrapping_add(se)),
            0x1A | 0x1B => {
                // LDL / LDR
                let a = self.g32(rs).wrapping_add(se as u32);
                let m = hw.ee_read64(a & !7);
                let b = (a & 7) as u64;
                let cur = self.g(rt);
                let v = if op == 0x1A {
                    let shift = 56 - b * 8;
                    let keep = if b == 7 { 0 } else { u64::MAX >> ((b + 1) * 8) };
                    (cur & keep) | (m << shift)
                } else {
                    let shift = b * 8;
                    let keep = if b == 0 { 0 } else { u64::MAX << ((8 - b) * 8) };
                    (cur & keep) | (m >> shift)
                };
                self.s(rt, v);
            }
            0x1C => self.mmi(i, rs, rt, rd, sh),
            0x1E => {
                let a = self.g32(rs).wrapping_add(se as u32);
                let v = hw.ee_read128(a);
                self.sq(rt, v);
            }
            0x1F => {
                let a = self.g32(rs).wrapping_add(se as u32);
                let v = self.q(rt);
                hw.ee_write128(a, v);
            }
            0x20 => { let a = self.g32(rs).wrapping_add(se as u32); let v = hw.ee_read8(a) as i8 as i64 as u64; self.s(rt, v) }
            0x21 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 1 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdEL); }
                let v = hw.ee_read16(a) as i16 as i64 as u64;
                self.s(rt, v)
            }
            0x22 | 0x26 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                let m = hw.ee_read32(a & !3);
                let cur = self.g32(rt);
                let v = if op == 0x22 {
                    match a & 3 {
                        0 => (cur & 0x00FF_FFFF) | (m << 24),
                        1 => (cur & 0x0000_FFFF) | (m << 16),
                        2 => (cur & 0x0000_00FF) | (m << 8),
                        _ => m,
                    }
                } else {
                    match a & 3 {
                        0 => m,
                        1 => (cur & 0xFF00_0000) | (m >> 8),
                        2 => (cur & 0xFFFF_0000) | (m >> 16),
                        _ => (cur & 0xFFFF_FF00) | (m >> 24),
                    }
                };
                // LWR only sign-extends when it loads the whole word.
                if op == 0x26 && a & 3 != 0 {
                    let hi = self.g(rt) & 0xFFFF_FFFF_0000_0000;
                    self.s(rt, hi | v as u64);
                } else {
                    self.s(rt, sx32(v));
                }
            }
            0x23 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdEL); }
                let v = hw.ee_read32(a);
                self.s(rt, sx32(v))
            }
            0x24 => { let a = self.g32(rs).wrapping_add(se as u32); let v = hw.ee_read8(a) as u64; self.s(rt, v) }
            0x25 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 1 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdEL); }
                let v = hw.ee_read16(a) as u64;
                self.s(rt, v)
            }
            0x27 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdEL); }
                let v = hw.ee_read32(a) as u64;
                self.s(rt, v)
            }
            0x28 => { let a = self.g32(rs).wrapping_add(se as u32); hw.ee_write8(a, self.g(rt) as u8) }
            0x29 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 1 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdES); }
                hw.ee_write16(a, self.g(rt) as u16)
            }
            0x2A | 0x2E => {
                let a = self.g32(rs).wrapping_add(se as u32);
                let aa = a & !3;
                let m = hw.ee_read32(aa);
                let v = self.g32(rt);
                let out = if op == 0x2A {
                    match a & 3 {
                        0 => (m & 0xFFFF_FF00) | (v >> 24),
                        1 => (m & 0xFFFF_0000) | (v >> 16),
                        2 => (m & 0xFF00_0000) | (v >> 8),
                        _ => v,
                    }
                } else {
                    match a & 3 {
                        0 => v,
                        1 => (m & 0x0000_00FF) | (v << 8),
                        2 => (m & 0x0000_FFFF) | (v << 16),
                        _ => (m & 0x00FF_FFFF) | (v << 24),
                    }
                };
                hw.ee_write32(aa, out);
            }
            0x2B => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdES); }
                hw.ee_write32(a, self.g32(rt))
            }
            0x2C | 0x2D => {
                // SDL / SDR
                let a = self.g32(rs).wrapping_add(se as u32);
                let aa = a & !7;
                let m = hw.ee_read64(aa);
                let b = (a & 7) as u64;
                let v = self.g(rt);
                let out = if op == 0x2C {
                    let keep = if b == 7 { 0 } else { u64::MAX << ((b + 1) * 8) };
                    (m & keep) | (v >> (56 - b * 8))
                } else {
                    let keep = if b == 0 { 0 } else { u64::MAX >> ((8 - b) * 8) };
                    (m & keep) | (v << (b * 8))
                };
                hw.ee_write64(aa, out);
            }
            0x2F | 0x33 => {} // CACHE, PREF
            0x31 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdEL); }
                self.f[rt] = hw.ee_read32(a);
            }
            0x36 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                let v = hw.ee_read128(a);
                if rt != 0 {
                    self.vf[rt] = [v as u32, (v >> 32) as u32, (v >> 64) as u32, (v >> 96) as u32];
                }
            }
            0x37 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 7 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdEL); }
                let v = hw.ee_read64(a);
                self.s(rt, v)
            }
            0x39 => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdES); }
                hw.ee_write32(a, self.f[rt]);
            }
            0x3E => {
                let a = self.g32(rs).wrapping_add(se as u32);
                let f = self.vf[rt];
                let v = f[0] as u128 | (f[1] as u128) << 32 | (f[2] as u128) << 64 | (f[3] as u128) << 96;
                hw.ee_write128(a, v);
            }
            0x3F => {
                let a = self.g32(rs).wrapping_add(se as u32);
                if a & 7 != 0 { self.bad_vaddr = a; return self.exception(Exc::AdES); }
                hw.ee_write64(a, self.g(rt))
            }
            _ => {
                self.note(&format!("op {op:02X}"));
                self.exception(Exc::RI)
            }
        }
    }

    fn special(&mut self, i: u32, _hw: &mut Hw, rs: usize, rt: usize, rd: usize, sh: u32) {
        let f = i & 0x3F;
        match f {
            0x00 => self.s(rd, sx32(self.g32(rt) << sh)),
            0x02 => self.s(rd, sx32(self.g32(rt) >> sh)),
            0x03 => self.s(rd, sx32(((self.g32(rt) as i32) >> sh) as u32)),
            0x04 => self.s(rd, sx32(self.g32(rt) << (self.g32(rs) & 31))),
            0x06 => self.s(rd, sx32(self.g32(rt) >> (self.g32(rs) & 31))),
            0x07 => self.s(rd, sx32(((self.g32(rt) as i32) >> (self.g32(rs) & 31)) as u32)),
            0x08 => { let t = self.g32(rs); self.jump(t) }
            0x09 => {
                let t = self.g32(rs);
                let ra = self.next_pc;
                self.s(rd, sx32(ra));
                self.jump(t);
            }
            0x0A => if self.g(rt) == 0 { let v = self.g(rs); self.s(rd, v) },
            0x0B => if self.g(rt) != 0 { let v = self.g(rs); self.s(rd, v) },
            0x0C => {
                let n = self.g32(3) as i32;
                *self.syscalls.entry(n).or_insert(0) += 1;
                if n == 0x7C && self.trace.len() < 200 {
                    self.trace.push(format!("syscall {:X} at {:08X} a0 {:08X}", n, self.cur_pc, self.g32(4)));
                }
                if n == 7 {
                    // ExecPS2(entry, gp, argc, argv)
                    self.exec_entry = Some(self.g32(4));
                    if self.trace.len() < 200 {
                        self.trace.push(format!("ExecPS2 entry {:08X}", self.g32(4)));
                    }
                } else if n == 6 && self.trace.len() < 200 {
                    self.trace.push("LoadExecPS2".into());
                }
                self.exception(Exc::Sys)
            }
            0x0D => self.exception(Exc::Bp),
            0x0F => {} // SYNC
            0x10 => { let v = self.hi[0]; self.s(rd, v) }
            0x11 => self.hi[0] = self.g(rs),
            0x12 => { let v = self.lo[0]; self.s(rd, v) }
            0x13 => self.lo[0] = self.g(rs),
            0x14 => self.s(rd, self.g(rt) << (self.g32(rs) & 63)),
            0x16 => self.s(rd, self.g(rt) >> (self.g32(rs) & 63)),
            0x17 => self.s(rd, ((self.g(rt) as i64) >> (self.g32(rs) & 63)) as u64),
            0x18 => {
                let r = (self.g32(rs) as i32 as i64) * (self.g32(rt) as i32 as i64);
                self.lo[0] = sx32(r as u32);
                self.hi[0] = sx32((r >> 32) as u32);
                let v = self.lo[0];
                self.s(rd, v);
            }
            0x19 => {
                let r = (self.g32(rs) as u64) * (self.g32(rt) as u64);
                self.lo[0] = sx32(r as u32);
                self.hi[0] = sx32((r >> 32) as u32);
                let v = self.lo[0];
                self.s(rd, v);
            }
            0x1A => {
                let (q, r) = div_s(self.g32(rs), self.g32(rt));
                self.lo[0] = sx32(q);
                self.hi[0] = sx32(r);
            }
            0x1B => {
                let (q, r) = div_u(self.g32(rs), self.g32(rt));
                self.lo[0] = sx32(q);
                self.hi[0] = sx32(r);
            }
            0x20 => match (self.g32(rs) as i32).checked_add(self.g32(rt) as i32) {
                Some(v) => self.s(rd, v as i64 as u64),
                None => self.exception(Exc::Ov),
            },
            0x21 => self.s(rd, sx32(self.g32(rs).wrapping_add(self.g32(rt)))),
            0x22 => match (self.g32(rs) as i32).checked_sub(self.g32(rt) as i32) {
                Some(v) => self.s(rd, v as i64 as u64),
                None => self.exception(Exc::Ov),
            },
            0x23 => self.s(rd, sx32(self.g32(rs).wrapping_sub(self.g32(rt)))),
            0x24 => self.s(rd, self.g(rs) & self.g(rt)),
            0x25 => self.s(rd, self.g(rs) | self.g(rt)),
            0x26 => self.s(rd, self.g(rs) ^ self.g(rt)),
            0x27 => self.s(rd, !(self.g(rs) | self.g(rt))),
            0x28 => { let v = self.sa as u64; self.s(rd, v) }
            0x29 => self.sa = self.g32(rs) & 0x7F,
            0x2A => self.s(rd, ((self.g(rs) as i64) < (self.g(rt) as i64)) as u64),
            0x2B => self.s(rd, (self.g(rs) < self.g(rt)) as u64),
            0x2C => match (self.g(rs) as i64).checked_add(self.g(rt) as i64) {
                Some(v) => self.s(rd, v as u64),
                None => self.exception(Exc::Ov),
            },
            0x2D => self.s(rd, self.g(rs).wrapping_add(self.g(rt))),
            0x2E => match (self.g(rs) as i64).checked_sub(self.g(rt) as i64) {
                Some(v) => self.s(rd, v as u64),
                None => self.exception(Exc::Ov),
            },
            0x2F => self.s(rd, self.g(rs).wrapping_sub(self.g(rt))),
            0x30..=0x36 => {} // traps
            0x38 => self.s(rd, self.g(rt) << sh),
            0x3A => self.s(rd, self.g(rt) >> sh),
            0x3B => self.s(rd, ((self.g(rt) as i64) >> sh) as u64),
            0x3C => self.s(rd, self.g(rt) << (sh + 32)),
            0x3E => self.s(rd, self.g(rt) >> (sh + 32)),
            0x3F => self.s(rd, ((self.g(rt) as i64) >> (sh + 32)) as u64),
            _ => {
                self.note(&format!("special {f:02X}"));
                self.exception(Exc::RI)
            }
        }
    }

    fn cop0_op(&mut self, i: u32, hw: &mut Hw, rs: usize, rt: usize, rd: usize) {
        match rs {
            0x00 => {
                let v = match rd {
                    8 => self.bad_vaddr,
                    9 => self.count,
                    11 => self.compare,
                    12 => self.status,
                    13 => self.cause,
                    14 => self.epc,
                    15 => 0x2E20,
                    30 => self.error_epc,
                    _ => self.cop0[rd],
                };
                self.s(rt, sx32(v));
            }
            0x04 => {
                let v = self.g32(rt);
                match rd {
                    9 => self.count = v,
                    11 => {
                        self.compare = v;
                        self.cause &= !(1 << 15);
                    }
                    12 => self.status = v,
                    13 => self.cause = (self.cause & !0x300) | (v & 0x300),
                    14 => self.epc = v,
                    30 => self.error_epc = v,
                    _ => self.cop0[rd] = v,
                }
            }
            0x08 => {
                // BC0F / BC0T: CPCOND0 is set when every DMA channel picked
                // in D_PCR has finished.
                let pcr = hw.dmac.pcr & 0x3FF;
                let cond = (!pcr | hw.dmac.stat) & 0x3FF == 0x3FF;
                let imm = i & 0xFFFF;
                match rt {
                    0 => self.br(!cond, imm),
                    1 => self.br(cond, imm),
                    2 => self.brl(!cond, imm),
                    3 => self.brl(cond, imm),
                    _ => {}
                }
            }
            0x10 => match i & 0x3F {
                0x01 => {
                    let idx = (self.cop0[0] & 63) as usize % 48;
                    let e = self.tlb[idx];
                    self.cop0[5] = e[0];
                    self.cop0[10] = e[1];
                    self.cop0[2] = e[2];
                    self.cop0[3] = e[3];
                }
                0x02 | 0x06 => {
                    let idx = if i & 0x3F == 0x02 { (self.cop0[0] & 63) as usize % 48 } else { (self.cop0[6] as usize).min(47) };
                    self.tlb[idx] = [self.cop0[5], self.cop0[10], self.cop0[2], self.cop0[3]];
                    hw.tlb_rebuild(&self.tlb);
                }
                0x08 => {
                    let hi = self.cop0[10] & !0x1FFF;
                    self.cop0[0] = match self.tlb.iter().position(|e| e[1] & !0x1FFF == hi) {
                        Some(p) => p as u32,
                        None => 0x8000_0000,
                    };
                }
                0x18 => {
                    // ERET
                    if self.status & 4 != 0 {
                        self.pc = self.error_epc;
                        self.status &= !4;
                    } else {
                        self.pc = self.epc;
                        self.status &= !2;
                    }
                    self.next_pc = self.pc.wrapping_add(4);
                    self.branch = false;
                }
                0x38 => {
                    if self.status & 0x2_0006 != 0 || self.status & 0x18 == 0 {
                        self.status |= 0x1_0000;
                    }
                }
                0x39 => {
                    if self.status & 0x2_0006 != 0 || self.status & 0x18 == 0 {
                        self.status &= !0x1_0000;
                    }
                }
                f => self.note(&format!("cop0 {f:02X}")),
            },
            _ => self.note(&format!("cop0 rs {rs:02X}")),
        }
    }

    fn cop1_op(&mut self, i: u32, rs: usize, rt: usize, rd: usize) {
        let ft = rt;
        let fs = rd;
        let fd = ((i >> 6) & 31) as usize;
        let ff = |s: &Self, r: usize| fbits(s.f[r]);
        match rs {
            0x00 => { let v = sx32(self.f[fs]); self.s(rt, v) }
            0x02 => {
                let v = match fs { 0 => 0x2E00, 31 => self.fcr31, _ => 0 };
                self.s(rt, sx32(v));
            }
            0x04 => self.f[fs] = self.g32(rt),
            0x06 => if fs == 31 { self.fcr31 = self.g32(rt) & 0x0083_C078 | 0x0100_0001 },
            0x08 => {
                let c = self.fcr31 & (1 << 23) != 0;
                let imm = i & 0xFFFF;
                match rt {
                    0 => self.br(!c, imm),
                    1 => self.br(c, imm),
                    2 => self.brl(!c, imm),
                    3 => self.brl(c, imm),
                    _ => {}
                }
            }
            0x10 => {
                let a = ff(self, fs);
                let b = ff(self, ft);
                let set = |s: &mut Self, r: usize, v: f32| s.f[r] = clampf(v).to_bits();
                match i & 0x3F {
                    0x00 => set(self, fd, a + b),
                    0x01 => set(self, fd, a - b),
                    0x02 => set(self, fd, a * b),
                    0x03 => {
                        let v = if b == 0.0 { f32::from_bits(0x7F7F_FFFF | ((a.to_bits() ^ b.to_bits()) & 0x8000_0000)) } else { a / b };
                        set(self, fd, v)
                    }
                    0x04 => set(self, fd, b.abs().sqrt()),
                    0x05 => self.f[fd] = self.f[fs] & 0x7FFF_FFFF,
                    0x06 => self.f[fd] = self.f[fs],
                    0x07 => self.f[fd] = self.f[fs] ^ 0x8000_0000,
                    0x16 => {
                        let d = b.abs().sqrt();
                        let v = if d == 0.0 { f32::from_bits(0x7F7F_FFFF | (a.to_bits() & 0x8000_0000)) } else { a / d };
                        set(self, fd, v)
                    }
                    0x18 => self.acc = clampf(a + b),
                    0x19 => self.acc = clampf(a - b),
                    0x1A => self.acc = clampf(a * b),
                    0x1C => { let v = self.acc + a * b; set(self, fd, v) }
                    0x1D => { let v = self.acc - a * b; set(self, fd, v) }
                    0x1E => self.acc = clampf(self.acc + a * b),
                    0x1F => self.acc = clampf(self.acc - a * b),
                    0x24 => {
                        let v = if a >= 2147483647.0 { 0x7FFF_FFFF } else if a <= -2147483648.0 { 0x8000_0000 } else { a as i32 as u32 };
                        self.f[fd] = v;
                    }
                    0x28 => set(self, fd, a.max(b)),
                    0x29 => set(self, fd, a.min(b)),
                    0x30 => self.fcr31 &= !(1 << 23),
                    0x32 => self.setc(a == b),
                    0x34 => self.setc(a < b),
                    0x36 => self.setc(a <= b),
                    f => self.note(&format!("fpu s {f:02X}")),
                }
            }
            0x14 => match i & 0x3F {
                0x20 => self.f[fd] = (self.f[fs] as i32 as f32).to_bits(),
                f => self.note(&format!("fpu w {f:02X}")),
            },
            _ => self.note(&format!("cop1 rs {rs:02X}")),
        }
    }

    fn setc(&mut self, c: bool) {
        if c {
            self.fcr31 |= 1 << 23;
        } else {
            self.fcr31 &= !(1 << 23);
        }
    }

    fn cop2_op(&mut self, i: u32, rs: usize, rt: usize, rd: usize) {
        match rs {
            0x01 => {
                let f = self.vf[rd];
                let v = f[0] as u128 | (f[1] as u128) << 32 | (f[2] as u128) << 64 | (f[3] as u128) << 96;
                self.sq(rt, v);
            }
            0x02 => {
                let v = match rd {
                    0..=15 => self.vi[rd] & 0xFFFF,
                    29 => 0, // VPU-STAT: both VUs idle
                    _ => self.vi[rd],
                };
                self.s(rt, sx32(v));
            }
            0x05 => {
                if rd != 0 {
                    let v = self.q(rt);
                    self.vf[rd] = [v as u32, (v >> 32) as u32, (v >> 64) as u32, (v >> 96) as u32];
                }
            }
            0x06 => {
                let v = self.g32(rt);
                match rd {
                    0 => {}
                    1..=15 => self.vi[rd] = v & 0xFFFF,
                    _ => self.vi[rd] = v,
                }
            }
            0x08 => {
                // BC2F / BC2T: VU0 never busy.
                let imm = i & 0xFFFF;
                match rt {
                    0 => self.br(true, imm),
                    1 => self.br(false, imm),
                    2 => self.brl(true, imm),
                    3 => self.brl(false, imm),
                    _ => {}
                }
            }
            0x10..=0x1F => crate::vu::macro_op(self, i),
            _ => self.note(&format!("cop2 rs {rs:02X}")),
        }
    }

    fn mmi(&mut self, i: u32, rs: usize, rt: usize, rd: usize, sh: u32) {
        let f = i & 0x3F;
        match f {
            0x00 | 0x01 | 0x20 | 0x21 => {
                // MADD / MADDU (pipeline 0 or 1)
                let p = (f >> 5) as usize;
                let acc = ((self.hi[p] as u32 as u64) << 32 | self.lo[p] as u32 as u64) as i64;
                let prod = if f & 1 == 0 {
                    (self.g32(rs) as i32 as i64) * (self.g32(rt) as i32 as i64)
                } else {
                    ((self.g32(rs) as u64) * (self.g32(rt) as u64)) as i64
                };
                let r = acc.wrapping_add(prod);
                self.lo[p] = sx32(r as u32);
                self.hi[p] = sx32((r >> 32) as u32);
                let v = self.lo[p];
                self.s(rd, v);
            }
            0x04 => {
                // PLZCW: leading sign bits - 1 of each word.
                let v = self.g(rs);
                let cnt = |w: u32| -> u64 {
                    let x = if w & 0x8000_0000 != 0 { !w } else { w };
                    (x.leading_zeros().max(1) - 1) as u64
                };
                self.s(rd, cnt(v as u32) | cnt((v >> 32) as u32) << 32);
            }
            0x08 => self.mmi0(rs, rt, rd, sh),
            0x09 => self.mmi2(rs, rt, rd, sh),
            0x10 => { let v = self.hi[1]; self.s(rd, v) }
            0x11 => self.hi[1] = self.g(rs),
            0x12 => { let v = self.lo[1]; self.s(rd, v) }
            0x13 => self.lo[1] = self.g(rs),
            0x18 => {
                let r = (self.g32(rs) as i32 as i64) * (self.g32(rt) as i32 as i64);
                self.lo[1] = sx32(r as u32);
                self.hi[1] = sx32((r >> 32) as u32);
                let v = self.lo[1];
                self.s(rd, v);
            }
            0x19 => {
                let r = (self.g32(rs) as u64) * (self.g32(rt) as u64);
                self.lo[1] = sx32(r as u32);
                self.hi[1] = sx32((r >> 32) as u32);
                let v = self.lo[1];
                self.s(rd, v);
            }
            0x1A => {
                let (q, r) = div_s(self.g32(rs), self.g32(rt));
                self.lo[1] = sx32(q);
                self.hi[1] = sx32(r);
            }
            0x1B => {
                let (q, r) = div_u(self.g32(rs), self.g32(rt));
                self.lo[1] = sx32(q);
                self.hi[1] = sx32(r);
            }
            0x28 => self.mmi1(rs, rt, rd, sh),
            0x29 => self.mmi3(rs, rt, rd, sh),
            0x30 => {
                // PMFHL
                let lo = self.lo;
                let hi = self.hi;
                let v: u128 = match sh {
                    0 => (lo[0] as u32 as u128) | (hi[0] as u32 as u128) << 32 | (lo[1] as u32 as u128) << 64 | (hi[1] as u32 as u128) << 96,
                    1 => ((lo[0] >> 32) as u128) | ((hi[0] >> 32) as u128) << 32 | ((lo[1] >> 32) as u128) << 64 | ((hi[1] >> 32) as u128) << 96,
                    3 => {
                        let h = |x: u64, k: u32| ((x >> (k * 32)) & 0xFFFF) as u128;
                        h(lo[0], 0) | h(lo[0], 1) << 16 | h(hi[0], 0) << 32 | h(hi[0], 1) << 48
                            | h(lo[1], 0) << 64 | h(lo[1], 1) << 80 | h(hi[1], 0) << 96 | h(hi[1], 1) << 112
                    }
                    _ => {
                        self.note(&format!("pmfhl {sh}"));
                        0
                    }
                };
                self.sq(rd, v);
            }
            0x31 => {
                if sh == 0 {
                    let v = self.q(rs);
                    let w = |k: u32| ((v >> (k * 32)) as u32) as u64;
                    self.lo[0] = (self.lo[0] & !0xFFFF_FFFF) | w(0);
                    self.hi[0] = (self.hi[0] & !0xFFFF_FFFF) | w(1);
                    self.lo[1] = (self.lo[1] & !0xFFFF_FFFF) | w(2);
                    self.hi[1] = (self.hi[1] & !0xFFFF_FFFF) | w(3);
                } else {
                    self.note("pmthl");
                }
            }
            0x34 | 0x36 | 0x37 => {
                let v = self.q(rt);
                let mut o: u128 = 0;
                for k in 0..8 {
                    let h = (v >> (k * 16)) as u16;
                    let r = match f {
                        0x34 => h << (sh & 15),
                        0x36 => h >> (sh & 15),
                        _ => ((h as i16) >> (sh & 15)) as u16,
                    };
                    o |= (r as u128) << (k * 16);
                }
                self.sq(rd, o);
            }
            0x3C | 0x3E | 0x3F => {
                let v = self.q(rt);
                let mut o: u128 = 0;
                for k in 0..4 {
                    let w = (v >> (k * 32)) as u32;
                    let r = match f {
                        0x3C => w << sh,
                        0x3E => w >> sh,
                        _ => ((w as i32) >> sh) as u32,
                    };
                    o |= (r as u128) << (k * 32);
                }
                self.sq(rd, o);
            }
            _ => self.note(&format!("mmi {f:02X}")),
        }
    }

    fn mmi0(&mut self, rs: usize, rt: usize, rd: usize, sub: u32) {
        let a = self.q(rs);
        let b = self.q(rt);
        let r = match sub {
            0x00 => lanes32(a, b, |x, y| x.wrapping_add(y)),
            0x01 => lanes32(a, b, |x, y| x.wrapping_sub(y)),
            0x02 => lanes32(a, b, |x, y| if (x as i32) > (y as i32) { !0 } else { 0 }),
            0x03 => lanes32(a, b, |x, y| (x as i32).max(y as i32) as u32),
            0x04 => lanes16(a, b, |x, y| x.wrapping_add(y)),
            0x05 => lanes16(a, b, |x, y| x.wrapping_sub(y)),
            0x06 => lanes16(a, b, |x, y| if (x as i16) > (y as i16) { !0 } else { 0 }),
            0x07 => lanes16(a, b, |x, y| (x as i16).max(y as i16) as u16),
            0x08 => lanes8(a, b, |x, y| x.wrapping_add(y)),
            0x09 => lanes8(a, b, |x, y| x.wrapping_sub(y)),
            0x0A => lanes8(a, b, |x, y| if (x as i8) > (y as i8) { !0 } else { 0 }),
            0x10 => lanes32(a, b, |x, y| (x as i32).saturating_add(y as i32) as u32),
            0x11 => lanes32(a, b, |x, y| (x as i32).saturating_sub(y as i32) as u32),
            0x12 => {
                // PEXTLW
                let w = |v: u128, k: u32| (v >> (k * 32)) as u32 as u128;
                w(b, 0) | w(a, 0) << 32 | w(b, 1) << 64 | w(a, 1) << 96
            }
            0x13 => {
                // PPACW
                let w = |v: u128, k: u32| (v >> (k * 32)) as u32 as u128;
                w(b, 0) | w(b, 2) << 32 | w(a, 0) << 64 | w(a, 2) << 96
            }
            0x14 => lanes16(a, b, |x, y| (x as i16).saturating_add(y as i16) as u16),
            0x15 => lanes16(a, b, |x, y| (x as i16).saturating_sub(y as i16) as u16),
            0x16 => interleave(a, b, 16, false),
            0x17 => pack(a, b, 16),
            0x18 => lanes8(a, b, |x, y| (x as i8).saturating_add(y as i8) as u8),
            0x19 => lanes8(a, b, |x, y| (x as i8).saturating_sub(y as i8) as u8),
            0x1A => interleave(a, b, 8, false),
            0x1B => pack(a, b, 8),
            0x1E => {
                // PEXT5: 1-5-5-5 → 8-8-8-8
                let mut o = 0u128;
                for k in 0..4 {
                    let x = (b >> (k * 32)) as u32;
                    let w = ((x & 0x1F) << 3) | ((x >> 5 & 0x1F) << 11) | ((x >> 10 & 0x1F) << 19) | ((x >> 15 & 1) << 31);
                    o |= (w as u128) << (k * 32);
                }
                o
            }
            0x1F => {
                let mut o = 0u128;
                for k in 0..4 {
                    let x = (b >> (k * 32)) as u32;
                    let w = (x >> 3 & 0x1F) | (x >> 11 & 0x1F) << 5 | (x >> 19 & 0x1F) << 10 | (x >> 31) << 15;
                    o |= (w as u128) << (k * 32);
                }
                o
            }
            _ => {
                self.note(&format!("mmi0 {sub:02X}"));
                return;
            }
        };
        self.sq(rd, r);
    }

    fn mmi1(&mut self, rs: usize, rt: usize, rd: usize, sub: u32) {
        let a = self.q(rs);
        let b = self.q(rt);
        let r = match sub {
            0x01 => lanes32(b, b, |x, _| if x == 0x8000_0000 { 0x7FFF_FFFF } else { (x as i32).abs() as u32 }),
            0x02 => lanes32(a, b, |x, y| if x == y { !0 } else { 0 }),
            0x03 => lanes32(a, b, |x, y| (x as i32).min(y as i32) as u32),
            0x05 => lanes16(b, b, |x, _| if x == 0x8000 { 0x7FFF } else { (x as i16).wrapping_abs() as u16 }),
            0x06 => lanes16(a, b, |x, y| if x == y { !0 } else { 0 }),
            0x07 => lanes16(a, b, |x, y| (x as i16).min(y as i16) as u16),
            0x0A => lanes8(a, b, |x, y| if x == y { !0 } else { 0 }),
            0x10 => lanes32(a, b, |x, y| x.saturating_add(y)),
            0x11 => lanes32(a, b, |x, y| x.saturating_sub(y)),
            0x12 => {
                let w = |v: u128, k: u32| (v >> (k * 32)) as u32 as u128;
                w(b, 2) | w(a, 2) << 32 | w(b, 3) << 64 | w(a, 3) << 96
            }
            0x14 => lanes16(a, b, |x, y| x.saturating_add(y)),
            0x15 => lanes16(a, b, |x, y| x.saturating_sub(y)),
            0x16 => interleave(a, b, 16, true),
            0x18 => lanes8(a, b, |x, y| x.saturating_add(y)),
            0x19 => lanes8(a, b, |x, y| x.saturating_sub(y)),
            0x1A => interleave(a, b, 8, true),
            0x1B => {
                // QFSRV: funnel shift rs:rt right by SA bits.
                let sa = self.sa & 0x7F;
                if sa == 0 { b } else { (b >> sa) | (a << (128 - sa)) }
            }
            _ => {
                self.note(&format!("mmi1 {sub:02X}"));
                return;
            }
        };
        self.sq(rd, r);
    }

    fn mmi2(&mut self, rs: usize, rt: usize, rd: usize, sub: u32) {
        let a = self.q(rs);
        let b = self.q(rt);
        let w = |v: u128, k: u32| (v >> (k * 32)) as u32;
        let r = match sub {
            0x02 => {
                let o0 = sx32(w(b, 0) << (w(a, 0) & 31));
                let o1 = sx32(w(b, 2) << (w(a, 2) & 31));
                o0 as u128 | (o1 as u128) << 64
            }
            0x03 => {
                let o0 = sx32(w(b, 0) >> (w(a, 0) & 31));
                let o1 = sx32(w(b, 2) >> (w(a, 2) & 31));
                o0 as u128 | (o1 as u128) << 64
            }
            0x08 => self.hi[0] as u128 | (self.hi[1] as u128) << 64,
            0x09 => self.lo[0] as u128 | (self.lo[1] as u128) << 64,
            0x0A => {
                // PINTH
                let h = |v: u128, k: u32| (v >> (k * 16)) as u16 as u128;
                let mut o = 0;
                for k in 0..4 {
                    o |= h(b, k) << (k * 32) | h(a, k + 4) << (k * 32 + 16);
                }
                o
            }
            0x0C => {
                let p0 = (w(a, 0) as i32 as i64) * (w(b, 0) as i32 as i64);
                let p1 = (w(a, 2) as i32 as i64) * (w(b, 2) as i32 as i64);
                self.lo = [sx32(p0 as u32), sx32(p1 as u32)];
                self.hi = [sx32((p0 >> 32) as u32), sx32((p1 >> 32) as u32)];
                p0 as u64 as u128 | (p1 as u64 as u128) << 64
            }
            0x0D => {
                let (q0, r0) = div_s(w(a, 0), w(b, 0));
                let (q1, r1) = div_s(w(a, 2), w(b, 2));
                self.lo = [sx32(q0), sx32(q1)];
                self.hi = [sx32(r0), sx32(r1)];
                return;
            }
            0x0E => (b as u64 as u128) | (a as u64 as u128) << 64,
            0x12 => a & b,
            0x13 => a ^ b,
            0x1A => {
                // PEXEH: swap halfwords 0 and 2 in each doubleword.
                let mut o = b;
                for d in 0..2 {
                    let h0 = (b >> (d * 64)) as u16 as u128;
                    let h2 = (b >> (d * 64 + 32)) as u16 as u128;
                    o &= !((0xFFFFu128 << (d * 64)) | (0xFFFFu128 << (d * 64 + 32)));
                    o |= h2 << (d * 64) | h0 << (d * 64 + 32);
                }
                o
            }
            0x1B => {
                // PREVH
                let mut o = 0u128;
                for d in 0..2 {
                    for k in 0..4 {
                        let h = (b >> (d * 64 + k * 16)) as u16 as u128;
                        o |= h << (d * 64 + (3 - k) * 16);
                    }
                }
                o
            }
            0x1E => {
                // PEXEW
                let x = |k| w(b, k) as u128;
                x(2) | x(1) << 32 | x(0) << 64 | x(3) << 96
            }
            0x1F => {
                // PROT3W
                let x = |k| w(b, k) as u128;
                x(1) | x(2) << 32 | x(0) << 64 | x(3) << 96
            }
            _ => {
                self.note(&format!("mmi2 {sub:02X}"));
                return;
            }
        };
        self.sq(rd, r);
    }

    fn mmi3(&mut self, rs: usize, rt: usize, rd: usize, sub: u32) {
        let a = self.q(rs);
        let b = self.q(rt);
        let w = |v: u128, k: u32| (v >> (k * 32)) as u32;
        let r = match sub {
            0x03 => {
                let o0 = sx32(((w(b, 0) as i32) >> (w(a, 0) & 31)) as u32);
                let o1 = sx32(((w(b, 2) as i32) >> (w(a, 2) & 31)) as u32);
                o0 as u128 | (o1 as u128) << 64
            }
            0x08 => {
                self.hi = [a as u64, (a >> 64) as u64];
                return;
            }
            0x09 => {
                self.lo = [a as u64, (a >> 64) as u64];
                return;
            }
            0x0A => {
                // PINTEH
                let h = |v: u128, k: u32| (v >> (k * 32)) as u16 as u128;
                let mut o = 0;
                for k in 0..4 {
                    o |= h(b, k) << (k * 32) | h(a, k) << (k * 32 + 16);
                }
                o
            }
            0x0C => {
                let p0 = (w(a, 0) as u64) * (w(b, 0) as u64);
                let p1 = (w(a, 2) as u64) * (w(b, 2) as u64);
                self.lo = [sx32(p0 as u32), sx32(p1 as u32)];
                self.hi = [sx32((p0 >> 32) as u32), sx32((p1 >> 32) as u32)];
                p0 as u128 | (p1 as u128) << 64
            }
            0x0D => {
                let (q0, r0) = div_u(w(a, 0), w(b, 0));
                let (q1, r1) = div_u(w(a, 2), w(b, 2));
                self.lo = [sx32(q0), sx32(q1)];
                self.hi = [sx32(r0), sx32(r1)];
                return;
            }
            0x0E => (a >> 64) | ((b >> 64) << 64),
            0x12 => a | b,
            0x13 => !(a | b),
            0x1A => {
                // PEXCH: swap halfwords 1 and 2 in each doubleword.
                let mut o = 0u128;
                for d in 0..2 {
                    let h = |k: u32| (b >> (d * 64 + k * 16)) as u16 as u128;
                    o |= (h(0) | h(2) << 16 | h(1) << 32 | h(3) << 48) << (d * 64);
                }
                o
            }
            0x1B => {
                // PCPYH
                let h0 = b as u16 as u128;
                let h4 = (b >> 64) as u16 as u128;
                let mut o = 0;
                for k in 0..4 {
                    o |= h0 << (k * 16) | h4 << (64 + k * 16);
                }
                o
            }
            0x1E => {
                // PEXCW
                let x = |k| w(b, k) as u128;
                x(0) | x(2) << 32 | x(1) << 64 | x(3) << 96
            }
            _ => {
                self.note(&format!("mmi3 {sub:02X}"));
                return;
            }
        };
        self.sq(rd, r);
    }
}

fn div_s(n: u32, d: u32) -> (u32, u32) {
    let (n, d) = (n as i32, d as i32);
    if d == 0 {
        (if n >= 0 { 0xFFFF_FFFF } else { 1 }, n as u32)
    } else if n == i32::MIN && d == -1 {
        (0x8000_0000, 0)
    } else {
        ((n / d) as u32, (n % d) as u32)
    }
}

fn div_u(n: u32, d: u32) -> (u32, u32) {
    if d == 0 { (0xFFFF_FFFF, n) } else { (n / d, n % d) }
}

fn lanes32(a: u128, b: u128, f: impl Fn(u32, u32) -> u32) -> u128 {
    let mut o = 0u128;
    for k in 0..4 {
        o |= (f((a >> (k * 32)) as u32, (b >> (k * 32)) as u32) as u128) << (k * 32);
    }
    o
}

fn lanes16(a: u128, b: u128, f: impl Fn(u16, u16) -> u16) -> u128 {
    let mut o = 0u128;
    for k in 0..8 {
        o |= (f((a >> (k * 16)) as u16, (b >> (k * 16)) as u16) as u128) << (k * 16);
    }
    o
}

fn lanes8(a: u128, b: u128, f: impl Fn(u8, u8) -> u8) -> u128 {
    let mut o = 0u128;
    for k in 0..16 {
        o |= (f((a >> (k * 8)) as u8, (b >> (k * 8)) as u8) as u128) << (k * 8);
    }
    o
}

/// PEXTL/PEXTU H and B: interleave the low (or high) halves, rt first.
fn interleave(a: u128, b: u128, bits: u32, upper: bool) -> u128 {
    let n = 64 / bits;
    let m = (1u128 << bits) - 1;
    let base = if upper { 64 } else { 0 };
    let mut o = 0u128;
    for k in 0..n {
        let x = (b >> (base + k * bits)) & m;
        let y = (a >> (base + k * bits)) & m;
        o |= x << (2 * k * bits) | y << ((2 * k + 1) * bits);
    }
    o
}

/// PPACH / PPACB: even elements of rt in the low half, of rs in the high.
fn pack(a: u128, b: u128, bits: u32) -> u128 {
    let n = 64 / bits;
    let m = (1u128 << bits) - 1;
    let mut o = 0u128;
    for k in 0..n {
        o |= ((b >> (2 * k * bits)) & m) << (k * bits);
        o |= ((a >> (2 * k * bits)) & m) << (64 + k * bits);
    }
    o
}
