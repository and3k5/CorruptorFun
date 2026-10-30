//! SIO0: controller and memory card port. A digital pad is attached to port
//! 1; no memory cards are inserted.

pub struct Sio {
    stat_irq: bool,
    ctrl: u16,
    mode: u16,
    baud: u16,
    rx: Option<u8>,
    tx_busy: u32,
    pending_rx: u8,
    pending_ack: bool,
    ack_timer: u32,
    /// Bytes exchanged since /SEL went low.
    step: u32,
    device: Device,
    /// Pressed buttons, active-high, in PS1 bit order.
    pub buttons: u16,
}

#[derive(PartialEq, Clone, Copy)]
enum Device {
    None,
    Pad,
    Nothing,
}

impl Sio {
    pub fn new() -> Self {
        Sio {
            stat_irq: false,
            ctrl: 0,
            mode: 0,
            baud: 0x88,
            rx: None,
            tx_busy: 0,
            pending_rx: 0xFF,
            pending_ack: false,
            ack_timer: 0,
            step: 0,
            device: Device::None,
            buttons: 0,
        }
    }

    pub fn read(&mut self, reg: u32) -> u16 {
        match reg {
            0 => self.rx.take().unwrap_or(0xFF) as u16,
            4 => {
                let mut s = 0u16;
                s |= ((self.tx_busy == 0) as u16) | (((self.tx_busy == 0) as u16) << 2);
                s |= (self.rx.is_some() as u16) << 1;
                s |= (self.stat_irq as u16) << 9;
                s
            }
            8 => self.mode,
            0xA => self.ctrl,
            0xE => self.baud,
            _ => 0,
        }
    }

    pub fn write(&mut self, reg: u32, v: u16) {
        match reg {
            0 => self.transfer(v as u8),
            8 => self.mode = v,
            0xA => {
                if v & 0x40 != 0 {
                    // Reset
                    self.ctrl = 0;
                    self.stat_irq = false;
                    self.rx = None;
                    self.step = 0;
                    self.device = Device::None;
                    return;
                }
                if v & 0x10 != 0 {
                    self.stat_irq = false;
                }
                let was_selected = self.ctrl & 2 != 0;
                self.ctrl = v & !0x50;
                if !was_selected || v & 2 == 0 {
                    self.step = 0;
                    self.device = Device::None;
                }
            }
            0xE => self.baud = v,
            _ => {}
        }
    }

    fn transfer(&mut self, b: u8) {
        let selected = self.ctrl & 2 != 0;
        let port2 = self.ctrl & 0x2000 != 0;
        let (resp, ack) = if !selected {
            (0xFF, false)
        } else {
            if self.step == 0 {
                self.device = match (b, port2) {
                    (0x01, false) => Device::Pad,
                    _ => Device::Nothing,
                };
            }
            let r = match self.device {
                Device::Pad => match self.step {
                    0 => (0xFF, true),
                    1 => (0x41, b == 0x42),
                    2 => (0x5A, true),
                    3 => (!(self.buttons as u8), true),
                    4 => (!((self.buttons >> 8) as u8), false),
                    _ => (0xFF, false),
                },
                _ => (0xFF, false),
            };
            self.step += 1;
            if !r.1 {
                self.device = Device::Nothing;
            }
            r
        };
        self.pending_rx = resp;
        self.pending_ack = ack;
        self.tx_busy = (self.baud as u32).max(1) * 8;
    }

    /// Returns true when the ACK interrupt fires.
    pub fn tick(&mut self, cycles: u32) -> bool {
        if self.tx_busy > 0 {
            if self.tx_busy > cycles {
                self.tx_busy -= cycles;
            } else {
                self.tx_busy = 0;
                self.rx = Some(self.pending_rx);
                if self.pending_ack {
                    self.ack_timer = 120;
                }
            }
            return false;
        }
        if self.ack_timer > 0 {
            if self.ack_timer > cycles {
                self.ack_timer -= cycles;
            } else {
                self.ack_timer = 0;
                if self.ctrl & 0x1000 != 0 {
                    self.stat_irq = true;
                    return true;
                }
            }
        }
        false
    }
}
