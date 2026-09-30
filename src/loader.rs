//! Decoding images on background threads.
//!
//! The UI says which files it wants, in order of priority ([`Loader::want`]:
//! the current image first, then its neighbours); worker threads take them
//! from the front of that list and send back ready pixels. The list is
//! replaced on every change, so files the user has already moved past are
//! never decoded. The loader exists before the window, so the first image
//! is decoded while the window is being created.

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Instant;

use egui::ColorImage;
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
}

pub struct Decoded {
    pub path: PathBuf,
    pub result: Result<(ColorImage, Meta), String>,
}

#[derive(Default)]
struct Queue {
    /// Files still to decode, most wanted first.
    wanted: VecDeque<PathBuf>,
    /// Files being decoded now.
    busy: Vec<PathBuf>,
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
        let wanted: VecDeque<PathBuf> = paths.into_iter().filter(|p| !q.busy.contains(p)).collect();
        if wanted != q.wanted {
            q.wanted = wanted;
            self.shared.wake.notify_all();
        }
    }

    /// A decoded image, if one is ready.
    pub fn poll(&self) -> Option<Decoded> {
        self.rx.try_recv().ok()
    }
}

fn worker(shared: &Shared, tx: &mpsc::Sender<Decoded>) {
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
        // Sent before leaving `busy`, so the UI never sees the file as
        // neither busy nor decoded and asks for it again.
        let sent = tx.send(Decoded { path: path.clone(), result }).is_ok();
        shared.queue.lock().unwrap().busy.retain(|p| *p != path);
        if !sent {
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
pub fn decode(path: &Path, max_side: usize) -> Result<(ColorImage, Meta), String> {
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
    let (width, height) = (img.width(), img.height());
    if width as usize > max_side || height as usize > max_side {
        img = img.thumbnail(max_side as u32, max_side as u32);
    }
    let meta = Meta {
        width,
        height,
        bits,
        format: format_name(format),
        file_size: bytes.len() as u64,
        modified,
    };
    Ok((color_image(img), meta))
}

/// Pixels for an egui texture (premultiplied RGBA).
fn color_image(img: DynamicImage) -> ColorImage {
    let size = [img.width() as usize, img.height() as usize];
    match img {
        DynamicImage::ImageRgb8(rgb) => ColorImage::from_rgb(size, rgb.as_raw()),
        other => ColorImage::from_rgba_unmultiplied(size, other.into_rgba8().as_raw()),
    }
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

    #[test]
    fn decodes_with_metadata() {
        let dir = temp_dir("meta");
        let path = dir.join("a.png");
        image::RgbaImage::from_pixel(30, 20, image::Rgba([255, 0, 0, 128])).save(&path).unwrap();
        let (img, meta) = decode(&path, 16384).unwrap();
        assert_eq!(img.size, [30, 20]);
        assert_eq!((meta.width, meta.height, meta.bits, meta.format), (30, 20, 32, "PNG"));
        assert_eq!(meta.file_size, std::fs::metadata(&path).unwrap().len());
        assert!(meta.modified > 0);
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
        let (img, meta) = decode(&path, 200).unwrap();
        assert_eq!(img.size, [200, 50]);
        assert_eq!((meta.width, meta.height), (400, 100));
        std::fs::remove_dir_all(&dir).unwrap();
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
