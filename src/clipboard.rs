//! Images through the clipboard. Copy Image (Ctrl+Shift+C) puts the
//! current image on it as it is shown (turned, mirrored, cropped while
//! cropping), decoded from its file at full size: as `CF_DIBV5`, from which
//! Windows makes `CF_DIB` and `CF_BITMAP` for older programs, and as PNG
//! too when it has transparency (browsers, Office and messengers keep the
//! alpha from it; a DIB's alpha is read in different ways). Paste (Ctrl+V)
//! opens what the clipboard holds: files copied in Explorer (the first),
//! a path as text, or an image, which is saved as a PNG into [`folder`]
//! and opened from there, so that it can be cropped and saved elsewhere
//! like any file.

use std::path::{Path, PathBuf};

use clipboard_win::{formats, raw};
use image::{DynamicImage, ImageFormat, RgbaImage};

use crate::edit;

/// The clipboard's names for PNG: Chrome and Firefox write "PNG", some
/// programs the MIME type.
const PNG_NAMES: [&str; 2] = ["PNG", "image/png"];

/// Put `job`'s image (see `edit::render`) on the clipboard. Windows' codecs
/// may decode it, so the calling thread has COM initialised.
pub fn copy_image(job: &edit::Job) -> Result<(), String> {
    let img = edit::render(job)?;
    let rgba = img.to_rgba8();
    let transparent = img.color().has_alpha() && rgba.pixels().any(|p| p.0[3] < 255);
    let png = if transparent { Some(fast_png(&img)?) } else { None };
    drop(img);
    let dib = dib_v5(&rgba);
    let _clipboard = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    raw::empty().map_err(|e| e.to_string())?;
    raw::set_without_clear(formats::CF_DIBV5, &dib).map_err(|e| e.to_string())?;
    if let Some(png) = png
        && let Some(format) = raw::register_format(PNG_NAMES[0])
    {
        raw::set_without_clear(format.get(), &png).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What the clipboard holds that can be opened, if anything (the menu
/// item's state); reading it is [`paste`]'s.
pub fn can_paste() -> bool {
    let png = PNG_NAMES.iter().filter_map(|n| raw::register_format(n)).any(|f| raw::is_format_avail(f.get()));
    png || [formats::CF_HDROP, formats::CF_DIBV5, formats::CF_DIB, formats::CF_UNICODETEXT].into_iter().any(raw::is_format_avail)
}

/// The file to open for the clipboard's contents: the first of the files
/// on it, an existing path given as text, or its image saved into
/// [`folder`].
pub fn paste() -> Result<PathBuf, String> {
    let nothing = || tr!("The clipboard holds no image", "В буфере обмена нет изображения").to_string();
    let contents = {
        let _clipboard = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
        read()
    };
    match contents {
        Some(Contents::Path(path)) => Ok(path),
        Some(Contents::Png(png)) => save(&png),
        Some(Contents::Dib(dib)) => save(&fast_png(&from_dib(&dib)?)?),
        None => Err(nothing()),
    }
}

enum Contents {
    Path(PathBuf),
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

/// The clipboard's contents, in the order of preference; the clipboard is
/// open.
fn read() -> Option<Contents> {
    let mut paths = Vec::new();
    if raw::get_file_list_path(&mut paths).is_ok()
        && let Some(path) = paths.into_iter().find(|p| p.exists())
    {
        return Some(Contents::Path(path));
    }
    for name in PNG_NAMES {
        let mut png = Vec::new();
        if let Some(format) = raw::register_format(name)
            && raw::get_vec(format.get(), &mut png).is_ok()
            && png.starts_with(b"\x89PNG")
        {
            return Some(Contents::Png(png));
        }
    }
    // Windows makes CF_DIB of a CF_DIBV5 and the other way round; V5 keeps
    // the alpha's mask.
    for format in [formats::CF_DIBV5, formats::CF_DIB] {
        let mut dib = Vec::new();
        if raw::get_vec(format, &mut dib).is_ok() && dib.len() > 40 {
            return Some(Contents::Dib(dib));
        }
    }
    let mut text = Vec::new();
    if raw::get_string(&mut text).is_ok() {
        // Explorer's "Copy as path" puts quotes around it.
        let text = String::from_utf8_lossy(&text);
        let path = PathBuf::from(text.trim().trim_matches('"'));
        if path.is_absolute() && path.exists() {
            return Some(Contents::Path(path));
        }
    }
    None
}

/// Where pasted images are saved: `qview` in the temporary folder (Windows'
/// Storage Sense clears it).
pub fn folder() -> PathBuf {
    std::env::temp_dir().join("qview")
}

/// Save `png` into [`folder`] under a new name of the local time.
fn save(png: &[u8]) -> Result<PathBuf, String> {
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = crate::win::local_stamp();
    let name = edit::suggested_name(Path::new(&format!("Clipboard {stamp}")), "png", "", |n| dir.join(n).exists());
    let path = dir.join(name);
    std::fs::write(&path, png).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `img` as a PNG, quickly: a screenshot's is made in tens of milliseconds
/// rather than seconds.
fn fast_png(img: &DynamicImage) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    img.write_with_encoder(PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive))
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// `img` as a `CF_DIBV5`: a BITMAPV5HEADER, 32 bits per pixel with masks
/// (BGRA, straight alpha), sRGB, rows from the bottom up, as most programs
/// expect.
fn dib_v5(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity(124 + (w * h * 4) as usize);
    let u32s = |out: &mut Vec<u8>, values: &[u32]| values.iter().for_each(|v| out.extend(v.to_le_bytes()));
    u32s(&mut out, &[124, w, h]);
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    // BI_BITFIELDS, the size, no resolution, no palette.
    u32s(&mut out, &[3, w * h * 4, 0, 0, 0, 0]);
    // The red, green, blue and alpha masks; LCS_sRGB.
    u32s(&mut out, &[0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000, 0x7352_4742]);
    // Endpoints and gammas, unused with sRGB.
    out.extend([0; 36 + 12]);
    // LCS_GM_IMAGES, no profile.
    u32s(&mut out, &[4, 0, 0, 0]);
    debug_assert_eq!(out.len(), 124);
    for row in img.rows().rev() {
        for p in row {
            let [r, g, b, a] = p.0;
            out.extend([b, g, r, a]);
        }
    }
    out
}

/// A clipboard DIB (a BITMAPINFOHEADER or a later one, then the colour
/// masks or palette, then the pixels) decoded as the BMP file it would be.
fn from_dib(dib: &[u8]) -> Result<DynamicImage, String> {
    let bad = || tr!("The clipboard's image cannot be read", "Не удалось прочитать изображение из буфера обмена").to_string();
    let u32_at = |i: usize| dib.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")));
    let header = u32_at(0).ok_or_else(bad)? as usize;
    let bits = dib.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(bad)?;
    let compression = u32_at(16).ok_or_else(bad)?;
    let colours = u32_at(32).ok_or_else(bad)? as usize;
    // BI_BITFIELDS and BI_ALPHABITFIELDS put their masks after a 40-byte
    // header; later headers hold them.
    let masks = match (header, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let palette = 4 * if bits <= 8 && colours == 0 { 1 << bits } else { colours };
    let offset = 14 + header + masks + palette;
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend(b"BM");
    bmp.extend(((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend([0; 4]);
    bmp.extend((offset as u32).to_le_bytes());
    bmp.extend(dib);
    image::load_from_memory_with_format(&bmp, ImageFormat::Bmp).map_err(|e| format!("{}: {e}", bad()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn dib_v5_reads_back() {
        let img = RgbaImage::from_fn(3, 2, |x, y| Rgba([x as u8 * 80, y as u8 * 120, 7, 255 - x as u8 * 100]));
        let back = from_dib(&dib_v5(&img)).unwrap().into_rgba8();
        assert_eq!(back, img);
    }

    #[test]
    fn plain_dib_reads_back() {
        // A 24-bit BITMAPINFOHEADER DIB, as Windows makes of a screenshot:
        // rows from the bottom up, padded to 4 bytes.
        let (w, h) = (2u32, 2u32);
        let mut dib = Vec::new();
        for v in [40, w, h] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes());
        dib.extend(24u16.to_le_bytes());
        for v in [0u32, 16, 0, 0, 0, 0] {
            dib.extend(v.to_le_bytes());
        }
        // Bottom row: blue, white; top row: red, green (BGR).
        dib.extend([255, 0, 0, 255, 255, 255, 0, 0]);
        dib.extend([0, 0, 255, 0, 255, 0, 0, 0]);
        let img = from_dib(&dib).unwrap().into_rgb8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0]);
        assert_eq!(img.get_pixel(0, 1).0, [0, 0, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [255, 255, 255]);
    }

    #[test]
    fn broken_dibs_are_refused() {
        assert!(from_dib(&[]).is_err());
        assert!(from_dib(&[40, 0, 0, 0, 1, 0]).is_err());
    }

    #[test]
    fn fast_png_reads_back() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(5, 4, Rgba([1, 2, 3, 4])));
        let png = fast_png(&img).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap(), img);
    }
}
