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
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Instant;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};

/// Decoder allocation limit: a 20000 x 20000 RGBA image fits, a corrupt
/// header claiming more does not take the machine's memory.
const MAX_ALLOC: u64 = 2 << 30;

/// What the status bar shows about an image.
#[derive(Clone, Debug)]
pub struct Meta {
    /// Size as shown (after the EXIF orientation), in image pixels; the
    /// texture may be smaller if the GPU cannot hold the image.
    pub width: u32,
    pub height: u32,
    /// Bits per pixel in the file: 24 for a colour JPEG.
    pub bits: u16,
    pub format: &'static str,
    pub file_size: u64,
    /// Last write time as a FILETIME, 0 if unknown.
    pub modified: u64,
    /// A GIF or WebP of more than one frame (see `anim`); the pixels are
    /// its first frame.
    pub animated: bool,
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
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Largest texture side the GPU takes; larger images are shrunk.
    max_side: AtomicUsize,
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

    /// A decoded image, if one is ready.
    pub fn poll(&self) -> Option<Decoded> {
        let decoded = self.rx.try_recv().ok()?;
        self.shared.queue.lock().unwrap().done.retain(|p| *p != decoded.path);
        Some(decoded)
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
        let result = decode(&path, shared.max_side.load(Relaxed));
        let took = start.elapsed();
        log::debug!("decoded {} in {:.1} ms", path.display(), took.as_secs_f64() * 1e3);
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
/// shrunk to `max_side` if larger.
pub fn decode(path: &Path, max_side: usize) -> Result<(Pixels, Meta), String> {
    let shrink = |img: DynamicImage| {
        if img.width() as usize > max_side || img.height() as usize > max_side {
            img.thumbnail(max_side as u32, max_side as u32)
        } else {
            img
        }
    };
    let first = match (!crate::wic::takes(path)).then(|| read_image(path)) {
        Some(Ok((img, meta))) => return Ok((to_pixels(shrink(img)), meta)),
        Some(Err(e)) => Some(e),
        None => None,
    };
    // Premultiplied BGRA already: turned and shrunk as it is (the image
    // is RGBA only by name), then only the mip levels are made.
    let (img, meta) = read_wic(path, true).map_err(|e| first.unwrap_or(e))?;
    let img = shrink(img);
    let (width, height) = (img.width(), img.height());
    let levels = mip_levels(img.into_rgba8().into_raw(), width as usize, height as usize);
    Ok((Pixels { width, height, levels }, meta))
}

/// Read and decode `path`, turned upright by its EXIF orientation: with
/// the `image` crate, or with Windows' codecs (`wic`) what it cannot read.
pub fn read(path: &Path) -> Result<(DynamicImage, Meta), String> {
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
fn read_wic(path: &Path, bgra: bool) -> Result<(DynamicImage, Meta), String> {
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
        format,
        file_size: file.len(),
        modified: file.last_write_time(),
        animated: false,
    };
    Ok((img, meta))
}

/// Read and decode `path` with the `image` crate, upright.
fn read_image(path: &Path) -> Result<(DynamicImage, Meta), String> {
    use std::os::windows::fs::MetadataExt;
    // One read of the whole file is faster than buffered reads through
    // the decoder.
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let modified = std::fs::metadata(path).map_or(0, |m| m.last_write_time());
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
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let animated = crate::anim::is_animated(&bytes, format);
    let meta = Meta {
        width: img.width(),
        height: img.height(),
        bits,
        format: format_name(format),
        file_size: bytes.len() as u64,
        modified,
        animated,
    };
    Ok((img, meta))
}

pub fn to_pixels(img: DynamicImage) -> Pixels {
    let (width, height) = (img.width(), img.height());
    let base = to_bgra(img);
    let levels = mip_levels(base, width as usize, height as usize);
    Pixels { width, height, levels }
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

/// `base` (`w` x `h` BGRA) followed by its mip levels down to 1x1.
pub fn mip_levels(base: Vec<u8>, w: usize, h: usize) -> Vec<Vec<u8>> {
    let mut levels = vec![base];
    let (mut w, mut h) = (w, h);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let next = half(levels.last().expect("base level"), w, h, nw, nh);
        levels.push(next);
        (w, h) = (nw, nh);
    }
    levels
}

/// `src` (`w` x `h` BGRA) at half size: every pixel the average of a 2x2
/// block, as `glGenerateMipmap` makes it. An odd last row or column is
/// dropped; a side of 1 is kept.
fn half(src: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let src = src.as_chunks::<4>().0;
    let px = |p: [u8; 4]| u32::from_ne_bytes(p);
    // Per-channel (a + b) / 2 on four packed bytes at once.
    let avg = |a: u32, b: u32| (a & b) + (((a ^ b) & 0xfefe_fefe) >> 1);
    let mut out = vec![0u8; nw * nh * 4];
    let dst = out.as_chunks_mut::<4>().0;
    if w >= 2 && h >= 2 {
        // Row pairs and pixel pairs, so the loop has no bounds to check.
        for (rows, dst) in src.chunks_exact(2 * w).zip(dst.chunks_exact_mut(nw)) {
            let (r0, r1) = rows.split_at(w);
            for ((a, b), d) in r0.as_chunks::<2>().0.iter().zip(r1.as_chunks::<2>().0).zip(dst) {
                *d = avg(avg(px(a[0]), px(a[1])), avg(px(b[0]), px(b[1]))).to_ne_bytes();
            }
        }
    } else {
        // A single row or column: pairs along it.
        for (i, d) in dst.iter_mut().enumerate() {
            let (a, b) = (src[(2 * i).min(w * h - 1)], src[(2 * i + 1).min(w * h - 1)]);
            *d = avg(px(a), px(b)).to_ne_bytes();
        }
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
        let (pixels, meta) = decode(&path, 16384).unwrap();
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
        let (_, meta) = decode(&jpeg, 16384).unwrap();
        assert_eq!((meta.format, meta.bits), ("JPEG", 24));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shrinks_what_the_gpu_cannot_hold() {
        let dir = temp_dir("shrink");
        let path = dir.join("wide.png");
        image::RgbImage::new(400, 100).save(&path).unwrap();
        let (pixels, meta) = decode(&path, 200).unwrap();
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
        let (own, _) = decode(&path, 16384).unwrap();
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
        let e = decode(&heic, 16384).err().unwrap();
        let expected = if crate::heif::takes(&heic) { "libheif" } else { "HEVC" };
        assert!(e.contains(expected), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mip_levels_average_blocks() {
        // 3x2 pixels, one channel varied: the odd column is dropped.
        let base: Vec<u8> = [10u8, 20, 99, 30, 40, 99].iter().flat_map(|&v| [v, 0, 0, 255]).collect();
        let levels = mip_levels(base, 3, 2);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], vec![25, 0, 0, 255]);
        // A single row keeps its height.
        let row: Vec<u8> = [0u8, 100, 200, 250].iter().flat_map(|&v| [v, v, v, 255]).collect();
        let levels = mip_levels(row, 4, 1);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), vec![16, 8, 4]);
        assert_eq!(&levels[1][..4], &[50, 50, 50, 255]);
        assert_eq!(&levels[2][..4], &[137, 137, 137, 255]);
    }

    #[test]
    fn reports_errors() {
        let dir = temp_dir("errors");
        let path = dir.join("broken.jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode(&path, 16384).is_err());
        assert!(decode(&dir.join("missing.jpg"), 16384).is_err());
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
}
