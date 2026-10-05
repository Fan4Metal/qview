//! Renaming several files at once (F2 with several images chosen in the
//! gallery): a name and a number each, in the order of the grid, the
//! extensions kept. All are renamed or none.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::app::file_name;

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
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
