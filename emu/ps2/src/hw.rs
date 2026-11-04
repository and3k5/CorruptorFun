//! Memory maps and the simple registers of both PS2 processors: the EE (main
//! CPU) and the IOP (the I/O processor, a PS1-class R3000A). They share the
//! BIOS ROM and talk through the SIF (mailbox registers plus DMA).

use std::collections::BTreeMap;

use crate::dmac::Dmac;
use crate::gs::Gs;
use crate::iopdma::IopDma;

pub const RAM_SIZE: usize = 32 << 20;
pub const IOP_RAM_SIZE: usize = 2 << 20;

/// EE interrupt controller bits.
pub mod int {
    pub const GS: u32 = 0;
    pub const SBUS: u32 = 1;
    pub const VBON: u32 = 2;
    pub const VBOF: u32 = 3;
    pub const VIF1: u32 = 5;
    pub const TIM0: u32 = 9;
}

/// IOP interrupt numbers.
pub mod iopint {
    pub const VBLANK: u32 = 0;
    pub const CDVD: u32 = 2;
    pub const DMA: u32 = 3;
    pub const TIM0: u32 = 4;
    pub const EVBLANK: u32 = 11;
    pub const TIM3: u32 = 14;
    pub const SIO2: u32 = 17;
}

#[derive(Default, Clone, Copy)]
pub struct EeTimer {
    pub count: u32,
    pub mode: u32,
    pub comp: u32,
    pub hold: u32,
    frac: u32,
}

#[derive(Default, Clone, Copy)]
pub struct IopTimer {
    pub count: u32,
    pub mode: u32,
    pub target: u32,
    frac: u32,
}

pub struct Hw {
    pub ram: Vec<u8>,
    pub iop_ram: Vec<u8>,
    pub bios: Vec<u8>,
    pub rom1: Vec<u8>,
    pub spr: Vec<u8>,
    pub vu0_code: Vec<u8>,
    pub vu0_data: Vec<u8>,
    pub vu1_code: Vec<u8>,
    pub vu1_data: Vec<u8>,

    // EE side
    pub intc_stat: u32,
    pub intc_mask: u32,
    pub timers: [EeTimer; 4],
    pub dmac: Dmac,
    pub vif0: crate::vif::Vif,
    pub vif1: crate::vif::Vif,
    pub vu0: crate::vu::Vu,
    pub vu1: crate::vu::Vu,
    pub gs: Gs,
    pub mch_ricm: u32,
    pub mch_drd: u32,
    rdram_sdevid: u32,

    // SIF mailbox
    pub sif_mscom: u32,
    pub sif_smcom: u32,
    pub sif_msflg: u32,
    pub sif_smflg: u32,
    pub sif_ctrl: u32,
    pub sif_bd6: u32,

    // IOP side
    pub iop_stat: u32,
    pub iop_mask: u32,
    pub iop_ctrl: u32,
    pub iop_timers: [IopTimer; 6],
    pub iop_dma: IopDma,
    pub cdvd: crate::cdvd::Cdvd,
    pub sio2: crate::sio2::Sio2,
    pub iop_cache_ctrl: u32,
    pub iop_irq_counts: [u32; 32],

    /// TLB lookup per 4 KB page: 0 = unmapped, else phys page | 1, | 2 for
    /// the scratchpad.
    pub tlb_map: Vec<u32>,
    tlb_mapped: Vec<u32>,
    /// Plain registers we only store (and log the first time).
    pub regs: BTreeMap<u32, u32>,
    /// I/O that no emulated device handles: (address, write, iop).
    pub unknown_io: BTreeMap<(u32, bool, bool), u32>,
    /// EE serial port output (the kernel prints its boot messages there).
    pub ee_tty: Vec<u8>,
    /// IOP debug output (bytes written to the IOP's printf hook).
    pub iop_tty: Vec<u8>,
    pub trace: Vec<String>,
}

#[inline(always)]
fn rd32(m: &[u8], a: usize) -> u32 {
    u32::from_le_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]])
}
#[inline(always)]
fn wr32(m: &mut [u8], a: usize, v: u32) {
    m[a..a + 4].copy_from_slice(&v.to_le_bytes());
}

