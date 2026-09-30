//! How the current image is shown: zoom, rotation and panning, and the
//! geometry that follows from them.
//!
//! Scales are in image pixels per physical screen pixel, so 1.0 is 100%
//! (one image pixel on one screen pixel) whatever the Windows display
//! scaling; egui works in points, hence the `ppp` (pixels per point)
//! arguments.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, TextureId, Vec2, pos2, vec2};

/// Zoom levels of Zoom In and Zoom Out.
pub const STEPS: [f32; 21] = [
    0.05, 0.10, 0.15, 0.20, 0.25, 0.33, 0.50, 0.66, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0,
    12.0, 16.0,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// Shrink to fit the window; smaller images stay at 100%.
    Fit,
    /// 100%, kept for the next image.
    Actual,
    /// Zoomed in or out by the user; the next image is fitted again.
    Scale(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: Zoom,
    /// Clockwise quarter turns, 0..=3 (display only, the file is not
    /// changed).
    pub turns: u8,
    /// Centre of the image relative to the centre of the viewport, in
    /// points.
    pub offset: Vec2,
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: Zoom::Fit, turns: 0, offset: Vec2::ZERO }
    }
}

/// The next zoom step above `scale` (`scale` itself above the last one).
pub fn step_up(scale: f32) -> f32 {
    STEPS.iter().copied().find(|&s| s > scale * 1.001).unwrap_or(scale.max(STEPS[STEPS.len() - 1]))
}

/// The next zoom step below `scale` (`scale` itself below the first one).
pub fn step_down(scale: f32) -> f32 {
    STEPS.iter().rev().copied().find(|&s| s < scale * 0.999).unwrap_or(scale.min(STEPS[0]))
}

/// Scale at which `image` (pixels) fits into `viewport` (pixels), never
/// above 1: small images are not enlarged.
pub fn fit_scale(image: Vec2, viewport: Vec2) -> f32 {
    if image.x <= 0.0 || image.y <= 0.0 {
        return 1.0;
    }
    (viewport.x / image.x).min(viewport.y / image.y).clamp(f32::MIN_POSITIVE, 1.0)
}

/// The mip level an image shown at `scale` can start from: the one nearest
/// to the shown size, so at most 1.4 times enlarged while the levels below
/// are still on their way (see `texture::Texture`). 0 at 71% and above.
pub fn mip_level(scale: f32) -> u32 {
    if scale >= 0.71 || scale <= 0.0 { 0 } else { (-scale.log2()).round() as u32 }
}

impl View {
    /// The view for the next image: rotation and panning are dropped, a
    /// zoom chosen by steps goes back to Fit.
    pub fn next_image(&mut self) {
        self.turns = 0;
        self.offset = Vec2::ZERO;
        if let Zoom::Scale(_) = self.zoom {
            self.zoom = Zoom::Fit;
        }
    }

    /// `size` (image pixels) turned by the view's rotation.
    pub fn rotated(&self, size: Vec2) -> Vec2 {
        if self.turns % 2 == 1 { vec2(size.y, size.x) } else { size }
    }

    /// The scale the image is shown at.
    pub fn scale(&self, size: Vec2, viewport: Rect, ppp: f32) -> f32 {
        match self.zoom {
            Zoom::Fit => fit_scale(self.rotated(size), viewport.size() * ppp),
            Zoom::Actual => 1.0,
            Zoom::Scale(s) => s,
        }
    }

    /// Zoom one step in (`up`) or out, keeping the image point under
    /// `anchor` (points) where it is.
    pub fn zoom_step(&mut self, up: bool, size: Vec2, viewport: Rect, ppp: f32, anchor: Pos2) {
        let old = self.scale(size, viewport, ppp);
        let new = if up { step_up(old) } else { step_down(old) };
        self.zoom_to(Zoom::Scale(new), size, viewport, ppp, anchor);
    }

    /// Switch to `zoom`, keeping the image point under `anchor` in place.
    pub fn zoom_to(&mut self, zoom: Zoom, size: Vec2, viewport: Rect, ppp: f32, anchor: Pos2) {
        let old = self.scale(size, viewport, ppp);
        self.zoom = zoom;
        let new = self.scale(size, viewport, ppp);
        let centre = viewport.center() + self.offset;
        let new_centre = anchor - (anchor - centre) * (new / old);
        self.offset = new_centre - viewport.center();
        self.clamp(size, viewport, ppp);
    }

    /// Size of the shown image in points.
    fn shown_size(&self, size: Vec2, viewport: Rect, ppp: f32) -> Vec2 {
        self.rotated(size) * self.scale(size, viewport, ppp) / ppp
    }

    /// Keep the image covering the viewport where it is larger than it,
    /// and centred where it is smaller.
    pub fn clamp(&mut self, size: Vec2, viewport: Rect, ppp: f32) {
        let spare = ((self.shown_size(size, viewport, ppp) - viewport.size()) / 2.0).max(Vec2::ZERO);
        self.offset = self.offset.clamp(-spare, spare);
    }

    /// Whether the image is larger than the viewport across (x) and down
    /// (y), so that it can be panned that way.
    pub fn pannable(&self, size: Vec2, viewport: Rect, ppp: f32) -> [bool; 2] {
        let shown = self.shown_size(size, viewport, ppp);
        // Half a pixel of slack against rounding.
        let slack = 0.5 / ppp;
        [shown.x > viewport.width() + slack, shown.y > viewport.height() + slack]
    }

    /// Move the image by `delta` points.
    pub fn pan(&mut self, delta: Vec2, size: Vec2, viewport: Rect, ppp: f32) {
        self.offset += delta;
        self.clamp(size, viewport, ppp);
    }

