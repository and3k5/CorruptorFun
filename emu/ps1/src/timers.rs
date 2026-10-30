//! Root counters 0-2.

use crate::gpu::TickEvents;

#[derive(Default, Clone, Copy)]
struct Timer {
    counter: u32,
    mode: u32,
    target: u32,
    frac: u32,
    fired: bool,
    paused: bool,
}

pub struct Timers {
    t: [Timer; 3],
}

impl Timers {
    pub fn new() -> Self {
        let mut t = [Timer::default(); 3];
        for x in t.iter_mut() {
            x.mode = 1 << 10;
        }
        Timers { t }
    }

    pub fn read(&mut self, p: u32) -> u32 {
        let i = ((p >> 4) & 3) as usize;
        if i > 2 {
            return 0;
        }
        match p & 0xF {
            0 => self.t[i].counter,
            4 => {
                let m = self.t[i].mode;
                // Reached flags clear on read.
                self.t[i].mode &= !((1 << 11) | (1 << 12));
                m
            }
            8 => self.t[i].target,
            _ => 0,
        }
    }

    pub fn write(&mut self, p: u32, v: u32) {
        let i = ((p >> 4) & 3) as usize;
        if i > 2 {
            return;
        }
        let t = &mut self.t[i];
        match p & 0xF {
            0 => t.counter = v & 0xFFFF,
            4 => {
                t.mode = (v & 0x3FF) | (1 << 10);
                t.counter = 0;
                t.fired = false;
                // Timer 2 sync modes 0 and 3 stop the counter.
                t.paused = i == 2 && v & 1 != 0 && matches!((v >> 1) & 3, 0 | 3);
            }
            8 => t.target = v & 0xFFFF,
            _ => {}
        }
    }

    /// Returns a bitmask of timers that raised an IRQ.
    pub fn tick(&mut self, cycles: u32, ev: &TickEvents, dot_div: u32) -> u32 {
        let mut irq = 0;
        for i in 0..3 {
            let t = &mut self.t[i];
            if t.paused {
                continue;
            }
            let src = (t.mode >> 8) & 3;
            let inc = match (i, src) {
                (0, 1) | (0, 3) => {
                    // Dot clock: GPU clock (11/7 CPU) divided by the dot divider.
                    t.frac += cycles * 11;
                    let d = 7 * dot_div;
                    let n = t.frac / d;
                    t.frac %= d;
                    n
                }
                (1, 1) | (1, 3) => ev.hblanks,
                (2, 2) | (2, 3) => {
                    t.frac += cycles;
                    let n = t.frac / 8;
                    t.frac %= 8;
                    n
                }
                _ => cycles,
            };
            if inc == 0 {
                continue;
            }
            let target = if t.target == 0 { 0x10000 } else { t.target };
            let old = t.counter;
            let mut c = old + inc;
            let mut hit = false;
            if old < target && c >= target {
                t.mode |= 1 << 11;
                if t.mode & (1 << 4) != 0 {
                    hit = true;
                }
                if t.mode & (1 << 3) != 0 {
                    c = (c - target) % target.max(1);
                }
            }
            if c >= 0x10000 {
                t.mode |= 1 << 12;
                if t.mode & (1 << 5) != 0 {
                    hit = true;
                }
                c &= 0xFFFF;
            }
            t.counter = c;
            if hit {
                let repeat = t.mode & (1 << 6) != 0;
                if repeat || !t.fired {
                    t.fired = true;
                    if t.mode & (1 << 7) != 0 {
                        t.mode ^= 1 << 10; // toggle mode
                        if t.mode & (1 << 10) == 0 {
                            irq |= 1 << i;
                        }
                    } else {
                        irq |= 1 << i;
                        t.mode &= !(1 << 10);
                        t.mode |= 1 << 10; // pulse back up
                    }
                }
            }
        }
        irq
    }
}
