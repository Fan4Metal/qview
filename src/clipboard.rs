//! Images through the clipboard. Copy Image (Ctrl+Shift+C) puts the
//! current image on it as it is shown (turned, mirrored, cropped while
//! cropping), decoded from its file at full size: as `CF_DIBV5`, from which
//! Windows makes `CF_DIB` and `CF_BITMAP` for older programs, and as PNG
//! too when it has transparency (browsers, Office and messengers keep the
//! alpha from it; a DIB's alpha is read in different ways). Paste (Ctrl+V)
//! opens what the clipboard holds: files copied in Explorer (the first),
//! a path as text, or an image, which is saved as a PNG into [`folder`]
//! and opened from there, so that it can be cropped and saved elsewhere
//! like any file. Copy (Ctrl+C) of images in an archive puts them on it
//! as files, unpacked into a temporary folder ([`copy_pages`]).

use std::io::Cursor;
use std::path::{Path, PathBuf};

use clipboard_win::{formats, raw};
use image::{DynamicImage, ImageFormat, ImageReader, RgbaImage};

use crate::edit;

/// The clipboard's names for PNG: Chrome and Firefox write "PNG", some
/// programs the MIME type.
const PNG_NAMES: [&str; 2] = ["PNG", "image/png"];

/// Put `job`'s image (see `edit::render`) on the clipboard. Windows' codecs
/// may decode it, so the calling thread has COM initialised. False when
/// something else was put on the clipboard while it was drawn (here or in
/// another program), which is left there.
pub fn copy_image(job: &edit::Job) -> Result<bool, String> {
    let sequence = crate::win::clipboard_sequence();
    let img = edit::render(job)?;
    // A DIB's size is a u32.
    if (img.width() as u64) * (img.height() as u64) * 4 > u32::MAX as u64 {
        return Err(tr!("the image is too large for the clipboard", "изображение слишком велико для буфера обмена").into());
    }
    let rgba = img.to_rgba8();
    let transparent = img.color().has_alpha() && rgba.pixels().any(|p| p.0[3] < 255);
    let png = if transparent { Some(fast_png(&img)?) } else { None };
    drop(img);
    let dib = dib_v5(&rgba);
    let _clipboard = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    // Open, nobody changes it now.
    if crate::win::clipboard_sequence() != sequence {
        return Ok(false);
    }
    raw::empty().map_err(|e| e.to_string())?;
    raw::set_without_clear(formats::CF_DIBV5, &dib).map_err(|e| e.to_string())?;
    if let Some(png) = png
        && let Some(format) = raw::register_format(PNG_NAMES[0])
    {
        raw::set_without_clear(format.get(), &png).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// Where the pages of an archive copied as files go ([`copy_pages`]), each
/// copy into a folder of its own: `qview-copy` in the temporary folder
/// (not inside [`folder`], which the gallery shows after a paste).
fn pages_folder() -> PathBuf {
    std::env::temp_dir().join("qview-copy")
}

/// Copies older than this are deleted before another is made: pasted
/// long since.
const KEEP_PAGES: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Put the images `paths` of an archive on the clipboard as files, as
/// Ctrl+C puts files on disk: unpacked into a new folder of
/// [`pages_folder`] under their own names, so that a paste into Explorer or
/// a message gives those; `progress` hears how many are unpacked. False,
/// and nothing left behind, when something else was put on the clipboard
/// meanwhile (a solid RAR's pages may take a while): that copy stays.
pub fn copy_pages(paths: &[PathBuf], mut progress: impl FnMut(usize)) -> Result<bool, String> {
    let sequence = crate::win::clipboard_sequence();
    let overtaken = || crate::win::clipboard_sequence() != sequence;
    // A solid RAR stays open for the pass, though another folder is shown.
    let _hold = crate::archive::hold(paths);
    let Some((dir, files)) = unpack(paths, &pages_folder(), &overtaken, &mut progress)? else {
        return Ok(false);
    };
    let put = crate::win::copy_files_since(&files, Some(sequence));
    if !matches!(put, Ok(true)) {
        let _ = std::fs::remove_dir_all(dir);
    }
    put
}

/// The files on the clipboard now (`CF_HDROP`), if any.
fn files_on_clipboard() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(_clipboard) = clipboard_win::Clipboard::new_attempts(10) {
        let _ = raw::get_file_list_path(&mut paths);
    }
    paths
}

/// `name` as Windows takes a file name: the characters it refuses (from
/// an archive made elsewhere: `?`, `:`, which would write an alternate
/// stream) as `_`, no dot or space at the end, a device's name (`aux.png`)
/// with `_` before it.
fn windows_name(name: &str) -> String {
    let name: String = name.chars().map(|c| if c < ' ' || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let name = name.trim_end_matches(['.', ' ']);
    match crate::folder::check_name(name) {
        Ok(()) => name.to_string(),
        Err(_) if name.is_empty() => "_".to_string(),
        Err(_) => format!("_{name}"),
    }
}

/// `paths` (images in an archive, or files) written into a new folder of
/// `root` (named by the local time) under their names ([`windows_name`]),
/// numbered past one of the same name from another of the archive's
/// folders; the copies in `root` older than [`KEEP_PAGES`] deleted first.
/// The folder and its files; None when `stop` said so before the end. The
/// folder is removed again when it is not finished.
fn unpack(
    paths: &[PathBuf],
    root: &Path,
    stop: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize),
) -> Result<Option<(PathBuf, Vec<PathBuf>)>, String> {
    if let Ok(entries) = std::fs::read_dir(root) {
        let now = std::time::SystemTime::now();
        // Not those still on the clipboard, however old.
        let pasting = files_on_clipboard();
        for entry in entries.flatten() {
            let age = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok());
            if age.is_some_and(|age| age > KEEP_PAGES) && !pasting.iter().any(|p| p.starts_with(entry.path())) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    // Two copies within a second each get a folder.
    let stamp = crate::win::local_stamp();
    let dir = (1..)
        .map(|n| root.join(if n == 1 { stamp.clone() } else { format!("{stamp} ({n})") }))
        .take(100)
        .find(|dir| std::fs::create_dir(dir).is_ok())
        .ok_or_else(|| format!("cannot make a folder in {}", root.display()))?;
    let filled = fill(paths, &dir, stop, progress);
    match filled {
        Ok(Some(files)) => Ok(Some((dir, files))),
        other => {
            let _ = std::fs::remove_dir_all(&dir);
            other.map(|_| None)
        }
    }
}

/// [`unpack`]'s writing of the files into `dir`.
fn fill(paths: &[PathBuf], dir: &Path, stop: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize)) -> Result<Option<Vec<PathBuf>>, String> {
    let mut files = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        if stop() {
            return Ok(None);
        }
        let shown = crate::app::file_name(path);
        let bytes = crate::archive::read(path).map_err(|e| format!("{shown}: {e}"))?;
        let safe = PathBuf::from(windows_name(&shown));
        let ext = safe.extension().unwrap_or_default().to_string_lossy().into_owned();
        let name = edit::suggested_name(&safe, &ext, "", |n| dir.join(n).exists());
        let file = dir.join(&name);
        std::fs::write(&file, bytes).map_err(|e| format!("{name}: {e}"))?;
        files.push(file);
        progress(i + 1);
    }
    Ok(Some(files))
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
    // As large as any file qview opens, not the decoder's default.
    let mut reader = ImageReader::with_format(Cursor::new(&bmp), ImageFormat::Bmp);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(crate::loader::MAX_ALLOC);
    reader.limits(limits);
    reader.decode().map_err(|e| format!("{}: {e}", bad()))
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

    #[test]
    fn pages_unpacked_under_their_names() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("qview_copy_pages_{}", std::process::id()));
        let root = dir.join("copies");
        std::fs::create_dir_all(&dir).unwrap();
        let cbz = dir.join("book.cbz");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&cbz).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in [("ch1/001.png", b"one"), ("ch2/001.png", b"two")] {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        // A copy of yesterday is deleted; one of now is kept.
        let old = root.join("old");
        std::fs::create_dir_all(&old).unwrap();
        let yesterday = std::time::SystemTime::now() - KEEP_PAGES - std::time::Duration::from_secs(60);
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_WRITE_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS: a folder.
            let handle = std::fs::OpenOptions::new().access_mode(0x100).custom_flags(0x0200_0000).open(&old).unwrap();
            handle.set_modified(yesterday).unwrap();
        }
        let pages = [cbz.join(r"ch1\001.png"), cbz.join(r"ch2\001.png")];
        let unpack = |pages: &[PathBuf], stop: &dyn Fn() -> bool| {
            let mut counted = 0;
            let done = unpack(pages, &root, stop, &mut |n| counted = n);
            done.map(|d| d.map(|(dir, files)| (dir, files, counted)))
        };
        let (first_dir, first, counted) = unpack(&pages, &|| false).unwrap().unwrap();
        assert!(!old.exists());
        assert_eq!(counted, 2);
        let names: Vec<_> = first.iter().map(|p| crate::app::file_name(p)).collect();
        assert_eq!(names, ["001.png", "001_2.png"]);
        assert_eq!(std::fs::read(&first[1]).unwrap(), b"two");
        // Another copy at once: a folder of its own.
        let (_, second, _) = unpack(&pages[..1], &|| false).unwrap().unwrap();
        assert_ne!(second[0].parent(), Some(first_dir.as_path()));
        assert_eq!(crate::app::file_name(&second[0]), "001.png");
        // A failure or a stop leaves no folder behind.
        let folders = || std::fs::read_dir(&root).unwrap().count();
        let before = folders();
        assert!(unpack(&[pages[0].clone(), cbz.join("missing.png")], &|| false).is_err());
        assert!(unpack(&pages, &|| true).unwrap().is_none());
        assert_eq!(folders(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_windows_takes() {
        assert_eq!(windows_name("001.png"), "001.png");
        assert_eq!(windows_name("What?.jpg"), "What_.jpg");
        assert_eq!(windows_name("Chapter 1: Start.jpg"), "Chapter 1_ Start.jpg");
        assert_eq!(windows_name("aux.png"), "_aux.png");
        assert_eq!(windows_name("a <b>.png. "), "a _b_.png");
    }
}
