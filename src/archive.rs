//! Images inside ZIP archives (comic books, `.cbz`), read without unpacking.
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

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

/// Extensions of the archives opened as folders, lowercase.
pub const EXTENSIONS: &[&str] = &["cbz", "zip"];

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
    let mut zip = open(archive)?;
    let mut items = Vec::new();
    for i in 0..zip.len() {
        let Ok(entry) = zip.by_index(i) else { continue };
        if entry.is_dir() {
            continue;
        }
        // With `\` between folders: paths are compared as text.
        let Some(name) = entry.enclosed_name().map(|n| n.components().collect::<PathBuf>()) else { continue };
        let hidden = name.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            c.starts_with('.') || c.eq_ignore_ascii_case("__MACOSX")
        });
        if hidden || !own_format(&name) {
            continue;
        }
        items.push(Item { path: archive.join(name), size: entry.size(), modified: modified(&entry, archive) });
    }
    Ok(items)
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
}
