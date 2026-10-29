//! Watching the folder shown for what other programs do to it
//! (`ReadDirectoryChangesW`): files added, deleted, renamed or written,
//! so that the listing and the image on screen follow without F5.
//!
//! `Watcher::new` starts a thread for a folder (with its sub-folders when
//! the listing has them). The thread gathers the changes and hands them
//! over as one `Changes` once the folder has been quiet for `SETTLE` (a
//! file being copied in is written many times over; one change after the
//! last write), or after `LATEST` at most, and asks for a repaint. Dropped,
//! it stops the thread. `App::poll_watch` takes the changes.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
    FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};

/// How long the folder is quiet before its changes are handed over.
pub const SETTLE: Duration = Duration::from_millis(300);
/// How long changes wait at most while the folder keeps changing.
pub const LATEST: Duration = Duration::from_secs(2);
/// The buffer for the changes: 64 KB is the most a network folder allows.
const BUFFER: usize = 64 * 1024;

/// What changed in the folder watched.
#[derive(Default, Debug, PartialEq)]
pub struct Changes {
    /// Added, removed or renamed (the old name and the new one): the
    /// folder is to be listed again.
    pub names: Vec<PathBuf>,
    /// Written: decoded again if shown. A sub-folder is here when
    /// something in it changed.
    pub written: Vec<PathBuf>,
    /// Too many changes to name them, or the watch ended: list again.
    pub all: bool,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.names.is_empty() && self.written.is_empty() && !self.all
    }

    fn add(&mut self, other: Changes) {
        for p in other.names {
            if !self.names.contains(&p) {
                self.names.push(p);
            }
        }
        for p in other.written {
            if !self.written.contains(&p) {
                self.written.push(p);
            }
        }
        self.all |= other.all;
    }
}

/// A folder watched on a thread; see the module.
pub struct Watcher {
    dir: PathBuf,
    subtree: bool,
    rx: mpsc::Receiver<Changes>,
    /// Signalled to stop the thread.
    stop: HANDLE,
    thread: Option<JoinHandle<()>>,
    /// Set by the thread once it is reading the changes (see `armed`).
    #[cfg_attr(not(test), allow(dead_code))]
    armed: Arc<AtomicBool>,
}

// The handle is only signalled and closed from the thread dropping it.
unsafe impl Send for Watcher {}

impl Watcher {
    /// Watch `dir`, and its sub-folders when `subtree`; `ctx` is asked to
    /// repaint when there are changes to take. A folder that cannot be
    /// opened for watching (a drive that refuses it, one gone) gives a
    /// watcher that reports nothing, so that it is not tried every frame.
    pub fn new(dir: &Path, subtree: bool, ctx: egui::Context) -> Watcher {
        let (tx, rx) = mpsc::channel();
        let armed = Arc::new(AtomicBool::new(false));
        let mut watcher =
            Watcher { dir: dir.to_path_buf(), subtree, rx, stop: std::ptr::null_mut(), thread: None, armed: armed.clone() };
        let name = crate::win::wide(dir);
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            log::debug!("cannot watch {}: error {}", dir.display(), unsafe { GetLastError() });
            return watcher;
        }
        let folder = Folder(handle);
        let stop = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if stop.is_null() {
            return watcher;
        }
        let base = dir.to_path_buf();
        let stop_for_thread = Stop(stop);
        let thread = std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || run(folder, &base, subtree, &stop_for_thread, &tx, &ctx, &armed));
        match thread {
            Ok(thread) => {
                log::debug!("watching {}{}", dir.display(), if subtree { " with its sub-folders" } else { "" });
                watcher.stop = stop;
                watcher.thread = Some(thread);
            }
            Err(e) => {
                log::warn!("cannot start a thread to watch {}: {e}", dir.display());
                unsafe { CloseHandle(stop) };
            }
        }
        watcher
    }

    /// Whether this watches `dir` as asked.
    pub fn watches(&self, dir: &Path, subtree: bool) -> bool {
        self.subtree == subtree && crate::folder::same_path(&self.dir, dir)
    }

    /// Whether the thread is reading the changes: what happens in the
    /// folder before that is not seen. Set within a millisecond of `new`.
    #[cfg(test)]
    pub fn armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    /// The changes handed over since the last call, all together.
    pub fn poll(&self) -> Option<Changes> {
        let mut all = Changes::default();
        for changes in self.rx.try_iter() {
            all.add(changes);
        }
        (!all.is_empty()).then_some(all)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if self.stop.is_null() {
            return;
        }
        unsafe { SetEvent(self.stop) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        unsafe { CloseHandle(self.stop) };
    }
}