impl Hw {
    pub fn new(mut bios: Vec<u8>) -> Self {
        if bios.len() < 4 << 20 {
            bios.resize(4 << 20, 0);
        }
        Hw {
            ram: vec![0; RAM_SIZE],
            iop_ram: vec![0; IOP_RAM_SIZE],
            bios,
            rom1: Vec::new(),
            spr: vec![0; 16 << 10],
            vu0_code: vec![0; 4 << 10],
            vu0_data: vec![0; 4 << 10],
            vu1_code: vec![0; 16 << 10],
            vu1_data: vec![0; 16 << 10],
            intc_stat: 0,
            intc_mask: 0,
            timers: [EeTimer::default(); 4],
            dmac: Dmac::new(),
            vif0: Default::default(),
            vif1: Default::default(),
            vu0: Default::default(),
            vu1: Default::default(),
            gs: Gs::new(),
            mch_ricm: 0,
            mch_drd: 0,
            rdram_sdevid: 0,
            sif_mscom: 0,
            sif_smcom: 0,
            sif_msflg: 0,
            sif_smflg: 0,
            sif_ctrl: 0,
            sif_bd6: 0,
            iop_stat: 0,
            iop_mask: 0,
            iop_ctrl: 0,
            iop_timers: [IopTimer::default(); 6],
            iop_dma: IopDma::new(),
            cdvd: crate::cdvd::Cdvd::new(),
            sio2: crate::sio2::Sio2::new(),
            iop_cache_ctrl: 0,
            iop_irq_counts: [0; 32],
            tlb_map: vec![0; 1 << 20],
            tlb_mapped: Vec::new(),
            regs: BTreeMap::new(),
            unknown_io: BTreeMap::new(),
            ee_tty: Vec::new(),
            iop_tty: Vec::new(),
            trace: Vec::new(),
        }
    }

    pub fn ee_irq(&mut self, bit: u32) {
        self.intc_stat |= 1 << bit;
    }

    pub fn iop_irq(&mut self, n: u32) {
        self.iop_stat |= 1 << n;
        self.iop_irq_counts[n as usize] += 1;
    }

    /// INT0 (INTC) and INT1 (DMAC) lines into the EE.
    pub fn ee_int_lines(&self) -> (bool, bool) {
        (self.intc_stat & self.intc_mask != 0, self.dmac.int1())
    }

    pub fn iop_irq_pending(&self) -> bool {
        self.iop_ctrl & 1 != 0 && self.iop_stat & self.iop_mask != 0
    }

    fn unknown(&mut self, addr: u32, write: bool, iop: bool) {
        let e = self.unknown_io.entry((addr, write, iop)).or_insert(0);
        *e += 1;
    }

    // ---------------------------------------------------------------- EE --

    /// Physical address for an EE virtual address. The BIOS maps kuseg
    /// one-to-one (with uncached aliases at 0x2000_0000 and 0x3000_0000) and
    /// the scratchpad at 0x7000_0000; we hardwire that instead of walking
    /// the TLB.
    /// Translates an EE virtual address. kseg0/kseg1 are direct; other
    /// addresses go through the TLB, and fall back to a one-to-one mapping
    /// when no entry covers them. Scratchpad addresses come back with bit 31
    /// set.
    #[inline(always)]
    fn ee_xlate(&self, addr: u32) -> u32 {
        if addr & 0xC000_0000 == 0x8000_0000 {
            return addr & 0x1FFF_FFFF;
        }
        let e = self.tlb_map[(addr >> 12) as usize];
        if e == 0 {
            if addr >> 14 == 0x7000_0000 >> 14 {
                return 0x8000_0000 | (addr & 0x3FFF);
            }
            return addr & 0x1FFF_FFFF;
        }
        if e & 2 != 0 {
            return 0x8000_0000 | (addr & 0x3FFF);
        }
        (e & !0xFFF) | (addr & 0xFFF)
    }

    /// Rebuilds the page lookup from all TLB entries, each
    /// [PageMask, EntryHi, EntryLo0, EntryLo1].
    pub fn tlb_rebuild(&mut self, tlb: &[[u32; 4]]) {
        for p in self.tlb_mapped.drain(..) {
            self.tlb_map[p as usize] = 0;
        }
        for e in tlb {
            let pages = ((e[0] >> 13) & 0xFFF) + 1; // 4 KB pages per half
            let vpn = (e[1] >> 12) & !(pages * 2 - 1);
            if e[2] & 0x8000_0000 != 0 {
                // Scratchpad: 16 KB.
                for k in 0..4 {
                    let v = ((e[1] >> 12) & !3) + k;
                    self.tlb_map[v as usize] = 3;
                    self.tlb_mapped.push(v);
                }
                continue;
            }
            for half in 0..2u32 {
                let lo = e[2 + half as usize];
                if lo & 2 == 0 {
                    continue;
                }
                for k in 0..pages {
                    let v = vpn + half * pages + k;
                    if v as usize >= self.tlb_map.len() {
                        continue;
                    }
                    let pfn = ((lo >> 6) & 0xF_FFFF) + k;
                    self.tlb_map[v as usize] = ((pfn << 12) & 0x1FFF_F000) | 1;
                    self.tlb_mapped.push(v);
                }
            }
        }
    }

