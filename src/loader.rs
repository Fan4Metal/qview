//! Decoding images on background threads.
//!
//! The UI says which files it wants, in order of priority ([`Loader::want`]:
//! the current image first, then its neighbours); worker threads take them
//! from the front of that list and send back ready pixels. The list is
//! replaced on every change, so files the user has already moved past are
//! never decoded. The loader exists before the window, so the first image
//! is decoded while the window is being created. The pixels come ready for
//! the GPU ([`Pixels`]: the driver's native BGRA order, with the mip
//! levels), so the UI thread has nothing left to convert.

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock, mpsc};
use std::time::Instant;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};

use crate::histogram::Histogram;

/// Decoder allocation limit: a 20000 x 20000 RGBA image fits, a corrupt
/// header claiming more does not take the machine's memory.
pub const MAX_ALLOC: u64 = 2 << 30;

/// What the status bar shows about an image.
#[derive(Clone, Debug)]
pub struct Meta {
    /// Size as shown (after the EXIF orientation), in image pixels; the
    /// texture may be smaller if the GPU cannot hold the image.
    pub width: u32,
    pub height: u32,
    /// Bits per pixel in the file: 24 for a colour JPEG.
    pub bits: u16,
    /// Whether the file has an alpha channel, when the decoder says.
    pub alpha: Option<bool>,
    pub format: &'static str,
    pub file_size: u64,
    /// Last write time as a FILETIME, 0 if unknown.
    pub modified: u64,
    /// A GIF or WebP of more than one frame (see `anim`); the pixels are
    /// its first frame.
    pub animated: bool,
    /// Counted by the decoder thread while the information panel's
    /// histogram is open (`Loader::set_histograms`); of the first frame of
    /// an animation.
    pub histogram: Option<Arc<Histogram>>,
}

/// An image as the GPU takes it: premultiplied BGRA, 8 bits per channel,
/// the full size first and then the mip levels down to 1x1, each half the
/// size of the one before (rounded down, at least 1).
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

pub struct Decoded {
    pub path: PathBuf,
    pub result: Result<(Pixels, Meta), String>,
}

#[derive(Default)]
struct Queue {
    /// Files still to decode, most wanted first.
    wanted: VecDeque<PathBuf>,
    /// Files being decoded now.
    busy: Vec<PathBuf>,
    /// Files decoded and sent, but not taken by [`Loader::poll`] yet: not
    /// to be decoded again if they are wanted meanwhile.
    done: Vec<PathBuf>,
    /// Files changed on disk while busy or done: their result is from the
    /// old contents and is dropped by [`Loader::poll`].
    stale: Vec<PathBuf>,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Largest texture side the GPU takes; larger images are shrunk.
    max_side: AtomicUsize,
    /// Count each image's histogram (`Meta::histogram`).
    histograms: AtomicBool,
    ctx: OnceLock<egui::Context>,
}

pub struct Loader {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Decoded>,
}

