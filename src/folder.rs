//! The images of a folder, in Explorer's name order or by date or size,
//! listed on a thread; with its sub-folders, folder by folder.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};

/// Extensions of the files listed for browsing (lowercase), the formats
/// the `image` crate is built with.
pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "ico", "webp", "tga", "qoi", "pnm",
    "pbm", "pgm", "ppm", "pam",
];

/// Whether `path` has one of [`EXTENSIONS`], or one Windows' codecs take
/// (`wic::extensions`).
pub fn is_image(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(ext))
        || crate::wic::extensions().iter().any(|x| x.eq_ignore_ascii_case(ext))
}

/// Why `name` cannot be a file name, if it cannot: what Explorer refuses.
pub fn check_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err(tr!("Enter a name", "Введите имя"));
    }
    if name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c)) {
        return Err(tr!(
            "A file name cannot contain any of \\ / : * ? \" < > |",
            "Имя файла не может содержать символы \\ / : * ? \" < > |"
        ));
    }
    if name.ends_with(['.', ' ']) {
        return Err(tr!("A file name cannot end with a dot or a space", "Имя файла не может заканчиваться точкой или пробелом"));
    }
    // CON, NUL, COM1 and the like, with any extension.
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if device {
        return Err(tr!("This name is reserved by Windows", "Это имя зарезервировано Windows"));
    }
    Ok(())
}

/// Windows paths ignore case.
pub fn same_path(a: &Path, b: &Path) -> bool {
    a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// Index of `path` in `files`.
pub fn position(files: &[PathBuf], path: &Path) -> Option<usize> {
    files.iter().position(|f| same_path(f, path))
}

/// What the images of a folder are sorted by (View → Sort).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    /// The file name, as Explorer sorts it (see `win::logical_cmp`).
    #[default]
    Name,
    /// The last write time, Explorer's "Date modified".
    Modified,
    Size,
    /// When it was marked as a favourite: its place in the favourites
    /// (only they are sorted so; a folder's files are all equal by it).
    Added,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [SortKey::Name, SortKey::Modified, SortKey::Size, SortKey::Added];

    /// Name in the settings.
    pub fn name(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Modified => "modified",
            SortKey::Size => "size",
            SortKey::Added => "added",
        }
    }

    pub fn from_name(name: &str) -> Option<SortKey> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// The order of a folder's images: by `key`, reversed when `descending`;
/// files equal by it in Explorer's name order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Order {
    pub key: SortKey,
    pub descending: bool,
}

/// A file of a listing with what it is sorted by.
struct Entry {
    path: PathBuf,
    /// The file name, NUL-terminated, for `win::logical_cmp`.
    name: Vec<u16>,
    /// FILETIME, 0 if unknown.
    modified: u64,
    size: u64,
    /// Its place among the favourites, 0 in a folder.
    added: usize,
}

impl Entry {
    fn new(path: PathBuf, meta: Option<&std::fs::Metadata>) -> Self {
        use std::os::windows::fs::MetadataExt;
        let name = crate::win::wide(path.file_name().unwrap_or_default());
        Self { name, modified: meta.map_or(0, |m| m.last_write_time()), size: meta.map_or(0, |m| m.len()), added: 0, path }
    }
}

/// Explorer's name order; names it considers equal by path, so that the
/// order is fixed.
fn by_name(a: &Entry, b: &Entry) -> Ordering {
    crate::win::logical_cmp(&a.name, &b.name).then_with(|| a.path.cmp(&b.path))
}

fn compare(a: &Entry, b: &Entry, order: Order) -> Ordering {
    let key = match order.key {
        SortKey::Name => by_name(a, b),
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Added => a.added.cmp(&b.added),
    };
    let key = if order.descending { key.reverse() } else { key };
    key.then_with(|| by_name(a, b))
}

/// Where `path`, which is not in `files` (listed in `order`), would be:
/// the index of the first file after it, `files.len()` past the last.
/// Known only in name order: the date and size of a file that is not
/// there are not.
pub fn insertion_point(files: &[PathBuf], path: &Path, order: Order) -> Option<usize> {
    if order.key != SortKey::Name {
        return None;
    }
    let missing = Entry::new(path.to_path_buf(), None);
    Some(files.partition_point(|f| compare(&Entry::new(f.clone(), None), &missing, order).is_lt()))
}

