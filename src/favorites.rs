//! The favourite images (S): paths kept in `favorites.txt` beside the
//! settings, one per line in the order they were marked, UTF-8. A file
//! name cannot hold a control character, so fields a later version may add
//! follow a tab; they are kept as they are. Saved on every change, through
//! a temporary file, so that a crash never leaves half a list.
//!
//! The gallery lists them in place of a folder: `App::dir` is then [`DIR`].
//!
//! The folders pinned to the favourites are kept the same way, in
//! [`PINNED_FILE`]: the tree shows them under the favourites, and the
//! gallery as cells before the favourite images.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The file in the settings folder.
pub const FILE: &str = "favorites.txt";
/// The pinned folders' file, beside it.
pub const PINNED_FILE: &str = "pinned.txt";

/// `App::dir` while the favourites are listed: no folder can have this
/// path (a colon), and as a path it keys the gallery's per-folder state,
/// the history and the tree's selection like any folder.
pub const DIR: &str = "::favorites";

/// Whether `dir` is [`DIR`].
pub fn is_dir(dir: &Path) -> bool {
    dir.as_os_str() == DIR
}

struct Entry {
    path: PathBuf,
    /// What follows the path on its line, from the tab on.
    rest: String,
}

#[derive(Default)]
pub struct Favorites {
    entries: Vec<Entry>,
    /// The lowercase paths: Windows paths ignore case.
    keys: HashSet<String>,
    /// Where they are kept; `None`: not kept (no settings folder).
    file: Option<PathBuf>,
    /// The pinned folders, not the favourite images (for the messages).
    folders: bool,
}

/// Windows paths ignore case, and a folder is the same with or without
/// its trailing separator.
fn key(path: &Path) -> String {
    path.components().collect::<PathBuf>().to_string_lossy().to_lowercase()
}

impl Favorites {
    /// Read from `file`; none yet is no favourites.
    pub fn load(file: Option<PathBuf>) -> Self {
        Self::read(file, false)
    }

    /// The pinned folders, read from `file`.
    pub fn load_pinned(file: Option<PathBuf>) -> Self {
        Self::read(file, true)
    }

    fn read(file: Option<PathBuf>, folders: bool) -> Self {
        let mut favorites = Self { file, folders, ..Self::default() };
        let text = match favorites.file.as_ref().map(std::fs::read_to_string) {
            None => None,
            Some(Ok(text)) => Some(text),
            Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Some(Err(e)) => {
                // Not UTF-8 (saved by hand in another encoding) or not
                // readable: kept as it is, not overwritten by an empty list.
                eprintln!("favorites: {e}");
                favorites.file = None;
                None
            }
        };
        if let Some(text) = text {
            // Notepad may start the file with a byte order mark.
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            for line in text.lines() {
                let (path, rest) = line.find('\t').map_or((line, ""), |i| line.split_at(i));
                let path = path.trim();
                if !path.is_empty() && favorites.keys.insert(key(Path::new(path))) {
                    favorites.entries.push(Entry { path: path.into(), rest: rest.to_owned() });
                }
            }
        }
        favorites
    }

