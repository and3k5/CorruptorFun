//! Graphics Synthesizer: privileged registers, the GIF (which unpacks
//! GIFtag packets into GS register writes) and the drawing side in
//! `gsdraw`.

use crate::gsdraw::Draw;

pub struct Gs {
    pub pmode: u64,
    pub smode2: u64,
    pub dispfb: [u64; 2],
    pub display: [u64; 2],
    pub bgcolor: u64,
    pub csr: u64,
    pub imr: u64,
    pub siglblid: u64,
    pub priv_other: std::collections::BTreeMap<u32, u64>,
    /// GIF packet state.
    gif: GifState,
    pub draw: Draw,
    /// GS register writes seen, by register number, for the report.
    pub reg_counts: [u32; 0x64],
    pub gif_qwords: [u64; 4],
}

#[derive(Default, Clone, Copy)]
struct GifState {
    nloop: u32,
    nreg: u32,
    regs: u64,
    flg: u32,
    reg_index: u32,
    active: bool,
    eop: bool,
    /// REGLIST packs two registers per qword.
    path: u32,
}

impl Gs {
    pub fn new() -> Self {
        Gs {
            pmode: 0,
            smode2: 0,
            dispfb: [0; 2],
            display: [0; 2],
            bgcolor: 0,
            csr: 0,
            imr: 0xFF00,
            siglblid: 0,
            priv_other: Default::default(),
            gif: GifState::default(),
            draw: Draw::new(),
            reg_counts: [0; 0x64],
            gif_qwords: [0; 4],
        }
    }

    pub fn read_priv32(&mut self, a: u32) -> u32 {
        let v = self.read_priv64(a & !7);
        if a & 4 != 0 { (v >> 32) as u32 } else { v as u32 }
    }

    pub fn read_priv64(&mut self, a: u32) -> u64 {
        match a & 0x13F0 {
            0x0000 => self.pmode,
            0x0020 => self.smode2,
            0x0070 => self.dispfb[0],
            0x0080 => self.display[0],
            0x0090 => self.dispfb[1],
            0x00A0 => self.display[1],
            0x00E0 => self.bgcolor,
            // CSR: revision 0x1B, id 0x55; FIFO empty.
            0x1000 => (self.csr & 0xFFFF_FFFF_0000_E3FF) | (0x55 << 24) | (0x1B << 16) | (1 << 14),
            0x1010 => self.imr,
            0x1080 => self.siglblid,
            o => *self.priv_other.get(&o).unwrap_or(&0),
        }
    }

    pub fn write_priv32(&mut self, a: u32, v: u32, mask: u32) {
        let base = a & !7;
        let old = self.read_priv64(base);
        let (v64, m64) = if a & 4 != 0 { ((v as u64) << 32, (mask as u64) << 32) } else { (v as u64, mask as u64) };
        if base & 0x13F0 == 0x1000 {
            self.write_csr(v64 & m64);
            return;
        }
        self.write_priv64(base, (old & !m64) | (v64 & m64));
    }

    fn write_csr(&mut self, v: u64) {
        // SIGNAL, FINISH, HSINT, VSINT, EDWINT: write 1 to clear.
        self.csr &= !(v & 0x1F);
        if v & 0x200 != 0 {
            self.csr = 0;
            self.gif_reset();
        }
    }

    pub fn write_priv64(&mut self, a: u32, v: u64) {
        match a & 0x13F0 {
            0x0000 => self.pmode = v,
            0x0020 => self.smode2 = v,
            0x0070 => self.dispfb[0] = v,
            0x0080 => self.display[0] = v,
            0x0090 => self.dispfb[1] = v,
            0x00A0 => self.display[1] = v,
            0x00E0 => self.bgcolor = v,
            0x1000 => self.write_csr(v),
            0x1010 => self.imr = v,
            0x1080 => self.siglblid = v,
            o => {
                self.priv_other.insert(o, v);
            }
        }
    }

    /// VSINT happened. Returns whether the GS interrupt line should fire.
    pub fn vsync(&mut self) -> bool {
        self.csr |= 1 << 3;
        self.csr ^= 1 << 13; // FIELD
        self.imr & (1 << 11) == 0
    }

    pub fn hsync(&mut self) -> bool {
        self.csr |= 1 << 2;
        self.imr & (1 << 10) == 0
    }

    /// FINISH / SIGNAL events raised by drawing, to forward to the INTC.
    pub fn take_irq(&mut self) -> bool {
        let fired = std::mem::take(&mut self.draw.events);
        let mut irq = false;
        if fired & 1 != 0 {
            self.csr |= 1;
            irq |= self.imr & (1 << 8) == 0;
        }
        if fired & 2 != 0 {
            self.csr |= 2;
            irq |= self.imr & (1 << 9) == 0;
        }
        if let Some(l) = self.draw.signal_id.take() {
            self.siglblid = (self.siglblid & !0xFFFF_FFFF) | l as u64;
        }
        if let Some(l) = self.draw.label_id.take() {
            self.siglblid = (self.siglblid & 0xFFFF_FFFF) | (l as u64) << 32;
        }
        irq
    }

