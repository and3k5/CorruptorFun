//! Headless runner for testing: boots a BIOS (or side-loads an EXE), runs N
//! frames and writes the last frame as a PPM.
//!
//! ps1-cli <bios> [--exe file] [--disc file.bin] [--frames N] [--out frame.ppm] [--report]

use ps1::System;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bios = std::fs::read(&args[1]).expect("bios");
    let mut sys = System::new(bios);
    let mut frames = 300;
    let mut out = String::from("frame.ppm");
    let mut exe = None;
    let mut shots = Vec::new();
    let mut i = 2;
    let mut report = false;
    let mut trace = 0u32;
    while i < args.len() {
        match args[i].as_str() {
            "--exe" => { exe = Some(std::fs::read(&args[i + 1]).expect("exe")); i += 1 }
            "--disc" => { sys.bus.cdrom.set_disc(Some(std::fs::read(&args[i + 1]).expect("disc"))); i += 1 }
            "--frames" => { frames = args[i + 1].parse().unwrap(); i += 1 }
            "--out" => { out = args[i + 1].clone(); i += 1 }
            "--report" => report = true,
            "--trace" => { trace = args[i + 1].parse().unwrap(); i += 1 }
            "--shot" => { shots.push(args[i + 1].parse::<u32>().unwrap()); i += 1 }
            _ => {}
        }
        i += 1;
    }
    if let Some(e) = exe {
        assert!(sys.load_exe(&e), "not a PS-X EXE");
    }
    let t = std::time::Instant::now();
    for f in 0..frames {
        sys.run_frame();
        if shots.contains(&f) {
            write_ppm(&sys, &format!("{}-{:04}.ppm", out.trim_end_matches(".ppm"), f));
        }
    }
    eprintln!("{} frames in {:?}, pc={:08x}", frames, t.elapsed(), sys.cpu.pc);
    // Instruction-level trace after the frames, for debugging.
    if trace > 0 {
        let d = sys.bus.read32(0x1F80_10F4);
        let p = sys.bus.read32(0x1F80_10F0);
        println!("DICR {d:08X} DPCR {p:08X}");
        for ch in 0..7u32 {
            let b = 0x1F80_1080 + ch * 0x10;
            println!("ch{ch} madr {:08X} bcr {:08X} chcr {:08X}", sys.bus.read32(b), sys.bus.read32(b + 4), sys.bus.read32(b + 8));
        }
    }
    sys.bus.io_trace = trace > 0;
    for _ in 0..trace {
        let pc = sys.cpu.pc;
        let instr = sys.bus.read32(pc);
        sys.bus.access_cycles = 0;
        let c = sys.cpu.step(&mut sys.bus);
        sys.bus.tick(c);
        println!(
            "{pc:08X} {instr:08X} sr {:08X} cause {:08X} istat {:03X} imask {:03X} v0 {:08X} a0 {:08X} t1 {:02X}",
            sys.cpu.sr, sys.cpu.cause, sys.bus.istat, sys.bus.imask, sys.cpu.regs[2], sys.cpu.regs[4], sys.cpu.regs[9]
        );
    }
    if !sys.bus.debug_out.is_empty() {
        std::io::stdout().write_all(&sys.bus.debug_out).unwrap();
    }
    if report {
        println!("{}", sys.debug_report());
    }
    if !sys.cpu.tty.is_empty() {
        eprintln!("--- tty ---\n{}", String::from_utf8_lossy(&sys.cpu.tty));
    }
    write_ppm(&sys, &out);
}

fn write_ppm(sys: &System, path: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    write!(f, "P6 {} {} 255\n", sys.width, sys.height).unwrap();
    for p in sys.frame.chunks_exact(4) {
        f.write_all(&p[..3]).unwrap();
    }
}
