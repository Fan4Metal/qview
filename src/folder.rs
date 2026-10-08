//! The images of a folder, in Explorer's name order or by date or size,
//! listed on a thread; with its sub-folders, folder by folder.

use std::cmp::Ordering;
use std::collections::HashMap;
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
    /// When the photo was taken, from its EXIF (`info::taken`); the date
    /// modified for an image that does not say.
    Taken,
    Size,
    /// When it was marked as a favourite: its place in the favourites
    /// (only they are sorted so; a folder's files are all equal by it).
    Added,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [SortKey::Name, SortKey::Modified, SortKey::Taken, SortKey::Size, SortKey::Added];

    /// Name in the settings.
    pub fn name(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Modified => "modified",
            SortKey::Taken => "taken",
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
    /// FILETIME of the date taken, read only to sort by it (`read_taken`);
    /// the date modified until then.
    taken: u64,
    /// `taken` is the date the file says it was taken (`read_taken`).
    dated: bool,
    size: u64,
    /// Its place among the favourites, 0 in a folder.
    added: usize,
}

impl Entry {
    fn new(path: PathBuf, meta: Option<&std::fs::Metadata>) -> Self {
        use std::os::windows::fs::MetadataExt;
        let name = crate::win::wide(path.file_name().unwrap_or_default());
        let modified = meta.map_or(0, |m| m.last_write_time());
        Self { name, modified, taken: modified, dated: false, size: meta.map_or(0, |m| m.len()), added: 0, path }
    }
}

/// The dates taken read so far, by path (lowercase), with the file's
/// date modified and size they were read for: a folder listed again (a
/// save, a rename, F5) does not read every file again. None: the file
/// does not say.
static TAKEN: Mutex<Option<HashMap<String, Taken>>> = Mutex::new(None);
/// A cached date taken: the file's date modified and size it was read
/// for, and the date (None: the file has none).
type Taken = (u64, u64, Option<u64>);
/// Entries kept in it at most; it starts over past that.
const TAKEN_CACHE: usize = 200_000;

/// The dates `files` were taken (`info::taken`), read when they are sorted
/// by them, on several threads (a file's header each); those that do not
/// say keep their date modified. Stops when `cancel` is set.
fn read_taken(files: &mut [Entry], order: Order, cancel: &AtomicBool) {
    use std::sync::atomic::AtomicU64;
    if order.key != SortKey::Taken {
        return;
    }
    let key = |e: &Entry| e.path.to_string_lossy().to_lowercase();
    let mut cache = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() > TAKEN_CACHE {
        cache.clear();
    }
    let known: Vec<Option<Option<u64>>> = files
        .iter()
        .map(|e| cache.get(&key(e)).filter(|(m, s, _)| *m == e.modified && *s == e.size).map(|(_, _, t)| *t))
        .collect();
    let next = AtomicUsize::new(0);
    let taken: Vec<AtomicU64> = files.iter().map(|e| AtomicU64::new(e.taken)).collect();
    // Read: the file's date, or 0 when it has none.
    let read: Vec<AtomicU64> = files.iter().map(|_| AtomicU64::new(u64::MAX)).collect();
    let unknown: Vec<usize> = (0..files.len()).filter(|&i| known[i].is_none()).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8)).min(unknown.len());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                // libheif and the `image` crate read the headers; Windows'
                // codecs are not asked.
                loop {
                    let k = next.fetch_add(1, Relaxed);
                    if k >= unknown.len() || cancel.load(Relaxed) {
                        break;
                    }
                    let i = unknown[k];
                    let t = crate::info::taken(&files[i].path);
                    read[i].store(t.unwrap_or(0), Relaxed);
                    if let Some(t) = t {
                        taken[i].store(t, Relaxed);
                    }
                }
            });
        }
    });
    for (i, e) in files.iter_mut().enumerate() {
        match (known[i], read[i].load(Relaxed)) {
            (Some(Some(t)), _) => {
                e.taken = t;
                e.dated = true;
            }
            (Some(None), _) => {}
            // Not read (cancelled): not remembered.
            (None, u64::MAX) => {}
            (None, t) => {
                e.taken = taken[i].load(Relaxed);
                e.dated = t != 0;
                cache.insert(key(e), (e.modified, e.size, (t != 0).then_some(t)));
            }
        }
    }
}

