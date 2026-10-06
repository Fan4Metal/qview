//! Thin Win32 helpers: Explorer's name order, the Recycle Bin, the shell,
//! the clipboard, local dates and the interface language.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

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

/// Move `paths` to the Recycle Bin through the shell, which asks before
/// deleting for good a file that cannot be recycled (a network drive, a
/// file too large for the bin). Blocks until done, so call it off the UI
/// thread. The shell's questions are owned by window `owner`, so they come
/// in front of it. `Err` says why it stopped; either way, check what is
/// left on disk.
pub fn recycle(paths: &[PathBuf], owner: Option<isize>) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    // pFrom is a list of paths, each NUL-terminated, ending with an empty one.
    let mut from: Vec<u16> = paths.iter().flat_map(wide).collect();
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: owner.unwrap_or(0) as windows_sys::Win32::Foundation::HWND,
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        // The confirmation is the app's own; the shell still warns before a
        // permanent delete. No progress window for a single file; for
        // several, the shell's.
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | if paths.len() == 1 { FOF_SILENT } else { 0 }) as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if op.fAnyOperationsAborted != 0 {
        Err("cancelled".into())
    } else if code != 0 {
        Err(tr!(format!("error {code:#x}"), format!("ошибка {code:#x}")))
    } else {
        Ok(())
    }
}

/// Put `replacement` in the place of `target`, which keeps its creation
/// date, attributes and permissions; `replacement` is gone afterwards.
pub fn replace_file(target: &Path, replacement: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};
    let (target, replacement) = (wide(target), wide(replacement));
    let ok = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Open Explorer on the folder of `path` with the file selected, in front
/// of qview. `explorer.exe /select` is not used: the process it starts
/// hands the folder to the Explorer already running, which Windows then
/// does not let in front of the window the user clicked in (its taskbar
/// button flashes instead). `SHOpenFolderAndSelectItems` asks that
/// Explorer itself and lets it come forward. On a thread: the shell may
/// take a while.
pub fn show_in_explorer(path: &Path) {
    use windows_sys::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
    let owned = path.to_path_buf();
    let spawned = std::thread::Builder::new().name("show in explorer".into()).spawn(move || {
        let path = owned;
        let _com = com_init();
        let wide = wide(&path);
        let shown = unsafe {
            let item = ILCreateFromPathW(wide.as_ptr());
            if item.is_null() {
                false
            } else {
                // No items: `item` itself is selected in its folder.
                let hr = SHOpenFolderAndSelectItems(item, 0, std::ptr::null(), 0);
                ILFree(item);
                hr >= 0
            }
        };
        if !shown {
            log::warn!("SHOpenFolderAndSelectItems failed for {}", path.display());
            explorer_select(&path);
        }
    });
    if let Err(e) = spawned {
        log::warn!("cannot start a thread to show {}: {e}", path.display());
    }
}

/// `explorer.exe /select,<path>`, should the shell call fail.
fn explorer_select(path: &Path) {
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

/// GET `https://host/path` through WinHTTP (the system's TLS and proxy, no
/// HTTP crate): the status code and the body, at most `limit` bytes. `headers` are CRLF-separated. Blocks: call it on a
/// thread.
pub fn https_get(
    host: &str,
    path: &str,
    agent: &str,
    headers: &str,
    timeout_ms: i32,
    limit: usize,
) -> Result<(u32, Vec<u8>), String> {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Networking::WinHttp::*;

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
    fn handle(h: *mut c_void) -> Result<Handle, String> {
        if h.is_null() { Err(winhttp_error()) } else { Ok(Handle(h)) }
    }
    fn check(ok: windows_sys::core::BOOL) -> Result<(), String> {
        if ok == 0 { Err(winhttp_error()) } else { Ok(()) }
    }

    let (agent, host, path, headers) = (wide(agent), wide(host), wide(path), wide(headers));
    let verb = wide("GET");
    let t = timeout_ms;
    // Declared in this order, the handles close request first.
    unsafe {
        let session = handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
        check(WinHttpSetTimeouts(session.0, t, t, t, t))?;
        let connection = handle(WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0))?;
        let request = handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        check(WinHttpSendRequest(request.0, headers.as_ptr(), (headers.len() - 1) as u32, null(), 0, 0, 0))?;
        check(WinHttpReceiveResponse(request.0, null_mut()))?;
        let mut status = 0u32;
        let mut len = size_of::<u32>() as u32;
        check(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&raw mut status).cast(),
            &mut len,
            null_mut(),
        ))?;
        let mut body = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            check(WinHttpReadData(request.0, buf.as_mut_ptr().cast(), buf.len() as u32, &mut read))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > limit {
                return Err(tr!(format!("the answer is larger than {limit} bytes"), format!("ответ больше {limit} байт")));
            }
        }
        Ok((status, body))
    }
}

