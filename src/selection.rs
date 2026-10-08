//! Several images chosen in the gallery: Ctrl and Shift with clicks and
//! with the keys, Ctrl+A, and a frame dragged over the grid (`ui::gallery`).
//! The commands that act on files (copy, delete, rename, convert, the
//! favourites) act on all of them; with none chosen, on the current image,
//! as in the viewer. The current image stays the one the keys move from.
//! Sub-folders' cells are chosen the same way (clicks, the frame), kept
//! apart: only deleting and renaming act on them, with the images chosen.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{Modifiers, Pos2};

use crate::app::App;

#[derive(Default)]
pub struct Selection {
    /// As listed in `App::files`.
    paths: HashSet<PathBuf>,
    /// Where a Shift range starts: the image last clicked without Shift;
    /// the current one when there is none.
    anchor: Option<PathBuf>,
    /// The frame being dragged over the grid.
    pub band: Option<Band>,
    /// The sub-folders chosen, as in `App::folders`.
    folders: HashSet<PathBuf>,
    /// Where a Shift range of folders starts: the folder last clicked
    /// without Shift; the one with the cursor when there is none.
    folder_anchor: Option<PathBuf>,
}

/// A frame being dragged: where it started, in the grid's coordinates, and
/// what was chosen before it (kept with Ctrl, otherwise nothing): images
/// and folders.
pub struct Band {
    pub start: Pos2,
    pub before: HashSet<PathBuf>,
    pub before_folders: HashSet<PathBuf>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.paths.contains(path)
    }

    /// Whether sub-folders are chosen.
    pub fn has_folders(&self) -> bool {
        !self.folders.is_empty()
    }

    pub fn folders_len(&self) -> usize {
        self.folders.len()
    }

    pub fn contains_folder(&self, path: &Path) -> bool {
        self.folders.contains(path)
    }

    /// Whether anything is chosen, images or folders: the cells show their
    /// circles.
    pub fn choosing(&self) -> bool {
        !self.paths.is_empty() || !self.folders.is_empty()
    }

    /// The chosen folders, in the order of `folders`.
    pub fn folders_in_order(&self, folders: &[PathBuf]) -> Vec<PathBuf> {
        folders.iter().filter(|f| self.folders.contains(*f)).cloned().collect()
    }

    /// The chosen folders, for a frame dragged with Ctrl.
    pub fn chosen_folders(&self) -> HashSet<PathBuf> {
        self.folders.clone()
    }

    /// Nothing chosen; Shift starts from the current image again.
    pub fn clear(&mut self) {
        self.paths.clear();
        self.anchor = None;
        self.band = None;
        self.clear_folders();
    }

    /// No folder chosen.
    pub fn clear_folders(&mut self) {
        self.folders.clear();
        self.folder_anchor = None;
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.remove(path);
        self.folders.remove(path);
    }

    /// The chosen images, in the order of `files`.
    pub fn in_order(&self, files: &[PathBuf]) -> Vec<PathBuf> {
        files.iter().filter(|f| self.paths.contains(*f)).cloned().collect()
    }

    /// Only what `files` and `folders` still list stays chosen (after the
    /// folder was listed again, or filtered).
    pub fn retain_listed(&mut self, files: &[PathBuf], folders: &[PathBuf]) {
        fn retain(set: &mut HashSet<PathBuf>, anchor: &mut Option<PathBuf>, list: &[PathBuf]) {
            if set.is_empty() && anchor.is_none() {
                return;
            }
            let listed: HashSet<&PathBuf> = list.iter().collect();
            set.retain(|p| listed.contains(p));
            if anchor.as_ref().is_some_and(|a| !listed.contains(a)) {
                *anchor = None;
            }
        }
        retain(&mut self.paths, &mut self.anchor, files);
        retain(&mut self.folders, &mut self.folder_anchor, folders);
    }

    /// A file or folder renamed: still chosen under its new name.
    pub fn renamed(&mut self, pairs: &[(PathBuf, PathBuf)]) {
        fn rename(set: &mut HashSet<PathBuf>, anchor: &mut Option<PathBuf>, pairs: &[(PathBuf, PathBuf)]) {
            // All taken out first: a new name may be another pair's old one.
            let chosen: Vec<&PathBuf> = pairs.iter().filter(|(old, _)| set.remove(old)).map(|(_, new)| new).collect();
            set.extend(chosen.into_iter().cloned());
            if let Some((_, new)) = pairs.iter().find(|(old, _)| anchor.as_ref() == Some(old)) {
                *anchor = Some(new.clone());
            }
        }
        rename(&mut self.paths, &mut self.anchor, pairs);
        rename(&mut self.folders, &mut self.folder_anchor, pairs);
    }

    /// Choose `paths` instead, or with `before` besides.
    pub fn set(&mut self, before: &HashSet<PathBuf>, paths: impl IntoIterator<Item = PathBuf>) {
        self.paths = before.clone();
        self.paths.extend(paths);
    }

    /// Choose the folders `folders` instead, or with `before` besides.
    pub fn set_folders(&mut self, before: &HashSet<PathBuf>, folders: impl IntoIterator<Item = PathBuf>) {
        self.folders = before.clone();
        self.folders.extend(folders);
    }
}

