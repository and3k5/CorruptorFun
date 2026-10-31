//! SPU: register file, sound RAM and a 24-voice ADPCM mixer.
//!
//! Implements what the BIOS boot sound needs: ADPCM decoding with loop
//! points, ADSR envelopes, pitch and volume. Reverb and noise are left out.

const RAM_SIZE: usize = 512 * 1024;
const CYCLES_PER_SAMPLE: u32 = 768; // 33.8688 MHz / 44100 Hz

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
    start: u32,
    repeat: u32,
    addr: u32,
    counter: u32,
    decoded: [i16; 28],
    prev: [i32; 2],
    block_flags: u8,
    phase: Phase,
    env: i32,
    env_wait: i32,
}

pub struct Spu {
    pub ram: Vec<u8>,
    regs: [u16; 0x200],
    transfer_addr: u32,
    voices: Vec<Voice>,
    endx: u32,
    sample_timer: u32,
    /// Interleaved stereo i16 samples produced since the last drain.
    pub out: Vec<i16>,
}

impl Spu {
    pub fn new() -> Self {
        Spu {
            ram: vec![0; RAM_SIZE],
            regs: [0; 0x200],
            transfer_addr: 0,
            voices: vec![Voice::default(); 24],
            endx: 0,
            sample_timer: 0,
            out: Vec::with_capacity(4096),
        }
    }

    fn reg(&self, off: usize) -> u16 {
        self.regs[off >> 1]
    }

