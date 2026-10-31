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
    while i < args.len() {
        match args[i].as_str() {
            "--exe" => { exe = Some(std::fs::read(&args[i + 1]).expect("exe")); i += 1 }
            "--disc" => { sys.bus.cdrom.set_disc(Some(std::fs::read(&args[i + 1]).expect("disc"))); i += 1 }
            "--frames" => { frames = args[i + 1].parse().unwrap(); i += 1 }
            "--out" => { out = args[i + 1].clone(); i += 1 }
            "--report" => report = true,
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
