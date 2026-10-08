//! Images inside ZIP and RAR archives (comic books, `.cbz` and `.cbr`),
//! read without unpacking.
//!
//! An image in an archive has a path made of the archive's and its own
//! within it, `D:\Comics\book.cbz\012.jpg` or `...\book.cbz\ch1\001.png`;
//! the archive is listed as a folder of those (`folder::scan_archive`), and
//! the code that reads files reads through [`read`] and [`metadata`]. Such
//! images are read-only: what would change the file (deletion, renaming,
//! saving over, conversion beside it, the favourites) is refused by `App`,
//! and only the `image` crate decodes them (Windows' codecs and libheif
//! read from a file). The archive is opened anew for each read: its central
//! directory is read in about a millisecond for a few hundred pages.
//!
//! Which of the two an archive is comes from its first bytes, not its
//! extension: many `.cbr` files are ZIPs renamed. RAR is read by RARLab's
//! UnRAR (`unrar-ng-sys`), which opens the file by name and goes through
//! its entries in order (see [`rar`]).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

/// Extensions of the archives opened as folders, lowercase.
pub const EXTENSIONS: &[&str] = &["cbz", "zip", "cbr", "rar"];

/// What an archive holds its files in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Zip,
    Rar,
}

/// The format of `archive` from its signature: RAR 1.5-4 and RAR 5 start
/// with `Rar!\x1a\x07`; anything else is left to the ZIP reader.
fn format(archive: &Path) -> std::io::Result<Format> {
    let mut head = [0u8; 6];
    let mut file = File::open(archive)?;
    let mut n = 0;
    while n < head.len() {
        match file.read(&mut head[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(if head[..n] == *b"Rar!\x1a\x07" { Format::Rar } else { Format::Zip })
}

/// Extensions of comic book archives, lowercase: shown in the gallery
/// among the sub-folders (`folder::read`), other ZIP and RAR files are not.
pub const COMICS: &[&str] = &["cbz", "cbr"];

/// Whether `path` has a comic book archive's extension.
pub fn is_comic(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| COMICS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Whether `path` has an archive's extension (it may not exist).
pub fn is_archive(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Whether `path` is an archive file that is there.
pub fn is_archive_file(path: &Path) -> bool {
    is_archive(path) && path.is_file()
}

/// The archive `path` is in and its name within it (`/` between folders),
/// if it is in one. Only paths with a folder named like an archive are
/// looked at on disk, so that this costs nothing for the others.
pub fn split(path: &Path) -> Option<(PathBuf, String)> {
    let archive = path.ancestors().skip(1).find(|a| is_archive(a) && a.is_file())?;
    let inner = path.strip_prefix(archive).ok()?;
    let parts: Vec<String> = inner.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| (archive.to_path_buf(), parts.join("/")))
}

/// Another folder or archive than `dir` is shown: what is kept of a solid
/// RAR archive read before is let go (see `rar::solid`).
pub fn release_unless(dir: &Path) {
    rar::release_unless(dir);
}

/// Whether `path` is an image in an archive.
pub fn inside(path: &Path) -> bool {
    split(path).is_some()
}

/// The folder on disk that holds `path`: its own, or its archive's.
pub fn folder_of(path: &Path) -> Option<PathBuf> {
    match split(path) {
        Some((archive, _)) => archive.parent().map(Path::to_path_buf),
        None if is_archive_file(path) => path.parent().map(Path::to_path_buf),
        None => path.parent().map(Path::to_path_buf),
    }
}

/// The folder of the tree that stands for `dir`: an archive is not in it,
/// its folder is.
pub fn tree_folder(dir: &Path) -> PathBuf {
    match dir.parent() {
        Some(parent) if is_archive_file(dir) => parent.to_path_buf(),
        _ => dir.to_path_buf(),
    }
}

/// Whether `path` is a file to show: one on disk, or an image in an
/// archive that is there.
pub fn is_file(path: &Path) -> bool {
    path.is_file() || inside(path)
}

/// The contents of `path`, a file or an image in an archive.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    let Some((archive, inner)) = split(path) else { return std::fs::read(path) };
    if format(&archive)? == Format::Rar {
        return rar::read(&archive, &inner);
    }
    let mut zip = open(&archive)?;
    let i = find(&zip, &inner).ok_or(std::io::ErrorKind::NotFound)?;
    let mut entry = zip.by_index(i).map_err(std::io::Error::other)?;
    let mut bytes = Vec::with_capacity(entry.size().min(1 << 30) as usize);
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The size and the last write time (FILETIME) of `path`, a file or an
/// image in an archive.
pub fn metadata(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::fs::MetadataExt;
    let Some((archive, inner)) = split(path) else {
        let m = std::fs::metadata(path).ok()?;
        return Some((m.len(), m.last_write_time()));
    };
    if format(&archive).ok()? == Format::Rar {
        return rar::metadata(&archive, &inner);
    }
    let mut zip = open(&archive).ok()?;
    let i = find(&zip, &inner)?;
    let entry = zip.by_index(i).ok()?;
    Some((entry.size(), modified(&entry, &archive)))
}

/// An image of an archive's listing.
pub struct Item {
    pub path: PathBuf,
    pub size: u64,
    /// FILETIME.
    pub modified: u64,
}

/// The images in `archive` that the `image` crate reads, in the order they
/// are stored. Folders, macOS's resource forks (`__MACOSX`) and hidden
/// files are left out, and names that would leave the archive (`..`, a
/// drive) too.
pub fn list(archive: &Path) -> std::io::Result<Vec<Item>> {
    if format(archive)? == Format::Rar {
        return rar::list(archive);
    }
    let mut zip = open(archive)?;
    let mut items = Vec::new();
    for i in 0..zip.len() {
        let Ok(entry) = zip.by_index(i) else { continue };
        if entry.is_dir() {
            continue;
        }
        // With `\` between folders: paths are compared as text.
        let Some(name) = entry.enclosed_name().map(|n| n.components().collect::<PathBuf>()) else { continue };
        if !shown(&name) {
            continue;
        }
        items.push(Item { path: archive.join(name), size: entry.size(), modified: modified(&entry, archive) });
    }
    Ok(items)
}

/// Whether the entry `name` (within the archive, `\` between folders) is
/// listed: an image the `image` crate reads, not hidden and not macOS's
/// leftovers.
fn shown(name: &Path) -> bool {
    let hidden = name.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c.starts_with('.') || c.eq_ignore_ascii_case("__MACOSX")
    });
    !hidden && own_format(name)
}

/// Entries of RAR archives, read through RARLab's UnRAR (its C API, from
/// `unrar-ng-sys`; the `unrar-ng` wrapper brought `regex` with its Unicode
/// tables, ~360 KB). UnRAR goes through the entries in the order stored: an
/// entry is found by reading the headers before it (skipping their data,
/// which costs nothing in an ordinary archive; in a solid one the data
/// before it is unpacked, see `solid`).
mod rar {
    use std::io::{Error, ErrorKind};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf};

    use unrar_ng_sys as ffi;

    /// An archive opened by UnRAR, closed when dropped, with its place
    /// among the entries.
    pub struct Open {
        handle: *const ffi::Handle,
        pub solid: bool,
        /// The header read last (large: kept off the stack).
        header: Box<ffi::HeaderDataEx>,
        /// Where the callback puts the data of the entry being read; its
        /// address is UnRAR's user data, so it stays in its box when the
        /// `Open` moves (into `solid`'s state).
        #[allow(clippy::box_collection)]
        sink: Box<Vec<u8>>,
    }

    // SAFETY: UnRAR's handle is heap data of its own with no tie to the
    // thread that opened it; an `Open` is used by one thread at a time
    // (moved into `solid`'s state, used under its lock).
    unsafe impl Send for Open {}

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: opened by `RAROpenArchiveEx`, closed once.
            unsafe { ffi::RARCloseArchive(self.handle) };
        }
    }

    /// An entry's header.
    pub struct Entry {
        /// Within the archive, as stored (`\` between folders).
        pub name: PathBuf,
        pub size: u64,
        /// FILETIME.
        pub modified: u64,
        flags: u32,
    }

    impl Entry {
        pub fn is_file(&self) -> bool {
            self.flags & ffi::RHDF_DIRECTORY == 0
        }

        /// Encrypted (no password is asked for), or the part of a file
        /// continued from or into another volume: not readable here.
        pub fn unreadable(&self) -> bool {
            self.flags & (ffi::RHDF_ENCRYPTED | ffi::RHDF_SPLITBEFORE | ffi::RHDF_SPLITAFTER) != 0
        }
    }

    /// UnRAR's callback: the data of the entry being read goes to the sink;
    /// passwords and other volumes are refused, as is a dictionary larger
    /// than UnRAR's default limit.
    extern "C" fn callback(msg: ffi::UINT, user: ffi::LPARAM, p1: ffi::LPARAM, p2: ffi::LPARAM) -> std::os::raw::c_int {
        match msg {
            ffi::UCM_PROCESSDATA => {
                // SAFETY: `user` is the `Vec` in `Open::sink`, alive while
                // the archive is open; UnRAR hands `p2` bytes at `p1`.
                let sink = unsafe { &mut *(user as *mut Vec<u8>) };
                if p2 > 0 {
                    sink.extend_from_slice(unsafe { std::slice::from_raw_parts(p1 as *const u8, p2 as usize) });
                }
                1
            }
            ffi::UCM_LARGEDICT => 0,
            _ => -1,
        }
    }

    fn error(code: u32) -> Error {
        let text = match code as i32 {
            ffi::ERAR_NO_MEMORY => "not enough memory",
            ffi::ERAR_BAD_DATA => "the archive is damaged (CRC error)",
            ffi::ERAR_BAD_ARCHIVE => "not a RAR archive",
            ffi::ERAR_UNKNOWN_FORMAT => "unknown archive format",
            ffi::ERAR_EOPEN => "cannot open the archive",
            ffi::ERAR_EREAD => "cannot read the archive",
            ffi::ERAR_MISSING_PASSWORD | ffi::ERAR_BAD_PASSWORD => "the archive is encrypted",
            ffi::ERAR_LARGE_DICT => "the archive's dictionary is too large",
            _ => return Error::other(format!("RAR error {code}")),
        };
        Error::other(text)
    }

    impl Open {
        /// Open `archive` to list its headers (`extract` false) or to read
        /// entries too.
        pub fn new(archive: &Path, extract: bool) -> std::io::Result<Open> {
            let wide: Vec<u16> = archive.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut sink = Box::new(Vec::new());
            // SAFETY: plain C structs, valid all zero.
            let mut data: ffi::OpenArchiveDataEx = unsafe { std::mem::zeroed() };
            data.archive_name_w = wide.as_ptr();
            data.open_mode = if extract { ffi::RAR_OM_EXTRACT } else { ffi::RAR_OM_LIST };
            data.callback = Some(callback);
            data.user_data = &mut *sink as *mut Vec<u8> as ffi::LPARAM;
            // SAFETY: `data` and the name it points to outlive the call.
            // UnRAR writes the result into it: a pointer from `&raw mut`.
            let handle = unsafe { ffi::RAROpenArchiveEx((&raw mut data).cast_const()) };
            let result = data.open_result;
            if handle.is_null() || result != 0 {
                if !handle.is_null() {
                    // SAFETY: just opened.
                    unsafe { ffi::RARCloseArchive(handle) };
                }
                return Err(error(result));
            }
            let flags = data.flags;
            // SAFETY: as above.
            let header = Box::new(unsafe { std::mem::zeroed() });
            Ok(Open { handle, solid: flags & ffi::ROADF_SOLID != 0, header, sink })
        }

        /// The next entry's header, None after the last.
        pub fn next(&mut self) -> std::io::Result<Option<Entry>> {
            // SAFETY: an open handle and a header struct of the right size.
            match unsafe { ffi::RARReadHeaderEx(self.handle, (&raw mut *self.header).cast_const()) } {
                ffi::ERAR_SUCCESS => {}
                ffi::ERAR_END_ARCHIVE => return Ok(None),
                code => return Err(error(code as u32)),
            }
            let h = &*self.header;
            let wide = h.filename_w;
            let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
            let name = PathBuf::from(String::from_utf16_lossy(&wide[..len]));
            let (size_low, size_high, flags) = (h.unp_size, h.unp_size_high, h.flags);
            let (mtime_low, mtime_high, dos) = (h.mtime_low, h.mtime_high, h.file_time);
            let mtime = (mtime_high as u64) << 32 | mtime_low as u64;
            let modified = if mtime != 0 { mtime } else { dos_filetime(dos).unwrap_or(0) };
            Ok(Some(Entry { name, size: (size_high as u64) << 32 | size_low as u64, modified, flags }))
        }

        /// Go past the entry whose header was read last.
        pub fn skip(&mut self) -> std::io::Result<()> {
            self.process(ffi::RAR_SKIP)
        }

        /// The contents of the entry whose header was read last.
        pub fn read(&mut self, size: u64) -> std::io::Result<Vec<u8>> {
            self.sink.clear();
            // Room for the size the header claims, which a damaged header
            // may overstate: when it cannot be had, the sink grows as the
            // data comes.
            let _ = self.sink.try_reserve(size.min(256 << 20) as usize);
            self.process(ffi::RAR_TEST)?;
            Ok(std::mem::take(&mut *self.sink))
        }

        fn process(&mut self, operation: std::os::raw::c_int) -> std::io::Result<()> {
            // SAFETY: an open handle; nothing is written to disk (no path).
            match unsafe { ffi::RARProcessFileW(self.handle, operation, std::ptr::null(), std::ptr::null()) } {
                ffi::ERAR_SUCCESS => Ok(()),
                code => Err(error(code as u32)),
            }
        }
    }

    /// An MS-DOS date and time (local) as a FILETIME, for the archives
    /// that keep no other.
    fn dos_filetime(dos: u32) -> Option<u64> {
        let (date, time) = ((dos >> 16) as u16, dos as u16);
        let (month, day) = (((date >> 5) & 0x0f) as u8, (date & 0x1f) as u8);
        let (hour, minute, second) = ((time >> 11) as u8, ((time >> 5) & 0x3f) as u8, ((time & 0x1f) * 2) as u8);
        crate::win::local_filetime(1980 + (date >> 9), month, day, hour, minute, second)
    }

    /// An entry's name as `split` gives it: `/` between folders,
    /// lowercase, to be compared ignoring case.
    fn key(name: &Path) -> String {
        name.to_string_lossy().replace('\\', "/").to_lowercase()
    }

    /// An entry's name within the archive with `\` between folders, unless
    /// it would leave the archive (`..`, a drive, a root).
    fn enclosed(name: &Path) -> Option<PathBuf> {
        let parts: Vec<Component> = name.components().collect();
        (!parts.is_empty() && parts.iter().all(|c| matches!(c, Component::Normal(_)))).then(|| parts.iter().collect())
    }

    /// An entry's time, or the archive's when it keeps none.
    fn time(entry: &Entry, archive: &Path) -> u64 {
        use std::os::windows::fs::MetadataExt;
        if entry.modified != 0 {
            return entry.modified;
        }
        std::fs::metadata(archive).map_or(0, |m| m.last_write_time())
    }

    /// The contents of the entry `inner` (`/` between folders, any case).
    pub fn read(archive: &Path, inner: &str) -> std::io::Result<Vec<u8>> {
        let wanted = inner.to_lowercase();
        if let Some(bytes) = solid::cached(archive, &wanted) {
            return Ok(bytes);
        }
        let mut open = Open::new(archive, true)?;
        if open.solid {
            return solid::read(archive, &wanted, open);
        }
        while let Some(entry) = open.next()? {
            if entry.is_file() && key(&entry.name) == wanted {
                return open.read(entry.size);
            }
            open.skip()?;
        }
        Err(ErrorKind::NotFound.into())
    }

    /// The size and time of the entry `inner`.
    pub fn metadata(archive: &Path, inner: &str) -> Option<(u64, u64)> {
        let wanted = inner.to_lowercase();
        let mut open = Open::new(archive, false).ok()?;
        while let Some(entry) = open.next().ok()? {
            if entry.is_file() && key(&entry.name) == wanted {
                return Some((entry.size, time(&entry, archive)));
            }
            open.skip().ok()?;
        }
        None
    }

    /// The images of `archive`, as `super::list` gives a ZIP's. Encrypted
    /// entries are left out (no password is asked for), as are the parts
    /// of an entry continued from another volume.
    pub fn list(archive: &Path) -> std::io::Result<Vec<super::Item>> {
        let mut open = Open::new(archive, false)?;
        let mut items = Vec::new();
        while let Some(entry) = open.next()? {
            if entry.is_file()
                && !entry.unreadable()
                && let Some(name) = enclosed(&entry.name)
                && super::shown(&name)
            {
                items.push(super::Item { path: archive.join(name), size: entry.size, modified: time(&entry, archive) });
            }
            open.skip()?;
        }
        Ok(items)
    }

    /// Reading a solid archive, whose entries are unpacked one after
    /// another: an entry costs unpacking all those before it (the last of
    /// 120 pages of 2.9 MB: ~570 ms, the first ~9 ms). So the archive read
    /// last stays open after the entry read last, to go on from there, and
    /// the images passed on the way are kept, up to `BUDGET` bytes, the
    /// ones used longest ago leaving first: browsing on and the gallery's
    /// thumbnails, which ask for the pages nearly in order from several
    /// threads, unpack the archive about once. One thread at a time reads
    /// it; the others wait, then find their pages kept.
    ///
    /// UnRAR holds the file open without sharing deletion, so the archive
    /// stays open only while it is the one shown (`release_unless` names
    /// it): a cover read for the archive's cell in another folder leaves
    /// it closed, and it can be renamed, deleted or replaced.
    mod solid {
        use std::collections::HashMap;
        use std::path::{Path, PathBuf};
        use std::sync::{Arc, Mutex, TryLockError};

        use super::{Open, key};

        /// The bytes of unpacked images kept.
        const BUDGET: usize = 256 << 20;

        struct State {
            archive: PathBuf,
            /// Its size and time, so that an archive changed is read anew.
            stamp: (u64, u64),
            /// Open after the entry read last.
            cursor: Option<Open>,
            /// Images unpacked, by `key`, with when each was last used.
            kept: HashMap<String, (Arc<Vec<u8>>, u64)>,
            bytes: usize,
            clock: u64,
        }

        static STATE: Mutex<Option<State>> = Mutex::new(None);

        /// The folder or archive shown (`release_unless`): only this
        /// archive is kept open after a read. Apart from `STATE`, which a
        /// reading thread holds for a whole pass.
        static SHOWN: Mutex<Option<PathBuf>> = Mutex::new(None);

        fn is_shown(archive: &Path) -> bool {
            SHOWN.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(archive)
        }

        fn stamp(archive: &Path) -> Option<(u64, u64)> {
            use std::os::windows::fs::MetadataExt;
            std::fs::metadata(archive).ok().map(|m| (m.len(), m.last_write_time()))
        }

        impl State {
            fn take(&mut self, wanted: &str) -> Option<Vec<u8>> {
                self.clock += 1;
                let (bytes, used) = self.kept.get_mut(wanted)?;
                *used = self.clock;
                Some(bytes.as_ref().clone())
            }

            fn keep(&mut self, name: String, bytes: Vec<u8>) {
                if bytes.len() > BUDGET / 4 || self.kept.contains_key(&name) {
                    return;
                }
                self.clock += 1;
                self.bytes += bytes.len();
                self.kept.insert(name, (Arc::new(bytes), self.clock));
                while self.bytes > BUDGET {
                    let Some(oldest) = self.kept.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
                    if let Some((b, _)) = self.kept.remove(&oldest) {
                        self.bytes -= b.len();
                    }
                }
            }
        }

        /// The state of `archive` as it is now on disk, made anew for
        /// another archive or one changed.
        fn state<'a>(slot: &'a mut Option<State>, archive: &Path) -> Option<&'a mut State> {
            let stamp = stamp(archive)?;
            if !slot.as_ref().is_some_and(|s| s.archive == archive && s.stamp == stamp) {
                *slot = Some(State { archive: archive.to_path_buf(), stamp, cursor: None, kept: HashMap::new(), bytes: 0, clock: 0 });
            }
            slot.as_mut()
        }

        /// The entry `wanted` (`key`) if it is kept.
        pub fn cached(archive: &Path, wanted: &str) -> Option<Vec<u8>> {
            let mut slot = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if !slot.as_ref().is_some_and(|s| s.archive == archive) {
                return None;
            }
            state(&mut slot, archive)?.take(wanted)
        }

        /// Read the entry `wanted` (`key`) of the solid `archive`, `fresh`
        /// just opened: from the place kept, or from the start. The
        /// archive stays open afterwards only if it is the one shown.
        pub fn read(archive: &Path, wanted: &str, fresh: Open) -> std::io::Result<Vec<u8>> {
            let mut slot = STATE.lock().unwrap_or_else(|e| e.into_inner());
            let result = read_on(&mut slot, archive, wanted, fresh);
            // Another folder is shown (or came to be shown during the
            // read, `release_unless` not waiting): closed, nothing kept.
            if !is_shown(archive) {
                *slot = None;
            }
            result
        }

        fn read_on(slot: &mut Option<State>, archive: &Path, wanted: &str, fresh: Open) -> std::io::Result<Vec<u8>> {
            let state = state(slot, archive).ok_or(std::io::ErrorKind::NotFound)?;
            // Another thread may have passed it meanwhile.
            if let Some(bytes) = state.take(wanted) {
                return Ok(bytes);
            }
            // On from the place kept; from the start if it was not after it.
            let mut fresh = Some(fresh);
            let mut open = match state.cursor.take() {
                Some(open) => open,
                None => fresh.take().ok_or(std::io::ErrorKind::NotFound)?,
            };
            loop {
                let Some(entry) = open.next()? else {
                    match fresh.take() {
                        Some(start) => {
                            open = start;
                            continue;
                        }
                        None => return Err(std::io::ErrorKind::NotFound.into()),
                    }
                };
                let name = key(&entry.name);
                if entry.is_file() && name == wanted {
                    let bytes = open.read(entry.size)?;
                    state.keep(name, bytes.clone());
                    state.cursor = Some(open);
                    return Ok(bytes);
                }
                // Unpacked anyway in a solid archive: kept.
                if entry.is_file() && !entry.unreadable() && super::super::shown(&entry.name) {
                    let bytes = open.read(entry.size)?;
                    state.keep(name, bytes);
                } else {
                    open.skip()?;
                }
            }
        }

        /// `dir` (a folder or an archive) is shown now: let go of the
        /// archive kept unless it is `dir`, its pages, UnRAR's dictionary
        /// and the file. Without waiting (the UI thread calls it): a thread
        /// reading it now lets go when its read is done (`read`).
        pub fn release_unless(dir: &Path) {
            *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.to_path_buf());
            let mut slot = match STATE.try_lock() {
                Ok(slot) => slot,
                Err(TryLockError::Poisoned(e)) => e.into_inner(),
                Err(TryLockError::WouldBlock) => return,
            };
            if slot.as_ref().is_some_and(|s| s.archive != dir) {
                *slot = None;
            }
        }
    }

    pub use solid::release_unless;
}

