//! IOP DMA controller (channels 0-13). Only the SIF channels (9: IOP → EE,
//! 10: EE → IOP) move data; the others complete as soon as they start.

use std::collections::VecDeque;

use crate::hw::Hw;

pub const SIF0: usize = 9;
pub const SIF1: usize = 10;

#[derive(Default, Clone, Copy)]
pub struct Chan {
    pub madr: u32,
    pub bcr: u32,
    pub chcr: u32,
    pub tadr: u32,
    /// Words left for the current SIF tag.
    counter: u32,
    end: bool,
}

pub struct IopDma {
    pub ch: [Chan; 14],
    pub dpcr: u32,
    pub dicr: u32,
    pub dpcr2: u32,
    pub dicr2: u32,
    pub dmacen: u32,
    pub dmacinten: u32,
    irq: bool,
    pub transfers: [u32; 14],
    pub sif_log: Vec<String>,
}

fn base(i: usize) -> u32 {
    if i < 7 {
        0x1F80_1080 + i as u32 * 0x10
    } else {
        0x1F80_1500 + (i as u32 - 7) * 0x10
    }
}

fn index(a: u32) -> Option<(usize, u32)> {
    match a {
        0x1F80_1080..=0x1F80_10EF => Some((((a - 0x1F80_1080) >> 4) as usize, a & 0xF)),
        0x1F80_1500..=0x1F80_156F => Some((7 + ((a - 0x1F80_1500) >> 4) as usize, a & 0xF)),
        _ => None,
    }
}

impl IopDma {
    pub fn new() -> Self {
        IopDma { ch: [Chan::default(); 14], dpcr: 0, dicr: 0, dpcr2: 0, dicr2: 0, dmacen: 0, dmacinten: 0, irq: false, transfers: [0; 14], sif_log: Vec::new() }
    }

    pub fn read(&self, a: u32) -> u32 {
        if let Some((i, r)) = index(a) {
            let c = &self.ch[i];
            return match r {
                0 => c.madr,
                4 => c.bcr,
                8 => c.chcr,
                _ => c.tadr,
            };
        }
        let _ = base;
        match a {
            0x1F80_10F0 => self.dpcr,
            0x1F80_10F4 => self.dicr_value(),
            0x1F80_1570 => self.dpcr2,
            0x1F80_1574 => self.dicr2,
            0x1F80_1578 => self.dmacen,
            0x1F80_157C => self.dmacinten,
            _ => 0,
        }
    }

    fn dicr_value(&self) -> u32 {
        let f1 = (self.dicr >> 24) & (self.dicr >> 16) & 0x7F;
        let f2 = (self.dicr2 >> 24) & (self.dicr2 >> 16) & 0x7F;
        let master = self.dicr & 0x80_0000 != 0;
        let force = self.dicr & 0x8000 != 0;
        (self.dicr & 0x7FFF_FFFF) | (((force || master && (f1 | f2) != 0) as u32) << 31)
    }

    pub fn write(&mut self, a: u32, v: u32) {
        if let Some((i, r)) = index(a) {
            let c = &mut self.ch[i];
            match r {
                0 => c.madr = v & 0xFF_FFFF,
                4 => c.bcr = v,
                8 => {
                    if v & 0x0100_0000 != 0 && c.chcr & 0x0100_0000 == 0 {
                        c.counter = 0;
                        c.end = false;
                    }
                    c.chcr = v;
                }
                _ => c.tadr = v & 0xFF_FFFF,
            }
            return;
        }
        match a {
            0x1F80_10F0 => self.dpcr = v,
            0x1F80_10F4 => {
                let flags = (self.dicr & 0x7F00_0000) & !(v & 0x7F00_0000);
                self.dicr = (v & 0x00FF_FFFF) | flags;
            }
            0x1F80_1570 => self.dpcr2 = v,
            0x1F80_1574 => {
                let flags = (self.dicr2 & 0x7F00_0000) & !(v & 0x7F00_0000);
                self.dicr2 = (v & 0x00FF_FFFF) | flags;
            }
            0x1F80_1578 => self.dmacen = v,
            0x1F80_157C => self.dmacinten = v,
            _ => {}
        }
    }

    fn running(&self, i: usize) -> bool {
        self.ch[i].chcr & 0x0100_0000 != 0
    }

    fn finish(&mut self, i: usize) {
        self.ch[i].chcr &= !0x0100_0000;
        self.transfers[i] += 1;
        let (reg, bit) = if i < 7 { (&mut self.dicr, i) } else { (&mut self.dicr2, i - 7) };
        if *reg & (1 << (16 + bit)) != 0 {
            *reg |= 1 << (24 + bit);
            if self.dicr & 0x80_0000 != 0 {
                self.irq = true;
            }
        }
    }

