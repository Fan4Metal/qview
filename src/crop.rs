//! Cropping (C): the frame dragged over the image and how drags change it.
//! The frame is in the pixels of the image as shown, turned by the view's
//! rotation; it turns with the image. Drawing it is `ui::crop`'s, saving
//! `edit`'s.

use std::path::PathBuf;

use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::view::Zoom;

/// The smallest side of the frame, in image pixels.
const MIN_SIDE: f32 = 1.0;
/// The first frame's sides, as a part of the image's: inside its edges,
/// so that they show and can be taken.
const FIRST_FRAME: f32 = 0.8;

/// Proportions the frame keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    Free,
    /// Those of the image.
    Original,
    /// Width to height.
    Ratio(u32, u32),
}

/// As the crop bar lists them.
pub const ASPECTS: [Aspect; 9] = [
    Aspect::Free,
    Aspect::Original,
    Aspect::Ratio(1, 1),
    Aspect::Ratio(4, 3),
    Aspect::Ratio(3, 4),
    Aspect::Ratio(3, 2),
    Aspect::Ratio(2, 3),
    Aspect::Ratio(16, 9),
    Aspect::Ratio(9, 16),
];

impl Aspect {
    /// Width over height for an image of `size`; None when free.
    pub fn value(self, size: Vec2) -> Option<f32> {
        match self {
            Aspect::Free => None,
            Aspect::Original => Some(size.x / size.y),
            Aspect::Ratio(w, h) => Some(w as f32 / h as f32),
        }
    }

    /// The same proportions after a quarter turn of the image.
    pub fn turned(self) -> Aspect {
        match self {
            Aspect::Ratio(w, h) => Aspect::Ratio(h, w),
            other => other,
        }
    }

    pub fn label(self) -> String {
        match self {
            Aspect::Free => tr!("Free", "Свободные").into(),
            Aspect::Original => tr!("Original", "Исходные").into(),
            Aspect::Ratio(w, h) => format!("{w}:{h}"),
        }
    }
}

/// What a drag holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// Inside the frame: moves it.
    Move,
    /// Outside it: draws a new one.
    New,
    /// Edges, two of them at a corner.
    Edges { left: bool, right: bool, top: bool, bottom: bool },
}

/// The image being cropped and the frame on it.
pub struct Crop {
    pub path: PathBuf,
    /// In pixels of the image as shown (turned by the view).
    pub rect: Rect,
    pub aspect: Aspect,
    /// The view's zoom and panning before cropping, given back after.
    pub zoom: Zoom,
    pub offset: Vec2,
    /// The drag under way: its grip, the frame and the pointer (in image
    /// pixels) when it started.
    pub drag: Option<(Grip, Rect, Pos2)>,
}

impl Crop {
    /// The frame in the middle of the image of `size` (as shown).
    pub fn new(path: PathBuf, size: Vec2, zoom: Zoom, offset: Vec2) -> Self {
        let rect = Rect::from_center_size(whole(size).center(), size * FIRST_FRAME);
        Self { path, rect, aspect: Aspect::Free, zoom, offset, drag: None }
    }

    /// The image of `size` (as shown, before) turned a quarter: the frame
    /// and its proportions turn with it.
    pub fn turn(&mut self, size: Vec2, clockwise: bool) {
        self.rect = turn(self.rect, size, clockwise);
        self.aspect = self.aspect.turned();
        self.drag = None;
    }

    /// Keep `aspect` from now on: the largest frame of those proportions
    /// around the frame's centre.
    pub fn set_aspect(&mut self, aspect: Aspect, size: Vec2) {
        self.aspect = aspect;
        if let Some(a) = aspect.value(size) {
            self.rect = largest(self.rect.center(), a, size);
        }
    }
}

fn whole(size: Vec2) -> Rect {
    Rect::from_min_size(Pos2::ZERO, size)
}

/// `rect` on an image of `size` after the image turns a quarter.
pub fn turn(rect: Rect, size: Vec2, clockwise: bool) -> Rect {
    if clockwise {
        // (x, y) goes to (height - y, x).
        Rect::from_min_max(pos2(size.y - rect.max.y, rect.min.x), pos2(size.y - rect.min.y, rect.max.x))
    } else {
        // (x, y) goes to (y, width - x).
        Rect::from_min_max(pos2(rect.min.y, size.x - rect.max.x), pos2(rect.max.y, size.x - rect.min.x))
    }
}

