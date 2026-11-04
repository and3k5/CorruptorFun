//! A PlayStation 2 emulator scoped to the BIOS boot, for the CorruptorFun
//! BIOS corruptor. Built to WebAssembly.
//!
//! Two CPUs share the BIOS: the EE (R5900, `ee`) and the IOP (R3000A,
//! `iop`). They talk through the SIF (`dmac::sif`). Graphics go through the
//! GIF to the GS (`gs`, `gsdraw`), fed by DMA directly or via VIF1.

pub mod cdvd;
pub mod dmac;
pub mod ee;
pub mod gs;
pub mod gsdraw;
pub mod gsmem;
pub mod hw;
pub mod iop;
pub mod iopdma;
pub mod romdir;
pub mod sio2;
pub mod vif;
pub mod vu;

use ee::Ee;
use hw::Hw;
use iop::Iop;

/// EE clock 294.912 MHz; the IOP runs at 1/8 of it.
pub const EE_HZ: u32 = 294_912_000;
const BATCH: u32 = 256;
const LOG_LINES: usize = 2000;

pub struct System {
    pub ee: Ee,
    pub iop: Iop,
    pub hw: Hw,
    pub frame: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub render: bool,
    pub log: Vec<String>,
    pub frame_no: u32,
    pal: bool,
    line_cycles: u32,
    iop_acc: u32,
    pub files: Vec<romdir::RomFile>,
    pub pc_hist_ee: std::collections::BTreeMap<u32, u32>,
}

impl System {
    pub fn new(bios: Vec<u8>) -> Self {
        let files = romdir::parse(&bios);
        let pal = romdir::find(&files, "ROMVER")
            .and_then(|f| bios.get(f.offset as usize + 4))
            .map(|&c| c == b'E')
            .unwrap_or(false);
        let region = romdir::find(&files, "ROMVER").and_then(|f| bios.get(f.offset as usize + 4)).copied().unwrap_or(b'E');
        let mut s = System {
            ee: Ee::new(),
            iop: Iop::new(),
            hw: Hw::new(bios),
            frame: Vec::new(),
            width: 640,
            height: 448,
            render: true,
            log: Vec::new(),
            frame_no: 0,
            pal,
            line_cycles: 0,
            iop_acc: 0,
            files,
            pc_hist_ee: Default::default(),
        };
        s.hw.cdvd.region = region;
        s.line_cycles = EE_HZ / if pal { 50 * 625 } else { 60 * 525 } * 2;
        s.log.push(format!(
            "ROM: {} files, {}",
            s.files.len(),
            if pal { "PAL" } else { "NTSC" }
        ));
        s
    }

    pub fn is_pal(&self) -> bool {
        self.pal
    }

    fn frame_cycles(&self) -> u32 {
        if self.pal { EE_HZ / 50 } else { (EE_HZ as u64 * 1001 / 60_000) as u32 }
    }

    pub fn run_frame(&mut self) {
        let total = self.frame_cycles();
        // Vertical blank starts this far into the field.
        let vblank_at = total / 100 * 92;
        let mut done = 0u32;
        let mut in_vblank = false;
        let mut hacc = 0u32;
        while done < total {
            if !in_vblank && done >= vblank_at {
                in_vblank = true;
                self.vblank_start();
            }
            let mut c = 0;
            while c < BATCH {
                c += self.ee.step(&mut self.hw);
            }
            self.ee.add_count(c);
            self.hw.tick_ee_timers(c / 2, self.line_cycles / 2);
            // IOP: one cycle per 8 EE cycles.
            self.iop_acc += c;
            while self.iop_acc >= 16 {
                self.iop_acc -= self.iop.step(&mut self.hw) * 8;
            }
            self.hw.tick_iop_timers(c / 8, self.line_cycles / 8);
            hacc += c;
            if hacc >= self.line_cycles {
                hacc -= self.line_cycles;
                if self.hw.gs.hsync() {
                    self.hw.ee_irq(hw::int::GS);
                }
            }
            if self.hw.gs.take_irq() {
                self.hw.ee_irq(hw::int::GS);
            }
            let (i0, i1) = self.hw.ee_int_lines();
            self.ee.set_int_lines(i0, i1);
            done += c;
        }
        // End of vblank: next field begins.
        self.hw.ee_irq(hw::int::VBOF);
        self.hw.iop_irq(hw::iopint::EVBLANK);
        *self.pc_hist_ee.entry(self.ee.pc & !0xFF).or_insert(0) += 1;
        self.frame_no += 1;
        if self.frame_no % if self.pal { 50 } else { 60 } == 0 {
            self.hw.cdvd.clock_secs += 1;
        }
        if self.render {
            self.render_frame();
        }
        self.collect_trace();
    }

