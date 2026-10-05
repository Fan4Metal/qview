//! The folders the gallery has shown, for Back and Forward (Alt+← and
//! Alt+→, the mouse's side buttons), as in Explorer.

use std::path::PathBuf;

/// Places kept behind and ahead at most.
const LIMIT: usize = 100;

/// A folder as it was left: how it was listed and where the user was in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub dir: PathBuf,
    /// Listed with its sub-folders.
    pub deep: bool,
    /// The image that was current.
    pub current: Option<PathBuf>,
    /// How far below the top of the grid the current image's row was, when
    /// it was left in the gallery.
    pub below: Option<f32>,
}

#[derive(Default)]
pub struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl History {
    /// `left` has been left for another folder: Back returns to it, and
    /// what Forward had is forgotten.
    pub fn visit(&mut self, left: Place) {
        push(&mut self.back, left);
        self.forward.clear();
    }

    /// The place before `here`, which Forward then returns to.
    pub fn back(&mut self, here: Option<Place>) -> Option<Place> {
        let to = self.back.pop()?;
        if let Some(here) = here {
            push(&mut self.forward, here);
        }
        Some(to)
    }

    /// The place `back` left, which Back then returns from.
    pub fn forward(&mut self, here: Option<Place>) -> Option<Place> {
        let to = self.forward.pop()?;
        if let Some(here) = here {
            push(&mut self.back, here);
        }
        Some(to)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

fn push(places: &mut Vec<Place>, place: Place) {
    if places.len() == LIMIT {
        places.remove(0);
    }
    places.push(place);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(dir: &str) -> Place {
        Place { dir: dir.into(), deep: false, current: None, below: None }
    }

    #[test]
    fn back_and_forward() {
        let mut h = History::default();
        assert!(!h.can_go_back() && !h.can_go_forward());
        assert_eq!(h.back(Some(at("a"))), None);
        // a → b → c
        h.visit(at("a"));
        h.visit(at("b"));
        assert_eq!(h.back(Some(at("c"))), Some(at("b")));
        assert_eq!(h.back(Some(at("b"))), Some(at("a")));
        assert!(!h.can_go_back());
        assert_eq!(h.forward(Some(at("a"))), Some(at("b")));
        assert_eq!(h.forward(Some(at("b"))), Some(at("c")));
        assert!(!h.can_go_forward());
        assert_eq!(h.back(Some(at("c"))), Some(at("b")));
        // A new folder from b forgets c.
        h.visit(at("b"));
        assert!(!h.can_go_forward());
        assert_eq!(h.back(Some(at("d"))), Some(at("b")));
        assert_eq!(h.back(Some(at("b"))), Some(at("a")));
    }

    #[test]
    fn keeps_the_latest() {
        let mut h = History::default();
        for i in 0..LIMIT + 5 {
            h.visit(at(&i.to_string()));
        }
        let mut last = None;
        while let Some(p) = h.back(None) {
            last = Some(p);
        }
        assert_eq!(last, Some(at("5")));
    }
}
