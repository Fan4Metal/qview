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
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use egui::{Rect, Vec2, pos2};

use crate::texture::Texture;
use crate::thumbs::{Made, Request, Thumbs};
use crate::tree::Tree;

/// Proportions of the cells' frames (width over height) offered above the
/// grid, by name (also as kept in the settings).
pub const ASPECTS: [(&str, f32); 7] = [
    ("1:1", 1.0),
    ("4:3", 4.0 / 3.0),
    ("3:2", 3.0 / 2.0),
    ("16:9", 16.0 / 9.0),
    ("3:4", 3.0 / 4.0),
    ("2:3", 2.0 / 3.0),
    ("9:16", 9.0 / 16.0),
];
/// Images of a folder whose proportions decide the Auto cells: all of a
/// smaller folder, this many spread over a larger one.
const AUTO_SAMPLE: usize = 200;
/// Cell sizes (the long side of the thumbnail's frame, in points) of the
/// slider's range.
pub const MIN_SIZE: f32 = 64.0;
pub const MAX_SIZE: f32 = 512.0;
pub const DEFAULT_SIZE: f32 = 160.0;
/// Sizes `+` and `-` step through.
const SIZE_STEPS: [f32; 12] = [64.0, 80.0, 96.0, 128.0, 160.0, 192.0, 224.0, 256.0, 320.0, 384.0, 448.0, 512.0];
/// Pixel sizes thumbnails are made at: those Windows keeps in its
/// thumbnail cache, so that they come from it without scaling.
const SIDES: [u32; 4] = [96, 256, 768, 1280];
/// How much a thumbnail may be enlarged before a larger one is made (see
/// `side_needed`).
const ENLARGE: f32 = 1.5;
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
/// Height of a folder's header when the images of the sub-folders are
/// shown by folder.
pub const HEADER: f32 = 30.0;

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// The current image in the middle (on entering the gallery).
    Centre,
    /// The current image just into view (after a key).
    Visible,
    /// The current image's row this many points below the top of the
    /// grid, where it was before the cells changed shape.
    Keep(f32),
}

pub struct Gallery {
    pub tree: Tree,
    thumbs: Thumbs,
    pub cache: HashMap<PathBuf, Thumb>,
    /// Made, waiting for their textures.
    made: VecDeque<Made>,
    /// Pixels of the textures in `cache`.
    pixels: usize,
    /// The grid and its whole rows on screen in the last frame, for the
    /// keys.
    pub layout: Layout,
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
    /// Proportions of the cells in the last frame: kept in Auto mode while
    /// a folder's are being found.
    pub shown_aspect: f32,
    /// The Auto proportions of the folders seen (see `auto_aspect`).
    auto: HashMap<PathBuf, f32>,
    finding: Option<Finding>,
    ctx: egui::Context,
}

