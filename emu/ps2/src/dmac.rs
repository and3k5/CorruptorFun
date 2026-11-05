//! EE DMA controller (10 channels) and the SIF data path to the IOP.
//!
//! Transfers run to completion as soon as they are started, except the SIF
//! channels, which move whatever the other side has ready and finish once
//! their chain ends.

use std::collections::VecDeque;

use crate::hw::Hw;

pub const VIF0: usize = 0;
pub const VIF1: usize = 1;
pub const GIF: usize = 2;
pub const SIF0: usize = 5;
pub const SIF1: usize = 6;
pub const SPR_FROM: usize = 8;
pub const SPR_TO: usize = 9;

const BASES: [u32; 10] =
    [0x1000_8000, 0x1000_9000, 0x1000_A000, 0x1000_B000, 0x1000_B400, 0x1000_C000, 0x1000_C400, 0x1000_C800, 0x1000_D000, 0x1000_D400];

#[derive(Default, Clone, Copy)]
pub struct Channel {
    pub chcr: u32,
    pub madr: u32,
    pub qwc: u32,
    pub tadr: u32,
    pub asr: [u32; 2],
    pub sadr: u32,
    /// The chain has reached its last tag.
    pub ended: bool,
}

pub struct Dmac {
    pub ch: [Channel; 10],
    pub ctrl: u32,
    pub stat: u32,
    pub pcr: u32,
    pub sqwc: u32,
    pub rbsr: u32,
    pub rbor: u32,
    pub stadr: u32,
    pub enable: u32,
    pub vif0_fifo: Vec<u128>,
    pub vif1_fifo: Vec<u128>,
    /// Words travelling IOP → EE.
    pub sif0_fifo: VecDeque<u32>,
    /// Words travelling EE → IOP.
    pub sif1_fifo: VecDeque<u32>,
    /// Counts of completed transfers per channel, for the debug report.
    pub transfers: [u32; 10],
    /// Bytes per channel.
    pub bytes: [u64; 10],
}

impl Dmac {
    pub fn new() -> Self {
        Dmac {
            ch: [Channel::default(); 10],
            ctrl: 0,
            stat: 0,
            pcr: 0,
            sqwc: 0,
            rbsr: 0,
            rbor: 0,
            stadr: 0,
            enable: 0x1201,
            vif0_fifo: Vec::new(),
            vif1_fifo: Vec::new(),
            sif0_fifo: VecDeque::new(),
            sif1_fifo: VecDeque::new(),
            transfers: [0; 10],
            bytes: [0; 10],
        }
    }

    pub fn int1(&self) -> bool {
        (self.stat & 0x3FF) & ((self.stat >> 16) & 0x3FF) != 0 || (self.stat & 0x2000 != 0 && self.stat & 0x2000_0000 != 0)
    }

    fn chan(addr: u32) -> Option<usize> {
        BASES.iter().position(|&b| addr & !0xFF == b)
    }

    pub fn read(&self, a: u32) -> u32 {
        if let Some(i) = Self::chan(a) {
            let c = &self.ch[i];
            return match a & 0xFF {
                0x00 => c.chcr,
                0x10 => c.madr,
                0x20 => c.qwc,
                0x30 => c.tadr,
                0x40 => c.asr[0],
                0x50 => c.asr[1],
                0x80 => c.sadr,
                _ => 0,
            };
        }
        match a {
            0x1000_E000 => self.ctrl,
            0x1000_E010 => self.stat,
            0x1000_E020 => self.pcr,
            0x1000_E030 => self.sqwc,
            0x1000_E040 => self.rbsr,
            0x1000_E050 => self.rbor,
            0x1000_E060 => self.stadr,
            _ => 0,
        }
    }

