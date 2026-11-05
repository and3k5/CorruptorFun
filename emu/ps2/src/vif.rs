//! VIF0 / VIF1: unpack DMA data into VU memory, upload microprograms, start
//! them, and (VIF1) pass GIF packets through to the GS (PATH2).

use crate::hw::Hw;

#[derive(Default, Clone)]
pub struct Vif {
    pub words: std::collections::VecDeque<u32>,
    pub cl: u32,
    pub wl: u32,
    pub mode: u32,
    pub mask: u32,
    pub row: [u32; 4],
    pub col: [u32; 4],
    pub itop: u32,
    pub itops: u32,
    pub base: u32,
    pub ofst: u32,
    pub top: u32,
    pub tops: u32,
    pub dbf: bool,
    pub mark: u32,
    pub mpg_count: u32,
    pub unpack_count: u32,
    pub direct_qwords: u32,
    pub mscal_count: u32,
    pub unknown: u32,
}

pub fn process(hw: &mut Hw, which: usize) {
    let fifo = if which == 1 { std::mem::take(&mut hw.dmac.vif1_fifo) } else { std::mem::take(&mut hw.dmac.vif0_fifo) };
    {
        let v = vif_mut(hw, which);
        for q in fifo {
            for k in 0..4 {
                v.words.push_back((q >> (k * 32)) as u32);
            }
        }
    }
    loop {
        let v = vif_mut(hw, which);
        let Some(&code) = v.words.front() else { break };
        let cmd = (code >> 24) & 0x7F;
        let imm = code & 0xFFFF;
        let num = (code >> 16) & 0xFF;
        let need = 1 + match cmd {
            0x20 => 1,
            0x30 | 0x31 => 4,
            0x4A => (if num == 0 { 256 } else { num }) * 2,
            0x50 | 0x51 => (if imm == 0 { 65536 } else { imm }) * 4,
            0x60..=0x7F => unpack_words(v, cmd, num),
            _ => 0,
        } as usize;
        if cmd == 0x50 || cmd == 0x51 {
            // DIRECT data starts on the next quadword boundary in practice;
            // the code itself sits in the last word before it.
        }
        if v.words.len() < need {
            break;
        }
        let w: Vec<u32> = v.words.drain(..need).collect();
        execute(hw, which, code, &w[1..]);
    }
}

fn vif_mut(hw: &mut Hw, which: usize) -> &mut Vif {
    if which == 1 { &mut hw.vif1 } else { &mut hw.vif0 }
}

fn unpack_words(v: &Vif, cmd: u32, num: u32) -> u32 {
    let num = if num == 0 { 256 } else { num };
    let vn = (cmd >> 2) & 3;
    let vl = cmd & 3;
    let bits = if vl == 3 { 16 } else { (32 >> vl) * (vn + 1) };
    let cl = v.cl.max(1);
    let wl = v.wl.max(1);
    let n = if wl <= cl { num } else { cl * (num / wl) + (num % wl).min(cl) };
    (n * bits).div_ceil(32)
}

fn execute(hw: &mut Hw, which: usize, code: u32, data: &[u32]) {
    let cmd = (code >> 24) & 0x7F;
    let imm = code & 0xFFFF;
    let num = (code >> 16) & 0xFF;
    match cmd {
        0x00 => {}
        0x01 => {
            let v = vif_mut(hw, which);
            v.cl = imm & 0xFF;
            v.wl = (imm >> 8) & 0xFF;
        }
        0x02 => {
            let v = vif_mut(hw, which);
            v.ofst = imm & 0x3FF;
            v.dbf = false;
            v.tops = v.base;
        }
        0x03 => vif_mut(hw, which).base = imm & 0x3FF,
        0x04 => vif_mut(hw, which).itops = imm & 0x3FF,
        0x05 => vif_mut(hw, which).mode = imm & 3,
        0x06 => {}
        0x07 => vif_mut(hw, which).mark = imm,
        0x10 | 0x11 | 0x13 => {}
        0x14 | 0x15 | 0x17 => {
            let addr = if cmd == 0x17 { None } else { Some(imm * 8) };
            let v = vif_mut(hw, which);
            v.mscal_count += 1;
            v.itop = v.itops;
            if which == 1 {
                v.top = v.tops;
                v.tops = v.base + if v.dbf { v.ofst } else { 0 };
                v.dbf = !v.dbf;
            }
            crate::vu::start(hw, which, addr);
        }
        0x20 => vif_mut(hw, which).mask = data[0],
        0x30 => vif_mut(hw, which).row.copy_from_slice(&data[..4]),
        0x31 => vif_mut(hw, which).col.copy_from_slice(&data[..4]),
        0x4A => {
            let n = if num == 0 { 256 } else { num } as usize;
            let mem = if which == 1 { &mut hw.vu1_code } else { &mut hw.vu0_code };
            let size = mem.len();
            for i in 0..n * 2 {
                let a = ((imm as usize * 8) + i * 4) & (size - 1);
                mem[a..a + 4].copy_from_slice(&data[i].to_le_bytes());
            }
            vif_mut(hw, which).mpg_count += 1;
        }
        0x50 | 0x51 => {
            let qn = data.len() / 4;
            for i in 0..qn {
                let q = data[i * 4] as u128
                    | (data[i * 4 + 1] as u128) << 32
                    | (data[i * 4 + 2] as u128) << 64
                    | (data[i * 4 + 3] as u128) << 96;
                hw.gs.gif_path_write(2, q);
            }
            vif_mut(hw, which).direct_qwords += qn as u32;
        }
        0x60..=0x7F => unpack(hw, which, code, data),
        _ => vif_mut(hw, which).unknown += 1,
    }
}