impl Loader {
    /// Start `workers` decoding threads.
    pub fn new(workers: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            // Every Windows GPU of the last decade takes 16384; corrected
            // from the real limit on the first frame.
            max_side: AtomicUsize::new(16384),
            histograms: AtomicBool::new(false),
            ctx: OnceLock::new(),
        });
        let (tx, rx) = mpsc::channel();
        for i in 0..workers.max(1) {
            let shared = shared.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name(format!("decoder {i}"))
                .spawn(move || worker(&shared, &tx))
                .expect("spawn decoder thread");
        }
        Self { shared, rx }
    }

    /// Repaint `ctx` whenever an image is ready.
    pub fn set_context(&self, ctx: egui::Context) {
        let _ = self.shared.ctx.set(ctx);
    }

    pub fn set_max_side(&self, side: usize) {
        self.shared.max_side.store(side, Relaxed);
    }

    /// Count the histogram of the images decoded from now on, or not.
    pub fn set_histograms(&self, on: bool) {
        self.shared.histograms.store(on, Relaxed);
    }

    /// Decode `paths`, in this order, instead of whatever was wanted before
    /// (files being decoded now finish anyway).
    pub fn want(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut q = self.shared.queue.lock().unwrap();
        let wanted: VecDeque<PathBuf> =
            paths.into_iter().filter(|p| !q.busy.contains(p) && !q.done.contains(p)).collect();
        if wanted != q.wanted {
            q.wanted = wanted;
            self.shared.wake.notify_all();
        }
    }

    /// `paths` changed on disk: what is being decoded of them now is from
    /// the old contents, and dropped when it comes; wanted again, they are
    /// decoded anew.
    pub fn forget(&self, paths: &[PathBuf]) {
        let mut q = self.shared.queue.lock().unwrap();
        let Queue { busy, done, stale, .. } = &mut *q;
        stale.extend(paths.iter().filter(|p| busy.contains(p) || done.contains(p)).cloned());
    }

    /// A decoded image, if one is ready.
    pub fn poll(&self) -> Option<Decoded> {
        loop {
            let decoded = self.rx.try_recv().ok()?;
            let mut q = self.shared.queue.lock().unwrap();
            q.done.retain(|p| *p != decoded.path);
            if let Some(i) = q.stale.iter().position(|p| *p == decoded.path) {
                q.stale.swap_remove(i);
                continue;
            }
            return Some(decoded);
        }
    }
}

fn worker(shared: &Shared, tx: &mpsc::Sender<Decoded>) {
    // For Windows' codecs (`wic`), which stay loaded while it lasts.
    let _com = crate::win::com_init();
    loop {
        let path = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                if let Some(path) = q.wanted.pop_front() {
                    q.busy.push(path.clone());
                    break path;
                }
                q = shared.wake.wait(q).unwrap();
            }
        };
        let start = Instant::now();
        let count = shared.histograms.load(Relaxed);
        let result = decode(&path, shared.max_side.load(Relaxed), count);
        let took = start.elapsed();
        let with = if count { " with its histogram" } else { "" };
        log::debug!("decoded {}{with} in {:.1} ms", path.display(), took.as_secs_f64() * 1e3);
        // Marked done before it is sent, so the UI never sees the file as
        // neither busy, nor done, nor received, and asks for it again.
        {
            let mut q = shared.queue.lock().unwrap();
            q.busy.retain(|p| *p != path);
            q.done.push(path.clone());
        }
        if tx.send(Decoded { path, result }).is_err() {
            return;
        }
        if let Some(ctx) = shared.ctx.get() {
            ctx.request_repaint();
        }
    }
}

/// Name of the format as the status bar shows it.
fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Png => "PNG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WEBP",
        ImageFormat::Bmp => "BMP",
        ImageFormat::Tiff => "TIFF",
        ImageFormat::Ico => "ICO",
        ImageFormat::Tga => "TGA",
        ImageFormat::Qoi => "QOI",
        ImageFormat::Pnm => "PNM",
        _ => "IMAGE",
    }
}

/// Read and decode `path`, turned upright by its EXIF orientation and
/// shrunk to `max_side` if larger; with `count`, its histogram counted
/// too (`Meta::histogram`), while the mip levels are made.
pub fn decode(path: &Path, max_side: usize, count: bool) -> Result<(Pixels, Meta), String> {
    let shrink = |img: DynamicImage| {
        if img.width() as usize > max_side || img.height() as usize > max_side {
            img.thumbnail(max_side as u32, max_side as u32)
        } else {
            img
        }
    };
    let first = match (!crate::wic::takes(path)).then(|| read_image(path)) {
        Some(Ok((img, mut meta))) => {
            let img = shrink(img);
            let (width, height) = (img.width(), img.height());
            let (pixels, histogram) = with_levels(to_bgra(img), width, height, count);
            meta.histogram = histogram.map(Arc::new);
            return Ok((pixels, meta));
        }
        Some(Err(e)) => Some(e),
        None => None,
    };
    // Premultiplied BGRA already: turned and shrunk as it is (the image
    // is RGBA only by name), then only the mip levels are made.
    let (img, mut meta) = read_wic(path, true).map_err(|e| first.unwrap_or(e))?;
    let img = shrink(img);
    let (width, height) = (img.width(), img.height());
    let (pixels, histogram) = with_levels(img.into_rgba8().into_raw(), width, height, count);
    meta.histogram = histogram.map(Arc::new);
    Ok((pixels, meta))
}

