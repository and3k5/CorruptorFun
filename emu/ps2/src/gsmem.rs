//! GS local memory layout: 4 MB of 32-bit words, organised in 8 KB pages
//! of 32 blocks. Each pixel format (PSM) arranges its pixels differently
//! inside a page, block and column; these tables come from the GS manual.

pub const PSMCT32: u32 = 0x00;
pub const PSMCT24: u32 = 0x01;
pub const PSMCT16: u32 = 0x02;
pub const PSMCT16S: u32 = 0x0A;
pub const PSMT8: u32 = 0x13;
pub const PSMT4: u32 = 0x14;
pub const PSMT8H: u32 = 0x1B;
pub const PSMT4HL: u32 = 0x24;
pub const PSMT4HH: u32 = 0x2C;
pub const PSMZ32: u32 = 0x30;
pub const PSMZ24: u32 = 0x31;
pub const PSMZ16: u32 = 0x32;
pub const PSMZ16S: u32 = 0x3A;

const BLOCK32: [[u32; 8]; 4] = [
    [0, 1, 4, 5, 16, 17, 20, 21],
    [2, 3, 6, 7, 18, 19, 22, 23],
    [8, 9, 12, 13, 24, 25, 28, 29],
    [10, 11, 14, 15, 26, 27, 30, 31],
];
const BLOCKZ32: [[u32; 8]; 4] = [
    [24, 25, 28, 29, 8, 9, 12, 13],
    [26, 27, 30, 31, 10, 11, 14, 15],
    [16, 17, 20, 21, 0, 1, 4, 5],
    [18, 19, 22, 23, 2, 3, 6, 7],
];
const COLUMN32: [[u32; 8]; 8] = [
    [0, 1, 4, 5, 8, 9, 12, 13],
    [2, 3, 6, 7, 10, 11, 14, 15],
    [16, 17, 20, 21, 24, 25, 28, 29],
    [18, 19, 22, 23, 26, 27, 30, 31],
    [32, 33, 36, 37, 40, 41, 44, 45],
    [34, 35, 38, 39, 42, 43, 46, 47],
    [48, 49, 52, 53, 56, 57, 60, 61],
    [50, 51, 54, 55, 58, 59, 62, 63],
];
const BLOCK16: [[u32; 4]; 8] = [
    [0, 2, 8, 10],
    [1, 3, 9, 11],
    [4, 6, 12, 14],
    [5, 7, 13, 15],
    [16, 18, 24, 26],
    [17, 19, 25, 27],
    [20, 22, 28, 30],
    [21, 23, 29, 31],
];
const BLOCK16S: [[u32; 4]; 8] = [
    [0, 2, 16, 18],
    [1, 3, 17, 19],
    [8, 10, 24, 26],
    [9, 11, 25, 27],
    [4, 6, 20, 22],
    [5, 7, 21, 23],
    [12, 14, 28, 30],
    [13, 15, 29, 31],
];
const BLOCKZ16: [[u32; 4]; 8] = [
    [24, 26, 16, 18],
    [25, 27, 17, 19],
    [28, 30, 20, 22],
    [29, 31, 21, 23],
    [8, 10, 0, 2],
    [9, 11, 1, 3],
    [12, 14, 4, 6],
    [13, 15, 5, 7],
];
const BLOCKZ16S: [[u32; 4]; 8] = [
    [24, 26, 8, 10],
    [25, 27, 9, 11],
    [16, 18, 0, 2],
    [17, 19, 1, 3],
    [28, 30, 12, 14],
    [29, 31, 13, 15],
    [20, 22, 4, 6],
    [21, 23, 5, 7],
];
const COLUMN16: [[u32; 16]; 8] = [
    [0, 2, 8, 10, 16, 18, 24, 26, 1, 3, 9, 11, 17, 19, 25, 27],
    [4, 6, 12, 14, 20, 22, 28, 30, 5, 7, 13, 15, 21, 23, 29, 31],
    [32, 34, 40, 42, 48, 50, 56, 58, 33, 35, 41, 43, 49, 51, 57, 59],
    [36, 38, 44, 46, 52, 54, 60, 62, 37, 39, 45, 47, 53, 55, 61, 63],
    [64, 66, 72, 74, 80, 82, 88, 90, 65, 67, 73, 75, 81, 83, 89, 91],
    [68, 70, 76, 78, 84, 86, 92, 94, 69, 71, 77, 79, 85, 87, 93, 95],
    [96, 98, 104, 106, 112, 114, 120, 122, 97, 99, 105, 107, 113, 115, 121, 123],
    [100, 102, 108, 110, 116, 118, 124, 126, 101, 103, 109, 111, 117, 119, 125, 127],
];
const BLOCK8: [[u32; 8]; 4] = BLOCK32;
const COLUMN8: [[u32; 16]; 16] = [
    [0, 4, 16, 20, 32, 36, 48, 52, 2, 6, 18, 22, 34, 38, 50, 54],
    [8, 12, 24, 28, 40, 44, 56, 60, 10, 14, 26, 30, 42, 46, 58, 62],
    [33, 37, 49, 53, 1, 5, 17, 21, 35, 39, 51, 55, 3, 7, 19, 23],
    [41, 45, 57, 61, 9, 13, 25, 29, 43, 47, 59, 63, 11, 15, 27, 31],
    [96, 100, 112, 116, 64, 68, 80, 84, 98, 102, 114, 118, 66, 70, 82, 86],
    [104, 108, 120, 124, 72, 76, 88, 92, 106, 110, 122, 126, 74, 78, 90, 94],
    [65, 69, 81, 85, 97, 101, 113, 117, 67, 71, 83, 87, 99, 103, 115, 119],
    [73, 77, 89, 93, 105, 109, 121, 125, 75, 79, 91, 95, 107, 111, 123, 127],
    [128, 132, 144, 148, 160, 164, 176, 180, 130, 134, 146, 150, 162, 166, 178, 182],
    [136, 140, 152, 156, 168, 172, 184, 188, 138, 142, 154, 158, 170, 174, 186, 190],
    [161, 165, 177, 181, 129, 133, 145, 149, 163, 167, 179, 183, 131, 135, 147, 151],
    [169, 173, 185, 189, 137, 141, 153, 157, 171, 175, 187, 191, 139, 143, 155, 159],
    [224, 228, 240, 244, 192, 196, 208, 212, 226, 230, 242, 246, 194, 198, 210, 214],
    [232, 236, 248, 252, 200, 204, 216, 220, 234, 238, 250, 254, 202, 206, 218, 222],
    [193, 197, 209, 213, 225, 229, 241, 245, 195, 199, 211, 215, 227, 231, 243, 247],
    [201, 205, 217, 221, 233, 237, 249, 253, 203, 207, 219, 223, 235, 239, 251, 255],
];
const BLOCK4: [[u32; 4]; 8] = BLOCK16;
const COLUMN4: [[u32; 32]; 16] = [
    [0, 8, 32, 40, 64, 72, 96, 104, 2, 10, 34, 42, 66, 74, 98, 106, 4, 12, 36, 44, 68, 76, 100, 108, 6, 14, 38, 46, 70, 78, 102, 110],
    [16, 24, 48, 56, 80, 88, 112, 120, 18, 26, 50, 58, 82, 90, 114, 122, 20, 28, 52, 60, 84, 92, 116, 124, 22, 30, 54, 62, 86, 94, 118, 126],
    [65, 73, 97, 105, 1, 9, 33, 41, 67, 75, 99, 107, 3, 11, 35, 43, 69, 77, 101, 109, 5, 13, 37, 45, 71, 79, 103, 111, 7, 15, 39, 47],
    [81, 89, 113, 121, 17, 25, 49, 57, 83, 91, 115, 123, 19, 27, 51, 59, 85, 93, 117, 125, 21, 29, 53, 61, 87, 95, 119, 127, 23, 31, 55, 63],
    [192, 200, 224, 232, 128, 136, 160, 168, 194, 202, 226, 234, 130, 138, 162, 170, 196, 204, 228, 236, 132, 140, 164, 172, 198, 206, 230, 238, 134, 142, 166, 174],
    [208, 216, 240, 248, 144, 152, 176, 184, 210, 218, 242, 250, 146, 154, 178, 186, 212, 220, 244, 252, 148, 156, 180, 188, 214, 222, 246, 254, 150, 158, 182, 190],
    [129, 137, 161, 169, 193, 201, 225, 233, 131, 139, 163, 171, 195, 203, 227, 235, 133, 141, 165, 173, 197, 205, 229, 237, 135, 143, 167, 175, 199, 207, 231, 239],
    [145, 153, 177, 185, 209, 217, 241, 249, 147, 155, 179, 187, 211, 219, 243, 251, 149, 157, 181, 189, 213, 221, 245, 253, 151, 159, 183, 191, 215, 223, 247, 255],
    [256, 264, 288, 296, 320, 328, 352, 360, 258, 266, 290, 298, 322, 330, 354, 362, 260, 268, 292, 300, 324, 332, 356, 364, 262, 270, 294, 302, 326, 334, 358, 366],
    [272, 280, 304, 312, 336, 344, 368, 376, 274, 282, 306, 314, 338, 346, 370, 378, 276, 284, 308, 316, 340, 348, 372, 380, 278, 286, 310, 318, 342, 350, 374, 382],
    [321, 329, 353, 361, 257, 265, 289, 297, 323, 331, 355, 363, 259, 267, 291, 299, 325, 333, 357, 365, 261, 269, 293, 301, 327, 335, 359, 367, 263, 271, 295, 303],
    [337, 345, 369, 377, 273, 281, 305, 313, 339, 347, 371, 379, 275, 283, 307, 315, 341, 349, 373, 381, 277, 285, 309, 317, 343, 351, 375, 383, 279, 287, 311, 319],
    [448, 456, 480, 488, 384, 392, 416, 424, 450, 458, 482, 490, 386, 394, 418, 426, 452, 460, 484, 492, 388, 396, 420, 428, 454, 462, 486, 494, 390, 398, 422, 430],
    [464, 472, 496, 504, 400, 408, 432, 440, 466, 474, 498, 506, 402, 410, 434, 442, 468, 476, 500, 508, 404, 412, 436, 444, 470, 478, 502, 510, 406, 414, 438, 446],
    [385, 393, 417, 425, 449, 457, 481, 489, 387, 395, 419, 427, 451, 459, 483, 491, 389, 397, 421, 429, 453, 461, 485, 493, 391, 399, 423, 431, 455, 463, 487, 495],
    [401, 409, 433, 441, 465, 473, 497, 505, 403, 411, 435, 443, 467, 475, 499, 507, 405, 413, 437, 445, 469, 477, 501, 509, 407, 415, 439, 447, 471, 479, 503, 511],
];

