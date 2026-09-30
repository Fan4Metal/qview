//! Human-readable values for the status bar, in the interface language
//! ([`crate::i18n`]); the `_in` variants take the language explicitly.

use crate::i18n::{Lang, lang};

const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
/// As in the Russian Explorer.
const UNITS_RU: [&str; 5] = ["байт", "КБ", "МБ", "ГБ", "ТБ"];

/// `337.3 KB`, `4.6 MB`, `512 bytes` (1024-based, one decimal); in Russian `337,3 КБ`.
pub fn file_size(bytes: u64) -> String {
    file_size_in(lang(), bytes)
}

pub fn file_size_in(lang: Lang, bytes: u64) -> String {
    let units = match lang {
        Lang::En => UNITS,
        Lang::Ru => UNITS_RU,
    };
    if bytes < 1024 {
        return format!("{bytes} {}", units[0]);
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < units.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let number = format!("{v:.1}");
    let number = match lang {
        Lang::En => number,
        Lang::Ru => number.replace('.', ","),
    };
    format!("{number} {}", units[unit])
}

/// Zoom as a whole percentage, `100%`; below 1% with one decimal.
pub fn zoom(scale: f32) -> String {
    let p = scale * 100.0;
    if p >= 1.0 { format!("{}%", p.round()) } else { format!("{p:.1}%") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(file_size_in(Lang::En, 0), "0 bytes");
        assert_eq!(file_size_in(Lang::En, 1023), "1023 bytes");
        assert_eq!(file_size_in(Lang::En, 1024), "1.0 KB");
        assert_eq!(file_size_in(Lang::En, 345_395), "337.3 KB");
        assert_eq!(file_size_in(Lang::En, 4_939_212_390), "4.6 GB");
        assert_eq!(file_size_in(Lang::Ru, 345_395), "337,3 КБ");
        assert_eq!(file_size_in(Lang::Ru, 512), "512 байт");
    }

    #[test]
    fn zooms() {
        assert_eq!(zoom(1.0), "100%");
        assert_eq!(zoom(0.333), "33%");
        assert_eq!(zoom(16.0), "1600%");
        assert_eq!(zoom(0.005), "0.5%");
    }
}
