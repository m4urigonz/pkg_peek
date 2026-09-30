//! PS4 PKG container: header and entry table.
//!
//! Only the metadata entries are read (PARAM.SFO, icons, background, change
//! notes). They are stored in plain text inside the PKG, so no keys are needed.
//! Every offset read from the file is validated before use so a corrupt or
//! truncated file produces an error, never a panic.

use std::io::{Read, Seek, SeekFrom};

use crate::Error;

pub const MAGIC: u32 = 0x7F43_4E54; // "\x7FCNT"

const HEADER_SIZE: usize = 0x1000;
const ENTRY_SIZE: usize = 32;
const MAX_ENTRIES: u32 = 10_000;
const ENTRY_FLAG_ENCRYPTED: u32 = 0x8000_0000;

/// Well-known entry IDs (see LibOrbisPkg `EntryId`).
pub mod id {
    pub const ENTRY_NAMES: u32 = 0x0200;
    pub const PARAM_SFO: u32 = 0x1000;
    pub const PIC1_PNG: u32 = 0x1006;
    pub const ICON0_PNG: u32 = 0x1200;
    pub const ICON0_XX_PNG: std::ops::RangeInclusive<u32> = 0x1201..=0x121F;
    pub const PIC0_PNG: u32 = 0x1220;
    pub const PIC1_XX_PNG: std::ops::RangeInclusive<u32> = 0x1241..=0x125F;
    pub const CHANGEINFO_XML: u32 = 0x1260;
    pub const TROPHY_TRP: u32 = 0x1400;
}

/// Size limits for the entries we load into memory.
pub mod limit {
    pub const SFO: u32 = 64 * 1024;
    pub const ICON: u32 = 8 * 1024 * 1024;
    pub const PIC: u32 = 16 * 1024 * 1024;
    pub const CHANGEINFO: u32 = 1024 * 1024;
    pub const NAMES: u32 = 1024 * 1024;
}

/// Content type stored at header offset 0x74.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    GameData,
    AdditionalContent,
    AdditionalLicense,
    DeltaPatch,
    Other(u32),
}

impl From<u32> for ContentType {
    fn from(v: u32) -> Self {
        match v {
            0x1A => Self::GameData,
            0x1B => Self::AdditionalContent,
            0x1C => Self::AdditionalLicense,
            0x1E => Self::DeltaPatch,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub pkg_type: u32,
    pub file_count: u32,
    pub entry_count: u32,
    pub table_offset: u32,
    pub body_offset: u64,
    pub body_size: u64,
    pub content_id: String,
    pub drm_type: u32,
    pub content_type: ContentType,
    pub content_flags: u32,
    pub pfs_image_offset: u64,
    pub pfs_image_size: u64,
    /// Total size the PKG should have once fully downloaded (0 if unknown).
    pub pkg_size: u64,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u32,
    pub name_offset: u32,
    pub flags1: u32,
    pub flags2: u32,
    pub offset: u32,
    pub size: u32,
}

impl Entry {
    pub fn is_encrypted(&self) -> bool {
        self.flags1 & ENTRY_FLAG_ENCRYPTED != 0
    }
}

pub struct Ps4Pkg<R> {
    reader: R,
    file_len: u64,
    pub header: Header,
    pub entries: Vec<Entry>,
}

fn be_u32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn be_u64(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().unwrap())
}

/// Reads a fixed-size, NUL-padded ASCII string.
pub(crate) fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// Returns true if the first bytes look like a PS4 PKG.
pub fn is_ps4_pkg(first_bytes: &[u8]) -> bool {
    first_bytes.len() >= 4 && be_u32(first_bytes, 0) == MAGIC
}

impl<R: Read + Seek> Ps4Pkg<R> {
    pub fn open(mut reader: R) -> Result<Self, Error> {
        let file_len = reader.seek(SeekFrom::End(0))?;
        if file_len < 0x80 {
            return Err(Error::NotPs4Pkg);
        }
        reader.seek(SeekFrom::Start(0))?;

        // Small (fake/homebrew) PKGs can be shorter than a full header page.
        let header_len = HEADER_SIZE.min(file_len as usize);
        let mut h = vec![0u8; HEADER_SIZE];
        reader.read_exact(&mut h[..header_len])?;
        if be_u32(&h, 0) != MAGIC {
            return Err(Error::NotPs4Pkg);
        }

        let header = Header {
            pkg_type: be_u32(&h, 0x04),
            file_count: be_u32(&h, 0x0C),
            entry_count: be_u32(&h, 0x10),
            table_offset: be_u32(&h, 0x18),
            body_offset: be_u64(&h, 0x20),
            body_size: be_u64(&h, 0x28),
            content_id: cstr(&h[0x40..0x64]),
            drm_type: be_u32(&h, 0x70),
            content_type: ContentType::from(be_u32(&h, 0x74)),
            content_flags: be_u32(&h, 0x78),
            pfs_image_offset: be_u64(&h, 0x410),
            pfs_image_size: be_u64(&h, 0x418),
            pkg_size: be_u64(&h, 0x430),
        };

        if header.entry_count > MAX_ENTRIES {
            return Err(Error::Corrupt("too many entries"));
        }
        let table_len = header.entry_count as u64 * ENTRY_SIZE as u64;
        if header.table_offset as u64 + table_len > file_len {
            return Err(Error::Truncated);
        }

        reader.seek(SeekFrom::Start(header.table_offset as u64))?;
        let mut table = vec![0u8; table_len as usize];
        reader.read_exact(&mut table)?;
        let entries = table
            .chunks_exact(ENTRY_SIZE)
            .map(|e| Entry {
                id: be_u32(e, 0x00),
                name_offset: be_u32(e, 0x04),
                flags1: be_u32(e, 0x08),
                flags2: be_u32(e, 0x0C),
                offset: be_u32(e, 0x10),
                size: be_u32(e, 0x14),
            })
            .collect();

        Ok(Self { reader, file_len, header, entries })
    }

