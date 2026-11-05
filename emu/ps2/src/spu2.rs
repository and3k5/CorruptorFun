//! SPU2: two SPU cores of 24 ADPCM voices each, sharing 2 MB of sound RAM.
//!
//! Enough for the BIOS boot sound: register file, manual and DMA transfers,
//! ADPCM decoding with loop points, ADSR envelopes, pitch and volume.
//! Reverb, noise and the streaming (ADMA) inputs are left out.

const RAM_WORDS: usize = 1 << 20; // 2 MB as 16-bit words
const IOP_CYCLES_PER_SAMPLE: u32 = 768; // 36.864 MHz / 48 kHz

const ADPCM_POS: [i32; 5] = [0, 60, 115, 98, 122];
const ADPCM_NEG: [i32; 5] = [0, 0, -52, -55, -60];

#[derive(Clone, Copy, PartialEq, Default)]
enum Phase {
    #[default]
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Default)]
struct Voice {
    /// Addresses in 16-bit words.
    start: u32,
    repeat: u32,
    repeat_set: bool,
    addr: u32,
    counter: u32,
    decoded: [i16; 28],
    prev: [i32; 2],
    block_flags: u8,
    phase: Phase,
    env: i32,
    env_wait: i32,
    note: Option<usize>,
    read: u64,
}

/// One note played: `[start address (bytes), pitch at key on, lowest pitch,
/// highest pitch, samples sounded, sound-RAM samples read, key-on time
/// (`Spu2::clock`), how it ended (0 on, 1 released, 2 sample ended, 3 re-keyed)]`.
/// Voice index is folded in: core * 24 + voice goes in the top byte of field 7.
pub type Note = [u32; 8];
const MAX_NOTES: usize = 2048;

pub struct Spu2 {
    pub ram: Vec<u16>,
    /// Register file, 0x800 bytes as words (both cores and the shared area).
    regs: Vec<u16>,
    tsa: [u32; 2],
    voices: Vec<Voice>,
    endx: [u32; 2],
    sample_timer: u32,
    /// Interleaved stereo i16 at 48 kHz, drained by the host.
    pub out: Vec<i16>,
    pub notes: Vec<Note>,
    pub clock: u32,
    pub dma_words: [u32; 2],
    pub key_ons: u32,
}

fn vol(x: u16) -> i32 {
    // Sweep mode is treated as full volume.
    if x & 0x8000 != 0 { 0x3FFF } else { ((x as i16) << 1) as i32 >> 1 }
}

impl Spu2 {
    pub fn new() -> Self {
        Spu2 {
            ram: vec![0; RAM_WORDS],
            regs: vec![0; 0x400],
            tsa: [0; 2],
            voices: vec![Voice::default(); 48],
            endx: [0; 2],
            sample_timer: 0,
            out: Vec::with_capacity(8192),
            notes: Vec::new(),
            clock: 0,
            dma_words: [0; 2],
            key_ons: 0,
        }
    }

    fn reg(&self, off: usize) -> u16 {
        self.regs[(off & 0x7FF) >> 1]
    }

    fn vreg(&self, core: usize, v: usize, r: usize) -> u16 {
        self.reg(core * 0x400 + v * 0x10 + r)
    }