/// The largest frame of proportions `aspect` in an image of `size`,
/// centred on `centre` as far as the image allows.
pub fn largest(centre: Pos2, aspect: f32, size: Vec2) -> Rect {
    let (mut w, mut h) = (size.x, size.y);
    if w > h * aspect {
        w = h * aspect;
    } else {
        h = w / aspect;
    }
    let half = vec2(w, h) / 2.0;
    let c = pos2(centre.x.clamp(half.x, size.x - half.x), centre.y.clamp(half.y, size.y - half.y));
    Rect::from_center_size(c, vec2(w, h))
}

/// The frame's whole pixels: x, y, width, height, at least one pixel, in
/// an image of `size`.
pub fn pixels(rect: Rect, size: Vec2) -> [u32; 4] {
    let axis = |min: f32, max: f32, side: f32| {
        let side = side.round().max(1.0);
        let a = min.round().clamp(0.0, side - 1.0);
        let b = max.round().clamp(a + 1.0, side);
        (a as u32, (b - a) as u32)
    };
    let (x, w) = axis(rect.min.x, rect.max.x, size.x);
    let (y, h) = axis(rect.min.y, rect.max.y, size.y);
    [x, y, w, h]
}

/// The frame holds the whole image.
pub fn is_whole(rect: Rect, size: Vec2) -> bool {
    pixels(rect, size) == [0, 0, size.x.round() as u32, size.y.round() as u32]
}

/// What a drag starting at `p` holds, `frame` and `p` in screen points:
/// an edge within `reach` of it (the nearer of two), a corner when two are,
/// otherwise the inside or the outside.
pub fn grip(frame: Rect, p: Pos2, reach: f32) -> Grip {
    let along_y = p.y >= frame.top() - reach && p.y <= frame.bottom() + reach;
    let along_x = p.x >= frame.left() - reach && p.x <= frame.right() + reach;
    let (l, r) = ((p.x - frame.left()).abs(), (p.x - frame.right()).abs());
    let (t, b) = ((p.y - frame.top()).abs(), (p.y - frame.bottom()).abs());
    let left = along_y && l <= reach && l <= r;
    let right = along_y && r <= reach && r < l;
    let top = along_x && t <= reach && t <= b;
    let bottom = along_x && b <= reach && b < t;
    if left || right || top || bottom {
        Grip::Edges { left, right, top, bottom }
    } else if frame.contains(p) {
        Grip::Move
    } else {
        Grip::New
    }
}

/// The frame after a drag of `grip` from `from` to `to`, which started
/// with the frame `start`; all in pixels of an image of `size`. With
/// `aspect` (width over height), the frame keeps those proportions.
pub fn dragged(grip: Grip, start: Rect, from: Pos2, to: Pos2, size: Vec2, aspect: Option<f32>) -> Rect {
    match grip {
        Grip::Move => {
            let d = to - from;
            let d = vec2(d.x.clamp(-start.min.x, size.x - start.max.x), d.y.clamp(-start.min.y, size.y - start.max.y));
            start.translate(d)
        }
        Grip::New => corner(whole(size).clamp(from), to, size, aspect),
        Grip::Edges { left, right, top, bottom } => {
            let d = to - from;
            let (sideways, upright) = (left || right, top || bottom);
            match aspect {
                Some(a) if sideways && upright => {
                    let anchor = pos2(if left { start.max.x } else { start.min.x }, if top { start.max.y } else { start.min.y });
                    let moved = pos2(if left { start.min.x } else { start.max.x }, if top { start.min.y } else { start.max.y });
                    corner(anchor, moved + d, size, Some(a))
                }
                Some(a) if sideways => side_edge(start, left, d.x, size, a),
                // An upper or lower edge is a side edge of the image
                // turned over its diagonal.
                Some(a) => transpose(side_edge(transpose(start), top, d.y, transpose_size(size), 1.0 / a)),
                None => {
                    let mut r = start;
                    if left {
                        r.min.x = (start.min.x + d.x).clamp(0.0, start.max.x - MIN_SIDE);
                    }
                    if right {
                        r.max.x = (start.max.x + d.x).clamp(start.min.x + MIN_SIDE, size.x);
                    }
                    if top {
                        r.min.y = (start.min.y + d.y).clamp(0.0, start.max.y - MIN_SIDE);
                    }
                    if bottom {
                        r.max.y = (start.max.y + d.y).clamp(start.min.y + MIN_SIDE, size.y);
                    }
                    r
                }
            }
        }
    }
}

