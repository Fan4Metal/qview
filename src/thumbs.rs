//! Thumbnails of the gallery, made on background threads.
//!
//! Windows keeps the thumbnails Explorer has shown in its thumbnail cache,
//! and `IShellItemImageFactory::GetImage` returns one of those in a
//! millisecond or two; for a file it has not seen, Windows makes the
//! thumbnail (with its own codecs, the EXIF orientation applied, a JPEG
//! decoded at a fraction of its size) and caches it for next time.
//! Formats Windows makes no thumbnails of (QOI, TGA, PNM, ...) are decoded
//! with the `image` crate and shrunk ([`loader::read`]).
//!
//! As with `loader::Loader`, the UI replaces the list of wanted thumbnails
//! wholesale ([`Thumbs::want`], the visible cells first), so cells the user
//! has scrolled past are never made.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Instant;

use crate::loader::{self, Pixels};

/// A thumbnail asked for: `path` fitted into `side` x `side` pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub path: PathBuf,
    pub side: u32,
}

/// What [`make`] found out about a file.
pub struct Made {
    pub request: Request,
    pub result: Result<Thumbnail, String>,
}

pub struct Thumbnail {
    /// The thumbnail, BGRA with its mip levels, as `texture::Texture`
    /// takes it.
    pub pixels: Pixels,
    /// Size of the image, upright; 0 when unknown.
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    /// Last write time as a FILETIME, 0 if unknown.
    pub modified: u64,
}

#[derive(Default)]
struct Queue {
    wanted: VecDeque<Request>,
    busy: Vec<Request>,
    /// Made and sent, not taken by [`Thumbs::poll`] yet.
    done: Vec<Request>,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    ctx: OnceLock<egui::Context>,
}

pub struct Thumbs {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Made>,
}

impl Thumbs {
    /// Start `workers` threads; they repaint `ctx` when a thumbnail is
    /// ready.
    pub fn new(workers: usize, ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared { queue: Mutex::default(), wake: Condvar::new(), ctx: OnceLock::new() });
        let _ = shared.ctx.set(ctx);
        let (tx, rx) = mpsc::channel();
        for i in 0..workers.max(1) {
            let shared = shared.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name(format!("thumbnails {i}"))
                .spawn(move || worker(&shared, &tx))
                .expect("spawn thumbnail thread");
        }
        Self { shared, rx }
    }

    /// Make `requests`, in this order, instead of whatever was wanted
    /// before (thumbnails being made now finish anyway).
    pub fn want(&self, requests: Vec<Request>) {
        let mut q = self.shared.queue.lock().unwrap();
        let wanted: VecDeque<Request> =
            requests.into_iter().filter(|r| !q.busy.contains(r) && !q.done.contains(r)).collect();
        if wanted != q.wanted {
            q.wanted = wanted;
            self.shared.wake.notify_all();
        }
    }

    /// A thumbnail, if one is ready.
    pub fn poll(&self) -> Option<Made> {
        let made = self.rx.try_recv().ok()?;
        self.shared.queue.lock().unwrap().done.retain(|r| *r != made.request);
        Some(made)
    }
}

fn worker(shared: &Shared, tx: &mpsc::Sender<Made>) {
    // The shell's thumbnail providers are COM objects.
    crate::win::com_init();
    loop {
        let request = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                if let Some(r) = q.wanted.pop_front() {
                    q.busy.push(r.clone());
                    break r;
                }
                q = shared.wake.wait(q).unwrap();
            }
        };
        let result = make(&request.path, request.side);
        {
            let mut q = shared.queue.lock().unwrap();
            q.busy.retain(|r| *r != request);
            q.done.push(request.clone());
        }
        if tx.send(Made { request, result }).is_err() {
            return;
        }
        if let Some(ctx) = shared.ctx.get() {
            ctx.request_repaint();
        }
    }
}