    /// Voices currently sounding, for the debug report.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| !matches!(v.phase, Phase::Off)).count()
    }

    pub fn read16(&self, p: u32) -> u16 {
        let off = (p - 0x1F80_1C00) as usize & 0x3FE;
        match off {
            0x19C => self.endx as u16,
            0x19E => (self.endx >> 16) as u16,
            0x1AE => {
                // SPUSTAT mirrors SPUCNT's mode bits; transfers complete instantly.
                self.reg(0x1AA) & 0x3F
            }
            0x1A6 => (self.transfer_addr / 8) as u16,
            _ if off < 0x180 && off & 0xF == 0xC => {
                // Current ADSR volume
                self.voices[off >> 4].env as u16
            }
            _ => self.reg(off),
        }
    }

    pub fn write16(&mut self, p: u32, v: u16) {
        let off = (p - 0x1F80_1C00) as usize & 0x3FE;
        self.regs[off >> 1] = v;
        match off {
            0x188 => self.key_on(v as u32),
            0x18A => self.key_on((v as u32) << 16),
            0x18C => self.key_off(v as u32),
            0x18E => self.key_off((v as u32) << 16),
            0x1A6 => self.transfer_addr = (v as u32) * 8,
            0x1A8 => {
                let a = self.transfer_addr as usize & (RAM_SIZE - 2);
                self.ram[a..a + 2].copy_from_slice(&v.to_le_bytes());
                self.transfer_addr = (self.transfer_addr + 2) & (RAM_SIZE as u32 - 1);
            }
            _ => {}
        }
    }

    pub fn dma_write(&mut self, v: u32) {
        for h in [v as u16, (v >> 16) as u16] {
            let a = self.transfer_addr as usize & (RAM_SIZE - 2);
            self.ram[a..a + 2].copy_from_slice(&h.to_le_bytes());
            self.transfer_addr = (self.transfer_addr + 2) & (RAM_SIZE as u32 - 1);
        }
    }

    pub fn dma_read(&mut self) -> u32 {
        let a = self.transfer_addr as usize & (RAM_SIZE - 4);
        self.transfer_addr = (self.transfer_addr + 4) & (RAM_SIZE as u32 - 1);
        u32::from_le_bytes([self.ram[a], self.ram[a + 1], self.ram[a + 2], self.ram[a + 3]])
    }

    fn key_on(&mut self, mask: u32) {
        for i in 0..24 {
            if mask & (1 << i) != 0 {
                let start = self.reg(i * 16 + 6) as u32 * 8;
                let v = &mut self.voices[i];
                v.start = start;
                v.addr = start;
                v.repeat = start;
                v.counter = 0;
                v.prev = [0, 0];
                v.phase = Phase::Attack;
                v.env = 0;
                v.env_wait = 0;
                self.endx &= !(1 << i);
                self.decode_block(i);
            }
        }
    }

    fn key_off(&mut self, mask: u32) {
        for i in 0..24 {
            if mask & (1 << i) != 0 && self.voices[i].phase != Phase::Off {
                self.voices[i].phase = Phase::Release;
                self.voices[i].env_wait = 0;
            }
        }
    }

    fn decode_block(&mut self, i: usize) {
        let a = self.voices[i].addr as usize & (RAM_SIZE - 16);
        let header = self.ram[a];
        let flags = self.ram[a + 1];
        let shift = (header & 0xF).min(12) as i32;
        let filter = ((header >> 4) & 7).min(4) as usize;
        let v = &mut self.voices[i];
        v.block_flags = flags;
        if flags & 4 != 0 {
            v.repeat = a as u32;
        }
        let (f0, f1) = (ADPCM_POS[filter], ADPCM_NEG[filter]);
        for n in 0..28 {
            let byte = self.ram[a + 2 + n / 2];
            let nib = if n & 1 == 0 { byte & 0xF } else { byte >> 4 };
            let s = (((nib as i16) << 12) >> shift) as i32;
            let s = s + ((v.prev[0] * f0 + v.prev[1] * f1 + 32) >> 6);
            let s = s.clamp(-0x8000, 0x7FFF);
            v.prev[1] = v.prev[0];
            v.prev[0] = s;
            v.decoded[n] = s as i16;
        }
    }

    fn adsr_step(&mut self, i: usize) {
        let a1 = self.reg(i * 16 + 8) as u32;
        let a2 = self.reg(i * 16 + 10) as u32;
        let v = &mut self.voices[i];
        // (exponential, decreasing, shift, step, target)
        let (exp, dec, shift, step, target): (bool, bool, i32, i32, i32) = match v.phase {
            Phase::Attack => (a1 & 0x8000 != 0, false, ((a1 >> 10) & 0x1F) as i32, 7 - ((a1 >> 8) & 3) as i32, 0x7FFF),
            Phase::Decay => (true, true, ((a1 >> 4) & 0xF) as i32 * 4, -8, (((a1 & 0xF) as i32 + 1) * 0x800).min(0x7FFF)),
            Phase::Sustain => {
                let dec = a2 & 0x4000 != 0;
                let st = ((a2 >> 6) & 3) as i32;
                (a2 & 0x8000 != 0, dec, ((a2 >> 8) & 0x1F) as i32, if dec { -8 + st } else { 7 - st }, -1)
            }
            Phase::Release => (a2 & 0x20 != 0, true, (a2 & 0x1F) as i32 * 4, -8, 0),
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

    fn sample(&mut self) -> (i32, i32) {
        let mut l = 0i32;
        let mut r = 0i32;
        for i in 0..24 {
            if self.voices[i].phase == Phase::Off {
                continue;
            }
            self.adsr_step(i);
            let pitch = (self.reg(i * 16 + 4) as u32).min(0x4000);
            let v = &self.voices[i];
            let idx = (v.counter >> 12) as usize;
            let s = v.decoded[idx.min(27)] as i32 * v.env >> 15;
            let vol = |x: u16| -> i32 { if x & 0x8000 != 0 { 0x3FFF } else { ((x as i16) << 1) as i32 >> 1 } };
            l += s * vol(self.reg(i * 16)) >> 14;
            r += s * vol(self.reg(i * 16 + 2)) >> 14;
            self.voices[i].counter += pitch;
            while self.voices[i].counter >= 28 << 12 {
                self.voices[i].counter -= 28 << 12;
                let flags = self.voices[i].block_flags;
                if flags & 1 != 0 {
                    self.endx |= 1 << i;
                    let v = &mut self.voices[i];
                    v.addr = v.repeat;
                    if flags & 2 == 0 {
                        v.phase = Phase::Off;
                        v.env = 0;
                    }
                } else {
                    let v = &mut self.voices[i];
                    v.addr = (v.addr + 16) & (RAM_SIZE as u32 - 1);
                }
                self.decode_block(i);
                if self.voices[i].phase == Phase::Off {
                    break;
                }
            }
        }
        let vol = |x: u16| -> i32 { if x & 0x8000 != 0 { 0x3FFF } else { ((x as i16) << 1) as i32 >> 1 } };
        let ml = vol(self.reg(0x180));
        let mr = vol(self.reg(0x182));
        ((l * ml) >> 14, (r * mr) >> 14)
    }

    /// Returns true if the SPU IRQ fires (not emulated).
    pub fn tick(&mut self, cycles: u32) -> bool {
        self.sample_timer += cycles;
        while self.sample_timer >= CYCLES_PER_SAMPLE {
            self.sample_timer -= CYCLES_PER_SAMPLE;
            let enabled = self.reg(0x1AA) & 0x8000 != 0;
            let unmuted = self.reg(0x1AA) & 0x4000 != 0;
            let (l, r) = if enabled { self.sample() } else { (0, 0) };
            if self.out.len() < 1 << 16 {
                if unmuted {
                    self.out.push(l.clamp(-0x8000, 0x7FFF) as i16);
                    self.out.push(r.clamp(-0x8000, 0x7FFF) as i16);
                } else {
                    self.out.push(0);
                    self.out.push(0);
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_voice_produces_sound() {
        let mut s = Spu::new();
        // One looping ADPCM block of a square-ish wave at 0x1000.
        let mut block = [0u8; 16];
        block[0] = 0x00; // shift 0, filter 0
        block[1] = 0x03; // loop end + repeat
        for b in block[2..].iter_mut() {
            *b = 0x77;
        }
        s.ram[0x1000..0x1010].copy_from_slice(&block);
        let base = 0x1F80_1C00;
        s.write16(base + 0x1AA, 0xC000); // SPU on, unmuted
        s.write16(base + 0x180, 0x3FFF);
        s.write16(base + 0x182, 0x3FFF);
        s.write16(base, 0x3FFF);
        s.write16(base + 2, 0x3FFF);
        s.write16(base + 4, 0x1000); // pitch 1.0
        s.write16(base + 6, 0x1000 / 8); // start address
        s.write16(base + 8, 0x000F); // fast attack
        s.write16(base + 10, 0x0000);
        s.write16(base + 0x188, 1); // key on voice 0
        s.tick(768 * 2000);
        assert!(s.out.iter().any(|&x| x.abs() > 1000));
        assert_eq!(s.read16(base + 0x1AE) & 0x3F, 0);
    }
}
