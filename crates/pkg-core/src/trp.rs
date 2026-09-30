//! Trophy container (`trophy/trophy00.trp`) embedded in a PKG entry.
//!
//! Patches often lack `icon0.png` in the PKG entry table, but the trophy
//! container usually ships the game's `ICON0.PNG` unencrypted.

const MAGIC: u32 = 0xDCA2_4D00;
const HEADER_SIZE: usize = 0x60;
const MAX_FILES: usize = 4096;

pub struct TrpFile {
    pub name: String,
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be_u64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Parses the file table from the start of a TRP. `head` must hold at least
/// the header plus the table; returns an empty list if it does not look like
/// a TRP.
pub fn files(head: &[u8]) -> Vec<TrpFile> {
    if be_u32(head, 0) != Some(MAGIC) {
        return Vec::new();
    }
    let count = be_u32(head, 0x10).unwrap_or(0) as usize;
    let elem = be_u32(head, 0x14).unwrap_or(0) as usize;
    if count > MAX_FILES || elem < 0x30 {
        return Vec::new();
    }
    (0..count)
        .map_while(|i| {
            let at = HEADER_SIZE + i * elem;
            let e = head.get(at..at + elem)?;
            Some(TrpFile {
                name: crate::pkg::cstr(&e[..0x20]),
                offset: be_u64(e, 0x20)?,
                size: be_u64(e, 0x28)?,
                flags: be_u32(e, 0x30)?,
            })
        })
        .collect()
}

/// Bytes needed to read the header and table of a TRP with `count` files.
pub fn table_len(head: &[u8]) -> Option<usize> {
    if be_u32(head, 0)? != MAGIC {
        return None;
    }
    let count = (be_u32(head, 0x10)? as usize).min(MAX_FILES);
    let elem = be_u32(head, 0x14)? as usize;
    Some(HEADER_SIZE + count * elem)
}