    pub fn take_irqs(&mut self) -> (bool, bool, bool) {
        let r = self.irq;
        self.irq = false;
        (r, false, false)
    }

    /// IOP side of SIF1: tags and data arrive from the EE through `fifo`.
    pub fn sif1_pull(&mut self, fifo: &mut VecDeque<u32>, ram: &mut [u8]) -> bool {
        if !self.running(SIF1) {
            return false;
        }
        let mut moved = false;
        loop {
            let c = &mut self.ch[SIF1];
            if c.counter > 0 {
                if fifo.is_empty() {
                    return moved;
                }
                while c.counter > 0 {
                    let Some(w) = fifo.pop_front() else { break };
                    let a = (c.madr & 0x1F_FFFC) as usize;
                    ram[a..a + 4].copy_from_slice(&w.to_le_bytes());
                    c.madr = (c.madr + 4) & 0xFF_FFFF;
                    c.counter -= 1;
                }
                moved = true;
                continue;
            }
            if c.end {
                self.finish(SIF1);
                return true;
            }
            if fifo.len() < 4 {
                return moved;
            }
            let data = fifo.pop_front().unwrap();
            let words = fifo.pop_front().unwrap();
            fifo.pop_front();
            fifo.pop_front();
            c.madr = data & 0xFF_FFFF;
            c.counter = (words + 3) & !3;
            if data & 0xC000_0000 != 0 {
                c.end = true;
            }
            if self.sif_log.len() < 300 {
                let w: Vec<String> = fifo.iter().take(12).map(|x| format!("{x:08X}")).collect();
                self.sif_log.push(format!("EE->IOP {:06X} {} words: {}", data & 0xFF_FFFF, words, w.join(" ")));
            }
            moved = true;
        }
    }

    /// IOP side of SIF0: reads tags and data from IOP memory into `fifo`.
    pub fn sif0_push(&mut self, fifo: &mut VecDeque<u32>, ram: &[u8]) -> bool {
        if !self.running(SIF0) || fifo.len() > 4096 {
            return false;
        }
        let rd = |a: u32| {
            let a = (a & 0x1F_FFFC) as usize;
            u32::from_le_bytes([ram[a], ram[a + 1], ram[a + 2], ram[a + 3]])
        };
        let c = &mut self.ch[SIF0];
        if c.counter > 0 {
            while c.counter > 0 {
                fifo.push_back(rd(c.madr));
                c.madr = (c.madr + 4) & 0xFF_FFFF;
                c.counter -= 1;
            }
            return true;
        }
        if c.end {
            self.finish(SIF0);
            return true;
        }
        let data = rd(c.tadr);
        let words = rd(c.tadr + 4);
        for k in 0..4 {
            fifo.push_back(rd(c.tadr + 8 + k * 4));
        }
        if self.sif_log.len() < 300 {
            let w: Vec<String> = (0..12).map(|k| format!("{:08X}", rd((data & 0xFF_FFFF) + k * 4))).collect();
            self.sif_log.push(format!("IOP->EE {:06X} {} words: {}", data & 0xFF_FFFF, words, w.join(" ")));
        }
        c.tadr = (c.tadr + 16) & 0xFF_FFFF;
        c.madr = data & 0xFF_FFFF;
        c.counter = (words + 3) & !3;
        if data & 0xC000_0000 != 0 {
            c.end = true;
        }
        true
    }
}

pub fn run(hw: &mut Hw) {
    for i in 0..14 {
        if i == SIF0 || i == SIF1 || !hw.iop_dma.running(i) {
            continue;
        }
        let c = hw.iop_dma.ch[i];
        let words = ((c.bcr & 0xFFFF).max(1) * (c.bcr >> 16).max(1)) as usize;
        match i {
            11 => {
                // Memory → SIO2 (commands).
                for k in 0..words * 4 {
                    let a = ((c.madr as usize) + k) & 0x1F_FFFF;
                    hw.sio2.fifo_in.push_back(hw.iop_ram[a]);
                }
            }
            12 => {
                // SIO2 → memory (replies): wait for them.
                if hw.sio2.fifo_out.is_empty() {
                    continue;
                }
                for k in 0..words * 4 {
                    let a = ((c.madr as usize) + k) & 0x1F_FFFF;
                    hw.iop_ram[a] = hw.sio2.fifo_out.pop_front().unwrap_or(0xFF);
                }
            }
            _ => {}
        }
        hw.iop_dma.finish(i);
    }
    crate::dmac::sif(hw);
}