/// The last WinHTTP error in words; the system's message table lacks them.
fn winhttp_error() -> String {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    match code {
        12002 => tr!("the server did not answer in time", "сервер не ответил вовремя").into(),
        12007 => tr!("the server name could not be resolved", "не удалось найти адрес сервера").into(),
        12029 | 12030 => tr!("the connection to the server failed", "не удалось соединиться с сервером").into(),
        12175 => tr!("the secure connection failed", "не удалось установить защищённое соединение").into(),
        _ => tr!(format!("WinHTTP error {code}"), format!("ошибка WinHTTP {code}")),
    }
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
    copy_files(&[path.to_path_buf()])
}

/// `paths` to the clipboard as files, to paste in Explorer.
pub fn copy_files(paths: &[PathBuf]) -> Result<(), String> {
    let texts = paths.iter().map(|p| p.to_str().ok_or("the path is not valid Unicode")).collect::<Result<Vec<_>, _>>()?;
    let _clipboard = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    clipboard_win::raw::set_file_list_with(&texts, clipboard_win::options::DoClear).map_err(|e| e.to_string())
}

/// Copy `files` into the folder `to` as Explorer does: the shell shows the
/// progress and asks about files of the same name; the questions belong to
/// `owner`. Blocks until done: call it on a thread.
pub fn copy_to(files: &[PathBuf], to: &Path, owner: Option<isize>) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{FO_COPY, FOF_ALLOWUNDO, FOF_NOCONFIRMMKDIR, SHFILEOPSTRUCTW, SHFileOperationW};
    // Lists of paths, each NUL-terminated, ending with an empty one.
    let mut from = Vec::new();
    for f in files {
        from.extend(wide(f));
    }
    from.push(0);
    let mut dest = wide(to);
    dest.push(0);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: owner.unwrap_or(0) as windows_sys::Win32::Foundation::HWND,
        wFunc: FO_COPY,
        pFrom: from.as_ptr(),
        pTo: dest.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR) as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if op.fAnyOperationsAborted != 0 {
        Err("cancelled".into())
    } else if code != 0 {
        Err(tr!(format!("error {code:#x}"), format!("ошибка {code:#x}")))
    } else {
        Ok(())
    }
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
/// the old colours until the next repaint of the frame.
pub fn dark_caption(hwnd: isize) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    // The attribute's number before Windows 10 20H1 (1809 to 1909).
    const DWMWA_USE_IMMERSIVE_DARK_MODE_OLD: u32 = 19;
    if !dwm_set_bool(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, true) {
        dwm_set_bool(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_OLD, true);
    }
}

/// Hide window `hwnd` from the screen (`DWMWA_CLOAK`) or show it again,
/// without changing its state: a cloaked window can be shown, maximized
/// and painted unseen.
pub fn cloak(hwnd: isize, on: bool) {
    const DWMWA_CLOAK: u32 = 13;
    if !dwm_set_bool(hwnd, DWMWA_CLOAK, on) {
        log::warn!("DWMWA_CLOAK {on} failed");
    }
}

/// The rectangle of window `hwnd` when it is neither maximized nor
/// minimized (left, top, right, bottom in workspace coordinates), also
/// while it is maximized or minimized.
pub fn normal_rect(hwnd: isize) -> Option<[i32; 4]> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, WINDOWPLACEMENT};
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = size_of::<WINDOWPLACEMENT>() as u32;
    if unsafe { GetWindowPlacement(hwnd as _, &mut placement) } == 0 {
        return None;
    }
    let r = placement.rcNormalPosition;
    (r.right > r.left && r.bottom > r.top).then_some([r.left, r.top, r.right, r.bottom])
}