    pub fn gif_stat(&self) -> u32 {
        if self.gif.active {
            (self.gif.path & 3) << 10
        } else {
            0
        }
    }

    pub fn gif_reset(&mut self) {
        self.gif = GifState::default();
    }

    pub fn gif_fifo_write(&mut self, q: u128) {
        self.gif_path_write(3, q);
    }

    /// One quadword of GIF data on `path` (1 = VU1 XGKICK, 2 = VIF1, 3 = DMA).
    pub fn gif_path_write(&mut self, path: u32, q: u128) {
        self.gif_qwords[path as usize & 3] += 1;
        let g = &mut self.gif;
        if !g.active {
            let tag = q as u64;
            g.nloop = (tag & 0x7FFF) as u32;
            g.eop = tag >> 15 & 1 != 0;
            let pre = tag >> 46 & 1 != 0;
            let prim = (tag >> 47) & 0x7FF;
            g.flg = ((tag >> 58) & 3) as u32;
            g.nreg = ((tag >> 60) & 0xF) as u32;
            if g.nreg == 0 {
                g.nreg = 16;
            }
            g.regs = (q >> 64) as u64;
            g.reg_index = 0;
            g.path = path;
            g.active = g.nloop > 0;
            if pre && g.flg == 0 {
                self.write_reg(0x00, prim);
            }
            return;
        }
        match g.flg {
            0 => {
                // PACKED: one register per quadword.
                let r = ((g.regs >> (g.reg_index * 4)) & 0xF) as u32;
                self.packed(r, q);
                self.advance(1);
            }
            1 => {
                // REGLIST: two 64-bit register values per quadword.
                for half in 0..2 {
                    if !self.gif.active {
                        break;
                    }
                    let r = ((self.gif.regs >> (self.gif.reg_index * 4)) & 0xF) as u32;
                    let v = (q >> (64 * half)) as u64;
                    if r != 0xE {
                        self.write_reg(r, v);
                    }
                    self.advance(1);
                }
            }
            _ => {
                // IMAGE: NLOOP quadwords of HWREG data.
                self.draw.hwreg(q as u64);
                self.draw.hwreg((q >> 64) as u64);
                self.gif.nloop -= 1;
                if self.gif.nloop == 0 {
                    self.gif.active = false;
                }
            }
        }
    }

    fn advance(&mut self, _n: u32) {
        let g = &mut self.gif;
        g.reg_index += 1;
        if g.reg_index >= g.nreg {
            g.reg_index = 0;
            g.nloop -= 1;
            if g.nloop == 0 {
                g.active = false;
            }
        }
    }

    fn packed(&mut self, r: u32, q: u128) {
        let lo = q as u64;
        let hi = (q >> 64) as u64;
        let w = |i: u32| ((q >> (i * 32)) as u32) as u64;
        match r {
            0x1 => {
                // RGBAQ: Q comes from the last ST.
                let v = (w(0) & 0xFF) | (w(1) & 0xFF) << 8 | (w(2) & 0xFF) << 16 | (w(3) & 0xFF) << 24 | (self.draw.q_bits as u64) << 32;
                self.write_reg(0x01, v);
            }
            0x2 => {
                self.draw.q_bits = w(2) as u32;
                self.write_reg(0x02, lo);
            }
            0x3 => self.write_reg(0x03, (w(0) & 0x3FFF) | (w(1) & 0x3FFF) << 16),
            0x4 => {
                let v = (w(0) & 0xFFFF) | (w(1) & 0xFFFF) << 16 | ((w(2) >> 4) & 0xFF_FFFF) << 32 | ((w(3) >> 4) & 0xFF) << 56;
                let disable = hi >> 47 & 1 != 0;
                self.write_reg(if disable { 0x0C } else { 0x04 }, v);
            }
            0x5 => {
                let v = (w(0) & 0xFFFF) | (w(1) & 0xFFFF) << 16 | w(2) << 32;
                let disable = hi >> 47 & 1 != 0;
                self.write_reg(if disable { 0x0D } else { 0x05 }, v);
            }
            0xA => self.write_reg(0x0A, ((w(3) >> 4) & 0xFF) << 56),
            0xE => {
                // A+D: address in the upper half.
                self.write_reg((hi & 0xFF) as u32, lo);
            }
            0xF => {}
            _ => self.write_reg(r, lo),
        }
    }

    pub fn write_reg(&mut self, r: u32, v: u64) {
        if (r as usize) < self.reg_counts.len() {
            self.reg_counts[r as usize] += 1;
        }
        self.draw.write_reg(r, v);
    }

    /// The displayed picture (read circuit 2 if enabled, else 1) as RGBA.
    pub fn render(&self, out: &mut Vec<u8>) -> (u32, u32) {
        self.draw.display(self.pmode, self.smode2, &self.dispfb, &self.display, self.bgcolor, out)
    }
}
