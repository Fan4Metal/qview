//! Images saved as one PDF, a page each (File → Save as PDF,
//! `App::save_pdf`).
//!
//! The PDF is written here by hand: a page per image, as large as the
//! image at the resolution its file asks to be printed at (96 dpi when it
//! names none), a bookmark per page with the file's name. A JPEG goes in as
//! it is (`DCTDecode`, turned by its EXIF orientation through the page's
//! `/Rotate`), and so does the compressed data of a PNG without
//! transparency (`FlateDecode` with the PNG predictors): nothing is decoded
//! or lost, and such a page costs about as much as copying its file.
//! Anything else is decoded (`edit::decode`), laid on white where it is
//! transparent and encoded: a lossy source as a JPEG, a lossless one as PNG
//! data. One page is in memory at a time; the PDF is written into a
//! temporary file beside it, which takes its place once it is whole.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

/// The resolution of an image whose file names none, as Windows takes it.
const DPI: f64 = 96.0;
/// The longest side of a page, in points: Acrobat's limit, 200 inches.
const MAX_SIDE: f64 = 14400.0;
/// Of the images encoded anew, as `edit` saves them.
const JPEG_QUALITY: u8 = 92;

/// What to make: `pages` in this order, into `dst`, titled `title`.
pub struct Job {
    pub pages: Vec<PathBuf>,
    pub dst: PathBuf,
    pub title: String,
}

/// What the thread making a PDF reports.
pub enum Progress {
    /// So many pages are written.
    Pages(usize),
    /// Finished: the PDF's size; or why not (empty when stopped), nothing
    /// left behind.
    Done(Result<u64, String>),
}