/// Show window `hwnd` maximized, restored later to `normal` (as given by
/// [`normal_rect`]); without it, to `default` (in points) centred where the
/// window is. `ShowWindow(SW_MAXIMIZE)` alone would restore it to the size
/// it was created with, the maximized one eframe saved.
pub fn show_maximized(hwnd: isize, normal: Option<[i32; 4]>, default: [f32; 2]) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT,
    };
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: isize) -> u32;
    }
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = size_of::<WINDOWPLACEMENT>() as u32;
    unsafe { GetWindowPlacement(hwnd as _, &mut placement) };
    let [left, top, right, bottom] = normal.unwrap_or_else(|| {
        // Centred where the window is now, in the work area's coordinates
        // as `rcNormalPosition` (GetWindowRect's are the screen's: a
        // taskbar at the top or left would shift it).
        let r = placement.rcNormalPosition;
        let scale = match unsafe { GetDpiForWindow(hwnd) } {
            0 => 1.0,
            dpi => dpi as f32 / 96.0,
        };
        let (w, h) = ((default[0] * scale) as i32, (default[1] * scale) as i32);
        let (x, y) = ((r.left + r.right - w) / 2, (r.top + r.bottom - h) / 2);
        [x, y, x + w, y + h]
    });
    placement.rcNormalPosition = RECT { left, top, right, bottom };
    placement.showCmd = SW_SHOWMAXIMIZED as u32;
    placement.flags = 0;
    // Windows moves a rectangle that would be off every screen onto one.
    unsafe { SetWindowPlacement(hwnd as _, &placement) };
}

/// Set a BOOL window attribute of DWM; false if DWM refused it. dwmapi is
/// declared by hand, to spare another `windows-sys` feature.
fn dwm_set_bool(hwnd: isize, attribute: u32, on: bool) -> bool {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const core::ffi::c_void, size: u32) -> i32;
    }
    let value = i32::from(on);
    unsafe { DwmSetWindowAttribute(hwnd, attribute, (&raw const value).cast(), 4) >= 0 }
}

thread_local! {
    /// Guards of [`com_init`] alive on this thread that initialised COM.
    static COM_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Initialise COM on this thread (single-threaded apartment), as the
/// shell's thumbnail providers, `SHGetFileInfoW` and WIC need, until the
/// guard is dropped. Guards may nest; the outermost one releases what WIC
/// keeps for the thread (`wic::release_kept`) before COM goes.
#[must_use = "COM is released when the guard is dropped"]
pub fn com_init() -> Com {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
    let hr = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
    if hr >= 0 {
        COM_DEPTH.set(COM_DEPTH.get() + 1);
    }
    Com { initialised: hr >= 0 }
}

/// How many guards of [`com_init`] are alive on this thread.
pub fn com_depth() -> u32 {
    COM_DEPTH.get()
}

/// COM on this thread (see [`com_init`]); released when dropped, if it was
/// initialised.
pub struct Com {
    initialised: bool,
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.initialised {
            let depth = COM_DEPTH.get() - 1;
            COM_DEPTH.set(depth);
            if depth == 0 {
                crate::wic::release_kept();
            }
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// Give a command-line mode the console of the process that started it
/// as standard error when it has none: the release build is a GUI program,
/// and a parent such as Python's `subprocess` (the release script) passes
/// it no handles, so `eprintln!` went nowhere. Attaching alone leaves the
/// handle empty: the console's output is opened and set as standard error.
pub fn attach_parent_console() {
    use std::os::windows::io::IntoRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(id: u32) -> isize;
        fn SetStdHandle(id: u32, handle: isize) -> i32;
        fn AttachConsole(pid: u32) -> i32;
    }
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if (handle != 0 && handle != -1) || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    // Read access too: Rust's stderr checks with GetConsoleMode, which needs
    // it, that the handle is a console, and then writes Unicode to it.
    if let Ok(console) = std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$") {
        // Kept open for the rest of the process.
        unsafe { SetStdHandle(STD_ERROR_HANDLE, console.into_raw_handle() as isize) };
    }
}

/// The user's Pictures and Desktop folders, those that exist.
pub fn known_folders() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Pictures, SHGetKnownFolderPath};
    let mut out = Vec::new();
    for id in [FOLDERID_Pictures, FOLDERID_Desktop] {
        let mut path = std::ptr::null_mut();
        let hr = unsafe { SHGetKnownFolderPath(&id, 0, std::ptr::null_mut(), &mut path) };
        if hr >= 0 && !path.is_null() {
            let len = (0..).take_while(|&i| unsafe { *path.add(i) } != 0).count();
            let wide = unsafe { std::slice::from_raw_parts(path, len) };
            out.push(PathBuf::from(std::ffi::OsString::from_wide(wide)));
        }
        unsafe { CoTaskMemFree(path.cast()) };
    }
    out.retain(|p| p.is_dir());
    out
}

