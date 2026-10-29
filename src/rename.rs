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

/// [`templated`] with a plain name and a number: `base_01.jpg`,
/// `base_02.heic`…; the number alone without a base.
#[cfg(test)]
fn numbered(paths: &[PathBuf], dirs: usize, base: &str, start: u32) -> Vec<PathBuf> {
    templated(paths, dirs, base, start, true, &[])
}

/// The dates of a file a template may name, as FILETIMEs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dates {
    /// When the photo was taken (`info::taken`); None when its metadata
    /// does not say, and `{taken}` then takes the date modified.
    pub taken: Option<u64>,
    pub modified: u64,
}

/// The dates of `paths` for [`templated`], read from their metadata
/// (milliseconds a file: call it on a thread for many).
pub fn read_dates(paths: &[PathBuf]) -> Vec<Dates> {
    use std::os::windows::fs::MetadataExt;
    paths
        .iter()
        .map(|p| Dates {
            taken: if p.is_dir() { None } else { crate::info::taken(p) },
            modified: std::fs::metadata(p).map_or(0, |m| m.last_write_time()),
        })
        .collect()
}

/// Whether `template` names a date (`{taken}`, `{modified}`), so that
/// [`read_dates`] is waited for.
pub fn uses_dates(template: &str) -> bool {
    placeholders(template).any(|(name, _)| name == "taken" || name == "modified")
}

/// Whether `template` places the number itself (`{n}`, `{n:4}`).
pub fn places_number(template: &str) -> bool {
    placeholders(template).any(|(name, _)| name == "n")
}

/// Whether `template` would leave the names empty: nothing in it and no
/// number added (the files would be called `.jpg`, ` (2).jpg`…).
pub fn blank(template: &str, number: bool) -> bool {
    !number && template.trim().is_empty()
}

/// The `{name}` and `{name:arg}` of `template`: (name, arg).
fn placeholders(template: &str) -> impl Iterator<Item = (&str, &str)> {
    template.split('{').skip(1).filter_map(|rest| {
        let inner = rest.split_once('}')?.0;
        Some(inner.split_once(':').unwrap_or((inner, "")))
    })
}

/// The date `t` as `pattern` says: `YYYY`, `MM`, `DD`, `HH`, `mm`, `ss`
/// replaced; `YYYY-MM-DD HH-mm-ss` when the pattern is empty.
fn date(t: u64, pattern: &str) -> String {
    let Some(d) = crate::win::local_parts(t) else { return "…".into() };
    let pattern = if pattern.is_empty() { "YYYY-MM-DD HH-mm-ss" } else { pattern };
    pattern
        .replace("YYYY", &format!("{:04}", d.year))
        .replace("MM", &format!("{:02}", d.month))
        .replace("DD", &format!("{:02}", d.day))
        .replace("HH", &format!("{:02}", d.hour))
        .replace("mm", &format!("{:02}", d.minute))
        .replace("ss", &format!("{:02}", d.second))
}

/// The new names of `paths` from `template`: its text with `{n}` the
/// number (`{n:4}` four digits), `{name}` the old name without its
/// extension, `{taken}` the date taken and `{modified}` the date modified
/// (`YYYY-MM-DD HH-mm-ss`, or as `{taken:YYYY-MM-DD}` says; `…` while
/// `dates` does not have them yet); anything else in braces stays as it
/// is. Without `{n}`, the number is added after `_` when `number` (the
/// number alone when the template is empty), otherwise names that came
/// out equal get ` (2)`, ` (3)`… as Explorer gives copies. The numbers
/// run from `start` with as many digits as the last has, at least two.
/// Each file stays in its own folder with its own extension; the first
/// `dirs` are folders, whose names keep nothing after a dot.
pub fn templated(paths: &[PathBuf], dirs: usize, template: &str, start: u32, number: bool, dates: &[Dates]) -> Vec<PathBuf> {
    let last = start as u64 + paths.len().saturating_sub(1) as u64;
    let width = last.to_string().len().max(2);
    let template = template.trim();
    let placed = places_number(template);
    let mut stems: Vec<String> = paths
        .iter()
        .enumerate()
        .map(|(k, path)| {
            let num = start as u64 + k as u64;
            let old = if k < dirs { file_name(path) } else { path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() };
            let expanded = expand(template, |name, arg| match name {
                "n" => Some(format!("{num:0w$}", w = arg.parse().unwrap_or(width))),
                "name" => Some(old.clone()),
                "taken" | "modified" => Some(match dates.get(k) {
                    Some(d) => date(if name == "taken" { d.taken.unwrap_or(d.modified) } else { d.modified }, arg),
                    None => "…".into(),
                }),
                _ => None,
            });
            if placed || !number {
                expanded
            } else if expanded.is_empty() {
                format!("{num:0width$}")
            } else {
                format!("{expanded}_{num:0width$}")
            }
        })
        .collect();
    let exts: Vec<String> = paths
        .iter()
        .enumerate()
        .map(|(k, path)| path.extension().filter(|_| k >= dirs).map_or_else(String::new, |e| format!(".{}", e.to_string_lossy())))
        .collect();
    if !placed && !number {
        distinguish(paths, &mut stems, &exts);
    }
    paths.iter().zip(stems).zip(exts).map(|((path, stem), ext)| path.with_file_name(format!("{stem}{ext}"))).collect()
}