    fn vblank_start(&mut self) {
        self.hw.ee_irq(hw::int::VBON);
        self.hw.iop_irq(hw::iopint::VBLANK);
        if self.hw.gs.vsync() {
            self.hw.ee_irq(hw::int::GS);
        }
    }

    pub fn render_frame(&mut self) {
        let (w, h) = self.hw.gs.render(&mut self.frame);
        self.width = w;
        self.height = h;
    }

    fn collect_trace(&mut self) {
        let f = self.frame_no;
        let mut lines: Vec<String> = Vec::new();
        lines.append(&mut self.ee.trace);
        lines.append(&mut self.iop.trace);
        lines.append(&mut self.hw.trace);
        lines.append(&mut self.hw.cdvd.trace);
        if f % 30 == 0 {
            lines.push(format!(
                "status EE pc {:08X} sr {:08X} INTC {:04X}/{:04X} | IOP pc {:08X} I_STAT {:06X}/{:06X} | SIF ms {:08X} sm {:08X} | GS prims {}",
                self.ee.pc,
                self.ee.status,
                self.hw.intc_stat,
                self.hw.intc_mask,
                self.iop.pc,
                self.hw.iop_stat,
                self.hw.iop_mask,
                self.hw.sif_msflg,
                self.hw.sif_smflg,
                self.hw.gs.draw.prims,
            ));
        }
        for l in lines {
            if self.log.len() < LOG_LINES {
                self.log.push(format!("{f:5} {l}"));
            }
        }
    }

    pub fn debug_report(&self) -> String {
        use std::fmt::Write;
        let mut r = String::new();
        let _ = writeln!(r, "CorruptorFun PS2 debug report");
        let _ = writeln!(r, "frames {} EE pc {:08X} IOP pc {:08X} ExecPS2 {:08X?}", self.frame_no, self.ee.pc, self.iop.pc, self.ee.exec_entry);
        let _ = write!(r, "EE recent jumps:");
        for k in 0..64 {
            let _ = write!(r, " {:08X}", self.ee.hist[(self.ee.hist_i + k) & 63]);
        }
        let _ = writeln!(r, "\nEE ra {:08X} sp {:08X} v0 {:08X} a0 {:08X}", self.ee.r[31][0], self.ee.r[29][0], self.ee.r[2][0], self.ee.r[4][0]);
        let _ = writeln!(r, "\n== timeline (frame, event) ==");
        for l in &self.log {
            let _ = writeln!(r, "{l}");
        }
        let _ = writeln!(r, "\n== EE kernel output ==");
        r.push_str(&String::from_utf8_lossy(&self.hw.ee_tty));
        let _ = writeln!(r, "\n== IOP output ==");
        r.push_str(&String::from_utf8_lossy(&self.hw.iop_tty));
        let _ = writeln!(r, "\n== EE syscalls (number count) ==");
        for (n, c) in &self.ee.syscalls {
            let _ = writeln!(r, "{n:3} ({n:#04X}) {c}");
        }
        let _ = writeln!(r, "\n== not emulated yet (count) ==");
        for (n, c) in &self.ee.unimpl {
            let _ = writeln!(r, "{n} {c}");
        }
        let _ = writeln!(r, "\n== DMA ==");
        for i in 0..10 {
            let _ = writeln!(r, "EE ch{i}: {} transfers, {} bytes", self.hw.dmac.transfers[i], self.hw.dmac.bytes[i]);
        }
        for i in 0..14 {
            if self.hw.iop_dma.transfers[i] > 0 {
                let _ = writeln!(r, "IOP ch{i}: {} transfers", self.hw.iop_dma.transfers[i]);
            }
        }
        let v = &self.hw.vif1;
        let _ = writeln!(
            r,
            "VIF1: {} MPG, {} UNPACK, {} DIRECT qwords, {} MSCAL; VU1 runs {}; GIF qwords path1-3 {:?}",
            v.mpg_count, v.unpack_count, v.direct_qwords, v.mscal_count, self.hw.vu1.runs, &self.hw.gs.gif_qwords[1..]
        );
        let _ = writeln!(r, "IOP interrupts: {:?}", self.hw.iop_irq_counts.iter().enumerate().filter(|(_, c)| **c > 0).collect::<Vec<_>>());
        let _ = writeln!(r, "IOP timers: {:?}", self.hw.iop_timers.iter().map(|t| (t.count, t.mode, t.target)).collect::<Vec<_>>());
        let _ = writeln!(r, "\n== SIF packets ==");
        for l in &self.hw.iop_dma.sif_log {
            let _ = writeln!(r, "{l}");
        }
        let _ = writeln!(r, "\n== GS register writes ==");
        for (i, c) in self.hw.gs.reg_counts.iter().enumerate() {
            if *c > 0 {
                let _ = write!(r, "{i:02X}:{c} ");
            }
        }
        let _ = writeln!(r, "\nPMODE {:X} DISPFB1 {:X} DISPLAY1 {:X} DISPFB2 {:X} DISPLAY2 {:X}", self.hw.gs.pmode, self.hw.gs.dispfb[0], self.hw.gs.display[0], self.hw.gs.dispfb[1], self.hw.gs.display[1]);
        let _ = writeln!(r, "\n== CDVD S-commands ==");
        for (c, n) in &self.hw.cdvd.s_counts {
            let _ = write!(r, "{c:02X}:{n} ");
        }
        let _ = write!(r, "\nN-commands: ");
        for (c, n) in &self.hw.cdvd.n_counts {
            let _ = write!(r, "{c:02X}:{n} ");
        }
        let _ = writeln!(r, "\n\n== I/O with no emulated device (cpu, rw, address, count) ==");
        for ((a, w, iop), c) in &self.hw.unknown_io {
            let _ = writeln!(r, "{} {} {:08X} {}", if *iop { "IOP" } else { "EE " }, if *w { "write" } else { "read " }, a, c);
        }
        r
    }
}