/// The frame from `anchor` to `to` (the pointer), within the image; with
/// `aspect`, as large as the pointer asks with those proportions.
fn corner(anchor: Pos2, to: Pos2, size: Vec2, aspect: Option<f32>) -> Rect {
    // The way from the anchor to the pointer; at the image's edge, the
    // only way there is room.
    let way = |to: f32, anchor: f32, side: f32| {
        let s = if to < anchor { -1.0 } else { 1.0 };
        let room = if s < 0.0 { anchor } else { side - anchor };
        if room < MIN_SIDE { (-s, side - room) } else { (s, room) }
    };
    let (sx, room_x) = way(to.x, anchor.x, size.x);
    let (sy, room_y) = way(to.y, anchor.y, size.y);
    let mut w = (to.x - anchor.x).abs().clamp(MIN_SIDE, room_x);
    let mut h = (to.y - anchor.y).abs().clamp(MIN_SIDE, room_y);
    if let Some(a) = aspect {
        if w < h * a {
            w = h * a;
        } else {
            h = w / a;
        }
        if w > room_x {
            w = room_x;
            h = w / a;
        }
        if h > room_y {
            h = room_y;
            w = h * a;
        }
    }
    Rect::from_two_pos(anchor, anchor + vec2(sx * w, sy * h))
}

/// The frame `start` with its left (`left`) or right edge moved `dx`,
/// keeping `aspect`: the height follows, around the frame's centre.
fn side_edge(start: Rect, left: bool, dx: f32, size: Vec2, aspect: f32) -> Rect {
    let anchor = if left { start.max.x } else { start.min.x };
    let room_x = if left { anchor } else { size.x - anchor };
    let cy = start.center().y;
    let room_y = 2.0 * cy.min(size.y - cy);
    let edge = if left { start.min.x } else { start.max.x } + dx;
    let mut w = (if left { anchor - edge } else { edge - anchor }).clamp(MIN_SIDE, room_x.max(MIN_SIDE));
    let mut h = w / aspect;
    if h > room_y {
        h = room_y;
        w = h * aspect;
    }
    let x = if left { anchor - w } else { anchor };
    Rect::from_min_size(pos2(x, cy - h / 2.0), vec2(w, h))
}

fn transpose(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.min.y, r.min.x), pos2(r.max.y, r.max.x))
}

