//! The viewer: which file is current, what is on screen, and what the
//! commands do.
//!
//! `current` is the file the user is on; `shown` is what is on screen,
//! which stays the previous image until the current one is decoded, so
//! browsing never flashes an empty window. Decoded images become textures
//! in `cache`, which keeps the current file and its two neighbours; the
//! neighbours are decoded ahead ([`App::update_wanted`]), so the next image
//! usually appears at once.
//!
//! The gallery (`gallery`, opened with G, Enter or a double click) shows the
//! same folder and current file as a grid of thumbnails.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, PointerButton, Rect, Sense, TextureHandle, Vec2};

use crate::folder::{self, Scan};
use crate::gallery::{self, Gallery, Scroll};
use crate::i18n::LangChoice;
use crate::input::{Arrow, Cmd, Wheel};
use crate::loader::{Decoded, Loader, Meta, Pixels};
use crate::texture::Texture;
use crate::view::{self, View, Zoom};
use crate::win;

/// Default background of the image area.
pub const DEFAULT_BACKGROUND: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1c);

/// Backgrounds offered in View → Background, the default first.
pub fn background_presets() -> [(Color32, &'static str); 4] {
    [
        (DEFAULT_BACKGROUND, tr!("Dark (default)", "Тёмный (по умолчанию)")),
        (Color32::BLACK, tr!("Black", "Чёрный")),
        (Color32::from_gray(0x80), tr!("Grey", "Серый")),
        (Color32::WHITE, tr!("White", "Белый")),
    ]
}

/// `#rrggbb` as kept in the settings.
pub fn background_to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// A colour saved by [`background_to_hex`] (any CSS hex form), opaque.
pub fn background_from_hex(s: &str) -> Option<Color32> {
    let [r, g, b, _] = Color32::from_hex(s.trim()).ok()?.to_srgba_unmultiplied();
    Some(Color32::from_rgb(r, g, b))
}

/// Colour of text and the spinner drawn on `bg`: dark on light
/// backgrounds, light on dark ones.
pub fn ink_on(bg: Color32) -> Color32 {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma > 140.0 { Color32::from_gray(70) } else { Color32::from_gray(170) }
}

/// How long a notice stays in the status bar.
const NOTICE_TIME: Duration = Duration::from_secs(4);
/// A window cloaked at start-up is shown after this at the latest.
const UNCLOAK_TIMEOUT: Duration = Duration::from_secs(1);
/// A spinner appears when an image takes longer than this.
const SPINNER_DELAY: Duration = Duration::from_millis(250);
/// Images decoded ahead in the direction of travel: the first always, the
/// others within `CACHE_BUDGET`, so that every decoder thread works while
/// a key is held. One image behind is kept for turning back.
const AHEAD: usize = 3;
/// Pixels of the textures kept at most, the current image included: four
/// 24-megapixel photos, about half a gigabyte of texture memory.
const CACHE_BUDGET: usize = 100_000_000;

const TOOLBAR_KEY: &str = "toolbar";
const STATUS_BAR_KEY: &str = "status_bar";
const BACKGROUND_KEY: &str = "background";
const ZOOM_KEY: &str = "zoom";
const THUMB_SIZE_KEY: &str = "thumb_size";
const TREE_WIDTH_KEY: &str = "tree_width";
const THUMB_FILL_KEY: &str = "thumb_fill";
const THUMB_ASPECT_KEY: &str = "thumb_aspect";
/// `thumb_aspect` in the settings when it is Auto.
const AUTO: &str = "auto";
const LANGUAGE_KEY: &str = "language";

#[derive(Clone)]
pub struct Picture {
    pub texture: Rc<Texture>,
    pub meta: Meta,
}

impl Picture {
    /// Size in image pixels (of the file, not of a shrunk texture).
    pub fn size(&self) -> Vec2 {
        egui::vec2(self.meta.width as f32, self.meta.height as f32)
    }
}

pub enum Slot {
    Ready(Picture),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    Shortcuts,
    About,
    Associations,
}

pub struct App {
    loader: Loader,
    /// The OpenGL context, for the image textures (`texture::Texture`).
    gl: Arc<glow::Context>,
    /// Images of the folder of `current`, in Explorer's order.
    pub files: Vec<PathBuf>,
    /// The folder of `files`.
    pub dir: Option<PathBuf>,
    /// The listing of `dir` while it runs.
    pub scan: Option<Scan>,
    /// The file the user is on.
    pub current: Option<PathBuf>,
    /// Position of `current` in `files`, once the folder is listed.
    pub index: Option<usize>,
    /// When `current` last changed, for the spinner.
    current_since: Instant,
    /// What is on screen.
    pub shown: Option<(PathBuf, Picture)>,
    pub cache: HashMap<PathBuf, Slot>,
    /// Decoded images waiting for their texture (see `poll_decoded`).
    pending: Vec<Decoded>,
    /// Pixels of the textures in `cache` that start at a mip level above 0
    /// (see `texture::Texture`), until the levels below are uploaded.
    partial: HashMap<PathBuf, Pixels>,
    pub view: View,
    /// The image area of the last frame.
    pub viewport: Rect,
    pub show_toolbar: bool,
    pub show_status_bar: bool,
    /// Background of the image area.
    pub background: Color32,
    /// The file the delete confirmation asks about.
    pub confirm_delete: Option<PathBuf>,
    /// Made when the gallery is first opened.
    pub gallery: Option<Gallery>,
    /// The gallery is shown instead of the image.
    pub gallery_open: bool,
    /// The image area got the last click. A double click on it counts only
    /// then: the third click of a triple click on a thumbnail, which lands
    /// on the image the first two opened, is not a request to go back.
    image_clicked: bool,
    /// Size of the gallery's thumbnails, in points.
    pub thumb_size: f32,
    /// Width of the gallery's folder tree, in points.
    pub tree_width: f32,
    /// The gallery's thumbnails fill their cells, cropped.
    pub thumb_fill: bool,
    /// Proportions of the gallery's cells, one of `gallery::ASPECTS`;
    /// `None`: Auto, those of most images of the folder.
    pub thumb_aspect: Option<f32>,
    /// Interface language, chosen in About.
    pub lang: LangChoice,
    deleting: Option<mpsc::Receiver<(PathBuf, Result<(), String>)>>,
    pub dialog: Option<Dialog>,
    /// `dialog` was opened this frame.
    pub dialog_fresh: bool,
    notice: Option<(String, Instant)>,
    pub wheel: Wheel,
    /// Direction of the last move, to decode ahead in that direction first.
    forward: bool,
    title: String,
    /// The app icon of the start screen and of About, rasterised for the
    /// display (see `ui::app_icon`).
    pub start_icon: Option<TextureHandle>,
    pub about_icon: Option<TextureHandle>,
    /// Icons of the file types in the associations dialog, rasterised for
    /// the display, with the pixel size they were made for.
    pub type_icons: Option<(u32, Vec<TextureHandle>)>,
    /// Commands clicked in menus and on the toolbar, run after drawing.
    pub clicked: Vec<Cmd>,
    first_image_logged: bool,
    /// The window handle, when it is a Win32 window.
    hwnd: Option<isize>,
    /// While the window is cloaked: when to show it (see `App::uncloak`).
    cloak: Option<Cloak>,
}

/// The window is cloaked at start-up until its first maximized frame is on
/// screen (see `App::new`), and when restored from minimized with a new
/// file until that image is on screen (see `instance`).
struct Cloak {
    /// Uncloaked at this time at the latest, so that it can never stay
    /// invisible.
    until: Instant,
    /// Wait for the window to be maximized.
    maximized: bool,
    /// Wait for the current image (or its error) to be on screen.
    image: bool,
    /// The frame in which the window was cloaked: a later frame starts
    /// once that one has been presented.
    frame: u64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, loader: Loader, initial: Option<PathBuf>) -> Self {
        let ctx = &cc.egui_ctx;
        // `main` started in the language of Windows.
        let lang = cc
            .storage
            .and_then(|s| s.get_string(LANGUAGE_KEY))
            .and_then(|v| LangChoice::from_name(&v))
            .unwrap_or_default();
        crate::i18n::set_lang(lang.resolve());
        log::debug!("window created at {:.0} ms", crate::since_start_ms());
        let gl = cc.gl.clone().expect("the glow renderer");
        if log::log_enabled!(log::Level::Debug) {
            use glow::HasContext;
            let (renderer, version) = unsafe { (gl.get_parameter_string(glow::RENDERER), gl.get_parameter_string(glow::VERSION)) };
            log::debug!("OpenGL: {renderer}, {version}");
        }
        loader.set_context(ctx.clone());
        // Ctrl+Plus and Ctrl+Minus zoom the image, not the interface.
        // A double click as slow as Windows allows, not egui's 300 ms.
        ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
            o.input_options.max_double_click_delay = win::double_click_time();
        });
        crate::ui::style(ctx);
        // The window is created with the Windows theme and egui turns it
        // dark only at the end of the first frame; Windows 10 would then
        // keep a white caption until the frame is repainted, and forcing
        // that repaint flickers. eframe shows the window after the first
        // frame, so making the caption dark now shows it dark from the start.
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(w)) => Some(w.hwnd.get()),
                _ => None,
            }
        };
        if let Some(hwnd) = hwnd {
            win::dark_caption(hwnd);
            crate::instance::listen(ctx, hwnd);
        }
        // A window saved maximized is created normal (see
        // `main::MAXIMIZE_WHEN_SHOWN`). Showing and maximizing it now would
        // show it before it is painted (a white flash); maximizing it after
        // eframe shows it makes it appear in two steps (the normal size,
        // then the maximized one). So it is maximized now but cloaked,
        // painted unseen at its final size, and uncloaked in `ui` once a
        // maximized frame is on screen.
        let mut cloak = None;
        if let Some(hwnd) = hwnd
            && crate::MAXIMIZE_WHEN_SHOWN.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            win::cloak(hwnd, true);
            win::show_maximized(hwnd);
            cloak = Some(Cloak { until: Instant::now() + UNCLOAK_TIMEOUT, maximized: true, image: false, frame: 0 });
        }
        let flag = |key: &str| cc.storage.and_then(|s| s.get_string(key)).as_deref() != Some("false");
        let mode = cc.storage.and_then(|s| s.get_string(ZOOM_KEY)).and_then(|v| Zoom::from_name(&v)).unwrap_or(Zoom::Fit);
        let number = |key: &str| cc.storage.and_then(|s| s.get_string(key)).and_then(|v| v.parse::<f32>().ok());
        let mut app = Self {
            loader,
            gl,
            files: Vec::new(),
            dir: None,
            scan: None,
            current: None,
            index: None,
            current_since: Instant::now(),
            shown: None,
            cache: HashMap::new(),
            pending: Vec::new(),
            partial: HashMap::new(),
            view: View { zoom: mode, mode, ..View::default() },
            viewport: ctx.content_rect(),
            show_toolbar: flag(TOOLBAR_KEY),
            show_status_bar: flag(STATUS_BAR_KEY),
            background: cc
                .storage
                .and_then(|s| s.get_string(BACKGROUND_KEY))
                .and_then(|v| background_from_hex(&v))
                .unwrap_or(DEFAULT_BACKGROUND),
            confirm_delete: None,
            gallery: None,
            gallery_open: false,
            image_clicked: false,
            thumb_size: number(THUMB_SIZE_KEY)
                .unwrap_or(gallery::DEFAULT_SIZE)
                .clamp(gallery::MIN_SIZE, gallery::MAX_SIZE),
            tree_width: number(TREE_WIDTH_KEY).unwrap_or(240.0).clamp(140.0, 640.0),
            thumb_fill: cc.storage.and_then(|s| s.get_string(THUMB_FILL_KEY)).as_deref() == Some("true"),
            thumb_aspect: match cc.storage.and_then(|s| s.get_string(THUMB_ASPECT_KEY)).as_deref() {
                Some(AUTO) => None,
                v => Some(v.and_then(|v| gallery::ASPECTS.iter().find(|(n, _)| *n == v)).map_or(1.0, |(_, a)| *a)),
            },
            lang,
            deleting: None,
            dialog: None,
            dialog_fresh: false,
            notice: None,
            wheel: Wheel::default(),
            forward: true,
            title: String::new(),
            start_icon: None,
            about_icon: None,
            type_icons: None,
            clicked: Vec::new(),
            first_image_logged: false,
            hwnd,
            cloak,
        };
        if let Some(path) = initial {
            app.open(ctx, path);
        }
        app
    }

    /// Show `path`, a file, and list its folder for browsing; a folder is
    /// shown in the gallery.
    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if path.is_dir() {
            self.open_folder(ctx, path);
            self.enter_gallery(ctx);
            return;
        }
        if self.gallery_open {
            self.leave_gallery();
        }
        let dir = path.parent().map(Path::to_path_buf);
        let listed =
            self.scan.is_none() && matches!((&self.dir, &dir), (Some(a), Some(b)) if folder::same_path(a, b));
        self.index = if listed { folder::position(&self.files, &path) } else { None };
        self.set_current(Some(path.clone()));
        if self.index.is_none()
            && let Some(dir) = dir
        {
            self.start_scan(ctx, dir, Some(path));
        }
    }

    /// List `dir`; its first image becomes the current one.
    pub fn open_folder(&mut self, ctx: &egui::Context, dir: PathBuf) {
        if self.dir.as_deref().is_some_and(|d| folder::same_path(d, &dir)) {
            return;
        }
        self.set_current(None);
        self.start_scan(ctx, dir, None);
    }

    /// Show the gallery of the folder of the current file.
    fn enter_gallery(&mut self, ctx: &egui::Context) {
        let gallery = self.gallery.get_or_insert_with(|| Gallery::new(ctx));
        if let Some(dir) = &self.dir {
            gallery.tree.reveal(dir);
        }
        gallery.scroll = Some(Scroll::Centre);
        self.gallery_open = true;
    }

    /// Back to the image, the current one.
    pub fn leave_gallery(&mut self) {
        self.gallery_open = false;
        self.image_clicked = false;
        if let Some(gallery) = &mut self.gallery {
            gallery.want(Vec::new());
            gallery.hovered = None;
        }
    }

    fn start_scan(&mut self, ctx: &egui::Context, dir: PathBuf, keep: Option<PathBuf>) {
        self.files.clear();
        self.index = None;
        self.dir = Some(dir.clone());
        self.scan = Some(folder::scan(dir, keep, ctx.clone()));
    }

    fn set_current(&mut self, path: Option<PathBuf>) {
        self.current = path;
        self.current_since = Instant::now();
    }

    /// Go to `files[i]`.
    pub fn go(&mut self, i: usize) {
        if self.index == Some(i) {
            return;
        }
        if let Some(path) = self.files.get(i).cloned() {
            self.forward = self.index.is_none_or(|old| i > old);
            self.index = Some(i);
            self.set_current(Some(path));
        }
    }

    /// Move `delta` images along the folder, stopping at its ends.
    fn step(&mut self, delta: isize) {
        if let Some(i) = self.index {
            let target = i as isize + delta;
            if target >= 0 && (target as usize) < self.files.len() {
                self.go(target as usize);
            }
        }
    }

    pub fn notice(&mut self, text: String) {
        self.notice = Some((text, Instant::now()));
    }

    /// The notice to show now, if any.
    pub fn current_notice(&self) -> Option<&str> {
        self.notice.as_ref().filter(|(_, at)| at.elapsed() < NOTICE_TIME).map(|(t, _)| t.as_str())
    }

    fn poll_scan(&mut self) {
        let Some(result) = self.scan.as_ref().and_then(Scan::poll) else { return };
        self.scan = None;
        match result {
            Ok(files) => self.files = files,
            Err(e) => {
                self.notice(tr!(format!("Cannot list the folder: {e}"), format!("Не удалось прочитать папку: {e}")));
                self.files = self.current.iter().cloned().collect();
            }
        }
        self.index = self.current.as_deref().and_then(|c| folder::position(&self.files, c));
        if self.index.is_some() || self.files.is_empty() {
            return;
        }
        match self.current.clone() {
            None => self.go(0),
            // The file is gone (deleted or renamed, or it never existed):
            // without a place in the folder nothing could be browsed. The
            // next image takes its place, as after a deletion.
            Some(missing) => {
                let i = folder::insertion_point(&self.files, &missing).min(self.files.len() - 1);
                let name = file_name(&missing);
                self.notice(tr!(format!("File not found: {name}"), format!("Файл не найден: {name}")));
                self.go(i);
            }
        }
    }

    /// The current file and its neighbours, most wanted first: these are
    /// decoded ahead and kept. The current file, the next one in the
    /// direction of travel and the previous one are always wanted; more
    /// ahead (`AHEAD`) within `CACHE_BUDGET`, every image counted as large
    /// as the current one, so that the set does not change while the
    /// neighbours are decoded.
    fn wanted(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.current.iter().cloned().collect();
        if self.gallery_open {
            // The selected image, and the one under the pointer, which is
            // likely to be clicked; the thumbnails need the threads more.
            if let Some((path, since)) = self.gallery.as_ref().and_then(|g| g.hovered.as_ref())
                && since.elapsed() >= gallery::HOVER_DELAY
                && !paths.contains(path)
            {
                paths.push(path.clone());
            }
            return paths;
        }
        let Some(i) = self.index else { return paths };
        let dir: isize = if self.forward { 1 } else { -1 };
        let at = |offset: isize| usize::try_from(i as isize + offset).ok().and_then(|j| self.files.get(j)).cloned();
        paths.extend(at(dir));
        paths.extend(at(-dir));
        let guess = match self.current.as_ref().and_then(|c| self.cache.get(c)) {
            Some(Slot::Ready(pic)) => pic.meta.width as usize * pic.meta.height as usize,
            _ => 0,
        };
        for k in 2..=AHEAD as isize {
            if (paths.len() + 1) * guess > CACHE_BUDGET {
                break;
            }
            let Some(p) = at(k * dir) else { break };
            paths.push(p);
        }
        paths
    }

    /// Take the decoded images and make the texture of one of them:
    /// uploading a large texture stalls the frame, so one per frame, the
    /// current image first; the others wait in `pending`. A texture starts
    /// at the mip level the image is shown at; a frame with nothing new to
    /// upload completes one of them (`partial`), the current image first.
    fn poll_decoded(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let wanted = self.wanted();
        let max_side = ctx.input(|i| i.max_texture_side);
        self.pending.extend(std::iter::from_fn(|| self.loader.poll()));
        // Dropped: what the user has moved past, and an image decoded
        // before the GPU's limit was known (decoded again).
        self.pending.retain(|d| {
            wanted.contains(&d.path)
                && !matches!(&d.result, Ok((p, _)) if p.width as usize > max_side || p.height as usize > max_side)
        });
        let is_current = |p: &PathBuf| Some(p) == self.current.as_ref();
        if self.pending.is_empty() {
            if let Some(path) = self.partial.keys().find(|p| is_current(p)).or(self.partial.keys().next()).cloned() {
                self.complete(&path);
            }
        } else {
            let first = self.pending.iter().position(|d| is_current(&d.path)).unwrap_or(0);
            let d = self.pending.remove(first);
            let slot = match d.result {
                Ok((pixels, meta)) => {
                    let level = self.start_level(&d.path, &pixels, &meta, ctx);
                    match Texture::new(self.gl.clone(), &pixels, level, |t| frame.register_native_glow_texture(t)) {
                        Ok(texture) => {
                            if texture.base_level() > 0 {
                                self.partial.insert(d.path.clone(), pixels);
                            }
                            Slot::Ready(Picture { texture: Rc::new(texture), meta })
                        }
                        Err(e) => {
                            log::warn!("cannot show {}: {e}", d.path.display());
                            Slot::Failed(e)
                        }
                    }
                }
                Err(e) => {
                    log::warn!("cannot open {}: {e}", d.path.display());
                    Slot::Failed(e)
                }
            };
            self.cache.insert(d.path, slot);
        }
        if !self.pending.is_empty() || !self.partial.is_empty() {
            ctx.request_repaint();
        }
    }

    /// The mip level the texture of `path` can start at: the one for the
    /// scale the image is (or, for a neighbour, will be) shown at.
    fn start_level(&self, path: &Path, pixels: &Pixels, meta: &Meta, ctx: &egui::Context) -> u32 {
        let mut view = self.view;
        if self.current.as_deref() != Some(path) {
            view.next_image();
        }
        let size = egui::vec2(meta.width as f32, meta.height as f32);
        // The scale refers to the file; the texture may be shrunk.
        let scale = view.scale(size, self.viewport, ctx.pixels_per_point()) * pixels.width as f32 / meta.width as f32;
        view::mip_level(scale)
    }

    /// Upload the levels the texture of `path` lacks.
    fn complete(&mut self, path: &Path) {
        if let Some(pixels) = self.partial.remove(path)
            && let Some(Slot::Ready(picture)) = self.cache.get(path)
        {
            picture.texture.complete(&pixels);
        }
    }

    /// The current image is completed at once when the view needs a level
    /// its texture lacks (zoomed in), before the frame is painted.
    fn complete_current_if_needed(&mut self, ctx: &egui::Context) {
        let Some(path) = self.current.clone() else { return };
        let Some(Slot::Ready(picture)) = self.cache.get(&path) else { return };
        let base = picture.texture.base_level();
        if base > 0 && view::mip_level(self.view.scale(picture.size(), self.viewport, ctx.pixels_per_point())) < base {
            self.complete(&path);
        }
    }

    /// Ask for the wanted files that are not decoded yet and forget the
    /// rest.
    fn update_wanted(&mut self) {
        let wanted = self.wanted();
        self.cache.retain(|p, _| wanted.contains(p));
        self.partial.retain(|p, _| wanted.contains(p));
        self.pending.retain(|d| wanted.contains(&d.path));
        let have = |p: &PathBuf| self.cache.contains_key(p) || self.pending.iter().any(|d| d.path == *p);
        self.loader.want(wanted.into_iter().filter(|p| !have(p)));
    }

    /// Put the current image on screen once it is decoded.
    fn sync_shown(&mut self) {
        let Some(current) = &self.current else {
            self.shown = None;
            return;
        };
        match self.cache.get(current) {
            Some(Slot::Ready(picture)) => {
                let (same_path, same_texture) = match &self.shown {
                    Some((p, s)) => (p == current, s.texture.id() == picture.texture.id()),
                    None => (false, false),
                };
                if !same_texture {
                    if !same_path {
                        self.view.next_image();
                    }
                    self.shown = Some((current.clone(), picture.clone()));
                    if !self.first_image_logged {
                        self.first_image_logged = true;
                        log::info!("first image on screen at {:.0} ms", crate::since_start_ms());
                    }
                }
            }
            Some(Slot::Failed(_)) => self.shown = None,
            None => {}
        }
    }

    fn poll_delete(&mut self) {
        let Some(rx) = &self.deleting else { return };
        let Ok((path, result)) = rx.try_recv() else { return };
        self.deleting = None;
        if path.exists() {
            if let Err(e) = result
                && e != "cancelled"
            {
                self.notice(tr!(format!("Cannot delete the file: {e}"), format!("Не удалось удалить файл: {e}")));
            }
            return;
        }
        self.cache.remove(&path);
        let was_current = self.current.as_deref().is_some_and(|c| folder::same_path(c, &path));
        match folder::position(&self.files, &path) {
            Some(pos) => {
                self.files.remove(pos);
                if was_current {
                    self.index = None;
                    if self.files.is_empty() {
                        self.set_current(None);
                    } else {
                        // The next image takes its place; after the last,
                        // the one before.
                        self.go(pos.min(self.files.len() - 1));
                    }
                } else if let Some(i) = self.index
                    && i > pos
                {
                    self.index = Some(i - 1);
                }
            }
            None if was_current => self.set_current(None),
            None => {}
        }
    }

    /// Move `path` to the Recycle Bin on a thread; the shell may ask
    /// questions of its own (a file that cannot be recycled).
    pub fn delete(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let owner = self.hwnd;
        std::thread::spawn(move || {
            let result = win::recycle(&path, owner);
            let _ = tx.send((path, result));
            ctx.request_repaint();
        });
        self.deleting = Some(rx);
    }

    fn handle_drop(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(ctx, path);
        }
    }

    fn is_fullscreen(ctx: &egui::Context) -> bool {
        ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
    }

    /// Carry out `cmd`.
    fn run(&mut self, ctx: &egui::Context, frame: &eframe::Frame, cmd: Cmd) {
        if self.gallery_open && self.run_in_gallery(ctx, cmd) {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let viewport = self.viewport;
        let size = self.shown.as_ref().map(|(_, p)| p.size());
        match cmd {
            Cmd::Next | Cmd::PageDown => self.step(1),
            Cmd::Prev | Cmd::PageUp => self.step(-1),
            Cmd::Gallery => self.enter_gallery(ctx),
            Cmd::First => self.go(0),
            Cmd::Last => self.go(self.files.len().saturating_sub(1)),
            Cmd::Arrow(arrow) => {
                let can = size.map_or([false; 2], |s| self.view.pannable(s, viewport, ppp));
                // An eighth of the window per press.
                let d = viewport.size() / 8.0;
                match (arrow, size) {
                    (Arrow::Left, Some(s)) if can[0] => self.view.pan(egui::vec2(d.x, 0.0), s, viewport, ppp),
                    (Arrow::Right, Some(s)) if can[0] => self.view.pan(egui::vec2(-d.x, 0.0), s, viewport, ppp),
                    (Arrow::Up, Some(s)) if can[1] => self.view.pan(egui::vec2(0.0, d.y), s, viewport, ppp),
                    (Arrow::Down, Some(s)) if can[1] => self.view.pan(egui::vec2(0.0, -d.y), s, viewport, ppp),
                    (Arrow::Left | Arrow::Up, _) => self.step(-1),
                    (Arrow::Right | Arrow::Down, _) => self.step(1),
                }
            }
            Cmd::ZoomIn | Cmd::ZoomOut => {
                if let Some(s) = size {
                    self.view.zoom_step(cmd == Cmd::ZoomIn, s, viewport, ppp, viewport.center());
                }
            }
            Cmd::Actual => self.view.choose(Zoom::Actual, size, viewport, ppp),
            Cmd::Fit => self.view.choose(Zoom::Fit, size, viewport, ppp),
            Cmd::Fill => self.view.choose(Zoom::Fill, size, viewport, ppp),
            Cmd::Cover => self.view.choose(Zoom::Cover, size, viewport, ppp),
            Cmd::RotateLeft => self.view.turns = (self.view.turns + 3) % 4,
            Cmd::RotateRight => self.view.turns = (self.view.turns + 1) % 4,
            Cmd::FullScreen => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!Self::is_fullscreen(ctx))),
            Cmd::Escape if Self::is_fullscreen(ctx) => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false)),
            Cmd::Escape | Cmd::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Delete => {
                if self.deleting.is_none() {
                    self.confirm_delete = self.current.clone().filter(|p| p.is_file());
                }
            }
            Cmd::Copy => {
                if let Some(path) = self.current.clone() {
                    let text = match win::copy_file(&path) {
                        Ok(()) => tr!("Copied to the clipboard".into(), "Скопировано в буфер обмена".into()),
                        Err(e) => tr!(format!("Cannot copy: {e}"), format!("Не удалось скопировать: {e}")),
                    };
                    self.notice(text);
                }
            }
            Cmd::Open => self.pick_file(ctx, frame),
            Cmd::ShowInExplorer => {
                if let Some(path) = &self.current {
                    win::show_in_explorer(path);
                }
            }
            Cmd::Refresh => {
                if let Some(path) = self.current.clone() {
                    // Decoded again; the old picture stays until then.
                    self.cache.remove(&path);
                    if let Some(dir) = path.parent() {
                        self.start_scan(ctx, dir.to_path_buf(), Some(path));
                    }
                } else if let Some(dir) = self.dir.clone() {
                    self.start_scan(ctx, dir, None);
                }
            }
            Cmd::ToggleToolbar => self.show_toolbar = !self.show_toolbar,
            Cmd::ToggleStatusBar => self.show_status_bar = !self.show_status_bar,
            Cmd::Shortcuts | Cmd::About | Cmd::Associations => {
                self.dialog = Some(match cmd {
                    Cmd::About => Dialog::About,
                    Cmd::Associations => Dialog::Associations,
                    _ => Dialog::Shortcuts,
                });
                self.dialog_fresh = true;
            }
        }
    }

    /// Carry out `cmd` the gallery's way; false for the commands that work
    /// as in the viewer.
    fn run_in_gallery(&mut self, ctx: &egui::Context, cmd: Cmd) -> bool {
        let Some(gallery) = &self.gallery else { return false };
        let (columns, page) = (gallery.columns, gallery.page_rows as isize);
        let rows = |app: &mut Self, rows: isize| {
            if let Some(i) = app.index {
                app.go(gallery::move_rows(i, rows, columns, app.files.len()));
            }
        };
        match cmd {
            Cmd::Arrow(Arrow::Left) => self.step(-1),
            Cmd::Arrow(Arrow::Right) => self.step(1),
            Cmd::Arrow(Arrow::Up) => rows(self, -1),
            Cmd::Arrow(Arrow::Down) => rows(self, 1),
            Cmd::PageUp => rows(self, -page),
            Cmd::PageDown => rows(self, page),
            Cmd::ZoomIn | Cmd::ZoomOut => {
                self.thumb_size = gallery::step_size(self.thumb_size, cmd == Cmd::ZoomIn);
                if let Some(gallery) = &mut self.gallery {
                    gallery.scroll = Some(Scroll::Visible);
                }
            }
            // Nothing to zoom or turn.
            Cmd::Actual | Cmd::Fit | Cmd::Fill | Cmd::Cover | Cmd::RotateLeft | Cmd::RotateRight => {}
            Cmd::Gallery => self.leave_gallery(),
            Cmd::Escape if !Self::is_fullscreen(ctx) => self.leave_gallery(),
            Cmd::Refresh => {
                // The thumbnails and the tree too; the folder is read again
                // as in the viewer.
                if let Some(gallery) = &mut self.gallery {
                    gallery.forget(&self.files);
                    if let Some(dir) = &self.dir {
                        gallery.tree.refresh(dir);
                    }
                }
                return false;
            }
            _ => return false,
        }
        true
    }

    fn pick_file(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr!("Open image", "Открыть изображение"))
            .add_filter(tr!("Images", "Изображения"), folder::EXTENSIONS)
            .add_filter(tr!("All files", "Все файлы"), &["*"])
            .set_parent(frame);
        if let Some(dir) = &self.dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(ctx, path);
        }
    }

    /// The image area: the picture, panning, the wheel and the context
    /// menu.
    fn image_area(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        self.viewport = rect;
        let ppp = ctx.pixels_per_point();
        // Not focusable: Space and Enter must not "click" it.
        let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
        let painter = ui.painter_at(rect);

        // Still decoding the current image.
        let waiting = self.current.as_ref().is_some_and(|c| {
            !self.cache.contains_key(c) && self.shown.as_ref().is_none_or(|(p, _)| p != c)
        }) || (self.current.is_none() && self.scan.is_some());
        if let Some((texture, size)) = self.placeholder().filter(|_| waiting) {
            // Its thumbnail, where the image will be, rather than the
            // previous image.
            let mut view = self.view;
            view.next_image();
            view::paint(&painter, texture, view.place(size, rect, ppp), 0);
        } else if let Some((_, picture)) = self.shown.clone() {
            let size = picture.size();
            if response.dragged_by(PointerButton::Primary) {
                self.view.pan(response.drag_delta(), size, rect, ppp);
            }
            let place = self.view.place(size, rect, ppp);
            view::paint(&painter, picture.texture.id(), place, self.view.turns);
            if self.view.pannable(size, rect, ppp).contains(&true) && response.hovered() {
                ctx.set_cursor_icon(if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                });
            }
        } else if self.current.is_none() && self.scan.is_none() && self.dir.is_none() {
            self.start_screen(ui, rect);
        } else {
            let text = match (&self.current, self.current.as_ref().and_then(|c| self.cache.get(c))) {
                (Some(_), Some(Slot::Failed(e))) => Some(tr!(format!("Cannot open the image\n\n{e}"), format!("Не удалось открыть изображение\n\n{e}"))),
                (None, _) if self.scan.is_none() => Some(tr!("No images in this folder".into(), "В этой папке нет изображений".into())),
                _ => None,
            };
            if let Some(text) = text {
                painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), ink_on(self.background));
            }
        }

        if waiting {
            let late = self.current_since.elapsed() > SPINNER_DELAY;
            if late {
                let r = Rect::from_center_size(rect.right_bottom() - egui::vec2(28.0, 28.0), egui::vec2(24.0, 24.0));
                egui::Spinner::new().color(ink_on(self.background)).paint_at(ui, r);
            } else {
                ctx.request_repaint_after(SPINNER_DELAY);
            }
        }

        if crate::input::double_clicked(&response) && self.image_clicked {
            self.clicked.push(Cmd::Gallery);
        }
        if response.clicked() {
            self.image_clicked = true;
        }
        if response.middle_clicked() {
            self.clicked.push(Cmd::FullScreen);
        }
        // Windows sends the wheel to the window under the pointer, so it
        // browses from anywhere in the window, but not under a menu or a
        // dialog.
        let modal_open = self.confirm_delete.is_some() || self.dialog.is_some();
        if !modal_open && !egui::Popup::is_any_open(&ctx) {
            let (browse, zoom) = self.wheel.read(&ctx);
            // Wheel up: the previous image.
            for _ in 0..browse.unsigned_abs() {
                self.clicked.push(if browse > 0 { Cmd::Prev } else { Cmd::Next });
            }
            if zoom != 0
                && let Some((_, picture)) = &self.shown
            {
                let anchor = ctx.pointer_latest_pos().filter(|p| rect.contains(*p)).unwrap_or(rect.center());
                for _ in 0..zoom.unsigned_abs() {
                    self.view.zoom_step(zoom > 0, picture.size(), rect, ppp, anchor);
                }
            }
        }
        response.context_menu(|ui| self.context_menu(ui));
    }

    /// The gallery's thumbnail of the current image and the image's size,
    /// to show while the image is decoded.
    fn placeholder(&self) -> Option<(egui::TextureId, Vec2)> {
        let thumb = self.gallery.as_ref()?.cache.get(self.current.as_ref()?)?;
        let texture = thumb.texture.as_ref()?;
        let size = if thumb.width > 0 && thumb.height > 0 {
            egui::vec2(thumb.width as f32, thumb.height as f32)
        } else {
            egui::vec2(thumb.px[0] as f32, thumb.px[1] as f32)
        };
        Some((texture.id(), size))
    }

    /// Show the cloaked window (see `Cloak`) once a frame of what it waits
    /// for has been presented: called before anything changes in this
    /// frame, so what is true now was painted in the previous one.
    fn uncloak(&mut self, ctx: &egui::Context) {
        let Some(cloak) = &self.cloak else { return };
        let ready = ctx.cumulative_frame_nr() > cloak.frame
            && (!cloak.maximized || ctx.input(|i| i.viewport().maximized == Some(true)))
            && (!cloak.image || self.current_on_screen());
        if ready || Instant::now() >= cloak.until {
            if let Some(hwnd) = self.hwnd {
                win::cloak(hwnd, false);
            }
            self.cloak = None;
            log::debug!("window uncloaked at {:.0} ms", crate::since_start_ms());
        } else {
            ctx.request_repaint();
        }
    }

    /// The current image, or its error, is what is on screen; with no
    /// current image, the folder being opened has been listed.
    fn current_on_screen(&self) -> bool {
        if self.gallery_open {
            return self.scan.is_none();
        }
        match &self.current {
            None => self.scan.is_none(),
            Some(c) => {
                self.shown.as_ref().is_some_and(|(p, _)| p == c) || matches!(self.cache.get(c), Some(Slot::Failed(_)))
            }
        }
    }

    /// Nothing opened yet: the icon, the name, the version and what to do.
    fn start_screen(&mut self, ui: &mut egui::Ui, rect: Rect) {
        const ICON: f32 = 112.0;
        let icon = crate::ui::app_icon(ui.ctx(), ICON, &mut self.start_icon);
        let ink = ink_on(self.background);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Center)));
        ui.add_space((rect.height() * 0.5 - ICON).max(0.0));
        ui.image((icon.id(), egui::vec2(ICON, ICON)));
        ui.add_space(14.0);
        ui.label(egui::RichText::new("qview").size(30.0).strong().color(ink));
        ui.label(
            egui::RichText::new(tr!(format!("Version {}", crate::VERSION), format!("Версия {}", crate::VERSION)))
                .color(ink.gamma_multiply(0.8)),
        );
        ui.add_space(18.0);
        ui.label(egui::RichText::new(tr!(
            "Open an image with Ctrl+O or drop a file here; G opens the gallery",
            "Откройте изображение: Ctrl+O или перетащите файл сюда; G — галерея"
        ))
        .color(ink));
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match (&self.current, &self.dir) {
            (_, Some(dir)) if self.gallery_open => format!("{} - qview", file_name(dir)),
            (Some(p), _) => format!("{} - qview", file_name(p)),
            _ => "qview".into(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// The file name of `path` for display.
pub fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        let frame_nr = ctx.cumulative_frame_nr();
        if frame_nr == 0 {
            self.loader.set_max_side(ctx.input(|i| i.max_texture_side));
            // The image area of this frame, near enough for the mip level
            // the first texture starts at (`start_level`): the window less
            // the bars; `image_area` then keeps it exact.
            let mut area = ctx.content_rect();
            if !Self::is_fullscreen(&ctx) {
                area.min.y += 24.0 + if self.show_toolbar { 32.0 } else { 0.0 };
                area.max.y -= if self.show_status_bar { 24.0 } else { 0.0 };
            }
            self.viewport = area;
        }
        if frame_nr < 8 {
            log::debug!("frame {frame_nr} at {:.0} ms", crate::since_start_ms());
        }
        self.uncloak(&ctx);

        // A file opened while qview is running (see `instance`).
        let received = crate::instance::take();
        if let Some(path) = received.path {
            self.confirm_delete = None;
            self.open(&ctx, path);
        }
        if received.cloaked {
            self.cloak = Some(Cloak { until: Instant::now() + UNCLOAK_TIMEOUT, maximized: false, image: true, frame: frame_nr });
            ctx.request_repaint();
        }
        self.poll_scan();
        self.poll_decoded(&ctx, frame);
        if let Some(gallery) = &mut self.gallery {
            gallery.poll(&self.gl, frame, &ctx);
        }
        self.poll_delete();
        self.handle_drop(&ctx);
        self.sync_shown();

        let modal_open = self.confirm_delete.is_some() || self.dialog.is_some();
        if !modal_open && !egui::Popup::is_any_open(&ctx) {
            // Keys are the viewer's: no widget keeps the focus to take
            // Space or Enter as a click.
            if let Some(id) = ctx.memory(|m| m.focused()) {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
            for cmd in crate::input::keys(&ctx, self.gallery_open) {
                self.run(&ctx, frame, cmd);
            }
            self.sync_shown();
        }
        self.complete_current_if_needed(&ctx);

        let fullscreen = Self::is_fullscreen(&ctx);
        if !fullscreen {
            self.menu_bar(root_ui);
            if self.show_toolbar {
                self.toolbar(root_ui);
            }
            if self.show_status_bar {
                self.status_bar(root_ui);
            }
        }
        if self.gallery_open {
            self.gallery_ui(root_ui);
        } else {
            egui::CentralPanel::no_frame().show(root_ui, |ui| self.image_area(ui));
        }
        self.dialogs(&ctx);

        let clicked = std::mem::take(&mut self.clicked);
        if !clicked.is_empty() {
            for cmd in clicked {
                self.run(&ctx, frame, cmd);
            }
            self.sync_shown();
            ctx.request_repaint();
        }
        self.update_wanted();
        self.update_title(&ctx);
        if let Some((_, at)) = &self.notice
            && at.elapsed() < NOTICE_TIME
        {
            ctx.request_repaint_after(NOTICE_TIME - at.elapsed());
        }
    }

    /// The textures are deleted now, while the GL context still lives.
    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.shown = None;
        self.cache.clear();
        self.pending.clear();
        self.partial.clear();
        if let Some(gallery) = &mut self.gallery {
            gallery.clear();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(TOOLBAR_KEY, self.show_toolbar.to_string());
        storage.set_string(STATUS_BAR_KEY, self.show_status_bar.to_string());
        storage.set_string(BACKGROUND_KEY, background_to_hex(self.background));
        storage.set_string(ZOOM_KEY, self.view.mode.name().unwrap_or("fit").to_string());
        storage.set_string(THUMB_SIZE_KEY, self.thumb_size.round().to_string());
        storage.set_string(TREE_WIDTH_KEY, self.tree_width.round().to_string());
        storage.set_string(THUMB_FILL_KEY, self.thumb_fill.to_string());
        let aspect = self.thumb_aspect.map_or(AUTO, gallery::aspect_name);
        storage.set_string(THUMB_ASPECT_KEY, aspect.to_string());
        storage.set_string(LANGUAGE_KEY, self.lang.name().to_string());
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // The image area has no frame: this is its background. In the
        // gallery, the grid's: it shows through between the panels.
        let color = if self.gallery_open { crate::ui::gallery::GRID_BG } else { self.background };
        color.to_normalized_gamma_f32()
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_round_trip() {
        for (c, _) in background_presets() {
            assert_eq!(background_from_hex(&background_to_hex(c)), Some(c));
        }
        assert_eq!(background_to_hex(DEFAULT_BACKGROUND), "#1c1c1c");
        assert_eq!(background_from_hex(" #fff "), Some(Color32::WHITE));
        assert_eq!(background_from_hex("bogus"), None);
    }

    #[test]
    fn ink_contrasts_with_the_background() {
        assert_eq!(ink_on(DEFAULT_BACKGROUND), Color32::from_gray(170));
        assert_eq!(ink_on(Color32::WHITE), Color32::from_gray(70));
    }
}
