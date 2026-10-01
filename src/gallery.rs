//! The gallery: the folder tree on the left and the images of the folder
//! as a grid of thumbnails. It shares the folder listing and the current
//! file with the viewer (`App::files`, `App::current`): the selected cell
//! is the current image, so the keys that act on the image (Delete,
//! Ctrl+C, ...) act on it, and leaving the gallery shows it.
//!
//! Thumbnails are made on threads (`thumbs`) for the visible cells first,
//! then a screen ahead; their textures stay within `BUDGET`, the least
//! recently seen dropped first, so going back to a folder is instant. The
//! drawing is in `ui::gallery`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Vec2;

use crate::texture::Texture;
use crate::thumbs::{Made, Request, Thumbs};
use crate::tree::Tree;

/// Cell sizes (the thumbnail's square, in points) of the slider's range.
pub const MIN_SIZE: f32 = 64.0;
pub const MAX_SIZE: f32 = 512.0;
pub const DEFAULT_SIZE: f32 = 160.0;
/// Sizes `+` and `-` step through.
const SIZE_STEPS: [f32; 12] = [64.0, 80.0, 96.0, 128.0, 160.0, 192.0, 224.0, 256.0, 320.0, 384.0, 448.0, 512.0];
/// Pixel sizes thumbnails are made at: those Windows keeps in its
/// thumbnail cache, so that they come from it without scaling.
const SIDES: [u32; 4] = [96, 256, 768, 1280];
/// Pixels of the thumbnail textures kept: about 1000 thumbnails of 256
/// pixels, 250 MB of texture memory with the mip levels.
const BUDGET: usize = 48_000_000;
/// Time of a frame spent uploading thumbnails at most (at least one is
/// uploaded).
const UPLOAD_TIME: Duration = Duration::from_millis(4);
/// The image under the pointer is decoded ahead after this, so that a
/// click shows it at once.
pub const HOVER_DELAY: Duration = Duration::from_millis(150);

/// Space around a thumbnail in its cell, and the height of the name.
pub const PAD: f32 = 6.0;
pub const LABEL: f32 = 18.0;

/// A thumbnail on the GPU, or the note that there is none.
pub struct Thumb {
    /// `None`: the file could not be read.
    pub texture: Option<Rc<Texture>>,
    /// Texture size in pixels.
    pub px: [u32; 2],
    /// The side it was made for (see `SIDES`).
    side: u32,
    /// Size of the image, 0 when unknown.
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub modified: u64,
    /// Frame it was last drawn in.
    used: u64,
}

/// Where to scroll the grid to on the next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// The current image in the middle (on entering the gallery).
    Centre,
    /// The current image just into view (after a key).
    Visible,
}

pub struct Gallery {
    pub tree: Tree,
    thumbs: Thumbs,
    pub cache: HashMap<PathBuf, Thumb>,
    /// Made, waiting for their textures.
    made: VecDeque<Made>,
    /// Pixels of the textures in `cache`.
    pixels: usize,
    /// Columns and whole rows of the grid in the last frame, for the keys.
    pub columns: usize,
    pub page_rows: usize,
    pub scroll: Option<Scroll>,
    /// Scroll offset of the grid in the last frame.
    pub top: f32,
    /// The current file when the grid last scrolled to it.
    pub scrolled_to: Option<PathBuf>,
    /// The cell under the pointer, and since when.
    pub hovered: Option<(PathBuf, Instant)>,
    /// Frames polled, for `Thumb::used`.
    pub frame: u64,
}

impl Gallery {
    pub fn new(ctx: &egui::Context) -> Self {
        let workers = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(2, 4));
        Self {
            tree: Tree::new(ctx.clone()),
            thumbs: Thumbs::new(workers, ctx.clone()),
            cache: HashMap::new(),
            made: VecDeque::new(),
            pixels: 0,
            columns: 1,
            page_rows: 1,
            scroll: None,
            top: 0.0,
            scrolled_to: None,
            hovered: None,
            frame: 0,
        }
    }

    /// Upload the thumbnails made since the last frame, for `UPLOAD_TIME`
    /// at most; the rest wait for the next frames.
    pub fn poll(&mut self, gl: &Arc<glow::Context>, frame: &mut eframe::Frame, ctx: &egui::Context) {
        self.frame += 1;
        self.made.extend(std::iter::from_fn(|| self.thumbs.poll()));
        let started = Instant::now();
        let mut uploaded = 0;
        while let Some(made) = self.made.pop_front() {
            if uploaded > 0 && started.elapsed() >= UPLOAD_TIME {
                self.made.push_front(made);
                break;
            }
            // Made for smaller cells after a larger one (cells shrunk back).
            if self.cache.get(&made.request.path).is_some_and(|t| t.texture.is_some() && t.side >= made.request.side) {
                continue;
            }
            uploaded += 1;
            let thumb = match made.result {
                Ok(t) => {
                    let texture = match Texture::new(gl.clone(), &t.pixels, 0, |n| frame.register_native_glow_texture(n)) {
                        Ok(texture) => Some(Rc::new(texture)),
                        Err(e) => {
                            log::warn!("cannot show the thumbnail of {}: {e}", made.request.path.display());
                            None
                        }
                    };
                    Thumb {
                        texture,
                        px: [t.pixels.width, t.pixels.height],
                        side: made.request.side,
                        width: t.width,
                        height: t.height,
                        file_size: t.file_size,
                        modified: t.modified,
                        used: self.frame,
                    }
                }
                Err(e) => {
                    log::debug!("no thumbnail of {}: {e}", made.request.path.display());
                    Thumb { texture: None, px: [0, 0], side: u32::MAX, width: 0, height: 0, file_size: 0, modified: 0, used: self.frame }
                }
            };
            self.pixels += texture_pixels(&thumb);
            if let Some(old) = self.cache.insert(made.request.path, thumb) {
                self.pixels -= texture_pixels(&old);
            }
        }
        if !self.made.is_empty() {
            ctx.request_repaint();
        }
        self.evict();
    }

    /// Drop the thumbnails seen longest ago while over `BUDGET`, down to
    /// three quarters of it; those drawn in this frame stay.
    fn evict(&mut self) {
        if self.pixels <= BUDGET {
            return;
        }
        let mut by_age: Vec<(u64, PathBuf)> =
            self.cache.iter().filter(|(_, t)| t.used < self.frame).map(|(p, t)| (t.used, p.clone())).collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        for (_, path) in by_age {
            if self.pixels <= BUDGET / 4 * 3 {
                break;
            }
            if let Some(t) = self.cache.remove(&path) {
                self.pixels -= texture_pixels(&t);
            }
        }
    }

    /// The thumbnail of `path`, marked as seen in this frame.
    pub fn thumb(&mut self, path: &Path) -> Option<&Thumb> {
        let frame = self.frame;
        let t = self.cache.get_mut(path)?;
        t.used = frame;
        Some(t)
    }

    /// Whether `path` has no thumbnail made for `side` pixels or more.
    pub fn needs(&self, path: &Path, side: u32) -> bool {
        self.cache.get(path).is_none_or(|t| t.side < side)
    }

    /// Make these thumbnails, most wanted first, instead of those wanted
    /// before.
    pub fn want(&self, requests: Vec<Request>) {
        self.thumbs.want(requests);
    }

    /// Forget the thumbnails of `paths` (the folder is read again).
    pub fn forget(&mut self, paths: &[PathBuf]) {
        for p in paths {
            if let Some(t) = self.cache.remove(p) {
                self.pixels -= texture_pixels(&t);
            }
        }
    }

    /// The textures are deleted now, while the GL context lives.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.made.clear();
        self.pixels = 0;
    }
}

