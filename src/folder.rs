//! The images of a folder, in Explorer's name order or by date or size,
//! listed on a thread.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Extensions of the files listed for browsing (lowercase), the formats
/// the `image` crate is built with.
pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "ico", "webp", "tga", "qoi", "pnm",
    "pbm", "pgm", "ppm", "pam",
];

/// Whether `path` has one of [`EXTENSIONS`].
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
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
}

impl SortKey {
    pub const ALL: [SortKey; 3] = [SortKey::Name, SortKey::Modified, SortKey::Size];

    /// Name in the settings.
    pub fn name(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Modified => "modified",
            SortKey::Size => "size",
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
}

impl Entry {
    fn new(path: PathBuf, meta: Option<&std::fs::Metadata>) -> Self {
        use std::os::windows::fs::MetadataExt;
        let name = crate::win::wide(path.file_name().unwrap_or_default());
        Self { name, modified: meta.map_or(0, |m| m.last_write_time()), size: meta.map_or(0, |m| m.len()), path }
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

/// Image files of `dir`, in `order`. `keep` (the file being shown) is
/// listed even when its extension is not one of [`EXTENSIONS`], so that it
/// keeps its place among the others.
pub fn list(dir: &Path, keep: Option<&Path>, order: Order) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        // The type, the size and the date come with the listing on
        // Windows: no extra call.
        let meta = entry.metadata().ok();
        if meta.as_ref().is_some_and(|m| m.is_dir()) {
            continue;
        }
        let path = entry.path();
        if is_image(&path) {
            files.push(Entry::new(path, meta.as_ref()));
        }
    }
    if let Some(keep) = keep
        && !files.iter().any(|e| same_path(&e.path, keep))
        && keep.is_file()
    {
        files.push(Entry::new(keep.to_path_buf(), std::fs::metadata(keep).ok().as_ref()));
    }
    files.sort_by(|a, b| compare(a, b, order));
    Ok(files.into_iter().map(|e| e.path).collect())
}

/// A folder being listed on a thread; see [`scan`].
pub struct Scan {
    rx: mpsc::Receiver<Result<Vec<PathBuf>, String>>,
}

impl Scan {
    /// The listing, once it is ready.
    pub fn poll(&self) -> Option<Result<Vec<PathBuf>, String>> {
        self.rx.try_recv().ok()
    }
}

/// List `dir` in `order` on a thread (a network folder can take a while)
/// and repaint `ctx` when done. Dropping the [`Scan`] discards the result.
pub fn scan(dir: PathBuf, keep: Option<PathBuf>, order: Order, ctx: egui::Context) -> Scan {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("folder".into())
        .spawn(move || {
            let result = list(&dir, keep.as_deref(), order).map_err(|e| e.to_string());
            if tx.send(result).is_ok() {
                ctx.request_repaint();
            }
        })
        .expect("spawn folder thread");
    Scan { rx }
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