/// What the information panel says of a folder's images, or of several
/// images chosen in the gallery.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub images: usize,
    pub bytes: u64,
    /// The images' dates modified, the oldest and the newest (FILETIMEs).
    pub modified: Option<(u64, u64)>,
    /// The dates taken of those whose metadata says (FILETIMEs), and how
    /// many say.
    pub taken: Option<(u64, u64)>,
    pub dated: usize,
    /// The folders the images are in.
    pub places: usize,
    /// Of a folder: its visible sub-folders, and its own dates modified
    /// and created (FILETIMEs).
    pub folders: Option<usize>,
    pub dates: Option<(u64, u64)>,
}

/// The images directly in `dir` and its sub-folders, counted: their size,
/// dates modified and dates taken (`read_taken`, its cache shared with the
/// order by date taken). Stops early, with what it has, when `cancel` is
/// set.
pub fn folder_stats(dir: &Path, cancel: &AtomicBool) -> std::io::Result<Stats> {
    use std::os::windows::fs::MetadataExt;
    let own = std::fs::metadata(dir)?;
    let (mut files, folders, _) = read(dir, true)?;
    let mut stats = stats_of(&mut files, true, cancel);
    stats.folders = Some(folders.len());
    stats.dates = Some((own.last_write_time(), own.creation_time()));
    Ok(stats)
}

/// The images of the comic book archive `archive` (a cell among the
/// sub-folders) counted as `folder_stats` counts a folder's: its pages,
/// their size and dates, and the archive's own dates.
pub fn archive_stats(archive: &Path) -> std::io::Result<Stats> {
    use std::os::windows::fs::MetadataExt;
    let own = std::fs::metadata(archive)?;
    let mut files: Vec<Entry> = crate::archive::list(archive)?
        .into_iter()
        .map(|i| {
            let mut e = Entry::new(i.path, None);
            (e.size, e.modified, e.taken) = (i.size, i.modified, i.modified);
            e
        })
        .collect();
    let mut stats = stats_of(&mut files, false, &AtomicBool::new(false));
    stats.dates = Some((own.last_write_time(), own.creation_time()));
    Ok(stats)
}

/// `paths` (files, or images in an archive) counted as `folder_stats`
/// counts a folder's; the dates taken are not read in an archive.
pub fn files_stats(paths: &[PathBuf], cancel: &AtomicBool) -> Stats {
    let mut files = Vec::with_capacity(paths.len());
    let mut archived = false;
    for path in paths {
        if cancel.load(Relaxed) {
            break;
        }
        if crate::archive::inside(path) {
            archived = true;
            let mut e = Entry::new(path.clone(), None);
            if let Some((size, modified)) = crate::archive::metadata(path) {
                (e.size, e.modified, e.taken) = (size, modified, modified);
            }
            files.push(e);
        } else {
            files.push(Entry::new(path.clone(), std::fs::metadata(path).ok().as_ref()));
        }
    }
    stats_of(&mut files, !archived, cancel)
}

fn stats_of(files: &mut [Entry], taken: bool, cancel: &AtomicBool) -> Stats {
    if taken {
        read_taken(files, Order { key: SortKey::Taken, descending: false }, cancel);
    }
    let range = |dates: &mut dyn Iterator<Item = u64>| {
        dates.filter(|&d| d != 0).fold(None, |r: Option<(u64, u64)>, d| Some(r.map_or((d, d), |(a, b)| (a.min(d), b.max(d)))))
    };
    let mut places: Vec<&Path> = files.iter().filter_map(|e| e.path.parent()).collect();
    places.sort();
    places.dedup();
    Stats {
        images: files.len(),
        bytes: files.iter().map(|e| e.size).sum(),
        modified: range(&mut files.iter().map(|e| e.modified)),
        taken: range(&mut files.iter().filter(|e| e.dated).map(|e| e.taken)),
        dated: files.iter().filter(|e| e.dated).count(),
        places: places.len(),
        folders: None,
        dates: None,
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
        SortKey::Taken => a.taken.cmp(&b.taken),
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
fn read(dir: &Path, folders: bool) -> std::io::Result<(Vec<Entry>, Vec<PathBuf>, Vec<PathBuf>)> {
    use std::os::windows::fs::MetadataExt;
    let mut files = Vec::new();
    let mut subfolders = Vec::new();
    let mut comics = Vec::new();
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
        } else if folders
            && crate::archive::is_comic(&path)
            // Not hidden, as the sub-folders shown.
            && meta.as_ref().is_some_and(|m| m.file_attributes() & 0x2 == 0)
        {
            comics.push((crate::win::wide(path.file_name().unwrap_or_default()), path));
        }
    }
    subfolders.sort_by(|(a, _), (b, _)| crate::win::logical_cmp(a, b));
    comics.sort_by(|(a, _), (b, _)| crate::win::logical_cmp(a, b));
    let names = |list: Vec<(Vec<u16>, PathBuf)>| list.into_iter().map(|(_, p)| p).collect();
    Ok((files, names(subfolders), names(comics)))
}