    pub fn ee_read8(&mut self, addr: u32) -> u8 {
        let p = self.ee_xlate(addr);
        if p & 0x8000_0000 != 0 {
            return self.spr[(p & 0x3FFF) as usize];
        }
        match p {
            0..=0x01FF_FFFF => self.ram[p as usize],
            0x1FC0_0000..=0x1FFF_FFFF => self.bios_byte(p),
            0x1C00_0000..=0x1C1F_FFFF => self.iop_ram[(p & 0x1F_FFFF) as usize],
            _ => (self.ee_io_read32(p & !3) >> ((p & 3) * 8)) as u8,
        }
    }

    pub fn ee_read16(&mut self, addr: u32) -> u16 {
        let p = self.ee_xlate(addr);
        if p & 0x8000_0000 != 0 {
            let a = (p & 0x3FFE) as usize;
            return u16::from_le_bytes([self.spr[a], self.spr[a + 1]]);
        }
        match p {
            0..=0x01FF_FFFF => u16::from_le_bytes([self.ram[p as usize], self.ram[p as usize + 1]]),
            0x1FC0_0000..=0x1FFF_FFFF => self.bios_byte(p) as u16 | (self.bios_byte(p + 1) as u16) << 8,
            0x1C00_0000..=0x1C1F_FFFF => {
                let a = (p & 0x1F_FFFE) as usize;
                u16::from_le_bytes([self.iop_ram[a], self.iop_ram[a + 1]])
            }
            _ => (self.ee_io_read32(p & !3) >> ((p & 2) * 8)) as u16,
        }
    }

    pub fn ee_read32(&mut self, addr: u32) -> u32 {
        let p = self.ee_xlate(addr);
        if p & 0x8000_0000 != 0 {
            return rd32(&self.spr, (p & 0x3FFC) as usize);
        }
        match p {
            0..=0x01FF_FFFF => rd32(&self.ram, p as usize),
            0x1FC0_0000..=0x1FFF_FFFF => {
                let a = (p - 0x1FC0_0000) as usize;
                if a + 4 <= self.bios.len() { rd32(&self.bios, a) } else { 0 }
            }
            0x1C00_0000..=0x1C1F_FFFF => rd32(&self.iop_ram, (p & 0x1F_FFFC) as usize),
            _ => self.ee_io_read32(p),
        }
    }

    pub fn ee_read64(&mut self, addr: u32) -> u64 {
        let p = self.ee_xlate(addr);
        if p < 0x0200_0000 {
            let a = p as usize & !7;
            return u64::from_le_bytes(self.ram[a..a + 8].try_into().unwrap());
        }
        let a = addr & !7;
        self.ee_read32(a) as u64 | (self.ee_read32(a + 4) as u64) << 32
    }

    pub fn ee_read128(&mut self, addr: u32) -> u128 {
        let a = addr & !15;
        let p = self.ee_xlate(a);
        if p < 0x0200_0000 {
            return u128::from_le_bytes(self.ram[p as usize..p as usize + 16].try_into().unwrap());
        }
        if (0x1000_4000..0x1000_8000).contains(&p) {
            // FIFO reads (VIF1 / IPU out): nothing there.
            return 0;
        }
        self.ee_read64(a) as u128 | (self.ee_read64(a + 8) as u128) << 64
    }

    pub fn ee_write8(&mut self, addr: u32, v: u8) {
        let p = self.ee_xlate(addr);
        if p & 0x8000_0000 != 0 {
            self.spr[(p & 0x3FFF) as usize] = v;
            return;
        }
        match p {
            0..=0x01FF_FFFF => self.ram[p as usize] = v,
            0x1C00_0000..=0x1C1F_FFFF => self.iop_ram[(p & 0x1F_FFFF) as usize] = v,
            0x1000_F180 => {
                if self.ee_tty.len() < 1 << 16 {
                    self.ee_tty.push(v);
                }
                if v == b'\n' && self.trace.len() < 400 {
                    let start = self.ee_tty[..self.ee_tty.len() - 1].iter().rposition(|&c| c == b'\n').map_or(0, |p| p + 1);
                    let line = String::from_utf8_lossy(&self.ee_tty[start..self.ee_tty.len() - 1]).to_string();
                    if !line.is_empty() {
                        self.trace.push(format!("EE: {line}"));
                    }
                }
            }
            0x1FC0_0000..=0x1FFF_FFFF => {}
            _ => {
                let sh = (p & 3) * 8;
                self.ee_io_write32(p & !3, (v as u32) << sh, 0xFF << sh);
            }
        }
    }

