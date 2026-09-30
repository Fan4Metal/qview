//! Thin Win32 helpers: Explorer's name order, the Recycle Bin, the shell,
//! the clipboard, local dates and the interface language.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

/// `s` as a NUL-terminated UTF-16 string for Win32.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Whether Windows shows its interface in Russian (primary language of
/// the user's UI language, `LANG_RUSSIAN`).
pub fn ui_language_is_russian() -> bool {
    const LANG_RUSSIAN: u16 = 0x19;
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    id & 0x3FF == LANG_RUSSIAN
}

/// Order of two NUL-terminated wide file names as Explorer sorts them:
/// case-insensitive, digits compared as numbers (`2.jpg` before `10.jpg`).
pub fn logical_cmp(a: &[u16], b: &[u16]) -> Ordering {
    debug_assert!(a.last() == Some(&0) && b.last() == Some(&0));
    let r = unsafe { windows_sys::Win32::UI::Shell::StrCmpLogicalW(a.as_ptr(), b.as_ptr()) };
    r.cmp(&0)
}

/// Move `path` to the Recycle Bin through the shell, which asks before
/// deleting for good a file that cannot be recycled (a network drive, a
/// file too large for the bin). Blocks until done, so call it off the UI
/// thread. `Err` says why it stopped; either way, check what is left on
/// disk.
pub fn recycle(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    // pFrom is a list of paths ending with an empty one: two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        // The confirmation is the app's own; the shell still warns before a
        // permanent delete. No progress window for a single file.
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_SILENT) as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if op.fAnyOperationsAborted != 0 {
        Err("cancelled".into())
    } else if code != 0 {
        Err(format!("error {code:#x}"))
    } else {
        Ok(())
    }
}

/// Open Explorer on the folder of `path` with the file selected.
pub fn show_in_explorer(path: &Path) {
    use std::os::windows::process::CommandExt;
    // raw_arg: Explorer parses its own command line. Paths cannot contain
    // quotes on Windows, so plain quoting is safe.
    let result = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
    if let Err(e) = result {
        log::warn!("explorer.exe failed for {}: {e}", path.display());
    }
}

/// Open `path` in the program Windows associates with it.
pub fn open_default(path: &Path) -> bool {
    shell_execute(path.as_os_str(), path.parent())
}

/// Open `target` (a file, a folder or a URI such as `ms-settings:…`) as
/// Explorer would.
pub fn shell_open(target: impl AsRef<OsStr>) -> bool {
    shell_execute(target.as_ref(), None)
}

fn shell_execute(target: &OsStr, dir: Option<&Path>) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = wide("open");
    let file = wide(target);
    let dir = dir.map(wide);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    result as isize > 32
}

/// Put `path` on the clipboard as a file (`CF_HDROP`), the way Explorer's
/// Copy does, so that it can be pasted into a folder or a message.
pub fn copy_file(path: &Path) -> Result<(), String> {
    let text = path.to_str().ok_or("the path is not valid Unicode")?;
    let _clipboard = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    clipboard_win::raw::set_file_list_with(&[text], clipboard_win::options::DoClear).map_err(|e| e.to_string())
}

/// A FILETIME (100 ns ticks since 1601, UTC) as a local date and time in
/// the user's short date and long time formats, as Explorer shows it:
/// `16.04.2012 15:01:26`.
pub fn local_date_time(filetime: u64) -> Option<String> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ft = FILETIME {
        dwLowDateTime: filetime as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    let mut date = [0u16; 128];
    let mut time = [0u16; 128];
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
        // A null locale name is LOCALE_NAME_USER_DEFAULT.
        let d = GetDateFormatEx(
            std::ptr::null(),
            DATE_SHORTDATE,
            &local,
            std::ptr::null(),
            date.as_mut_ptr(),
            date.len() as i32,
            std::ptr::null(),
        );
        let t = GetTimeFormatEx(std::ptr::null(), 0, &local, std::ptr::null(), time.as_mut_ptr(), time.len() as i32);
        if d <= 0 || t <= 0 {
            return None;
        }
        // The lengths count the terminating NUL.
        Some(format!(
            "{} {}",
            String::from_utf16_lossy(&date[..d as usize - 1]),
            String::from_utf16_lossy(&time[..t as usize - 1])
        ))
    }
}

/// Give window `hwnd` the dark caption and frame
/// (`DWMWA_USE_IMMERSIVE_DARK_MODE`). Called while the window is still
/// hidden, it is shown dark from the start; set later, Windows 10 would keep
/// the old colours until the next repaint of the frame. dwmapi is declared
/// by hand, to spare another `windows-sys` feature.
pub fn dark_caption(hwnd: isize) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    dwm_set_bool(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, true);
}

/// Hide window `hwnd` from the screen (`DWMWA_CLOAK`) or show it again,
/// without changing its state: a cloaked window can be shown, maximized
/// and painted unseen.
pub fn cloak(hwnd: isize, on: bool) {
    const DWMWA_CLOAK: u32 = 13;
    dwm_set_bool(hwnd, DWMWA_CLOAK, on);
}

/// Show window `hwnd` maximized.
pub fn show_maximized(hwnd: isize) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    }
    const SW_MAXIMIZE: i32 = 3;
    unsafe {
        ShowWindow(hwnd, SW_MAXIMIZE);
    }
}

/// A BOOL window attribute of DWM. dwmapi is declared by hand, to spare
/// another `windows-sys` feature.
fn dwm_set_bool(hwnd: isize, attribute: u32, on: bool) {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const core::ffi::c_void, size: u32) -> i32;
    }
    let value = i32::from(on);
    unsafe {
        DwmSetWindowAttribute(hwnd, attribute, (&raw const value).cast(), 4);
    }
}

/// Show `text` in a system message box with an error icon, waiting until
/// it is closed. It needs no window of the app, so it also works when the
/// app is failing.
pub fn error_box(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_SETFOREGROUND, MessageBoxW};
    let (text, title) = (wide(text), wide(title));
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR | MB_SETFOREGROUND);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_order() {
        let cmp = |a: &str, b: &str| logical_cmp(&wide(a), &wide(b));
        assert_eq!(cmp("2.jpg", "10.jpg"), Ordering::Less);
        assert_eq!(cmp("IMG_9.jpg", "img_10.jpg"), Ordering::Less);
        assert_eq!(cmp("a.jpg", "A.jpg"), Ordering::Equal);
        assert_eq!(cmp("Яблоко.png", "арбуз.png"), Ordering::Greater);
    }

    #[test]
    fn local_dates() {
        // 2012-04-16 11:01:26 UTC; the local form depends on the machine,
        // so only check that the year and the seconds are there.
        let unix = 1_334_574_086u64;
        let filetime = (unix + 11_644_473_600) * 10_000_000;
        let text = local_date_time(filetime).unwrap();
        assert!(text.contains("2012"), "{text}");
        assert!(text.contains(":26"), "{text}");
    }
}
