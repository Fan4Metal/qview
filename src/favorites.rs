//! The favourite images (S): paths kept in `favorites.txt` beside the
//! settings, one per line in the order they were marked, UTF-8. A file
//! name cannot hold a control character, so fields a later version may add
//! follow a tab; they are kept as they are. Saved on every change, through
//! a temporary file, so that a crash never leaves half a list.
//!
//! The gallery lists them in place of a folder: `App::dir` is then [`DIR`].
//!
//! The folders pinned to Quick Access (as in Explorer) are kept the same
//! way, in [`PINNED_FILE`]: the tree shows them under Quick Access, whose
//! grid ([`PINNED_DIR`]) is their cells. A pinned folder may have a key,
//! 1 to 9 (`key=3` after the tab): Alt+3 moves the current image into it,
//! Shift+Alt+3 copies it there.

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
/// `App::dir` while Quick Access, the pinned folders, is shown.
pub const PINNED_DIR: &str = "::pinned";

/// The field of a pinned folder's key.
const KEY_FIELD: &str = "key";
/// The keys a pinned folder may have: Alt+1 to Alt+9.
pub const KEYS: std::ops::RangeInclusive<u8> = 1..=9;

/// Whether `dir` is [`DIR`].
pub fn is_dir(dir: &Path) -> bool {
    dir.as_os_str() == DIR
}

/// Whether `dir` is [`PINNED_DIR`].
pub fn is_pinned_dir(dir: &Path) -> bool {
    dir.as_os_str() == PINNED_DIR
}

/// Whether `dir` is a list of qview's shown in place of a folder: the
/// favourites or Quick Access.
pub fn is_virtual(dir: &Path) -> bool {
    is_dir(dir) || is_pinned_dir(dir)
}

/// The value of the field `name` among the tab-separated `name=value`
/// fields of `rest`.
fn field<'a>(rest: &'a str, name: &str) -> Option<&'a str> {
    rest.split('\t').filter_map(|f| f.split_once('=')).find(|(n, _)| n.trim() == name).map(|(_, v)| v.trim())
}

/// `rest` with the field `name` set to `value`, or taken out with None;
/// the other fields stay as they are.
fn with_field(rest: &str, name: &str, value: Option<&str>) -> String {
    let mut out = String::new();
    for f in rest.split('\t').skip(1).filter(|f| !f.trim().is_empty()) {
        if f.split_once('=').is_some_and(|(n, _)| n.trim() == name) {
            continue;
        }
        out.push('\t');
        out.push_str(f);
    }
    if let Some(value) = value {
        out.push('\t');
        out.push_str(name);
        out.push('=');
        out.push_str(value);
    }
    out
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

    /// The key of the pinned folder `path`, if it has one.
    pub fn key(&self, path: &Path) -> Option<u8> {
        let k = key(path);
        let e = self.entries.iter().find(|e| key(&e.path) == k)?;
        field(&e.rest, KEY_FIELD)?.parse().ok().filter(|n| KEYS.contains(n))
    }

    /// The pinned folder with the key `n`.
    pub fn with_key(&self, n: u8) -> Option<PathBuf> {
        self.entries.iter().find(|e| field(&e.rest, KEY_FIELD).and_then(|v| v.parse().ok()) == Some(n)).map(|e| e.path.clone())
    }

    /// The pinned folders with a key, by key.
    pub fn keyed(&self) -> Vec<(u8, PathBuf)> {
        let mut keyed: Vec<(u8, PathBuf)> = self.entries.iter().filter_map(|e| Some((self.key(&e.path)?, e.path.clone()))).collect();
        keyed.sort_by_key(|(k, _)| *k);
        keyed
    }

    /// Each pinned folder with its key, in the order pinned.
    pub fn entries(&self) -> Vec<(PathBuf, Option<u8>)> {
        self.entries.iter().map(|e| (e.path.clone(), self.key(&e.path))).collect()
    }

    /// Give the pinned folder `path` the key `n` (taken from the folder
    /// that had it), or no key with None.
    pub fn set_key(&mut self, path: &Path, n: Option<u8>) -> Result<(), String> {
        let k = key(path);
        if !self.keys.contains(&k) {
            return Ok(());
        }
        for e in &mut self.entries {
            let value = if key(&e.path) == k {
                n.map(|n| n.to_string())
            } else if n.is_some() && field(&e.rest, KEY_FIELD).and_then(|v| v.parse().ok()) == n {
                None
            } else {
                continue;
            };
            e.rest = with_field(&e.rest, KEY_FIELD, value.as_deref());
        }
        self.save()
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
        assert!(is_pinned_dir(Path::new(PINNED_DIR)) && is_virtual(Path::new(DIR)) && !is_virtual(Path::new(r"C:\pinned")));
    }

    #[test]
    fn pinned_folders_keep_their_keys() {
        let dir = std::env::temp_dir().join(format!("qview_pinned_{}", std::process::id()));
        let file = dir.join(PINNED_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        // Fields of other kinds are kept, a key out of range is none.
        std::fs::write(&file, "D:\\a\tcolor=red\tkey=2\nD:\\b\tkey=12\nD:\\c\n").unwrap();
        let mut p = Favorites::load_pinned(Some(file.clone()));
        assert_eq!(p.key(Path::new(r"d:\A\")), Some(2));
        assert_eq!(p.key(Path::new(r"D:\b")), None);
        assert_eq!(p.with_key(2), Some(PathBuf::from(r"D:\a")));
        assert_eq!(p.with_key(3), None);
        // A key given to another folder is taken from the one that had it.
        p.set_key(Path::new(r"D:\c"), Some(2)).unwrap();
        p.set_key(Path::new(r"D:\b"), Some(1)).unwrap();
        assert_eq!(p.keyed(), [(1, PathBuf::from(r"D:\b")), (2, PathBuf::from(r"D:\c"))]);
        assert_eq!(p.key(Path::new(r"D:\a")), None);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "D:\\a\tcolor=red\nD:\\b\tkey=1\nD:\\c\tkey=2\n");
        // No key; a folder not pinned gets none.
        p.set_key(Path::new(r"D:\b"), None).unwrap();
        p.set_key(Path::new(r"D:\x"), Some(5)).unwrap();
        assert_eq!(p.keyed(), [(2, PathBuf::from(r"D:\c"))]);
        assert_eq!(p.entries(), [(PathBuf::from(r"D:\a"), None), (PathBuf::from(r"D:\b"), None), (PathBuf::from(r"D:\c"), Some(2))]);
        // Unpinned, the key goes with it.
        assert_eq!(p.toggle(Path::new(r"D:\c")), Ok(false));
        assert_eq!(p.with_key(2), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
