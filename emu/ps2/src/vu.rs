//! Vector units. VU0 macro instructions (COP2 from the EE) and VU1
//! microprograms are not emulated yet; we count them for the report.

use crate::ee::Ee;
use crate::hw::Hw;

pub fn macro_op(ee: &mut Ee, i: u32) {
    let f = i & 0x3F;
    let name = if f >= 0x3C { format!("vu0 special2 {:03X}", (i & 3) | ((i >> 4) & 0x7C)) } else { format!("vu0 macro {f:02X}") };
    *ee.unimpl.entry(name).or_insert(0) += 1;
}

pub fn start(hw: &mut Hw, which: usize, addr: Option<u32>) {
    let _ = addr;
    if which == 1 {
        hw.vu1_runs += 1;
    } else {
        hw.vu0_runs += 1;
    }
}
