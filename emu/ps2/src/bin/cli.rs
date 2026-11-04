//! Headless PS2 runner for debugging:
//! `ps2-cli BIOS.BIN [--frames N] [--out frame.ppm] [--report] [--files]`

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: ps2-cli BIOS.BIN [--frames N] [--out frame.ppm] [--report] [--files] [--every N]");
        std::process::exit(1);
    }
    let bios = std::fs::read(&args[1]).expect("read BIOS");
    let mut frames = 300;
    let mut out: Option<String> = None;
    let mut report = false;
    let mut every = 0;
    let mut dump: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--frames" => {
                frames = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--out" => {
                out = Some(args[i + 1].clone());
                i += 1;
            }
            "--every" => {
                every = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--dump" => {
                dump = Some(args[i + 1].clone());
                i += 1;
            }
            "--report" => report = true,
            "--files" => {
                for f in ps2::romdir::parse(&bios) {
                    println!("{:10} {:08X} {:08X}", f.name, f.offset, f.size);
                }
                return;
            }
            a => panic!("unknown argument {a}"),
        }
        i += 1;
    }
    let mut sys = ps2::System::new(bios);
    sys.render = false;
    let t = std::time::Instant::now();
    for f in 0..frames {
        sys.run_frame();
        if every > 0 && f % every == 0 {
            if let Some(o) = &out {
                sys.render_frame();
                save(&sys, &o.replace(".ppm", &format!("-{f:04}.ppm")));
            }
        }
    }
    eprintln!("{} frames in {:.2?}", frames, t.elapsed());
    if let Some(o) = out {
        sys.render_frame();
        save(&sys, &o);
    }
    if let Some(d) = dump {
        std::fs::write(format!("{d}.iop"), &sys.hw.iop_ram).unwrap();
        std::fs::write(format!("{d}.ee"), &sys.hw.ram).unwrap();
    }
    if report {
        println!("{}", sys.debug_report());
    }
}

fn save(sys: &ps2::System, path: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    write!(f, "P6\n{} {}\n255\n", sys.width, sys.height).unwrap();
    let mut rgb = Vec::with_capacity((sys.width * sys.height * 3) as usize);
    for p in sys.frame.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    f.write_all(&rgb).unwrap();
}