    pub fn ee_write16(&mut self, addr: u32, v: u16) {
        let p = self.ee_xlate(addr);
        if p & 0x8000_0000 != 0 {
            let a = (p & 0x3FFE) as usize;
            self.spr[a..a + 2].copy_from_slice(&v.to_le_bytes());
            return;
        }
        match p {
            0..=0x01FF_FFFF => {
                let a = p as usize & !1;
                self.ram[a..a + 2].copy_from_slice(&v.to_le_bytes());
            }
            0x1C00_0000..=0x1C1F_FFFF => {
                let a = (p & 0x1F_FFFE) as usize;
                self.iop_ram[a..a + 2].copy_from_slice(&v.to_le_bytes());
            }
            0x1FC0_0000..=0x1FFF_FFFF => {}
            _ => {
                let sh = (p & 2) * 8;
                self.ee_io_write32(p & !3, (v as u32) << sh, 0xFFFF << sh);
            }
        }
    }

    pub fn ee_write32(&mut self, addr: u32, v: u32) {
        let p = self.ee_xlate(addr);
        if p & 0x8000_0000 != 0 {
            wr32(&mut self.spr, (p & 0x3FFC) as usize, v);
            return;
        }
        match p {
            0..=0x01FF_FFFF => wr32(&mut self.ram, p as usize & !3, v),
            0x1C00_0000..=0x1C1F_FFFF => wr32(&mut self.iop_ram, (p & 0x1F_FFFC) as usize, v),
            0x1FC0_0000..=0x1FFF_FFFF => {}
            _ => self.ee_io_write32(p, v, 0xFFFF_FFFF),
        }
    }

    pub fn ee_write64(&mut self, addr: u32, v: u64) {
        let p = self.ee_xlate(addr);
        if p < 0x0200_0000 {
            let a = p as usize & !7;
            self.ram[a..a + 8].copy_from_slice(&v.to_le_bytes());
            return;
        }
        if (0x1200_0000..0x1200_2000).contains(&p) {
            self.gs.write_priv64(p & !7, v);
            return;
        }
        let a = addr & !7;
        self.ee_write32(a, v as u32);
        self.ee_write32(a + 4, (v >> 32) as u32);
    }

    pub fn ee_write128(&mut self, addr: u32, v: u128) {
        let a = addr & !15;
        let p = self.ee_xlate(a);
        if p < 0x0200_0000 {
            self.ram[p as usize..p as usize + 16].copy_from_slice(&v.to_le_bytes());
            return;
        }
        match p {
            0x1000_4000..=0x1000_4FFF => self.dmac.vif0_fifo.push(v),
            0x1000_5000..=0x1000_5FFF => self.dmac.vif1_fifo.push(v),
            0x1000_6000..=0x1000_6FFF => self.gs.gif_fifo_write(v),
            0x1000_7000..=0x1000_7FFF => {}
            _ => {
                self.ee_write64(a, v as u64);
                self.ee_write64(a + 8, (v >> 64) as u64);
            }
        }
    }

    fn bios_byte(&self, p: u32) -> u8 {
        *self.bios.get((p - 0x1FC0_0000) as usize).unwrap_or(&0)
    }

    fn ee_io_read32(&mut self, p: u32) -> u32 {
        match p {
            0x1000_0000..=0x1000_1FFF => {
                let t = &self.timers[((p >> 11) & 3) as usize];
                match (p >> 4) & 0xF {
                    0 => t.count & 0xFFFF,
                    1 => t.mode,
                    2 => t.comp,
                    3 => t.hold,
                    _ => 0,
                }
            }
            0x1000_2000..=0x1000_2FFF => match p {
                0x1000_2010 => 0, // IPU_CTRL: idle
                _ => 0,
            },
            0x1000_3000..=0x1000_3FFF => match p {
                0x1000_3020 => self.gs.gif_stat(),
                0x1000_3800 | 0x1000_3C00 => 0, // VIF_STAT: idle, FIFO empty
                _ => *self.regs.get(&p).unwrap_or(&0),
            },
            0x1000_8000..=0x1000_EFFF => self.dmac.read(p),
            0x1000_F000 => self.intc_stat,
            0x1000_F010 => self.intc_mask,
            0x1000_F130 => 0,
            0x1000_F200 => self.sif_mscom,
            0x1000_F210 => self.sif_smcom,
            0x1000_F220 => self.sif_msflg,
            0x1000_F230 => self.sif_smflg,
            0x1000_F240 => self.sif_ctrl | 0xF000_0102,
            0x1000_F260 => 0,
            0x1000_F430 => self.mch_ricm,
            0x1000_F440 => self.mch_drd_read(),
            0x1000_F400..=0x1000_F4FF => *self.regs.get(&p).unwrap_or(&0),
            0x1000_F520 => self.dmac.enable,
            0x1000_F590 => self.dmac.enable,
            0x1100_0000..=0x1100_FFFF => {
                let m = match p & 0xC000 {
                    0x0000 => &self.vu0_code,
                    0x4000 => &self.vu0_data,
                    0x8000 => &self.vu1_code,
                    _ => &self.vu1_data,
                };
                let a = (p as usize) & (m.len() - 1) & !3;
                rd32(m, a)
            }
            0x1200_0000..=0x1200_1FFF => self.gs.read_priv32(p),
            _ => {
                self.unknown(p, false, false);
                *self.regs.get(&p).unwrap_or(&0)
            }
        }
    }