fn unpack(hw: &mut Hw, which: usize, code: u32, data: &[u32]) {
    let cmd = (code >> 24) & 0x7F;
    let imm = code & 0xFFFF;
    let num = { let n = (code >> 16) & 0xFF; if n == 0 { 256 } else { n } };
    let vn = (cmd >> 2) & 3;
    let vl = cmd & 3;
    let usn = imm & 0x4000 != 0;
    let flg = imm & 0x8000 != 0;
    let (cl, wl, mode, mut row, tops) = {
        let v = vif_mut(hw, which);
        v.unpack_count += 1;
        (v.cl.max(1), v.wl.max(1), v.mode, v.row, v.tops)
    };
    let mut addr = (imm & 0x3FF) + if flg && which == 1 { tops } else { 0 };
    let mem_mask = if which == 1 { 0x3FF } else { 0xFF };
    // Bit reader over the packed input.
    let mut bitpos = 0usize;
    let mut take = |bits: usize| -> u32 {
        let mut v: u64 = 0;
        for b in 0..bits {
            let p = bitpos + b;
            let w = *data.get(p / 32).unwrap_or(&0) as u64;
            v |= ((w >> (p % 32)) & 1) << b;
        }
        bitpos += bits;
        v as u32
    };
    let ext = |v: u32, bits: u32| -> u32 {
        if usn || bits == 32 { v } else { ((v << (32 - bits)) as i32 >> (32 - bits)) as u32 }
    };
    let mut cycle = 0;
    for _ in 0..num {
        let use_data = wl <= cl || cycle < cl;
        let mut out = [0u32; 4];
        if use_data {
            if vl == 3 {
                let c = take(16);
                out = [(c & 0x1F) << 3, ((c >> 5) & 0x1F) << 3, ((c >> 10) & 0x1F) << 3, ((c >> 15) & 1) << 7];
            } else {
                let bits = 32 >> vl;
                let n = vn + 1;
                let mut comp = [0u32; 4];
                for c in comp.iter_mut().take(n as usize) {
                    *c = ext(take(bits as usize), bits);
                }
                out = match vn {
                    0 => [comp[0]; 4],
                    1 => [comp[0], comp[1], comp[0], comp[1]],
                    2 => [comp[0], comp[1], comp[2], 0],
                    _ => comp,
                };
                if vn == 2 {
                    // W is left as it was in memory.
                    out[3] = read_vu_word(hw, which, (addr & mem_mask) * 16 + 12);
                }
                if mode == 1 {
                    for k in 0..4 {
                        out[k] = out[k].wrapping_add(row[k]);
                    }
                } else if mode == 2 {
                    for k in 0..4 {
                        out[k] = out[k].wrapping_add(row[k]);
                        row[k] = out[k];
                    }
                }
            }
        } else {
            out = row;
        }
        for k in 0..4 {
            write_vu_word(hw, which, (addr & mem_mask) * 16 + k as u32 * 4, out[k]);
        }
        cycle += 1;
        addr += 1;
        if cycle >= wl.max(cl) || (wl <= cl && cycle >= wl) {
            if wl < cl {
                addr += cl - wl;
            }
            cycle = 0;
        }
    }
    if mode == 2 {
        vif_mut(hw, which).row = row;
    }
}

fn read_vu_word(hw: &Hw, which: usize, a: u32) -> u32 {
    let m = if which == 1 { &hw.vu1_data } else { &hw.vu0_data };
    let a = a as usize & (m.len() - 1);
    u32::from_le_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]])
}

fn write_vu_word(hw: &mut Hw, which: usize, a: u32, v: u32) {
    let m = if which == 1 { &mut hw.vu1_data } else { &mut hw.vu0_data };
    let a = a as usize & (m.len() - 1);
    m[a..a + 4].copy_from_slice(&v.to_le_bytes());
}