    /// `v` is already merged with the old value for partial writes, except
    /// for D_STAT, whose bits are write-1-to-clear / toggle.
    pub fn write(&mut self, a: u32, v: u32) {
        if let Some(i) = Self::chan(a) {
            let c = &mut self.ch[i];
            match a & 0xFF {
                0x00 => {
                    let starting = v & 0x100 != 0 && c.chcr & 0x100 == 0;
                    c.chcr = v;
                    if starting {
                        c.ended = false;
                    }
                }
                0x10 => c.madr = v & !0xF,
                0x20 => c.qwc = v & 0xFFFF,
                0x30 => c.tadr = v & !0xF,
                0x40 => c.asr[0] = v & !0xF,
                0x50 => c.asr[1] = v & !0xF,
                0x80 => c.sadr = v & 0x3FF0,
                _ => {}
            }
            return;
        }
        match a {
            0x1000_E000 => self.ctrl = v,
            0x1000_E010 => {
                // Hw passes the raw written value for D_STAT.
                let clear = v & 0xFFFF;
                let toggle = v & 0xFFFF_0000;
                self.stat = (self.stat & !clear) ^ toggle;
            }
            0x1000_E020 => self.pcr = v,
            0x1000_E030 => self.sqwc = v,
            0x1000_E040 => self.rbsr = v,
            0x1000_E050 => self.rbor = v,
            0x1000_E060 => self.stadr = v,
            _ => {}
        }
    }

    fn finish(&mut self, i: usize) {
        self.ch[i].chcr &= !0x100;
        self.stat |= 1 << i;
        self.transfers[i] += 1;
    }
}

fn running(hw: &Hw, i: usize) -> bool {
    hw.dmac.ch[i].chcr & 0x100 != 0 && hw.dmac.ctrl & 1 != 0 && hw.dmac.enable & 0x10000 == 0
}

fn ram_qword(hw: &mut Hw, addr: u32) -> u128 {
    if addr & 0x8000_0000 != 0 {
        let a = (addr & 0x3FF0) as usize;
        u128::from_le_bytes(hw.spr[a..a + 16].try_into().unwrap())
    } else {
        let a = (addr & 0x01FF_FFF0) as usize;
        u128::from_le_bytes(hw.ram[a..a + 16].try_into().unwrap())
    }
}

fn ram_write_qword(hw: &mut Hw, addr: u32, v: u128) {
    if addr & 0x8000_0000 != 0 {
        let a = (addr & 0x3FF0) as usize;
        hw.spr[a..a + 16].copy_from_slice(&v.to_le_bytes());
    } else {
        let a = (addr & 0x01FF_FFF0) as usize;
        hw.ram[a..a + 16].copy_from_slice(&v.to_le_bytes());
    }
}

fn sink(hw: &mut Hw, i: usize, q: u128) {
    hw.dmac.bytes[i] += 16;
    match i {
        GIF => hw.gs.gif_path_write(3, q),
        VIF1 => hw.dmac.vif1_fifo.push(q),
        VIF0 => hw.dmac.vif0_fifo.push(q),
        SIF1 => {
            for k in 0..4 {
                hw.dmac.sif1_fifo.push_back((q >> (k * 32)) as u32);
            }
        }
        _ => {}
    }
}

