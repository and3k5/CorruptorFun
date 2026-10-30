//! CD-ROM controller (HLE of the drive's microcontroller).
//!
//! Supports no disc (the BIOS opens its shell) and a single-track Mode 2
//! data disc image in raw 2352-byte sectors (the licensed boot path).

use std::collections::VecDeque;

const SECTOR: usize = 2352;
const CPU_HZ: u32 = 33_868_800;
const FIRST_RESPONSE: u32 = 25_000;
const SECOND_RESPONSE: u32 = 100_000;

struct Pending {
    delay: u32,
    int: u8,
    data: Vec<u8>,
}

pub struct Cdrom {
    index: u8,
    params: VecDeque<u8>,
    response: VecDeque<u8>,
    data: VecDeque<u8>,
    int_enable: u8,
    int_flag: u8,
    queue: VecDeque<Pending>,
    busy: bool,
    stat: u8,
    mode: u8,
    setloc: u32,
    pos: u32,
    reading: bool,
    read_timer: u32,
    sector_buf: Vec<u8>,
    last_header: [u8; 8],
    disc: Option<Vec<u8>>,
    /// Region letter used to answer GetID: b'A', b'E' or b'I'.
    pub region: u8,
}

fn bcd(v: u32) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

fn from_bcd(v: u8) -> u32 {
    ((v >> 4) as u32) * 10 + (v & 0xF) as u32
}

impl Cdrom {
    pub fn new() -> Self {
        Cdrom {
            index: 0,
            params: VecDeque::new(),
            response: VecDeque::new(),
            data: VecDeque::new(),
            int_enable: 0,
            int_flag: 0,
            queue: VecDeque::new(),
            busy: false,
            stat: 0,
            mode: 0,
            setloc: 0,
            pos: 0,
            reading: false,
            read_timer: 0,
            sector_buf: Vec::new(),
            last_header: [0; 8],
            disc: None,
            region: b'E',
        }
    }

    pub fn set_disc(&mut self, image: Option<Vec<u8>>) {
        self.disc = image;
        self.stat = if self.disc.is_some() { 0x02 } else { 0x00 };
    }

    fn sectors(&self) -> u32 {
        self.disc.as_ref().map(|d| (d.len() / SECTOR) as u32).unwrap_or(0)
    }

    pub fn read(&mut self, reg: u32) -> u8 {
        match reg {
            0 => {
                let mut s = self.index & 3;
                s |= (self.params.is_empty() as u8) << 3;
                s |= ((self.params.len() < 16) as u8) << 4;
                s |= (!self.response.is_empty() as u8) << 5;
                s |= (!self.data.is_empty() as u8) << 6;
                s |= (self.busy as u8) << 7;
                s
            }
            1 => self.response.pop_front().unwrap_or(0),
            2 => self.data.pop_front().unwrap_or(0),
            _ => {
                if self.index & 1 == 0 {
                    self.int_enable | 0xE0
                } else {
                    self.int_flag | 0xE0
                }
            }
        }
    }

    /// Returns true if an interrupt should be raised now.
    pub fn write(&mut self, reg: u32, v: u8) -> bool {
        match (reg, self.index) {
            (0, _) => self.index = v & 3,
            (1, 0) => self.command(v),
            (2, 0) => {
                if self.params.len() < 16 {
                    self.params.push_back(v)
                }
            }
            (3, 0) => {
                if v & 0x80 != 0 {
                    if self.data.is_empty() {
                        self.data.extend(self.sector_buf.iter().copied());
                    }
                } else {
                    self.data.clear();
                }
            }
            (2, 1) => self.int_enable = v & 0x1F,
            (3, 1) => {
                self.int_flag &= !(v & 0x1F);
                if v & 0x40 != 0 {
                    self.params.clear();
                }
                if self.int_flag == 0 {
                    self.response.clear();
                }
            }
            _ => {} // audio volume / sound map registers
        }
        false
    }