    /// Where the (rotated) image goes on screen, in points; its corner is
    /// on a whole screen pixel so that 100% maps pixel to pixel.
    pub fn place(&mut self, size: Vec2, viewport: Rect, ppp: f32) -> Rect {
        self.clamp(size, viewport, ppp);
        let shown = self.shown_size(size, viewport, ppp);
        let min = viewport.center() + self.offset - shown / 2.0;
        let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
        Rect::from_min_size(min, shown)
    }
}

/// Paint texture `texture` into `rect` turned by `turns` clockwise quarter
/// turns. The corners are fixed and the texture coordinates rotate, which
/// is exact (no trigonometry).
pub fn paint(painter: &Painter, texture: TextureId, rect: Rect, turns: u8) {
    const UV: [Pos2; 4] = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let turns = (turns % 4) as usize;
    let mut mesh = Mesh::with_texture(texture);
    for (i, pos) in corners.into_iter().enumerate() {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv: UV[(i + 4 - turns) % 4], color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_level_nearest_to_the_shown_size() {
        assert_eq!(mip_level(1.0), 0);
        assert_eq!(mip_level(0.75), 0);
        assert_eq!(mip_level(0.71), 0);
        assert_eq!(mip_level(0.7), 1);
        assert_eq!(mip_level(0.5), 1);
        assert_eq!(mip_level(0.36), 1);
        assert_eq!(mip_level(0.3), 2);
        assert_eq!(mip_level(0.05), 4);
        assert_eq!(mip_level(0.0), 0);
    }

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(0.0, 30.0), vec2(1000.0, 600.0))
    }

    #[test]
    fn steps() {
        assert_eq!(step_up(1.0), 1.25);
        assert_eq!(step_down(1.0), 0.75);
        assert_eq!(step_up(0.4), 0.5);
        assert_eq!(step_down(0.4), 0.33);
        // Past either end the scale stays.
        assert_eq!(step_up(16.0), 16.0);
        assert_eq!(step_up(20.0), 20.0);
        assert_eq!(step_down(0.03), 0.03);
    }

    #[test]
    fn fit_only_shrinks() {
        assert_eq!(fit_scale(vec2(800.0, 532.0), vec2(1920.0, 1000.0)), 1.0);
        assert_eq!(fit_scale(vec2(4000.0, 3000.0), vec2(1000.0, 600.0)), 0.2);
        assert_eq!(fit_scale(vec2(0.0, 0.0), vec2(1000.0, 600.0)), 1.0);
    }

    #[test]
    fn fit_uses_the_rotated_size() {
        let mut v = View::default();
        let size = vec2(3000.0, 1000.0);
        assert!((v.scale(size, viewport(), 1.0) - 1.0 / 3.0).abs() < 1e-6);
        v.turns = 1;
        assert!((v.scale(size, viewport(), 1.0) - 0.2).abs() < 1e-6);
        // At 150% display scaling the viewport has more pixels.
        v.turns = 0;
        assert!((v.scale(size, viewport(), 1.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn small_images_are_centred_and_pixel_aligned() {
        let mut v = View::default();
        let rect = v.place(vec2(801.0, 533.0), viewport(), 1.0);
        assert_eq!(rect.size(), vec2(801.0, 533.0));
        assert_eq!(rect.min, pos2(100.0, 64.0)); // 99.5 and 63.5 rounded
        // 125%: the corner lands on a whole pixel, the size is 1:1 in pixels.
        let rect = v.place(vec2(801.0, 533.0), viewport(), 1.25);
        assert_eq!(((rect.min.x * 1.25).fract(), (rect.min.y * 1.25).fract()), (0.0, 0.0));
        assert!((rect.width() * 1.25 - 801.0).abs() < 1e-3);
    }

    #[test]
    fn panning_is_limited_to_the_edges() {
        let mut v = View { zoom: Zoom::Actual, ..View::default() };
        let size = vec2(2000.0, 500.0);
        assert_eq!(v.pannable(size, viewport(), 1.0), [true, false]);
        v.pan(vec2(10_000.0, 50.0), size, viewport(), 1.0);
        // 500 px of spare width each side; the short side stays centred.
        assert_eq!(v.offset, vec2(500.0, 0.0));
        let rect = v.place(size, viewport(), 1.0);
        assert_eq!(rect.left(), 0.0);
    }

    #[test]
    fn zoom_keeps_the_anchor_in_place() {
        let mut v = View { zoom: Zoom::Actual, ..View::default() };
        let size = vec2(4000.0, 3000.0);
        let anchor = pos2(700.0, 200.0);
        let before = v.place(size, viewport(), 1.0);
        let image_point = (anchor - before.min) / before.width();
        v.zoom_step(true, size, viewport(), 1.0, anchor);
        assert_eq!(v.zoom, Zoom::Scale(1.25));
        let after = v.place(size, viewport(), 1.0);
        let moved = after.min + image_point * after.width() - anchor;
        assert!(moved.length() < 1.0, "{moved:?}");
    }

    #[test]
    fn next_image_resets_all_but_fixed_zooms() {
        let mut v = View { zoom: Zoom::Scale(2.0), turns: 3, offset: vec2(5.0, 5.0) };
        v.next_image();
        assert_eq!(v, View::default());
        let mut v = View { zoom: Zoom::Actual, turns: 1, offset: vec2(5.0, 5.0) };
        v.next_image();
        assert_eq!(v.zoom, Zoom::Actual);
    }
}