/// The roots of the drives, `C:\` and on.
pub fn drives() -> Vec<PathBuf> {
    let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    (0..26u8).filter(|i| mask & (1 << i) != 0).map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char))).collect()
}

/// The name Explorer shows for `path`: `Media (H:)` for a drive, the
/// localised name of a known folder (`Изображения`). It may wait for a
/// slow or disconnected drive: call it off the UI thread, with COM
/// initialised (`com_init`).
pub fn display_name(path: &Path) -> Option<String> {
    use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_DISPLAYNAME, SHGetFileInfoW};
    let wide = wide(path);
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let ok = unsafe { SHGetFileInfoW(wide.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_DISPLAYNAME) };
    let len = info.szDisplayName.iter().position(|&c| c == 0).unwrap_or(0);
    (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&info.szDisplayName[..len]))
}

/// Whether a file with `attributes` (from `MetadataExt::file_attributes`)
/// is a folder Explorer shows: not hidden.
pub fn is_visible_folder(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && attributes & FILE_ATTRIBUTE_HIDDEN == 0
}

/// The longest pause between the clicks of a double click set in Windows,
/// in seconds (500 ms by default).
pub fn double_click_time() -> f64 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDoubleClickTime() -> u32;
    }
    unsafe { GetDoubleClickTime() as f64 / 1000.0 }
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

/// Milliseconds from the creation of this process (loading the exe and its
/// DLLs) to now, for the start-up trace.
pub fn ms_since_process_start() -> Option<f64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) };
    if ok == 0 {
        return None;
    }
    let mut now = zero;
    unsafe { GetSystemTimePreciseAsFileTime(&mut now) };
    let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Some(ticks(now).saturating_sub(ticks(created)) as f64 / 10_000.0)
}


/// A local date and time (as ZIP keeps them) as a FILETIME, in UTC.
pub fn local_filetime(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<u64> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};
    let local = SYSTEMTIME {
        wYear: year,
        wMonth: month as u16,
        wDayOfWeek: 0,
        wDay: day as u16,
        wHour: hour as u16,
        wMinute: minute as u16,
        wSecond: second as u16,
        wMilliseconds: 0,
    };
    let mut utc = SYSTEMTIME::default();
    let mut ft = FILETIME::default();
    unsafe {
        if TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut utc) == 0
            || SystemTimeToFileTime(&utc, &mut ft) == 0
        {
            return None;
        }
    }
    Some(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
}

/// Set by [`watch_paste`]'s hook when Ctrl+V or Shift+Insert is pressed.
static PASTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Watch the keys of this thread (the UI thread) for Ctrl+V and
/// Shift+Insert; [`take_paste`] says whether they were pressed. egui-winit
/// reads the clipboard's text for these keys and passes on nothing at all
/// when it holds none (an image, files), so they are taken from Windows: a
/// keyboard hook of this thread alone (`WH_KEYBOARD`, no other process sees
/// it), which only looks and passes every key on.
pub fn watch_paste() {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_KEYBOARD};
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD, Some(paste_hook), std::ptr::null_mut(), GetCurrentThreadId()) };
    if hook.is_null() {
        log::warn!("cannot watch for Ctrl+V: {}", std::io::Error::last_os_error());
    }
}

/// Whether Ctrl+V or Shift+Insert was pressed since the last call.
pub fn take_paste() -> bool {
    PASTE.swap(false, std::sync::atomic::Ordering::Relaxed)
}

unsafe extern "system" fn paste_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_INSERT, VK_MENU, VK_SHIFT, VK_V};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CallNextHookEx, HC_ACTION};
    // HC_ACTION: the message is taken from the queue (not only peeked at).
    // Bit 31 of lParam: released; bit 30: down before (a repeat).
    if code == HC_ACTION as i32 && lparam & (3 << 30) == 0 {
        let down = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
        let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
        let key = wparam as u16;
        if (key == VK_V && ctrl && !shift && !alt) || (key == VK_INSERT && shift && !ctrl && !alt) {
            PASTE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// The local date and time as a file name may hold it:
/// `2026-10-06 10-30-15`.
pub fn local_stamp() -> String {
    let mut t = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
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