/// Read and decode `path`, turned upright by its EXIF orientation: with
/// the `image` crate, or with Windows' codecs (`wic`) what it cannot read.
/// An image in an archive (`archive`) only with the `image` crate.
pub fn read(path: &Path) -> Result<(DynamicImage, Meta), String> {
    if crate::archive::inside(path) {
        return read_image(path);
    }
    let first = match (!crate::wic::takes(path)).then(|| read_image(path)) {
        Some(Ok(read)) => return Ok(read),
        Some(Err(e)) => Some(e),
        None => None,
    };
    read_wic(path, false).map_err(|e| first.unwrap_or(e))
}

/// `path` decoded by Windows' codecs, upright: premultiplied BGRA with
/// `bgra` (in an RGBA image only by name), otherwise RGBA. HEIF goes to
/// libheif first when its DLL is there (`heif`), then to Windows if that
/// fails. Otherwise the error says which extension from the Microsoft
/// Store the format needs.
pub fn read_wic(path: &Path, bgra: bool) -> Result<(DynamicImage, Meta), String> {
    use std::os::windows::fs::MetadataExt;
    let file = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let image = match crate::heif::decode(path, bgra) {
        Some(Ok(image)) => image,
        Some(Err(e)) => crate::wic::decode(path, bgra).map_err(|_| e)?,
        None => crate::wic::decode(path, bgra).map_err(|e| match crate::wic::needs(path) {
            Some(needs) => format!("{e}\n\n{needs}"),
            None => e,
        })?,
    };
    let buffer = image::RgbaImage::from_raw(image.width, image.height, image.pixels).ok_or("bad image size")?;
    let mut img = DynamicImage::ImageRgba8(buffer);
    img.apply_orientation(Orientation::from_exif(image.orientation as u8).unwrap_or(Orientation::NoTransforms));
    let format = ImageFormat::from_path(path).map_or_else(|_| crate::wic::format_name(path), format_name);
    let meta = Meta {
        width: img.width(),
        height: img.height(),
        bits: image.bits,
        alpha: image.alpha,
        format,
        file_size: file.len(),
        modified: file.last_write_time(),
        animated: false,
        histogram: None,
    };
    Ok((img, meta))
}

