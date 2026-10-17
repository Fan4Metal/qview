//! The part of an enlarged drawing (`svg`) that is on screen, drawn anew at
//! the screen's resolution.
//!
//! A drawing's texture is drawn large enough to fill the screen
//! (`svg::view_scale`); zoomed in further, its pixels would show. Then the
//! visible part, with a margin for panning, is drawn on a thread at one
//! pixel per screen pixel ([`Part`]) and painted over the image. A part
//! drawn for another zoom is painted, scaled, until the new one comes. One
//! drawing at a time: what is wanted meanwhile is drawn next, the latest
//! only. The parsed drawing is kept for the next parts.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use egui::{Rect, pos2};

use crate::loader::Pixels;
use crate::svg::Svg;
use crate::texture::Texture;
use crate::view;

/// Drawn around the visible part, on each side, as a share of the area's
/// size: panning that far needs no new part.
const MARGIN: f32 = 0.125;

/// Shown larger than its texture by more than this, a drawing gets a part.
const ENLARGED: f32 = 1.01;

/// A part to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    pub path: PathBuf,
    /// Of the drawing as stored (not turned or mirrored), 0 to 1 across
    /// and down it.
    pub uv: Rect,
    /// Its pixels, as stored.
    pub size: [u32; 2],
}

impl Wanted {
    /// Where the part is on screen, for the image shown at `place`.
    pub fn rect(&self, place: Rect, turns: u8, flip: bool) -> Rect {
        let at = |uv| place.min + view::shown_at(uv, turns, flip).to_vec2() * place.size();
        Rect::from_two_pos(at(self.uv.min), at(self.uv.max))
    }

    /// Whether this part has the scale of `other` and holds `visible` (on
    /// the drawing, as `uv`).
    fn covers(&self, other: &Wanted, visible: Rect) -> bool {
        let scale = |w: &Wanted| w.size[0] as f32 / w.uv.width();
        self.path == other.path
            && (scale(self) / scale(other) - 1.0).abs() < 1e-3
            && self.uv.expand(1e-6).contains_rect(visible)
    }
}