const WORDS: usize = 1 << 20; // 4 MB

/// Word address of a 32-bit pixel. `bp` in blocks (64 words), `bw` in
/// 64-pixel units.
#[inline(always)]
pub fn addr32(bp: u32, bw: u32, x: u32, y: u32, z: bool) -> usize {
    let page = (x >> 6) + (y >> 5) * bw.max(1);
    let bt = if z { &BLOCKZ32 } else { &BLOCK32 };
    let block = bp + page * 32 + bt[((y >> 3) & 3) as usize][((x >> 3) & 7) as usize];
    ((block * 64 + COLUMN32[(y & 7) as usize][(x & 7) as usize]) as usize) & (WORDS - 1)
}

/// Halfword address of a 16-bit pixel.
#[inline(always)]
pub fn addr16(psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (x >> 6) + (y >> 6) * bw.max(1);
    let bt = match psm {
        PSMCT16S => &BLOCK16S,
        PSMZ16 => &BLOCKZ16,
        PSMZ16S => &BLOCKZ16S,
        _ => &BLOCK16,
    };
    let block = bp + page * 32 + bt[((y >> 3) & 7) as usize][((x >> 4) & 3) as usize];
    ((block * 128 + COLUMN16[(y & 7) as usize][(x & 15) as usize]) as usize) & (WORDS * 2 - 1)
}

