//! Saving an image turned and cropped (see `crop` for the frame).
//!
//! A JPEG that is only turned keeps its compressed data: its EXIF
//! orientation is changed instead (and the XMP one, if it has one), so
//! nothing is lost. Anything else is decoded, turned, cropped and encoded
//! anew by the `image` crate, with the ICC profile and the EXIF data of the
//! original (the orientation reset, the dimensions updated, the thumbnail
//! dropped). A file is never written in place: the new data goes to a
//! temporary file, which then replaces the old one (`ReplaceFileW`, which
//! keeps its creation date and permissions). The old contents come back so
//! that Ctrl+Z can restore them.

use std::fs::File;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use image::{DynamicImage, ImageEncoder, ImageFormat, ImageReader, metadata::Orientation};

/// Quality of JPEGs encoded anew: little visible loss, files about the size
/// of a camera's.
const JPEG_QUALITY: u8 = 92;

/// The formats an image can be saved in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    /// Lossless only: the `image` crate has no lossy WebP encoder.
    WebP,
    Bmp,
    Tiff,
}

impl Format {
    /// In the order the Save As dialog offers them.
    pub const ALL: [Format; 5] = [Format::Jpeg, Format::Png, Format::WebP, Format::Tiff, Format::Bmp];

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Format::Jpeg => &["jpg", "jpeg", "jpe", "jfif"],
            Format::Png => &["png"],
            Format::WebP => &["webp"],
            Format::Bmp => &["bmp", "dib"],
            Format::Tiff => &["tif", "tiff"],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::WebP => tr!("WebP (lossless)", "WebP (без потерь)"),
            Format::Bmp => "BMP",
            Format::Tiff => "TIFF",
        }
    }

    /// The format of `path`'s extension.
    pub fn of(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Format::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }
}

/// `path` can be saved over in its own format: one of `Format`, and for a
/// WebP a lossless one (a lossy one would come back several times larger).
pub fn can_overwrite(path: &Path) -> bool {
    match Format::of(path) {
        Some(Format::WebP) => std::fs::read(path).is_ok_and(|b| webp_lossless(&b)),
        Some(_) => true,
        None => false,
    }
}

/// A WebP whose image is lossless (a "VP8L" chunk, not "VP8 ").
fn webp_lossless(b: &[u8]) -> bool {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return false;
    }
    let mut i = 12;
    while let Some(head) = b.get(i..i + 8) {
        match &head[..4] {
            b"VP8L" => return true,
            b"VP8 " | b"ANIM" => return false,
            _ => {}
        }
        let len = u32::from_le_bytes(head[4..].try_into().expect("4 bytes")) as usize;
        // Chunks are padded to an even length.
        i += 8 + len + len % 2;
    }
    false
}

/// The file name Save As suggests for `path` saved with the extension
/// `ext`: its name with `suffix` ("_crop", "_rotate"; English whatever the
/// language, as file names go), which a copy made before already has
/// ("photo_crop" or "photo_crop_2" gets no second one), numbered "_2",
/// "_3"… past the names `taken` in its folder, so that nothing is
/// overwritten unasked.
pub fn suggested_name(path: &Path, ext: &str, suffix: &str, taken: impl Fn(&str) -> bool) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    // "photo_crop_2" is "photo_crop" numbered.
    let unnumbered = match stem.rsplit_once('_') {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => stem.as_str(),
    };
    let base = if suffix.is_empty() {
        stem.clone()
    } else if unnumbered.ends_with(suffix) {
        unnumbered.to_string()
    } else {
        format!("{stem}{suffix}")
    };
    (1..)
        .map(|n| if n == 1 { format!("{base}.{ext}") } else { format!("{base}_{n}.{ext}") })
        .find(|name| !taken(name))
        .expect("the numbers go on")
}

/// What to save: `src` turned by `turns` clockwise quarter turns, then
/// cropped to `crop` (x, y, width, height in the turned image's pixels),
/// into `dst`, which may be `src`.
#[derive(Clone, Debug)]
pub struct Job {
    pub src: PathBuf,
    pub dst: PathBuf,
    /// The upright size `crop` was chosen on: a file that has changed since
    /// is not cropped at the wrong place. Unused without a crop.
    pub size: [u32; 2],
    pub turns: u8,
    pub crop: Option<[u32; 4]>,
}