/// Runs a source-chain or normal transfer from memory into channel `i`.
fn source(hw: &mut Hw, i: usize) {
    let mut guard = 0;
    loop {
        // Data still to send for the current tag.
        let qwc = hw.dmac.ch[i].qwc;
        let madr = hw.dmac.ch[i].madr;
        for k in 0..qwc {
            let q = ram_qword(hw, madr.wrapping_add(k * 16));
            sink(hw, i, q);
        }
        let c = &mut hw.dmac.ch[i];
        c.madr = c.madr.wrapping_add(qwc * 16);
        c.qwc = 0;
        let mode = (c.chcr >> 2) & 3;
        if mode != 1 || c.ended {
            hw.dmac.finish(i);
            return;
        }
        guard += 1;
        if guard > 100_000 {
            hw.trace.push(format!("DMA ch{i}: chain did not end, stopped"));
            hw.dmac.finish(i);
            return;
        }
        // Read the next tag.
        let tadr = hw.dmac.ch[i].tadr;
        let tag = ram_qword(hw, tadr);
        let t = tag as u64;
        let qwc = (t & 0xFFFF) as u32;
        let id = ((t >> 28) & 7) as u32;
        let irq = t >> 31 & 1 != 0;
        let addr = ((t >> 32) as u32) & 0xFFFF_FFF0 | ((t >> 32) as u32 & 0x8000_0000);
        let c = &mut hw.dmac.ch[i];
        c.chcr = (c.chcr & 0xFFFF) | ((t as u32) & 0xFFFF_0000);
        let tte = c.chcr & 0x40 != 0;
        let tie = c.chcr & 0x80 != 0;
        c.qwc = qwc;
        match id {
            0 => {
                c.madr = addr;
                c.tadr = tadr.wrapping_add(16);
                c.ended = true;
            }
            1 => {
                c.madr = tadr.wrapping_add(16);
                c.tadr = c.madr.wrapping_add(qwc * 16);
            }
            2 => {
                c.madr = tadr.wrapping_add(16);
                c.tadr = addr;
            }
            3 | 4 => {
                c.madr = addr;
                c.tadr = tadr.wrapping_add(16);
            }
            5 => {
                c.madr = tadr.wrapping_add(16);
                let asp = (c.chcr >> 4) & 3;
                let ret = c.madr.wrapping_add(qwc * 16);
                if asp < 2 {
                    c.asr[asp as usize] = ret;
                    c.chcr = (c.chcr & !0x30) | ((asp + 1) << 4);
                }
                c.tadr = addr;
            }
            6 => {
                c.madr = tadr.wrapping_add(16);
                let asp = (c.chcr >> 4) & 3;
                if asp > 0 {
                    c.tadr = c.asr[asp as usize - 1];
                    c.chcr = (c.chcr & !0x30) | ((asp - 1) << 4);
                } else {
                    c.ended = true;
                }
            }
            _ => {
                c.madr = tadr.wrapping_add(16);
                c.ended = true;
            }
        }
        if irq && tie {
            c.ended = true;
        }
        if tte && (i == VIF0 || i == VIF1) {
            // The upper half of the tag goes to the VIF as two VIF codes.
            sink(hw, i, tag & !0xFFFF_FFFF_FFFF_FFFFu128);
        }
    }
}

pub fn run(hw: &mut Hw) {
    for i in [VIF0, VIF1, GIF, SPR_FROM, SPR_TO, 3, 4, 7] {
        if !running(hw, i) {
            continue;
        }
        match i {
            SPR_FROM | SPR_TO => spr(hw, i),
            3 | 4 | 7 => {
                hw.dmac.ch[i].qwc = 0;
                hw.dmac.finish(i);
            }
            _ => {
                if hw.dmac.ch[i].chcr & 1 == 0 && i != GIF {
                    // VIF reading back to memory: nothing to give.
                    hw.dmac.ch[i].qwc = 0;
                    hw.dmac.finish(i);
                } else {
                    source(hw, i);
                }
            }
        }
        if i == VIF1 || i == VIF0 {
            crate::vif::process(hw, i);
        }
    }
    sif(hw);
}