/// Byte address of an 8-bit pixel.
#[inline(always)]
pub fn addr8(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (x >> 7) + (y >> 6) * (bw >> 1).max(1);
    let block = bp + page * 32 + BLOCK8[((y >> 4) & 3) as usize][((x >> 4) & 7) as usize];
    ((block * 256 + COLUMN8[(y & 15) as usize][(x & 15) as usize]) as usize) & (WORDS * 4 - 1)
}

/// Nibble address of a 4-bit pixel.
#[inline(always)]
pub fn addr4(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (x >> 7) + (y >> 7) * (bw >> 1).max(1);
    let block = bp + page * 32 + BLOCK4[((y >> 4) & 7) as usize][((x >> 5) & 3) as usize];
    ((block * 512 + COLUMN4[(y & 15) as usize][(x & 31) as usize]) as usize) & (WORDS * 8 - 1)
}

/// Reads a pixel's raw value in format `psm`.
#[inline]
pub fn read(mem: &[u32], psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    match psm {
        PSMCT32 | PSMCT24 | PSMT8H | PSMT4HL | PSMT4HH => {
            let w = mem[addr32(bp, bw, x, y, false)];
            match psm {
                PSMCT24 => w & 0xFF_FFFF,
                PSMT8H => w >> 24,
                PSMT4HL => (w >> 24) & 0xF,
                PSMT4HH => w >> 28,
                _ => w,
            }
        }
        PSMZ32 => mem[addr32(bp, bw, x, y, true)],
        PSMZ24 => mem[addr32(bp, bw, x, y, true)] & 0xFF_FFFF,
        PSMCT16 | PSMCT16S | PSMZ16 | PSMZ16S => {
            let a = addr16(psm, bp, bw, x, y);
            (mem[a >> 1] >> ((a & 1) * 16)) & 0xFFFF
        }
        PSMT8 => {
            let a = addr8(bp, bw, x, y);
            (mem[a >> 2] >> ((a & 3) * 8)) & 0xFF
        }
        PSMT4 => {
            let a = addr4(bp, bw, x, y);
            (mem[a >> 3] >> ((a & 7) * 4)) & 0xF
        }
        _ => mem[addr32(bp, bw, x, y, false)],
    }
}