/// Read and decode `path` (a file, or an image in an archive) with the
/// `image` crate, upright.
fn read_image(path: &Path) -> Result<(DynamicImage, Meta), String> {
    // One read of the whole file is faster than buffered reads through
    // the decoder.
    let bytes = crate::archive::read(path).map_err(|e| e.to_string())?;
    let modified = crate::archive::metadata(path).map_or(0, |(_, m)| m);
    let mut reader = ImageReader::new(Cursor::new(&bytes[..])).with_guessed_format().map_err(|e| e.to_string())?;
    if reader.format().is_none()
        && let Ok(format) = ImageFormat::from_path(path)
    {
        reader.set_format(format);
    }
    let format = reader.format().ok_or_else(|| tr!("Unknown file format", "Неизвестный формат файла"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let bits = decoder.original_color_type().bits_per_pixel();
    let alpha = decoder.color_type().has_alpha();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    // A GIF whose screen is 0 pixels wide decodes without an error.
    if img.width() == 0 || img.height() == 0 {
        return Err(format!("the image is {}x{} pixels", img.width(), img.height()));
    }
    img.apply_orientation(orientation);
    let animated = crate::anim::is_animated(&bytes, format);
    let meta = Meta {
        width: img.width(),
        height: img.height(),
        bits,
        alpha: Some(alpha),
        format: format_name(format),
        file_size: bytes.len() as u64,
        modified,
        animated,
        histogram: None,
    };
    Ok((img, meta))
}

pub fn to_pixels(img: DynamicImage) -> Pixels {
    let (width, height) = (img.width(), img.height());
    let base = to_bgra(img);
    let levels = mip_levels(base, width as usize, height as usize);
    Pixels { width, height, levels }
}

/// Whether the mip levels are averaged in linear light (`set_linear_mips`).
static LINEAR_MIPS: AtomicBool = AtomicBool::new(true);

/// Average the mip levels of the images decoded from now on in linear
/// light (sRGB decoded, averaged, encoded again; View → Filtering → Reduce
/// in Linear Light), or as stored. Averaging sRGB values as they are
/// darkens thin bright lines and fine texture in a reduced image.
pub fn set_linear_mips(on: bool) {
    LINEAR_MIPS.store(on, Relaxed);
}

pub fn linear_mips() -> bool {
    LINEAR_MIPS.load(Relaxed)
}

/// Premultiplied BGRA of `img`.
fn to_bgra(img: DynamicImage) -> Vec<u8> {
    let n = img.width() as usize * img.height() as usize;
    let mut out = vec![0u8; n * 4];
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            for (dst, p) in out.as_chunks_mut::<4>().0.iter_mut().zip(rgb.as_raw().as_chunks::<3>().0) {
                *dst = [p[2], p[1], p[0], 255];
            }
        }
        other => {
            let rgba = other.into_rgba8();
            for (dst, p) in out.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_raw().as_chunks::<4>().0) {
                let a = p[3] as u32;
                let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
                *dst = [pm(p[2]), pm(p[1]), pm(p[0]), p[3]];
            }
        }
    }
    out
}

/// `base` (`width` x `height` premultiplied BGRA) with its mip levels,
/// and with `count` its histogram, counted on other threads while the
/// levels are made here (a 14.7 MP photo: ~5 ms of levels, ~8 ms of
/// counting in parts, see `histogram`), so that it delays the image little.
fn with_levels(base: Vec<u8>, width: u32, height: u32, count: bool) -> (Pixels, Option<Histogram>) {
    let (w, h) = (width as usize, height as usize);
    let linear = linear_mips();
    let (below, histogram) = if count {
        std::thread::scope(|s| {
            let counting = s.spawn(|| Histogram::of_bgra(&base));
            let below = smaller_levels(&base, w, h, linear);
            (below, Some(counting.join().expect("histogram thread")))
        })
    } else {
        (smaller_levels(&base, w, h, linear), None)
    };
    let mut levels = Vec::with_capacity(below.len() + 1);
    levels.push(base);
    levels.extend(below);
    (Pixels { width, height, levels }, histogram)
}

/// `base` (`w` x `h` BGRA) followed by its mip levels down to 1x1,
/// averaged as chosen with `set_linear_mips`.
pub fn mip_levels(base: Vec<u8>, w: usize, h: usize) -> Vec<Vec<u8>> {
    mip_levels_with(base, w, h, linear_mips())
}

/// `mip_levels`, averaged in linear light or as stored.
pub fn mip_levels_with(base: Vec<u8>, w: usize, h: usize, linear: bool) -> Vec<Vec<u8>> {
    let below = smaller_levels(&base, w, h, linear);
    let mut levels = Vec::with_capacity(below.len() + 1);
    levels.push(base);
    levels.extend(below);
    levels
}

/// The mip levels below `base` (`w` x `h` BGRA), down to 1x1.
fn smaller_levels(base: &[u8], w: usize, h: usize, linear: bool) -> Vec<Vec<u8>> {
    let mut levels: Vec<Vec<u8>> = Vec::new();
    let (mut w, mut h) = (w, h);
    // No pixels, no levels (and `half` would index an empty slice).
    while (w > 1 || h > 1) && w > 0 && h > 0 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let next = half(levels.last().map_or(base, Vec::as_slice), w, h, nw, nh, linear);
        levels.push(next);
        (w, h) = (nw, nh);
    }
    levels
}