fn spr(hw: &mut Hw, i: usize) {
    // Normal or chain transfers between RAM and the scratchpad.
    let mut guard = 0;
    loop {
        let c = hw.dmac.ch[i];
        for k in 0..c.qwc {
            let s = (c.sadr + k * 16) & 0x3FF0;
            let m = c.madr.wrapping_add(k * 16);
            if i == SPR_FROM {
                let q = u128::from_le_bytes(hw.spr[s as usize..s as usize + 16].try_into().unwrap());
                ram_write_qword(hw, m, q);
            } else {
                let q = ram_qword(hw, m);
                hw.spr[s as usize..s as usize + 16].copy_from_slice(&q.to_le_bytes());
            }
        }
        let c = &mut hw.dmac.ch[i];
        c.sadr = (c.sadr + c.qwc * 16) & 0x3FF0;
        c.madr = c.madr.wrapping_add(c.qwc * 16);
        c.qwc = 0;
        let mode = (c.chcr >> 2) & 3;
        guard += 1;
        if mode != 1 || c.ended || guard > 10_000 {
            hw.dmac.finish(i);
            return;
        }
        if i == SPR_FROM {
            // Destination chain: the tag comes from the scratchpad.
            let s = c.sadr as usize;
            let tag = u128::from_le_bytes(hw.spr[s..s + 16].try_into().unwrap()) as u64;
            let c = &mut hw.dmac.ch[i];
            c.sadr = (c.sadr + 16) & 0x3FF0;
            c.qwc = (tag & 0xFFFF) as u32;
            c.madr = (tag >> 32) as u32 & !0xF;
            let id = (tag >> 28) & 7;
            if id == 7 || (tag >> 31 & 1 != 0 && c.chcr & 0x80 != 0) {
                c.ended = true;
            }
        } else {
            source(hw, i);
            return;
        }
    }
}

/// Moves data between the EE and IOP SIF channels while both have it.
pub fn sif(hw: &mut Hw) {
    for _ in 0..64 {
        let mut progress = false;
        // ---- SIF1: EE (source chain, ch6) → FIFO → IOP (ch10)
        if running(hw, SIF1) && hw.dmac.sif1_fifo.len() < 4096 {
            source(hw, SIF1);
            progress = true;
        }
        if hw.iop_dma.sif1_pull(&mut hw.dmac.sif1_fifo, &mut hw.iop_ram) {
            progress = true;
        }
        // ---- SIF0: IOP (ch9) → FIFO → EE (destination chain, ch5)
        if hw.iop_dma.sif0_push(&mut hw.dmac.sif0_fifo, &hw.iop_ram) {
            progress = true;
        }
        if running(hw, SIF0) && sif0_ee(hw) {
            progress = true;
        }
        if !progress {
            break;
        }
    }
    let (eirq, done10, done9) = hw.iop_dma.take_irqs();
    let _ = (done10, done9);
    if eirq {
        hw.iop_irq(crate::hw::iopint::DMA);
    }
}

/// EE side of SIF0. Returns whether anything moved.
fn sif0_ee(hw: &mut Hw) -> bool {
    let mut moved = false;
    loop {
        let c = hw.dmac.ch[SIF0];
        if c.qwc > 0 {
            if hw.dmac.sif0_fifo.len() < 4 {
                return moved;
            }
            let mut q: u128 = 0;
            for k in 0..4 {
                q |= (hw.dmac.sif0_fifo.pop_front().unwrap() as u128) << (k * 32);
            }
            ram_write_qword(hw, c.madr, q);
            hw.dmac.bytes[SIF0] += 16;
            let c = &mut hw.dmac.ch[SIF0];
            c.madr = c.madr.wrapping_add(16);
            c.qwc -= 1;
            moved = true;
            continue;
        }
        if c.ended {
            hw.dmac.finish(SIF0);
            return true;
        }
        if hw.dmac.sif0_fifo.len() < 4 {
            return moved;
        }
        let lo = hw.dmac.sif0_fifo.pop_front().unwrap() as u64;
        let hi = hw.dmac.sif0_fifo.pop_front().unwrap() as u64;
        hw.dmac.sif0_fifo.pop_front();
        hw.dmac.sif0_fifo.pop_front();
        let tag = lo | hi << 32;
        let c = &mut hw.dmac.ch[SIF0];
        c.chcr = (c.chcr & 0xFFFF) | ((tag as u32) & 0xFFFF_0000);
        c.qwc = (tag & 0xFFFF) as u32;
        c.madr = (tag >> 32) as u32 & !0xF;
        let id = (tag >> 28) & 7;
        if id == 7 || id == 0 || (tag >> 31 & 1 != 0 && c.chcr & 0x80 != 0) {
            c.ended = true;
        }
        moved = true;
    }
}
