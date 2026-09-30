//! PARAM.SFO key/value table (little-endian).

use crate::Error;

const MAGIC: u32 = 0x4653_5000; // "\0PSF"
const FMT_UTF8_SPECIAL: u16 = 0x0004;
const FMT_UTF8: u16 = 0x0204;
const FMT_INT32: u16 = 0x0404;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(u32),
}

#[derive(Debug, Clone, Default)]
pub struct Sfo {
    /// Entries in file order.
    pub entries: Vec<(String, Value)>,
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl Sfo {
    pub fn parse(b: &[u8]) -> Result<Self, Error> {
        let bad = || Error::Corrupt("PARAM.SFO");
        if le_u32(b, 0) != Some(MAGIC) {
            return Err(bad());
        }
        let key_table = le_u32(b, 0x08).ok_or_else(bad)? as usize;
        let data_table = le_u32(b, 0x0C).ok_or_else(bad)? as usize;
        let count = le_u32(b, 0x10).ok_or_else(bad)? as usize;
        if count > 1024 {
            return Err(bad());
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let at = 0x14 + i * 16;
            let key_off = le_u16(b, at).ok_or_else(bad)? as usize;
            let fmt = le_u16(b, at + 2).ok_or_else(bad)?;
            let len = le_u32(b, at + 4).ok_or_else(bad)? as usize;
            let data_off = le_u32(b, at + 12).ok_or_else(bad)? as usize;

            let key_start = key_table.checked_add(key_off).ok_or_else(bad)?;
            let key = b.get(key_start..).ok_or_else(bad)?;
            let key = crate::pkg::cstr(key);

            let start = data_table.checked_add(data_off).ok_or_else(bad)?;
            let data = b.get(start..start.checked_add(len).ok_or_else(bad)?).ok_or_else(bad)?;
            let value = match fmt {
                FMT_INT32 => Value::Int(le_u32(data, 0).ok_or_else(bad)?),
                FMT_UTF8 | FMT_UTF8_SPECIAL => {
                    let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
                    Value::Str(String::from_utf8_lossy(&data[..end]).into_owned())
                }
                _ => continue,
            };
            entries.push((key, value));
        }
        Ok(Self { entries })
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// String value, trimmed; `None` when missing or empty.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s.trim()).filter(|s| !s.is_empty()),
            Value::Int(_) => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<u32> {
        match self.get(key)? {
            Value::Int(v) => Some(*v),
            Value::Str(_) => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a PARAM.SFO blob; used by the tests of this crate.
    pub fn build(entries: &[(&str, Value)]) -> Vec<u8> {
        let mut keys = Vec::new();
        let mut data = Vec::new();
        let mut index = Vec::new();
        for (k, v) in entries {
            let key_off = keys.len() as u16;
            keys.extend_from_slice(k.as_bytes());
            keys.push(0);
            let data_off = data.len() as u32;
            let (fmt, bytes) = match v {
                Value::Int(n) => (FMT_INT32, n.to_le_bytes().to_vec()),
                Value::Str(s) => {
                    let mut b = s.as_bytes().to_vec();
                    b.push(0);
                    (FMT_UTF8, b)
                }
            };
            let max = bytes.len().next_multiple_of(4) as u32;
            data.extend_from_slice(&bytes);
            data.resize(data_off as usize + max as usize, 0);
            index.extend_from_slice(&key_off.to_le_bytes());
            index.extend_from_slice(&fmt.to_le_bytes());
            index.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            index.extend_from_slice(&max.to_le_bytes());
            index.extend_from_slice(&data_off.to_le_bytes());
        }
        while keys.len() % 4 != 0 {
            keys.push(0);
        }
        let key_table = 0x14 + index.len();
        let data_table = key_table + keys.len();
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&0x0101u32.to_le_bytes());
        out.extend_from_slice(&(key_table as u32).to_le_bytes());
        out.extend_from_slice(&(data_table as u32).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&index);
        out.extend_from_slice(&keys);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn roundtrip() {
        let blob = build(&[
            ("TITLE", Value::Str("Apollo Save Tool".into())),
            ("SYSTEM_VER", Value::Int(0x0505_0000)),
        ]);
        let sfo = Sfo::parse(&blob).unwrap();
        assert_eq!(sfo.str("TITLE"), Some("Apollo Save Tool"));
        assert_eq!(sfo.int("SYSTEM_VER"), Some(0x0505_0000));
        assert_eq!(sfo.str("MISSING"), None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Sfo::parse(b"not an sfo at all").is_err());
        let mut blob = build(&[("TITLE", Value::Str("x".into()))]);
        blob.truncate(0x20);
        assert!(Sfo::parse(&blob).is_err());
    }
}
