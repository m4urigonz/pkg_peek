//! Formatting in the user's language (Spanish UI strings).

use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows_core::PWSTR;

/// Preferred UI languages as BCP-47 tags, most preferred first.
pub fn ui_languages() -> Vec<String> {
    unsafe {
        let mut count = 0u32;
        let mut len = 0u32;
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_err() {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, Some(PWSTR(buf.as_mut_ptr())), &mut len)
            .is_err()
        {
            return Vec::new();
        }
        buf.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}

/// Size like Explorer shows it (1024-based), with a decimal comma: "41,5 GB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = if value < 100.0 { format!("{value:.1}") } else { format!("{value:.0}") };
    format!("{} {}", text.replace('.', ","), UNITS[unit])
}

/// "2024-05-10" -> "10/05/2024".
pub fn format_date(iso: &str) -> String {
    let mut parts = iso.split('-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(y), Some(m), Some(d)) => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(22_649_241), "21,6 MB");
        assert_eq!(format_size(0xA223B0000), "40,5 GB");
        assert_eq!(format_size(371_261_440), "354 MB");
    }

    #[test]
    fn dates() {
        assert_eq!(format_date("2024-05-10"), "10/05/2024");
    }
}
