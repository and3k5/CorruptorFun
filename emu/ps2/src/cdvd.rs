//! CD/DVD drive controller (seen from the IOP). No disc yet: we answer the
//! mechanism controller's ("MechaCon") S-commands the BIOS asks at boot —
//! version, clock, config — and report an empty tray.

pub struct Cdvd {
    nstatus: u8,
    nparam: Vec<u8>,
    pub intr: u8,
    sparam: Vec<u8>,
    sresult: std::collections::VecDeque<u8>,
    scmd: u8,
    /// RTC seconds since 00:00 (advanced by the system once per second).
    pub clock_secs: u32,
    pub s_counts: std::collections::BTreeMap<u8, u32>,
    pub n_counts: std::collections::BTreeMap<u8, u32>,
    pending_irq: bool,
    /// ROM region letter (from ROMVER), e.g. b'E'.
    pub region: u8,
    pub trace: Vec<String>,
}

fn bcd(v: u32) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

impl Cdvd {
    pub fn new() -> Self {
        Cdvd {
            nstatus: 0x40,
            nparam: Vec::new(),
            intr: 0,
            sparam: Vec::new(),
            sresult: Default::default(),
            scmd: 0,
            clock_secs: 12 * 3600,
            s_counts: Default::default(),
            n_counts: Default::default(),
            pending_irq: false,
            region: b'E',
            trace: Vec::new(),
        }
    }

    pub fn read8(&mut self, p: u32) -> u8 {
        match p & 0xFF {
            0x04 => 0,
            0x05 => self.nstatus,
            0x06 => 0,
            0x08 => self.intr,
            0x0A => 0x00, // drive stopped
            0x0B => 0,    // tray closed
            0x0F => 0x00, // no disc
            0x16 => self.scmd,
            0x17 => {
                if self.sresult.is_empty() { 0x40 } else { 0x00 }
            }
            0x18 => self.sresult.pop_front().unwrap_or(0),
            0x3A => 0,
            _ => 0,
        }
    }

    /// Returns whether an IOP interrupt should be raised.
    pub fn write8(&mut self, p: u32, v: u8) -> bool {
        match p & 0xFF {
            0x04 => {
                // N-command: nothing to read, so finish with an error.
                *self.n_counts.entry(v).or_insert(0) += 1;
                self.nparam.clear();
                self.nstatus = 0x40;
                self.intr |= 0x02;
                return true;
            }
            0x05 => self.nparam.push(v),
            0x06 => {}
            0x07 => {}
            0x08 => self.intr &= !v,
            0x16 => {
                self.scmd = v;
                let n = self.s_counts.entry(v).or_insert(0);
                *n += 1;
                if *n <= 3 {
                    self.trace.push(format!("CDVD S-cmd {v:02X} params {:02X?}", self.sparam));
                }
                self.s_command(v);
                self.sparam.clear();
            }
            0x17 => self.sparam.push(v),
            _ => {}
        }
        let _ = self.pending_irq;
        false
    }

    fn s_command(&mut self, cmd: u8) {
        self.sresult.clear();
        let out: Vec<u8> = match cmd {
            0x03 => match self.sparam.first().copied().unwrap_or(0) {
                0x00 => vec![0x03, 0x06, 0x02, 0x00], // MechaCon version
                0x30 => vec![0x00, 0x00],
                _ => vec![0x00],
            },
            0x05 => vec![0x00],
            0x08 => {
                let s = self.clock_secs;
                vec![0, bcd(s % 60), bcd(s / 60 % 60), bcd(s / 3600 % 24), 0, bcd(1), bcd(11), bcd(26)]
            }
            0x09 => vec![0x00],
            0x0A => vec![0x00, 0x00, 0x00],
            0x0B => vec![0x00],
            0x15 => vec![0x05],
            0x1A => vec![0x01],
            0x1B => vec![0x00],
            0x40 | 0x42 | 0x43 => vec![0x00],
            0x41 => vec![0x00; 16],
            0x22 => vec![0x00; 10], // wake-up time: none set
            0x1E => vec![0x00, 0x14, 0x00, 0x00, 0x00], // remote control: nothing
            0x1F | 0x20 => vec![0x00, 0x14, 0x00],
            0x12 => vec![0x00; 9], // i.LINK id
            0x17 => vec![0x00; 9], // model number
            0x36 => {
                // Region parameters: encryption zone bit, then ROM region,
                // OSD region / language ("Eeng"), VERSTR region, DVD region.
                let r = self.region;
                let (zone, osd) = match r {
                    b'J' => (0, *b"Jjpn"),
                    b'A' => (1, *b"Aeng"),
                    b'E' => (2, *b"Eeng"),
                    b'H' => (4, *b"Heng"),
                    b'R' => (5, *b"Reng"),
                    b'C' => (6, *b"Csch"),
                    _ => (2, *b"Eeng"),
                };
                let mut v = vec![0u8; 15];
                v[1] = 1 << zone;
                v[3] = r;
                v[4..8].copy_from_slice(&osd);
                v[8] = r;
                v[9] = if r == b'A' { b'U' } else { r };
                v
            }
            _ => vec![0x00],
        };
        self.sresult.extend(out);
    }
}