    fn mch_drd_read(&mut self) -> u32 {
        // RDRAM initialisation: the BIOS counts the memory chips and reads
        // their configuration through these two registers.
        if (self.mch_ricm >> 6) & 0xF == 0 {
            match (self.mch_ricm >> 16) & 0xFFF {
                0x21 => {
                    if self.rdram_sdevid < 2 {
                        self.rdram_sdevid += 1;
                        return 0x1F;
                    }
                    return 0;
                }
                0x23 => return 0x0D0D,
                0x24 => return 0x0090,
                0x40 => return self.mch_ricm & 0x1F,
                _ => {}
            }
        }
        0
    }

    fn ee_io_write32(&mut self, p: u32, v: u32, mask: u32) {
        let merge = |old: u32| (old & !mask) | (v & mask);
        match p {
            0x1000_0000..=0x1000_1FFF => {
                let t = &mut self.timers[((p >> 11) & 3) as usize];
                match (p >> 4) & 0xF {
                    0 => t.count = merge(t.count) & 0xFFFF,
                    1 => {
                        // EQUF / OVFF are cleared by writing 1.
                        let flags = t.mode & 0xC00 & !(v & 0xC00);
                        t.mode = (merge(t.mode) & 0x3FF) | flags;
                    }
                    2 => t.comp = merge(t.comp) & 0xFFFF,
                    3 => t.hold = merge(t.hold) & 0xFFFF,
                    _ => {}
                }
            }
            0x1000_3000 => {
                if v & 1 != 0 {
                    self.gs.gif_reset();
                }
            }
            0x1000_E010 => {
                self.dmac.write(p, v & mask);
            }
            0x1000_8000..=0x1000_EFFF => {
                let old = self.dmac.read(p);
                self.dmac.write(p, merge(old));
                self.run_dma();
            }
            0x1000_F000 => self.intc_stat &= !(v & mask),
            0x1000_F010 => self.intc_mask ^= v & mask,
            0x1000_F200 => self.sif_mscom = merge(self.sif_mscom),
            0x1000_F220 => self.sif_msflg |= v & mask,
            0x1000_F230 => self.sif_smflg &= !(v & mask),
            0x1000_F240 => {
                if v & 0x100 == 0 {
                    self.sif_ctrl &= !0x100;
                } else {
                    self.sif_ctrl |= 0x100;
                }
            }
            0x1000_F260 => self.sif_bd6 = 0,
            0x1000_F430 => {
                let sa = (v >> 16) & 0xFFF;
                let sbc = (v >> 6) & 0xF;
                if sa == 0x21 && sbc == 1 && (self.mch_drd >> 7) & 1 == 0 {
                    self.rdram_sdevid = 0;
                }
                self.mch_ricm = v & !0x8000_0000;
            }
            0x1000_F440 => self.mch_drd = v,
            0x1000_F400..=0x1000_F4FF => {
                // Memory controller: commands finish at once (busy bit 31).
                let old = *self.regs.get(&p).unwrap_or(&0);
                self.regs.insert(p, merge(old) & !0x8000_0000);
            }
            0x1000_F590 => {
                self.dmac.enable = v;
                self.run_dma();
            }
            0x1100_0000..=0x1100_FFFF => {
                let m = match p & 0xC000 {
                    0x0000 => &mut self.vu0_code,
                    0x4000 => &mut self.vu0_data,
                    0x8000 => &mut self.vu1_code,
                    _ => &mut self.vu1_data,
                };
                let a = (p as usize) & (m.len() - 1) & !3;
                let old = rd32(m, a);
                wr32(m, a, (old & !mask) | (v & mask));
            }
            0x1200_0000..=0x1200_1FFF => self.gs.write_priv32(p, v, mask),
            _ => {
                if !(0x1000_2000..0x1000_4000).contains(&p) && !(0x1000_F000..0x1000_F600).contains(&p) {
                    self.unknown(p, true, false);
                }
                let old = *self.regs.get(&p).unwrap_or(&0);
                self.regs.insert(p, merge(old));
            }
        }
    }

