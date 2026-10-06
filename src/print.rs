//! Print: Windows' Print Pictures dialog (printer, paper, layouts, copies),
//! the one Explorer's Print opens for images, given the current image or
//! those chosen in the gallery. Explorer reaches it as a drop target
//! (`SystemFileAssociations\image\shell\print\DropTarget`), and so does
//! this: the files are "dropped" on it.
//!
//! The dialog reads files through Windows' codecs, so a file goes to it as
//! it is only when Windows reads it upright on its own (`as_is`); any
//! other, an image turned or cropped in the viewer, or one in an archive,
//! is rendered as shown into a PNG in [`folder`] (with its ICC profile).

use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::POINTL;
use windows_sys::core::{GUID, HRESULT};

use crate::edit;
use crate::wic::{Com, Unknown};

/// Shell's `CLSID_PrintPhotosDropTarget` (photowiz.dll).
const CLSID_PRINT_PHOTOS_DROP_TARGET: GUID = GUID::from_u128(0x60fd46de_f830_4894_a628_6fa81bc0190d);
const IID_IDROP_TARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);

/// `IDropTarget::DragEnter` and `Drop`.
type DropMethod = unsafe extern "system" fn(Unknown, Unknown, u32, POINTL, *mut u32) -> HRESULT;

// Vtable indices, from oleidl.h.
const DRAG_ENTER: usize = 3;
const DROP: usize = 6;

const MK_LBUTTON: u32 = 1;
const DROPEFFECT_COPY: u32 = 1;

/// The extensions of the formats Windows reads without extensions from the
/// Store.
const NATIVE: [&str; 10] = ["jpg", "jpeg", "jpe", "jfif", "png", "bmp", "dib", "gif", "tif", "tiff"];

/// Rendered files older than this are deleted before others are made: the
/// dialog has long been closed.
const KEEP: Duration = Duration::from_secs(60 * 60);

/// Where images rendered for printing go: `Print` in qview's temporary
/// folder.
fn folder() -> PathBuf {
    crate::clipboard::folder().join("Print")
}

/// `job` is printed from its file as it is: nothing to turn or crop, a
/// file on disk in a format Windows reads, upright without its EXIF
/// orientation. Only the last needs the file read.
fn as_is(job: &edit::Job) -> bool {
    use image::metadata::Orientation;
    plain(job) && crate::header::orientation(&job.src) == Orientation::NoTransforms
}

/// `as_is` without reading the file: when it is false, `prepare` renders.
pub fn plain(job: &edit::Job) -> bool {
    let native = job.src.extension().and_then(|e| e.to_str()).is_some_and(|e| NATIVE.contains(&e.to_ascii_lowercase().as_str()));
    native && job.turns.is_multiple_of(4) && !job.flip && job.crop.is_none() && !crate::archive::inside(&job.src)
}

/// The files to print for `jobs`: theirs when they are printed as they are
/// (`as_is`), otherwise the image rendered into [`folder`]. Windows' codecs
/// may decode them, so the calling thread has COM initialised.
pub fn prepare(jobs: &[edit::Job]) -> Result<Vec<PathBuf>, String> {
    let dir = folder();
    if jobs.iter().any(|j| !as_is(j)) {
        forget_old(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    jobs.iter()
        .map(|job| {
            if as_is(job) {
                return Ok(job.src.clone());
            }
            let png = edit::render_png(job).map_err(|e| format!("{}: {e}", crate::app::file_name(&job.src)))?;
            let name = edit::suggested_name(&job.src, "png", "", |n| dir.join(n).exists());
            let path = dir.join(name);
            std::fs::write(&path, png).map_err(|e| e.to_string())?;
            Ok(path)
        })
        .collect()
}

/// Delete the files rendered more than [`KEEP`] ago.
fn forget_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > KEEP));
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Open the Print Pictures dialog for `files`. On the UI thread: the
/// dialog may call back into the data object made here, which needs a
/// thread that dispatches messages.
pub fn show(files: &[PathBuf]) -> Result<(), String> {
    use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    let _com = crate::win::com_init();
    let data = crate::editors::data_object(files)?;
    let mut target = null_mut();
    let hr = unsafe {
        CoCreateInstance(&CLSID_PRINT_PHOTOS_DROP_TARGET, null_mut(), CLSCTX_INPROC_SERVER, &IID_IDROP_TARGET, &mut target)
    };
    if hr < 0 || target.is_null() {
        return Err(tr!(
            format!("Windows has no Print Pictures dialog (error {hr:#x})"),
            format!("В Windows нет окна «Печать изображений» (ошибка {hr:#x})")
        ));
    }
    let target = Com(target);
    let at = POINTL { x: 0, y: 0 };
    let mut effect = DROPEFFECT_COPY;
    let hr = unsafe { target.method::<DropMethod>(DRAG_ENTER)(target.0, data.0, MK_LBUTTON, at, &mut effect) };
    if hr < 0 || effect == 0 {
        return Err(tr!(format!("the dialog refused the files (error {hr:#x})"), format!("окно не приняло файлы (ошибка {hr:#x})")));
    }
    let hr = unsafe { target.method::<DropMethod>(DROP)(target.0, data.0, 0, at, &mut effect) };
    if hr < 0 {
        return Err(tr!(format!("error {hr:#x}"), format!("ошибка {hr:#x}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(src: &str, turns: u8) -> edit::Job {
        edit::Job { src: src.into(), dst: PathBuf::new(), size: [0, 0], turns, flip: false, crop: None }
    }

    #[test]
    fn what_is_rendered() {
        assert!(plain(&job(r"C:\a\photo.JPG", 0)));
        assert!(plain(&job(r"C:\a\scan.tiff", 4)));
        assert!(!plain(&job(r"C:\a\photo.jpg", 1)));
        assert!(!plain(&job(r"C:\a\photo.webp", 0)));
        assert!(!plain(&job(r"C:\a\photo.heic", 0)));
        assert!(!plain(&edit::Job { flip: true, ..job(r"C:\a\photo.png", 0) }));
        assert!(!plain(&edit::Job { crop: Some([0, 0, 1, 1]), ..job(r"C:\a\photo.png", 0) }));
    }

    #[test]
    fn rendered_as_png() {
        let _com = crate::win::com_init();
        let dir = std::env::temp_dir().join(format!("qview_print_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.qoi");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255])).save(&src).unwrap();
        let png = dir.join("b.png");
        image::RgbaImage::new(2, 2).save(&png).unwrap();
        let files = prepare(&[job(src.to_str().unwrap(), 1), job(png.to_str().unwrap(), 0)]).unwrap();
        assert!(files[0].starts_with(folder()) && files[0].extension().unwrap() == "png");
        assert_eq!(image::open(&files[0]).unwrap().into_rgba8().dimensions(), (2, 3));
        assert_eq!(files[1], png);
        std::fs::remove_file(&files[0]).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Opens Windows' Print Pictures dialog for `QVIEW_PRINT_FILE`, with
    /// `--nocapture` printing how long `Drop` took; the dialog stays while
    /// messages are dispatched (up to a minute).
    #[test]
    #[ignore]
    fn print_dialog() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
        let file = PathBuf::from(std::env::var("QVIEW_PRINT_FILE").expect("QVIEW_PRINT_FILE"));
        let _com = crate::win::com_init();
        let start = std::time::Instant::now();
        show(&[file]).unwrap();
        println!("Drop returned after {:?}", start.elapsed());
        let until = std::time::Instant::now() + Duration::from_secs(60);
        while std::time::Instant::now() < until {
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
