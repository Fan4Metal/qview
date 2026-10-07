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

/// `v` with up to `digits` decimals, trailing zeros dropped: `2.8`, `8`;
/// with a decimal comma in Russian.
pub fn decimal_in(lang: Lang, v: f64, digits: usize) -> String {
    let s = format!("{v:.digits$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    let s = if s == "-0" { "0".into() } else { s };
    match lang {
        Lang::En => s,
        Lang::Ru => s.replace('.', ","),
    }
}

/// A whole number with its thousands apart: `3,356,123`, in Russian
/// `3 356 123` (with no-break spaces).
pub fn thousands_in(lang: Lang, n: u64) -> String {
    let digits = n.to_string();
    let sep = match lang {
        Lang::En => ',',
        Lang::Ru => '\u{a0}',
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// A file's size and its exact number of bytes: `3.2 MB (3,356,123 bytes)`.
pub fn exact_size(bytes: u64) -> String {
    exact_size_in(lang(), bytes)
}

pub fn exact_size_in(lang: Lang, bytes: u64) -> String {
    let short = file_size_in(lang, bytes);
    if bytes < 1024 {
        return short;
    }
    let n = thousands_in(lang, bytes);
    match lang {
        Lang::En => format!("{short} ({n} bytes)"),
        Lang::Ru => format!("{short} ({n} байт)"),
    }
}

/// Width and height with the megapixels: `4000 × 3000 (12.0 MP)`.
pub fn dimensions(width: u32, height: u32) -> String {
    dimensions_in(lang(), width, height)
}

pub fn dimensions_in(lang: Lang, width: u32, height: u32) -> String {
    let mp = width as f64 * height as f64 / 1e6;
    if mp < 0.1 {
        return format!("{width} × {height}");
    }
    let mp = decimal_in(lang, mp, 1);
    match lang {
        Lang::En => format!("{width} × {height} ({mp} MP)"),
        Lang::Ru => format!("{width} × {height} ({mp} Мп)"),
    }
}

/// An exposure time as cameras show it: `1/250 s` up to a quarter of a
/// second, `0.5 s`, `2 s` from there.
pub fn exposure(seconds: f64) -> String {
    exposure_in(lang(), seconds)
}

pub fn exposure_in(lang: Lang, seconds: f64) -> String {
    let unit = match lang {
        Lang::En => "s",
        Lang::Ru => "с",
    };
    if seconds > 0.0 && seconds <= 0.25 + 1e-9 {
        format!("1/{} {unit}", decimal_in(lang, 1.0 / seconds, 0))
    } else {
        format!("{} {unit}", decimal_in(lang, seconds, 1))
    }
}

/// `f/2.8`, `f/8`.
pub fn aperture(f: f64) -> String {
    format!("f/{}", decimal_in(lang(), f, 1))
}

/// `50 mm`, with the 35 mm equivalent when it differs: `4.2 mm (26 mm
/// equiv.)`.
pub fn focal(mm: Option<f64>, mm_35: Option<u32>) -> Option<String> {
    focal_in(lang(), mm, mm_35)
}

pub fn focal_in(lang: Lang, mm: Option<f64>, mm_35: Option<u32>) -> Option<String> {
    let unit = match lang {
        Lang::En => "mm",
        Lang::Ru => "мм",
    };
    match (mm, mm_35) {
        (Some(mm), Some(e)) if (mm - e as f64).abs() >= 0.5 => Some(match lang {
            Lang::En => format!("{} {unit} ({e} {unit} equiv.)", decimal_in(lang, mm, 1)),
            Lang::Ru => format!("{} {unit} (экв. {e} {unit})", decimal_in(lang, mm, 1)),
        }),
        (Some(mm), _) => Some(format!("{} {unit}", decimal_in(lang, mm, 1))),
        (None, Some(e)) => Some(match lang {
            Lang::En => format!("{e} {unit} equiv."),
            Lang::Ru => format!("экв. {e} {unit}"),
        }),
        (None, None) => None,
    }
}

/// Exposure compensation: `+0.7 EV`, `−1.3 EV`.
pub fn bias(ev: f64) -> String {
    bias_in(lang(), ev)
}

pub fn bias_in(lang: Lang, ev: f64) -> String {
    let v = decimal_in(lang, ev.abs(), 1);
    let sign = if v == "0" {
        ""
    } else if ev < 0.0 {
        "\u{2212}"
    } else {
        "+"
    };
    format!("{sign}{v} EV")
}

/// Metres: `150 m`, `−20 m`.
pub fn metres(m: f64) -> String {
    let v = decimal_in(lang(), m.abs(), 0);
    let sign = if m < 0.0 && v != "0" { "\u{2212}" } else { "" };
    tr!(format!("{sign}{v} m"), format!("{sign}{v} м"))
}

/// A subject distance: `2.5 m`, `12 m`, infinity (EXIF writes
/// 0xFFFFFFFF/1 for it).
pub fn distance(m: f64) -> String {
    distance_in(lang(), m)
}

pub fn distance_in(lang: Lang, m: f64) -> String {
    if m >= 1e6 {
        return match lang {
            Lang::En => "infinity".into(),
            Lang::Ru => "бесконечность".into(),
        };
    }
    let v = decimal_in(lang, m, if m < 10.0 { 2 } else { 1 });
    match lang {
        Lang::En => format!("{v} m"),
        Lang::Ru => format!("{v} м"),
    }
}

/// Where a camera looked: `123° SE`, from magnetic north `123° SE
/// (magnetic)`; in Russian `123° ЮВ (магнитное)`.
pub fn direction(degrees: f64, magnetic: bool) -> String {
    direction_in(lang(), degrees, magnetic)
}

pub fn direction_in(lang: Lang, degrees: f64, magnetic: bool) -> String {
    let d = degrees.rem_euclid(360.0);
    let point = ((d / 45.0).round() as usize) % 8;
    let (points, note) = match lang {
        Lang::En => (["N", "NE", "E", "SE", "S", "SW", "W", "NW"], " (magnetic)"),
        Lang::Ru => (["С", "СВ", "В", "ЮВ", "Ю", "ЮЗ", "З", "СЗ"], " (магнитное)"),
    };
    let note = if magnetic { note } else { "" };
    format!("{}° {}{note}", d.round() as u32 % 360, points[point])
}

/// A rating of 1 to 5 as stars, `★★★★☆`.
pub fn stars(rating: u32) -> String {
    let rating = rating.min(5) as usize;
    format!("{}{}", "★".repeat(rating), "☆".repeat(5 - rating))
}

/// A zoom ratio: `2×`, `1.5×`.
pub fn ratio(v: f64) -> String {
    format!("{}×", decimal_in(lang(), v, 1))
}

/// A share of 0 to 1 as a percentage: `0.42%`, `12.5%`, `0%`, `<0.01%`;
/// in Russian `0,42 %`.
pub fn percent(share: f64) -> String {
    percent_in(lang(), share)
}

pub fn percent_in(lang: Lang, share: f64) -> String {
    let p = share * 100.0;
    let v = if p > 0.0 && p < 0.01 {
        format!("<{}", decimal_in(lang, 0.01, 2))
    } else {
        decimal_in(lang, p, if p < 10.0 { 2 } else { 1 })
    };
    match lang {
        Lang::En => format!("{v}%"),
        Lang::Ru => format!("{v}\u{a0}%"),
    }
}

/// A camera's name from the EXIF Make and Model: the model alone when it
/// names the maker already (`Canon EOS R6`, `NIKON D850` for "NIKON
/// CORPORATION"), else both (`Apple iPhone 13`).
pub fn camera(make: Option<&str>, model: Option<&str>) -> Option<String> {
    match (make, model) {
        (Some(make), Some(model)) => {
            let first = make.split_whitespace().next().unwrap_or(make).to_lowercase();
            if model.to_lowercase().starts_with(&first) { Some(model.into()) } else { Some(format!("{make} {model}")) }
        }
        (make, model) => make.or(model).map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shooting_values() {
        assert_eq!(distance_in(Lang::En, 2.5), "2.5 m");
        assert_eq!(distance_in(Lang::Ru, 12.34), "12,3 м");
        assert_eq!(distance_in(Lang::En, 4294967295.0), "infinity");
        assert_eq!(direction_in(Lang::En, 123.45, false), "123° SE");
        assert_eq!(direction_in(Lang::Ru, 359.8, true), "0° С (магнитное)");
        assert_eq!(direction_in(Lang::En, 337.4, false), "337° NW");
        assert_eq!(stars(4), "★★★★☆");
        assert_eq!(stars(9), "★★★★★");
    }

    #[test]
    fn percentages() {
        assert_eq!(percent_in(Lang::En, 0.0042), "0.42%");
        assert_eq!(percent_in(Lang::En, 0.125), "12.5%");
        assert_eq!(percent_in(Lang::En, 0.0), "0%");
        assert_eq!(percent_in(Lang::En, 1.0), "100%");
        assert_eq!(percent_in(Lang::En, 0.000_01), "<0.01%");
        assert_eq!(percent_in(Lang::Ru, 0.0042), "0,42\u{a0}%");
    }

    #[test]
    fn camera_values() {
        assert_eq!(decimal_in(Lang::En, 2.8, 1), "2.8");
        assert_eq!(decimal_in(Lang::En, 8.0, 1), "8");
        assert_eq!(decimal_in(Lang::Ru, 2.8, 1), "2,8");
        assert_eq!(thousands_in(Lang::En, 3_356_123), "3,356,123");
        assert_eq!(thousands_in(Lang::Ru, 999), "999");
        assert_eq!(exact_size_in(Lang::En, 3_356_123), "3.2 MB (3,356,123 bytes)");
        assert_eq!(exact_size_in(Lang::En, 100), "100 bytes");
        assert_eq!(dimensions_in(Lang::En, 4000, 3000), "4000 × 3000 (12 MP)");
        assert_eq!(dimensions_in(Lang::Ru, 6000, 4000), "6000 × 4000 (24 Мп)");
        assert_eq!(dimensions_in(Lang::En, 16, 16), "16 × 16");
        assert_eq!(exposure_in(Lang::En, 1.0 / 250.0), "1/250 s");
        assert_eq!(exposure_in(Lang::En, 0.25), "1/4 s");
        assert_eq!(exposure_in(Lang::En, 0.5), "0.5 s");
        assert_eq!(exposure_in(Lang::Ru, 30.0), "30 с");
        assert_eq!(focal_in(Lang::En, Some(4.26), Some(26)), Some("4.3 mm (26 mm equiv.)".into()));
        assert_eq!(focal_in(Lang::En, Some(50.0), Some(50)), Some("50 mm".into()));
        assert_eq!(focal_in(Lang::Ru, Some(50.0), None), Some("50 мм".into()));
        assert_eq!(bias_in(Lang::En, -2.0 / 3.0), "\u{2212}0.7 EV");
        assert_eq!(bias_in(Lang::En, 1.0 / 3.0), "+0.3 EV");
        assert_eq!(bias_in(Lang::En, 0.0), "0 EV");
        assert_eq!(camera(Some("Canon"), Some("Canon EOS R6")).as_deref(), Some("Canon EOS R6"));
        assert_eq!(camera(Some("NIKON CORPORATION"), Some("NIKON D850")).as_deref(), Some("NIKON D850"));
        assert_eq!(camera(Some("Apple"), Some("iPhone 13")).as_deref(), Some("Apple iPhone 13"));
        assert_eq!(camera(None, Some("X100V")).as_deref(), Some("X100V"));
    }

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