/// The image files of `dir`, unsorted, and with `folders` its sub-folders
/// that Explorer shows, in its name order (as in the gallery's tree);
/// symbolic links and junctions are left out, since they can loop.
fn read(dir: &Path, folders: bool) -> std::io::Result<(Vec<Entry>, Vec<PathBuf>)> {
    use std::os::windows::fs::MetadataExt;
    let mut files = Vec::new();
    let mut subfolders = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        // The type, the size and the date come with the listing on
        // Windows: no extra call.
        let meta = entry.metadata().ok();
        if let Some(m) = meta.as_ref().filter(|m| m.is_dir()) {
            if folders
                && crate::win::is_visible_folder(m.file_attributes())
                && !entry.file_type().is_ok_and(|t| t.is_symlink())
            {
                let path = entry.path();
                subfolders.push((crate::win::wide(path.file_name().unwrap_or_default()), path));
            }
            continue;
        }
        let path = entry.path();
        if is_image(&path) {
            files.push(Entry::new(path, meta.as_ref()));
        }
    }
    subfolders.sort_by(|(a, _), (b, _)| crate::win::logical_cmp(a, b));
    Ok((files, subfolders.into_iter().map(|(_, p)| p).collect()))
}

/// Image files of `dir`, in `order`. `keep` (the file being shown) is
/// listed even when its extension is not one of [`EXTENSIONS`], so that it
/// keeps its place among the others.
pub fn list(dir: &Path, keep: Option<&Path>, order: Order) -> std::io::Result<Vec<PathBuf>> {
    let (mut files, _) = read(dir, false)?;
    if let Some(keep) = keep
        && !files.iter().any(|e| same_path(&e.path, keep))
        && keep.is_file()
    {
        files.push(Entry::new(keep.to_path_buf(), std::fs::metadata(keep).ok().as_ref()));
    }
    files.sort_by(|a, b| compare(a, b, order));
    Ok(files.into_iter().map(|e| e.path).collect())
}

/// What a listing holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// The images of the folder.
    Folder,
    /// Those of its sub-folders too, folder by folder (see [`list_deep`]).
    ByFolder,
    /// Those of its sub-folders too, all in one order.
    Flat,
}

/// Image files of `dir` and of all its sub-folders. `by_folder`: folder by
/// folder, a folder's own images in `order`, then those of each of its
/// sub-folders in Explorer's name order, whatever `order` is; otherwise
/// all of them in `order`. A sub-folder that cannot be read is skipped.
/// `found` counts the images found so far; stops when `cancel` is set.
pub fn list_deep(
    dir: &Path,
    order: Order,
    by_folder: bool,
    found: &AtomicUsize,
    cancel: &AtomicBool,
) -> std::io::Result<Vec<PathBuf>> {
    fn walk(
        dir: &Path,
        order: Option<Order>,
        files: &mut Vec<Entry>,
        found: &AtomicUsize,
        cancel: &AtomicBool,
    ) -> std::io::Result<()> {
        if cancel.load(Relaxed) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let (mut own, subfolders) = read(dir, true)?;
        if let Some(order) = order {
            own.sort_by(|a, b| compare(a, b, order));
        }
        files.extend(own);
        found.store(files.len(), Relaxed);
        for sub in subfolders {
            match walk(&sub, order, files, found, cancel) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, by_folder.then_some(order), &mut files, found, cancel)?;
    if !by_folder {
        files.sort_by(|a, b| compare(a, b, order));
    }
    Ok(files.into_iter().map(|e| e.path).collect())
}

/// The files of `paths` that are there (the favourites, in the order they
/// were marked, which [`SortKey::Added`] keeps), in `order`;
/// `by_folder`: folder by folder, the folders in Explorer's name order of
/// their paths, `order` within each. The paths whose folder is there but
/// not the file go to `gone`; those of a folder that cannot be reached (a
/// drive unplugged) to neither. `found` counts the files looked at; stops
/// when `cancel` is set.
pub fn list_files(
    paths: Vec<PathBuf>,
    order: Order,
    by_folder: bool,
    gone: &mut Vec<PathBuf>,
    found: &AtomicUsize,
    cancel: &AtomicBool,
) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::with_capacity(paths.len());
    for (i, path) in paths.into_iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => files.push(Entry { added: i, ..Entry::new(path, Some(&meta)) }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path.parent().is_some_and(Path::is_dir) => {
                gone.push(path)
            }
            _ => {}
        }
        found.store(i + 1, Relaxed);
    }
    if by_folder {
        let folder = |e: &Entry| crate::win::wide(e.path.parent().unwrap_or(&e.path));
        let mut keyed: Vec<(Vec<u16>, Entry)> = files.into_iter().map(|e| (folder(&e), e)).collect();
        keyed.sort_by(|(fa, a), (fb, b)| crate::win::logical_cmp(fa, fb).then_with(|| compare(a, b, order)));
        files = keyed.into_iter().map(|(_, e)| e).collect();
    } else {
        files.sort_by(|a, b| compare(a, b, order));
    }
    Ok(files.into_iter().map(|e| e.path).collect())
}

