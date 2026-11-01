//! A PlayStation emulator that is just complete enough to boot the BIOS:
//! intro, shell, and the licensed-disc screen. Built to WebAssembly for the
//! CorruptorFun BIOS corruptor.

pub mod bus;
pub mod cdrom;
pub mod cpu;
pub mod gpu;
pub mod gte;
pub mod sio;
pub mod spu;
pub mod timers;

use bus::Bus;
use cpu::Cpu;

/// CPU cycles between peripheral updates.
const TICK_BATCH: u32 = 64;

pub struct System {
    pub cpu: Cpu,
    pub bus: Bus,
    pub frame: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// When false, `run_frame` skips building the RGBA frame (headless runs).
    pub render: bool,
    /// Timeline for the debug report: notable events, one line each.
    pub log: Vec<String>,
    frame_no: u32,
}

const LOG_LINES: usize = 1500;

impl System {
    pub fn new(bios: Vec<u8>) -> Self {
        System { cpu: Cpu::new(), bus: Bus::new(bios), frame: Vec::new(), width: 320, height: 240, render: true, log: Vec::new(), frame_no: 0 }
    }

    /// Runs until the GPU finishes a frame (or a safety cap of cycles).
    pub fn run_frame(&mut self) {
        let start = self.bus.gpu.frames;
        let mut budget: u64 = 0;
        let mut acc = 0;
        while self.bus.gpu.frames == start && budget < 2_000_000 && !self.cpu.game_started {
            self.bus.access_cycles = 0;
            let c = self.cpu.step(&mut self.bus);
            if self.bus.cdrom.exe_entry == Some(self.cpu.pc) {
                self.cpu.game_started = true;
            }
            acc += c;
            budget += c as u64;
            if acc >= TICK_BATCH {
                self.bus.tick(acc);
                acc = 0;
            }
        }
        if acc > 0 {
            self.bus.tick(acc);
        }
        if self.render {
            self.render_frame();
        }
        self.frame_no += 1;
        self.collect_trace();
    }

    fn collect_trace(&mut self) {
        let f = self.frame_no;
        let mut lines: Vec<String> = Vec::new();
        lines.append(&mut self.cpu.trace);
        lines.append(&mut self.bus.cdrom.trace);
        if f % 60 == 0 {
            let b = &self.bus;
            lines.push(format!(
                "status pc {:08X} sr {:08X} I_STAT {:03X} I_MASK {:03X} display {} {}x{} voices {}",
                self.cpu.pc,
                self.cpu.sr,
                b.istat,
                b.imask,
                if b.gpu.display_disabled { "off" } else { "on" },
                self.width,
                self.height,
                b.spu.active_voices()
            ));
        }
        for l in lines {
            if self.log.len() < LOG_LINES {
                self.log.push(format!("{f:5} {l}"));
            }
        }
    }

    /// Plain-text report of what the BIOS did, for bug reports.
    pub fn debug_report(&self) -> String {
        use std::fmt::Write;
        let mut r = String::new();
        let _ = writeln!(r, "CorruptorFun PS1 debug report");
        let _ = writeln!(r, "frames {} pc {:08X} game started {}", self.frame_no, self.cpu.pc, self.cpu.game_started);
        let _ = writeln!(r, "\n== timeline (frame, event) ==");
        for l in &self.log {
            let _ = writeln!(r, "{l}");
        }
        if self.log.len() >= LOG_LINES {
            let _ = writeln!(r, "(timeline full)");
        }
        let _ = writeln!(r, "\n== BIOS calls (table:function count) ==");
        for ((t, n), c) in &self.cpu.bios_calls {
            let _ = writeln!(r, "{:X}:{:02X} {}", t >> 4, n, c);
        }
        let _ = writeln!(r, "\n== I/O with no emulated device ==");
        for (a, w) in &self.bus.unknown_io {
            let _ = writeln!(r, "{} {:08X}", if *w { "write" } else { "read" }, a);
        }
        let _ = writeln!(r, "\n== BIOS text output ==");
        r.push_str(&String::from_utf8_lossy(&self.cpu.tty));
        r
    }

    pub fn render_frame(&mut self) {
        let (w, h) = self.bus.gpu.render(&mut self.frame);
        self.width = w;
        self.height = h;
    }

