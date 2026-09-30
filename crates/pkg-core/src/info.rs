//! Human-oriented summary of a PKG: what the shell handlers display.

use std::io::{Read, Seek};

use crate::pkg::{ContentType, Ps4Pkg};
use crate::sfo::Sfo;
use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Game,
    /// `gd` package whose title ID is not an official one (homebrew).
    App,
    Patch,
    Dlc,
    Other(String),
}

impl Category {
    fn from_sfo(raw: &str, title_id: &str) -> Self {
        match raw {
            "gd" if is_official_title_id(title_id) => Self::Game,
            "gd" | "gde" => Self::App,
            "gp" => Self::Patch,
            "ac" => Self::Dlc,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Game => "Juego",
            Self::App => "App",
            Self::Patch => "Actualización",
            Self::Dlc => "DLC",
            Self::Other(raw) if raw.is_empty() => "Desconocido",
            Self::Other(raw) => raw,
        }
    }
}

fn is_official_title_id(id: &str) -> bool {
    const PREFIXES: [&str; 11] = [
        "CUSA", "PLAS", "PCAS", "PLJS", "PLJM", "PCJS", "PCKS", "PLKS", "PCCS", "PCES", "PCUS",
    ];
    PREFIXES.iter().any(|p| id.starts_with(p))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Usa,
    Europe,
    Japan,
    Asia,
    Korea,
}