/// Writes a raw pixel value in format `psm`, keeping bits outside it.
#[inline]
pub fn write(mem: &mut [u32], psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
    match psm {
        PSMCT32 => mem[addr32(bp, bw, x, y, false)] = v,
        PSMZ32 => mem[addr32(bp, bw, x, y, true)] = v,
        PSMCT24 | PSMZ24 => {
            let a = addr32(bp, bw, x, y, psm == PSMZ24);
            mem[a] = (mem[a] & 0xFF00_0000) | (v & 0xFF_FFFF);
        }
        PSMT8H => {
            let a = addr32(bp, bw, x, y, false);
            mem[a] = (mem[a] & 0x00FF_FFFF) | (v << 24);
        }
        PSMT4HL => {
            let a = addr32(bp, bw, x, y, false);
            mem[a] = (mem[a] & !0x0F00_0000) | ((v & 0xF) << 24);
        }
        PSMT4HH => {
            let a = addr32(bp, bw, x, y, false);
            mem[a] = (mem[a] & !0xF000_0000) | ((v & 0xF) << 28);
        }
        PSMCT16 | PSMCT16S | PSMZ16 | PSMZ16S => {
            let a = addr16(psm, bp, bw, x, y);
            let sh = (a & 1) * 16;
            let w = &mut mem[a >> 1];
            *w = (*w & !(0xFFFF << sh)) | ((v & 0xFFFF) << sh);
        }
        PSMT8 => {
            let a = addr8(bp, bw, x, y);
            let sh = (a & 3) * 8;
            let w = &mut mem[a >> 2];
            *w = (*w & !(0xFF << sh)) | ((v & 0xFF) << sh);
        }
        PSMT4 => {
            let a = addr4(bp, bw, x, y);
            let sh = (a & 7) * 4;
            let w = &mut mem[a >> 3];
            *w = (*w & !(0xF << sh)) | ((v & 0xF) << sh);
        }
        _ => mem[addr32(bp, bw, x, y, false)] = v,
    }
}