/// The visible sub-folders of `dir`, in Explorer's name order (none when
/// it cannot be read).
pub fn subfolders(dir: &Path) -> Vec<PathBuf> {
    read(dir, true).map(|(_, folders, _)| folders).unwrap_or_default()
}

/// Image files of `dir`, in `order`. `keep` (the file being shown) is
/// listed even when its extension is not one of [`EXTENSIONS`], so that it
/// keeps its place among the others.
#[cfg(test)]
pub fn list(dir: &Path, keep: Option<&Path>, order: Order) -> std::io::Result<Vec<PathBuf>> {
    list_with_folders(dir, keep, order, false).map(|(files, _, _)| files)
}

/// [`list`], and with `folders` the sub-folders of `dir` that Explorer
/// shows, in its name order (the gallery's folder cells).
pub fn list_with_folders(dir: &Path, keep: Option<&Path>, order: Order, folders: bool) -> std::io::Result<Listed> {
    list_with_folders_until(dir, keep, order, folders, &AtomicBool::new(false))
}

/// [`list_with_folders`], stopping (with `Interrupted`) when `cancel` is
/// set while the dates taken are read.
fn list_with_folders_until(
    dir: &Path,
    keep: Option<&Path>,
    order: Order,
    folders: bool,
    cancel: &AtomicBool,
) -> std::io::Result<Listed> {
    let (mut files, subfolders, comics) = read(dir, folders)?;
    if let Some(keep) = keep
        && !files.iter().any(|e| same_path(&e.path, keep))
        && keep.is_file()
    {
        files.push(Entry::new(keep.to_path_buf(), std::fs::metadata(keep).ok().as_ref()));
    }
    read_taken(&mut files, order, cancel);
    if cancel.load(Relaxed) {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    files.sort_by(|a, b| compare(a, b, order));
    Ok((files.into_iter().map(|e| e.path).collect(), subfolders, comics))
}

/// A folder's images in order, its visible sub-folders and its comic book
/// archives (`archive::COMICS`), both in Explorer's name order.
pub type Listed = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>);

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
        let (mut own, subfolders, _) = read(dir, true)?;
        if let Some(order) = order {
            read_taken(&mut own, order, cancel);
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
        read_taken(&mut files, order, cancel);
        if cancel.load(Relaxed) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
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
            Err(e) if is_gone(&path, &e) => gone.push(path),
            _ => {}
        }
        found.store(i + 1, Relaxed);
    }
    read_taken(&mut files, order, cancel);
    Ok(sorted(files, order, by_folder))
}

/// `files` in `order`; `by_folder`: folder by folder, the folders in
/// Explorer's name order of their paths, `order` within each.
fn sorted(mut files: Vec<Entry>, order: Order, by_folder: bool) -> Vec<PathBuf> {
    if by_folder {
        let folder = |e: &Entry| crate::win::wide(e.path.parent().unwrap_or(&e.path));
        let mut keyed: Vec<(Vec<u16>, Entry)> = files.into_iter().map(|e| (folder(&e), e)).collect();
        keyed.sort_by(|(fa, a), (fb, b)| crate::win::logical_cmp(fa, fb).then_with(|| compare(a, b, order)));
        files = keyed.into_iter().map(|(_, e)| e).collect();
    } else {
        files.sort_by(|a, b| compare(a, b, order));
    }
    files.into_iter().map(|e| e.path).collect()
}

/// The images of `archive` (see `archive::list`) in `order`; `by_folder`:
/// its folders one after another, as [`list_files`] does. Named by their
/// paths within it, so that a comic's chapters (`ch1\001.jpg`,
/// `ch2\001.jpg`) stay apart in name order.
pub fn list_archive(archive: &Path, order: Order, by_folder: bool) -> std::io::Result<Vec<PathBuf>> {
    let files = crate::archive::list(archive)?
        .into_iter()
        .map(|i| Entry {
            name: crate::win::wide(i.path.strip_prefix(archive).unwrap_or(&i.path)),
            modified: i.modified,
            taken: i.modified,
            dated: false,
            size: i.size,
            added: 0,
            path: i.path,
        })
        .collect();
    Ok(sorted(files, order, by_folder))
}