/// The folder's handle, closed when dropped.
struct Folder(HANDLE);
unsafe impl Send for Folder {}
impl Drop for Folder {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// The stop event's handle, owned by the `Watcher`.
struct Stop(HANDLE);
unsafe impl Send for Stop {}

/// The event's handle, closed when dropped.
struct Event(HANDLE);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;

/// The thread: reads the changes as they come, gathers them, and hands
/// them over once the folder is quiet (see the module).
fn run(
    folder: Folder,
    base: &Path,
    subtree: bool,
    stop: &Stop,
    tx: &mpsc::Sender<Changes>,
    ctx: &egui::Context,
    armed: &AtomicBool,
) {
    let event = Event(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) });
    if event.0.is_null() {
        return;
    }
    // u64 for the alignment FILE_NOTIFY_INFORMATION wants.
    let mut buffer = vec![0u64; BUFFER / 8];
    let mut pending = Changes::default();
    // When the first pending change came and when the last one did.
    let mut since: Option<(Instant, Instant)> = None;
    let hand_over = |pending: &mut Changes, since: &mut Option<(Instant, Instant)>| -> bool {
        *since = None;
        let changes = std::mem::take(pending);
        log::debug!("changes in {}: {changes:?}", base.display());
        let sent = tx.send(changes).is_ok();
        ctx.request_repaint();
        sent
    };
    'reads: loop {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = event.0;
        let ok = unsafe {
            ReadDirectoryChangesW(
                folder.0,
                buffer.as_mut_ptr().cast(),
                BUFFER as u32,
                subtree as i32,
                FILTER,
                std::ptr::null_mut(),
                &mut overlapped,
                None,
            )
        };
        if ok == 0 {
            // The folder is gone, or cannot be watched: whoever lists it
            // again finds out.
            log::debug!("watching {} ended: error {}", base.display(), unsafe { GetLastError() });
            pending.all = true;
            hand_over(&mut pending, &mut since);
            return;
        }
        armed.store(true, Ordering::Release);
        loop {
            let timeout = match since {
                None => INFINITE,
                Some((first, last)) => {
                    let due = (last + SETTLE).min(first + LATEST);
                    due.saturating_duration_since(Instant::now()).as_millis() as u32
                }
            };
            let handles = [stop.0, event.0];
            let waited = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout) };
            let mut n = 0u32;
            if waited == WAIT_OBJECT_0 {
                // Stopped: the read is cancelled and waited for, so that
                // the buffer is no longer written to.
                unsafe {
                    CancelIoEx(folder.0, &overlapped);
                    GetOverlappedResult(folder.0, &overlapped, &mut n, 1);
                }
                return;
            }
            if waited == WAIT_TIMEOUT {
                if !hand_over(&mut pending, &mut since) {
                    unsafe {
                        CancelIoEx(folder.0, &overlapped);
                        GetOverlappedResult(folder.0, &overlapped, &mut n, 1);
                    }
                    return;
                }
                continue;
            }
            if waited != WAIT_OBJECT_0 + 1 {
                return;
            }
            let ok = unsafe { GetOverlappedResult(folder.0, &overlapped, &mut n, 0) };
            let now = Instant::now();
            since = Some(since.map_or((now, now), |(first, _)| (first, now)));
            if ok == 0 {
                log::debug!("watching {} failed: error {}", base.display(), unsafe { GetLastError() });
                pending.all = true;
                hand_over(&mut pending, &mut since);
                return;
            }
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), n as usize) };
            pending.add(parse(bytes, base));
            continue 'reads;
        }
    }
}

