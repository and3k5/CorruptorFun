//! The PS2 BIOS keeps its files (kernel, IOP modules, OSDSYS, fonts…) in a
//! simple table called ROMDIR. Each 16-byte entry is a 10-byte name, a
//! 16-bit EXTINFO size and a 32-bit file size; the files follow each other
//! from the start of the ROM, each aligned to 16 bytes. The table starts with
//! the entry for RESET, the file at offset 0.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomFile {
    pub name: String,
    pub offset: u32,
    pub size: u32,
}

pub fn parse(rom: &[u8]) -> Vec<RomFile> {
    let start = match find_reset(rom) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut offset: u32 = 0;
    let mut at = start;
    while at + 16 <= rom.len() && rom[at] != 0 && out.len() < 512 {
        let raw = &rom[at..at + 10];
        let name: String = raw.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
        let size = u32::from_le_bytes([rom[at + 12], rom[at + 13], rom[at + 14], rom[at + 15]]);
        if !name.chars().all(|c| c.is_ascii_graphic()) || offset as usize > rom.len() {
            break;
        }
        out.push(RomFile { name, offset, size });
        offset = offset.saturating_add(size.saturating_add(15) & !15);
        at += 16;
    }
    out
}

fn find_reset(rom: &[u8]) -> Option<usize> {
    let pat = b"RESET\0\0\0\0\0";
    let limit = rom.len().min(0x40000);
    (0..limit.saturating_sub(16)).step_by(16).find(|&i| &rom[i..i + 10] == pat)
}

/// The file called `name`, if the ROM has one.
pub fn find<'a>(files: &'a [RomFile], name: &str) -> Option<&'a RomFile> {
    files.iter().find(|f| f.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u32) -> [u8; 16] {
        let mut e = [0u8; 16];
        e[..name.len()].copy_from_slice(name.as_bytes());
        e[12..16].copy_from_slice(&size.to_le_bytes());
        e
    }

    #[test]
    fn parses_a_small_table() {
        let mut rom = vec![0u8; 0x8000];
        let table = 0x2700;
        let entries = [entry("RESET", 0x2700), entry("ROMDIR", 0x40), entry("EXTINFO", 0x13), entry("OSDSYS", 0x100)];
        for (i, e) in entries.iter().enumerate() {
            rom[table + i * 16..table + i * 16 + 16].copy_from_slice(e);
        }
        let files = parse(&rom);
        assert_eq!(files.len(), 4);
        assert_eq!(files[1], RomFile { name: "ROMDIR".into(), offset: 0x2700, size: 0x40 });
        assert_eq!(files[2].offset, 0x2740);
        assert_eq!(files[3].offset, 0x2760);
        assert_eq!(find(&files, "OSDSYS").unwrap().size, 0x100);
    }
}