/// The cells from `a` to `b`, either way round.
fn range(a: usize, b: usize) -> std::ops::RangeInclusive<usize> {
    a.min(b)..=a.max(b)
}

impl App {
    /// What the file commands act on: the chosen images in the gallery, in
    /// the order of the grid, otherwise the current image.
    pub fn targets(&self) -> Vec<PathBuf> {
        if self.gallery_open && !self.selection.is_empty() {
            self.selection.in_order(&self.files)
        } else {
            self.current.iter().cloned().collect()
        }
    }

    /// Where a Shift range starts.
    fn anchor_index(&self) -> Option<usize> {
        self.selection.anchor.as_deref().and_then(|a| crate::folder::position(&self.files, a)).or(self.index)
    }

    /// A click on cell `i` with `m` held: alone it chooses that image only,
    /// Ctrl adds it or takes it out, Shift chooses the images from the
    /// anchor to it (Ctrl+Shift adds them). The image becomes current.
    pub fn click_cell(&mut self, i: usize, m: Modifiers) {
        let Some(path) = self.files.get(i).cloned() else { return };
        if m.shift {
            let from = self.anchor_index().unwrap_or(i);
            let before = if m.ctrl { self.chosen_or_current() } else { HashSet::new() };
            let range: Vec<PathBuf> = self.files[range(from, i)].to_vec();
            self.selection.set(&before, range);
            if !m.ctrl {
                self.selection.clear_folders();
            }
        } else if m.ctrl {
            // The current image is chosen until something else is.
            let mut chosen = self.chosen_or_current();
            if !chosen.remove(&path) {
                chosen.insert(path.clone());
            }
            self.selection.paths = chosen;
            self.selection.anchor = Some(path);
        } else {
            self.selection.clear();
            self.selection.anchor = Some(path);
        }
        self.go(i);
    }

    /// The chosen images, or the current one when nothing is chosen (with
    /// folders chosen, the current image is not among them).
    pub fn chosen_or_current(&self) -> HashSet<PathBuf> {
        if self.selection.choosing() { self.selection.paths.clone() } else { self.current.iter().cloned().collect() }
    }