/// Where the images of each folder start in `files` (of one folder, or
/// listed folder by folder by [`list_deep`] or [`list_files`]).
pub fn starts(files: &[PathBuf]) -> Vec<usize> {
    (0..files.len()).filter(|&i| i == 0 || files[i].parent() != files[i - 1].parent()).collect()
}

/// A folder being listed on a thread; see [`scan`]. Dropping it stops the
/// listing.
pub struct Scan {
    rx: mpsc::Receiver<Result<Vec<PathBuf>, String>>,
    /// Images found so far in a listing with sub-folders.
    found: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    /// Of a listing of files ([`scan_files`]): those found gone, set
    /// before the listing is sent.
    gone: Arc<Mutex<Vec<PathBuf>>>,
}

impl Scan {
    /// The listing, once it is ready.
    pub fn poll(&self) -> Option<Result<Vec<PathBuf>, String>> {
        self.rx.try_recv().ok()
    }

    /// The files of a listing of files that are gone from their folder
    /// (see [`list_files`]), once the listing is ready.
    pub fn take_gone(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.gone.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Images found so far when the sub-folders are listed too.
    pub fn found(&self) -> usize {
        self.found.load(Relaxed)
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Relaxed);
    }
}

/// List `dir` in `order` on a thread (a network folder can take a while),
/// as deep as `depth`, and repaint `ctx` when done. Dropping the [`Scan`]
/// discards the result.
pub fn scan(dir: PathBuf, depth: Depth, keep: Option<PathBuf>, order: Order, ctx: egui::Context) -> Scan {
    spawn(ctx, move |found, cancel| match depth {
        Depth::Folder => list(&dir, keep.as_deref(), order),
        Depth::ByFolder => list_deep(&dir, order, true, found, cancel),
        Depth::Flat => list_deep(&dir, order, false, found, cancel),
    })
}

/// [`list_files`] on a thread, as [`scan`] lists a folder.
pub fn scan_files(paths: Vec<PathBuf>, order: Order, by_folder: bool, ctx: egui::Context) -> Scan {
    let gone = Arc::new(Mutex::new(Vec::new()));
    let out = gone.clone();
    let mut scan = spawn(ctx, move |found, cancel| {
        let mut missing = Vec::new();
        let files = list_files(paths, order, by_folder, &mut missing, found, cancel);
        *out.lock().unwrap_or_else(|e| e.into_inner()) = missing;
        files
    });
    scan.gone = gone;
    scan
}