/// [`list_archive`] on a thread, as [`scan`] lists a folder.
pub fn scan_archive(archive: PathBuf, order: Order, by_folder: bool, ctx: egui::Context) -> Scan {
    let up = Arc::new(Mutex::new(None));
    let above = up.clone();
    let mut scan = spawn(ctx, move |_, _| {
        let result = list_archive(&archive, order, by_folder);
        if result.is_err() && !archive.is_file() {
            *above.lock().unwrap_or_else(|e| e.into_inner()) = nearest_folder(&archive);
        }
        result
    });
    scan.up = up;
    scan
}

/// Whether `path`, which `metadata` failed on with `e`, is gone from its
/// folder: the folder is there without it. A folder that cannot be reached
/// (a drive unplugged) says nothing.
fn is_gone(path: &Path, e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound && path.parent().is_some_and(Path::is_dir)
}

/// The paths of `paths` gone from their folder (see [`list_files`]),
/// checked on a thread; `ctx` is repainted when they are known.
pub fn find_gone(paths: Vec<PathBuf>, ctx: egui::Context) -> mpsc::Receiver<Vec<PathBuf>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("gone".into())
        .spawn(move || {
            let gone = paths.into_iter().filter(|p| std::fs::metadata(p).is_err_and(|e| is_gone(p, &e))).collect();
            if tx.send(gone).is_ok() {
                ctx.request_repaint();
            }
        })
        .expect("spawn gone thread");
    rx
}

/// Whether an image shown as `shown` (its name, or its path from the
/// folder listed with the sub-folders, or its whole path among the
/// favourites) passes the gallery's `filter`: every word of it, ignoring
/// case, is in `shown`, or, with `*` or `?`, matches the file's name as a
/// whole (`*.png`, `IMG_20??*`).
pub fn matches(shown: &str, filter: &str) -> bool {
    let shown = shown.to_lowercase();
    let name = shown.rsplit(['\\', '/']).next().unwrap_or(&shown);
    filter.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        if word.contains(['*', '?']) {
            let (pattern, name): (Vec<char>, Vec<char>) = (word.chars().collect(), name.chars().collect());
            wildcard(&pattern, &name)
        } else {
            shown.contains(&word)
        }
    })
}