/// One of the formats the `image` crate reads (`folder::EXTENSIONS`): the
/// others need a file on disk.
fn own_format(name: &Path) -> bool {
    let Some(ext) = name.extension().and_then(|e| e.to_str()) else { return false };
    crate::folder::EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(ext))
}

fn open(archive: &Path) -> std::io::Result<ZipArchive<BufReader<File>>> {
    ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(std::io::Error::other)
}

/// The index of `inner` (`/` between folders) in `zip`: as stored, or
/// written with `\` by a Windows program, ignoring case.
fn find<R: Read + std::io::Seek>(zip: &ZipArchive<R>, inner: &str) -> Option<usize> {
    zip.index_for_name(inner).or_else(|| {
        let wanted = inner.to_lowercase();
        let stored = zip.file_names().find(|n| n.replace('\\', "/").to_lowercase() == wanted)?;
        zip.index_for_name(stored)
    })
}

/// The entry's time (local, as ZIP keeps it) as a FILETIME; the archive's
/// own when it has none.
fn modified<R: Read>(entry: &zip::read::ZipFile<'_, R>, archive: &Path) -> u64 {
    use std::os::windows::fs::MetadataExt;
    entry
        .last_modified()
        .and_then(|t| crate::win::local_filetime(t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second()))
        .or_else(|| std::fs::metadata(archive).ok().map(|m| m.last_write_time()))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A ZIP of a red PNG `a.png`, a folder `ch1` with `b.jpg`, a text file
    /// and macOS's leftovers.
    pub fn sample(path: &Path) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let png = |c: [u8; 3]| {
            let mut out = Vec::new();
            image::RgbImage::from_pixel(4, 3, image::Rgb(c)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        zip.start_file("a.png", options).unwrap();
        zip.write_all(&png([255, 0, 0])).unwrap();
        zip.add_directory("ch1/", options).unwrap();
        zip.start_file("ch1/b.jpg", options).unwrap();
        let mut jpg = Vec::new();
        image::RgbImage::from_pixel(8, 2, image::Rgb([0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
        zip.write_all(&jpg).unwrap();
        zip.start_file("notes.txt", options).unwrap();
        zip.write_all(b"text").unwrap();
        zip.start_file("__MACOSX/._a.png", options).unwrap();
        zip.write_all(b"fork").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn reads_images_inside() {
        let dir = std::env::temp_dir().join(format!("qview_archive_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cbz = dir.join("Book.CBZ");
        sample(&cbz);
        let items = list(&cbz).unwrap();
        let paths: Vec<&Path> = items.iter().map(|i| i.path.as_path()).collect();
        assert_eq!(paths, [cbz.join("a.png"), cbz.join("ch1").join("b.jpg")]);
        assert!(items.iter().all(|i| i.size > 0 && i.modified > 0));

        let page = cbz.join("ch1").join("b.jpg");
        assert_eq!(split(&page), Some((cbz.clone(), "ch1/b.jpg".into())));
        assert!(inside(&page) && is_file(&page));
        assert_eq!(folder_of(&page).as_deref(), Some(dir.as_path()));
        let img = image::load_from_memory(&read(&page).unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (8, 2));
        // Names ignore case, as Windows paths do.
        assert!(read(&cbz.join("A.PNG")).is_ok());
        assert_eq!(metadata(&page).map(|m| m.0), Some(items[1].size));
        assert!(read(&cbz.join("missing.png")).is_err());

        // Paths outside archives are left alone.
        let plain = dir.join("x.png");
        std::fs::write(&plain, b"x").unwrap();
        assert_eq!(split(&plain), None);
        assert_eq!(read(&plain).unwrap(), b"x");
        assert_eq!(split(&cbz), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// RAR archives (made by WinRAR, `testdata`): `a.png` (4x3), `ch1\b.png`
    /// (8x2), the folder `ch1` and `notes.txt`, ordinary and solid; and a
    /// `.cbr` that is a ZIP inside.
    #[test]
    fn reads_rar_archives() {
        let dir = std::env::temp_dir().join(format!("qview_rar_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let samples: [(&str, &[u8]); 2] =
            [("Book.cbr", include_bytes!("../testdata/sample.rar")), ("Solid.CBR", include_bytes!("../testdata/sample_solid.rar"))];
        let size = |bytes: Vec<u8>| {
            let img = image::load_from_memory(&bytes).unwrap();
            (img.width(), img.height())
        };
        for (name, bytes) in samples {
            let cbr = dir.join(name);
            std::fs::write(&cbr, bytes).unwrap();
            assert_eq!(format(&cbr).unwrap(), Format::Rar);
            let items = list(&cbr).unwrap();
            let paths: Vec<&Path> = items.iter().map(|i| i.path.as_path()).collect();
            assert_eq!(paths, [cbr.join("a.png"), cbr.join("ch1").join("b.png")], "{name}");
            assert!(items.iter().all(|i| i.size > 0 && i.modified > 0));
            // Shown, as when opened: a solid archive stays open after the
            // page read last. The last first, then the first: it goes on
            // from its place, then starts over (or finds it kept).
            release_unless(&cbr);
            assert_eq!(size(read(&items[1].path).unwrap()), (8, 2));
            assert_eq!(size(read(&cbr.join("A.PNG")).unwrap()), (4, 3));
            assert_eq!(size(read(&items[1].path).unwrap()), (8, 2));
            assert_eq!(metadata(&items[1].path).map(|m| m.0), Some(items[1].size));
            assert!(read(&cbr.join("missing.png")).is_err());
            assert!(read(&cbr.join("notes.txt")).is_ok());
            assert_eq!(split(&items[0].path), Some((cbr.clone(), "a.png".into())));
            // Its folder shown instead: closed, so it can be renamed (the
            // solid one was held open by UnRAR, which shares no deletion).
            release_unless(&dir);
            let moved = dir.join(format!("moved_{name}"));
            std::fs::rename(&cbr, &moved).unwrap();
            // A cover read for its cell in that folder leaves it closed.
            assert_eq!(size(read(&moved.join("a.png")).unwrap()), (4, 3));
            std::fs::remove_file(&moved).unwrap();
        }
        // Many .cbr files are ZIPs: told by their contents.
        let zip = dir.join("zip.cbr");
        sample(&zip);
        assert_eq!(format(&zip).unwrap(), Format::Zip);
        assert_eq!(list(&zip).unwrap().len(), 2);
        release_unless(Path::new(""));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Times listing an archive and reading its first, middle and last
    /// images: `$env:QVIEW_ARCHIVE_FILE="<cbr or cbz>"; cargo test --release
    /// archive_timings -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn archive_timings() {
        let Some(file) = std::env::var_os("QVIEW_ARCHIVE_FILE").map(PathBuf::from) else { return };
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
        // Shown, as when opened: a solid archive keeps its cursor.
        release_unless(&file);
        let t = std::time::Instant::now();
        let format = format(&file).unwrap();
        println!("{format:?}: signature read in {:.2} ms", ms(t));
        let t = std::time::Instant::now();
        let items = list(&file).unwrap();
        println!("{} images listed in {:.1} ms", items.len(), ms(t));
        let n = items.len();
        for k in [0, n / 2, n.saturating_sub(1)] {
            let t = std::time::Instant::now();
            let size = metadata(&items[k].path).unwrap().0;
            let meta = ms(t);
            let t = std::time::Instant::now();
            let bytes = read(&items[k].path).unwrap();
            assert_eq!(bytes.len() as u64, size);
            println!("image {} of {n}: metadata {meta:.1} ms, read {:.1} ms ({:.1} MB)", k + 1, ms(t), bytes.len() as f64 / 1e6);
        }
        // All of them in order, as the gallery's thumbnails ask, then the
        // first again.
        let t = std::time::Instant::now();
        for item in &items {
            read(&item.path).unwrap();
        }
        println!("all {n} in order: {:.0} ms", ms(t));
        let t = std::time::Instant::now();
        read(&items[0].path).unwrap();
        println!("image 1 again: {:.1} ms", ms(t));
        // From several threads at once, after letting go of what is kept
        // (another folder shown, then the archive again).
        release_unless(Path::new(""));
        release_unless(&file);
        let t = std::time::Instant::now();
        std::thread::scope(|s| {
            for w in 0..3 {
                let items = &items;
                s.spawn(move || {
                    for item in items.iter().skip(w).step_by(3) {
                        read(&item.path).unwrap();
                    }
                });
            }
        });
        println!("all {n} on 3 threads: {:.0} ms", ms(t));
    }
}
