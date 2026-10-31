//! Memory map, interrupt controller and DMA. Owns every peripheral.

use crate::cdrom::Cdrom;
use crate::gpu::Gpu;
use crate::sio::Sio;
use crate::spu::Spu;
use crate::timers::Timers;

pub const IRQ_VBLANK: u32 = 0;
pub const IRQ_GPU: u32 = 1;
pub const IRQ_CDROM: u32 = 2;
pub const IRQ_DMA: u32 = 3;
pub const IRQ_TIMER0: u32 = 4;
pub const IRQ_SIO0: u32 = 7;
pub const IRQ_SPU: u32 = 9;

const REGION_MASK: [u32; 8] = [
    0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, // KUSEG
    0x7FFF_FFFF, // KSEG0
    0x1FFF_FFFF, // KSEG1
    0xFFFF_FFFF, 0xFFFF_FFFF, // KSEG2
];

#[inline(always)]
fn phys(addr: u32) -> u32 {
    addr & REGION_MASK[(addr >> 29) as usize]
}

pub struct Bus {
    pub ram: Vec<u8>,
    pub scratch: [u8; 1024],
    pub bios: Vec<u8>,
    pub gpu: Gpu,
    pub cdrom: Cdrom,
    pub sio: Sio,
    pub spu: Spu,
    pub timers: Timers,
    pub istat: u32,
    pub imask: u32,
    mem_ctrl: [u32; 9],
    ram_size: u32,
    cache_ctrl: u32,
    // DMA
    madr: [u32; 7],
    bcr: [u32; 7],
    chcr: [u32; 7],
    dpcr: u32,
    dicr: u32,
    /// Extra cycles spent on slow memory accesses during the current step.
    pub access_cycles: u32,
    /// Bytes written to the debug UART (expansion 2), for the test harness.
    pub debug_out: Vec<u8>,
    /// I/O addresses with no emulated device, noted once each for the debug
    /// report.
    pub unknown_io: std::collections::BTreeSet<(u32, bool)>,
}

impl Bus {
    pub fn new(bios: Vec<u8>) -> Self {
        let mut b = bios;
        b.resize(512 * 1024, 0);
        Bus {
            ram: vec![0; 2 * 1024 * 1024],
            scratch: [0; 1024],
            bios: b,
            gpu: Gpu::new(),
            cdrom: Cdrom::new(),
            sio: Sio::new(),
            spu: Spu::new(),
            timers: Timers::new(),
            istat: 0,
            imask: 0,
            mem_ctrl: [0; 9],
            ram_size: 0,
            cache_ctrl: 0,
            madr: [0; 7],
            bcr: [0; 7],
            chcr: [0; 7],
            dpcr: 0x0765_4321,
            dicr: 0,
            access_cycles: 0,
            debug_out: Vec::new(),
            unknown_io: std::collections::BTreeSet::new(),
        }
    }

    pub fn raise(&mut self, irq: u32) {
        self.istat |= 1 << irq;
    }

    #[inline(always)]
    pub fn update_irq_line(&mut self) {}

    #[inline(always)]
    pub fn irq_pending(&self) -> bool {
        self.istat & self.imask & 0x7FF != 0
    }

    /// Advances every peripheral by `cycles` CPU cycles.
    pub fn tick(&mut self, cycles: u32) {
        let ev = self.gpu.tick(cycles);
        if ev.vblank_start {
            self.raise(IRQ_VBLANK);
        }
        let tirq = self.timers.tick(cycles, &ev, self.gpu.dot_divider());
        for i in 0..3 {
            if tirq & (1 << i) != 0 {
                self.raise(IRQ_TIMER0 + i);
            }
        }
        if self.cdrom.tick(cycles) {
            self.raise(IRQ_CDROM);
        }
        if self.sio.tick(cycles) {
            self.raise(IRQ_SIO0);
        }
        if self.spu.tick(cycles) {
            self.raise(IRQ_SPU);
        }
    }