/// The thumbnail of `path` fitted into `side` x `side` pixels (a smaller
/// image is not enlarged): Windows' own, or decoded here if Windows has
/// none. Call it on a thread where COM is initialised (`win::com_init`).
pub fn make(path: &Path, side: u32) -> Result<Thumbnail, String> {
    use std::os::windows::fs::MetadataExt;
    let started = Instant::now();
    let shell = shell_thumbnail(path, side);
    let thumbnail = match shell {
        Ok((w, h, bgra)) => {
            let file = std::fs::metadata(path).ok();
            let (width, height) = image_size(path, w, h).unwrap_or((0, 0));
            Thumbnail {
                pixels: Pixels { width: w, height: h, levels: loader::mip_levels(bgra, w as usize, h as usize) },
                width,
                height,
                file_size: file.as_ref().map_or(0, |m| m.len()),
                modified: file.as_ref().map_or(0, |m| m.last_write_time()),
            }
        }
        Err(e) => {
            log::debug!("no shell thumbnail of {}: {e}", path.display());
            let (mut img, meta) = loader::read(path)?;
            if img.width() > side || img.height() > side {
                img = img.thumbnail(side, side);
            }
            Thumbnail {
                pixels: loader::to_pixels(img),
                width: meta.width,
                height: meta.height,
                file_size: meta.file_size,
                modified: meta.modified,
            }
        }
    };
    log::trace!("thumbnail of {} in {:.1} ms", path.display(), started.elapsed().as_secs_f64() * 1e3);
    Ok(thumbnail)
}

/// Size of the image in `path` from its header, turned to match the
/// thumbnail `tw` x `th` (Windows applies the EXIF orientation, the header
/// does not).
fn image_size(path: &Path, tw: u32, th: u32) -> Option<(u32, u32)> {
    let (w, h) = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_dimensions().ok()?;
    let aspect = |a: u32, b: u32| a as f32 / b.max(1) as f32;
    let thumb = aspect(tw, th);
    if (aspect(h, w) - thumb).abs() < (aspect(w, h) - thumb).abs() { Some((h, w)) } else { Some((w, h)) }
}

/// `IShellItemImageFactory`, declared by hand: `windows-sys` has no COM
/// interfaces. Only `Release` and `GetImage` are called.
#[repr(C)]
struct ImageFactoryVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
    get_image: unsafe extern "system" fn(
        this: *mut c_void,
        size: windows_sys::Win32::Foundation::SIZE,
        flags: i32,
        bitmap: *mut windows_sys::Win32::Graphics::Gdi::HBITMAP,
    ) -> windows_sys::core::HRESULT,
}

const IID_ISHELLITEMIMAGEFACTORY: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0xbcc18b79_ba16_442f_80c4_8a59c30c463b);

/// The shell's thumbnail of `path` within `side` x `side` pixels, as
/// premultiplied BGRA: `(width, height, pixels)`. Fails for a file the
/// shell has no thumbnail of (rather than returning its icon).
fn shell_thumbnail(path: &Path, side: u32) -> Result<(u32, u32, Vec<u8>), String> {
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::DeleteObject;
    use windows_sys::Win32::UI::Shell::{SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY};
    let wide = crate::win::wide(path);
    let mut factory: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        SHCreateItemFromParsingName(wide.as_ptr(), std::ptr::null_mut(), &IID_ISHELLITEMIMAGEFACTORY, &mut factory)
    };
    if hr < 0 || factory.is_null() {
        return Err(format!("SHCreateItemFromParsingName failed: {hr:#x}"));
    }
    let mut bitmap = std::ptr::null_mut();
    let hr = unsafe {
        let vtbl = &**factory.cast::<*const ImageFactoryVtbl>();
        let size = SIZE { cx: side as i32, cy: side as i32 };
        let hr = (vtbl.get_image)(factory, size, SIIGBF_THUMBNAILONLY, &mut bitmap);
        (vtbl.release)(factory);
        hr
    };
    if hr < 0 || bitmap.is_null() {
        return Err(format!("GetImage failed: {hr:#x}"));
    }
    let pixels = bitmap_pixels(bitmap);
    unsafe { DeleteObject(bitmap) };
    pixels
}