fn transpose_size(size: Vec2) -> Vec2 {
    vec2(size.y, size.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = vec2(400.0, 300.0);

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    fn close(a: Rect, b: Rect) -> bool {
        (a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3
    }

    #[test]
    fn grips() {
        let frame = rect(100.0, 100.0, 200.0, 200.0);
        let edges = |left, right, top, bottom| Grip::Edges { left, right, top, bottom };
        assert_eq!(grip(frame, pos2(150.0, 150.0), 8.0), Grip::Move);
        assert_eq!(grip(frame, pos2(20.0, 150.0), 8.0), Grip::New);
        assert_eq!(grip(frame, pos2(95.0, 150.0), 8.0), edges(true, false, false, false));
        assert_eq!(grip(frame, pos2(204.0, 196.0), 8.0), edges(false, true, false, true));
        assert_eq!(grip(frame, pos2(150.0, 99.0), 8.0), edges(false, false, true, false));
        // Beside the frame but past its ends: outside.
        assert_eq!(grip(frame, pos2(95.0, 300.0), 8.0), Grip::New);
        // A frame narrower than the reach: the nearer edge.
        let thin = rect(100.0, 100.0, 104.0, 200.0);
        assert_eq!(grip(thin, pos2(103.5, 150.0), 8.0), edges(false, true, false, false));
    }

    #[test]
    fn free_drags_stay_inside() {
        let start = rect(100.0, 100.0, 200.0, 200.0);
        let moved = dragged(Grip::Move, start, pos2(150.0, 150.0), pos2(500.0, 0.0), SIZE, None);
        assert!(close(moved, rect(300.0, 0.0, 400.0, 100.0)));
        let left = Grip::Edges { left: true, right: false, top: false, bottom: false };
        assert!(close(dragged(left, start, pos2(100.0, 150.0), pos2(-50.0, 0.0), SIZE, None), rect(0.0, 100.0, 200.0, 200.0)));
        // Past the opposite edge it stops a pixel before it.
        assert!(close(dragged(left, start, pos2(100.0, 150.0), pos2(390.0, 0.0), SIZE, None), rect(199.0, 100.0, 200.0, 200.0)));
        let new = dragged(Grip::New, start, pos2(300.0, 250.0), pos2(250.0, 900.0), SIZE, None);
        assert!(close(new, rect(250.0, 250.0, 300.0, 300.0)));
    }

    #[test]
    fn proportions_are_kept() {
        let start = rect(100.0, 100.0, 200.0, 200.0);
        let corner = Grip::Edges { left: false, right: true, top: false, bottom: true };
        // 2:1 from the top left corner: large enough to reach the pointer.
        let r = dragged(corner, start, pos2(200.0, 200.0), pos2(300.0, 210.0), SIZE, Some(2.0));
        assert!(close(r, rect(100.0, 100.0, 320.0, 210.0)), "{r:?}");
        // Too wide for the image: limited, still 2:1.
        let r = dragged(corner, start, pos2(200.0, 200.0), pos2(900.0, 210.0), SIZE, Some(2.0));
        assert!(close(r, rect(100.0, 100.0, 400.0, 250.0)), "{r:?}");
        // A side edge: the height follows around the centre.
        let right = Grip::Edges { left: false, right: true, top: false, bottom: false };
        let r = dragged(right, start, pos2(200.0, 150.0), pos2(260.0, 150.0), SIZE, Some(1.0));
        assert!(close(r, rect(100.0, 70.0, 260.0, 230.0)), "{r:?}");
        // The lower edge, through the transposition.
        let bottom = Grip::Edges { left: false, right: false, top: false, bottom: true };
        let r = dragged(bottom, start, pos2(150.0, 200.0), pos2(150.0, 250.0), SIZE, Some(0.5));
        assert!(close(r, rect(112.5, 100.0, 187.5, 250.0)), "{r:?}");
        // A new frame drawn up and left.
        let r = dragged(Grip::New, start, pos2(300.0, 250.0), pos2(200.0, 240.0), SIZE, Some(4.0 / 3.0));
        assert!(close(r, rect(200.0, 175.0, 300.0, 250.0)), "{r:?}");
    }

    #[test]
    fn frames_turn_with_the_image() {
        let r = rect(10.0, 20.0, 110.0, 70.0);
        let right = turn(r, SIZE, true);
        assert!(close(right, rect(230.0, 10.0, 280.0, 110.0)));
        assert!(close(turn(right, vec2(SIZE.y, SIZE.x), false), r));
        let mut crop = Crop::new(PathBuf::new(), SIZE, Zoom::Fit, Vec2::ZERO);
        crop.set_aspect(Aspect::Ratio(16, 9), SIZE);
        crop.turn(SIZE, true);
        assert_eq!(crop.aspect, Aspect::Ratio(9, 16));
        assert!((crop.rect.width() / crop.rect.height() - 9.0 / 16.0).abs() < 1e-4);
    }

    #[test]
    fn the_first_frame_is_inside_the_edges() {
        let crop = Crop::new(PathBuf::new(), SIZE, Zoom::Fit, Vec2::ZERO);
        assert!(close(crop.rect, rect(40.0, 30.0, 360.0, 270.0)));
        assert!(!is_whole(crop.rect, SIZE));
    }

    #[test]
    fn largest_frames_and_pixels() {
        let r = largest(pos2(380.0, 150.0), 1.0, SIZE);
        assert!(close(r, rect(100.0, 0.0, 400.0, 300.0)));
        assert_eq!(pixels(rect(-3.0, 0.4, 100.6, 299.9), SIZE), [0, 0, 101, 300]);
        assert_eq!(pixels(rect(399.8, 10.0, 400.0, 10.2), SIZE), [399, 10, 1, 1]);
        assert!(is_whole(whole(SIZE), SIZE));
        assert!(!is_whole(rect(1.0, 0.0, 400.0, 300.0), SIZE));
    }
}