    pub fn dma_read(&mut self) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (self.data.pop_front().unwrap_or(0) as u32) << (8 * i);
        }
        v
    }

    fn push(&mut self, delay: u32, int: u8, data: Vec<u8>) {
        self.queue.push_back(Pending { delay, int, data });
    }

    fn ack(&mut self) {
        let s = self.stat;
        self.push(FIRST_RESPONSE, 3, vec![s]);
    }

    fn error(&mut self, code: u8) {
        let s = self.stat | 1;
        self.push(FIRST_RESPONSE, 5, vec![s, code]);
    }

    fn command(&mut self, cmd: u8) {
        let p: Vec<u8> = self.params.drain(..).collect();
        self.busy = true;
        let has_disc = self.disc.is_some();
        match cmd {
            0x01 => {
                self.ack();
                // Shell-open flag clears once read.
                self.stat &= !0x10;
            }
            0x02 => {
                if p.len() >= 3 {
                    let lba = (from_bcd(p[0]) * 60 + from_bcd(p[1])) * 75 + from_bcd(p[2]);
                    self.setloc = lba.saturating_sub(150);
                }
                self.ack();
            }
            0x03 => {
                self.stat |= 0x80;
                self.ack();
            }
            0x06 | 0x1B => {
                if !has_disc {
                    return self.error(0x80);
                }
                self.ack();
                self.pos = self.setloc;
                self.reading = true;
                self.stat = (self.stat | 0x22) & !0xC0;
                self.read_timer = self.sector_time() + FIRST_RESPONSE;
            }
            0x07 => {
                self.stat |= 0x02;
                self.ack();
                let s = self.stat;
                self.push(SECOND_RESPONSE, 2, vec![s]);
            }
            0x08 => {
                self.reading = false;
                self.stat &= !0xE2;
                self.ack();
                let s = self.stat;
                self.push(SECOND_RESPONSE, 2, vec![s]);
            }
            0x09 => {
                self.ack();
                self.reading = false;
                self.stat &= !0xE0;
                let s = self.stat;
                self.push(if has_disc { SECOND_RESPONSE } else { 9000 }, 2, vec![s]);
            }
            0x0A => {
                self.ack();
                self.mode = 0;
                self.reading = false;
                self.stat = if has_disc { 0x02 } else { 0x00 };
                let s = self.stat;
                self.push(SECOND_RESPONSE, 2, vec![s]);
            }
            0x0B | 0x0C | 0x0D => self.ack(),
            0x0E => {
                if let Some(&m) = p.first() {
                    self.mode = m;
                }
                self.ack();
            }
            0x0F => {
                let s = self.stat;
                let m = self.mode;
                self.push(FIRST_RESPONSE, 3, vec![s, m, 0, 0, 0]);
            }
            0x10 => {
                let h = self.last_header;
                self.push(FIRST_RESPONSE, 3, h.to_vec());
            }
            0x11 => {
                let lba = self.pos + 150;
                let (m, s, f) = (lba / 75 / 60, (lba / 75) % 60, lba % 75);
                let rel = self.pos;
                let (rm, rs, rf) = (rel / 75 / 60, (rel / 75) % 60, rel % 75);
                self.push(FIRST_RESPONSE, 3, vec![1, 1, bcd(rm), bcd(rs), bcd(rf), bcd(m), bcd(s), bcd(f)]);
            }
            0x12 => {
                self.ack();
                let s = self.stat;
                self.push(SECOND_RESPONSE, 2, vec![s]);
            }
            0x13 => {
                if !has_disc {
                    return self.error(0x80);
                }
                let s = self.stat;
                self.push(FIRST_RESPONSE, 3, vec![s, 1, 1]);
            }
            0x14 => {
                if !has_disc {
                    return self.error(0x80);
                }
                let t = p.first().copied().unwrap_or(0);
                let lba = if t == 0 { self.sectors() + 150 } else { 150 };
                let s = self.stat;
                self.push(FIRST_RESPONSE, 3, vec![s, bcd(lba / 75 / 60), bcd((lba / 75) % 60)]);
            }
            0x15 | 0x16 => {
                if !has_disc {
                    return self.error(0x80);
                }
                self.reading = false;
                self.pos = self.setloc;
                self.stat = (self.stat | 0x42) & !0xA0;
                self.ack();
                self.stat &= !0x40;
                let s = self.stat;
                self.push(SECOND_RESPONSE, 2, vec![s]);
            }
            0x19 => {
                match p.first().copied().unwrap_or(0) {
                    // Controller BIOS date/version.
                    0x20 => self.push(FIRST_RESPONSE, 3, vec![0x94, 0x09, 0x19, 0xC0]),
                    _ => self.ack(),
                }
            }
            0x1A => {
                if !has_disc {
                    self.ack();
                    self.push(SECOND_RESPONSE, 5, vec![0x08, 0x40, 0, 0, 0, 0, 0, 0]);
                } else {
                    self.ack();
                    let s = self.stat;
                    let r = self.region;
                    let region = match r { b'A' => b'A', b'I' => b'I', _ => b'E' };
                    self.push(SECOND_RESPONSE, 2, vec![s, 0x00, 0x20, 0x00, b'S', b'C', b'E', region]);
                }
            }
            0x1E => {
                if !has_disc {
                    return self.error(0x80);
                }
                self.ack();
                let s = self.stat;
                self.push(SECOND_RESPONSE * 4, 2, vec![s]);
            }
            _ => {
                let s = self.stat | 1;
                self.push(FIRST_RESPONSE, 5, vec![s, 0x40]);
            }
        }
    }

    fn sector_time(&self) -> u32 {
        if self.mode & 0x80 != 0 { CPU_HZ / 150 } else { CPU_HZ / 75 }
    }

    fn load_sector(&mut self) {
        let Some(disc) = self.disc.as_ref() else { return };
        let off = self.pos as usize * SECTOR;
        if off + SECTOR > disc.len() {
            self.sector_buf = vec![0; if self.mode & 0x20 != 0 { 0x924 } else { 0x800 }];
            return;
        }
        let raw = &disc[off..off + SECTOR];
        self.last_header.copy_from_slice(&raw[12..20]);
        self.sector_buf = if self.mode & 0x20 != 0 { raw[12..12 + 0x924].to_vec() } else { raw[24..24 + 0x800].to_vec() };
    }

    /// Returns true when a new interrupt becomes pending.
    pub fn tick(&mut self, cycles: u32) -> bool {
        let mut raised = false;

        // Sector delivery while reading (only when no other INT is pending).
        if self.reading {
            if self.read_timer > cycles {
                self.read_timer -= cycles;
            } else {
                self.read_timer = self.sector_time();
                if self.int_flag == 0 {
                    self.load_sector();
                    self.pos += 1;
                    self.int_flag = 1;
                    self.response.clear();
                    self.response.push_back(self.stat);
                    raised = self.int_flag & self.int_enable != 0;
                }
            }
        }

        if let Some(front) = self.queue.front_mut() {
            front.delay = front.delay.saturating_sub(cycles);
            if front.delay == 0 && self.int_flag == 0 {
                let p = self.queue.pop_front().unwrap();
                self.int_flag = p.int;
                self.response.clear();
                self.response.extend(p.data);
                self.busy = false;
                raised |= self.int_flag & self.int_enable != 0;
            }
        } else {
            self.busy = false;
        }
        raised
    }
}