    #[inline(always)]
    pub fn fetch(&mut self, addr: u32) -> u32 {
        let p = phys(addr);
        if p < 0x0080_0000 {
            let a = (p & 0x1F_FFFF) as usize;
            return u32::from_le_bytes([self.ram[a], self.ram[a + 1], self.ram[a + 2], self.ram[a + 3]]);
        }
        if (0x1FC0_0000..0x1FC8_0000).contains(&p) {
            let a = (p & 0x7_FFFF) as usize;
            self.access_cycles += 4;
            return u32::from_le_bytes([self.bios[a], self.bios[a + 1], self.bios[a + 2], self.bios[a + 3]]);
        }
        self.read32(addr)
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => self.ram[(p & 0x1F_FFFF) as usize],
            0x1F80_0000..=0x1F80_03FF => self.scratch[(p & 0x3FF) as usize],
            0x1FC0_0000..=0x1FC7_FFFF => { self.access_cycles += 6; self.bios[(p & 0x7_FFFF) as usize] }
            0x1F00_0000..=0x1F7F_FFFF => 0xFF, // no expansion cartridge
            0x1F80_1800..=0x1F80_1803 => self.cdrom.read(p & 3),
            0x1F80_1040..=0x1F80_104F => self.sio.read(p & 0xF) as u8,
            0x1F80_2000..=0x1F80_2FFF => 0,
            _ => {
                let v = self.io_read(p & !3);
                (v >> ((p & 3) * 8)) as u8
            }
        }
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => {
                let a = (p & 0x1F_FFFF) as usize;
                u16::from_le_bytes([self.ram[a], self.ram[a + 1]])
            }
            0x1F80_0000..=0x1F80_03FF => {
                let a = (p & 0x3FF) as usize;
                u16::from_le_bytes([self.scratch[a], self.scratch[a + 1]])
            }
            0x1FC0_0000..=0x1FC7_FFFF => {
                self.access_cycles += 6;
                let a = (p & 0x7_FFFF) as usize;
                u16::from_le_bytes([self.bios[a], self.bios[a + 1]])
            }
            0x1F80_1040..=0x1F80_104F => self.sio.read(p & 0xF),
            0x1F80_1C00..=0x1F80_1FFF => self.spu.read16(p),
            0x1F80_1800..=0x1F80_1803 => {
                let lo = self.cdrom.read(p & 3) as u16;
                let hi = self.cdrom.read(p & 3) as u16;
                lo | (hi << 8)
            }
            0x1F00_0000..=0x1F7F_FFFF => 0xFFFF,
            _ => {
                let v = self.io_read(p & !3);
                (v >> ((p & 2) * 8)) as u16
            }
        }
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => {
                let a = (p & 0x1F_FFFF) as usize;
                u32::from_le_bytes([self.ram[a], self.ram[a + 1], self.ram[a + 2], self.ram[a + 3]])
            }
            0x1F80_0000..=0x1F80_03FF => {
                let a = (p & 0x3FF) as usize;
                u32::from_le_bytes([self.scratch[a], self.scratch[a + 1], self.scratch[a + 2], self.scratch[a + 3]])
            }
            0x1FC0_0000..=0x1FC7_FFFF => {
                self.access_cycles += 12;
                let a = (p & 0x7_FFFF) as usize;
                u32::from_le_bytes([self.bios[a], self.bios[a + 1], self.bios[a + 2], self.bios[a + 3]])
            }
            0x1F00_0000..=0x1F7F_FFFF => 0xFFFF_FFFF,
            0x1F80_1C00..=0x1F80_1FFF => self.spu.read16(p) as u32 | ((self.spu.read16(p + 2) as u32) << 16),
            _ => self.io_read(p),
        }
    }

    fn io_read(&mut self, p: u32) -> u32 {
        match p {
            0x1F80_1000..=0x1F80_1020 => self.mem_ctrl[((p - 0x1F80_1000) / 4) as usize],
            0x1F80_1060 => self.ram_size,
            0x1F80_1040..=0x1F80_104F => self.sio.read(p & 0xF) as u32,
            0x1F80_1050..=0x1F80_105F => 0,
            0x1F80_1070 => self.istat,
            0x1F80_1074 => self.imask,
            0x1F80_1080..=0x1F80_10EF => {
                let ch = ((p >> 4) & 7) as usize;
                match p & 0xF {
                    0 => self.madr[ch],
                    4 => self.bcr[ch],
                    8 => self.chcr[ch],
                    _ => 0,
                }
            }
            0x1F80_10F0 => self.dpcr,
            0x1F80_10F4 => self.dicr,
            0x1F80_1100..=0x1F80_112F => self.timers.read(p),
            0x1F80_1810 => self.gpu.read_data(),
            0x1F80_1814 => self.gpu.status(),
            0x1F80_1820 | 0x1F80_1824 => 0, // MDEC
            0x1F80_1C00..=0x1F80_1FFF => self.spu.read16(p) as u32,
            0xFFFE_0130 => self.cache_ctrl,
            _ => {
                self.note_unknown(p, false);
                0
            }
        }
    }

    pub fn write8(&mut self, addr: u32, v: u8) {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => self.ram[(p & 0x1F_FFFF) as usize] = v,
            0x1F80_0000..=0x1F80_03FF => self.scratch[(p & 0x3FF) as usize] = v,
            0x1F80_1800..=0x1F80_1803 => {
                if self.cdrom.write(p & 3, v) {
                    self.raise(IRQ_CDROM);
                }
            }
            0x1F80_1040 => self.sio.write(0, v as u16),
            0x1F80_2000..=0x1F80_2FFF => {
                // Expansion 2: DUART TX (0x1F802023) and the POST display.
                if p == 0x1F80_2023 || p == 0x1F80_2080 {
                    if self.debug_out.len() < 1 << 16 {
                        self.debug_out.push(v);
                    }
                }
            }
            _ => self.io_write(p & !3, (v as u32) << ((p & 3) * 8), 1),
        }
    }

    pub fn write16(&mut self, addr: u32, v: u16) {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => {
                let a = (p & 0x1F_FFFF) as usize;
                self.ram[a..a + 2].copy_from_slice(&v.to_le_bytes());
            }
            0x1F80_0000..=0x1F80_03FF => {
                let a = (p & 0x3FF) as usize;
                self.scratch[a..a + 2].copy_from_slice(&v.to_le_bytes());
            }
            0x1F80_1C00..=0x1F80_1FFF => self.spu.write16(p, v),
            0x1F80_1040..=0x1F80_104F => self.sio.write(p & 0xF, v),
            0x1F80_1100..=0x1F80_112F => self.timers.write(p, v as u32),
            0x1F80_1070 | 0x1F80_1074 => self.io_write(p, v as u32, 2),
            _ => self.io_write(p & !3, (v as u32) << ((p & 2) * 8), 2),
        }
    }

    pub fn write32(&mut self, addr: u32, v: u32) {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => {
                let a = (p & 0x1F_FFFF) as usize;
                self.ram[a..a + 4].copy_from_slice(&v.to_le_bytes());
            }
            0x1F80_0000..=0x1F80_03FF => {
                let a = (p & 0x3FF) as usize;
                self.scratch[a..a + 4].copy_from_slice(&v.to_le_bytes());
            }
            0x1F80_1C00..=0x1F80_1FFF => {
                self.spu.write16(p, v as u16);
                self.spu.write16(p + 2, (v >> 16) as u16);
            }
            _ => self.io_write(p, v, 4),
        }
    }

    fn io_write(&mut self, p: u32, v: u32, _width: u32) {
        match p {
            0x1F80_1000..=0x1F80_1020 => self.mem_ctrl[((p - 0x1F80_1000) / 4) as usize] = v,
            0x1F80_1060 => self.ram_size = v,
            0x1F80_1040..=0x1F80_104F => self.sio.write(p & 0xF, v as u16),
            0x1F80_1070 => self.istat &= v,
            0x1F80_1074 => self.imask = v & 0x7FF,
            0x1F80_1080..=0x1F80_10EF => {
                let ch = ((p >> 4) & 7) as usize;
                match p & 0xF {
                    0 => self.madr[ch] = v & 0xFF_FFFF,
                    4 => self.bcr[ch] = v,
                    8 => {
                        self.chcr[ch] = if ch == 6 { (v & 0x5100_0000) | 2 } else { v };
                        self.maybe_run_dma(ch);
                    }
                    _ => {}
                }
            }
            0x1F80_10F0 => self.dpcr = v,
            0x1F80_10F4 => {
                let ack = v & 0x7F00_0000;
                self.dicr = (self.dicr & 0x7F00_0000 & !ack) | (v & 0x00FF_803F);
                self.update_dicr_master();
            }
            0x1F80_1100..=0x1F80_112F => self.timers.write(p, v),
            0x1F80_1810 => self.gpu.gp0(v),
            0x1F80_1814 => {
                if self.gpu.gp1(v) {
                    // GPU IRQ (GP0 0x1F) is rarely used; nothing to do here.
                }
            }
            0x1F80_1820 | 0x1F80_1824 => {} // MDEC
            0xFFFE_0130 => self.cache_ctrl = v,
            _ => self.note_unknown(p, true),
        }
    }

    fn note_unknown(&mut self, p: u32, write: bool) {
        if self.unknown_io.len() < 256 {
            self.unknown_io.insert((p, write));
        }
    }

    // ---- DMA ----

    fn update_dicr_master(&mut self) {
        let force = self.dicr & (1 << 15) != 0;
        let enabled = self.dicr & (1 << 23) != 0;
        let flags = (self.dicr >> 24) & 0x7F;
        let en = (self.dicr >> 16) & 0x7F;
        let master = force || (enabled && flags & en != 0);
        let was = self.dicr & (1 << 31) != 0;
        if master {
            self.dicr |= 1 << 31;
            if !was {
                self.raise(IRQ_DMA);
            }
        } else {
            self.dicr &= !(1 << 31);
        }
    }

    fn maybe_run_dma(&mut self, ch: usize) {
        let c = self.chcr[ch];
        let sync = (c >> 9) & 3;
        let start = if sync == 0 { c & (1 << 24) != 0 && c & (1 << 28) != 0 } else { c & (1 << 24) != 0 };
        if !start {
            return;
        }
        if sync == 2 {
            self.dma_linked(ch);
        } else {
            self.dma_block(ch);
        }
        self.chcr[ch] &= !((1 << 24) | (1 << 28));
        if self.dicr & (1 << (16 + ch)) != 0 {
            self.dicr |= 1 << (24 + ch);
        }
        self.update_dicr_master();
    }

    fn ram_r32(&self, a: u32) -> u32 {
        let a = (a & 0x1F_FFFC) as usize;
        u32::from_le_bytes([self.ram[a], self.ram[a + 1], self.ram[a + 2], self.ram[a + 3]])
    }

    fn ram_w32(&mut self, a: u32, v: u32) {
        let a = (a & 0x1F_FFFC) as usize;
        self.ram[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn dma_block(&mut self, ch: usize) {
        let c = self.chcr[ch];
        let from_ram = c & 1 != 0;
        let step: u32 = if c & 2 != 0 { (-4i32) as u32 } else { 4 };
        let sync = (c >> 9) & 3;
        let words = if sync == 0 {
            let n = self.bcr[ch] & 0xFFFF;
            if n == 0 { 0x10000 } else { n }
        } else {
            (self.bcr[ch] & 0xFFFF) * (self.bcr[ch] >> 16)
        };
        let mut addr = self.madr[ch];
        for i in 0..words {
            if from_ram {
                let v = self.ram_r32(addr);
                match ch {
                    2 => self.gpu.gp0(v),
                    4 => self.spu.dma_write(v),
                    _ => {}
                }
            } else {
                let v = match ch {
                    6 => {
                        if i == words - 1 { 0x00FF_FFFF } else { addr.wrapping_sub(4) & 0x1F_FFFF }
                    }
                    2 => self.gpu.read_data(),
                    3 => self.cdrom.dma_read(),
                    4 => self.spu.dma_read(),
                    _ => 0,
                };
                self.ram_w32(addr, v);
            }
            addr = addr.wrapping_add(step) & 0x1F_FFFF;
        }
        if sync != 0 {
            self.madr[ch] = addr;
            self.bcr[ch] &= 0xFFFF;
        }
    }

    fn dma_linked(&mut self, ch: usize) {
        if ch != 2 {
            return;
        }
        let mut addr = self.madr[ch] & 0x1F_FFFC;
        // Guard against corrupted lists that loop forever.
        for _ in 0..0x20000 {
            let header = self.ram_r32(addr);
            let n = header >> 24;
            for i in 0..n {
                let v = self.ram_r32(addr.wrapping_add(4 * (i + 1)));
                self.gpu.gp0(v);
            }
            if header & 0x80_0000 != 0 {
                break;
            }
            addr = header & 0x1F_FFFC;
        }
        self.madr[ch] = 0x00FF_FFFF;
    }
}