    /// Size of the file on disk.
    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    /// Size the PKG should have when fully downloaded, if the header says so.
    pub fn expected_len(&self) -> Option<u64> {
        let h = &self.header;
        let pfs_end = h.pfs_image_offset.saturating_add(h.pfs_image_size);
        let body_end = h.body_offset.saturating_add(h.body_size);
        [h.pkg_size, pfs_end, body_end].into_iter().filter(|&v| v > 0).max()
    }

    pub fn entry(&self, id: u32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Reads a plain-text entry. Returns `Ok(None)` if the entry does not
    /// exist, is encrypted or is larger than `max_size`.
    pub fn read_entry(&mut self, id: u32, max_size: u32) -> Result<Option<Vec<u8>>, Error> {
        let Some(e) = self.entry(id).cloned() else {
            return Ok(None);
        };
        if e.is_encrypted() || e.size == 0 || e.size > max_size {
            return Ok(None);
        }
        self.read_at(e.offset as u64, e.size as usize).map(Some)
    }

    /// Reads the first readable entry among `ids`.
    fn read_first(
        &mut self,
        ids: impl IntoIterator<Item = u32>,
        max_size: u32,
    ) -> Result<Option<Vec<u8>>, Error> {
        for id in ids {
            if let Some(data) = self.read_entry(id, max_size)? {
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    pub fn read_param_sfo(&mut self) -> Result<Option<Vec<u8>>, Error> {
        self.read_entry(id::PARAM_SFO, limit::SFO)
    }

    /// icon0.png, falling back to the first localized icon0_XX.png.
    pub fn read_icon0(&mut self) -> Result<Option<Vec<u8>>, Error> {
        self.read_first(std::iter::once(id::ICON0_PNG).chain(id::ICON0_XX_PNG), limit::ICON)
    }

    /// pic1.png (background), falling back to pic1_XX.png and pic0.png.
    pub fn read_pic1(&mut self) -> Result<Option<Vec<u8>>, Error> {
        let ids = std::iter::once(id::PIC1_PNG)
            .chain(id::PIC1_XX_PNG)
            .chain(std::iter::once(id::PIC0_PNG));
        self.read_first(ids, limit::PIC)
    }

    pub fn read_changeinfo(&mut self) -> Result<Option<Vec<u8>>, Error> {
        self.read_entry(id::CHANGEINFO_XML, limit::CHANGEINFO)
    }

    /// Game icon: icon0.png, or the `ICON0.PNG` of the trophy container when
    /// the PKG (typically a patch) does not ship one.
    pub fn read_icon(&mut self) -> Result<Option<Vec<u8>>, Error> {
        match self.read_icon0()? {
            Some(icon) => Ok(Some(icon)),
            None => self.read_trophy_icon(),
        }
    }

    /// `ICON0.PNG` from `trophy/trophy00.trp`, if present and unencrypted.
    pub fn read_trophy_icon(&mut self) -> Result<Option<Vec<u8>>, Error> {
        let Some(e) = self.entry(id::TROPHY_TRP).cloned() else {
            return Ok(None);
        };
        if e.is_encrypted() {
            return Ok(None);
        }
        let entry_end = e.offset as u64 + e.size as u64;
        let head = self.read_at(e.offset as u64, 0x60.min(e.size as usize))?;
        let Some(table_len) = crate::trp::table_len(&head) else {
            return Ok(None);
        };
        let table = self.read_at(e.offset as u64, table_len.min(e.size as usize))?;
        let Some(icon) = crate::trp::files(&table)
            .into_iter()
            .find(|f| f.name.eq_ignore_ascii_case("ICON0.PNG"))
        else {
            return Ok(None);
        };
        let start = e.offset as u64 + icon.offset;
        if icon.size == 0 || icon.size > limit::ICON as u64 || start + icon.size > entry_end {
            return Ok(None);
        }
        let data = self.read_at(start, icon.size as usize)?;
        Ok(data.starts_with(b"\x89PNG").then_some(data))
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, Error> {
        if offset + len as u64 > self.file_len {
            return Err(Error::Truncated);
        }
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Names of the entries that have one (from the ENTRY_NAMES table).
    pub fn entry_names(&mut self) -> Result<Vec<(u32, String)>, Error> {
        let Some(names) = self.read_entry(id::ENTRY_NAMES, limit::NAMES)? else {
            return Ok(Vec::new());
        };
        Ok(self
            .entries
            .iter()
            .filter(|e| e.name_offset != 0 && (e.name_offset as usize) < names.len())
            .map(|e| (e.id, cstr(&names[e.name_offset as usize..])))
            .collect())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::sfo::{Value, tests::build as build_sfo};
    use std::io::Cursor;

    /// Builds a minimal PS4 PKG with the given (id, flags1, data) entries.
    pub fn build_pkg(content_id: &str, entries: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
        let table_offset = HEADER_SIZE;
        let mut data_offset = table_offset + entries.len() * ENTRY_SIZE;
        let mut out = vec![0u8; data_offset];
        out[..4].copy_from_slice(&MAGIC.to_be_bytes());
        out[0x10..0x14].copy_from_slice(&(entries.len() as u32).to_be_bytes());
        out[0x18..0x1C].copy_from_slice(&(table_offset as u32).to_be_bytes());
        out[0x40..0x40 + content_id.len()].copy_from_slice(content_id.as_bytes());
        out[0x74..0x78].copy_from_slice(&0x1Au32.to_be_bytes());
        for (i, (id, flags1, data)) in entries.iter().enumerate() {
            let e = table_offset + i * ENTRY_SIZE;
            out[e..e + 4].copy_from_slice(&id.to_be_bytes());
            out[e + 8..e + 12].copy_from_slice(&flags1.to_be_bytes());
            out[e + 16..e + 20].copy_from_slice(&(data_offset as u32).to_be_bytes());
            out[e + 20..e + 24].copy_from_slice(&(data.len() as u32).to_be_bytes());
            data_offset += data.len();
        }
        for (_, _, data) in entries {
            out.extend_from_slice(data);
        }
        let len = out.len() as u64;
        out[0x430..0x438].copy_from_slice(&len.to_be_bytes());
        out
    }

    fn sample() -> Vec<u8> {
        let sfo = build_sfo(&[
            ("CATEGORY", Value::Str("gp".into())),
            ("TITLE", Value::Str("Test Game".into())),
            ("TITLE_03", Value::Str("Juego de prueba".into())),
            ("TITLE_ID", Value::Str("CUSA12345".into())),
            ("APP_VER", Value::Str("01.05".into())),
            ("SYSTEM_VER", Value::Int(0x0900_0000)),
        ]);
        build_pkg(
            "EP0001-CUSA12345_00-TESTGAME00000000",
            &[
                (id::PARAM_SFO, 0, sfo),
                (id::ICON0_PNG, 0, b"\x89PNGicon".to_vec()),
                (id::PIC1_PNG, ENTRY_FLAG_ENCRYPTED, b"secret".to_vec()),
            ],
        )
    }

    #[test]
    fn reads_synthetic_pkg() {
        let mut pkg = Ps4Pkg::open(Cursor::new(sample())).unwrap();
        assert_eq!(pkg.header.content_id, "EP0001-CUSA12345_00-TESTGAME00000000");
        assert_eq!(pkg.header.content_type, ContentType::GameData);
        assert_eq!(pkg.read_icon0().unwrap().as_deref(), Some(&b"\x89PNGicon"[..]));
        // Encrypted entries are skipped, not returned as garbage.
        assert_eq!(pkg.read_pic1().unwrap(), None);

        let info = crate::PkgInfo::read(&mut pkg).unwrap();
        assert_eq!(info.title_id, "CUSA12345");
        assert_eq!(info.category, crate::Category::Patch);
        assert_eq!(info.region, Some(crate::Region::Europe));
        assert_eq!(info.firmware().as_deref(), Some("9.00"));
        assert_eq!(info.title_for(&["es-ES".into()]), "Juego de prueba");
        assert_eq!(info.missing_bytes(), None);
    }

    #[test]
    fn detects_incomplete_download() {
        let mut bytes = sample();
        let full = bytes.len() as u64;
        bytes[0x430..0x438].copy_from_slice(&(full + 1000).to_be_bytes());
        let mut pkg = Ps4Pkg::open(Cursor::new(bytes)).unwrap();
        let info = crate::PkgInfo::read(&mut pkg).unwrap();
        assert_eq!(info.missing_bytes(), Some(1000));
    }

    #[test]
    fn rejects_non_ps4_and_truncated() {
        assert!(matches!(Ps4Pkg::open(Cursor::new(vec![0u8; 0x2000])), Err(Error::NotPs4Pkg)));
        let mut ps3 = vec![0u8; 0x200];
        ps3[..4].copy_from_slice(b"\x7FPKG");
        assert!(matches!(Ps4Pkg::open(Cursor::new(ps3)), Err(Error::NotPs4Pkg)));

        // Entry table pointing past the end of the file.
        let mut bytes = sample();
        bytes.truncate(HEADER_SIZE + 8);
        assert!(matches!(Ps4Pkg::open(Cursor::new(bytes)), Err(Error::Truncated)));

        // Entry data pointing past the end of the file: cut through icon0
        // (pic1's 6 bytes are last, icon0's 8 bytes come right before).
        let mut bytes = sample();
        let len = bytes.len();
        bytes.truncate(len - 6 - 4);
        let mut pkg = Ps4Pkg::open(Cursor::new(bytes)).unwrap();
        assert!(pkg.read_entry(id::PIC1_PNG, limit::PIC).is_ok()); // encrypted: skipped
        assert!(matches!(pkg.read_entry(id::ICON0_PNG, limit::ICON), Err(Error::Truncated)));
    }

    #[test]
    fn never_panics_on_random_headers() {
        let base = sample();
        let mut state = 0x1234_5678u32;
        for _ in 0..2000 {
            let mut bytes = base.clone();
            for _ in 0..8 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let at = 4 + (state as usize % (bytes.len() - 4));
                bytes[at] = (state >> 8) as u8;
            }
            if let Ok(mut pkg) = Ps4Pkg::open(Cursor::new(bytes)) {
                let _ = crate::PkgInfo::read(&mut pkg);
                let _ = pkg.read_icon();
                let _ = pkg.entry_names();
            }
        }
    }
}