/// The pixels of `bitmap` as premultiplied BGRA, top row first.
fn bitmap_pixels(bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP) -> Result<(u32, u32, Vec<u8>), String> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW, ReleaseDC,
    };
    let mut bm: BITMAP = unsafe { std::mem::zeroed() };
    if unsafe { GetObjectW(bitmap, size_of::<BITMAP>() as i32, (&raw mut bm).cast()) } == 0 {
        return Err("GetObject failed".into());
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 {
        return Err("empty thumbnail".into());
    }
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        // Negative: top row first.
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { std::mem::zeroed() }
    };
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let lines = unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let lines = GetDIBits(dc, bitmap, 0, h as u32, out.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
        ReleaseDC(std::ptr::null_mut(), dc);
        lines
    };
    if lines != h {
        return Err("GetDIBits failed".into());
    }
    fix_alpha(&mut out, bm.bmBitsPixel == 32);
    Ok((w as u32, h as u32, out))
}

/// The shell's bitmaps of opaque images leave alpha at 0: make those
/// opaque. Bitmaps with alpha come premultiplied, as `AlphaBlend` takes
/// them; any that are not are premultiplied here.
fn fix_alpha(bgra: &mut [u8], has_alpha: bool) {
    let px = bgra.as_chunks_mut::<4>().0;
    if !has_alpha || px.iter().all(|p| p[3] == 0) {
        px.iter_mut().for_each(|p| p[3] = 255);
    } else if px.iter().any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]) {
        for p in px {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_thumbs_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn alpha_is_fixed() {
        let mut opaque = vec![10, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut opaque, true);
        assert_eq!(opaque, [10, 20, 30, 255, 40, 50, 60, 255]);
        let mut straight = vec![200, 100, 0, 128];
        fix_alpha(&mut straight, true);
        assert_eq!(straight, [100, 50, 0, 128]);
        let mut premultiplied = vec![100, 50, 0, 128];
        fix_alpha(&mut premultiplied, true);
        assert_eq!(premultiplied, [100, 50, 0, 128]);
    }

    /// Windows' thumbnail of a PNG, or the fallback where thumbnails are
    /// turned off; a QOI, which Windows has no thumbnails of, decoded
    /// here. Either way upright, fitted, with the image's size.
    #[test]
    fn makes_thumbnails() {
        crate::win::com_init();
        let dir = temp_dir("make");
        let png = dir.join("wide.png");
        image::RgbImage::from_pixel(600, 300, image::Rgb([200, 30, 30])).save(&png).unwrap();
        let t = make(&png, 256).unwrap();
        assert_eq!((t.pixels.width, t.pixels.height), (256, 128));
        assert_eq!((t.width, t.height), (600, 300));
        assert_eq!(t.file_size, std::fs::metadata(&png).unwrap().len());
        // Red, opaque, in the middle.
        let mid = (64 * 256 + 128) * 4;
        let p = &t.pixels.levels[0][mid..mid + 4];
        assert!(p[2] > 150 && p[1] < 80 && p[3] == 255, "{p:?}");

        let qoi = dir.join("tall.qoi");
        image::RgbImage::new(100, 400).save(&qoi).unwrap();
        let t = make(&qoi, 96).unwrap();
        assert_eq!((t.pixels.width, t.pixels.height), (24, 96));
        assert_eq!((t.width, t.height), (100, 400));

        // A small image is not enlarged.
        let small = dir.join("small.png");
        image::RgbImage::new(20, 10).save(&small).unwrap();
        let t = make(&small, 256).unwrap();
        assert!(t.pixels.width <= 20, "{}", t.pixels.width);
        assert!(make(&dir.join("missing.png"), 256).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Thumbnails of every image in `QVIEW_THUMB_DIR`, timed, printed with
    /// `--nocapture`.
    #[test]
    #[ignore]
    fn thumbnail_timings() {
        crate::win::com_init();
        let dir = PathBuf::from(std::env::var("QVIEW_THUMB_DIR").expect("QVIEW_THUMB_DIR"));
        let files = crate::folder::list(&dir, None).unwrap();
        let started = Instant::now();
        let mut slowest = (0.0, PathBuf::new());
        for f in &files {
            let t = Instant::now();
            let thumb = make(f, 256);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let size = thumb.map_or_else(|e| e, |t| format!("{}x{} of {}x{}", t.pixels.width, t.pixels.height, t.width, t.height));
            println!("{ms:7.1} ms {size} {}", f.display());
            if ms > slowest.0 {
                slowest = (ms, f.clone());
            }
        }
        println!("{} files in {:.0} ms, slowest {:.1} ms", files.len(), started.elapsed().as_secs_f64() * 1e3, slowest.0);
    }
}