impl Region {
    fn from_content_id(cid: &str) -> Option<Self> {
        match cid.get(..2)? {
            "UP" => Some(Self::Usa),
            "EP" => Some(Self::Europe),
            "JP" => Some(Self::Japan),
            "HP" => Some(Self::Asia),
            "KP" => Some(Self::Korea),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Usa => "USA",
            Self::Europe => "EUR",
            Self::Japan => "JPN",
            Self::Asia => "ASIA",
            Self::Korea => "KOR",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Usa => "América",
            Self::Europe => "Europa",
            Self::Japan => "Japón",
            Self::Asia => "Asia",
            Self::Korea => "Corea",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PkgInfo {
    pub content_id: String,
    pub title_id: String,
    /// Default `TITLE` from PARAM.SFO.
    pub title: String,
    /// Localized `TITLE_XX` entries: (XX, title).
    pub titles: Vec<(u8, String)>,
    pub category: Category,
    pub category_raw: String,
    pub app_ver: Option<String>,
    pub version: Option<String>,
    pub system_ver: Option<u32>,
    pub region: Option<Region>,
    /// `YYYY-MM-DD`, from PUBTOOLINFO.
    pub creation_date: Option<String>,
    pub sdk_ver: Option<String>,
    pub content_type: ContentType,
    pub content_flags: u32,
    pub file_len: u64,
    pub expected_len: Option<u64>,
    /// Plain-text patch notes from changeinfo.xml.
    pub change_notes: Option<String>,
}

impl PkgInfo {
    /// Reads everything except the images.
    pub fn read<R: Read + Seek>(pkg: &mut Ps4Pkg<R>) -> Result<Self, Error> {
        let sfo = match pkg.read_param_sfo()? {
            Some(b) => Sfo::parse(&b)?,
            None => Sfo::default(),
        };
        let content_id = sfo
            .str("CONTENT_ID")
            .map(str::to_string)
            .unwrap_or_else(|| pkg.header.content_id.clone());
        let title_id = sfo
            .str("TITLE_ID")
            .map(str::to_string)
            .or_else(|| content_id.get(7..16).map(str::to_string))
            .unwrap_or_default();
        let category_raw = sfo.str("CATEGORY").unwrap_or_default().to_string();
        let titles = sfo
            .entries
            .iter()
            .filter_map(|(k, _)| {
                let code: u8 = k.strip_prefix("TITLE_")?.parse().ok()?;
                Some((code, sfo.str(k)?.to_string()))
            })
            .collect();
        let pubtool = sfo.str("PUBTOOLINFO").unwrap_or_default();
        let app_ver = sfo.str("APP_VER").map(str::to_string);

        let change_notes = match pkg.read_changeinfo()? {
            Some(xml) => changeinfo_text(&String::from_utf8_lossy(&xml), app_ver.as_deref()),
            None => None,
        };

        Ok(Self {
            region: Region::from_content_id(&content_id),
            category: Category::from_sfo(&category_raw, &title_id),
            title: sfo.str("TITLE").unwrap_or(&title_id).to_string(),
            titles,
            app_ver,
            version: sfo.str("VERSION").map(str::to_string),
            system_ver: sfo.int("SYSTEM_VER").filter(|&v| v != 0),
            creation_date: pubtool_field(pubtool, "c_date").and_then(format_date),
            sdk_ver: pubtool_field(pubtool, "sdk_ver").and_then(format_sdk),
            content_type: pkg.header.content_type,
            content_flags: pkg.header.content_flags,
            file_len: pkg.file_len(),
            expected_len: pkg.expected_len(),
            content_id,
            title_id,
            category_raw,
            change_notes,
        })
    }

    /// Best title for the given UI languages (BCP-47 tags, most preferred
    /// first), falling back to the default `TITLE`.
    pub fn title_for(&self, languages: &[String]) -> &str {
        languages
            .iter()
            .flat_map(|l| sfo_language_codes(l))
            .find_map(|code| self.titles.iter().find(|(c, _)| *c == code))
            .map(|(_, t)| t.as_str())
            .unwrap_or(&self.title)
    }

    /// Required firmware, e.g. `0x05050000` -> "5.05".
    pub fn firmware(&self) -> Option<String> {
        let v = self.system_ver?;
        Some(format!("{:x}.{:02x}", v >> 24, (v >> 16) & 0xFF))
    }

    /// Bytes missing if the file is an incomplete download.
    pub fn missing_bytes(&self) -> Option<u64> {
        self.expected_len.filter(|&e| e > self.file_len).map(|e| e - self.file_len)
    }

    /// Version without leading zero ("01.07" -> "1.07").
    pub fn short_app_ver(&self) -> Option<String> {
        let v = self.app_ver.as_deref()?;
        Some(v.strip_prefix('0').filter(|s| !s.starts_with('.')).unwrap_or(v).to_string())
    }
}

/// PARAM.SFO `TITLE_XX` suffixes for a language tag, best match first.
pub fn sfo_language_codes(tag: &str) -> Vec<u8> {
    let tag = tag.to_ascii_lowercase().replace('_', "-");
    let (lang, rest) = tag.split_once('-').unwrap_or((&tag, ""));
    match (lang, rest) {
        ("ja", _) => vec![0],
        ("en", "gb" | "ie" | "au" | "nz" | "in") => vec![18, 1],
        ("en", _) => vec![1, 18],
        ("fr", "ca") => vec![22, 2],
        ("fr", _) => vec![2, 22],
        ("es", "es" | "") => vec![3, 20],
        ("es", _) => vec![20, 3],
        ("de", _) => vec![4],
        ("it", _) => vec![5],
        ("nl", _) => vec![6],
        ("pt", "br") => vec![17, 7],
        ("pt", _) => vec![7, 17],
        ("ru", _) => vec![8],
        ("ko", _) => vec![9],
        ("zh", r) if r.contains("tw") || r.contains("hk") || r.contains("hant") => vec![10, 11],
        ("zh", _) => vec![11, 10],
        ("fi", _) => vec![12],
        ("sv", _) => vec![13],
        ("da", _) => vec![14],
        ("nb" | "nn" | "no", _) => vec![15],
        ("pl", _) => vec![16],
        ("tr", _) => vec![19],
        ("ar", _) => vec![21],
        ("cs", _) => vec![23],
        ("hu", _) => vec![24],
        ("el", _) => vec![25],
        ("ro", _) => vec![26],
        ("th", _) => vec![27],
        ("vi", _) => vec![28],
        ("id", _) => vec![29],
        _ => vec![],
    }
}

/// `c_date=20190212,sdk_ver=06508001,...` -> value of `key`.
fn pubtool_field<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    info.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

fn format_date(d: &str) -> Option<String> {
    (d.len() == 8 && d.bytes().all(|c| c.is_ascii_digit()))
        .then(|| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]))
}

fn format_sdk(v: &str) -> Option<String> {
    if v.len() != 8 || !v.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let major = v[..2].trim_start_matches('0');
    Some(format!("{}.{}", if major.is_empty() { "0" } else { major }, &v[2..5]))
}

/// Extracts the notes of the `<changes>` block matching `app_ver` (or the
/// first one) as plain text.
fn changeinfo_text(xml: &str, app_ver: Option<&str>) -> Option<String> {
    let mut blocks = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<changes") {
        rest = &rest[start..];
        let open_end = rest.find('>')?;
        let open = &rest[..open_end];
        let body_start = open_end + 1;
        let close = rest[body_start..].find("</changes>")? + body_start;
        let ver = open
            .split("app_ver=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        blocks.push((ver.to_string(), &rest[body_start..close]));
        rest = &rest[close..];
    }
    let body = app_ver
        .and_then(|v| blocks.iter().find(|(bv, _)| bv == v))
        .or(blocks.first())?
        .1;
    let text = strip_markup(body);
    (!text.is_empty()).then_some(text)
}

fn strip_markup(s: &str) -> String {
    let s = s.replace("<![CDATA[", "").replace("]]>", "");
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_is_bcd() {
        let mut info = sample();
        info.system_ver = Some(0x0505_0000);
        assert_eq!(info.firmware().as_deref(), Some("5.05"));
        info.system_ver = Some(0x1100_0000);
        assert_eq!(info.firmware().as_deref(), Some("11.00"));
    }

    #[test]
    fn localized_title() {
        let mut info = sample();
        info.titles = vec![(3, "Título ES".into()), (20, "Título LATAM".into())];
        assert_eq!(info.title_for(&["es-MX".into()]), "Título LATAM");
        assert_eq!(info.title_for(&["es-ES".into()]), "Título ES");
        assert_eq!(info.title_for(&["de-DE".into()]), "Default");
    }

    #[test]
    fn pubtool_parsing() {
        let p = "c_date=20190212,sdk_ver=06508001,st_type=digital50";
        assert_eq!(pubtool_field(p, "c_date").and_then(format_date).as_deref(), Some("2019-02-12"));
        assert_eq!(pubtool_field(p, "sdk_ver").and_then(format_sdk).as_deref(), Some("6.508"));
    }

    #[test]
    fn changeinfo_picks_matching_version() {
        let xml = r#"<?xml version="1.0"?><changeinfo>
            <changes app_ver="01.01">Old fix</changes>
            <changes app_ver="01.02"><![CDATA[- New thing & more
              - Another]]></changes></changeinfo>"#;
        assert_eq!(changeinfo_text(xml, Some("01.02")).as_deref(), Some("- New thing & more\n- Another"));
        assert_eq!(changeinfo_text(xml, None).as_deref(), Some("Old fix"));
    }

    #[test]
    fn short_version() {
        let mut info = sample();
        info.app_ver = Some("01.07".into());
        assert_eq!(info.short_app_ver().as_deref(), Some("1.07"));
        info.app_ver = Some("10.00".into());
        assert_eq!(info.short_app_ver().as_deref(), Some("10.00"));
    }

    fn sample() -> PkgInfo {
        PkgInfo {
            content_id: "UP0000-CUSA00000_00-0000000000000000".into(),
            title_id: "CUSA00000".into(),
            title: "Default".into(),
            titles: vec![],
            category: Category::Game,
            category_raw: "gd".into(),
            app_ver: None,
            version: None,
            system_ver: None,
            region: None,
            creation_date: None,
            sdk_ver: None,
            content_type: ContentType::GameData,
            content_flags: 0,
            file_len: 0,
            expected_len: None,
            change_notes: None,
        }
    }
}
