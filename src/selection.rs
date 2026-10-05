//! Several images chosen in the gallery: Ctrl and Shift with clicks and
//! with the keys, Ctrl+A, and a frame dragged over the grid (`ui::gallery`).
//! The commands that act on files (copy, delete, rename, convert, the
//! favourites) act on all of them; with none chosen, on the current image,
//! as in the viewer. The current image stays the one the keys move from.

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
}

/// A frame being dragged: where it started, in the grid's coordinates, and
/// what was chosen before it (kept with Ctrl, otherwise nothing).
pub struct Band {
    pub start: Pos2,
    pub before: HashSet<PathBuf>,
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

    /// Nothing chosen; Shift starts from the current image again.
    pub fn clear(&mut self) {
        self.paths.clear();
        self.anchor = None;
        self.band = None;
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.remove(path);
    }

    /// The chosen images, in the order of `files`.
    pub fn in_order(&self, files: &[PathBuf]) -> Vec<PathBuf> {
        files.iter().filter(|f| self.paths.contains(*f)).cloned().collect()
    }

    /// Only what `files` still lists stays chosen (after the folder was
    /// listed again).
    pub fn retain_listed(&mut self, files: &[PathBuf]) {
        if self.paths.is_empty() && self.anchor.is_none() {
            return;
        }
        let listed: HashSet<&PathBuf> = files.iter().collect();
        self.paths.retain(|p| listed.contains(p));
        if self.anchor.as_ref().is_some_and(|a| !listed.contains(a)) {
            self.anchor = None;
        }
    }

    /// A file renamed: still chosen under its new name.
    pub fn renamed(&mut self, old: &Path, new: &Path) {
        if self.paths.remove(old) {
            self.paths.insert(new.to_path_buf());
        }
        if self.anchor.as_deref() == Some(old) {
            self.anchor = Some(new.to_path_buf());
        }
    }

    /// Choose `paths` instead, or with `before` besides.
    pub fn set(&mut self, before: &HashSet<PathBuf>, paths: impl IntoIterator<Item = PathBuf>) {
        self.paths = before.clone();
        self.paths.extend(paths);
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

    /// The chosen images, or the current one when none is.
    pub fn chosen_or_current(&self) -> HashSet<PathBuf> {
        if self.selection.is_empty() { self.current.iter().cloned().collect() } else { self.selection.paths.clone() }
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
        s.renamed(&files[1], Path::new("z"));
        assert!(s.contains(Path::new("z")));
        s.retain_listed(&files);
        assert_eq!(s.in_order(&files), [files[3].clone()]);
        assert_eq!(range(3, 1), 1..=3);
    }
}