    fn save(&self) -> Result<(), String> {
        let Some(file) = &self.file else { return Ok(()) };
        let mut text = String::new();
        for e in &self.entries {
            text.push_str(&e.path.to_string_lossy());
            text.push_str(&e.rest);
            text.push('\n');
        }
        let temp = file.with_extension("tmp");
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&temp, text))
            .and_then(|()| std::fs::rename(&temp, file));
        result.map_err(|e| {
            log::warn!("cannot save {}: {e}", file.display());
            if self.folders {
                tr!(format!("Cannot save the pinned folders: {e}"), format!("Не удалось сохранить закреплённые папки: {e}"))
            } else {
                tr!(format!("Cannot save the favorites: {e}"), format!("Не удалось сохранить избранное: {e}"))
            }
        })
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.keys.contains(&key(path))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// In the order they were marked.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries.iter().map(|e| e.path.clone()).collect()
    }

    /// Mark `path`, or unmark it if it is marked; whether it is marked now.
    pub fn toggle(&mut self, path: &Path) -> Result<bool, String> {
        let marked = if self.keys.remove(&key(path)) {
            self.entries.retain(|e| key(&e.path) != key(path));
            false
        } else {
            self.keys.insert(key(path));
            self.entries.push(Entry { path: path.to_path_buf(), rest: String::new() });
            true
        };
        self.save()?;
        Ok(marked)
    }

    /// Mark those of `paths` that are not marked; how many.
    pub fn add_all(&mut self, paths: &[PathBuf]) -> Result<usize, String> {
        let mut added = 0;
        for path in paths {
            if self.keys.insert(key(path)) {
                self.entries.push(Entry { path: path.clone(), rest: String::new() });
                added += 1;
            }
        }
        if added > 0 {
            self.save()?;
        }
        Ok(added)
    }

    /// `old` is now called `new`.
    pub fn renamed(&mut self, pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
        // One lookup per entry: a new name may be another pair's old one.
        let news: std::collections::HashMap<String, &PathBuf> = pairs.iter().map(|(old, new)| (key(old), new)).collect();
        let mut changed = false;
        for e in &mut self.entries {
            if let Some(new) = news.get(&key(&e.path)) {
                e.path = (*new).clone();
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
        self.keys = self.entries.iter().map(|e| key(&e.path)).collect();
        self.save()
    }

    /// Unmark the paths `gone` says, if any; how many.
    pub fn remove_if(&mut self, gone: impl Fn(&Path) -> bool) -> Result<usize, String> {
        let before = self.entries.len();
        self.entries.retain(|e| !gone(&e.path));
        let removed = before - self.entries.len();
        if removed == 0 {
            return Ok(0);
        }
        self.keys = self.entries.iter().map(|e| key(&e.path)).collect();
        self.save().map(|()| removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_keeps_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("qview_favorites_{}", std::process::id()));
        let file = dir.join(FILE);
        // Blank lines and spaces around a path are skipped, a repeated path
        // is read once, and what follows a tab is kept.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, "C:\\a\\1.jpg\n\n  C:\\b\\Фото.png \t added=1\nc:\\A\\1.JPG\n").unwrap();
        let mut f = Favorites::load(Some(file.clone()));
        assert_eq!(f.paths(), [PathBuf::from(r"C:\a\1.jpg"), PathBuf::from(r"C:\b\Фото.png")]);
        assert!(f.contains(Path::new(r"c:\B\фото.PNG")));
        assert_eq!(f.toggle(Path::new(r"D:\c.gif")), Ok(true));
        assert_eq!(f.toggle(Path::new(r"c:\a\1.JPG")), Ok(false));
        f.renamed(&[(PathBuf::from(r"C:\b\Фото.png"), PathBuf::from(r"C:\b\Море.png"))]).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "C:\\b\\Море.png\t added=1\nD:\\c.gif\n");
        assert!(!f.contains(Path::new(r"C:\b\Фото.png")));
        let f = Favorites::load(Some(file.clone()));
        assert_eq!(f.len(), 2);
        assert!(f.contains(Path::new(r"C:\b\Море.png")));
        let mut f = f;
        assert_eq!(f.remove_if(|p| p.extension().is_some_and(|e| e == "gif")), Ok(1));
        assert_eq!(f.remove_if(|_| false), Ok(0));
        assert_eq!(f.paths(), [PathBuf::from(r"C:\b\Море.png")]);
        // A chain of renames (1 to 2, 2 to 3) keeps both.
        let p = |s: &str| PathBuf::from(format!(r"C:\c\{s}.jpg"));
        f.add_all(&[p("1"), p("2")]).unwrap();
        f.renamed(&[(p("1"), p("2")), (p("2"), p("3"))]).unwrap();
        assert!(f.contains(&p("2")) && f.contains(&p("3")) && !f.contains(&p("1")));
        // A byte order mark is skipped.
        std::fs::write(&file, "\u{feff}C:\\d.png\n").unwrap();
        assert!(Favorites::load(Some(file.clone())).contains(Path::new(r"C:\d.png")));
        std::fs::remove_dir_all(&dir).unwrap();
        // No file yet: none.
        assert_eq!(Favorites::load(Some(file)).len(), 0);
        assert!(is_dir(Path::new(DIR)) && !is_dir(Path::new(r"C:\favorites")));
        // The pinned folders: the same list, in a file of their own.
        let pinned = dir.join(PINNED_FILE);
        let mut p = Favorites::load_pinned(Some(pinned.clone()));
        assert_eq!(p.toggle(Path::new(r"D:\Фото")), Ok(true));
        assert!(p.contains(Path::new(r"D:\Фото\")));
        assert!(Favorites::load_pinned(Some(pinned)).contains(Path::new(r"d:\фото")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
