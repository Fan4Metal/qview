//! Set as Wallpaper: the current image as it is shown (turned, mirrored,
//! cropped while cropping) becomes the desktop background, placed as the
//! Windows settings say (Fill, Fit, Center…).
//!
//! The image is rendered into a PNG of its own in [`folder`], so that any
//! format qview reads (and an image in an archive) can be one, and the
//! picture does not change when its file is edited or deleted. Two names
//! take turns: Windows is given another file than the one it shows.

use std::path::PathBuf;

use crate::edit;
use crate::win::wide;

/// Where the wallpaper's file is kept: `%LOCALAPPDATA%\qview`, which,
/// unlike the temporary folder, is not cleared behind Windows' back.
fn folder() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from).join("qview")
}

/// Make `job`'s image (see `edit::render`) the desktop background. Windows'
/// codecs may decode it, so the calling thread has COM initialised.
pub fn set(job: &edit::Job) -> Result<(), String> {
    let png = edit::render_png(job)?;
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let shown = current();
    let path = ["Wallpaper 1.png", "Wallpaper 2.png"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|p| !shown.as_ref().is_some_and(|s| crate::folder::same_path(s, p)))
        .expect("two names");
    std::fs::write(&path, png).map_err(|e| e.to_string())?;
    apply(&path)
}

/// The file of the desktop background, if it is one.
fn current() -> Option<PathBuf> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETDESKWALLPAPER, SystemParametersInfoW};
    let mut buf = [0u16; 1024];
    let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr().cast(), 0) };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (ok != 0 && len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// Show `path` on the desktop and keep it in the user's settings. Every
/// top-level window is told, so this waits for them: off the UI thread.
fn apply(path: &std::path::Path) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
    };
    let path = wide(path);
    let ok = unsafe {
        SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, path.as_ptr() as *mut _, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