/// Pixels of `t`'s texture, its mip levels included.
fn texture_pixels(t: &Thumb) -> usize {
    if t.texture.is_none() { 0 } else { t.px[0] as usize * t.px[1] as usize * 4 / 3 }
}

/// The pixel size to make thumbnails at for cells of `px` pixels.
pub fn side_for(px: f32) -> u32 {
    SIDES.iter().copied().find(|&s| s as f32 >= px).unwrap_or(SIDES[SIDES.len() - 1])
}

/// The next cell size up or down from `size`.
pub fn step_size(size: f32, up: bool) -> f32 {
    let next = if up {
        SIZE_STEPS.iter().copied().find(|&s| s > size + 0.5)
    } else {
        SIZE_STEPS.iter().rev().copied().find(|&s| s < size - 0.5)
    };
    next.unwrap_or(size).clamp(MIN_SIZE, MAX_SIZE)
}

/// The grid of `count` cells of thumbnails `size` points wide in a list
/// `width` points wide: as many columns as fit, stretched to fill the
/// width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub columns: usize,
    pub rows: usize,
    pub cell: Vec2,
}

pub fn grid(width: f32, size: f32, count: usize) -> Grid {
    let min = size + 2.0 * PAD;
    let columns = ((width / min).floor() as usize).max(1);
    Grid { columns, rows: count.div_ceil(columns), cell: egui::vec2(width / columns as f32, size + 2.0 * PAD + LABEL) }
}

/// The cell `rows` rows below `i` (above for negative), in the same
/// column, stopping at the first and last rows; in a last row too short
/// for that column, the last cell.
pub fn move_rows(i: usize, rows: isize, columns: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let last_row = ((count - 1) / columns) as isize;
    let row = ((i / columns) as isize + rows).clamp(0, last_row) as usize;
    (row * columns + i % columns).min(count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fills_the_width() {
        let g = grid(1000.0, 160.0, 69);
        // 172 points a cell at least: 5 fit, stretched to 200.
        assert_eq!((g.columns, g.rows), (5, 14));
        assert_eq!(g.cell, egui::vec2(200.0, 160.0 + 2.0 * PAD + LABEL));
        // Narrower than one cell: still one column.
        assert_eq!(grid(50.0, 160.0, 3).columns, 1);
        assert_eq!(grid(1000.0, 160.0, 0).rows, 0);
    }

    #[test]
    fn rows_keep_the_column() {
        // 10 cells in rows of 4: 0-3, 4-7, 8-9.
        assert_eq!(move_rows(1, 1, 4, 10), 5);
        assert_eq!(move_rows(5, -1, 4, 10), 1);
        assert_eq!(move_rows(1, -1, 4, 10), 1);
        // Under 6 there is no cell: the last one.
        assert_eq!(move_rows(6, 1, 4, 10), 9);
        assert_eq!(move_rows(9, 1, 4, 10), 9);
        // A page past either end stops at the first or last row.
        assert_eq!(move_rows(5, 10, 4, 10), 9);
        assert_eq!(move_rows(4, 10, 4, 10), 8);
        assert_eq!(move_rows(9, -10, 4, 10), 1);
    }

    #[test]
    fn sizes() {
        assert_eq!(side_for(64.0), 96);
        assert_eq!(side_for(200.0), 256);
        assert_eq!(side_for(257.0), 768);
        assert_eq!(side_for(769.0), 1280);
        assert_eq!(side_for(5000.0), 1280);
        assert_eq!(step_size(160.0, true), 192.0);
        assert_eq!(step_size(150.0, false), 128.0);
        assert_eq!(step_size(320.0, true), 384.0);
        assert_eq!(step_size(512.0, true), 512.0);
        assert_eq!(step_size(64.0, false), 64.0);
    }
}