    /// Bus clock is half the EE clock.
    pub fn tick_ee_timers(&mut self, bus_cycles: u32, hblank_cycles: u32) {
        for i in 0..4 {
            let t = &mut self.timers[i];
            if t.mode & 0x80 == 0 {
                continue;
            }
            let div = match t.mode & 3 {
                0 => 1,
                1 => 16,
                2 => 256,
                _ => hblank_cycles.max(1),
            };
            t.frac += bus_cycles;
            let inc = t.frac / div;
            t.frac %= div;
            if inc == 0 {
                continue;
            }
            let before = t.count;
            let mut c = before + inc;
            let mut irq = false;
            if t.comp > before && t.comp <= c || (t.comp == 0 && c >= 0x10000) {
                if t.mode & 0x100 != 0 && t.mode & 0x400 == 0 {
                    irq = true;
                }
                t.mode |= 0x400;
                if t.mode & 0x40 != 0 {
                    c = if t.comp > 0 { (c - t.comp) % (t.comp + 1) } else { 0 };
                }
            }
            if c > 0xFFFF {
                if t.mode & 0x200 != 0 && t.mode & 0x800 == 0 {
                    irq = true;
                }
                t.mode |= 0x800;
                c &= 0xFFFF;
            }
            t.count = c;
            if irq {
                self.intc_stat |= 1 << (int::TIM0 + i as u32);
            }
        }
    }

    // --------------------------------------------------------------- IOP --

    #[inline(always)]
    fn iop_phys(addr: u32) -> u32 {
        addr & 0x1FFF_FFFF
    }

    pub fn iop_fetch(&mut self, addr: u32) -> u32 {
        self.iop_read32(addr)
    }

    pub fn iop_read8(&mut self, addr: u32) -> u8 {
        let p = Self::iop_phys(addr);
        match p {
            0..=0x007F_FFFF => self.iop_ram[(p & 0x1F_FFFF) as usize],
            0x1FC0_0000..=0x1FFF_FFFF => self.bios_byte(p),
            0x1F40_2000..=0x1F40_20FF => self.cdvd.read8(p),
            0x1E00_0000..=0x1E3F_FFFF => *self.rom1.get((p - 0x1E00_0000) as usize).unwrap_or(&0),
            _ => (self.iop_io_read32(p & !3) >> ((p & 3) * 8)) as u8,
        }
    }

    pub fn iop_read16(&mut self, addr: u32) -> u16 {
        let p = Self::iop_phys(addr);
        match p {
            0..=0x007F_FFFF => {
                let a = (p & 0x1F_FFFE) as usize;
                u16::from_le_bytes([self.iop_ram[a], self.iop_ram[a + 1]])
            }
            0x1FC0_0000..=0x1FFF_FFFF => self.bios_byte(p) as u16 | (self.bios_byte(p + 1) as u16) << 8,
            0x1F90_0000..=0x1F90_07FF => self.spu2_read(p),
            _ => (self.iop_io_read32(p & !3) >> ((p & 2) * 8)) as u16,
        }
    }

    pub fn iop_read32(&mut self, addr: u32) -> u32 {
        if addr >= 0xFFFE_0000 {
            return if addr == 0xFFFE_0130 { self.iop_cache_ctrl } else { 0 };
        }
        let p = Self::iop_phys(addr);
        match p {
            0..=0x007F_FFFF => rd32(&self.iop_ram, (p & 0x1F_FFFC) as usize),
            0x1FC0_0000..=0x1FFF_FFFF => {
                let a = (p - 0x1FC0_0000) as usize;
                if a + 4 <= self.bios.len() { rd32(&self.bios, a) } else { 0 }
            }
            _ => self.iop_io_read32(p),
        }
    }

