//! Renaming several files at once (F2 with several images chosen in the
//! gallery): a name and a number each, in the order of the grid, the
//! extensions kept. All are renamed or none. Moving files into a pinned
//! folder (Alt+1 to Alt+9) goes the same way ([`move_all`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::app::file_name;

/// Why files could not be moved.
#[derive(Debug, PartialEq, Eq)]
pub enum MoveError {
    /// The folder is on another drive: a rename cannot take them there.
    OtherDrive,
    Other(String),
}

/// The paths of `files` in the folder `to`, with their names.
pub fn into_folder(files: &[PathBuf], to: &Path) -> Vec<(PathBuf, PathBuf)> {
    files.iter().map(|f| (f.clone(), to.join(f.file_name().unwrap_or_default()))).collect()
}

/// Move every `(old, new)` of `pairs`, new in another folder, by renaming.
/// A failure puts back what was done; on another drive nothing is done.
pub fn move_all(pairs: &[(PathBuf, PathBuf)]) -> Result<(), MoveError> {
    const ERROR_NOT_SAME_DEVICE: i32 = 17;
    let mut done: Vec<&(PathBuf, PathBuf)> = Vec::new();
    let mut result = Ok(());
    for pair in pairs {
        match std::fs::rename(&pair.0, &pair.1) {
            Ok(()) => done.push(pair),
            Err(e) if e.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE) => {
                result = Err(MoveError::OtherDrive);
                break;
            }
            Err(e) => {
                let name = file_name(&pair.0);
                result = Err(MoveError::Other(tr!(format!("Cannot move {name}: {e}"), format!("Не удалось переместить {name}: {e}"))));
                break;
            }
        }
    }
    if result.is_err() {
        for (old, new) in done.into_iter().rev() {
            let _ = std::fs::rename(new, old);
        }
    }
    result
}

/// The new names of `paths`: `base_01.jpg`, `base_02.heic`… numbered from
/// `start` with as many digits as the last number has (at least two), each
/// in its own folder with its own extension. Without a base, the number
/// alone.
pub fn numbered(paths: &[PathBuf], base: &str, start: u32) -> Vec<PathBuf> {
    let last = start as u64 + paths.len().saturating_sub(1) as u64;
    let width = last.to_string().len().max(2);
    let base = base.trim();
    paths
        .iter()
        .enumerate()
        .map(|(k, path)| {
            let number = format!("{:0width$}", start as u64 + k as u64);
            let stem = if base.is_empty() { number } else { format!("{base}_{number}") };
            let name = match path.extension() {
                Some(ext) => format!("{stem}.{}", ext.to_string_lossy()),
                None => stem,
            };
            path.with_file_name(name)
        })
        .collect()
}

fn lowercase(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Why `pairs` (old, new) cannot be renamed, if they cannot: a name
/// Windows refuses, or one taken by a file that is not among them.
pub fn check(pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    let olds: HashSet<String> = pairs.iter().map(|(old, _)| lowercase(old)).collect();
    let mut news = HashSet::new();
    for (_, new) in pairs {
        let name = file_name(new);
        crate::folder::check_name(&name)?;
        if !news.insert(lowercase(new)) || (new.exists() && !olds.contains(&lowercase(new))) {
            return Err(tr!(format!("{name} already exists"), format!("{name} уже существует")));
        }
    }
    Ok(())
}

/// Rename every `(old, new)` of `pairs`, first to temporary names, so that
/// the files may take each other's names. A failure puts back what was
/// done.
pub fn rename_all(pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    let temps: Vec<PathBuf> = pairs
        .iter()
        .enumerate()
        .map(|(k, (old, _))| old.with_file_name(format!(".qview-rename-{}-{k}", std::process::id())))
        .collect();
    // The renames done, to undo them backwards.
    let mut done: Vec<(&Path, &Path)> = Vec::new();
    let mut result = Ok(());
    let steps = pairs.iter().zip(&temps).map(|((old, _), tmp)| (old.as_path(), tmp.as_path()));
    let steps = steps.chain(pairs.iter().zip(&temps).map(|((_, new), tmp)| (tmp.as_path(), new.as_path())));
    for (from, to) in steps {
        match std::fs::rename(from, to) {
            Ok(()) => done.push((from, to)),
            Err(e) => {
                let name = file_name(from);
                result = Err(tr!(format!("Cannot rename {name}: {e}"), format!("Не удалось переименовать {name}: {e}")));
                break;
            }
        }
    }
    if result.is_err() {
        for (from, to) in done.into_iter().rev() {
            let _ = std::fs::rename(to, from);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_rename_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|p| file_name(p)).collect()
    }

    #[test]
    fn numbers_keep_the_extensions() {
        let paths: Vec<PathBuf> = ["a.jpg", "b.HEIC", "c"].iter().map(|n| PathBuf::from("C:/x").join(n)).collect();
        assert_eq!(names(&numbered(&paths, " trip ", 1)), ["trip_01.jpg", "trip_02.HEIC", "trip_03"]);
        assert_eq!(names(&numbered(&paths, "", 99)), ["099.jpg", "100.HEIC", "101"]);
        assert_eq!(numbered(&paths, "t", 1)[0], PathBuf::from("C:/x/t_01.jpg"));
    }

    #[test]
    fn names_can_be_swapped_and_failures_undone() {
        let dir = temp_dir("swap");
        let (a, b) = (dir.join("a.jpg"), dir.join("b.jpg"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let swap = vec![(a.clone(), b.clone()), (b.clone(), a.clone())];
        check(&swap).unwrap();
        rename_all(&swap).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "b");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");

        // A name taken by another file.
        let other = dir.join("c.jpg");
        std::fs::write(&other, "c").unwrap();
        assert!(check(&[(a.clone(), other.clone())]).is_err());
        assert!(check(&[(a.clone(), dir.join("bad?.jpg"))]).is_err());
        assert!(check(&[(a.clone(), dir.join("n.jpg")), (b.clone(), dir.join("N.jpg"))]).is_err());

        // The second file is missing: the first gets its name back.
        let missing = vec![(a.clone(), dir.join("x.jpg")), (dir.join("gone.jpg"), dir.join("y.jpg"))];
        assert!(rename_all(&missing).is_err());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "b");
        // No temporary name left (another program, an antivirus, may put
        // a file of its own beside one for a moment: not counted).
        let listed: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!listed.iter().any(|n| n.starts_with(".qview-rename-")), "{listed:?}");
        assert_eq!(listed.iter().filter(|n| n.ends_with(".jpg")).count(), 3, "{listed:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moves_into_a_folder_or_not_at_all() {
        let dir = temp_dir("move");
        let to = dir.join("to");
        std::fs::create_dir_all(&to).unwrap();
        let (a, b) = (dir.join("a.jpg"), dir.join("b.jpg"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let pairs = into_folder(&[a.clone(), b.clone()], &to);
        assert_eq!(pairs[1], (b.clone(), to.join("b.jpg")));
        // A name taken in the folder.
        std::fs::write(to.join("b.jpg"), "x").unwrap();
        assert!(check(&pairs).is_err());
        // The second cannot be moved: the first comes back.
        let missing = vec![pairs[0].clone(), (dir.join("gone.jpg"), to.join("gone.jpg"))];
        assert!(matches!(move_all(&missing), Err(MoveError::Other(_))));
        assert!(a.exists() && !to.join("a.jpg").exists());
        std::fs::remove_file(to.join("b.jpg")).unwrap();
        check(&pairs).unwrap();
        move_all(&pairs).unwrap();
        assert!(!a.exists() && to.join("a.jpg").exists() && to.join("b.jpg").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