    fn addr_reg(&self, off: usize) -> u32 {
        ((self.reg(off) as u32 & 0xF) << 16) | self.reg(off + 2) as u32
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.phase != Phase::Off).count()
    }

    pub fn read16(&self, p: u32) -> u16 {
        let off = (p & 0x7FF) as usize;
        let core = if off < 0x760 { off >> 10 & 1 } else { (off >= 0x788) as usize };
        let lo = off & 0x3FF;
        if off < 0x760 {
            match lo {
                0x340 => return self.endx[core] as u16,
                0x342 => return (self.endx[core] >> 16) as u16,
                0x344 => return 0x80, // STATX: DMA ready, not busy
                0x1A8 => return (self.tsa[core] >> 16) as u16,
                0x1AA => return self.tsa[core] as u16,
                0x19A => return self.reg(off) & 0x7FFF,
                _ if lo < 0x180 && lo & 0xF == 0xA => return self.voices[core * 24 + (lo >> 4)].env as u16,
                _ if (0x1C0..0x2E0).contains(&lo) && (lo - 0x1C0) % 0xC >= 8 => {
                    // NAX: where the voice is now.
                    let v = &self.voices[core * 24 + (lo - 0x1C0) / 0xC];
                    return if (lo - 0x1C0) % 0xC == 8 { (v.addr >> 16) as u16 & 0xF } else { v.addr as u16 };
                }
                _ => {}
            }
        }
        self.reg(off)
    }

    pub fn write16(&mut self, p: u32, v: u16) {
        let off = (p & 0x7FF) as usize;
        self.regs[off >> 1] = v;
        if off >= 0x760 {
            return;
        }
        let core = off >> 10 & 1;
        match off & 0x3FF {
            0x19A => {
                if v & 0x8000 != 0 {
                    // Core reset.
                    for i in 0..24 {
                        self.voices[core * 24 + i] = Voice::default();
                    }
                    self.endx[core] = 0;
                }
            }
            0x1A0 => self.key_on(core, v as u32),
            0x1A2 => self.key_on(core, (v as u32 & 0xFF) << 16),
            0x1A4 => self.key_off(core, v as u32),
            0x1A6 => self.key_off(core, (v as u32 & 0xFF) << 16),
            0x1A8 => self.tsa[core] = (self.tsa[core] & 0xFFFF) | ((v as u32 & 0xF) << 16),
            0x1AA => self.tsa[core] = (self.tsa[core] & 0xF_0000) | v as u32,
            0x1AC => self.data_write(core, v),
            0x340 | 0x342 => self.endx[core] = 0,
            lo if (0x1C0..0x2E0).contains(&lo) && (lo - 0x1C0) % 0xC == 6 => {
                // LSAX written: the program sets the loop point itself.
                self.voices[core * 24 + (lo - 0x1C0) / 0xC].repeat_set = true;
            }
            _ => {}
        }
    }

    fn data_write(&mut self, core: usize, v: u16) {
        let a = self.tsa[core] as usize & (RAM_WORDS - 1);
        self.ram[a] = v;
        self.tsa[core] = (self.tsa[core] + 1) & (RAM_WORDS as u32 - 1);
    }

    /// IOP DMA (channel 4 = core 0, 7 = core 1) into sound RAM.
    pub fn dma_write(&mut self, core: usize, w: u32) {
        self.data_write(core, w as u16);
        self.data_write(core, (w >> 16) as u16);
        self.dma_words[core] += 1;
    }

    pub fn dma_read(&mut self, core: usize) -> u32 {
        let a = self.tsa[core] as usize & (RAM_WORDS - 1);
        let b = (a + 1) & (RAM_WORDS - 1);
        self.tsa[core] = (self.tsa[core] + 2) & (RAM_WORDS as u32 - 1);
        self.ram[a] as u32 | (self.ram[b] as u32) << 16
    }

    fn key_on(&mut self, core: usize, mask: u32) {
        for i in 0..24 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let n = core * 24 + i;
            self.key_ons += 1;
            let start = self.addr_reg(core * 0x400 + 0x1C0 + i * 0xC);
            let pitch = (self.vreg(core, i, 4) as u32).min(0x3FFF);
            self.end_note(n, 3);
            if self.notes.len() < MAX_NOTES {
                self.voices[n].note = Some(self.notes.len());
                self.notes.push([start * 2, pitch, pitch, pitch, 0, 0, self.clock, (n as u32) << 24]);
            }
            let v = &mut self.voices[n];
            v.start = start;
            v.addr = start;
            v.repeat = start;
            v.repeat_set = false;
            v.counter = 0;
            v.read = 0;
            v.prev = [0, 0];
            v.phase = Phase::Attack;
            v.env = 0;
            v.env_wait = 0;
            self.endx[core] &= !(1 << i);
            self.decode_block(n);
        }
    }

    fn key_off(&mut self, core: usize, mask: u32) {
        for i in 0..24 {
            let v = &mut self.voices[core * 24 + i];
            if mask & (1 << i) != 0 && v.phase != Phase::Off {
                v.phase = Phase::Release;
                v.env_wait = 0;
            }
        }
    }

    fn end_note(&mut self, n: usize, how: u32) {
        if let Some(k) = self.voices[n].note.take() {
            self.notes[k][7] = (self.notes[k][7] & 0xFF00_0000) | how;
        }
    }

    fn decode_block(&mut self, n: usize) {
        let a = self.voices[n].addr as usize & (RAM_WORDS - 8);
        let w0 = self.ram[a];
        let header = w0 as u8;
        let flags = (w0 >> 8) as u8;
        let shift = (header & 0xF).min(12) as i32;
        let filter = ((header >> 4) & 7).min(4) as usize;
        let (f0, f1) = (ADPCM_POS[filter], ADPCM_NEG[filter]);
        let core = n / 24;
        let lsax_off = core * 0x400 + 0x1C0 + (n % 24) * 0xC + 4;
        let lsax = self.addr_reg(lsax_off);
        let v = &mut self.voices[n];
        v.block_flags = flags;
        if flags & 4 != 0 && !v.repeat_set {
            v.repeat = a as u32;
        } else if v.repeat_set {
            v.repeat = lsax;
        }
        for k in 0..28 {
            let w = self.ram[a + 1 + k / 4];
            let nib = (w >> ((k & 3) * 4)) & 0xF;
            let s = ((((nib as i16) << 12) >> shift) as i32) + ((v.prev[0] * f0 + v.prev[1] * f1 + 32) >> 6);
            let s = s.clamp(-0x8000, 0x7FFF);
            v.prev[1] = v.prev[0];
            v.prev[0] = s;
            v.decoded[k] = s as i16;
        }
    }

    fn adsr_step(&mut self, n: usize) {
        let (core, i) = (n / 24, n % 24);
        let a1 = self.vreg(core, i, 6) as u32;
        let a2 = self.vreg(core, i, 8) as u32;
        let v = &mut self.voices[n];
        let (exp, dec, shift, step, target): (bool, bool, i32, i32, i32) = match v.phase {
            Phase::Attack => (a1 & 0x8000 != 0, false, ((a1 >> 10) & 0x1F) as i32, 7 - ((a1 >> 8) & 3) as i32, 0x7FFF),
            Phase::Decay => (true, true, ((a1 >> 4) & 0xF) as i32, -8, (((a1 & 0xF) as i32 + 1) * 0x800).min(0x7FFF)),
            Phase::Sustain => {
                let dec = a2 & 0x4000 != 0;
                let st = ((a2 >> 6) & 3) as i32;
                (a2 & 0x8000 != 0, dec, ((a2 >> 8) & 0x1F) as i32, if dec { -8 + st } else { 7 - st }, -1)
            }
            Phase::Release => (a2 & 0x20 != 0, true, (a2 & 0x1F) as i32, -8, 0),
            Phase::Off => return,
        };
        if v.env_wait > 0 {
            v.env_wait -= 1;
            return;
        }
        let mut wait = 1i32 << (shift - 11).max(0);
        let mut s = step << (11 - shift).max(0);
        if exp && !dec && v.env > 0x6000 {
            wait *= 4;
        }
        if exp && dec {
            s = (s * v.env) >> 15;
        }
        v.env_wait = wait - 1;
        v.env = (v.env + s).clamp(0, 0x7FFF);
        match v.phase {
            Phase::Attack if v.env >= 0x7FFF => v.phase = Phase::Decay,
            Phase::Decay if v.env <= target => v.phase = Phase::Sustain,
            Phase::Release if v.env <= 0 => v.phase = Phase::Off,
            _ => {}
        }
    }

    fn voice_sample(&mut self, n: usize) -> i32 {
        let (core, i) = (n / 24, n % 24);
        self.adsr_step(n);
        if self.voices[n].phase == Phase::Off {
            self.end_note(n, 1);
            return 0;
        }
        let pitch = (self.vreg(core, i, 4) as u32).min(0x3FFF);
        if let Some(k) = self.voices[n].note {
            let e = &mut self.notes[k];
            e[2] = e[2].min(pitch);
            e[3] = e[3].max(pitch);
            e[4] += 1;
            self.voices[n].read += pitch as u64;
            e[5] = (self.voices[n].read >> 12) as u32;
        }
        let v = &self.voices[n];
        let s = v.decoded[((v.counter >> 12) as usize).min(27)] as i32 * v.env >> 15;
        self.voices[n].counter += pitch;
        while self.voices[n].counter >= 28 << 12 {
            self.voices[n].counter -= 28 << 12;
            let flags = self.voices[n].block_flags;
            if flags & 1 != 0 {
                self.endx[core] |= 1 << i;
                let v = &mut self.voices[n];
                v.addr = v.repeat;
                if flags & 2 == 0 {
                    v.phase = Phase::Off;
                    v.env = 0;
                }
            } else {
                let v = &mut self.voices[n];
                v.addr = (v.addr + 8) & (RAM_WORDS as u32 - 1);
            }
            self.decode_block(n);
            if self.voices[n].phase == Phase::Off {
                self.end_note(n, 2);
                break;
            }
        }
        s
    }

    fn sample(&mut self) -> (i32, i32) {
        let mut ext = (0i32, 0i32);
        for core in 0..2 {
            let base = core * 0x400;
            let attr = self.reg(base + 0x19A);
            let mix_l = (self.reg(base + 0x188) as u32) | (self.reg(base + 0x18A) as u32 & 0xFF) << 16;
            let mix_r = (self.reg(base + 0x190) as u32) | (self.reg(base + 0x192) as u32 & 0xFF) << 16;
            let (mut l, mut r) = (0i32, 0i32);
            for i in 0..24 {
                let n = core * 24 + i;
                if self.voices[n].phase == Phase::Off {
                    continue;
                }
                let s = self.voice_sample(n);
                if mix_l & (1 << i) != 0 {
                    l += s * vol(self.vreg(core, i, 0)) >> 14;
                }
                if mix_r & (1 << i) != 0 {
                    r += s * vol(self.vreg(core, i, 2)) >> 14;
                }
            }
            // Core 0's output feeds core 1's external input.
            let mmix = self.reg(base + 0x198);
            if core == 1 && mmix & 0xC0 != 0 {
                l += ext.0;
                r += ext.1;
            }
            let mv = 0x760 + core * 0x28;
            let (ml, mr) = (vol(self.reg(mv)), vol(self.reg(mv + 2)));
            let on = attr & 0x8000 == 0 || true;
            let (ol, or) = if on { ((l * ml) >> 14, (r * mr) >> 14) } else { (0, 0) };
            if core == 0 {
                ext = (ol, or);
            } else {
                return (ol, or);
            }
        }
        (0, 0)
    }

    pub fn tick(&mut self, iop_cycles: u32) {
        self.sample_timer += iop_cycles;
        while self.sample_timer >= IOP_CYCLES_PER_SAMPLE {
            self.sample_timer -= IOP_CYCLES_PER_SAMPLE;
            self.clock = self.clock.wrapping_add(1);
            let (l, r) = self.sample();
            if self.out.len() < 1 << 17 {
                self.out.push(l.clamp(-0x8000, 0x7FFF) as i16);
                self.out.push(r.clamp(-0x8000, 0x7FFF) as i16);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_voice_makes_sound() {
        let mut s = Spu2::new();
        // One looping ADPCM block of loud nibbles at word 0x1000.
        s.ram[0x1000] = 0x0300; // shift 0, flags end+repeat
        for k in 1..8 {
            s.ram[0x1000 + k] = 0x7777;
        }
        s.write16(0x1F90_01C0 + 2, 0x1000); // SSA lo, voice 0 core 0
        s.write16(0x1F90_0004, 0x1000); // pitch
        s.write16(0x1F90_0006, 0x00FF); // fast attack
        s.write16(0x1F90_0000, 0x3FFF);
        s.write16(0x1F90_0002, 0x3FFF);
        s.write16(0x1F90_0188, 1); // VMIXL
        s.write16(0x1F90_0190, 1);
        s.write16(0x1F90_0760, 0x3FFF);
        s.write16(0x1F90_0762, 0x3FFF);
        s.write16(0x1F90_0598, 0xFF); // core 1 MMIX: take core 0
        s.write16(0x1F90_0788, 0x3FFF);
        s.write16(0x1F90_078A, 0x3FFF);
        s.write16(0x1F90_01A0, 1);
        s.tick(768 * 2000);
        assert!(s.out.iter().any(|&x| x.abs() > 1000));
        assert_eq!(s.notes.len(), 1);
    }
}