/// The proportions of a folder's images being found on a thread; it stops
/// when this is dropped.
struct Finding {
    dir: PathBuf,
    rx: mpsc::Receiver<Option<f32>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Finding {
    fn drop(&mut self) {
        self.cancel.store(true, Relaxed);
    }
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
            layout: Layout::default(),
            page_rows: 1,
            scroll: None,
            top: 0.0,
            scrolled_to: None,
            hovered: None,
            frame: 0,
            shown_aspect: 1.0,
            auto: HashMap::new(),
            finding: None,
            ctx: ctx.clone(),
        }
    }

    /// The cells' proportions in Auto mode for folder `dir` listing
    /// `files`: those most of its images have (see `common_aspect`), read
    /// from their headers on a thread. `None` until found.
    pub fn auto_aspect(&mut self, dir: &Path, files: &[PathBuf]) -> Option<f32> {
        if let Some(&aspect) = self.auto.get(dir) {
            return Some(aspect);
        }
        if let Some(finding) = self.finding.as_ref().filter(|f| f.dir == dir) {
            match finding.rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return None,
                found => {
                    // No image could be read: square cells.
                    let aspect = found.ok().flatten().unwrap_or(1.0);
                    self.auto.insert(dir.to_path_buf(), aspect);
                    self.finding = None;
                    return Some(aspect);
                }
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (sample, stop, ctx) = (sample(files), cancel.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new().name("cell proportions".into()).spawn(move || {
            let started = Instant::now();
            let mut ratios = Vec::with_capacity(sample.len());
            for path in &sample {
                if stop.load(Relaxed) {
                    return;
                }
                if let Some((w, h)) = crate::header::read(path).map(crate::header::Size::upright) {
                    ratios.push(w as f32 / h.max(1) as f32);
                }
            }
            let aspect = common_aspect(ratios);
            log::debug!(
                "cell proportions {aspect:?} from {} images in {:.0} ms",
                sample.len(),
                started.elapsed().as_secs_f64() * 1e3
            );
            if tx.send(aspect).is_ok() {
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("cannot start a thread for the cell proportions: {e}");
            self.auto.insert(dir.to_path_buf(), 1.0);
            return Some(1.0);
        }
        self.finding = Some(Finding { dir: dir.to_path_buf(), rx, cancel });
        None
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
    /// three quarters of it. Those drawn in the last frame stay: this runs
    /// in `poll`, before the grid is drawn, so they are the visible ones.
    fn evict(&mut self) {
        if self.pixels <= BUDGET {
            return;
        }
        let frame = self.frame;
        let mut by_age: Vec<(u64, PathBuf)> =
            self.cache.iter().filter(|(_, t)| t.used + 1 < frame).map(|(p, t)| (t.used, p.clone())).collect();
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

    /// Forget the thumbnails of `paths` (the folder is read again), and
    /// any made or being made before now.
    pub fn forget(&mut self, paths: &[PathBuf]) {
        self.thumbs.forget();
        self.made.clear();
        // The images may have changed too.
        self.auto.clear();
        self.finding = None;
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

impl Thumb {
    /// Width over height, once there is a texture.
    pub fn ratio(&self) -> Option<f32> {
        let [w, h] = self.px.map(|v| v.max(1) as f32);
        self.texture.as_ref().map(|_| w / h)
    }

    /// The thumbnail is smaller than the image (or the image's size is
    /// not known).
    pub fn shrunk(&self) -> bool {
        self.width == 0 || self.width > self.px[0]
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

/// The frame of a thumbnail `size` points on its long side, with the
/// proportions `aspect` (width over height).
pub fn frame_size(size: f32, aspect: f32) -> Vec2 {
    if aspect >= 1.0 { egui::vec2(size, size / aspect) } else { egui::vec2(size * aspect, size) }
}

/// The pixel size to make thumbnails at for frames of `frame` pixels.
/// When the frames are filled (`fill`), the thumbnail has to cover the
/// frame both ways: its long side follows from its proportions `ratio`
/// (width over height; a landscape 3:2 photo's until the thumbnail shows).
/// A thumbnail may be enlarged up to `ENLARGE` times: the next size, 768,
/// is rarely in Windows' cache, and it is made anew (~30 ms) and nine
/// times the pixels of 256.
pub fn side_needed(frame: Vec2, fill: bool, ratio: Option<f32>) -> u32 {
    let long = if fill {
        let r = ratio.unwrap_or(1.5).clamp(0.25, 4.0);
        if r >= 1.0 { frame.x.max(frame.y * r) } else { frame.y.max(frame.x / r) }
    } else {
        frame.max_elem()
    };
    side_for(long / ENLARGE)
}

/// How many cells' thumbnails of `side` pixels fit into the part of
/// `BUDGET` eviction keeps, counting each as square: the grid asks for no
/// more ahead of and behind the visible ones, or what it made would be
/// evicted and made again.
pub fn cells_in_budget(side: u32) -> usize {
    BUDGET / 4 * 3 / (side as usize * side as usize * 4 / 3).max(1)
}

/// Where a thumbnail of `px` pixels goes in `frame` (points) and the part
/// of it shown there, as `(rect, uv)`: fitted whole, or, with `fill`,
/// covering the frame with what sticks out cropped. Enlarged only while
/// the thumbnail is smaller than the image (`shrunk`): a thumbnail of the
/// image itself is not, so a small image keeps its size. On whole pixels.
pub fn place_thumb(frame: Rect, px: Vec2, ppp: f32, fill: bool, shrunk: bool) -> (Rect, Rect) {
    let snap = |p: egui::Pos2| pos2((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp);
    let frame = Rect::from_min_max(snap(frame.min), snap(frame.max));
    let room = frame.size() * ppp;
    let (fit, cover) = ((room.x / px.x).min(room.y / px.y), (room.x / px.x).max(room.y / px.y));
    let scale = if fill { cover } else { fit };
    let scale = if shrunk { scale } else { scale.min(1.0) };
    let shown = px * scale / ppp;
    let image = Rect::from_min_size(snap(frame.center() - shown / 2.0), shown);
    let rect = image.intersect(frame);
    let uv = Rect::from_min_max(
        ((rect.min - image.min) / image.size()).to_pos2(),
        ((rect.max - image.min) / image.size()).to_pos2(),
    );
    (rect, uv)
}

/// The files whose proportions decide the Auto cells (see
/// `AUTO_SAMPLE`).
fn sample(files: &[PathBuf]) -> Vec<PathBuf> {
    if files.len() <= AUTO_SAMPLE {
        return files.to_vec();
    }
    (0..AUTO_SAMPLE).map(|k| files[k * files.len() / AUTO_SAMPLE].clone()).collect()
}

/// The proportions of `ASPECTS` nearest to `ratio` (width over height),
/// compared as logarithms, so that 1:2 is as far from 1:1 as 2:1.
pub fn nearest_aspect(ratio: f32) -> f32 {
    let distance = |a: f32| (a.ln() - ratio.ln()).abs();
    ASPECTS.iter().map(|(_, a)| *a).min_by(|a, b| distance(*a).total_cmp(&distance(*b))).unwrap_or(1.0)
}

/// The proportions of `ASPECTS` most of `ratios` are nearest to, the
/// earlier of equally common ones; `None` for no ratios.
pub fn common_aspect(ratios: impl IntoIterator<Item = f32>) -> Option<f32> {
    let mut votes = [0usize; ASPECTS.len()];
    for r in ratios.into_iter().filter(|r| r.is_finite() && *r > 0.0) {
        let a = nearest_aspect(r);
        if let Some(i) = ASPECTS.iter().position(|(_, x)| *x == a) {
            votes[i] += 1;
        }
    }
    let (i, &n) = votes.iter().enumerate().max_by_key(|&(i, n)| (*n, std::cmp::Reverse(i)))?;
    (n > 0).then_some(ASPECTS[i].1)
}

/// The name of `aspect`, one of `ASPECTS`.
pub fn aspect_name(aspect: f32) -> &'static str {
    ASPECTS.iter().find(|(_, a)| *a == aspect).map_or("1:1", |(n, _)| n)
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

/// The grid of `count` cells of thumbnail frames of `frame` points in a
/// list `width` points wide: as many columns as fit, stretched to fill the
/// width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub columns: usize,
    pub rows: usize,
    pub cell: Vec2,
}

pub fn grid(width: f32, frame: Vec2, count: usize) -> Grid {
    let min = frame.x + 2.0 * PAD;
    let columns = ((width / min).floor() as usize).max(1);
    Grid { columns, rows: count.div_ceil(columns), cell: egui::vec2(width / columns as f32, frame.y + 2.0 * PAD + LABEL) }
}

/// Where the cells go: rows of the [`grid`], in sections that each start
/// a row under a header `header` points high: one per folder when the
/// images of the sub-folders are shown by folder, otherwise one with no
/// header.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub columns: usize,
    pub cell: Vec2,
    pub header: f32,
    pub sections: Vec<Section>,
    count: usize,
    /// Height of the whole grid.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    /// Its first cell.
    pub first: usize,
    /// Rows of cells above it.
    row: usize,
    /// Top of its header.
    pub y: f32,
}

impl Layout {
    /// `count` cells in a list `width` points wide; `starts` are the first
    /// cells of the sections, ascending (empty: one section).
    pub fn new(width: f32, frame: Vec2, count: usize, starts: &[usize], header: f32) -> Self {
        let g = grid(width, frame, count);
        let starts = if starts.is_empty() { &[0][..] } else { starts };
        let mut sections = Vec::with_capacity(starts.len());
        let (mut row, mut y) = (0, 0.0);
        for (k, &first) in starts.iter().enumerate() {
            let end = starts.get(k + 1).copied().unwrap_or(count).min(count);
            if first >= end {
                continue;
            }
            sections.push(Section { first, row, y });
            let rows = (end - first).div_ceil(g.columns);
            row += rows;
            y += header + rows as f32 * g.cell.y;
        }
        Self { columns: g.columns, cell: g.cell, header, sections, count, height: y }
    }

    /// Cells in section `k`.
    pub fn len(&self, k: usize) -> usize {
        self.sections.get(k + 1).map_or(self.count, |s| s.first) - self.sections[k].first
    }

    /// The section of cell `i`.
    fn section_of(&self, i: usize) -> usize {
        self.sections.partition_point(|s| s.first <= i).saturating_sub(1)
    }

    /// Top left of cell `i` from the top left of the grid.
    pub fn cell_pos(&self, i: usize) -> Vec2 {
        let Some(s) = self.sections.get(self.section_of(i)) else { return Vec2::ZERO };
        let local = i.saturating_sub(s.first);
        let (row, col) = (local / self.columns, local % self.columns);
        egui::vec2(col as f32 * self.cell.x, s.y + self.header + row as f32 * self.cell.y)
    }

    /// The cells of the rows between `top` and `bottom`, wholly or partly.
    pub fn visible(&self, top: f32, bottom: f32) -> std::ops::Range<usize> {
        self.index_at(top, false)..self.index_at(bottom, true).max(self.index_at(top, false))
    }

    /// The first cell of the row at `y`, or with `after` of the row after
    /// it (the cells above `y` end there).
    fn index_at(&self, y: f32, after: bool) -> usize {
        let k = self.sections.partition_point(|s| s.y <= y).saturating_sub(1);
        let Some(s) = self.sections.get(k) else { return 0 };
        let local = (y - s.y - self.header) / self.cell.y;
        if local <= 0.0 {
            return s.first;
        }
        let rows = if after { local.ceil() } else { local.floor() } as usize;
        (s.first + rows * self.columns).min(s.first + self.len(k))
    }

    /// Rows of cells.
    fn rows(&self) -> usize {
        self.sections.last().map_or(0, |s| s.row + self.len(self.sections.len() - 1).div_ceil(self.columns))
    }

    /// The cell `rows` rows below `i` (above for negative), in the same
    /// column, stopping at the first and last rows; in a row too short for
    /// that column, its last cell. The headers are not rows.
    pub fn move_rows(&self, i: usize, rows: isize) -> usize {
        let k = self.section_of(i);
        let Some(s) = self.sections.get(k) else { return 0 };
        let local = i.saturating_sub(s.first);
        let row = ((s.row + local / self.columns) as isize + rows).clamp(0, self.rows() as isize - 1) as usize;
        let t = self.sections.partition_point(|x| x.row <= row) - 1;
        let s = &self.sections[t];
        (s.first + (row - s.row) * self.columns + local % self.columns).min(s.first + self.len(t) - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fills_the_width() {
        let g = grid(1000.0, frame_size(160.0, 1.0), 69);
        // 172 points a cell at least: 5 fit, stretched to 200.
        assert_eq!((g.columns, g.rows), (5, 14));
        assert_eq!(g.cell, egui::vec2(200.0, 160.0 + 2.0 * PAD + LABEL));
        // 16:9 frames are lower; 9:16 ones narrower, so more fit.
        assert_eq!(grid(1000.0, frame_size(160.0, 16.0 / 9.0), 69).cell.y, 90.0 + 2.0 * PAD + LABEL);
        assert_eq!(grid(1000.0, frame_size(160.0, 9.0 / 16.0), 69).columns, 9);
        // Narrower than one cell: still one column.
        assert_eq!(grid(50.0, frame_size(160.0, 1.0), 3).columns, 1);
        assert_eq!(grid(1000.0, frame_size(160.0, 1.0), 0).rows, 0);
    }

    #[test]
    fn rows_keep_the_column() {
        // 10 cells in rows of 4: 0-3, 4-7, 8-9.
        let frame = egui::vec2(100.0 - 2.0 * PAD, 50.0);
        let l = Layout::new(400.0, frame, 10, &[], 0.0);
        assert_eq!(l.columns, 4);
        let move_rows = |i, rows| l.move_rows(i, rows);
        assert_eq!(move_rows(1, 1), 5);
        assert_eq!(move_rows(5, -1), 1);
        assert_eq!(move_rows(1, -1), 1);
        // Under 6 there is no cell: the last one.
        assert_eq!(move_rows(6, 1), 9);
        assert_eq!(move_rows(9, 1), 9);
        // A page past either end stops at the first or last row.
        assert_eq!(move_rows(5, 10), 9);
        assert_eq!(move_rows(4, 10), 8);
        assert_eq!(move_rows(9, -10), 1);
        assert_eq!(Layout::default().move_rows(3, 1), 0);
    }

    #[test]
    fn sections_start_rows_under_headers() {
        // Rows of 4 cells 80 points high; folders of 6, 2 and 5 cells:
        // 0-3, 4-5 | 6-7 | 8-11, 12.
        let frame = egui::vec2(100.0 - 2.0 * PAD, 80.0 - 2.0 * PAD - LABEL);
        let l = Layout::new(400.0, frame, 13, &[0, 6, 8], 30.0);
        assert_eq!(l.cell, egui::vec2(100.0, 80.0));
        assert_eq!(l.sections.iter().map(|s| s.y).collect::<Vec<_>>(), [0.0, 190.0, 300.0]);
        assert_eq!(l.height, 300.0 + 30.0 + 160.0);
        assert_eq!((l.len(0), l.len(1), l.len(2)), (6, 2, 5));
        assert_eq!(l.cell_pos(0), egui::vec2(0.0, 30.0));
        assert_eq!(l.cell_pos(5), egui::vec2(100.0, 110.0));
        assert_eq!(l.cell_pos(7), egui::vec2(100.0, 220.0));
        assert_eq!(l.cell_pos(12), egui::vec2(0.0, 410.0));
        // Down from the second column: the next folder's second cell; a
        // row too short for the column ends at its last cell.
        assert_eq!(l.move_rows(1, 1), 5);
        assert_eq!(l.move_rows(5, 1), 7);
        assert_eq!(l.move_rows(3, 1), 5);
        assert_eq!(l.move_rows(7, 1), 9);
        assert_eq!(l.move_rows(11, -1), 7);
        assert_eq!(l.move_rows(9, 10), 12);
        assert_eq!(l.move_rows(12, -10), 0);
        // On screen: from the middle of the first row to a header.
        assert_eq!(l.visible(50.0, 200.0), 0..6);
        assert_eq!(l.visible(120.0, 230.0), 4..8);
        assert_eq!(l.visible(195.0, 215.0), 6..6);
        assert_eq!(l.visible(400.0, 1000.0), 8..13);
        // Empty sections are left out.
        assert_eq!(Layout::new(400.0, frame, 5, &[0, 0, 5], 30.0).sections.len(), 1);
    }

    #[test]
    fn thumbnails_fit_or_fill() {
        let square = Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(100.0, 100.0));
        // A 3:2 thumbnail, fitted: whole, letterboxed.
        let (rect, uv) = place_thumb(square, egui::vec2(300.0, 200.0), 1.0, false, true);
        assert_eq!(rect.min, pos2(10.0, 37.0));
        assert!((rect.size() - egui::vec2(100.0, 200.0 / 3.0)).length() < 1e-3, "{rect:?}");
        assert_eq!(uv, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
        // Filled: the square, the middle two thirds across.
        let (rect, uv) = place_thumb(square, egui::vec2(300.0, 200.0), 1.0, true, true);
        assert_eq!(rect, square);
        assert!((uv.min.x - 1.0 / 6.0).abs() < 1e-3 && (uv.max.x - 5.0 / 6.0).abs() < 1e-3, "{uv:?}");
        assert_eq!((uv.min.y, uv.max.y), (0.0, 1.0));
        // A small image is not enlarged either way, only cropped.
        let (rect, uv) = place_thumb(square, egui::vec2(150.0, 40.0), 1.0, true, false);
        assert_eq!(rect.size(), egui::vec2(100.0, 40.0));
        assert!((uv.width() - 100.0 / 150.0).abs() < 1e-3);
        let (rect, _) = place_thumb(square, egui::vec2(30.0, 40.0), 1.0, false, false);
        assert_eq!(rect.size(), egui::vec2(30.0, 40.0));
        // A thumbnail smaller than its image is enlarged to the frame.
        let (rect, _) = place_thumb(square, egui::vec2(60.0, 40.0), 1.0, false, true);
        assert!((rect.size() - egui::vec2(100.0, 200.0 / 3.0)).length() < 1e-3, "{rect:?}");
        // Filled frames need the thumbnail to cover them both ways, up to
        // 1.5 times enlarged.
        let square = egui::vec2(200.0, 200.0);
        assert_eq!(side_needed(square, false, Some(1.5)), 256);
        assert_eq!(side_needed(square, true, None), 256);
        assert_eq!(side_needed(egui::vec2(400.0, 400.0), true, None), 768);
        assert_eq!(side_needed(egui::vec2(512.0, 512.0), false, None), 768);
        // A 3:2 photo in a 16:9 frame: the width decides; in a 9:16 one,
        // the height times 1.5.
        assert_eq!(side_needed(egui::vec2(240.0, 135.0), true, Some(1.5)), 256);
        assert_eq!(side_needed(egui::vec2(135.0, 240.0), true, Some(1.5)), 256);
        assert_eq!(side_needed(egui::vec2(270.0, 480.0), true, Some(1.5)), 768);
        // A portrait photo in a portrait frame.
        assert_eq!(side_needed(egui::vec2(135.0, 240.0), true, Some(2.0 / 3.0)), 256);
        // Prefetching stays within what eviction keeps.
        assert_eq!(cells_in_budget(256), 411);
        assert_eq!(cells_in_budget(768), 45);
    }

    #[test]
    fn auto_proportions() {
        assert_eq!(nearest_aspect(1.5), 1.5);
        assert_eq!(nearest_aspect(1.0), 1.0);
        // A 2:1 panorama: 16:9 is the widest.
        assert_eq!(nearest_aspect(2.0), 16.0 / 9.0);
        assert_eq!(nearest_aspect(1080.0 / 2400.0), 9.0 / 16.0);
        assert_eq!(nearest_aspect(1.4), 4.0 / 3.0);
        // Mostly 3:2 photos, a few portrait ones.
        assert_eq!(common_aspect([1.5, 1.5, 0.667, 1.499, 1.333]), Some(1.5));
        // Phone screenshots.
        assert_eq!(common_aspect([0.45, 0.46, 0.5625, 1.78]), Some(9.0 / 16.0));
        // A tie: the earlier of ASPECTS.
        assert_eq!(common_aspect([1.5, 1.0]), Some(1.0));
        assert_eq!(common_aspect([]), None);
        assert_eq!(common_aspect([f32::NAN, 0.0]), None);
        let files: Vec<PathBuf> = (0..1000).map(|i| PathBuf::from(format!("{i}.jpg"))).collect();
        let s = sample(&files);
        assert_eq!(s.len(), AUTO_SAMPLE);
        assert_eq!((s[0].as_path(), s[199].as_path()), (Path::new("0.jpg"), Path::new("995.jpg")));
        assert_eq!(sample(&files[..3]).len(), 3);
        assert_eq!(aspect_name(16.0 / 9.0), "16:9");
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
