//! Headless PS2 runner for debugging:
//! `ps2-cli BIOS.BIN [--frames N] [--out frame.ppm] [--report] [--files]`

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: ps2-cli BIOS.BIN [--frames N] [--out frame.ppm] [--report] [--files] [--every N]");
        std::process::exit(1);
    }
    // REPLAY=file: time drawing alone, from commands saved with REC=file.
    if let Ok(f) = std::env::var("REPLAY") {
        let raw = std::fs::read(f).expect("read replay");
        let words: Vec<u64> = raw.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        let mut gs = ps2::gs::Gs::new();
        let t = std::time::Instant::now();
        let (mut i, mut n) = (0, 0);
        while i < words.len() {
            let len = words[i] as usize;
            gs.replay(&words[i + 1..i + 1 + len]);
            i += 1 + len;
            n += 1;
        }
        eprintln!("replayed {n} frames in {:.2?}", t.elapsed());
        if let Ok(o) = std::env::var("REPLAY_OUT") {
            let mut frame = Vec::new();
            let (w, h) = gs.render(&mut frame);
            let mut f = std::fs::File::create(o).unwrap();
            write!(f, "P6\n{} {}\n255\n", w, h).unwrap();
            let rgb: Vec<u8> = frame.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            f.write_all(&rgb).unwrap();
        }
        return;
    }
    let bios = std::fs::read(&args[1]).expect("read BIOS");
    let mut frames = 300;
    let mut out: Option<String> = None;
    let mut report = false;
    let mut pcs = false;
    let mut wav: Option<String> = None;
    let mut audio: Vec<i16> = Vec::new();
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
            "--pcs" => pcs = true,
            "--wav" => {
                wav = Some(args[i + 1].clone());
                i += 1;
            }
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
    sys.sample_pcs = pcs;
    if let Ok(v) = std::env::var("CPI") {
        sys.ee_cpi = v.parse().unwrap();
    }
    // REMOTE=1: draw in a second GS fed with the recorded commands, as the
    // browser does with two workers.
    let mut rec = std::env::var("REC").ok().map(|f| std::io::BufWriter::new(std::fs::File::create(f).unwrap()));
    if rec.is_some() {
        sys.hw.gs.remote = true;
    }
    let mut drawer = std::env::var("REMOTE").ok().map(|_| {
        sys.hw.gs.remote = true;
        ps2::gs::Gs::new()
    });
    let t = std::time::Instant::now();
    let mut lap = std::time::Instant::now();
    for f in 0..frames {
        if std::env::var("LAPS").is_ok() && f % 50 == 0 && f > 0 {
            eprintln!("frames {}-{}: {:.2?}, GS pixels {} prims {} tex {:?}", f - 50, f, lap.elapsed(), sys.hw.gs.draw.pixels, sys.hw.gs.draw.prims, sys.hw.gs.draw.stat_tex);
            lap = std::time::Instant::now();
        }
        if std::env::var("SKIPODD").is_ok() {
            sys.hw.gs.draw.skip = f % 2 == 1;
        }
        if std::env::var("SKIPALL").is_ok() {
            sys.hw.gs.draw.skip = true;
        }
        sys.run_frame();
        if let Some(r) = rec.as_mut() {
            // The display registers ride along, marked 0x7F.
            for v in sys.hw.gs.display_regs() {
                sys.hw.gs.cmds.push(0x7F);
                sys.hw.gs.cmds.push(v);
            }
            let c = &sys.hw.gs.cmds;
            r.write_all(&(c.len() as u64).to_le_bytes()).unwrap();
            for v in c {
                r.write_all(&v.to_le_bytes()).unwrap();
            }
            sys.hw.gs.cmds.clear();
        }
        if let Some(d) = drawer.as_mut() {
            d.replay(&sys.hw.gs.cmds);
            sys.hw.gs.cmds.clear();
            d.set_display_regs(&sys.hw.gs.display_regs());
        }
        if wav.is_some() {
            audio.extend_from_slice(&sys.hw.spu2.out);
            sys.hw.spu2.out.clear();
        }
        if every > 0 && f % every == 0 {
            if let Some(o) = &out {
                if let Some(d) = drawer.as_ref() {
                    let (w, h) = d.render(&mut sys.frame);
                    sys.width = w;
                    sys.height = h;
                } else {
                    sys.render_frame();
                }
                save(&sys, &o.replace(".ppm", &format!("-{f:04}.ppm")));
            }
        }
    }
    eprintln!("{} frames in {:.2?} (idle batches {})", frames, t.elapsed(), sys.idle_batches);
    if let Some(o) = out {
        if let Some(d) = drawer.as_ref() {
            let (w, h) = d.render(&mut sys.frame);
            sys.width = w;
            sys.height = h;
        } else {
            sys.render_frame();
        }
        save(&sys, &o);
    }
    if let Some(d) = dump {
        std::fs::write(format!("{d}.iop"), &sys.hw.iop_ram).unwrap();
        std::fs::write(format!("{d}.ee"), &sys.hw.ram).unwrap();
    }
    if let Some(w) = wav {
        let mut b = Vec::new();
        let n = (audio.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + n).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        for v in [16u32, 0x0002_0001, 48000, 48000 * 4, 0x0010_0004] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(b"data");
        b.extend_from_slice(&n.to_le_bytes());
        for x in audio {
            b.extend_from_slice(&x.to_le_bytes());
        }
        std::fs::write(w, b).unwrap();
    }
    if report {
        println!("{}", sys.debug_report());
    }
    if pcs {
        let mut v: Vec<_> = sys.pc_hist_ee.iter().map(|(a, b)| (*b, *a)).collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        let total: u32 = v.iter().map(|x| x.0).sum();
        for (n, pc) in v.iter().take(40) {
            println!("PC {pc:08X} {:5.2}%", *n as f64 * 100.0 / total as f64);
        }
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