/// `template` with each `{name}` or `{name:arg}` replaced by what
/// `field(name, arg)` gives, kept as it is when that is None.
fn expand(template: &str, field: impl Fn(&str, &str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(close) = rest.find('}') else { break };
        let inner = &rest[1..close];
        let (name, arg) = inner.split_once(':').unwrap_or((inner, ""));
        match field(name, arg) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Equal names in one folder (`stems` with their `exts`, case ignored;
/// the folder of each of `paths`) get ` (2)`, ` (3)`… after the stem from
/// the second on.
fn distinguish(paths: &[PathBuf], stems: &mut [String], exts: &[String]) {
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for ((stem, ext), path) in stems.iter_mut().zip(exts).zip(paths) {
        let folder = path.parent().map(lowercase).unwrap_or_default();
        let count = seen.entry(format!("{folder}\u{0}{stem}{ext}").to_lowercase()).or_insert(0);
        *count += 1;
        if *count > 1 {
            *stem = format!("{stem} ({count})");
        }
    }
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
        assert_eq!(names(&numbered(&paths, 0, " trip ", 1)), ["trip_01.jpg", "trip_02.HEIC", "trip_03"]);
        assert_eq!(names(&numbered(&paths, 0, "", 99)), ["099.jpg", "100.HEIC", "101"]);
        assert_eq!(numbered(&paths, 0, "t", 1)[0], PathBuf::from("C:/x/t_01.jpg"));
        // Folders first: a dot in a folder's name is not an extension.
        let mixed: Vec<PathBuf> = ["2024.05", "a.jpg"].iter().map(|n| PathBuf::from("C:/x").join(n)).collect();
        assert_eq!(names(&numbered(&mixed, 1, "trip", 1)), ["trip_01", "trip_02.jpg"]);
    }

    #[test]
    fn templates_place_numbers_names_and_dates() {
        let paths: Vec<PathBuf> = ["IMG_1.jpg", "IMG_2.jpg", "IMG_3.png"].iter().map(|n| PathBuf::from("C:/x").join(n)).collect();
        let t = |hour: u8| {
            crate::win::local_to_filetime(crate::info::DateTime { year: 2024, month: 5, day: 16, hour, minute: 30, second: 15 }).unwrap()
        };
        let dates = [
            Dates { taken: Some(t(10)), modified: t(12) },
            Dates { taken: None, modified: t(12) },
            Dates { taken: Some(t(10)), modified: t(13) },
        ];
        // `{n}` places the number: nothing is added, even with `number`.
        let news = templated(&paths, 0, "p{n:3}-{name}", 7, true, &dates);
        assert_eq!(names(&news), ["p007-IMG_1.jpg", "p008-IMG_2.jpg", "p009-IMG_3.png"]);
        // The date taken, the date modified of a file without one, a pattern.
        let news = templated(&paths, 0, "{taken}", 1, false, &dates);
        assert_eq!(names(&news), ["2024-05-16 10-30-15.jpg", "2024-05-16 12-30-15.jpg", "2024-05-16 10-30-15.png"]);
        let news = templated(&paths, 0, "{modified:YYYY-MM-DD}", 1, true, &dates);
        assert_eq!(names(&news), ["2024-05-16_01.jpg", "2024-05-16_02.jpg", "2024-05-16_03.png"]);
        // Equal names without a number are told apart, by the extension
        // too; unknown braces stay; no dates read yet.
        let news = templated(&paths, 0, "{taken:YYYY} {x}", 1, false, &dates);
        assert_eq!(names(&news), ["2024 {x}.jpg", "2024 {x} (2).jpg", "2024 {x}.png"]);
        assert_eq!(names(&templated(&paths[..1], 0, "{taken}", 1, false, &[])), ["….jpg"]);
        assert!(blank(" ", false) && !blank("", true) && !blank("{x}", false));
        // Equal names in other folders are not told apart.
        let apart: Vec<PathBuf> = ["C:/x/a.jpg", "C:/y/b.jpg", "C:/y/c.jpg"].iter().map(PathBuf::from).collect();
        let news = templated(&apart, 0, "same", 1, false, &[]);
        assert_eq!(names(&news), ["same.jpg", "same.jpg", "same (2).jpg"]);
        assert!(uses_dates("a{modified}") && !uses_dates("{name}"));
        assert!(places_number("{n:2}") && !places_number("{name}"));
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