/// A level of this many pixels or more is made in bands of rows on
/// several threads (a 14.7 MP photo: its first two levels).
const BAND_PIXELS: usize = 1 << 20;

/// `src` (`w` x `h` BGRA) at half size: every pixel the average of a 2x2
/// block, as `glGenerateMipmap` makes it, in linear light or of the
/// stored values. An odd last row or column is dropped; a side of 1 is
/// kept.
fn half(src: &[u8], w: usize, h: usize, nw: usize, nh: usize, linear: bool) -> Vec<u8> {
    let src = src.as_chunks::<4>().0;
    let mut out = vec![0u8; nw * nh * 4];
    let dst = out.as_chunks_mut::<4>().0;
    if w >= 2 && h >= 2 {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4);
        let bands = if nw * nh >= BAND_PIXELS { threads } else { 1 };
        let rows = nh.div_ceil(bands);
        // An odd last row of `src` falls into a chunk of its own, which
        // `zip` leaves out, or into a band, which drops it.
        let pairs = src.chunks(2 * rows * w).zip(dst.chunks_mut(rows * nw));
        if bands == 1 {
            for (src, dst) in pairs {
                half_rows(src, dst, w, nw, linear);
            }
        } else {
            std::thread::scope(|s| {
                for (src, dst) in pairs {
                    s.spawn(move || half_rows(src, dst, w, nw, linear));
                }
            });
        }
    } else {
        // A single row or column: pairs along it.
        for (i, d) in dst.iter_mut().enumerate() {
            let (a, b) = (src[(2 * i).min(w * h - 1)], src[(2 * i + 1).min(w * h - 1)]);
            *d = if linear { avg4_linear([a, a, b, b]) } else { avg2(a, b) };
        }
    }
    out
}

/// Rows of `src` (`w` wide) in pairs into rows of `dst` (`nw` wide).
fn half_rows(src: &[[u8; 4]], dst: &mut [[u8; 4]], w: usize, nw: usize, linear: bool) {
    // Row pairs and pixel pairs, so the loop has no bounds to check.
    for (rows, dst) in src.chunks_exact(2 * w).zip(dst.chunks_exact_mut(nw)) {
        let (r0, r1) = rows.split_at(w);
        let blocks = r0.as_chunks::<2>().0.iter().zip(r1.as_chunks::<2>().0).zip(dst);
        if linear {
            for ((a, b), d) in blocks {
                *d = avg4_linear([a[0], a[1], b[0], b[1]]);
            }
        } else {
            for ((a, b), d) in blocks {
                *d = avg2(avg2(a[0], a[1]), avg2(b[0], b[1]));
            }
        }
    }
}

/// Per-channel (a + b) / 2 of the stored values, on four packed bytes at
/// once (rounded down).
fn avg2(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    let (a, b) = (u32::from_ne_bytes(a), u32::from_ne_bytes(b));
    ((a & b) + (((a ^ b) & 0xfefe_fefe) >> 1)).to_ne_bytes()
}

/// sRGB to linear light and back, for `avg4_linear`.
struct Tables {
    /// The sRGB byte's linear light, 0..=65535.
    linear: [u16; 256],
    /// The sRGB byte nearest to linear light `i * 8 + 4` (8192 entries:
    /// the darkest sRGB steps are ~20 apart in these units, so every step
    /// gets an entry of its own).
    srgb: [u8; 8192],
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let linear = std::array::from_fn(|i| {
        let c = i as f64 / 255.0;
        let l = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        (l * 65535.0).round() as u16
    });
    let srgb = std::array::from_fn(|i| {
        let l = (i as f64 * 8.0 + 4.0) / 65535.0;
        let c = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (c * 255.0).round().clamp(0.0, 255.0) as u8
    });
    Tables { linear, srgb }
});

