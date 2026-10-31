//! MIPS R3000A interpreter with COP0 and the GTE (COP2).
//!
//! Emulates branch delay slots, load delay slots and the exception model the
//! BIOS relies on. Instruction timing is approximate (fixed cost per
//! instruction plus extra for memory accesses).

use crate::bus::Bus;
use crate::gte::Gte;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exception {
    Interrupt = 0x0,
    LoadAddressError = 0x4,
    StoreAddressError = 0x5,
    BusErrorInstr = 0x6,
    Syscall = 0x8,
    Break = 0x9,
    ReservedInstruction = 0xA,
    CoprocessorUnusable = 0xB,
    Overflow = 0xC,
}

pub struct Cpu {
    pub regs: [u32; 32],
    pub pc: u32,
    pub next_pc: u32,
    current_pc: u32,
    pub hi: u32,
    pub lo: u32,
    /// Load waiting in the delay slot: (register, value).
    load: (usize, u32),
    branch: bool,
    delay_slot: bool,
    // COP0
    pub sr: u32,
    pub cause: u32,
    pub epc: u32,
    pub bad_vaddr: u32,
    cop0_misc: [u32; 16],
    pub gte: Gte,
    /// Register (+1) written directly by the current instruction, so a load
    /// still in flight to the same register is dropped (the instruction wins).
    wrote: u32,
    /// Set when the BIOS hands control to a game (A0:43h DoExecute). We
    /// stop there: only the boot sequence is emulated.
    pub game_started: bool,
    /// Optional hook for the test harness: called on syscall-free "debug
    /// putchar" style BIOS calls. Unused by the web build.
    pub tty: Vec<u8>,
    /// Notable events for the debug report, drained once per frame.
    pub trace: Vec<String>,
    /// How often each BIOS function (A0/B0/C0 table, number) was called.
    pub bios_calls: std::collections::BTreeMap<(u32, u32), u32>,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            regs: [0; 32],
            pc: 0xBFC0_0000,
            next_pc: 0xBFC0_0004,
            current_pc: 0xBFC0_0000,
            hi: 0,
            lo: 0,
            load: (0, 0),
            branch: false,
            delay_slot: false,
            sr: 0,
            cause: 0,
            epc: 0,
            bad_vaddr: 0,
            cop0_misc: [0; 16],
            gte: Gte::new(),
            wrote: 0,
            game_started: false,
            tty: Vec::new(),
            trace: Vec::new(),
            bios_calls: std::collections::BTreeMap::new(),
        }
    }

    #[inline(always)]
    fn reg(&self, i: usize) -> u32 {
        self.regs[i]
    }

    #[inline(always)]
    fn set_reg(&mut self, i: usize, v: u32) {
        self.regs[i] = v;
        self.regs[0] = 0;
    }

    #[inline(always)]
    fn delayed_load(&mut self, i: usize, v: u32) {
        self.load = (i, v);
    }

    /// Executes one instruction. Returns the number of cycles it took.
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        self.current_pc = self.pc;

        // BIOS putchar hooks (A0:3C, B0:3D) are mirrored to the TTY buffer for
        // debugging. They don't change behaviour. A0:43 is DoExecute, the
        // hand-off to the game.
        if matches!(self.pc, 0xA0 | 0xB0 | 0xC0) {
            let f = self.regs[9];
            *self.bios_calls.entry((self.pc, f & 0xFF)).or_insert(0) += 1;
            if (self.pc == 0xA0 && f == 0x3C || self.pc == 0xB0 && f == 0x3D) && self.tty.len() < 1 << 16 {
                self.tty.push(self.regs[4] as u8);
            }
            if self.pc == 0xA0 && f == 0x43 {
                self.game_started = true;
            }
        }
        if self.pc == 0xBFC0_0000 && !self.bios_calls.is_empty() {
            self.trace.push(format!("jumped back to the reset vector from {:08X}", self.next_pc.wrapping_sub(8)));
        }

        if self.current_pc & 3 != 0 {
            self.bad_vaddr = self.current_pc;
            self.exception(Exception::LoadAddressError);
            return 2;
        }

        let instr = bus.fetch(self.current_pc);

        self.delay_slot = self.branch;
        self.branch = false;

        // Interrupts are taken before the instruction executes. A GTE command
        // at that address still runs first; the BIOS handler skips it.
        bus.update_irq_line();
        if bus.irq_pending() {
            self.cause |= 1 << 10;
        } else {
            self.cause &= !(1 << 10);
        }
        if self.sr & 1 != 0 && (self.sr & self.cause & 0xFF00) != 0 {
            if instr >> 25 == 0x25 {
                self.gte.command(instr);
            }
            self.exception(Exception::Interrupt);
            return 2;
        }

        self.pc = self.next_pc;
        self.next_pc = self.next_pc.wrapping_add(4);

        let pending = self.load;
        self.load = (0, 0);
        let before = bus.access_cycles;
        self.execute(instr, bus, pending);
        // Apply the previous instruction's load unless this one overwrote it.
        if pending.0 != 0 && self.wrote != pending.0 as u32 + 1 {
            self.regs[pending.0] = pending.1;
        }
        self.wrote = 0;
        let extra = bus.access_cycles - before;
        2 + extra
    }

    fn exception(&mut self, cause: Exception) {
        if !matches!(cause, Exception::Interrupt | Exception::Syscall) && self.trace.len() < 64 {
            self.trace.push(format!(
                "exception {:?} at {:08X} (bad addr {:08X}, sr {:08X})",
                cause, self.current_pc, self.bad_vaddr, self.sr
            ));
        }
        let handler = if self.sr & (1 << 22) != 0 { 0xBFC0_0180 } else { 0x8000_0080 };
        let mode = self.sr & 0x3F;
        self.sr = (self.sr & !0x3F) | ((mode << 2) & 0x3F);
        self.cause = (self.cause & !0x7C) | ((cause as u32) << 2);
        if self.delay_slot {
            self.epc = self.current_pc.wrapping_sub(4);
            self.cause |= 1 << 31;
        } else {
            self.epc = self.current_pc;
            self.cause &= !(1 << 31);
        }
        // Flush the load delay slot on exceptions.
        if self.load.0 != 0 {
            self.regs[self.load.0] = self.load.1;
            self.load = (0, 0);
        }
        self.pc = handler;
        self.next_pc = handler.wrapping_add(4);
        self.branch = false;
        self.delay_slot = false;
    }

    fn branch_to(&mut self, target: u32) {
        self.next_pc = target;
        self.branch = true;
    }

    fn branch_rel(&mut self, offset: u32) {
        let off = offset << 2;
        self.next_pc = self.pc.wrapping_add(off);
        self.branch = true;
    }

    #[inline(always)]
    fn w(&mut self, r: usize, v: u32) {
        self.set_reg(r, v);
        self.wrote = r as u32 + 1;
    }

    fn execute(&mut self, instr: u32, bus: &mut Bus, pending: (usize, u32)) {
        let op = instr >> 26;
        let rs = ((instr >> 21) & 31) as usize;
        let rt = ((instr >> 16) & 31) as usize;
        let rd = ((instr >> 11) & 31) as usize;
        let shamt = (instr >> 6) & 31;
        let imm = instr & 0xFFFF;
        let imm_se = (instr as i16) as i32 as u32;
        let funct = instr & 0x3F;

        match op {
            0x00 => match funct {
                0x00 => { let v = self.reg(rt) << shamt; self.w(rd, v) }
                0x02 => { let v = self.reg(rt) >> shamt; self.w(rd, v) }
                0x03 => { let v = ((self.reg(rt) as i32) >> shamt) as u32; self.w(rd, v) }
                0x04 => { let v = self.reg(rt) << (self.reg(rs) & 31); self.w(rd, v) }
                0x06 => { let v = self.reg(rt) >> (self.reg(rs) & 31); self.w(rd, v) }
                0x07 => { let v = ((self.reg(rt) as i32) >> (self.reg(rs) & 31)) as u32; self.w(rd, v) }
                0x08 => { let t = self.reg(rs); self.branch_to(t) }
                0x09 => {
                    let t = self.reg(rs);
                    let ra = self.next_pc;
                    self.w(rd, ra);
                    self.branch_to(t);
                }
                0x0C => self.exception(Exception::Syscall),
                0x0D => self.exception(Exception::Break),
                0x10 => { let v = self.hi; self.w(rd, v) }
                0x11 => self.hi = self.reg(rs),
                0x12 => { let v = self.lo; self.w(rd, v) }
                0x13 => self.lo = self.reg(rs),
                0x18 => {
                    let r = (self.reg(rs) as i32 as i64) * (self.reg(rt) as i32 as i64);
                    self.hi = (r >> 32) as u32;
                    self.lo = r as u32;
                }
                0x19 => {
                    let r = (self.reg(rs) as u64) * (self.reg(rt) as u64);
                    self.hi = (r >> 32) as u32;
                    self.lo = r as u32;
                }
                0x1A => {
                    let n = self.reg(rs) as i32;
                    let d = self.reg(rt) as i32;
                    if d == 0 {
                        self.hi = n as u32;
                        self.lo = if n >= 0 { 0xFFFF_FFFF } else { 1 };
                    } else if n as u32 == 0x8000_0000 && d == -1 {
                        self.hi = 0;
                        self.lo = 0x8000_0000;
                    } else {
                        self.hi = (n % d) as u32;
                        self.lo = (n / d) as u32;
                    }
                }
                0x1B => {
                    let n = self.reg(rs);
                    let d = self.reg(rt);
                    if d == 0 {
                        self.hi = n;
                        self.lo = 0xFFFF_FFFF;
                    } else {
                        self.hi = n % d;
                        self.lo = n / d;
                    }
                }
                0x20 => match (self.reg(rs) as i32).checked_add(self.reg(rt) as i32) {
                    Some(v) => self.w(rd, v as u32),
                    None => self.exception(Exception::Overflow),
                },
                0x21 => { let v = self.reg(rs).wrapping_add(self.reg(rt)); self.w(rd, v) }
                0x22 => match (self.reg(rs) as i32).checked_sub(self.reg(rt) as i32) {
                    Some(v) => self.w(rd, v as u32),
                    None => self.exception(Exception::Overflow),
                },
                0x23 => { let v = self.reg(rs).wrapping_sub(self.reg(rt)); self.w(rd, v) }
                0x24 => { let v = self.reg(rs) & self.reg(rt); self.w(rd, v) }
                0x25 => { let v = self.reg(rs) | self.reg(rt); self.w(rd, v) }
                0x26 => { let v = self.reg(rs) ^ self.reg(rt); self.w(rd, v) }
                0x27 => { let v = !(self.reg(rs) | self.reg(rt)); self.w(rd, v) }
                0x2A => { let v = ((self.reg(rs) as i32) < (self.reg(rt) as i32)) as u32; self.w(rd, v) }
                0x2B => { let v = (self.reg(rs) < self.reg(rt)) as u32; self.w(rd, v) }
                _ => self.exception(Exception::ReservedInstruction),
            },
            0x01 => {
                // BLTZ, BGEZ, BLTZAL, BGEZAL (any rt with bit 4 set links).
                let s = self.reg(rs) as i32;
                let is_bgez = rt & 1;
                let test = ((s < 0) as usize) ^ is_bgez;
                if rt & 0x1E == 0x10 {
                    let ra = self.next_pc;
                    self.w(31, ra);
                }
                if test != 0 {
                    self.branch_rel(imm_se);
                } else {
                    self.branch = true;
                }
            }
            0x02 => {
                let t = (self.pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2);
                self.branch_to(t);
            }
            0x03 => {
                let t = (self.pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2);
                let ra = self.next_pc;
                self.w(31, ra);
                self.branch_to(t);
            }
            0x04 => { if self.reg(rs) == self.reg(rt) { self.branch_rel(imm_se) } else { self.branch = true } }
            0x05 => { if self.reg(rs) != self.reg(rt) { self.branch_rel(imm_se) } else { self.branch = true } }
            0x06 => { if (self.reg(rs) as i32) <= 0 { self.branch_rel(imm_se) } else { self.branch = true } }
            0x07 => { if (self.reg(rs) as i32) > 0 { self.branch_rel(imm_se) } else { self.branch = true } }
            0x08 => match (self.reg(rs) as i32).checked_add(imm_se as i32) {
                Some(v) => self.w(rt, v as u32),
                None => self.exception(Exception::Overflow),
            },
            0x09 => { let v = self.reg(rs).wrapping_add(imm_se); self.w(rt, v) }
            0x0A => { let v = ((self.reg(rs) as i32) < (imm_se as i32)) as u32; self.w(rt, v) }
            0x0B => { let v = (self.reg(rs) < imm_se) as u32; self.w(rt, v) }
            0x0C => { let v = self.reg(rs) & imm; self.w(rt, v) }
            0x0D => { let v = self.reg(rs) | imm; self.w(rt, v) }
            0x0E => { let v = self.reg(rs) ^ imm; self.w(rt, v) }
            0x0F => self.w(rt, imm << 16),
            0x10 => self.cop0(instr, rt, rd),
            0x11 | 0x13 => self.exception(Exception::CoprocessorUnusable),
            0x12 => self.cop2(instr, rs, rt, rd),
            0x20 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                let v = bus.read8(a) as i8 as i32 as u32;
                self.delayed_load(rt, v);
            }
            0x21 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 1 != 0 { self.bad_vaddr = a; return self.exception(Exception::LoadAddressError); }
                let v = bus.read16(a) as i16 as i32 as u32;
                self.delayed_load(rt, v);
            }
            0x22 | 0x26 => {
                // LWL / LWR merge with the value in flight for rt.
                let a = self.reg(rs).wrapping_add(imm_se);
                let cur = if pending.0 == rt && rt != 0 { pending.1 } else { self.reg(rt) };
                let aligned = bus.read32(a & !3);
                let v = if op == 0x22 {
                    match a & 3 {
                        0 => (cur & 0x00FF_FFFF) | (aligned << 24),
                        1 => (cur & 0x0000_FFFF) | (aligned << 16),
                        2 => (cur & 0x0000_00FF) | (aligned << 8),
                        _ => aligned,
                    }
                } else {
                    match a & 3 {
                        0 => aligned,
                        1 => (cur & 0xFF00_0000) | (aligned >> 8),
                        2 => (cur & 0xFFFF_0000) | (aligned >> 16),
                        _ => (cur & 0xFFFF_FF00) | (aligned >> 24),
                    }
                };
                self.delayed_load(rt, v);
            }
            0x23 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exception::LoadAddressError); }
                let v = bus.read32(a);
                self.delayed_load(rt, v);
            }
            0x24 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                let v = bus.read8(a) as u32;
                self.delayed_load(rt, v);
            }
            0x25 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 1 != 0 { self.bad_vaddr = a; return self.exception(Exception::LoadAddressError); }
                let v = bus.read16(a) as u32;
                self.delayed_load(rt, v);
            }
            0x28 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if self.sr & 0x10000 != 0 { return; } // cache isolated
                bus.write8(a, self.reg(rt) as u8);
            }
            0x29 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 1 != 0 { self.bad_vaddr = a; return self.exception(Exception::StoreAddressError); }
                if self.sr & 0x10000 != 0 { return; }
                bus.write16(a, self.reg(rt) as u16);
            }
            0x2A | 0x2E => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if self.sr & 0x10000 != 0 { return; }
                let v = self.reg(rt);
                let aa = a & !3;
                let mem = bus.read32(aa);
                let out = if op == 0x2A {
                    match a & 3 {
                        0 => (mem & 0xFFFF_FF00) | (v >> 24),
                        1 => (mem & 0xFFFF_0000) | (v >> 16),
                        2 => (mem & 0xFF00_0000) | (v >> 8),
                        _ => v,
                    }
                } else {
                    match a & 3 {
                        0 => v,
                        1 => (mem & 0x0000_00FF) | (v << 8),
                        2 => (mem & 0x0000_FFFF) | (v << 16),
                        _ => (mem & 0x00FF_FFFF) | (v << 24),
                    }
                };
                bus.write32(aa, out);
            }
            0x2B => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exception::StoreAddressError); }
                if self.sr & 0x10000 != 0 { return; }
                bus.write32(a, self.reg(rt));
            }
            0x30 | 0x31 | 0x33 | 0x38 | 0x39 | 0x3B => self.exception(Exception::CoprocessorUnusable),
            0x32 => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exception::LoadAddressError); }
                let v = bus.read32(a);
                self.gte.write_data(rt, v);
            }
            0x3A => {
                let a = self.reg(rs).wrapping_add(imm_se);
                if a & 3 != 0 { self.bad_vaddr = a; return self.exception(Exception::StoreAddressError); }
                let v = self.gte.read_data(rt);
                bus.write32(a, v);
            }
            _ => self.exception(Exception::ReservedInstruction),
        }
    }

    fn cop0(&mut self, instr: u32, rt: usize, rd: usize) {
        match (instr >> 21) & 31 {
            0x00 => {
                let v = match rd {
                    12 => self.sr,
                    13 => self.cause,
                    14 => self.epc,
                    8 => self.bad_vaddr,
                    15 => 0x0000_0002,
                    _ => self.cop0_misc[rd & 15],
                };
                self.delayed_load(rt, v);
            }
            0x04 => {
                let v = self.reg(rt);
                match rd {
                    12 => self.sr = v,
                    13 => self.cause = (self.cause & !0x300) | (v & 0x300),
                    _ => self.cop0_misc[rd & 15] = v,
                }
            }
            0x10 => {
                if instr & 0x3F == 0x10 {
                    // RFE
                    let mode = self.sr & 0x3F;
                    self.sr = (self.sr & !0xF) | (mode >> 2);
                } else {
                    self.exception(Exception::ReservedInstruction);
                }
            }
            _ => self.exception(Exception::ReservedInstruction),
        }
    }

    fn cop2(&mut self, instr: u32, rs: usize, rt: usize, rd: usize) {
        if self.sr & (1 << 30) == 0 {
            return self.exception(Exception::CoprocessorUnusable);
        }
        if rs & 0x10 != 0 {
            self.gte.command(instr);
            return;
        }
        match rs {
            0x00 => { let v = self.gte.read_data(rd); self.delayed_load(rt, v) }
            0x02 => { let v = self.gte.read_ctrl(rd); self.delayed_load(rt, v) }
            0x04 => { let v = self.reg(rt); self.gte.write_data(rd, v) }
            0x06 => { let v = self.reg(rt); self.gte.write_ctrl(rd, v) }
            _ => self.exception(Exception::ReservedInstruction),
        }
    }
}
