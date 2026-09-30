//! The images of a folder, in Explorer's order, listed on a thread.

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

/// Sort `files` by file name as Explorer does (see `win::logical_cmp`);
/// names Explorer considers equal keep a fixed order.
pub fn sort(files: &mut Vec<PathBuf>) {
    let mut keyed: Vec<(Vec<u16>, PathBuf)> = std::mem::take(files)
        .into_iter()
        .map(|p| (crate::win::wide(p.file_name().unwrap_or_default()), p))
        .collect();
    keyed.sort_by(|(a, pa), (b, pb)| crate::win::logical_cmp(a, b).then_with(|| pa.cmp(pb)));
    *files = keyed.into_iter().map(|(_, p)| p).collect();
}

/// Image files of `dir`, sorted. `keep` (the file being shown) is listed
/// even when its extension is not one of [`EXTENSIONS`], so that it keeps
/// its place among the others.
pub fn list(dir: &Path, keep: Option<&Path>) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        // The type comes with the listing on Windows: no extra call.
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        if is_image(&path) {
            files.push(path);
        }
    }
    if let Some(keep) = keep
        && position(&files, keep).is_none()
        && keep.is_file()
    {
        files.push(keep.to_path_buf());
    }
    sort(&mut files);
    Ok(files)
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

/// List `dir` on a thread (a network folder can take a while) and repaint
/// `ctx` when done. Dropping the [`Scan`] discards the result.
pub fn scan(dir: PathBuf, keep: Option<PathBuf>, ctx: egui::Context) -> Scan {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("folder".into())
        .spawn(move || {
            let result = list(&dir, keep.as_deref()).map_err(|e| e.to_string());
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
        let files = list(&dir, None).unwrap();
        assert_eq!(names(files), ["1.JPG", "2.png", "10.jpg", "а.bmp", "Б.gif"]);
        // The file being shown stays listed whatever its extension.
        let files = list(&dir, Some(&dir.join("odd.xyz"))).unwrap();
        assert_eq!(names(files), ["1.JPG", "2.png", "10.jpg", "odd.xyz", "а.bmp", "Б.gif"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