/// What of the drawing `path` shown at `place` (its texture of `texture`
/// pixels) is to be drawn anew for the image area `area`, with the part of
/// it that is visible (on the drawing, as `Wanted::uv`); None when the
/// texture is enough. The part is in whole screen pixels from the image's
/// corner, which `View::place` puts on a whole pixel, so that it is painted
/// one pixel to one.
pub fn wanted(
    path: &Path,
    texture: (u32, u32),
    place: Rect,
    area: Rect,
    ppp: f32,
    turns: u8,
    flip: bool,
) -> Option<(Wanted, Rect)> {
    // The image's size on screen in pixels, as shown and as stored.
    let (w, h) = (place.width() * ppp, place.height() * ppp);
    let (sw, sh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    if sw <= texture.0 as f32 * ENLARGED && sh <= texture.1 as f32 * ENLARGED {
        return None;
    }
    let visible = place.intersect(area);
    if !(visible.width() > 0.0 && visible.height() > 0.0) {
        return None;
    }
    let margin = visible.expand2(area.size() * MARGIN).intersect(place);
    // In whole pixels from the image's corner.
    let pixels = |r: Rect| {
        let at = |p: egui::Pos2, round: fn(f32) -> f32| {
            let p = (p - place.min) * ppp;
            pos2(round(p.x).clamp(0.0, w.ceil()), round(p.y).clamp(0.0, h.ceil()))
        };
        Rect::from_min_max(at(r.min, f32::floor), at(r.max, f32::ceil))
    };
    let on_drawing = |r: Rect| {
        let uv = |p: egui::Pos2| view::uv_at(pos2(p.x / w, p.y / h), turns, flip);
        Rect::from_two_pos(uv(r.min), uv(r.max))
    };
    let part = pixels(margin);
    let (pw, ph) = (part.width() as u32, part.height() as u32);
    if pw == 0 || ph == 0 {
        return None;
    }
    let size = if turns % 2 == 1 { [ph, pw] } else { [pw, ph] };
    Some((Wanted { path: path.to_path_buf(), uv: on_drawing(part), size }, on_drawing(pixels(visible))))
}

/// A part drawn, on the GPU.
pub struct Part {
    pub drawn: Wanted,
    pub texture: Texture,
}

/// What a drawing thread sends back.
struct Drawn {
    wanted: Wanted,
    svg: Option<Arc<Svg>>,
    /// Premultiplied BGRA.
    pixels: Result<Vec<u8>, String>,
}

#[derive(Default)]
pub struct Detail {
    part: Option<Part>,
    /// The drawing parsed, for the next parts.
    svg: Option<(PathBuf, Arc<Svg>)>,
    drawing: Option<(Wanted, mpsc::Receiver<Drawn>)>,
    /// Wanted while another part is drawn.
    next: Option<Wanted>,
    /// A drawing that could not be drawn: not tried again.
    failed: Option<PathBuf>,
}

impl Detail {
    /// Draw `want` (from [`wanted`]) unless a part drawn or being drawn
    /// covers it; None drops the part.
    pub fn want(&mut self, ctx: &egui::Context, want: Option<(Wanted, Rect)>) {
        let Some((wanted, visible)) = want else {
            self.part = None;
            self.next = None;
            return;
        };
        if self.part.as_ref().is_some_and(|p| p.drawn.path != wanted.path) {
            self.part = None;
        }
        if self.covered(&wanted, visible) || self.failed.as_ref() == Some(&wanted.path) {
            self.next = None;
            return;
        }
        match &self.drawing {
            Some((drawing, _)) if drawing.covers(&wanted, visible) => self.next = None,
            Some(_) => self.next = Some(wanted),
            None => self.start(ctx, wanted),
        }
    }

    /// Whether the part drawn covers `wanted`'s visible part `visible` at
    /// its scale: the image under it need not be painted.
    pub fn covered(&self, wanted: &Wanted, visible: Rect) -> bool {
        self.part.as_ref().is_some_and(|p| p.drawn.covers(wanted, visible))
    }

    /// The part drawn of `path`, of whatever scale.
    pub fn part(&self, path: &Path) -> Option<&Part> {
        self.part.as_ref().filter(|p| p.drawn.path == path)
    }

    /// `paths` changed on disk: nothing drawn of them is kept.
    pub fn forget(&mut self, paths: &[PathBuf]) {
        if self.part.as_ref().is_some_and(|p| paths.contains(&p.drawn.path)) {
            self.part = None;
        }
        if self.svg.as_ref().is_some_and(|(p, _)| paths.contains(p)) {
            self.svg = None;
        }
        if self.failed.as_ref().is_some_and(|p| paths.contains(p)) {
            self.failed = None;
        }
    }

    fn start(&mut self, ctx: &egui::Context, wanted: Wanted) {
        let svg = self.svg.as_ref().filter(|(p, _)| *p == wanted.path).map(|(_, s)| s.clone());
        let (tx, rx) = mpsc::channel();
        let (ctx, w) = (ctx.clone(), wanted.clone());
        let spawned = std::thread::Builder::new().name("drawing part".into()).spawn(move || {
            let started = Instant::now();
            let svg = match svg {
                Some(svg) => Ok(svg),
                None => crate::archive::read(&w.path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| Svg::parse(&w.path, &bytes, true))
                    .map(Arc::new),
            };
            let pixels = svg.clone().and_then(|svg| {
                let uv = [w.uv.min.x, w.uv.min.y, w.uv.max.x, w.uv.max.y];
                let mut rgba = svg.render_part(uv, w.size[0], w.size[1])?;
                for p in rgba.as_chunks_mut::<4>().0 {
                    p.swap(0, 2);
                }
                Ok(rgba)
            });
            log::debug!(
                "drew {}x{} of {} in {:.1} ms",
                w.size[0],
                w.size[1],
                w.path.display(),
                started.elapsed().as_secs_f64() * 1e3
            );
            let _ = tx.send(Drawn { wanted: w, svg: svg.ok(), pixels });
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.drawing = Some((wanted, rx)),
            Err(e) => log::warn!("cannot start drawing: {e}"),
        }
    }

    /// Take a part drawn and upload it (on the UI thread, where `frame`
    /// registers textures); then draw what was wanted meanwhile.
    pub fn poll(&mut self, ctx: &egui::Context, gl: &Arc<glow::Context>, frame: &mut eframe::Frame) {
        let Some((_, rx)) = &self.drawing else { return };
        let drawn = match rx.try_recv() {
            Ok(drawn) => drawn,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.drawing = None;
                return;
            }
        };
        self.drawing = None;
        let Drawn { wanted, svg, pixels } = drawn;
        if let Some(svg) = svg {
            self.svg = Some((wanted.path.clone(), svg));
        }
        match pixels {
            Ok(bgra) => {
                let pixels = Pixels { width: wanted.size[0], height: wanted.size[1], levels: vec![bgra] };
                match Texture::new(gl.clone(), &pixels, 0, |t| frame.register_native_glow_texture(t)) {
                    Ok(texture) => self.part = Some(Part { drawn: wanted, texture }),
                    Err(e) => log::warn!("cannot show a part of {}: {e}", wanted.path.display()),
                }
            }
            Err(e) => {
                log::warn!("cannot draw a part of {}: {e}", wanted.path.display());
                self.failed = Some(wanted.path);
            }
        }
        if let Some(next) = self.next.take() {
            self.start(ctx, next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn parts_of_an_enlarged_drawing() {
        let path = Path::new("a.svg");
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
        // Fitted, the texture is enough.
        let place = Rect::from_min_size(pos2(250.0, 0.0), vec2(500.0, 500.0));
        assert!(wanted(path, (1000, 1000), place, area, 1.0, 0, false).is_none());
        // Enlarged 4x past its texture of 1000: the area with a margin
        // inside, in whole pixels, the scale of the texture, 4000 wide.
        let place = Rect::from_min_size(pos2(-1500.0, -1750.0), vec2(4000.0, 4000.0));
        let (w, visible) = wanted(path, (1000, 1000), place, area, 1.0, 0, false).unwrap();
        assert_eq!(w.size, [1250, 626]);
        let near = |a: Rect, b: Rect| (a.min - b.min).length() < 1e-5 && (a.max - b.max).length() < 1e-5;
        assert!(near(w.uv, Rect::from_min_max(pos2(1375.0 / 4000.0, 1687.5f32.floor() / 4000.0), pos2(2625.0 / 4000.0, 2313.0 / 4000.0))), "{:?}", w.uv);
        assert!(near(visible, Rect::from_min_max(pos2(0.375, 0.4375), pos2(0.625, 0.5625))));
        assert!(w.covers(&w, visible));
        // Panned within the margin, it still covers; zoomed, it does not.
        let panned = place.translate(vec2(50.0, 0.0));
        let (w2, visible2) = wanted(path, (1000, 1000), panned, area, 1.0, 0, false).unwrap();
        assert!(w.covers(&w2, visible2));
        let zoomed = Rect::from_center_size(place.center(), place.size() * 1.25);
        let (w3, visible3) = wanted(path, (1000, 1000), zoomed, area, 1.0, 0, false).unwrap();
        assert!(!w.covers(&w3, visible3));
        // Turned a quarter: the part is stored the other way up.
        let wide = Rect::from_min_size(pos2(-1500.0, -1750.0), vec2(4000.0, 2000.0));
        let (t, _) = wanted(path, (1000, 2000), wide, area, 1.0, 1, false).unwrap();
        assert_eq!(t.size, [313, 1250]);
        // Off the area: nothing.
        let away = Rect::from_min_size(pos2(2000.0, 0.0), vec2(4000.0, 4000.0));
        assert!(wanted(path, (1000, 1000), away, area, 1.0, 0, false).is_none());
    }

    #[test]
    fn a_part_is_painted_where_it_was_drawn() {
        let place = Rect::from_min_size(pos2(-100.0, -50.0), vec2(800.0, 400.0));
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0));
        for turns in 0..4 {
            for flip in [false, true] {
                let (w, _) = wanted(Path::new("a.svg"), (10, 10), place, area, 1.0, turns, flip).unwrap();
                let margin = area.expand2(area.size() * MARGIN).intersect(place);
                let rect = w.rect(place, turns, flip);
                assert!((rect.min - margin.min.floor()).length() < 1e-3, "{turns} {flip}: {rect:?}");
                assert!((rect.max - margin.max.ceil()).length() < 1e-3, "{turns} {flip}: {rect:?}");
            }
        }
    }
}