// ---- WebAssembly interface ----

static mut SYSTEM: Option<System> = None;

#[allow(static_mut_refs)]
fn sys() -> &'static mut System {
    unsafe { SYSTEM.as_mut().expect("ps2_init not called") }
}

#[no_mangle]
pub extern "C" fn alloc(size: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(size);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` must come from `alloc(size)`.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, size: usize) {
    drop(Vec::from_raw_parts(ptr, 0, size));
}

/// Takes ownership of a buffer from `alloc(len)` holding the BIOS.
///
/// # Safety
/// `ptr` must come from `alloc(len)` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn ps2_init(ptr: *mut u8, len: usize) {
    let bios = Vec::from_raw_parts(ptr, len, len);
    SYSTEM = Some(System::new(bios));
}

#[no_mangle]
pub extern "C" fn ps2_run_frame() {
    sys().run_frame();
}

#[no_mangle]
pub extern "C" fn ps2_set_render(on: u32) {
    sys().render = on != 0;
}

#[no_mangle]
pub extern "C" fn ps2_render() {
    sys().render_frame();
}

static mut REPORT: Vec<u8> = Vec::new();

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ps2_report() {
    unsafe {
        REPORT = sys().debug_report().into_bytes();
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ps2_report_ptr() -> *const u8 {
    unsafe { REPORT.as_ptr() }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ps2_report_len() -> usize {
    unsafe { REPORT.len() }
}

#[no_mangle]
pub extern "C" fn ps2_frame_ptr() -> *const u8 {
    sys().frame.as_ptr()
}

#[no_mangle]
pub extern "C" fn ps2_frame_width() -> u32 {
    sys().width
}

#[no_mangle]
pub extern "C" fn ps2_frame_height() -> u32 {
    sys().height
}

#[no_mangle]
pub extern "C" fn ps2_is_pal() -> u32 {
    sys().is_pal() as u32
}

#[no_mangle]
pub extern "C" fn ps2_ee_pc() -> u32 {
    sys().ee.pc
}