    fn iop_io_read32(&mut self, p: u32) -> u32 {
        match p {
            0x1D00_0000 => self.sif_mscom,
            0x1D00_0010 => self.sif_smcom,
            0x1D00_0020 => self.sif_msflg,
            0x1D00_0030 => self.sif_smflg,
            0x1D00_0040 => self.sif_ctrl | 0xF000_0002,
            0x1D00_0060 => self.sif_bd6,
            0x1F80_1070 => self.iop_stat,
            0x1F80_1074 => self.iop_mask,
            0x1F80_1078 => {
                // Reading I_CTRL returns it and disables interrupts.
                let v = self.iop_ctrl;
                self.iop_ctrl = 0;
                v
            }
            0x1F80_1080..=0x1F80_10FF | 0x1F80_1500..=0x1F80_157F => self.iop_dma.read(p),
            0x1F80_1100..=0x1F80_112F | 0x1F80_1480..=0x1F80_14AF => {
                let (i, r) = iop_timer_index(p);
                let t = &mut self.iop_timers[i];
                match r {
                    0 => t.count,
                    1 => {
                        let v = t.mode;
                        t.mode &= !0x1800; // reached flags clear on read
                        v
                    }
                    _ => t.target,
                }
            }
            0x1F40_2000..=0x1F40_20FF => self.cdvd.read8(p) as u32,
            0x1F90_0000..=0x1F90_07FF => self.spu2_read(p) as u32 | (self.spu2_read(p + 2) as u32) << 16,
            0x1F80_1450 => *self.regs.get(&p).unwrap_or(&0) & !8, // bit 3: 0 = PS2 mode
            0x1F80_8200..=0x1F80_82FF => self.sio2.read(p),
            _ => {
                if !(0x1F80_1000..0x1F80_1100).contains(&p) && !(0x1F80_1400..0x1F80_1600).contains(&p) {
                    self.unknown(p, false, true);
                }
                *self.regs.get(&(p | 0x4000_0000)).unwrap_or(&0)
            }
        }
    }

    fn spu2_read(&mut self, p: u32) -> u16 {
        match p & 0x7FF {
            // STATX (core 0 / core 1): not busy, DMA ready.
            0x344 | 0x744 => 0x80,
            _ => *self.regs.get(&(p & !1 | 0x4000_0000)).unwrap_or(&0) as u16,
        }
    }

    pub fn iop_write8(&mut self, addr: u32, v: u8) {
        let p = Self::iop_phys(addr);
        match p {
            0..=0x007F_FFFF => self.iop_ram[(p & 0x1F_FFFF) as usize] = v,
            0x1F40_2000..=0x1F40_20FF => {
                let irq = self.cdvd.write8(p, v);
                if irq {
                    self.iop_irq(iopint::CDVD);
                }
            }
            0x1F80_380C => {
                if self.iop_tty.len() < 1 << 16 {
                    self.iop_tty.push(v);
                }
            }
            _ => {
                let sh = (p & 3) * 8;
                self.iop_io_write32(p & !3, (v as u32) << sh, 0xFF << sh);
            }
        }
    }

    pub fn iop_write16(&mut self, addr: u32, v: u16) {
        let p = Self::iop_phys(addr);
        match p {
            0..=0x007F_FFFF => {
                let a = (p & 0x1F_FFFE) as usize;
                self.iop_ram[a..a + 2].copy_from_slice(&v.to_le_bytes());
            }
            0x1F90_0000..=0x1F90_07FF => {
                // SPU2: store, and drop the reset / busy bits right away.
                let v = if p & 0x7FF == 0x19A || p & 0x7FF == 0x59A { v & !0x8000 } else { v };
                self.regs.insert(p & !1 | 0x4000_0000, v as u32);
            }
            _ => {
                let sh = (p & 2) * 8;
                self.iop_io_write32(p & !3, (v as u32) << sh, 0xFFFF << sh);
            }
        }
    }

    pub fn iop_write32(&mut self, addr: u32, v: u32) {
        if addr >= 0xFFFE_0000 {
            if addr == 0xFFFE_0130 {
                self.iop_cache_ctrl = v;
            }
            return;
        }
        let p = Self::iop_phys(addr);
        match p {
            0..=0x007F_FFFF => wr32(&mut self.iop_ram, (p & 0x1F_FFFC) as usize, v),
            _ => self.iop_io_write32(p, v, 0xFFFF_FFFF),
        }
    }