fn spawn(
    ctx: egui::Context,
    job: impl FnOnce(&AtomicUsize, &AtomicBool) -> std::io::Result<Vec<PathBuf>> + Send + 'static,
) -> Scan {
    let (tx, rx) = mpsc::channel();
    let found = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (count, stop) = (found.clone(), cancel.clone());
    std::thread::Builder::new()
        .name("folder".into())
        .spawn(move || {
            let result = job(&count, &stop).map_err(|e| e.to_string());
            if tx.send(result).is_ok() {
                ctx.request_repaint();
            }
        })
        .expect("spawn folder thread");
    Scan { rx, found, cancel, gone: Arc::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert!(is_image(Path::new(r"C:\a\b.JPG")));
        assert!(is_image(Path::new("x.webp")));
        assert!(!is_image(Path::new("x.txt")));
        assert!(!is_image(Path::new("jpg")));
        // Windows' codecs add some.
        assert!(is_image(Path::new("IMG_0001.HEIC")));
    }

    #[test]
    fn file_names() {
        assert!(check_name("Отпуск 2026.jpg").is_ok());
        assert!(check_name(".hidden").is_ok());
        assert!(check_name("com10.png").is_ok());
        for bad in ["", "  ", "a/b.jpg", "a:b", "what?.png", "a\tb", "name.", "name ", "CON", "nul.jpg", "Com1.png", "lpt9"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn paths_ignore_case() {
        assert!(same_path(Path::new(r"C:\Фото\A.jpg"), Path::new(r"c:\фото\a.JPG")));
        assert!(!same_path(Path::new(r"C:\a.jpg"), Path::new(r"C:\b.jpg")));
    }

    #[test]
    fn lists_images_in_explorer_order() {
        let dir = std::env::temp_dir().join(format!("qview_folder_test_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub.jpg")).unwrap();
        for name in ["10.jpg", "2.png", "1.JPG", "Б.gif", "а.bmp", "notes.txt", "odd.xyz"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let names = |files: Vec<PathBuf>| -> Vec<String> {
            files.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        let by_name = Order::default();
        let files = list(&dir, None, by_name).unwrap();
        assert_eq!(names(files), ["1.JPG", "2.png", "10.jpg", "а.bmp", "Б.gif"]);
        // The file being shown stays listed whatever its extension.
        let files = list(&dir, Some(&dir.join("odd.xyz")), by_name).unwrap();
        assert_eq!(names(files), ["1.JPG", "2.png", "10.jpg", "odd.xyz", "а.bmp", "Б.gif"]);
        // A file that is not there would go before the first one after it.
        let files = list(&dir, None, by_name).unwrap();
        let at = |name: &str, order| insertion_point(&files, &dir.join(name), order);
        assert_eq!(at("0.jpg", by_name), Some(0));
        assert_eq!(at("5.jpg", by_name), Some(2));
        assert_eq!(at("аб.png", by_name), Some(4));
        assert_eq!(at("б.png", by_name), Some(5));
        assert_eq!(at("я.png", by_name), Some(5));
        let names_down = Order { descending: true, ..by_name };
        let files = list(&dir, None, names_down).unwrap();
        assert_eq!(names(files.clone()), ["Б.gif", "а.bmp", "10.jpg", "2.png", "1.JPG"]);
        assert_eq!(insertion_point(&files, &dir.join("5.jpg"), names_down), Some(3));
        assert_eq!(insertion_point(&files, &dir.join("5.jpg"), Order { key: SortKey::Size, ..by_name }), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_sub_folders_folder_by_folder() {
        let dir = std::env::temp_dir().join(format!("qview_folder_deep_{}", std::process::id()));
        for d in [r"b10\x", "b2", "hidden", "empty"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        // The sizes set the order within a folder; the folders keep theirs.
        for (name, size) in [
            ("z.jpg", 1),
            ("a.jpg", 2),
            (r"b2\c.png", 2),
            (r"b2\d.png", 1),
            (r"b2\notes.txt", 1),
            (r"b10\e.gif", 1),
            (r"b10\x\f.bmp", 1),
            (r"hidden\g.jpg", 1),
        ] {
            std::fs::write(dir.join(name), vec![0u8; size]).unwrap();
        }
        let hidden = crate::win::wide(dir.join("hidden"));
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(hidden.as_ptr(), 0x2);
        }
        let relative = |order, by_folder| -> Vec<String> {
            let (found, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
            let files = list_deep(&dir, order, by_folder, &found, &cancel).unwrap();
            assert_eq!(found.load(Relaxed), files.len());
            files.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned()).collect()
        };
        let by_name = Order::default();
        assert_eq!(relative(by_name, true), ["a.jpg", "z.jpg", r"b2\c.png", r"b2\d.png", r"b10\e.gif", r"b10\x\f.bmp"]);
        let by_size = Order { key: SortKey::Size, descending: false };
        assert_eq!(relative(by_size, true), ["z.jpg", "a.jpg", r"b2\d.png", r"b2\c.png", r"b10\e.gif", r"b10\x\f.bmp"]);
        let names_down = Order { descending: true, ..by_name };
        assert_eq!(relative(names_down, true), ["z.jpg", "a.jpg", r"b2\d.png", r"b2\c.png", r"b10\e.gif", r"b10\x\f.bmp"]);
        // Not by folder: one order through all of them.
        assert_eq!(relative(by_name, false), ["a.jpg", r"b2\c.png", r"b2\d.png", r"b10\e.gif", r"b10\x\f.bmp", "z.jpg"]);
        assert_eq!(relative(by_size, false), [r"b2\d.png", r"b10\e.gif", r"b10\x\f.bmp", "z.jpg", "a.jpg", r"b2\c.png"]);
        assert!(list_deep(&dir, by_size, true, &AtomicUsize::new(0), &AtomicBool::new(true)).is_err());
        let files = list_deep(&dir, by_size, true, &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(starts(&files), [0, 2, 4, 5]);
        assert!(starts(&[]).is_empty());
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(hidden.as_ptr(), 0x80);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_files_of_several_folders() {
        let dir = std::env::temp_dir().join(format!("qview_folder_files_{}", std::process::id()));
        for d in ["b10", "b2"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for (name, size) in [(r"b10\a.jpg", 1), (r"b2\c.png", 3), (r"b2\d.png", 2), ("z.jpg", 4)] {
            std::fs::write(dir.join(name), vec![0u8; size]).unwrap();
        }
        let relative = |order, by_folder| -> Vec<String> {
            let paths = [r"b2\c.png", r"b10\a.jpg", "gone.jpg", r"nowhere\x.jpg", "b2", "z.jpg", r"b2\d.png"]
                .map(|p| dir.join(p))
                .to_vec();
            let mut gone = Vec::new();
            let files = list_files(paths, order, by_folder, &mut gone, &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
            // Gone from a folder that is there; not when the folder is not.
            assert_eq!(gone, [dir.join("gone.jpg")]);
            files.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned()).collect()
        };
        let by_size = Order { key: SortKey::Size, descending: false };
        // What is not there, and a folder, are left out; the folders in
        // their order, the files by size within each.
        assert_eq!(relative(by_size, true), ["z.jpg", r"b2\d.png", r"b2\c.png", r"b10\a.jpg"]);
        assert_eq!(relative(by_size, false), [r"b10\a.jpg", r"b2\d.png", r"b2\c.png", "z.jpg"]);
        assert_eq!(relative(Order::default(), false), [r"b10\a.jpg", r"b2\c.png", r"b2\d.png", "z.jpg"]);
        // In the order they were marked, or the other way.
        let added = Order { key: SortKey::Added, descending: false };
        assert_eq!(relative(added, false), [r"b2\c.png", r"b10\a.jpg", "z.jpg", r"b2\d.png"]);
        assert_eq!(relative(Order { descending: true, ..added }, false), [r"b2\d.png", "z.jpg", r"b10\a.jpg", r"b2\c.png"]);
        assert_eq!(relative(added, true), ["z.jpg", r"b2\c.png", r"b2\d.png", r"b10\a.jpg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sorts_by_date_and_size() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("qview_folder_sort_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // Name, size, age in days; b and c are equally large.
        for (name, size, days) in [("a.jpg", 300, 1), ("b.jpg", 100, 3), ("c.jpg", 100, 2), ("d.jpg", 200, 2)] {
            let path = dir.join(name);
            std::fs::write(&path, vec![0u8; size]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(base - Duration::from_secs(days * 86_400)).unwrap();
        }
        let names = |order| -> Vec<String> {
            list(&dir, None, order).unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        let order = |key, descending| Order { key, descending };
        // Oldest first; c and d are as old, so by name.
        assert_eq!(names(order(SortKey::Modified, false)), ["b.jpg", "c.jpg", "d.jpg", "a.jpg"]);
        assert_eq!(names(order(SortKey::Modified, true)), ["a.jpg", "c.jpg", "d.jpg", "b.jpg"]);
        assert_eq!(names(order(SortKey::Size, false)), ["b.jpg", "c.jpg", "d.jpg", "a.jpg"]);
        assert_eq!(names(order(SortKey::Size, true)), ["a.jpg", "d.jpg", "b.jpg", "c.jpg"]);
        for key in SortKey::ALL {
            assert_eq!(SortKey::from_name(key.name()), Some(key));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