/// Where a pixel lives: (word index, bit shift, value mask).
#[inline]
pub fn locate(psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> (usize, u32, u32) {
    match psm {
        PSMCT32 => (addr32(bp, bw, x, y, false), 0, 0xFFFF_FFFF),
        PSMZ32 => (addr32(bp, bw, x, y, true), 0, 0xFFFF_FFFF),
        PSMCT24 => (addr32(bp, bw, x, y, false), 0, 0xFF_FFFF),
        PSMZ24 => (addr32(bp, bw, x, y, true), 0, 0xFF_FFFF),
        PSMT8H => (addr32(bp, bw, x, y, false), 24, 0xFF),
        PSMT4HL => (addr32(bp, bw, x, y, false), 24, 0xF),
        PSMT4HH => (addr32(bp, bw, x, y, false), 28, 0xF),
        PSMCT16 | PSMCT16S | PSMZ16 | PSMZ16S => {
            let a = addr16(psm, bp, bw, x, y);
            (a >> 1, (a as u32 & 1) * 16, 0xFFFF)
        }
        PSMT8 => {
            let a = addr8(bp, bw, x, y);
            (a >> 2, (a as u32 & 3) * 8, 0xFF)
        }
        PSMT4 => {
            let a = addr4(bp, bw, x, y);
            (a >> 3, (a as u32 & 7) * 4, 0xF)
        }
        _ => (addr32(bp, bw, x, y, false), 0, 0xFFFF_FFFF),
    }
}

/// Pixel size of a page in a format: (width, height).
pub fn page_size(psm: u32) -> (u32, u32) {
    match bpp(psm) {
        16 => (64, 64),
        8 => (128, 64),
        4 => (128, 128),
        _ => (64, 32),
    }
}

/// Bits per pixel of a format.
pub fn bpp(psm: u32) -> u32 {
    match psm {
        PSMCT32 | PSMZ32 | PSMT8H | PSMT4HL | PSMT4HH => 32,
        PSMCT24 | PSMZ24 => 24,
        PSMCT16 | PSMCT16S | PSMZ16 | PSMZ16S => 16,
        PSMT8 => 8,
        PSMT4 => 4,
        _ => 32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_perm(v: &[u32]) -> bool {
        let mut s: Vec<u32> = v.to_vec();
        s.sort();
        s.iter().enumerate().all(|(i, &x)| x == i as u32)
    }

    #[test]
    fn tables_are_permutations() {
        assert!(is_perm(&COLUMN32.concat()));
        assert!(is_perm(&COLUMN16.concat()));
        assert!(is_perm(&COLUMN8.concat()));
        assert!(is_perm(&COLUMN4.concat()));
        assert!(is_perm(&BLOCK32.concat()));
        assert!(is_perm(&BLOCKZ32.concat()));
        assert!(is_perm(&BLOCK16.concat()));
        assert!(is_perm(&BLOCK16S.concat()));
        assert!(is_perm(&BLOCKZ16.concat()));
        assert!(is_perm(&BLOCKZ16S.concat()));
    }

    #[test]
    fn page_is_fully_covered() {
        // Each format fills a whole 8 KB page with its page-sized rectangle.
        let mut seen = vec![false; 2048 * 8];
        for y in 0..128 {
            for x in 0..128 {
                let a = addr4(0, 2, x, y);
                assert!(!seen[a]);
                seen[a] = true;
            }
        }
        let mut seen = vec![false; 2048 * 4];
        for y in 0..64 {
            for x in 0..128 {
                let a = addr8(0, 2, x, y);
                assert!(!seen[a]);
                seen[a] = true;
            }
        }
    }

    #[test]
    fn write_read_round_trip() {
        let mut m = vec![0u32; WORDS];
        for (i, psm) in [PSMCT32, PSMCT16, PSMCT16S, PSMT8, PSMT4].into_iter().enumerate() {
            write(&mut m, psm, 64 * i as u32, 4, 13, 21, 0x9);
            assert_eq!(read(&m, psm, 64 * i as u32, 4, 13, 21), 0x9);
        }
    }
}