    fn iop_io_write32(&mut self, p: u32, v: u32, mask: u32) {
        let merge = |old: u32| (old & !mask) | (v & mask);
        match p {
            0x1D00_0010 => self.sif_smcom = merge(self.sif_smcom),
            0x1D00_0020 => self.sif_msflg &= !(v & mask),
            0x1D00_0030 => self.sif_smflg |= v & mask,
            0x1D00_0040 => {
                let v = v & mask;
                let temp = v & 0xF0;
                if v & 0x20 != 0 || v & 0x80 != 0 {
                    self.sif_ctrl &= !0xF000;
                    self.sif_ctrl |= 0x2000;
                }
                if self.sif_ctrl & temp != 0 {
                    self.sif_ctrl &= !temp;
                } else {
                    self.sif_ctrl |= temp;
                }
            }
            0x1D00_0060 => self.sif_bd6 = merge(self.sif_bd6),
            0x1F80_1070 => self.iop_stat &= v | !mask,
            0x1F80_1074 => self.iop_mask = merge(self.iop_mask),
            0x1F80_1078 => self.iop_ctrl = merge(self.iop_ctrl) & 1,
            0x1F80_1080..=0x1F80_10FF | 0x1F80_1500..=0x1F80_157F => {
                let old = self.iop_dma.read(p);
                self.iop_dma.write(p, merge(old));
                self.run_iop_dma();
            }
            0x1F80_1100..=0x1F80_112F | 0x1F80_1480..=0x1F80_14AF => {
                let (i, r) = iop_timer_index(p);
                let t = &mut self.iop_timers[i];
                match r {
                    0 => t.count = merge(t.count),
                    1 => {
                        t.mode = (merge(t.mode) & 0x3FF) | 0x400;
                        t.count = 0;
                    }
                    _ => t.target = merge(t.target),
                }
            }
            0x1F80_8200..=0x1F80_82FF => {
                // SIO2 FIFO bytes come as byte writes; pass the byte itself.
                let val = if mask == 0xFFFF_FFFF { v } else { (v & mask) >> mask.trailing_zeros() };
                self.sio2.write(p, val);
                if std::mem::take(&mut self.sio2.irq) {
                    self.iop_irq(iopint::SIO2);
                }
                self.run_iop_dma();
            }
            0x1F80_2070 => {
                // POST code: the IOP boot reports its progress here.
                if self.trace.len() < 400 {
                    self.trace.push(format!("IOP POST {:02X}", v & mask & 0xFF));
                }
            }
            0x1F80_1450 => {
                let old = *self.regs.get(&p).unwrap_or(&0);
                self.regs.insert(p, merge(old));
            }
            _ => {
                if !(0x1F80_1000..0x1F80_1100).contains(&p) && !(0x1F80_1400..0x1F80_1600).contains(&p) {
                    self.unknown(p, true, true);
                }
                let k = p | 0x4000_0000;
                let old = *self.regs.get(&k).unwrap_or(&0);
                self.regs.insert(k, merge(old));
            }
        }
    }

    /// IOP timers 0-2 are 16-bit, 3-5 32-bit. Clock source is the IOP
    /// clock (we ignore the prescalers except /8 /16 /256 on 2 and 3-5).
    /// `line` is IOP cycles per scanline (for the hblank clock source).
    pub fn tick_iop_timers(&mut self, cycles: u32, line: u32) {
        for i in 0..6 {
            let t = &mut self.iop_timers[i];
            let div = match i {
                // Pixel clock (~13.5 MHz) for timer 0, hblank for 1 and 3.
                0 if t.mode & 0x100 != 0 => 3,
                1 | 3 if t.mode & 0x100 != 0 => line.max(1),
                2 if t.mode & 0x200 != 0 => 8,
                3..=5 => match (t.mode >> 13) & 3 {
                    1 => 8,
                    2 => 16,
                    3 => 256,
                    _ => 1,
                },
                _ => 1,
            };
            t.frac += cycles;
            let inc = t.frac / div;
            t.frac %= div;
            if inc == 0 {
                continue;
            }
            let wide = i >= 3;
            let max: u64 = if wide { 0xFFFF_FFFF } else { 0xFFFF };
            let before = t.count as u64;
            let mut c = before + inc as u64;
            let mut irq = false;
            let target = t.target as u64 & max;
            if before < target && c >= target {
                t.mode |= 0x800;
                if t.mode & 0x10 != 0 {
                    irq = true;
                }
                if t.mode & 0x8 != 0 {
                    c -= target;
                }
            }
            if c > max {
                t.mode |= 0x1000;
                if t.mode & 0x20 != 0 {
                    irq = true;
                }
                c &= max;
            }
            t.count = c as u32;
            if irq && t.mode & 0x400 != 0 {
                if t.mode & 0x40 == 0 {
                    t.mode &= !0x400; // one-shot
                }
                let n = if i < 3 { iopint::TIM0 + i as u32 } else { iopint::TIM3 + i as u32 - 3 };
                self.iop_stat |= 1 << n;
                self.iop_irq_counts[n as usize] += 1;
            }
        }
    }

    pub fn run_dma(&mut self) {
        crate::dmac::run(self);
    }

    pub fn run_iop_dma(&mut self) {
        crate::iopdma::run(self);
    }
}

fn iop_timer_index(p: u32) -> (usize, u32) {
    if p < 0x1F80_1480 {
        (((p >> 4) & 3) as usize, (p >> 2) & 3)
    } else {
        (3 + ((p - 0x1F80_1480) >> 4) as usize, (p >> 2) & 3)
    }
}