/// The contents of a file before it was saved over, for Ctrl+Z.
pub struct Before {
    pub bytes: Vec<u8>,
    pub modified: Option<SystemTime>,
}

pub struct Saved {
    /// None when `dst` did not exist.
    pub before: Option<Before>,
    /// Turned through the EXIF orientation, the pixels untouched.
    pub lossless: bool,
}

/// Carry out `job`. Windows' codecs may be needed to decode the source, so
/// the calling thread has COM initialised.
pub fn save(job: &Job) -> Result<Saved, String> {
    let format = Format::of(&job.dst).ok_or_else(|| {
        tr!("This format cannot be saved; JPEG, PNG, WebP, TIFF and BMP can", "Этот формат не сохраняется; можно JPEG, PNG, WebP, TIFF и BMP")
            .to_string()
    })?;
    let bytes = std::fs::read(&job.src).map_err(|e| e.to_string())?;
    let lossless = (job.crop.is_none() && format == Format::Jpeg && bytes.starts_with(&[0xff, 0xd8]))
        .then(|| turn_jpeg(&bytes, job.turns))
        .flatten();
    let (out, lossless) = match lossless {
        Some(out) => (out, true),
        None => (encode_anew(job, &bytes, format)?, false),
    };
    let before = if crate::folder::same_path(&job.src, &job.dst) {
        Some(Before { bytes, modified: modified(&job.src) })
    } else if job.dst.exists() {
        Some(Before { bytes: std::fs::read(&job.dst).map_err(|e| e.to_string())?, modified: modified(&job.dst) })
    } else {
        None
    };
    write_file(&job.dst, &out)?;
    Ok(Saved { before, lossless })
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Put `before` back into `path`, with its date.
pub fn restore(path: &Path, before: &Before) -> Result<(), String> {
    write_file(path, &before.bytes)?;
    if let Some(time) = before.modified {
        // The contents are back; an old date is only a nicety.
        let _ = File::options().write(true).open(path).and_then(|f| f.set_modified(time));
    }
    Ok(())
}

/// Write `data` into `path` through a temporary file beside it, which then
/// takes its place: a failure leaves the old file whole.
pub fn write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    let name = path.file_name().ok_or("no file name")?.to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.{}.qview-tmp", std::process::id()));
    let written = File::create(&tmp).and_then(|mut f| {
        f.write_all(data)?;
        f.sync_all()
    });
    let result = match written {
        Err(e) => Err(e.to_string()),
        Ok(()) if path.exists() => crate::win::replace_file(path, &tmp),
        Ok(()) => std::fs::rename(&tmp, path).map_err(|e| e.to_string()),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The EXIF orientation that shows an image turned `turns` more clockwise
/// quarter turns than `orientation` does.
pub fn turned_orientation(orientation: u8, turns: u8) -> u8 {
    // Each orientation as the `image` crate applies it: turned clockwise by
    // quarter turns, then flipped horizontally or not.
    const AS_TURNS: [(u8, bool); 8] =
        [(0, false), (0, true), (2, false), (2, true), (1, true), (1, false), (3, true), (3, false)];
    let (r, flip) = AS_TURNS[(orientation.clamp(1, 8) - 1) as usize];
    // Turning after a flip is the flip after turning the other way.
    let r = if flip { (r + 4 - turns % 4) % 4 } else { (r + turns) % 4 };
    AS_TURNS.iter().position(|&o| o == (r, flip)).expect("all eight are listed") as u8 + 1
}

/// The JPEG segments before the image data: (marker, offset of its 0xFF,
/// offset past its end).
fn jpeg_segments(b: &[u8]) -> Option<Vec<(u8, usize, usize)>> {
    let mut segments = Vec::new();
    let mut i = 2;
    loop {
        let start = i;
        if *b.get(i)? != 0xff {
            return None;
        }
        while *b.get(i)? == 0xff {
            i += 1;
        }
        let code = b[i];
        i += 1;
        if matches!(code, 0x01 | 0xd0..=0xd7) {
            continue;
        }
        if matches!(code, 0xd9 | 0xda) {
            return Some(segments);
        }
        let len = u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize;
        let end = i + len;
        if len < 2 || end > b.len() {
            return None;
        }
        segments.push((code, start, end));
        i = end;
    }
}

const EXIF_HEADER: &[u8] = b"Exif\0\0";
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// `jpeg` turned `turns` clockwise quarter turns by its orientation, its
/// compressed data as it is. None if its segments or its EXIF cannot be
/// read, or the EXIF would outgrow its segment.
pub fn turn_jpeg(jpeg: &[u8], turns: u8) -> Option<Vec<u8>> {
    let segments = jpeg_segments(jpeg)?;
    // The payload of an APP1 segment starts 4 bytes in.
    let exif = segments.iter().find(|&&(code, s, e)| code == 0xe1 && jpeg[s + 4..e].starts_with(EXIF_HEADER));
    let mut out = Vec::with_capacity(jpeg.len() + 64);
    let value = match exif {
        Some(&(_, s, e)) => {
            let tiff = &jpeg[s + 4 + EXIF_HEADER.len()..e];
            let value = turned_orientation(crate::exif::orientation(tiff).unwrap_or(1), turns);
            let tiff = crate::exif::with_orientation(tiff, value)?;
            out.extend(&jpeg[..s]);
            push_app1(&mut out, &tiff)?;
            out.extend(&jpeg[e..]);
            value
        }
        None => {
            let value = turned_orientation(1, turns);
            // After APP0 (JFIF), which must come first.
            let at = segments.iter().take_while(|s| s.0 == 0xe0).last().map_or(2, |s| s.2);
            out.extend(&jpeg[..at]);
            push_app1(&mut out, &crate::exif::minimal(value))?;
            out.extend(&jpeg[at..]);
            value
        }
    };
    set_xmp_orientation(&mut out, value);
    Some(out)
}

/// An APP1 segment with `tiff` as its EXIF; None if it is too long.
fn push_app1(out: &mut Vec<u8>, tiff: &[u8]) -> Option<()> {
    let len = u16::try_from(2 + EXIF_HEADER.len() + tiff.len()).ok()?;
    out.extend([0xff, 0xe1]);
    out.extend(len.to_be_bytes());
    out.extend(EXIF_HEADER);
    out.extend(tiff);
    Some(())
}

/// The orientation in the JPEG's XMP, if it has one, set to `value` (one
/// digit for another, so nothing moves). Programs that read XMP first
/// would otherwise turn the image back.
fn set_xmp_orientation(jpeg: &mut [u8], value: u8) {
    let Some(segments) = jpeg_segments(jpeg) else { return };
    for (code, s, e) in segments {
        if code != 0xe1 || !jpeg[s + 4..e].starts_with(XMP_HEADER) {
            continue;
        }
        let xmp = &mut jpeg[s + 4..e];
        for pattern in [&b"tiff:Orientation=\""[..], b"<tiff:Orientation>"] {
            let mut from = 0;
            while let Some(at) = xmp[from..].windows(pattern.len()).position(|w| w == pattern) {
                let digit = from + at + pattern.len();
                if xmp.get(digit).is_some_and(u8::is_ascii_digit) && !xmp.get(digit + 1).is_some_and(u8::is_ascii_digit) {
                    xmp[digit] = b'0' + value;
                }
                from = digit;
            }
        }
    }
}

/// Decode `bytes` (from `job.src`), turn and crop it, and encode it as
/// `format`.
fn encode_anew(job: &Job, bytes: &[u8], format: Format) -> Result<Vec<u8>, String> {
    if image::guess_format(bytes).is_ok_and(|f| crate::anim::is_animated(bytes, f)) {
        return Err(tr!("Animated images cannot be edited", "Анимированные изображения не редактируются").into());
    }
    let Decoded { img, icc, exif } = decode(&job.src, bytes)?;
    if job.crop.is_some() && [img.width(), img.height()] != job.size {
        return Err(tr!("The file has changed on disk; open it again", "Файл изменился на диске; откройте его заново").into());
    }
    let img = match job.turns % 4 {
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => img,
    };
    let img = match job.crop {
        Some([x, y, w, h]) if x + w <= img.width() && y + h <= img.height() && w > 0 && h > 0 => img.crop_imm(x, y, w, h),
        Some(_) => return Err("the crop is outside the image".into()),
        None => img,
    };
    let exif = exif.and_then(|e| crate::exif::for_new_pixels(&e, img.width(), img.height()));
    encode(&img, format, icc, exif).map_err(|e| e.to_string())
}

/// An image decoded upright, with the metadata carried into the new file.
struct Decoded {
    img: DynamicImage,
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
}

/// `src` decoded upright, with its ICC profile and EXIF data: by the
/// `image` crate, or by Windows' codecs (no metadata then).
fn decode(src: &Path, bytes: &[u8]) -> Result<Decoded, String> {
    let first = match (!crate::wic::takes(src)).then(|| decode_image(src, bytes)) {
        Some(Ok(decoded)) => return Ok(decoded),
        Some(Err(e)) => Some(e),
        None => None,
    };
    let (img, _) = crate::loader::read_wic(src, false).map_err(|e| first.unwrap_or(e))?;
    Ok(Decoded { img, icc: None, exif: None })
}

fn decode_image(src: &Path, bytes: &[u8]) -> Result<Decoded, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    if reader.format().is_none()
        && let Ok(format) = ImageFormat::from_path(src)
    {
        reader.set_format(format);
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(crate::loader::MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let icc = image::ImageDecoder::icc_profile(&mut decoder).ok().flatten();
    let exif = image::ImageDecoder::exif_metadata(&mut decoder).ok().flatten();
    let orientation = image::ImageDecoder::orientation(&mut decoder).unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(Decoded { img, icc, exif })
}

/// `img` in `format`, with `icc` and `exif` where the format holds them.
fn encode(img: &DynamicImage, format: Format, icc: Option<Vec<u8>>, exif: Option<Vec<u8>>) -> image::ImageResult<Vec<u8>> {
    use image::codecs::{bmp::BmpEncoder, jpeg::JpegEncoder, png::PngEncoder, tiff::TiffEncoder, webp::WebPEncoder};
    fn metadata(encoder: &mut impl ImageEncoder, icc: Option<Vec<u8>>, exif: Option<Vec<u8>>) {
        // Formats without a place for them refuse; the pixels still count.
        if let Some(icc) = icc {
            let _ = encoder.set_icc_profile(icc);
        }
        if let Some(exif) = exif {
            let _ = encoder.set_exif_metadata(exif);
        }
    }
    let mut out = Cursor::new(Vec::new());
    match format {
        Format::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
            metadata(&mut encoder, icc, exif);
            img.write_with_encoder(encoder)?;
        }
        Format::Png => {
            let mut encoder = PngEncoder::new(&mut out);
            metadata(&mut encoder, icc, exif);
            img.write_with_encoder(encoder)?;
        }
        Format::WebP => {
            let mut encoder = WebPEncoder::new_lossless(&mut out);
            metadata(&mut encoder, icc, exif);
            eight_bit(img).write_with_encoder(encoder)?;
        }
        Format::Tiff => {
            let mut encoder = TiffEncoder::new(&mut out);
            metadata(&mut encoder, icc, None);
            img.write_with_encoder(encoder)?;
        }
        Format::Bmp => {
            let mut encoder = BmpEncoder::new(&mut out);
            metadata(&mut encoder, icc, None);
            eight_bit(img).write_with_encoder(encoder)?;
        }
    }
    Ok(out.into_inner())
}

/// `img` with 8 bits per channel, as WebP and BMP take it.
fn eight_bit(img: &DynamicImage) -> std::borrow::Cow<'_, DynamicImage> {
    use image::ColorType::*;
    use std::borrow::Cow;
    match img.color() {
        L8 | La8 | Rgb8 | Rgba8 => Cow::Borrowed(img),
        L16 => Cow::Owned(img.to_luma8().into()),
        La16 => Cow::Owned(img.to_luma_alpha8().into()),
        c if c.has_alpha() => Cow::Owned(img.to_rgba8().into()),
        _ => Cow::Owned(img.to_rgb8().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_edit_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A 3x2 image whose every pixel differs, so that any turn or flip
    /// shows.
    fn asymmetric() -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 50, y as u8 * 100, 7])))
    }

    #[test]
    fn turning_composes_with_every_orientation() {
        for o in 1..=8u8 {
            for turns in 0..4u8 {
                let mut expected = asymmetric();
                expected.apply_orientation(Orientation::from_exif(o).unwrap());
                for _ in 0..turns {
                    expected = expected.rotate90();
                }
                let mut got = asymmetric();
                got.apply_orientation(Orientation::from_exif(turned_orientation(o, turns)).unwrap());
                assert_eq!(got, expected, "orientation {o}, {turns} turns");
            }
        }
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, _| image::Rgb([(x * 30) as u8, 90, 200])))
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        out
    }

    fn decoded(bytes: &[u8]) -> DynamicImage {
        decode_image(Path::new("x.jpg"), bytes).unwrap().img
    }

    #[test]
    fn a_turned_jpeg_keeps_its_image_data() {
        let plain = jpeg(40, 10);
        // Without EXIF: one is added.
        let turned = turn_jpeg(&plain, 1).unwrap();
        assert_eq!((decoded(&turned).width(), decoded(&turned).height()), (10, 40));
        let scan = |b: &[u8]| b[b.windows(2).position(|w| w == [0xff, 0xda]).unwrap()..].to_vec();
        assert_eq!(scan(&turned), scan(&plain));
        // With EXIF: changed in place; three more turns are a whole one.
        let back = turn_jpeg(&turned, 3).unwrap();
        assert_eq!(back.len(), turned.len());
        assert_eq!(decoded(&back).to_rgb8(), decoded(&plain).to_rgb8());
        let segments = jpeg_segments(&back).unwrap();
        assert_eq!(segments.iter().filter(|s| s.0 == 0xe1).count(), 1);
    }

    #[test]
    fn xmp_orientation_follows() {
        let plain = jpeg(8, 8);
        let xmp = b"<x:xmpmeta><rdf:Description tiff:Orientation=\"1\"/><tiff:Orientation>1</tiff:Orientation></x:xmpmeta>";
        let mut payload = XMP_HEADER.to_vec();
        payload.extend(xmp);
        let mut with_xmp = plain[..2].to_vec();
        with_xmp.extend([0xff, 0xe1]);
        with_xmp.extend(((payload.len() + 2) as u16).to_be_bytes());
        with_xmp.extend(&payload);
        with_xmp.extend(&plain[2..]);
        let turned = turn_jpeg(&with_xmp, 3).unwrap();
        let text = String::from_utf8_lossy(&turned);
        assert!(text.contains("tiff:Orientation=\"8\""), "{text}");
        assert!(text.contains("<tiff:Orientation>8<"), "{text}");
    }

    #[test]
    fn webp_kinds() {
        let mut lossless = Vec::new();
        asymmetric().write_to(&mut Cursor::new(&mut lossless), ImageFormat::WebP).unwrap();
        assert!(webp_lossless(&lossless));
        let mut lossy = b"RIFF\0\0\0\0WEBPVP8 \x04\0\0\0abcd".to_vec();
        assert!(!webp_lossless(&lossy));
        lossy[12..16].copy_from_slice(b"XXXX");
        assert!(!webp_lossless(&lossy));
        assert!(!webp_lossless(b"RIFF"));
    }

    #[test]
    fn suggested_names() {
        let free = |_: &str| false;
        let name = |path: &str, ext, suffix, taken: &[&str]| {
            suggested_name(Path::new(path), ext, suffix, |n| taken.contains(&n))
        };
        assert_eq!(suggested_name(Path::new("C:/p/photo.jpg"), "jpg", "_crop", free), "photo_crop.jpg");
        assert_eq!(name("photo.png", "png", "_rotate", &[]), "photo_rotate.png");
        // Taken: numbered.
        assert_eq!(name("photo.jpg", "jpg", "_crop", &["photo_crop.jpg", "photo_crop_2.jpg"]), "photo_crop_3.jpg");
        // A copy cropped again: no second suffix.
        assert_eq!(name("photo_crop.jpg", "jpg", "_crop", &["photo_crop.jpg"]), "photo_crop_2.jpg");
        assert_eq!(name("photo_crop_2.jpg", "jpg", "_crop", &["photo_crop.jpg", "photo_crop_2.jpg"]), "photo_crop_3.jpg");
        // A number that is not a copy's stays.
        assert_eq!(name("img_2024.jpg", "jpg", "_crop", &[]), "img_2024_crop.jpg");
        // Rotated after cropping is another edit.
        assert_eq!(name("photo_crop.jpg", "jpg", "_rotate", &[]), "photo_crop_rotate.jpg");
        // Only converted: the name as it is, unless taken.
        assert_eq!(name("photo.heic", "jpg", "", &[]), "photo.jpg");
        assert_eq!(name("photo.jpg", "jpg", "", &["photo.jpg"]), "photo_2.jpg");
    }

    #[test]
    fn formats_by_extension() {
        assert_eq!(Format::of(Path::new("a.JPG")), Some(Format::Jpeg));
        assert_eq!(Format::of(Path::new("a.tif")), Some(Format::Tiff));
        assert_eq!(Format::of(Path::new("a.heic")), None);
        assert_eq!(Format::of(Path::new("a")), None);
    }

    #[test]
    fn saves_turned_and_cropped_and_restores() {
        let dir = temp_dir("save");
        let src = dir.join("a.png");
        asymmetric().save(&src).unwrap();
        let original = std::fs::read(&src).unwrap();
        // Turned right the image is 2x3; its middle row.
        let job = Job { src: src.clone(), dst: src.clone(), size: [3, 2], turns: 1, crop: Some([0, 1, 2, 1]) };
        let saved = save(&job).unwrap();
        assert!(!saved.lossless);
        let expected = asymmetric().rotate90().crop_imm(0, 1, 2, 1);
        assert_eq!(image::open(&src).unwrap().to_rgb8(), expected.to_rgb8());
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let before = saved.before.unwrap();
        assert_eq!(before.bytes, original);
        restore(&src, &before).unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), original);

        // Into a new file, as JPEG: the turn is in the pixels.
        let dst = dir.join("b.jpg");
        let job = Job { src: src.clone(), dst: dst.clone(), size: [3, 2], turns: 2, crop: None };
        let saved = save(&job).unwrap();
        assert!(saved.before.is_none());
        assert_eq!((image::open(&dst).unwrap().width(), image::open(&dst).unwrap().height()), (3, 2));

        // A JPEG only turned stays as it was but for its orientation.
        let job = Job { src: dst.clone(), dst: dst.clone(), size: [3, 2], turns: 1, crop: None };
        assert!(save(&job).unwrap().lossless);
        assert_eq!(decoded(&std::fs::read(&dst).unwrap()).width(), 2);

        // A file changed since the crop was chosen is not cropped.
        let job = Job { src: src.clone(), dst: src.clone(), size: [30, 20], turns: 0, crop: Some([0, 0, 1, 1]) };
        assert!(save(&job).is_err());
        assert!(save(&Job { dst: dir.join("c.gif"), ..job }).is_err());
        // Without a crop the size is not needed (the gallery has none).
        let job = Job { src: src.clone(), dst: dir.join("d.bmp"), size: [0, 0], turns: 0, crop: None };
        assert!(save(&job).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exif_is_carried_with_the_orientation_reset() {
        let dir = temp_dir("exif");
        let src = dir.join("a.jpg");
        std::fs::write(&src, turn_jpeg(&jpeg(40, 10), 1).unwrap()).unwrap();
        // Upright it is 10x40; its top half.
        let job = Job { src: src.clone(), dst: src.clone(), size: [10, 40], turns: 0, crop: Some([0, 0, 10, 20]) };
        save(&job).unwrap();
        let bytes = std::fs::read(&src).unwrap();
        let img = decoded(&bytes);
        assert_eq!((img.width(), img.height()), (10, 20));
        let (_, s, e) = jpeg_segments(&bytes).unwrap().into_iter().find(|s| s.0 == 0xe1).expect("EXIF kept");
        assert_eq!(crate::exif::orientation(&bytes[s + 10..e]), Some(1));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