/// The average of four premultiplied BGRA pixels in linear light: each
/// channel decoded from sRGB, averaged and encoded again, the alpha
/// averaged as it is. Opaque pixels (nearly all) take the short way; the
/// others are unpremultiplied first, so that the colour under a
/// half-transparent edge is averaged with its own weight.
fn avg4_linear(p: [[u8; 4]; 4]) -> [u8; 4] {
    let t = &*TABLES;
    if p.iter().all(|q| q[3] == 255) {
        let mut out = [0, 0, 0, 255];
        for c in 0..3 {
            let sum: u32 = p.iter().map(|q| u32::from(t.linear[usize::from(q[c])])).sum();
            out[c] = t.srgb[(((sum + 2) / 4) >> 3) as usize];
        }
        return out;
    }
    let total: u32 = p.iter().map(|q| u32::from(q[3])).sum();
    if total == 0 {
        return [0; 4];
    }
    let alpha = (total + 2) / 4;
    let mut out = [0, 0, 0, alpha as u8];
    for c in 0..3 {
        // Linear light (0..=65535) weighted by alpha (0..=255).
        let mut sum = 0u64;
        for q in p {
            let a = u32::from(q[3]);
            // A transparent pixel adds nothing (and has no colour to divide by).
            if let Some(stored) = (u32::from(q[c]) * 255 + a / 2).checked_div(a) {
                sum += u64::from(t.linear[stored.min(255) as usize]) * u64::from(a);
            }
        }
        // The alpha-weighted mean colour, encoded and premultiplied again.
        let mean = ((sum + u64::from(total) / 2) / u64::from(total)) as u32;
        let stored = u32::from(t.srgb[(mean >> 3) as usize]);
        out[c] = ((stored * alpha + 127) / 255) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_loader_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Timings of the phases of `decode` for the file named by
    /// `QVIEW_BENCH_FILE`, printed with `--nocapture`.
    #[test]
    #[ignore]
    fn phase_timings() {
        let path = PathBuf::from(std::env::var("QVIEW_BENCH_FILE").expect("QVIEW_BENCH_FILE"));
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        for round in 0..3 {
            let t = Instant::now();
            let bytes = std::fs::read(&path).unwrap();
            let read = ms(t);
            let t = Instant::now();
            let reader = ImageReader::new(Cursor::new(&bytes[..])).with_guessed_format().unwrap();
            let mut decoder = reader.into_decoder().unwrap();
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            let mut img = DynamicImage::from_decoder(decoder).unwrap();
            let decoded = ms(t);
            let t = Instant::now();
            img.apply_orientation(orientation);
            let oriented = ms(t);
            let (w, h) = (img.width(), img.height());
            let t = Instant::now();
            let base = to_bgra(img);
            let converted = ms(t);
            let t = Instant::now();
            let levels = mip_levels(base, w as usize, h as usize);
            let mips = ms(t);
            let mp = w as f64 * h as f64 / 1e6;
            println!("round {round}: {w}x{h} ({mp:.1} MP), {orientation:?}");
            println!("  read {read:.1} ms, decode {decoded:.1} ms, orient {oriented:.1} ms, to BGRA {converted:.1} ms");
            println!("  {} mip levels {mips:.1} ms", levels.len() - 1);
        }
    }

    #[test]
    fn decodes_with_metadata() {
        let dir = temp_dir("meta");
        let path = dir.join("a.png");
        image::RgbaImage::from_pixel(30, 20, image::Rgba([255, 0, 0, 128])).save(&path).unwrap();
        let (pixels, meta) = decode(&path, 16384, false).unwrap();
        assert_eq!((pixels.width, pixels.height), (30, 20));
        assert_eq!(pixels.levels.len(), 5); // 30x20, 15x10, 7x5, 3x2, 1x1
        // Premultiplied BGRA: red at half opacity.
        assert_eq!(&pixels.levels[0][..4], &[0, 0, 128, 128]);
        assert_eq!(pixels.levels[4].len(), 4);
        assert_eq!((meta.width, meta.height, meta.bits, meta.format), (30, 20, 32, "PNG"));
        assert_eq!(meta.file_size, std::fs::metadata(&path).unwrap().len());
        assert!(meta.modified > 0);
        assert!(!meta.animated);
        // Content decides, not the extension.
        let jpeg = dir.join("really_a_jpeg.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3])).save_with_format(&jpeg, ImageFormat::Jpeg).unwrap();
        let (_, meta) = decode(&jpeg, 16384, false).unwrap();
        assert_eq!((meta.format, meta.bits), ("JPEG", 24));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shrinks_what_the_gpu_cannot_hold() {
        let dir = temp_dir("shrink");
        let path = dir.join("wide.png");
        image::RgbImage::new(400, 100).save(&path).unwrap();
        let (pixels, meta) = decode(&path, 200, false).unwrap();
        assert_eq!((pixels.width, pixels.height), (200, 50));
        assert_eq!((meta.width, meta.height), (400, 100));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What the `image` crate cannot read goes to Windows' codecs, which
    /// give the same pixels, turned by the EXIF orientation.
    #[test]
    fn windows_codecs_take_over() {
        let _com = crate::win::com_init();
        let dir = temp_dir("wic");
        let path = dir.join("a.png");
        image::RgbaImage::from_pixel(30, 20, image::Rgba([255, 0, 0, 128])).save(&path).unwrap();
        let (own, _) = decode(&path, 16384, false).unwrap();
        let (img, meta) = read_wic(&path, true).unwrap();
        assert_eq!(img.into_rgba8().into_raw(), own.levels[0]);
        assert_eq!((meta.width, meta.height, meta.bits, meta.format), (30, 20, 32, "PNG"));
        let mut jpeg = Vec::new();
        image::RgbImage::new(40, 20).write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg).unwrap();
        let turned = dir.join("turned.jpg");
        std::fs::write(&turned, crate::wic::tests::with_orientation(&jpeg, 6)).unwrap();
        let (img, _) = read_wic(&turned, false).unwrap();
        assert_eq!((img.width(), img.height()), (20, 40));
        // A HEIC that is none: libheif's error when heif.dll is there,
        // otherwise what Windows needs.
        let heic = dir.join("broken.heic");
        std::fs::write(&heic, b"not an image").unwrap();
        let e = decode(&heic, 16384, false).err().unwrap();
        let expected = if crate::heif::takes(&heic) { "libheif" } else { "HEVC" };
        assert!(e.contains(expected), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mip_levels_average_blocks() {
        // 3x2 pixels, one channel varied: the odd column is dropped.
        let base: Vec<u8> = [10u8, 20, 99, 30, 40, 99].iter().flat_map(|&v| [v, 0, 0, 255]).collect();
        let levels = mip_levels_with(base, 3, 2, false);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], vec![25, 0, 0, 255]);
        // A single row keeps its height.
        let row: Vec<u8> = [0u8, 100, 200, 250].iter().flat_map(|&v| [v, v, v, 255]).collect();
        let levels = mip_levels_with(row, 4, 1, false);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), vec![16, 8, 4]);
        assert_eq!(&levels[1][..4], &[50, 50, 50, 255]);
        assert_eq!(&levels[2][..4], &[137, 137, 137, 255]);
        // No pixels (a GIF with a screen 0 wide): no levels, no panic.
        assert_eq!(mip_levels_with(Vec::new(), 0, 5, false).len(), 1);
    }

    #[test]
    fn mip_levels_in_linear_light() {
        // The tables invert each other at every byte.
        let t = &*TABLES;
        for v in 0..=255u8 {
            assert_eq!(t.srgb[(usize::from(t.linear[usize::from(v)])) >> 3], v, "{v}");
        }
        // Black and white average to the sRGB middle grey (188), not 127.
        let square = |a: [u8; 4], b: [u8; 4]| {
            let base = [a, b, b, a].concat();
            mip_levels_with(base, 2, 2, true)[1].clone()
        };
        assert_eq!(square([0, 0, 0, 255], [255, 255, 255, 255]), [188, 188, 188, 255]);
        // Equal pixels stay as they are, in every channel.
        assert_eq!(square([10, 20, 30, 255], [10, 20, 30, 255]), [10, 20, 30, 255]);
        // A row and a column as well, and equal values stay in them.
        let row: Vec<u8> = [0u8, 255, 200, 200].iter().flat_map(|&v| [v, v, v, 255]).collect();
        let levels = mip_levels_with(row.clone(), 4, 1, true);
        assert_eq!(&levels[1][..8], &[188, 188, 188, 255, 200, 200, 200, 255]);
        assert_eq!(&mip_levels_with(row, 1, 4, true)[1][..8], &[188, 188, 188, 255, 200, 200, 200, 255]);
        // Premultiplied: a white pixel at half alpha (stored 128) beside a
        // transparent one is white at a quarter alpha, not a dark grey:
        // the colour is weighted by alpha, not averaged with the nothing.
        let clear = [0, 0, 0, 0];
        assert_eq!(square([128, 128, 128, 128], clear), [64, 64, 64, 64]);
        // Half-transparent black beside opaque white: the mean colour in
        // linear light is 2/3 white (sRGB 213), premultiplied by alpha 192.
        assert_eq!(square([0, 0, 0, 128], [255, 255, 255, 255]), [160, 160, 160, 192]);
        assert_eq!(square(clear, clear), clear);
        // A level large enough for the bands equals one made in one piece.
        let (w, h) = (2600, 1700);
        let mut seed = 7u32;
        let base: Vec<u8> = (0..w * h * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        let banded = half(&base, w, h, w / 2, h / 2, true);
        let mut whole = vec![0u8; w * h];
        half_rows(base.as_chunks::<4>().0, whole.as_chunks_mut::<4>().0, w, w / 2, true);
        assert!(banded == whole);
    }

    /// Timings of the mip levels of a 14.7 MP image, averaged as stored
    /// and in linear light, printed with `--nocapture`.
    #[test]
    #[ignore]
    fn mip_timings() {
        let (w, h) = (4700, 3130);
        let mut seed = 1u32;
        let base: Vec<u8> = (0..w * h)
            .flat_map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let [b, g, r, _] = seed.to_le_bytes();
                [b, g, r, 255]
            })
            .collect();
        for linear in [false, true] {
            let mut best = f64::MAX;
            for _ in 0..7 {
                let base = base.clone();
                let t = Instant::now();
                let levels = mip_levels_with(base, w, h, linear);
                best = best.min(t.elapsed().as_secs_f64() * 1e3);
                assert_eq!(levels.len(), 13);
            }
            println!("{w}x{h}, linear {linear}: all levels {best:.1} ms (best of 7)");
        }
    }

    #[test]
    fn reports_errors() {
        let dir = temp_dir("errors");
        let path = dir.join("broken.jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode(&path, 16384, false).is_err());
        assert!(decode(&dir.join("missing.jpg"), 16384, false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn workers_deliver_what_is_wanted() {
        let dir = temp_dir("workers");
        let paths: Vec<PathBuf> = (0..3).map(|i| dir.join(format!("{i}.png"))).collect();
        for p in &paths {
            image::RgbImage::new(4, 4).save(p).unwrap();
        }
        let loader = Loader::new(2);
        loader.want(paths.clone());
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < 3 && Instant::now() < deadline {
            match loader.poll() {
                Some(d) => got.push(d.path),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        got.sort();
        assert_eq!(got, paths);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forgotten_results_are_dropped_and_decoded_again() {
        let dir = temp_dir("forget");
        let path = dir.join("a.png");
        image::RgbImage::new(4, 4).save(&path).unwrap();
        let loader = Loader::new(1);
        loader.want([path.clone()]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !loader.shared.queue.lock().unwrap().done.contains(&path) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Written over after it was decoded, before it was taken.
        loader.forget(std::slice::from_ref(&path));
        assert!(loader.poll().is_none());
        let mut got = None;
        while got.is_none() && Instant::now() < deadline {
            // Asked for every frame, as `App::update_wanted` does.
            loader.want([path.clone()]);
            got = loader.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got.map(|d| d.path), Some(path));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