    /// A click on the cell of sub-folder `k` with `m` held: alone it puts
    /// the cursor there and chooses nothing, Ctrl adds the folder or takes
    /// it out (with the folder that had the cursor, when nothing was
    /// chosen), Shift chooses the folders from the anchor to it (Ctrl+Shift
    /// adds them, keeping the images chosen). The folder gets the cursor.
    pub fn click_folder(&mut self, k: usize, m: Modifiers) {
        let Some(path) = self.folders.get(k).cloned() else { return };
        if m.shift {
            let anchor = self.selection.folder_anchor.as_deref().and_then(|a| crate::folder::position(&self.folders, a));
            let from = anchor.or(self.folder_focus).filter(|&f| f < self.folders.len()).unwrap_or(k);
            if self.selection.folder_anchor.is_none() {
                self.selection.folder_anchor = self.folders.get(from).cloned();
            }
            let before = if m.ctrl { self.selection.folders.clone() } else { HashSet::new() };
            if !m.ctrl {
                self.selection.paths.clear();
                self.selection.anchor = None;
            }
            let range: Vec<PathBuf> = self.folders[range(from, k)].to_vec();
            self.selection.set_folders(&before, range);
        } else if m.ctrl {
            let mut chosen = self.selection.folders.clone();
            if !self.selection.choosing()
                && let Some(focused) = self.focused_folder()
            {
                chosen.insert(focused);
            }
            if !chosen.remove(&path) {
                chosen.insert(path.clone());
            }
            self.selection.folders = chosen;
            self.selection.folder_anchor = Some(path);
        } else {
            self.selection.clear();
            self.selection.folder_anchor = Some(path);
        }
        self.folder_focus = Some(k);
    }

    /// A right click on the cell of sub-folder `k`: a chosen folder keeps
    /// the others chosen, for its menu to act on them all; with another,
    /// nothing is chosen. The folder gets the cursor.
    pub fn right_click_folder(&mut self, k: usize) {
        let Some(path) = self.folders.get(k) else { return };
        if !self.selection.contains_folder(path) {
            self.selection.clear();
        }
        self.folder_focus = Some(k);
    }

    /// What Delete and F2 act on while folders are chosen: those folders,
    /// in the grid's order, then the images chosen.
    pub fn chosen_items(&self) -> Vec<PathBuf> {
        let mut items = self.selection.folders_in_order(&self.folders);
        items.extend(self.selection.in_order(&self.files));
        items
    }

    /// A right click on cell `i`: a chosen image keeps the others chosen,
    /// for its menu to act on them all; another is chosen alone.
    pub fn right_click_cell(&mut self, i: usize) {
        let Some(path) = self.files.get(i) else { return };
        if !self.selection.contains(path) {
            self.selection.clear();
        }
        self.go(i);
    }

    /// Shift with a key that moves to cell `i`: the images from the anchor
    /// to it are chosen.
    pub fn select_to(&mut self, i: usize) {
        let Some(from) = self.anchor_index() else { return };
        if self.selection.anchor.is_none() {
            self.selection.anchor = self.files.get(from).cloned();
        }
        let range: Vec<PathBuf> = self.files[range(from, i.min(self.files.len().saturating_sub(1)))].to_vec();
        self.selection.set(&HashSet::new(), range);
        self.go(i);
    }

    /// Ctrl+A: every image listed.
    pub fn select_all(&mut self) {
        self.selection.set(&HashSet::new(), self.files.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_in_the_order_listed() {
        let files: Vec<PathBuf> = ["a", "b", "c", "d"].iter().map(PathBuf::from).collect();
        let mut s = Selection::default();
        s.set(&HashSet::new(), [files[3].clone(), files[1].clone()]);
        assert_eq!(s.in_order(&files), [files[1].clone(), files[3].clone()]);
        s.renamed(&[(files[1].clone(), PathBuf::from("z"))]);
        assert!(s.contains(Path::new("z")));
        s.retain_listed(&files, &[]);
        assert_eq!(s.in_order(&files), [files[3].clone()]);
        assert_eq!(range(3, 1), 1..=3);
    }

    #[test]
    fn folders_kept_apart() {
        let folders: Vec<PathBuf> = ["x", "y", "z"].iter().map(PathBuf::from).collect();
        let mut s = Selection::default();
        assert!(!s.choosing());
        s.set_folders(&HashSet::new(), [folders[2].clone(), folders[0].clone()]);
        assert!(s.choosing() && s.is_empty() && s.has_folders());
        assert_eq!(s.folders_in_order(&folders), [folders[0].clone(), folders[2].clone()]);
        s.renamed(&[(folders[0].clone(), PathBuf::from("w"))]);
        assert!(s.contains_folder(Path::new("w")));
        s.retain_listed(&[], &folders);
        assert_eq!(s.folders_in_order(&folders), [folders[2].clone()]);
        s.remove(&folders[2]);
        assert!(!s.choosing());
    }
}