/// Make the PDF of `job`, calling `progress` with the pages written after
/// each; `stop` set gives up (an empty error). Windows' codecs may decode
/// the images, so the calling thread has COM initialised.
pub fn make(job: &Job, stop: &AtomicBool, progress: impl Fn(usize)) -> Result<u64, String> {
    let name = job.dst.file_name().ok_or_else(|| tr!("no file name", "нет имени файла").to_string())?.to_string_lossy();
    let tmp = job.dst.with_file_name(format!("{name}.{}.qview-tmp", std::process::id()));
    let result = write_into(&tmp, job, stop, &progress).and_then(|size| {
        let placed = if job.dst.exists() { crate::win::replace_file(&job.dst, &tmp) } else { std::fs::rename(&tmp, &job.dst) };
        placed.map(|()| size).map_err(|e| e.to_string())
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_into(tmp: &Path, job: &Job, stop: &AtomicBool, progress: &dyn Fn(usize)) -> Result<u64, String> {
    let file = File::create(tmp).map_err(|e| e.to_string())?;
    let mut pdf = Pdf::new(BufWriter::with_capacity(1 << 20, file)).map_err(|e| e.to_string())?;
    // A solid RAR's pages unpacked once for the pass.
    let _hold = crate::archive::hold(&job.pages);
    for (i, path) in job.pages.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            return Err(String::new());
        }
        let name = page_name(path);
        let image = page(path).map_err(|e| format!("{name}: {e}"))?;
        pdf.page(&image, &name).map_err(|e| e.to_string())?;
        progress(i + 1);
    }
    let (out, size) = pdf.finish(&job.title).map_err(|e| e.to_string())?;
    let file = out.into_inner().map_err(|e| e.error().to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(size)
}

/// The bookmark of `path`'s page: its file name (in an archive, the
/// name within it).
fn page_name(path: &Path) -> String {
    match crate::archive::split(path) {
        Some((_, inner)) => inner.rsplit(['\\', '/']).next().unwrap_or(&inner).to_string(),
        None => path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
    }
}

/// Colours of an image in the PDF.
#[derive(Clone, Debug, PartialEq)]
enum Colour {
    Gray,
    Rgb,
    /// A PNG's palette, RGB triples.
    Indexed(Vec<u8>),
}

impl Colour {
    /// Components of a pixel in the image data.
    fn channels(&self) -> u8 {
        if *self == Colour::Rgb { 3 } else { 1 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Filter {
    /// A JPEG file.
    Dct,
    /// A PNG's image data: zlib, each row with its predictor.
    Flate,
}

/// A page's image, its data compressed as the PDF takes it.
struct Image {
    width: u32,
    height: u32,
    colour: Colour,
    bits: u8,
    filter: Filter,
    data: Vec<u8>,
    icc: Option<Vec<u8>>,
    /// Degrees clockwise the page is turned when shown.
    rotate: u16,
    /// Dots per inch across and down.
    dpi: (f64, f64),
}

impl Image {
    /// The page's width and height in points, as the image is stored (the
    /// page's `/Rotate` turns it), within `MAX_SIDE`.
    fn page_size(&self) -> (f64, f64) {
        let (w, h) = (f64::from(self.width) * 72.0 / self.dpi.0, f64::from(self.height) * 72.0 / self.dpi.1);
        let k = (MAX_SIDE / w.max(h)).min(1.0);
        (w * k, h * k)
    }
}

/// The page of the image in `path` (a file, or an image in an archive).
fn page(path: &Path) -> Result<Image, String> {
    let bytes = crate::archive::read(path).map_err(|e| e.to_string())?;
    let (dpi, icc) = crate::info::print_metadata(&bytes);
    let dpi = resolution(dpi);
    match jpeg(&bytes).or_else(|| png(&bytes)) {
        Some(image) => Ok(Image { icc, dpi, ..image }),
        None => decoded(path, &bytes, dpi),
    }
}

/// The resolution of a page from the file's; 96 dpi when it names none or
/// an absurd one.
fn resolution(dpi: Option<(f64, f64)>) -> (f64, f64) {
    let sane = |d: f64| (1.0..=100_000.0).contains(&d);
    match dpi {
        Some((x, y)) if sane(x) && sane(y) => (x, y),
        _ => (DPI, DPI),
    }
}

/// A JPEG that a PDF reader decodes as it is: baseline, extended or
/// progressive with Huffman coding, 8 bits, grey or colour, its EXIF
/// orientation a turn (a mirrored one is decoded). None otherwise.
fn jpeg(bytes: &[u8]) -> Option<Image> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let (mut rotate, mut exif, mut sof) = (0, false, None);
    for (code, _, p, e) in crate::edit::jpeg_segments(bytes)? {
        let data = &bytes[p..e];
        match code {
            0xe1 if !exif && data.starts_with(b"Exif\0\0") => {
                exif = true;
                rotate = match crate::exif::orientation(&data[6..]).unwrap_or(1) {
                    1 => 0,
                    3 => 180,
                    6 => 90,
                    8 => 270,
                    _ => return None,
                };
            }
            // SOFn, but DHT, JPG and DAC, which share the range.
            0xc0..=0xcf if !matches!(code, 0xc4 | 0xc8 | 0xcc) && sof.is_none() => sof = Some((code, data)),
            _ => {}
        }
    }
    let (code, s) = sof?;
    if !matches!(code, 0xc0..=0xc2) || s.len() < 6 || s[0] != 8 {
        return None;
    }
    let height = u32::from(u16::from_be_bytes([s[1], s[2]]));
    let width = u32::from(u16::from_be_bytes([s[3], s[4]]));
    let colour = match s[5] {
        1 => Colour::Gray,
        3 => Colour::Rgb,
        // CMYK, whose inversion readers disagree on: decoded.
        _ => return None,
    };
    // A height of 0 is given later (DNL), which readers may not follow.
    if width == 0 || height == 0 {
        return None;
    }
    Some(Image { width, height, colour, bits: 8, filter: Filter::Dct, data: bytes.to_vec(), icc: None, rotate, dpi: (DPI, DPI) })
}

/// A PNG whose image data a PDF reader decodes as it is: not interlaced,
/// no alpha channel and no transparent colour (`tRNS`). None otherwise.
fn png(bytes: &[u8]) -> Option<Image> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let (mut header, mut palette, mut data) = (None, None, Vec::new());
    let mut at = 8usize;
    loop {
        let len = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        let body = bytes.get(at + 8..(at + 8).checked_add(len)?)?;
        match kind {
            b"IHDR" if body.len() == 13 => header = Some(body),
            b"PLTE" => palette = Some(body),
            b"tRNS" => return None,
            b"IDAT" => data.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let h = header?;
    let width = u32::from_be_bytes(h[0..4].try_into().ok()?);
    let height = u32::from_be_bytes(h[4..8].try_into().ok()?);
    let (bits, kind) = (h[8], h[9]);
    // Compression 0, filtering 0, not interlaced.
    if h[10] != 0 || h[11] != 0 || h[12] != 0 || width == 0 || height == 0 || data.is_empty() {
        return None;
    }
    let colour = match (kind, bits) {
        (0, 1 | 2 | 4 | 8 | 16) => Colour::Gray,
        (2, 8 | 16) => Colour::Rgb,
        (3, 1 | 2 | 4 | 8) => Colour::Indexed(palette.filter(|p| !p.is_empty() && p.len() % 3 == 0 && p.len() <= 768)?.to_vec()),
        // With an alpha channel: decoded and laid on white.
        _ => return None,
    };
    Some(Image { width, height, colour, bits, filter: Filter::Flate, data, icc: None, rotate: 0, dpi: (DPI, DPI) })
}

/// `bytes` from `path` decoded upright, laid on white, and encoded as a
/// JPEG when it was lossy, else as PNG data.
fn decoded(path: &Path, bytes: &[u8], dpi: (f64, f64)) -> Result<Image, String> {
    let lossy = lossy(path, bytes);
    let decoded = crate::edit::decode(path, bytes)?;
    let eight = crate::edit::eight_bit(&decoded.img);
    let flat = crate::edit::on_white(&eight);
    let mut out = Vec::new();
    let image = if lossy {
        flat.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)).map_err(|e| e.to_string())?;
        jpeg(&out)
    } else {
        flat.write_with_encoder(PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive))
            .map_err(|e| e.to_string())?;
        png(&out)
    };
    let image = image.ok_or_else(|| "the image could not be encoded".to_string())?;
    Ok(Image { icc: decoded.icc, dpi, ..image })
}

/// The image in `path` was compressed with loss: a JPEG, a lossy WebP, an
/// AVIF, or what Windows' codecs or libheif read (HEIC, RAW, JPEG XR).
fn lossy(path: &Path, bytes: &[u8]) -> bool {
    use image::ImageFormat;
    if crate::wic::takes(path) || crate::heif::takes(path) {
        return true;
    }
    match image::guess_format(bytes) {
        Ok(ImageFormat::Jpeg | ImageFormat::Avif) => true,
        Ok(ImageFormat::WebP) => !crate::edit::webp_lossless(bytes),
        _ => false,
    }
}

/// Components of the colours of an ICC profile, for those a page can
/// have.
fn icc_channels(icc: &[u8]) -> Option<u8> {
    match icc.get(16..20)? {
        b"RGB " => Some(3),
        b"GRAY" => Some(1),
        _ => None,
    }
}

/// A number as PDF writes it: at most three decimals, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// `s` as a PDF text string: UTF-16BE with its byte order mark, in hex.
fn text(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in s.encode_utf16() {
        out += &format!("{unit:04X}");
    }
    out + ">"
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

const CATALOG: u32 = 1;
const PAGES: u32 = 2;
const OUTLINES: u32 = 3;
const INFO: u32 = 4;

/// A PDF being written: the objects of each page as it comes, those that
/// refer to all the pages at the end.
struct Pdf<W: Write> {
    out: W,
    /// Bytes written.
    at: u64,
    /// Where each object starts, by its number less one.
    offsets: Vec<u64>,
    /// Each page's object and bookmark.
    pages: Vec<(u32, String)>,
    /// The ICC profiles written, shared by the pages that have them.
    profiles: HashMap<Vec<u8>, u32>,
}

impl<W: Write> Pdf<W> {
    fn new(out: W) -> io::Result<Self> {
        let mut pdf = Pdf { out, at: 0, offsets: vec![0; INFO as usize], pages: Vec::new(), profiles: HashMap::new() };
        // Bytes above 127 tell programs that the file is binary.
        pdf.write(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")?;
        Ok(pdf)
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.at += bytes.len() as u64;
        Ok(())
    }

    /// The number of a new object.
    fn id(&mut self) -> u32 {
        self.offsets.push(0);
        self.offsets.len() as u32
    }

    fn start(&mut self, id: u32) -> io::Result<()> {
        self.offsets[id as usize - 1] = self.at;
        self.write(format!("{id} 0 obj\n").as_bytes())
    }

    fn object(&mut self, id: u32, body: &str) -> io::Result<()> {
        self.start(id)?;
        self.write(body.as_bytes())?;
        self.write(b"\nendobj\n")
    }

    /// A stream of `data` whose dictionary has `dict` besides its length.
    fn stream(&mut self, id: u32, dict: &str, data: &[u8]) -> io::Result<()> {
        self.start(id)?;
        self.write(format!("<<{dict} /Length {}>>\nstream\n", data.len()).as_bytes())?;
        self.write(data)?;
        self.write(b"\nendstream\nendobj\n")
    }

    /// A page of `image`, bookmarked as `title`.
    fn page(&mut self, image: &Image, title: &str) -> io::Result<()> {
        let channels = image.colour.channels();
        let profile = image.icc.as_deref().filter(|icc| icc_channels(icc) == Some(channels));
        let colour = match (&image.colour, profile) {
            (Colour::Indexed(palette), _) => format!("[/Indexed /DeviceRGB {} <{}>]", palette.len() / 3 - 1, hex(palette)),
            (_, Some(icc)) => {
                let id = match self.profiles.get(icc) {
                    Some(&id) => id,
                    None => {
                        let id = self.id();
                        self.stream(id, &format!("/N {channels}"), icc)?;
                        self.profiles.insert(icc.to_vec(), id);
                        id
                    }
                };
                format!("[/ICCBased {id} 0 R]")
            }
            (Colour::Gray, None) => "/DeviceGray".into(),
            (Colour::Rgb, None) => "/DeviceRGB".into(),
        };
        let filter = match image.filter {
            Filter::Dct => "/Filter /DCTDecode".to_string(),
            Filter::Flate => format!(
                "/Filter /FlateDecode /DecodeParms <</Predictor 15 /Colors {channels} /BitsPerComponent {} /Columns {}>>",
                image.bits, image.width
            ),
        };
        let xobject = self.id();
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {colour} /BitsPerComponent {} {filter}",
            image.width, image.height, image.bits
        );
        self.stream(xobject, &dict, &image.data)?;
        let (w, h) = image.page_size();
        let (w, h) = (num(w), num(h));
        let contents = self.id();
        self.stream(contents, "", format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q").as_bytes())?;
        let page = self.id();
        let rotate = if image.rotate == 0 { String::new() } else { format!(" /Rotate {}", image.rotate) };
        self.object(
            page,
            &format!(
                "<</Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {w} {h}]{rotate} /Resources <</XObject <</Im0 {xobject} 0 R>>>> /Contents {contents} 0 R>>"
            ),
        )?;
        self.pages.push((page, title.to_string()));
        Ok(())
    }

    /// The page tree, the bookmarks, the catalog, the document's title, and
    /// the cross-reference table: the PDF is whole, of the size given.
    fn finish(mut self, title: &str) -> io::Result<(W, u64)> {
        let pages = std::mem::take(&mut self.pages);
        let n = pages.len();
        let kids: Vec<String> = pages.iter().map(|(id, _)| format!("{id} 0 R")).collect();
        self.object(PAGES, &format!("<</Type /Pages /Kids [{}] /Count {n}>>", kids.join(" ")))?;
        let first = self.offsets.len() as u32 + 1;
        for (k, (page, name)) in pages.iter().enumerate() {
            let id = self.id();
            let prev = if k > 0 { format!(" /Prev {} 0 R", id - 1) } else { String::new() };
            let next = if k + 1 < n { format!(" /Next {} 0 R", id + 1) } else { String::new() };
            self.object(id, &format!("<</Title {} /Parent {OUTLINES} 0 R{prev}{next} /Dest [{page} 0 R /Fit]>>", text(name)))?;
        }
        let outlines = match n {
            0 => "<</Type /Outlines /Count 0>>".to_string(),
            _ => format!("<</Type /Outlines /First {first} 0 R /Last {} 0 R /Count {n}>>", first + n as u32 - 1),
        };
        self.object(OUTLINES, &outlines)?;
        self.object(CATALOG, &format!("<</Type /Catalog /Pages {PAGES} 0 R /Outlines {OUTLINES} 0 R>>"))?;
        let producer = text(&format!("qview {}", crate::VERSION));
        self.object(INFO, &format!("<</Title {} /Producer {producer}>>", text(title)))?;
        let xref = self.at;
        let count = self.offsets.len() + 1;
        let mut table = format!("xref\n0 {count}\n0000000000 65535 f\r\n");
        for offset in &self.offsets {
            table += &format!("{offset:010} 00000 n\r\n");
        }
        table += &format!("trailer\n<</Size {count} /Root {CATALOG} 0 R /Info {INFO} 0 R>>\nstartxref\n{xref}\n%%EOF\n");
        self.write(table.as_bytes())?;
        Ok((self.out, self.at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat};
    use std::io::Cursor;

    fn encoded(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), format).unwrap();
        out
    }

    fn rgb(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 20) as u8, (y * 30) as u8, 90])))
    }

    /// The objects of `pdf` by number, as the cross-reference table finds
    /// them: each offset must start "N 0 obj".
    fn objects(pdf: &[u8]) -> Vec<Vec<u8>> {
        let tail = String::from_utf8_lossy(&pdf[pdf.len() - 40..]);
        let start: usize = tail.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[start..].starts_with(b"xref\n"));
        let table = String::from_utf8_lossy(&pdf[start..]);
        let mut lines = table.lines().skip(1);
        let count: usize = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        let offsets: Vec<usize> = lines.skip(1).take(count - 1).map(|l| l[..10].parse().unwrap()).collect();
        offsets
            .iter()
            .enumerate()
            .map(|(i, &at)| {
                let head = format!("{} 0 obj\n", i + 1);
                assert!(pdf[at..].starts_with(head.as_bytes()), "object {}", i + 1);
                let end = at + pdf[at..].windows(7).position(|w| w == b"endobj\n").unwrap();
                pdf[at + head.len()..end].to_vec()
            })
            .collect()
    }

    /// The data of a stream object.
    fn stream_data(object: &[u8]) -> &[u8] {
        let from = object.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let to = object.windows(10).rposition(|w| w == b"\nendstream").unwrap();
        &object[from..to]
    }

    fn contains(object: &[u8], what: &str) -> bool {
        object.windows(what.len()).any(|w| w == what.as_bytes())
    }

    #[test]
    fn a_jpeg_goes_in_as_it_is() {
        let file = encoded(rgb(40, 30), ImageFormat::Jpeg);
        let image = jpeg(&file).unwrap();
        assert_eq!((image.width, image.height, image.colour.clone(), image.rotate), (40, 30, Colour::Rgb, 0));
        // Turned by its orientation, a mirrored one decoded.
        let turned = crate::edit::turn_jpeg(&file, 1, false).unwrap();
        assert_eq!(jpeg(&turned).unwrap().rotate, 90);
        assert_eq!(jpeg(&crate::edit::turn_jpeg(&file, 3, false).unwrap()).unwrap().rotate, 270);
        assert!(jpeg(&crate::edit::turn_jpeg(&file, 0, true).unwrap()).is_none());
        let grey = encoded(DynamicImage::ImageLuma8(rgb(8, 8).to_luma8()), ImageFormat::Jpeg);
        assert_eq!(jpeg(&grey).unwrap().colour, Colour::Gray);
        assert!(jpeg(b"\xff\xd8\xff").is_none());
    }

    #[test]
    fn a_png_goes_in_as_its_data_unless_transparent() {
        let file = encoded(rgb(10, 4), ImageFormat::Png);
        let image = png(&file).unwrap();
        assert_eq!((image.width, image.height, image.bits, image.colour.clone()), (10, 4, 8, Colour::Rgb));
        // The zlib stream of its rows.
        let rows = inflate(&image.data);
        assert_eq!(rows.len(), 4 * (1 + 10 * 3));
        let alpha = encoded(DynamicImage::ImageRgba8(rgb(4, 4).to_rgba8()), ImageFormat::Png);
        assert!(png(&alpha).is_none());
        let grey16 = encoded(DynamicImage::ImageLuma16(rgb(4, 4).to_luma16()), ImageFormat::Png);
        assert_eq!(png(&grey16).map(|i| (i.bits, i.colour)), Some((16, Colour::Gray)));
    }

    fn inflate(data: &[u8]) -> Vec<u8> {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn pages_are_sized_by_resolution() {
        let mut image = png(&encoded(rgb(960, 480), ImageFormat::Png)).unwrap();
        assert_eq!(image.page_size(), (720.0, 360.0));
        image.dpi = (300.0, 300.0);
        assert_eq!(image.page_size(), (230.4, 115.2));
        image.dpi = resolution(Some((0.0, 72.0)));
        assert_eq!(image.dpi, (DPI, DPI));
        // Within Acrobat's limit, the proportions kept.
        image.dpi = (2.0, 2.0);
        assert_eq!(image.page_size(), (MAX_SIDE, MAX_SIDE / 2.0));
        assert_eq!(num(230.4), "230.4");
        assert_eq!(num(720.0), "720");
        assert_eq!(num(0.12345), "0.123");
        assert_eq!(text("Aя"), "<FEFF0041044F>");
    }

    #[test]
    fn writes_a_pdf_whose_objects_are_where_its_table_says() {
        let jpeg_file = encoded(rgb(40, 30), ImageFormat::Jpeg);
        let png_file = encoded(rgb(10, 4), ImageFormat::Png);
        let mut pdf = Pdf::new(Vec::new()).unwrap();
        let mut first = jpeg(&jpeg_file).unwrap();
        first.icc = Some([vec![0; 16], b"RGB ".to_vec(), vec![0; 100]].concat());
        pdf.page(&first, "один.jpg").unwrap();
        let mut second = png(&png_file).unwrap();
        // The same profile is written once.
        second.icc = first.icc.clone();
        pdf.page(&second, "two.png").unwrap();
        let (out, size) = pdf.finish("Тест").unwrap();
        assert_eq!(out.len() as u64, size);
        assert!(out.starts_with(b"%PDF-1.7\n"));
        assert!(out.ends_with(b"%%EOF\n"));
        let objects = objects(&out);
        let images: Vec<&Vec<u8>> = objects.iter().filter(|o| contains(o, "/Subtype /Image")).collect();
        assert_eq!(images.len(), 2);
        assert_eq!(stream_data(images[0]), &jpeg_file[..]);
        assert!(contains(images[0], "/DCTDecode"));
        assert_eq!(stream_data(images[1]), &png(&png_file).unwrap().data[..]);
        assert!(contains(images[1], "/Predictor 15 /Colors 3 /BitsPerComponent 8 /Columns 10"));
        assert_eq!(objects.iter().filter(|o| contains(o, "/N 3")).count(), 1);
        assert!(contains(&objects[PAGES as usize - 1], "/Count 2"));
        assert!(contains(&objects[OUTLINES as usize - 1], "/Count 2"));
        assert!(contains(&objects[INFO as usize - 1], &text("Тест")));
        assert_eq!(objects.iter().filter(|o| contains(o, "/Type /Page ")).count(), 2);
        assert!(objects.iter().any(|o| contains(o, &text("один.jpg"))));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_pdf_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn makes_a_pdf_of_files_and_leaves_nothing_when_stopped() {
        let dir = temp_dir("make");
        let paths = [dir.join("a.jpg"), dir.join("b.png"), dir.join("c.png"), dir.join("d.bmp")];
        std::fs::write(&paths[0], encoded(rgb(40, 30), ImageFormat::Jpeg)).unwrap();
        std::fs::write(&paths[1], encoded(rgb(10, 4), ImageFormat::Png)).unwrap();
        std::fs::write(&paths[2], encoded(DynamicImage::ImageRgba8(rgb(6, 6).to_rgba8()), ImageFormat::Png)).unwrap();
        std::fs::write(&paths[3], encoded(rgb(5, 5), ImageFormat::Bmp)).unwrap();
        let dst = dir.join("book.pdf");
        let job = Job { pages: paths.to_vec(), dst: dst.clone(), title: "book".into() };
        let done = std::cell::Cell::new(0);
        let size = make(&job, &AtomicBool::new(false), |n| done.set(n)).unwrap();
        assert_eq!(done.get(), 4);
        let out = std::fs::read(&dst).unwrap();
        assert_eq!(out.len() as u64, size);
        let objects = objects(&out);
        assert_eq!(objects.iter().filter(|o| contains(o, "/Subtype /Image")).count(), 4);
        // Stopped: neither the PDF nor its temporary file.
        let other = Job { dst: dir.join("other.pdf"), ..job };
        assert_eq!(make(&other, &AtomicBool::new(true), |_| {}), Err(String::new()));
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 5, "{left:?}");
        // A page that cannot be read names its file.
        let broken = dir.join("e.png");
        std::fs::write(&broken, b"not an image").unwrap();
        let job = Job { pages: vec![broken], dst: dir.join("broken.pdf"), title: String::new() };
        assert!(make(&job, &AtomicBool::new(false), |_| {}).unwrap_err().starts_with("e.png: "));
        assert!(!dir.join("broken.pdf").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `QVIEW_THUMB_DIR`'s images into one PDF in the temporary folder
    /// (kept as `QVIEW_PDF_OUT` when that is set): the time per page and
    /// the size against the files'.
    #[test]
    #[ignore]
    fn pdf_timings() {
        let dir = PathBuf::from(std::env::var("QVIEW_THUMB_DIR").expect("QVIEW_THUMB_DIR"));
        let mut pages: Vec<PathBuf> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| crate::folder::is_image(p)).collect();
        pages.sort();
        let files: u64 = pages.iter().map(|p| std::fs::metadata(p).unwrap().len()).sum();
        let _com = crate::win::com_init();
        let out = std::env::var_os("QVIEW_PDF_OUT").map(PathBuf::from);
        let dst = out.clone().unwrap_or_else(|| std::env::temp_dir().join("qview_pdf_timings.pdf"));
        let n = pages.len();
        let job = Job { pages, dst: dst.clone(), title: "timings".into() };
        let t = std::time::Instant::now();
        let size = make(&job, &AtomicBool::new(false), |_| {}).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("{n} pages in {ms:.0} ms ({:.1} ms a page); PDF {size} bytes, files {files} bytes", ms / n.max(1) as f64);
        if out.is_none() {
            std::fs::remove_file(&dst).unwrap();
        }
    }
}
