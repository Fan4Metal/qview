//! One window per copy of qview: a second start hands its file to the
//! window already open, brings that window forward and exits.
//!
//! The first start owns a named mutex; once its window exists it also has a
//! message-only window of a class with the same name, which receives the
//! path in a `WM_COPYDATA`. Both names are derived from the path of the exe,
//! so an installed copy and a development build do not take each other's
//! files.

use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, FindWindowExW, HWND_MESSAGE, IsIconic, RegisterClassW, SMTO_ABORTIFHUNG,
    SMTO_BLOCK, SW_RESTORE, SendMessageTimeoutW, SetForegroundWindow, ShowWindow, WM_COPYDATA, WNDCLASSW,
};

use crate::win::wide;

/// Marks a `WM_COPYDATA` as qview's (`COPYDATASTRUCT::dwData`).
const MAGIC: usize = 0x7176_6965;
/// How long a second start waits for the window of the first one, which
/// appears ~150 ms after its start, or for a closing one to exit.
const WAIT: Duration = Duration::from_secs(5);
/// How long the first start may take to answer.
const ANSWER_MS: u32 = 3000;

/// What later starts handed over since `App::ui` last took it.
#[derive(Default)]
pub struct Received {
    /// The last path; earlier ones are dropped (Explorer starts one
    /// process per file of a selection).
    pub path: Option<PathBuf>,
    /// The main window was minimized and has been restored cloaked: `App`
    /// shows it once the new image is on screen.
    pub cloaked: bool,
}

static RECEIVED: Mutex<Received> = Mutex::new(Received { path: None, cloaked: false });
/// The UI to wake when a path arrives, and the main window.
static TARGET: OnceLock<(egui::Context, isize)> = OnceLock::new();

/// Name of the mutex and of the window class, the same for every start of
/// this exe in this session.
fn name() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    exe.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("qview.{:016x}", hasher.finish())
}

/// Called at start-up. If qview is already running from this exe, hand it
/// `path` (none: only bring its window forward) and return true: this
/// process has nothing more to do. Otherwise this process is the first one
/// and stays so until it exits.
pub fn forward(path: Option<&Path>) -> bool {
    let name = name();
    let mutex_name = wide(format!("Local\\{name}"));
    let class = wide(&name);
    let deadline = Instant::now() + WAIT;
    loop {
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            log::warn!("CreateMutexW failed: {}", std::io::Error::last_os_error());
            return false;
        }
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            // The first start: the handle stays open while the process lives.
            return false;
        }
        unsafe { CloseHandle(mutex) };
        let window = unsafe { FindWindowExW(HWND_MESSAGE, std::ptr::null_mut(), class.as_ptr(), std::ptr::null()) };
        if !window.is_null() {
            // A first start that does not answer (hung) is left alone and
            // this one runs on its own.
            return send(window, path);
        }
        // The first start has not made its window yet, or is closing.
        if Instant::now() >= deadline {
            log::warn!("qview is running but its window was not found");
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Send `path` to the message window `to`, then bring the main window it
/// answers with to the foreground: this process was started by the user,
/// so it may.
fn send(to: HWND, path: Option<&Path>) -> bool {
    let text: Vec<u16> = path.map(wide).unwrap_or_default();
    // Without the terminating NUL; none at all for no path.
    let bytes = text.len().saturating_sub(1) * 2;
    let data = COPYDATASTRUCT { dwData: MAGIC, cbData: bytes as u32, lpData: text.as_ptr() as *mut _ };
    let mut answer = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            to,
            WM_COPYDATA,
            0,
            &raw const data as LPARAM,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            ANSWER_MS,
            &mut answer,
        )
    };
    if ok == 0 || answer == 0 {
        log::warn!("the running qview did not take the file");
        return false;
    }
    unsafe { SetForegroundWindow(answer as HWND) };
    true
}

/// Make this process the one later starts hand their files to: a
/// message-only window on the UI thread, whose messages the event loop
/// dispatches. `main` is the app's window.
pub fn listen(ctx: &egui::Context, main: isize) {
    if TARGET.set((ctx.clone(), main)).is_err() {
        return;
    }
    let class = wide(name());
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            log::warn!("RegisterClassW failed: {}", std::io::Error::last_os_error());
            return;
        }
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if window.is_null() {
            log::warn!("CreateWindowExW failed: {}", std::io::Error::last_os_error());
        }
    }
}

/// What was handed over since the previous call.
pub fn take() -> Received {
    std::mem::take(&mut *RECEIVED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Takes a path from a later start, restores the main window if it is
/// minimized and answers with it (0: not a message of qview).
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg != WM_COPYDATA || lparam == 0 {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let data = unsafe { &*(lparam as *const COPYDATASTRUCT) };
    let Some((ctx, main)) = TARGET.get().filter(|_| data.dwData == MAGIC) else {
        return 0;
    };
    let path = (data.cbData > 0 && !data.lpData.is_null()).then(|| {
        // Read as bytes: the copy the system makes need not be aligned.
        let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        PathBuf::from(OsString::from_wide(&units))
    });
    let minimized = unsafe { IsIconic(*main as HWND) } != 0;
    // Restored at once, the window would show the old image until the new
    // one is decoded: it is restored cloaked, and `App` uncloaks it.
    let cloak = minimized && path.is_some();
    {
        let mut received = RECEIVED.lock().unwrap_or_else(|e| e.into_inner());
        if path.is_some() {
            received.path = path;
        }
        received.cloaked |= cloak;
    }
    if cloak {
        crate::win::cloak(*main, true);
    }
    if minimized {
        unsafe { ShowWindow(*main as HWND, SW_RESTORE) };
    }
    ctx.request_repaint();
    *main as LRESULT
}