/// `name` matches `pattern`: `*` any run of characters, `?` one.
fn wildcard(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    // The last `*` seen, and where in `name` it was tried up to.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                // The `*` takes one more character.
                Some((sp, sn)) => {
                    star = Some((sp, sn + 1));
                    p = sp + 1;
                    n = sn + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
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
    /// Of a folder listed alone: its sub-folders and its comic book
    /// archives, set before the listing is sent.
    folders: Arc<Mutex<Vec<PathBuf>>>,
    comics: Arc<Mutex<Vec<PathBuf>>>,
    /// The folder (or archive) listed is gone: the nearest folder above it
    /// that is there, set before the error is sent.
    up: Arc<Mutex<Option<PathBuf>>>,
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

    /// The sub-folders of a folder listed alone, once the listing is
    /// ready.
    pub fn take_folders(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.folders.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The comic book archives of a folder listed alone, once the listing
    /// is ready.
    pub fn take_comics(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.comics.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The nearest folder above the one listed that is there, when that
    /// one is gone (deleted or renamed), once the listing has failed.
    pub fn take_up(&self) -> Option<PathBuf> {
        self.up.lock().unwrap_or_else(|e| e.into_inner()).take()
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
    let folders = Arc::new(Mutex::new(Vec::new()));
    let out = folders.clone();
    let comics = Arc::new(Mutex::new(Vec::new()));
    let comics_out = comics.clone();
    let up = Arc::new(Mutex::new(None));
    let above = up.clone();
    let mut scan = spawn(ctx, move |found, cancel| {
        let result = match depth {
            Depth::Folder => list_with_folders_until(&dir, keep.as_deref(), order, true, cancel).map(|(files, subfolders, archives)| {
                *out.lock().unwrap_or_else(|e| e.into_inner()) = subfolders;
                *comics_out.lock().unwrap_or_else(|e| e.into_inner()) = archives;
                files
            }),
            Depth::ByFolder => list_deep(&dir, order, true, found, cancel),
            Depth::Flat => list_deep(&dir, order, false, found, cancel),
        };
        if result.is_err() && !dir.is_dir() {
            *above.lock().unwrap_or_else(|e| e.into_inner()) = nearest_folder(&dir);
        }
        result
    });
    scan.folders = folders;
    scan.comics = comics;
    scan.up = up;
    scan
}

/// `path` once the folder `old` is called `new`: `new` for `old` itself,
/// the same place under `new` for what is under it, None for any other
/// path. Case is ignored, as Windows does.
pub fn rebase(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let lower = |c: std::path::Component| c.as_os_str().to_string_lossy().to_lowercase();
    let mut rest = path.components();
    for c in old.components() {
        if rest.next().map(lower) != Some(lower(c)) {
            return None;
        }
    }
    let mut out = new.to_path_buf();
    out.extend(rest);
    Some(out)
}

/// Whether `path` is the folder `dir` or under it.
pub fn is_within(path: &Path, dir: &Path) -> bool {
    rebase(path, dir, dir).is_some()
}

/// The nearest folder above `path` that is there; None when even its
/// drive cannot be reached (unplugged), when there is nowhere to go.
pub fn nearest_folder(path: &Path) -> Option<PathBuf> {
    path.ancestors().skip(1).find(|a| !a.as_os_str().is_empty() && a.is_dir()).map(Path::to_path_buf)
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
    Scan { rx, found, cancel, gone: Arc::default(), folders: Arc::default(), comics: Arc::default(), up: Arc::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_folder_and_files() {
        let dir = std::env::temp_dir().join(format!("qview_stats_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.jpg"), [0u8; 100]).unwrap();
        std::fs::write(dir.join("b.png"), [0u8; 50]).unwrap();
        std::fs::write(dir.join("notes.txt"), [0u8; 999]).unwrap();
        std::fs::write(dir.join("sub").join("c.gif"), [0u8; 7]).unwrap();
        let stop = AtomicBool::new(false);
        let st = folder_stats(&dir, &stop).unwrap();
        // The folder's own images only; nothing in them says when taken.
        assert_eq!((st.images, st.bytes, st.folders, st.dated, st.taken, st.places), (2, 150, Some(1), 0, None, 1));
        let (oldest, newest) = st.modified.unwrap();
        assert!(oldest > 0 && oldest <= newest);
        assert!(st.dates.is_some_and(|(m, c)| m > 0 && c > 0));
        let st = files_stats(&[dir.join("a.jpg"), dir.join("sub").join("c.gif")], &stop);
        assert_eq!((st.images, st.bytes, st.places, st.folders), (2, 107, 2, None));
        // Cancelled before it starts: nothing counted.
        assert_eq!(files_stats(&[dir.join("a.jpg")], &AtomicBool::new(true)).images, 0);
        assert!(folder_stats(&dir.join("missing"), &stop).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `$env:QVIEW_THUMB_DIR="<folder>"; cargo test --release stats_timings -- --ignored --nocapture`:
    /// what the information panel counts of a folder, the dates taken read
    /// first (if not cached in this run) and then cached.
    #[test]
    #[ignore]
    fn stats_timings() {
        let dir = PathBuf::from(std::env::var("QVIEW_THUMB_DIR").expect("QVIEW_THUMB_DIR"));
        let _com = crate::win::com_init();
        for round in 0..2 {
            let t = std::time::Instant::now();
            let st = folder_stats(&dir, &AtomicBool::new(false)).unwrap();
            println!("round {round}: {:.1} ms, {st:?}", t.elapsed().as_secs_f64() * 1e3);
        }
    }

    #[test]
    fn lists_comic_books_apart() {
        let dir = std::env::temp_dir().join(format!("qview_comics_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Sub")).unwrap();
        for name in ["p.png", "A10.cbz", "A9.cbz", "b.CBR", "backup.zip", "other.rar"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let (files, folders, comics) = list_with_folders(&dir, None, Order::default(), true).unwrap();
        assert_eq!(files, [dir.join("p.png")]);
        assert_eq!(folders, [dir.join("Sub")]);
        // Comic books only, in Explorer's order; other archives are not shown.
        assert_eq!(comics, [dir.join("A9.cbz"), dir.join("A10.cbz"), dir.join("b.CBR")]);
        assert!(list_with_folders(&dir, None, Order::default(), false).unwrap().2.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filters_by_words_and_wildcards() {
        assert!(matches("IMG_2024_Beach.jpg", ""));
        assert!(matches("IMG_2024_Beach.jpg", "beach"));
        assert!(matches("IMG_2024_Beach.jpg", "2024  BEACH"));
        assert!(!matches("IMG_2024_Beach.jpg", "2024 mountains"));
        assert!(matches("IMG_2024_Beach.jpg", "*.jpg"));
        assert!(!matches("IMG_2024_Beach.jpg", "*.png"));
        assert!(matches("IMG_2024_Beach.jpg", "img_20??_*"));
        assert!(!matches("IMG_2024_Beach.jpg", "img_20?_*"));
        assert!(matches("a.b.c", "*.*.c"));
        assert!(matches("x", "*"));
        // A word matches the path from the folder; a wildcard, the name.
        assert!(matches(r"Trip\IMG_1.jpg", "trip"));
        assert!(!matches(r"Trip\IMG_1.jpg", "trip*"));
        assert!(matches(r"Trip\IMG_1.jpg", "img*"));
        assert!(matches("Отпуск.jpg", "ОТПУСК"));
    }

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
    fn paths_follow_a_folder_renamed() {
        let (old, new) = (Path::new(r"D:\Фото\Лето"), Path::new(r"D:\Фото\Лето 2025"));
        assert_eq!(rebase(Path::new(r"d:\фото\лето\a\1.jpg"), old, new), Some(PathBuf::from(r"D:\Фото\Лето 2025\a\1.jpg")));
        assert_eq!(rebase(old, old, new), Some(new.to_path_buf()));
        // A name that only starts the same is another folder.
        assert_eq!(rebase(Path::new(r"D:\Фото\Летом\1.jpg"), old, new), None);
        assert_eq!(rebase(Path::new(r"D:\Фото"), old, new), None);
        assert!(is_within(Path::new(r"D:\Фото\Лето\x"), old) && !is_within(Path::new(r"D:\Фото\x"), old));
    }

    #[test]
    fn a_folder_gone_leads_up() {
        let dir = std::env::temp_dir().join(format!("qview_folder_gone_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a")).unwrap();
        // Two levels gone: the nearest that is there.
        assert_eq!(nearest_folder(&dir.join(r"a\b\c")), Some(dir.join("a")));
        // A drive that is not there: nowhere.
        assert_eq!(nearest_folder(Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x")), None);
        // A listing of a folder gone says where to go; one that is there
        // does not.
        let ctx = egui::Context::default();
        let wait = |scan: &Scan| loop {
            if let Some(result) = scan.poll() {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let scan = super::scan(dir.join(r"a\gone"), Depth::Folder, None, Order::default(), ctx.clone());
        assert!(wait(&scan).is_err());
        assert_eq!(scan.take_up(), Some(dir.join("a")));
        let scan = super::scan(dir.join("a"), Depth::Flat, None, Order::default(), ctx);
        assert!(wait(&scan).is_ok());
        assert_eq!(scan.take_up(), None);
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

    #[test]
    fn sorts_by_date_taken() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("qview_folder_taken_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A JPEG's start with an EXIF whose first IFD holds DateTime.
        let jpeg = |date: &str| {
            let mut tiff = b"II*\0\x08\0\0\0\x01\0\x32\x01\x02\0\x14\0\0\0\x1a\0\0\0\0\0\0\0".to_vec();
            tiff.extend(date.as_bytes());
            tiff.push(0);
            let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
            out.extend(((tiff.len() + 8) as u16).to_be_bytes());
            out.extend(b"Exif\0\0");
            out.extend(tiff);
            out.extend([0xff, 0xd9]);
            out
        };
        // b has no date taken: its date modified, 2023, counts.
        let files = [("a.jpg", jpeg("2020:06:01 12:00:00")), ("b.jpg", vec![0; 10]), ("c.jpg", jpeg("2010:01:01 08:00:00"))];
        for (name, bytes) in files {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)).unwrap();
        }
        let names = |descending| -> Vec<String> {
            let order = Order { key: SortKey::Taken, descending };
            list(&dir, None, order).unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        assert_eq!(names(false), ["c.jpg", "a.jpg", "b.jpg"]);
        assert_eq!(names(true), ["b.jpg", "a.jpg", "c.jpg"]);
        // The dates are cached by the file's date and size: a file written
        // anew (another size) is read again.
        std::fs::write(dir.join("b.jpg"), jpeg("2000:01:01 00:00:00")).unwrap();
        assert_eq!(names(false), ["b.jpg", "c.jpg", "a.jpg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