/// The changes in `bytes`, the `FILE_NOTIFY_INFORMATION` records
/// `ReadDirectoryChangesW` filled, as paths under `base`. No records
/// means the buffer overflowed: everything may have changed.
fn parse(bytes: &[u8], base: &Path) -> Changes {
    let mut changes = Changes::default();
    if bytes.is_empty() {
        changes.all = true;
        return changes;
    }
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut at = 0;
    while let (Some(next), Some(action), Some(len)) = (u32_at(at), u32_at(at + 4), u32_at(at + 8)) {
        let name = at + 12;
        let Some(name) = bytes.get(name..name + len as usize) else { break };
        let units: Vec<u16> = name.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        let path = base.join(OsString::from_wide(&units));
        let list = match action {
            FILE_ACTION_ADDED | FILE_ACTION_REMOVED | FILE_ACTION_RENAMED_OLD_NAME | FILE_ACTION_RENAMED_NEW_NAME => {
                Some(&mut changes.names)
            }
            FILE_ACTION_MODIFIED => Some(&mut changes.written),
            _ => None,
        };
        if let Some(list) = list
            && !list.contains(&path)
        {
            list.push(path);
        }
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(action: u32, name: &str, last: bool) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let len = units.len() * 2;
        // Records are aligned to 4 bytes.
        let size = (12 + len).div_ceil(4) * 4;
        let mut bytes = Vec::with_capacity(size);
        bytes.extend((if last { 0 } else { size as u32 }).to_le_bytes());
        bytes.extend(action.to_le_bytes());
        bytes.extend((len as u32).to_le_bytes());
        for u in units {
            bytes.extend(u.to_le_bytes());
        }
        bytes.resize(size, 0);
        bytes
    }

    #[test]
    fn parses_records() {
        let base = Path::new(r"C:\pics");
        let mut bytes = record(FILE_ACTION_ADDED, "a.jpg", false);
        bytes.extend(record(FILE_ACTION_MODIFIED, "a.jpg", false));
        bytes.extend(record(FILE_ACTION_MODIFIED, "a.jpg", false));
        bytes.extend(record(FILE_ACTION_RENAMED_OLD_NAME, "b.png", false));
        bytes.extend(record(FILE_ACTION_RENAMED_NEW_NAME, r"sub\c.png", false));
        bytes.extend(record(FILE_ACTION_MODIFIED, "sub", true));
        let changes = parse(&bytes, base);
        assert_eq!(changes.names, vec![base.join("a.jpg"), base.join("b.png"), base.join(r"sub\c.png")]);
        assert_eq!(changes.written, vec![base.join("a.jpg"), base.join("sub")]);
        assert!(!changes.all);
    }

    #[test]
    fn empty_means_everything() {
        let changes = parse(&[], Path::new(r"C:\pics"));
        assert!(changes.all);
        assert!(changes.names.is_empty());
    }

    #[test]
    fn truncated_records_stop() {
        let mut bytes = record(FILE_ACTION_ADDED, "a.jpg", false);
        bytes.extend(&record(FILE_ACTION_ADDED, "b.jpg", true)[..10]);
        let changes = parse(&bytes, Path::new(r"C:\pics"));
        assert_eq!(changes.names, vec![PathBuf::from(r"C:\pics\a.jpg")]);
    }

    /// A folder watched for real: a file written, renamed and deleted.
    #[test]
    fn watches_a_folder() {
        let dir = std::env::temp_dir().join(format!("qview_watch_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = egui::Context::default();
        let watcher = Watcher::new(&dir, false, ctx);
        assert!(watcher.thread.is_some(), "watching the temp folder");
        let started = Instant::now();
        while !watcher.armed() {
            assert!(started.elapsed() < Duration::from_secs(5), "not reading the changes within 5 s");
            std::thread::sleep(Duration::from_millis(1));
        }
        let take = |watcher: &Watcher| {
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(changes) = watcher.poll() {
                    return changes;
                }
                assert!(Instant::now() < until, "no changes within 5 s");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let file = dir.join("a.png");
        std::fs::write(&file, b"x").unwrap();
        let changes = take(&watcher);
        assert!(changes.names.contains(&file), "{changes:?}");
        assert!(changes.written.contains(&file), "{changes:?}");

        let renamed = dir.join("b.png");
        std::fs::rename(&file, &renamed).unwrap();
        let changes = take(&watcher);
        assert!(changes.names.contains(&file) && changes.names.contains(&renamed), "{changes:?}");

        std::fs::remove_file(&renamed).unwrap();
        let changes = take(&watcher);
        assert_eq!(changes.names, vec![renamed], "{changes:?}");

        // Stopped within the time a frame takes, so the folder can go.
        let started = Instant::now();
        drop(watcher);
        assert!(started.elapsed() < Duration::from_secs(1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_folder_watches_nothing() {
        let dir = std::env::temp_dir().join(format!("qview_watch_missing_{}", std::process::id()));
        let watcher = Watcher::new(&dir, false, egui::Context::default());
        assert!(watcher.thread.is_none());
        assert!(watcher.watches(&dir, false));
        assert!(watcher.poll().is_none());
    }
}