    /// Side-loads a PS-X EXE and jumps to it (used by the test harness).
    pub fn load_exe(&mut self, exe: &[u8]) -> bool {
        if exe.len() < 0x800 || &exe[0..8] != b"PS-X EXE" {
            return false;
        }
        let rd = |o: usize| u32::from_le_bytes([exe[o], exe[o + 1], exe[o + 2], exe[o + 3]]);
        let pc = rd(0x10);
        let gp = rd(0x14);
        let dest = rd(0x18);
        let size = rd(0x1C) as usize;
        let sp = rd(0x30).wrapping_add(rd(0x34));
        for i in 0..size.min(exe.len() - 0x800) {
            let a = (dest.wrapping_add(i as u32) & 0x1F_FFFF) as usize;
            self.bus.ram[a] = exe[0x800 + i];
        }
        self.cpu.pc = pc;
        self.cpu.next_pc = pc.wrapping_add(4);
        self.cpu.regs[28] = gp;
        if sp != 0 {
            self.cpu.regs[29] = sp;
            self.cpu.regs[30] = sp;
        } else {
            self.cpu.regs[29] = 0x801F_FFF0;
        }
        true
    }
}

// ---- WebAssembly interface ----

static mut SYSTEM: Option<System> = None;

#[allow(static_mut_refs)]
fn sys() -> &'static mut System {
    unsafe { SYSTEM.as_mut().expect("ps1_init not called") }
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
pub unsafe extern "C" fn ps1_init(ptr: *mut u8, len: usize) {
    let bios = Vec::from_raw_parts(ptr, len, len);
    SYSTEM = Some(System::new(bios));
}

/// Takes ownership of a buffer from `alloc(len)` holding a raw
/// 2352-byte-per-sector data track. Ownership avoids a second copy of a
/// 700 MB image.
///
/// # Safety
/// `ptr` must come from `alloc(len)` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn ps1_set_disc(ptr: *mut u8, len: usize) {
    let disc = if len == 0 { None } else { Some(Vec::from_raw_parts(ptr, len, len)) };
    sys().bus.cdrom.set_disc(disc);
}

#[no_mangle]
pub extern "C" fn ps1_set_region(r: u8) {
    sys().bus.cdrom.region = r;
}

#[no_mangle]
pub extern "C" fn ps1_run_frame() {
    sys().run_frame();
}

/// 0 skips building frames in `ps1_run_frame` (used when searching for
/// corruptions); `ps1_render` then builds one on demand.
#[no_mangle]
pub extern "C" fn ps1_set_render(on: u32) {
    sys().render = on != 0;
}

#[no_mangle]
pub extern "C" fn ps1_render() {
    sys().render_frame();
}

/// Side-loads a PS-X EXE from a buffer from `alloc(len)`. Returns 1 on success.
///
/// # Safety
/// `ptr` must come from `alloc(len)` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn ps1_load_exe(ptr: *mut u8, len: usize) -> u32 {
    let exe = Vec::from_raw_parts(ptr, len, len);
    sys().load_exe(&exe) as u32
}

static mut REPORT: Vec<u8> = Vec::new();

/// Builds the debug report; read it with `ps1_report_ptr` and
/// `ps1_report_len`.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ps1_report() {
    unsafe {
        REPORT = sys().debug_report().into_bytes();
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ps1_report_ptr() -> *const u8 {
    unsafe { REPORT.as_ptr() }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ps1_report_len() -> usize {
    unsafe { REPORT.len() }
}

#[no_mangle]
pub extern "C" fn ps1_frame_ptr() -> *const u8 {
    sys().frame.as_ptr()
}

#[no_mangle]
pub extern "C" fn ps1_frame_width() -> u32 {
    sys().width
}

#[no_mangle]
pub extern "C" fn ps1_frame_height() -> u32 {
    sys().height
}

#[no_mangle]
pub extern "C" fn ps1_set_buttons(b: u32) {
    sys().bus.sio.buttons = b as u16;
}

#[no_mangle]
pub extern "C" fn ps1_audio_ptr() -> *const i16 {
    sys().bus.spu.out.as_ptr()
}

/// Number of i16 values (stereo interleaved) waiting.
#[no_mangle]
pub extern "C" fn ps1_audio_len() -> usize {
    sys().bus.spu.out.len()
}

#[no_mangle]
pub extern "C" fn ps1_audio_clear() {
    sys().bus.spu.out.clear();
}

#[no_mangle]
pub extern "C" fn ps1_pc() -> u32 {
    sys().cpu.pc
}

/// 1 once the BIOS has started the game on the disc.
#[no_mangle]
pub extern "C" fn ps1_game_started() -> u32 {
    sys().cpu.game_started as u32
}

#[no_mangle]
pub extern "C" fn ps1_is_pal() -> u32 {
    sys().bus.gpu.is_pal() as u32
}
