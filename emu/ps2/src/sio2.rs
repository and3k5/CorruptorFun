//! SIO2: the IOP's serial bus to the controller and memory card ports.
//! No controller or memory card is plugged in yet, so every transfer comes
//! back as "no device".

use std::collections::VecDeque;

pub struct Sio2 {
    pub send3: [u32; 16],
    pub send12: [u32; 8],
    pub ctrl: u32,
    pub recv1: u32,
    pub istat: u32,
    pub fifo_in: VecDeque<u8>,
    pub fifo_out: VecDeque<u8>,
    pub transfers: u32,
    /// Raise SIO2's IOP interrupt.
    pub irq: bool,
}

impl Sio2 {
    pub fn new() -> Self {
        Sio2 { send3: [0; 16], send12: [0; 8], ctrl: 0, recv1: 0x1D100, istat: 0, fifo_in: Default::default(), fifo_out: Default::default(), transfers: 0, irq: false }
    }

    pub fn read(&mut self, p: u32) -> u32 {
        match p & 0xFF {
            o @ 0x00..=0x3F => self.send3[(o >> 2) as usize],
            o @ 0x40..=0x5F => self.send12[((o - 0x40) >> 2) as usize],
            0x64 => self.fifo_out.pop_front().unwrap_or(0xFF) as u32,
            0x68 => self.ctrl,
            0x6C => self.recv1,
            0x70 => 0xF,
            0x74 => 0,
            0x80 => self.istat,
            _ => 0,
        }
    }

    pub fn write(&mut self, p: u32, v: u32) {
        match p & 0xFF {
            o @ 0x00..=0x3F => self.send3[(o >> 2) as usize] = v,
            o @ 0x40..=0x5F => self.send12[((o - 0x40) >> 2) as usize] = v,
            0x60 => self.fifo_in.push_back(v as u8),
            0x68 => {
                self.ctrl = v & !1;
                if v & 0xC == 0xC {
                    self.fifo_in.clear();
                    self.fifo_out.clear();
                }
                if v & 1 != 0 {
                    self.run();
                }
            }
            0x80 => self.istat &= !v,
            _ => {}
        }
    }

    fn run(&mut self) {
        self.transfers += 1;
        for i in 0..16 {
            let e = self.send3[i];
            if e == 0 {
                break;
            }
            let in_len = ((e >> 8) & 0x1FF) as usize;
            let out_len = ((e >> 18) & 0x1FF) as usize;
            for _ in 0..in_len {
                self.fifo_in.pop_front();
            }
            for _ in 0..out_len {
                self.fifo_out.push_back(0xFF);
            }
        }
        self.fifo_in.clear();
        // Transfer done, no device connected.
        self.recv1 = 0x1D100;
        self.istat |= 1;
        self.irq = true;
    }
}
