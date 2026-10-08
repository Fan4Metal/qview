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

use crate::crop::Crop;
use crate::editors::Editor;
use crate::edit;
use crate::favorites::{self, Favorites};
use crate::folder::{self, Order, Scan, SortKey};
use crate::gallery::{self, Cell, Gallery, Scroll};
use crate::history::{History, Place};
use crate::i18n::LangChoice;
use crate::input::{Arrow, Cmd, Mode, Move, Wheel};
use crate::loader::{Decoded, Loader, Meta, Pixels};
use crate::selection::Selection;
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
/// Opening more files than this in an editor asks first: many editors
/// open a window for each.
const EDIT_WITHOUT_ASKING: usize = 5;
/// Renames and saves Ctrl+Z can undo, the latest last.
const UNDO_STEPS: usize = 20;
/// The old contents of saved files kept for Ctrl+Z, at most; the oldest
/// are forgotten first, the latest always kept.
const UNDO_BYTES: usize = 512 << 20;

const TOOLBAR_KEY: &str = "toolbar";
const STATUS_BAR_KEY: &str = "status_bar";
const BACKGROUND_KEY: &str = "background";
const CHECKER_KEY: &str = "checker";
const FILTER_KEY: &str = "filter";
const ZOOM_KEY: &str = "zoom";
const THUMB_SIZE_KEY: &str = "thumb_size";
const TREE_WIDTH_KEY: &str = "tree_width";
const INFO_KEY: &str = "info_panel";
const FOLDERS_KEY: &str = "folders_folded";
const INFO_WIDTH_KEY: &str = "info_width";
const HISTOGRAM_KEY: &str = "info_histogram";
const HISTOGRAM_CHANNELS_KEY: &str = "info_histogram_channels";
const COLOUR_KEY: &str = "info_colour";
const SLIDESHOW_SECONDS_KEY: &str = "slideshow_seconds";
const SLIDESHOW_LOOP_KEY: &str = "slideshow_loop";
const THUMB_FILL_KEY: &str = "thumb_fill";
const THUMB_ASPECT_KEY: &str = "thumb_aspect";
/// `thumb_aspect` in the settings when it is Auto.
const AUTO: &str = "auto";
const LANGUAGE_KEY: &str = "language";
const SORT_KEY: &str = "sort";
const SORT_DESCENDING_KEY: &str = "sort_descending";
const FAVORITES_SORT_KEY: &str = "favorites_sort";
const FAVORITES_SORT_DESCENDING_KEY: &str = "favorites_sort_descending";
const CHECK_UPDATES_KEY: &str = "check_updates";
const LAST_UPDATE_CHECK_KEY: &str = "last_update_check";
const BY_FOLDER_KEY: &str = "by_folder";
/// The editor chosen last (see `editors`): its handler's name and how it is
/// shown.
const EDITOR_KEY: &str = "editor";
const EDITOR_NAME_KEY: &str = "editor_name";
/// The window's normal rectangle (`win::normal_rect`), as "left,top,right,bottom":
/// eframe saves a maximized window with its maximized size, which it would
/// then be restored to.
const WINDOW_NORMAL_KEY: &str = "window_normal";

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

/// A file being renamed (F2): the name as typed, and why it could not be
/// used.
pub struct Rename {
    pub path: PathBuf,
    pub name: String,
    pub error: Option<String>,
    /// Focus the name on the next frame, and with `select` select it but
    /// the extension, as Explorer does.
    pub focus: bool,
    pub select: bool,
}

/// A folder to make: in the folder shown (`transfer` None), or beside the
/// current image, or those chosen, to move them into (Alt+N, `Some(false)`)
/// or copy them into (Shift+Alt+N, `Some(true)`); where, the name as
/// typed, and why it could not be used.
pub struct NewFolder {
    pub parent: PathBuf,
    pub name: String,
    pub transfer: Option<bool>,
    pub error: Option<String>,
    /// Focus the name on the next frame, and with `select` select it.
    pub focus: bool,
    pub select: bool,
}

/// What Ctrl+Z undoes, all of it at once.
pub enum Undo {
    /// Files renamed, `(old, new)`.
    Rename(Vec<(PathBuf, PathBuf)>),
    /// Files moved into a folder (Alt+1 to Alt+9, the Move to Folder
    /// menu), `(old, new)`; `created`: the folder made for them (Alt+N),
    /// removed again if it is empty once they are back.
    Move { pairs: Vec<(PathBuf, PathBuf)>, created: Option<PathBuf> },
    /// Copies made in a folder (Shift+Alt+1 to Shift+Alt+9, the Copy to
    /// Folder menu), where no file of the name was: undone by deleting
    /// them; `created` as for `Move`.
    Copy { copies: Vec<PathBuf>, created: Option<PathBuf> },
    /// Files saved (turned, cropped, converted): each one's contents
    /// before, None if saving made it.
    Save(Vec<(PathBuf, Option<edit::Before>)>),
}

/// Files to open in an editor once the user agrees (more than
/// `EDIT_WITHOUT_ASKING`): the editor chosen for them, if any.
pub struct EditRequest {
    pub files: Vec<PathBuf>,
    pub editor: Option<Editor>,
}

/// Files sent to the Recycle Bin, and how it went (see `App::delete`).
type Deleted = (Vec<PathBuf>, Result<(), String>);

/// Images being saved on a thread, one after another (see `edit::save`).
struct Saving {
    jobs: Vec<edit::Job>,
    /// Copies in another format (Convert To): the originals stay current.
    convert: bool,
    done: mpsc::Receiver<Result<edit::Saved, String>>,
    /// The outcomes so far, in the order of `jobs`.
    results: Vec<Result<edit::Saved, String>>,
}

/// Several files being renamed at once (F2 with several images chosen in
/// the gallery): a name and the first number (see `rename::numbered`).
pub struct BatchRename {
    /// The folders chosen first (`dirs` of them), then the images.
    pub paths: Vec<PathBuf>,
    pub dirs: usize,
    pub base: String,
    pub start: u32,
    pub error: Option<String>,
    /// Focus the name on the next frame.
    pub focus: bool,
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
    /// `files` has the images of all the sub-folders of `dir` too, folder
    /// by folder ("With sub-folders" in the list above the gallery's grid; folders chosen in
    /// the tree are listed the same way).
    pub deep: bool,
    /// Where each folder's images start in `files`.
    pub starts: Vec<usize>,
    /// With `deep`, each folder's images are ordered and shown on their
    /// own, under a header in the gallery; otherwise in one order.
    pub by_folder: bool,
    /// The listing of `dir` while it runs.
    pub scan: Option<Scan>,
    /// The order of `files` (View → Sort).
    pub sort: Order,
    /// The order of the favourites, their own: by when they were marked
    /// unless chosen otherwise.
    pub favorites_sort: Order,
    /// Where `current` was in `files` before the folder was listed again:
    /// its successor takes that place if it has gone and its place in the
    /// new listing is not known (see `poll_scan`).
    place: Option<usize>,
    /// Scroll the gallery to the current image once the listing is in:
    /// it moved when the order changed.
    centre_after_scan: bool,
    /// The sub-folder whose cell gets the cursor once the listing is in:
    /// the one returned from (Back, Up, the folder above chosen in the
    /// tree), so that a long list of folders is not scrolled through again.
    refocus: Option<PathBuf>,
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
    /// A checkerboard behind the image, seen where it is transparent.
    pub checker: bool,
    /// Its texture, made when first painted.
    checker_texture: Option<TextureHandle>,
    /// How the image is filtered when not at 100% (View → Filtering).
    pub filter: view::Filter,
    /// The filters' programs, made when first needed; None if one cannot
    /// be made (a mesh then, see `paint_picture`).
    programs: HashMap<view::Filter, Option<Arc<crate::filter::Program>>>,
    /// The program of the filter chosen is made in this frame.
    program_due: bool,
    /// The files the delete confirmation asks about.
    pub confirm_delete: Option<Vec<PathBuf>>,
    /// How many of `confirm_delete` are folders: the first ones.
    pub delete_folders: usize,
    pub rename: Option<Rename>,
    pub batch_rename: Option<BatchRename>,
    /// The images chosen in the gallery (see `selection`).
    pub selection: Selection,
    /// The editor chosen last, which Ctrl+E opens (kept between runs).
    pub editor: Option<Editor>,
    /// The editors Windows offers, by extension, read once.
    editors: HashMap<String, Vec<Editor>>,
    /// Those listed in the Edit With menu this frame, which
    /// `Cmd::EditWith` counts in.
    pub menu_editors: Vec<Editor>,
    /// The folders listed in the Move to Folder menu this frame (the
    /// parent folder and the sub-folders of the image's folder), which
    /// `Cmd::MoveToListed` counts in.
    pub menu_folders: Vec<PathBuf>,
    /// The sub-folders of the image's folder read for that menu when the
    /// listing is not that folder alone: the folder and what it holds
    /// (forgotten by a new listing and by a move).
    sibling_folders: Option<(PathBuf, Vec<PathBuf>)>,
    /// A folder being named, to move or copy the images into.
    pub new_folder: Option<NewFolder>,
    /// An editor being started on a thread.
    opening: Option<mpsc::Receiver<Result<(), String>>>,
    /// Many files to open in an editor, waiting for the user's yes.
    pub confirm_edit: Option<EditRequest>,
    /// The renames and saves of this session, for Ctrl+Z.
    pub undo: Vec<Undo>,
    /// The frame of C over the current image, while it is cropped.
    pub crop: Option<Crop>,
    saving: Option<Saving>,
    /// The current file was saved over: when it is decoded again, the
    /// view's rotation, now in the file, is dropped.
    reloading: Option<PathBuf>,
    /// The folders shown before and after this one, for Back and Forward.
    pub history: History,
    /// The favourite images (S), listed in the gallery as `favorites::DIR`.
    pub favorites: Favorites,
    /// The folders pinned to Quick Access: under it in the tree, cells in
    /// its grid (`favorites::PINNED_DIR`), and with a key (Alt+1 to Alt+9)
    /// the folders the current image is moved or copied into.
    pub pinned: Favorites,
    /// Clearing the favourites waits for the user's yes.
    pub confirm_clear_favorites: bool,
    /// Unpinning every folder from Quick Access waits for the user's yes.
    pub confirm_unpin_all: bool,
    /// The favourites gone from their folders, looked for at start-up, so
    /// that their count is right before they are listed.
    favorites_gone: Option<mpsc::Receiver<Vec<PathBuf>>>,
    /// The pinned folders gone (deleted or renamed outside), looked for at
    /// start-up and on F5 in the gallery.
    pinned_gone: Option<mpsc::Receiver<Vec<PathBuf>>>,
    /// Files being copied to a folder by the shell.
    copying: Option<Copying>,
    /// Files being moved to a folder on another drive by the shell.
    moving: Option<Moving>,
    /// The listing as it came, before the gallery's filter: `files` is what
    /// of it passes `name_filter`.
    listed: Vec<PathBuf>,
    /// The sub-folders of `dir` listed alone, as the gallery shows them
    /// above its images (not those of the favourites, an archive or a
    /// listing with the sub-folders); `folders` is what of them passes the
    /// filter.
    listed_folders: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
    /// The gallery's section of the sub-folders is folded to its header
    /// (a click on it).
    pub folders_folded: bool,
    /// The cell of `folders[k]` has the gallery's cursor instead of the
    /// current image: Enter opens it, the file commands do nothing.
    pub folder_focus: Option<usize>,
    /// The gallery's filter (Ctrl+F, `folder::matches`); empty: none. Kept
    /// while the same folder is listed again, cleared for another.
    pub name_filter: String,
    /// Its field had the focus when last drawn. egui takes the focus away
    /// on Esc before the frame begins, so by then the field no longer has
    /// it: its Esc would close the gallery.
    pub filter_focused: bool,
    /// The field is wanted in this frame (`/`, Ctrl+F, the magnifier), before
    /// it has the focus.
    pub filter_open: bool,
    /// `dir` is a ZIP archive (`archive`), listed as a folder.
    pub archive: bool,
    /// The information panel (I) is shown, and its width in points.
    pub show_info: bool,
    pub info_width: f32,
    /// The slideshow (Shift+F), while it runs.
    pub slideshow: Option<Slideshow>,
    /// Seconds each image stays, and whether it starts over after the
    /// last one (View → Slideshow; persisted).
    pub slideshow_seconds: u32,
    pub slideshow_loop: bool,
    /// What it shows of the current image, read on a thread.
    pub info: Option<InfoSlot>,
    /// The panel's Histogram section is open (persisted): only then are
    /// histograms counted (`update_histogram`).
    pub show_histogram: bool,
    /// The channels its graph shows (persisted).
    pub histogram_channels: crate::histogram::Channels,
    /// The panel's Colour section is open (persisted): only then is the
    /// current image kept in memory (`update_colour`).
    pub show_colour: bool,
    /// The current image's pixels for the Colour section, read again from
    /// its file on a thread.
    pub colour_pixels: Option<PixelSlot>,
    /// The pixel under the pointer, found while the image area is drawn.
    pub picked: Option<Picked>,
    /// The pixel the image's context menu was opened on.
    pub menu_picked: Option<Picked>,
    /// What the panel says of the gallery's focused folder or of the
    /// images chosen, counted on a thread (`update_stats`).
    pub stats: Option<StatsSlot>,
    /// The current image's histogram counted from the file read again,
    /// when it was decoded before the section was opened.
    pub histogram: Option<HistogramSlot>,
    /// Copy Image or Paste under way on a thread (`clipboard`).
    clipboard: Option<mpsc::Receiver<Clipped>>,
    /// Set as Wallpaper, or the files for Print, under way on a thread.
    handing: Option<mpsc::Receiver<Handed>>,
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
    /// The update check (off unless enabled in About).
    pub updates: crate::update::Updates,
    deleting: Option<mpsc::Receiver<Deleted>>,
    /// The folders among the paths being deleted (known before they go).
    deleting_folders: Vec<PathBuf>,
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
    /// In full screen in the last frame: the window's normal rectangle is
    /// then the screen's, and is not saved.
    fullscreen: bool,
    /// While the window is cloaked: when to show it (see `App::uncloak`).
    cloak: Option<Cloak>,
    /// The animation of the image on screen, while it plays.
    pub player: Option<crate::anim::Player>,
}

/// Files being copied by the shell on a thread (see `win::copy_to`).
struct Copying {
    done: mpsc::Receiver<Result<(), String>>,
    to: PathBuf,
    count: usize,
    /// The copies that would be new in `to` (Shift+Alt+1 to 9), for the
    /// notice and Ctrl+Z; None: the favourites (Copy All to Folder).
    fresh: Option<Vec<PathBuf>>,
    /// The folder made for them (Shift+Alt+N), for Ctrl+Z.
    created: Option<PathBuf>,
}

/// Files being moved by the shell on a thread (see `win::move_to`).
struct Moving {
    done: mpsc::Receiver<Result<(), String>>,
    /// Each file and where it goes.
    pairs: Vec<(PathBuf, PathBuf)>,
    /// Remember the move for Ctrl+Z (not when it is one).
    undoable: bool,
    /// The folder made for them (Alt+N), for Ctrl+Z.
    created: Option<PathBuf>,
    /// Undoing a move into a folder made for it: the folder to remove
    /// once the files are back, if it is empty then.
    remove_empty: Option<PathBuf>,
}

/// The name of a folder for a message: a drive's root has none, so its
/// path.
pub fn folder_label(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// The commands that change the files (or the favourites), refused for the
/// images of an archive. Save goes to Save As there (`edit::can_overwrite`).
fn changes_files(cmd: Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Delete
            | Cmd::Rename
            | Cmd::Copy
            | Cmd::ConvertTo(_)
            | Cmd::Edit
            | Cmd::EditWith(_)
            | Cmd::EditWithOther
            | Cmd::Favorite
            | Cmd::MoveTo(_)
            | Cmd::CopyTo(_)
            | Cmd::MoveToListed(_)
            | Cmd::CopyToListed(_)
            | Cmd::MoveToNew
            | Cmd::CopyToNew
            | Cmd::MoveToOther
            | Cmd::CopyToOther
    )
}

/// The commands of the current image or the chosen ones besides Delete
/// and F2, which do nothing while a folder's cell has the cursor or only
/// folders are chosen.
fn acts_on_files(cmd: Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Copy
            | Cmd::CopyImage
            | Cmd::ConvertTo(_)
            | Cmd::Edit
            | Cmd::EditWith(_)
            | Cmd::EditWithOther
            | Cmd::Favorite
            | Cmd::Print
            | Cmd::Wallpaper
            | Cmd::MoveTo(_)
            | Cmd::CopyTo(_)
            | Cmd::MoveToListed(_)
            | Cmd::CopyToListed(_)
            | Cmd::MoveToNew
            | Cmd::CopyToNew
            | Cmd::MoveToOther
            | Cmd::CopyToOther
    )
}

/// What a clipboard thread did.
enum Clipped {
    Copied(Result<(), String>),
    /// The file to open.
    Pasted(Result<PathBuf, String>),
}

/// A slideshow under way: the next image comes `slideshow_seconds` after
/// the current one is on screen.
pub struct Slideshow {
    /// When the next image is due, once the image it was armed for is on
    /// screen; None until then.
    next_at: Option<Instant>,
    /// The image the timer runs for: another current image (browsed to by
    /// hand) starts the time over.
    armed_for: Option<PathBuf>,
    pub paused: bool,
}

/// The information panel's data of one file (`info::read`).
pub struct InfoSlot {
    pub path: PathBuf,
    /// None while it is read.
    pub info: Option<std::sync::Arc<crate::info::Info>>,
    rx: Option<mpsc::Receiver<crate::info::Info>>,
}

/// The histogram of an image decoded without it (see `App::update_histogram`).
pub struct HistogramSlot {
    pub path: PathBuf,
    /// None while it is counted, or when the file could not be read.
    pub histogram: Option<Arc<crate::histogram::Histogram>>,
    rx: Option<mpsc::Receiver<Option<crate::histogram::Histogram>>>,
}

/// The decoded pixels of one image, upright, at full size (see
/// `App::update_colour`).
pub struct PixelSlot {
    pub path: PathBuf,
    /// None while it is read, or when it could not be.
    pub image: Option<Arc<image::DynamicImage>>,
    rx: Option<mpsc::Receiver<Option<image::DynamicImage>>>,
}

impl PixelSlot {
    pub fn reading(&self) -> bool {
        self.rx.is_some()
    }
}

/// A pixel of the image on screen: where it is on the image as shown
/// (turned and mirrored as the view is), and its colour, straight RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picked {
    pub x: u32,
    pub y: u32,
    pub rgba: [u8; 4],
}

impl Picked {
    /// `#3A6EA5`.
    pub fn hex(&self) -> String {
        let [r, g, b, _] = self.rgba;
        format!("#{r:02X}{g:02X}{b:02X}")
    }

    /// `58, 110, 165`.
    pub fn rgb(&self) -> String {
        let [r, g, b, _] = self.rgba;
        format!("{r}, {g}, {b}")
    }
}

/// What the information panel counts in the gallery.
#[derive(Clone, Debug, PartialEq)]
pub enum StatsOf {
    /// The sub-folder with the cursor.
    Folder(PathBuf),
    /// The images chosen, more than one.
    Files(Vec<PathBuf>),
}

/// `folder::Stats` of a folder or of several images (`App::update_stats`).
pub struct StatsSlot {
    pub of: StatsOf,
    /// None while counted; an error when the folder cannot be read.
    pub stats: Option<Result<crate::folder::Stats, String>>,
    rx: Option<mpsc::Receiver<Result<crate::folder::Stats, String>>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for StatsSlot {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// What a thread handing an image to Windows did.
enum Handed {
    Wallpaper(Result<(), String>),
    /// The files to show the Print Pictures dialog for.
    Print(Result<Vec<PathBuf>, String>),
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
        // A wheel notch scrolls 60 points instead of egui's 40.
        ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
            o.input_options.max_double_click_delay = win::double_click_time();
            o.input_options.line_scroll_speed = 60.0;
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
            let normal = cc.storage.and_then(|s| s.get_string(WINDOW_NORMAL_KEY)).and_then(|v| {
                let n: Vec<i32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                <[i32; 4]>::try_from(n).ok()
            });
            win::show_maximized(hwnd, normal, crate::DEFAULT_SIZE);
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
            deep: false,
            starts: Vec::new(),
            by_folder: cc.storage.and_then(|s| s.get_string(BY_FOLDER_KEY)).as_deref() == Some("true"),
            scan: None,
            sort: Order {
                key: cc.storage.and_then(|s| s.get_string(SORT_KEY)).and_then(|v| SortKey::from_name(&v)).unwrap_or_default(),
                descending: cc.storage.and_then(|s| s.get_string(SORT_DESCENDING_KEY)).as_deref() == Some("true"),
            },
            favorites_sort: Order {
                key: cc
                    .storage
                    .and_then(|s| s.get_string(FAVORITES_SORT_KEY))
                    .and_then(|v| SortKey::from_name(&v))
                    .unwrap_or(SortKey::Added),
                descending: cc.storage.and_then(|s| s.get_string(FAVORITES_SORT_DESCENDING_KEY)).as_deref() == Some("true"),
            },
            place: None,
            centre_after_scan: false,
            refocus: None,
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
            checker: flag(CHECKER_KEY),
            checker_texture: None,
            filter: cc
                .storage
                .and_then(|s| s.get_string(FILTER_KEY))
                .and_then(|v| view::Filter::from_name(&v))
                .unwrap_or(view::Filter::Bilinear),
            programs: HashMap::new(),
            program_due: false,
            confirm_delete: None,
            delete_folders: 0,
            rename: None,
            batch_rename: None,
            selection: Selection::default(),
            editor: cc
                .storage
                .and_then(|s| Some(Editor { id: s.get_string(EDITOR_KEY)?, name: s.get_string(EDITOR_NAME_KEY)? }))
                .filter(|e| !e.id.is_empty()),
            editors: HashMap::new(),
            menu_editors: Vec::new(),
            menu_folders: Vec::new(),
            sibling_folders: None,
            new_folder: None,
            opening: None,
            confirm_edit: None,
            undo: Vec::new(),
            crop: None,
            saving: None,
            reloading: None,
            history: History::default(),
            favorites: Favorites::load(crate::settings_dir().map(|d| d.join(favorites::FILE))),
            pinned: Favorites::load_pinned(crate::settings_dir().map(|d| d.join(favorites::PINNED_FILE))),
            confirm_clear_favorites: false,
            confirm_unpin_all: false,
            favorites_gone: None,
            pinned_gone: None,
            copying: None,
            moving: None,
            listed: Vec::new(),
            name_filter: String::new(),
            filter_focused: false,
            filter_open: false,
            archive: false,
            clipboard: None,
            handing: None,
            listed_folders: Vec::new(),
            folders: Vec::new(),
            folders_folded: cc.storage.and_then(|s| s.get_string(FOLDERS_KEY)).as_deref() == Some("true"),
            folder_focus: None,
            gallery: None,
            gallery_open: false,
            image_clicked: false,
            thumb_size: number(THUMB_SIZE_KEY)
                .unwrap_or(gallery::DEFAULT_SIZE)
                .clamp(gallery::MIN_SIZE, gallery::MAX_SIZE),
            tree_width: number(TREE_WIDTH_KEY).unwrap_or(240.0).clamp(140.0, 640.0),
            show_info: cc.storage.and_then(|s| s.get_string(INFO_KEY)).as_deref() == Some("true"),
            info_width: number(INFO_WIDTH_KEY).unwrap_or(280.0).clamp(200.0, 600.0),
            slideshow: None,
            slideshow_seconds: number(SLIDESHOW_SECONDS_KEY).map_or(5, |s| (s as u32).clamp(1, 3600)),
            slideshow_loop: cc.storage.and_then(|s| s.get_string(SLIDESHOW_LOOP_KEY)).as_deref() != Some("false"),
            info: None,
            show_histogram: cc.storage.and_then(|s| s.get_string(HISTOGRAM_KEY)).as_deref() == Some("true"),
            histogram: None,
            show_colour: cc.storage.and_then(|s| s.get_string(COLOUR_KEY)).as_deref() == Some("true"),
            colour_pixels: None,
            picked: None,
            menu_picked: None,
            stats: None,
            histogram_channels: cc
                .storage
                .and_then(|s| s.get_string(HISTOGRAM_CHANNELS_KEY))
                .and_then(|name| crate::histogram::Channels::from_name(&name))
                .unwrap_or(crate::histogram::Channels::Rgb),
            thumb_fill: cc.storage.and_then(|s| s.get_string(THUMB_FILL_KEY)).as_deref() == Some("true"),
            thumb_aspect: match cc.storage.and_then(|s| s.get_string(THUMB_ASPECT_KEY)).as_deref() {
                Some(AUTO) => None,
                v => Some(v.and_then(|v| gallery::ASPECTS.iter().find(|(n, _)| *n == v)).map_or(1.0, |(_, a)| *a)),
            },
            lang,
            updates: crate::update::Updates::new(
                cc.storage.and_then(|s| s.get_string(CHECK_UPDATES_KEY)).as_deref() == Some("true"),
                cc.storage.and_then(|s| s.get_string(LAST_UPDATE_CHECK_KEY)).and_then(|v| v.parse().ok()).unwrap_or(0),
            ),
            deleting: None,
            deleting_folders: Vec::new(),
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
            fullscreen: false,
            cloak,
            player: None,
        };
        if let Some(path) = initial {
            app.open(ctx, path);
        }
        app.updates.start_if_due(ctx);
        win::watch_paste(app.hwnd.unwrap_or(0));
        if app.favorites.len() > 0 {
            app.favorites_gone = Some(folder::find_gone(app.favorites.paths(), ctx.clone()));
        }
        app.check_pinned(ctx);
        app
    }

    /// Show `path`, a file, and list its folder for browsing; a folder is
    /// shown in the gallery.
    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if path.is_dir() {
            self.open_folder(ctx, path, false);
            self.enter_gallery(ctx);
            return;
        }
        if self.gallery_open {
            self.leave_gallery();
        }
        // An archive: its first image, as a comic book reader opens it.
        if crate::archive::is_archive_file(&path) {
            self.open_folder(ctx, path, false);
            return;
        }
        // An image in an archive is listed with the archive's others.
        let dir = match crate::archive::split(&path) {
            Some((archive, _)) => Some(archive),
            None => path.parent().map(Path::to_path_buf),
        };
        let listed = self.scan.is_none()
            && !self.deep
            && matches!((&self.dir, &dir), (Some(a), Some(b)) if folder::same_path(a, b));
        if let Some(dir) = dir.as_deref().filter(|_| !listed) {
            self.leave_for(dir);
        }
        self.index = if listed { folder::position(&self.files, &path) } else { None };
        self.set_current(Some(path.clone()));
        if self.index.is_none()
            && let Some(dir) = dir
        {
            self.start_scan(ctx, dir, false, Some(path));
        }
    }

    /// List `dir`, with `deep` its sub-folders too; its first image becomes
    /// the current one.
    pub fn open_folder(&mut self, ctx: &egui::Context, dir: PathBuf, deep: bool) {
        if self.deep == deep && self.dir.as_deref().is_some_and(|d| folder::same_path(d, &dir)) {
            return;
        }
        self.leave_for(&dir);
        self.set_current(None);
        // The folder above chosen (in the tree): the one left has the
        // cursor there, as with Up.
        let left = self.dir.clone().filter(|d| d.parent().is_some_and(|p| folder::same_path(p, &dir)));
        self.start_scan(ctx, dir, deep, None);
        if left.is_some()
            && let Some(gallery) = &mut self.gallery
        {
            gallery.scroll = Some(Scroll::Centre);
        }
        self.refocus = left;
    }

    /// The folder on screen as Back would return to it.
    fn here(&self) -> Option<Place> {
        let dir = self.dir.clone()?;
        // The sub-folder with the cursor, if any.
        let folder = self.focused_folder().filter(|_| self.gallery_open);
        // Where the row of the cell with the cursor is on screen, once the
        // grid shows this listing.
        let below = match (&self.gallery, self.folder_focus.filter(|_| folder.is_some()), self.index) {
            (Some(g), Some(k), _) if self.scan.is_none() => Some(g.layout.folder_pos(k).y - g.top),
            (Some(g), None, Some(i)) if self.gallery_open && self.scan.is_none() => Some(g.layout.cell_pos(i).y - g.top),
            _ => None,
        };
        Some(Place { dir, deep: self.deep, current: self.current.clone(), folder, below })
    }

    /// Remember the folder on screen when `dir` is to replace it.
    fn leave_for(&mut self, dir: &Path) {
        if self.dir.as_deref().is_some_and(|d| !folder::same_path(d, dir))
            && let Some(here) = self.here()
        {
            self.history.visit(here);
        }
    }

    /// Show `place` again (Back, Forward): listed as it was, its image
    /// current and where it was on screen.
    fn return_to(&mut self, ctx: &egui::Context, place: Place) {
        let keep = place.current.filter(|p| crate::archive::is_file(p));
        self.set_current(keep.clone());
        self.start_scan(ctx, place.dir.clone(), place.deep, keep);
        self.refocus = place.folder;
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.reveal(&crate::archive::tree_folder(&place.dir));
            gallery.scroll = Some(place.below.map_or(Scroll::Centre, Scroll::Keep));
        }
    }

    /// The folder above the one on screen, if there is one.
    pub fn parent_dir(&self) -> Option<PathBuf> {
        self.dir.as_deref().filter(|d| !favorites::is_virtual(d)).and_then(Path::parent).map(Path::to_path_buf)
    }

    /// The current image's own folder can be gone to: the favourites or
    /// the sub-folders are listed.
    pub fn can_go_to_folder(&self) -> bool {
        self.mixed() && !self.archive && self.current.is_some()
    }

    /// List the current image's folder alone, as opening it would, the
    /// image staying current; Back returns.
    fn go_to_folder(&mut self, ctx: &egui::Context) {
        let Some(path) = self.current.clone().filter(|_| self.can_go_to_folder()) else { return };
        let Some(dir) = path.parent().map(Path::to_path_buf) else { return };
        self.leave_for(&dir);
        self.start_scan(ctx, dir.clone(), false, Some(path));
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.reveal(&crate::archive::tree_folder(&dir));
            gallery.scroll = Some(Scroll::Centre);
        }
    }

    /// Show the folder above (Alt+↑). With the sub-folders, the current
    /// image is among its images and stays current.
    fn go_up(&mut self, ctx: &egui::Context) {
        let Some(parent) = self.parent_dir() else { return };
        self.leave_for(&parent);
        let keep = self.current.clone().filter(|_| self.deep);
        if keep.is_none() {
            self.set_current(None);
        }
        // The folder left has the cursor there, as in Explorer.
        let left = self.dir.clone();
        self.start_scan(ctx, parent.clone(), self.deep, keep);
        self.refocus = left;
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.reveal(&parent);
            gallery.scroll = Some(Scroll::Centre);
        }
    }

    /// List `dir` again with or without its sub-folders. The current image
    /// stays if the new listing has it; otherwise the first one is current.
    pub fn set_deep(&mut self, ctx: &egui::Context, deep: bool) {
        let Some(dir) = self.dir.clone() else {
            self.deep = deep;
            return;
        };
        let keep = self.current.clone().filter(|c| deep || c.parent().is_some_and(|p| folder::same_path(p, &dir)));
        if keep.is_none() {
            self.set_current(None);
        }
        self.centre_after_scan = true;
        self.start_scan(ctx, dir, deep, keep);
    }

    /// What `files` lists, as a key for what the gallery keeps per folder
    /// (the scroll position, the Auto proportions): `dir`, and with its
    /// sub-folders `dir\*`, which no folder can be called.
    pub fn listing(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| if self.deep && !favorites::is_virtual(d) { d.join("*") } else { d.clone() })
    }

    /// The favourites are listed in place of a folder.
    pub fn in_favorites(&self) -> bool {
        self.dir.as_deref().is_some_and(favorites::is_dir)
    }

    /// Quick Access, the pinned folders as cells, is shown in place of a
    /// folder.
    pub fn in_quick_access(&self) -> bool {
        self.dir.as_deref().is_some_and(favorites::is_pinned_dir)
    }

    /// `files` come from several folders: the sub-folders of `dir`, or the
    /// favourites.
    pub fn mixed(&self) -> bool {
        self.deep || self.in_favorites() || self.archive
    }

    /// The images listed are those of a ZIP archive: read-only.
    pub fn in_archive(&self) -> bool {
        self.archive
    }

    /// The name of `path` for display: with the sub-folders listed, its
    /// path from `dir`; among the favourites, its whole path.
    pub fn display_name(&self, path: &Path) -> String {
        if self.in_favorites() {
            return path.display().to_string();
        }
        match self.dir.as_deref().filter(|_| self.deep || self.archive).and_then(|d| path.strip_prefix(d).ok()) {
            Some(relative) => relative.to_string_lossy().into_owned(),
            None => file_name(path),
        }
    }

    /// What of the listing passes the gallery's filter.
    fn filtered(&self) -> Vec<PathBuf> {
        if self.name_filter.trim().is_empty() {
            return self.listed.clone();
        }
        self.listed.iter().filter(|p| folder::matches(&self.display_name(p), &self.name_filter)).cloned().collect()
    }

    /// The sub-folders whose names pass the gallery's filter.
    fn filtered_folders(&self) -> Vec<PathBuf> {
        let filter = self.name_filter.trim();
        self.listed_folders.iter().filter(|p| filter.is_empty() || folder::matches(&file_name(p), filter)).cloned().collect()
    }

    /// The folder that can be pinned to Quick Access from the menus: the
    /// one listed, a real folder.
    pub fn pinnable_dir(&self) -> Option<PathBuf> {
        self.dir.clone().filter(|d| !favorites::is_virtual(d) && !self.archive)
    }

    /// Pin `dir` to Quick Access, or unpin it if it is pinned.
    pub fn toggle_pin(&mut self, dir: PathBuf) {
        // `D:\Photos\` (a path from the command line) is `D:\Photos`.
        let dir: PathBuf = dir.components().collect();
        let name = folder_label(&dir);
        let text = match self.pinned.toggle(&dir) {
            Ok(true) => tr!(format!("Pinned to Quick Access: {name}"), format!("Закреплено на панели быстрого доступа: {name}")),
            Ok(false) => tr!(format!("Unpinned from Quick Access: {name}"), format!("Откреплено от панели быстрого доступа: {name}")),
            Err(e) => e,
        };
        self.notice(text);
        self.pinned_changed();
    }

    /// Give the pinned folder `dir` the key `key` (Alt+1 to Alt+9), taken
    /// from the folder that had it, or none.
    pub fn set_pin_key(&mut self, dir: PathBuf, key: Option<u8>) {
        let dir: PathBuf = dir.components().collect();
        let name = folder_label(&dir);
        let text = match (self.pinned.set_key(&dir, key), key) {
            (Ok(()), Some(k)) => tr!(format!("Alt+{k} moves the image to {name}"), format!("Alt+{k} перемещает изображение в {name}")),
            (Ok(()), None) => tr!(format!("No key for {name}"), format!("Клавиша для {name} снята")),
            (Err(e), _) => e,
        };
        self.notice(text);
        self.pinned_changed();
    }

    /// The pinned folders or their keys have changed: the tree and Quick
    /// Access's grid show them anew.
    fn pinned_changed(&mut self) {
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.set_pinned(&self.pinned.entries());
        }
        if self.in_quick_access() && self.scan.is_none() {
            self.listed_folders = self.pinned.paths();
            self.filter_changed();
        }
    }

    /// The sub-folder whose cell has the gallery's cursor.
    pub fn focused_folder(&self) -> Option<PathBuf> {
        self.folder_focus.and_then(|k| self.folders.get(k).cloned())
    }

    /// Open a sub-folder from its cell or its header, as the tree would.
    pub fn open_subfolder(&mut self, ctx: &egui::Context, dir: PathBuf) {
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.reveal(&dir);
        }
        self.open_folder(ctx, dir, self.deep);
    }

    /// The gallery's filter has changed: `files` is the listing filtered
    /// anew (not listed again); the current image stays if it passes, else
    /// the first that does is current, and with none passing it stays (the
    /// filter cleared, it is there again).
    pub fn filter_changed(&mut self) {
        self.files = self.filtered();
        self.folders = self.filtered_folders();
        self.folder_focus = None;
        self.starts = folder::starts(&self.files);
        self.selection.retain_listed(&self.files, &self.folders);
        self.index = self.current.as_deref().and_then(|c| folder::position(&self.files, c));
        if self.index.is_none() && !self.files.is_empty() {
            self.go(0);
        }
        if let Some(gallery) = &mut self.gallery {
            gallery.scroll = Some(Scroll::Visible);
        }
    }

    /// How many images the listing has, before the filter.
    pub fn listed_count(&self) -> usize {
        self.listed.len()
    }

    /// Show the gallery of the folder of the current file, out of full
    /// screen (Ctrl+Shift+F there brings it back, bars kept).
    fn enter_gallery(&mut self, ctx: &egui::Context) {
        self.stop_slideshow();
        if Self::is_fullscreen(ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        let gallery = self.gallery.get_or_insert_with(|| Gallery::new(ctx));
        gallery.tree.set_pinned(&self.pinned.entries());
        if let Some(dir) = &self.dir {
            gallery.tree.reveal(&crate::archive::tree_folder(dir));
        }
        gallery.scroll = Some(Scroll::Centre);
        self.gallery_open = true;
    }

    /// Back to the image, the current one.
    pub fn leave_gallery(&mut self) {
        self.gallery_open = false;
        self.selection.clear();
        self.folder_focus = None;
        self.image_clicked = false;
        if let Some(gallery) = &mut self.gallery {
            gallery.want(Vec::new());
            gallery.hovered = None;
        }
    }

    fn start_scan(&mut self, ctx: &egui::Context, dir: PathBuf, deep: bool, keep: Option<PathBuf>) {
        let same = self.deep == deep && self.dir.as_deref().is_some_and(|d| folder::same_path(d, &dir));
        self.place = self.index.filter(|_| same);
        // Set again after this by those returning to a folder.
        self.refocus = None;
        self.sibling_folders = None;
        crate::archive::release_unless(&dir);
        if !same {
            self.selection.clear();
        }
        if !self.dir.as_deref().is_some_and(|d| folder::same_path(d, &dir)) {
            self.name_filter.clear();
        }
        self.listed.clear();
        self.files.clear();
        self.starts.clear();
        self.listed_folders.clear();
        self.folders.clear();
        self.folder_focus = None;
        self.index = None;
        self.archive = crate::archive::is_archive_file(&dir);
        self.dir = Some(dir.clone());
        // An archive's folders are listed with it, by folder or not.
        self.deep = deep && !self.archive;
        self.scan = Some(self.scan_dir(dir, keep, ctx));
    }

    /// List `dir`, or the favourites, as they are to be ordered, on a thread.
    /// Quick Access has no images: its cells are the pinned folders, put
    /// in by `poll_scan`.
    fn scan_dir(&self, dir: PathBuf, keep: Option<PathBuf>, ctx: &egui::Context) -> Scan {
        if favorites::is_dir(&dir) {
            folder::scan_files(self.favorites.paths(), self.favorites_sort, self.by_folder, ctx.clone())
        } else if favorites::is_pinned_dir(&dir) {
            folder::scan_files(Vec::new(), self.favorites_sort, false, ctx.clone())
        } else if crate::archive::is_archive_file(&dir) {
            folder::scan_archive(dir, self.sort, self.by_folder, ctx.clone())
        } else {
            folder::scan(dir, self.depth(), keep, self.sort, ctx.clone())
        }
    }

    /// What the listing of `dir` holds: with the sub-folders, by folder
    /// the order applies within each folder, otherwise through all.
    fn depth(&self) -> folder::Depth {
        match (self.deep, self.by_folder) {
            (false, _) => folder::Depth::Folder,
            (true, true) => folder::Depth::ByFolder,
            (true, false) => folder::Depth::Flat,
        }
    }

    /// The order of what is listed: the folders' or the favourites'.
    pub fn order(&self) -> Order {
        if self.in_favorites() { self.favorites_sort } else { self.sort }
    }

    /// List the folder again in `order`. The old listing stays until then,
    /// so the image and the gallery stay on screen.
    fn sort_by(&mut self, ctx: &egui::Context, order: Order) {
        if order == self.order() {
            return;
        }
        if self.in_favorites() {
            self.favorites_sort = order;
        } else {
            self.sort = order;
        }
        // The sub-folders' cells show their first images in the order.
        if let Some(gallery) = &mut self.gallery {
            gallery.forget_previews();
        }
        self.relist(ctx);
    }

    /// List the folder again as it is now to be ordered (the order, or "By
    /// folder" with the sub-folders); the old listing stays until then.
    pub fn relist(&mut self, ctx: &egui::Context) {
        if let Some(dir) = self.dir.clone() {
            self.place = self.index;
            self.centre_after_scan = true;
            self.scan = Some(self.scan_dir(dir, self.current.clone(), ctx));
        }
    }

    /// A modal dialog is open: keys and the wheel are its own.
    pub fn modal_open(&self) -> bool {
        self.confirm_delete.is_some()
            || self.rename.is_some()
            || self.new_folder.is_some()
            || self.batch_rename.is_some()
            || self.confirm_edit.is_some()
            || self.dialog.is_some()
            || self.confirm_clear_favorites
            || self.confirm_unpin_all
    }

    /// Rename `path` to `name` in its folder, so that Ctrl+Z can undo it;
    /// why not, if it cannot be.
    pub fn rename_to(&mut self, ctx: &egui::Context, path: &Path, name: &str) -> Result<(), String> {
        folder::check_name(name)?;
        let new = path.with_file_name(name);
        if new == path {
            return Ok(());
        }
        self.move_file(ctx, path, &new)?;
        self.push_undo(Undo::Rename(vec![(path.to_path_buf(), new)]));
        Ok(())
    }

    /// Rename `paths` to `base` and a number from `start` each (see
    /// `rename::numbered`), all or none, so that Ctrl+Z can undo it; why
    /// not, if they cannot be.
    pub fn rename_batch(&mut self, ctx: &egui::Context, paths: &[PathBuf], dirs: usize, base: &str, start: u32) -> Result<(), String> {
        let news = crate::rename::numbered(paths, dirs, base, start);
        let pairs: Vec<(PathBuf, PathBuf)> = paths.iter().cloned().zip(news).filter(|(old, new)| old != new).collect();
        if pairs.is_empty() {
            return Ok(());
        }
        crate::rename::check(&pairs)?;
        crate::rename::rename_all(&pairs)?;
        self.moved(&pairs);
        self.list_again(ctx);
        let n = pairs.len();
        self.notice(if dirs > 0 {
            tr!(format!("Renamed: {n}"), format!("Переименовано: {n}"))
        } else {
            tr!(format!("Files renamed: {n}"), format!("Переименовано файлов: {n}"))
        });
        self.push_undo(Undo::Rename(pairs));
        Ok(())
    }

    /// Remember `undo` for Ctrl+Z, forgetting the oldest beyond the limits.
    fn push_undo(&mut self, undo: Undo) {
        self.undo.push(undo);
        let bytes = |u: &Undo| match u {
            Undo::Save(files) => files.iter().filter_map(|(_, b)| b.as_ref()).map(|b| b.bytes.len()).sum(),
            Undo::Rename(_) | Undo::Move { .. } | Undo::Copy { .. } => 0,
        };
        while self.undo.len() > UNDO_STEPS
            || (self.undo.len() > 1 && self.undo.iter().map(bytes).sum::<usize>() > UNDO_BYTES)
        {
            self.undo.remove(0);
        }
    }

    /// Undo the last rename, move, copy or save (Ctrl+Z): the old name or
    /// place back, the copies deleted, or the old contents (a file made by
    /// saving is deleted).
    fn undo(&mut self, ctx: &egui::Context) {
        if self.saving.is_some() {
            self.notice(tr!("Saving…".into(), "Сохранение…".into()));
            return;
        }
        if self.moving.is_some() || self.copying.is_some() {
            self.notice(tr!("Still moving or copying…".into(), "Перемещение или копирование ещё идёт…".into()));
            return;
        }
        match self.undo.pop() {
            None => {}
            Some(Undo::Move { pairs, created }) => {
                let back: Vec<(PathBuf, PathBuf)> = pairs.iter().map(|(old, new)| (new.clone(), old.clone())).collect();
                self.move_pairs(ctx, back, false, None);
                // The folder made for them goes once they are out of it:
                // now, or when the shell has moved them (`poll_move`).
                if let Some(moving) = &mut self.moving {
                    moving.remove_empty = created;
                } else if let Some(dir) = created {
                    self.remove_if_empty(ctx, &dir);
                }
            }
            Some(Undo::Copy { copies, created }) => {
                let (mut removed, mut error) = (Vec::new(), None);
                for path in &copies {
                    match std::fs::remove_file(path) {
                        Ok(()) => removed.push(path.clone()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            error.get_or_insert(format!("{}: {e}", file_name(path)));
                        }
                    }
                }
                self.forget_files(&removed);
                for path in &removed {
                    self.unlist(path);
                }
                if self.scan.is_some() {
                    self.list_again(ctx);
                }
                let text = match error {
                    None => tr!("Copy undone".into(), "Копирование отменено".into()),
                    Some(e) => tr!(format!("Cannot undo the copy of {e}"), format!("Не удалось отменить копирование {e}")),
                };
                self.notice(text);
                if let Some(dir) = created {
                    self.remove_if_empty(ctx, &dir);
                }
            }
            Some(Undo::Rename(pairs)) => {
                let back: Vec<(PathBuf, PathBuf)> = pairs.iter().map(|(old, new)| (new.clone(), old.clone())).collect();
                let text = match crate::rename::check(&back).and_then(|()| crate::rename::rename_all(&back)) {
                    Ok(()) => {
                        self.moved(&back);
                        self.list_again(ctx);
                        tr!("Rename undone", "Переименование отменено").into()
                    }
                    Err(e) => tr!(format!("Cannot rename back: {e}"), format!("Не удалось вернуть имена: {e}")),
                };
                self.notice(text);
            }
            Some(Undo::Save(files)) => {
                let (mut restored, mut removed, mut error) = (Vec::new(), Vec::new(), None);
                for (path, before) in &files {
                    let result = match before {
                        Some(before) => edit::restore(path, before).map(|()| restored.push(path.clone())),
                        None => std::fs::remove_file(path).map(|()| removed.push(path.clone())).map_err(|e| e.to_string()),
                    };
                    if let Err(e) = result {
                        error.get_or_insert(format!("{}: {e}", file_name(path)));
                    }
                }
                self.written(ctx, &restored);
                self.forget_files(&removed);
                for path in &removed {
                    self.unlist(path);
                }
                let text = match error {
                    None if files.len() == 1 => {
                        let name = file_name(&files[0].0);
                        tr!(format!("Save undone: {name}"), format!("Сохранение отменено: {name}"))
                    }
                    None => tr!("Saves undone".into(), "Сохранения отменены".into()),
                    Some(e) => tr!(format!("Cannot undo the save of {e}"), format!("Не удалось отменить сохранение {e}")),
                };
                self.notice(text);
            }
        }
    }

    /// What Ctrl+Z would undo, for the menu.
    pub fn undo_label(&self) -> &'static str {
        match self.undo.last() {
            Some(Undo::Save(_)) => tr!("Undo Save", "Отменить сохранение"),
            Some(Undo::Move { .. }) => tr!("Undo Move", "Отменить перемещение"),
            Some(Undo::Copy { .. }) => tr!("Undo Copy", "Отменить копирование"),
            _ => tr!("Undo Rename", "Отменить переименование"),
        }
    }

    /// Drop what was decoded of `paths`, the images and their thumbnails;
    /// the picture on screen stays until it is decoded again.
    fn forget_files(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        for path in paths {
            self.cache.remove(path);
            self.partial.remove(path);
        }
        self.pending.retain(|d| !paths.contains(&d.path));
        self.loader.forget(paths);
        if let Some(gallery) = &mut self.gallery {
            gallery.forget(paths);
        }
        if self.info.as_ref().is_some_and(|i| paths.contains(&i.path)) {
            self.info = None;
        }
        if self.histogram.as_ref().is_some_and(|h| paths.contains(&h.path)) {
            self.histogram = None;
        }
        if self.colour_pixels.as_ref().is_some_and(|c| paths.contains(&c.path)) {
            self.colour_pixels = None;
        }
    }

    /// Read the current image's information for the panel, on a thread,
    /// when it is shown; take it once read.
    fn update_info(&mut self, ctx: &egui::Context) {
        if !self.show_info {
            return;
        }
        let Some(path) = self.current.clone() else {
            self.info = None;
            return;
        };
        // One reading at a time: while browsing fast, the images passed
        // over are not read (each would take a thread, COM and a codec).
        let reading = self.info.as_ref().is_some_and(|i| i.rx.is_some());
        if !reading && self.info.as_ref().is_none_or(|i| i.path != path) {
            let (tx, rx) = mpsc::channel();
            let (ctx, file) = (ctx.clone(), path.clone());
            std::thread::spawn(move || {
                // The header may be read by Windows' codecs.
                let _com = win::com_init();
                if tx.send(crate::info::read(&file)).is_ok() {
                    ctx.request_repaint();
                }
            });
            self.info = Some(InfoSlot { path, info: None, rx: Some(rx) });
        }
        if let Some(slot) = &mut self.info
            && let Some(Ok(info)) = slot.rx.as_ref().map(mpsc::Receiver::try_recv)
        {
            slot.info = Some(std::sync::Arc::new(info));
            slot.rx = None;
        }
    }

    /// Histograms are counted while the panel shows its Histogram section
    /// (`shown`): the decoder threads count one for every image they
    /// decode from then on (`Meta::histogram`); the current image, when it
    /// was decoded before, is read again and counted on a thread, one at a
    /// time as `update_info` reads. Nothing is counted or kept otherwise.
    pub(crate) fn update_histogram(&mut self, ctx: &egui::Context, shown: bool) {
        let wanted = shown && self.show_histogram;
        self.loader.set_histograms(wanted);
        let Some(path) = self.current.clone().filter(|_| wanted) else {
            self.histogram = None;
            return;
        };
        if let Some(slot) = &mut self.histogram
            && let Some(Ok(histogram)) = slot.rx.as_ref().map(mpsc::Receiver::try_recv)
        {
            slot.histogram = histogram.map(Arc::new);
            slot.rx = None;
        }
        let counting = self.histogram.as_ref().is_some_and(|h| h.rx.is_some());
        let without = matches!(self.cache.get(&path), Some(Slot::Ready(p)) if p.meta.histogram.is_none());
        if !counting && without && self.histogram.as_ref().is_none_or(|h| h.path != path) {
            let (tx, rx) = mpsc::channel();
            let (ctx, file) = (ctx.clone(), path.clone());
            std::thread::spawn(move || {
                let _com = win::com_init();
                let start = Instant::now();
                let histogram = crate::loader::read(&file).ok().map(|(img, _)| crate::histogram::Histogram::of_image(&img));
                log::debug!("histogram of {} read again in {:.1} ms", file.display(), start.elapsed().as_secs_f64() * 1e3);
                if tx.send(histogram).is_ok() {
                    ctx.request_repaint();
                }
            });
            self.histogram = Some(HistogramSlot { path, histogram: None, rx: Some(rx) });
        }
    }

    /// While the panel shows its Colour section (`shown`), the current
    /// image is read again on a thread and kept decoded at full size, so
    /// that the pixel under the pointer is found at once; one reading at a
    /// time, as `update_info` reads. Nothing is read or kept otherwise.
    pub(crate) fn update_colour(&mut self, ctx: &egui::Context, shown: bool) {
        let Some(path) = self.current.clone().filter(|_| shown && self.show_colour) else {
            self.colour_pixels = None;
            self.picked = None;
            return;
        };
        if self.gallery_open {
            // No image area: no pointer over the image.
            self.picked = None;
        }
        if let Some(slot) = &mut self.colour_pixels
            && let Some(Ok(image)) = slot.rx.as_ref().map(mpsc::Receiver::try_recv)
        {
            slot.image = image.map(Arc::new);
            slot.rx = None;
        }
        let reading = self.colour_pixels.as_ref().is_some_and(|c| c.rx.is_some());
        if !reading && self.colour_pixels.as_ref().is_none_or(|c| c.path != path) {
            // The previous image goes now, not after the next is read.
            self.colour_pixels = None;
            let (tx, rx) = mpsc::channel();
            let (ctx, file) = (ctx.clone(), path.clone());
            std::thread::spawn(move || {
                let _com = win::com_init();
                let start = Instant::now();
                let image = crate::loader::read(&file).ok().map(|(img, _)| img);
                log::debug!("pixels of {} read again in {:.1} ms", file.display(), start.elapsed().as_secs_f64() * 1e3);
                if tx.send(image).is_ok() {
                    ctx.request_repaint();
                }
            });
            self.colour_pixels = Some(PixelSlot { path, image: None, rx: Some(rx) });
        }
    }

    /// What the panel is about in the gallery besides the current image:
    /// the folder cell with the cursor, or more than one image chosen.
    pub(crate) fn stats_of(&self) -> Option<StatsOf> {
        if !self.gallery_open {
            return None;
        }
        if let Some(k) = self.folder_focus {
            return self.folders.get(k).cloned().map(StatsOf::Folder);
        }
        (self.selection.len() > 1).then(|| StatsOf::Files(self.targets()))
    }

    /// Count the focused folder's images or the chosen ones on a thread
    /// while the panel is `shown`. One count at a time: when what is wanted
    /// changes (a frame dragged over the grid changes it every frame), the
    /// count under way is cancelled and the next starts once it has
    /// stopped.
    pub(crate) fn update_stats(&mut self, ctx: &egui::Context, shown: bool) {
        let Some(of) = self.stats_of().filter(|_| shown) else {
            self.stats = None;
            return;
        };
        if let Some(slot) = &mut self.stats
            && let Some(Ok(stats)) = slot.rx.as_ref().map(mpsc::Receiver::try_recv)
        {
            slot.stats = Some(stats);
            slot.rx = None;
        }
        if let Some(slot) = &self.stats {
            if slot.of == of {
                return;
            }
            if slot.rx.is_some() {
                slot.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (ctx, what, stop) = (ctx.clone(), of.clone(), cancel.clone());
        std::thread::spawn(move || {
            // The dates taken may be read by libheif or the `image` crate.
            let _com = win::com_init();
            let start = Instant::now();
            let stats = match &what {
                StatsOf::Folder(dir) => crate::folder::folder_stats(dir, &stop).map_err(|e| e.to_string()),
                StatsOf::Files(paths) => Ok(crate::folder::files_stats(paths, &stop)),
            };
            log::debug!("counted {:?} in {:.1} ms", stats.as_ref().map(|s| s.images), start.elapsed().as_secs_f64() * 1e3);
            if tx.send(stats).is_ok() {
                ctx.request_repaint();
            }
        });
        self.stats = Some(StatsSlot { of, stats: None, rx: Some(rx), cancel });
    }

    /// The pixel of the image on screen under `pointer`, the picture shown
    /// at `place`; None without kept pixels of it, or off the image.
    fn pick(&self, pointer: Option<egui::Pos2>, place: Rect, picture_path: &Path, size: Vec2) -> Option<Picked> {
        let slot = self.colour_pixels.as_ref().filter(|c| c.path == picture_path)?;
        let image = slot.image.as_ref()?;
        let (uv, s) = view::image_point(pointer?, place, self.view.turns, self.view.flip)?;
        let (w, h) = (image.width(), image.height());
        let at = |v: f32, n: u32| ((v * n as f32) as u32).min(n.saturating_sub(1));
        use image::GenericImageView;
        let rgba = image.get_pixel(at(uv.x, w), at(uv.y, h)).0;
        let shown = self.view.rotated(size);
        Some(Picked { x: at(s.x, shown.x as u32), y: at(s.y, shown.y as u32), rgba })
    }

    /// The current image's histogram for the panel: None when there is
    /// none (no image, not decodable), Some(None) while it is counted.
    pub(crate) fn current_histogram(&self) -> Option<Option<Arc<crate::histogram::Histogram>>> {
        let path = self.current.as_ref()?;
        match self.cache.get(path) {
            Some(Slot::Ready(p)) if p.meta.histogram.is_some() => return Some(p.meta.histogram.clone()),
            Some(Slot::Failed(_)) => return None,
            _ => {}
        }
        match &self.histogram {
            Some(h) if h.path == *path && h.rx.is_none() => h.histogram.clone().map(Some),
            _ => Some(None),
        }
    }

    /// `paths` were written: they are decoded again, the current image
    /// without the view's rotation (it is in the file now), and the folder
    /// listed again, since their dates and sizes changed or they are new
    /// there.
    fn written(&mut self, ctx: &egui::Context, paths: &[PathBuf]) {
        self.forget_files(paths);
        if let Some(current) = self.current.as_ref().filter(|c| paths.contains(c)) {
            self.reloading = Some(current.clone());
        }
        let shown_here = |path: &PathBuf| {
            let in_dir = path.parent().zip(self.dir.as_deref()).is_some_and(|(p, d)| folder::same_path(p, d));
            in_dir || folder::position(&self.files, path).is_some()
        };
        if paths.iter().any(shown_here) {
            self.list_again(ctx);
        }
    }

    /// List the folder again, the current image staying current.
    fn list_again(&mut self, ctx: &egui::Context) {
        if let Some(dir) = self.dir.clone() {
            self.place = self.index;
            self.scan = Some(self.scan_dir(dir, self.current.clone(), ctx));
        }
    }

    /// Rename `path` to `new`, unless another file has that name.
    fn move_file(&mut self, ctx: &egui::Context, path: &Path, new: &Path) -> Result<(), String> {
        // A change of case only is a rename too.
        if !folder::same_path(new, path) && new.exists() {
            return Err(tr!("A file or folder with this name already exists", "Файл или папка с таким именем уже существует").into());
        }
        std::fs::rename(path, new).map_err(|e| tr!(format!("Cannot rename: {e}"), format!("Не удалось переименовать: {e}")))?;
        self.moved(&[(path.to_path_buf(), new.to_path_buf())]);
        // Its place in the order may have changed.
        self.list_again(ctx);
        Ok(())
    }

    /// Each `old` is now its `new`: what was decoded for it, its thumbnail,
    /// its place among the favourites and the chosen images go with it.
    /// Every old name is taken out before any new one goes in, since in a
    /// batch (01 to 02, 02 to 03) a new name may be another pair's old one.
    fn moved(&mut self, pairs: &[(PathBuf, PathBuf)]) {
        // A folder renamed: everything known in it goes with it.
        let folders: Vec<(PathBuf, PathBuf)> = pairs.iter().filter(|(_, new)| new.is_dir()).cloned().collect();
        let expanded;
        let pairs = if folders.is_empty() {
            pairs
        } else {
            expanded = self.within_folders(pairs, &folders);
            &expanded[..]
        };
        self.selection.renamed(pairs);
        let places: Vec<(usize, &PathBuf)> =
            pairs.iter().filter_map(|(old, new)| folder::position(&self.files, old).map(|i| (i, new))).collect();
        for (i, new) in places {
            self.files[i] = new.clone();
        }
        for (old, new) in pairs {
            if let Some(i) = folder::position(&self.listed, old) {
                self.listed[i] = new.clone();
            }
        }
        for path in [self.current.as_mut(), self.shown.as_mut().map(|(p, _)| p), self.reloading.as_mut()].into_iter().flatten() {
            if let Some((_, new)) = pairs.iter().find(|(old, _)| old == path) {
                *path = new.clone();
            }
        }
        fn rekey<T>(map: &mut HashMap<PathBuf, T>, pairs: &[(PathBuf, PathBuf)]) {
            let taken: Vec<(PathBuf, T)> =
                pairs.iter().filter_map(|(old, new)| map.remove(old).map(|v| (new.clone(), v))).collect();
            map.extend(taken);
        }
        rekey(&mut self.cache, pairs);
        rekey(&mut self.partial, pairs);
        if let Some(gallery) = &mut self.gallery {
            rekey(&mut gallery.cache, pairs);
        }
        if let Err(e) = self.favorites.renamed(pairs) {
            self.notice(e);
        }
        if !folders.is_empty() {
            self.folders_moved(&folders);
        }
    }

    /// `pairs` with each path known (listed, decoded, a favourite) under
    /// one of the folders `folders` (old, new) paired with its new path.
    fn within_folders(&self, pairs: &[(PathBuf, PathBuf)], folders: &[(PathBuf, PathBuf)]) -> Vec<(PathBuf, PathBuf)> {
        let to = |p: &Path| folders.iter().find_map(|(old, new)| folder::rebase(p, old, new));
        let favorites = self.favorites.paths();
        let mut known: Vec<&PathBuf> = self.files.iter().chain(&self.listed).chain(&favorites).collect();
        known.extend([self.current.as_ref(), self.shown.as_ref().map(|(p, _)| p), self.reloading.as_ref()].into_iter().flatten());
        known.extend(self.cache.keys().chain(self.partial.keys()));
        if let Some(gallery) = &self.gallery {
            known.extend(gallery.cache.keys());
        }
        let mut out = pairs.to_vec();
        let mut seen: std::collections::HashSet<PathBuf> = pairs.iter().map(|(old, _)| old.clone()).collect();
        for path in known {
            if seen.insert(path.clone())
                && let Some(new) = to(path)
            {
                out.push((path.clone(), new));
            }
        }
        out
    }

    /// The folders `folders` (old, new) are renamed: the folder shown, its
    /// sub-folders, the history and the pinned folders follow; the tree is
    /// read again.
    fn folders_moved(&mut self, folders: &[(PathBuf, PathBuf)]) {
        let to = |p: &Path| folders.iter().find_map(|(old, new)| folder::rebase(p, old, new));
        let dir = self.dir.as_deref().and_then(to);
        if let Some(dir) = &dir {
            self.dir = Some(dir.clone());
        }
        for list in [&mut self.listed_folders, &mut self.folders] {
            for path in list.iter_mut() {
                if let Some(new) = to(path) {
                    *path = new;
                }
            }
        }
        self.history.rebase(to);
        let pinned: Vec<(PathBuf, PathBuf)> = self.pinned.paths().into_iter().filter_map(|p| to(&p).map(|n| (p, n))).collect();
        if !pinned.is_empty() {
            if let Err(e) = self.pinned.renamed(&pinned) {
                self.notice(e);
            }
            self.pinned_changed();
        }
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.refresh();
            if let Some(dir) = &dir {
                gallery.tree.reveal(dir);
            }
        }
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

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(result) = self.scan.as_ref().and_then(Scan::poll) else { return };
        // Favourites whose folder no longer has them are no longer
        // favourites; those of a drive that is not there stay.
        let scan = self.scan.take();
        // The folder itself is gone: the nearest one above it instead.
        if result.is_err()
            && let Some(up) = scan.as_ref().and_then(Scan::take_up)
        {
            self.folder_gone(ctx, up);
            return;
        }
        let gone = scan.as_ref().map(Scan::take_gone).unwrap_or_default();
        self.listed_folders = scan.as_ref().map(Scan::take_folders).unwrap_or_default();
        // Quick Access: the pinned folders.
        if self.in_quick_access() {
            self.listed_folders = self.pinned.paths();
        }
        drop(scan);
        self.forget_gone_favorites(&gone);
        match result {
            Ok(files) => {
                self.listed = files;
                // The tree learns what the folder holds now: a sub-folder
                // deleted, renamed or made outside.
                if let (Some(dir), Some(gallery)) = (self.dir.as_deref(), self.gallery.as_mut())
                    && !self.deep
                    && !self.archive
                    && !favorites::is_virtual(dir)
                {
                    let names: Vec<String> = self.listed_folders.iter().map(|p| file_name(p)).collect();
                    gallery.tree.sync(dir, &names);
                }
            }
            Err(e) => {
                self.notice(tr!(format!("Cannot list the folder: {e}"), format!("Не удалось прочитать папку: {e}")));
                self.listed = self.current.iter().cloned().collect();
            }
        }
        self.files = self.filtered();
        self.folders = self.filtered_folders();
        self.folder_focus = self.folder_focus.filter(|&k| k < self.folders.len());
        // Back in a folder from one of its sub-folders: that one's cell.
        if let Some(left) = self.refocus.take().filter(|_| !self.folders_folded) {
            self.folder_focus = self.folders.iter().position(|f| folder::same_path(f, &left)).or(self.folder_focus);
        }
        // Nothing listed (the last favourites cleared or gone): nothing is
        // current, and the status bar is empty.
        // Opened, an archive shows its first image: none to show.
        if self.archive && self.listed.is_empty() && !self.gallery_open {
            self.notice(tr!("No images in this archive".into(), "В этом архиве нет изображений".into()));
        }
        if self.listed.is_empty()
            && let Some(current) = self.current.take()
        {
            // A file opened that is not there, in a folder with no images.
            if !self.in_favorites() && !self.archive && !crate::archive::is_file(&current) {
                let name = file_name(&current);
                self.notice(tr!(format!("File not found: {name}"), format!("Файл не найден: {name}")));
            }
            self.set_current(None);
        }
        self.starts = folder::starts(&self.files);
        self.selection.retain_listed(&self.files, &self.folders);
        self.index = self.current.as_deref().and_then(|c| folder::position(&self.files, c));
        if std::mem::take(&mut self.centre_after_scan)
            && let Some(gallery) = &mut self.gallery
        {
            gallery.scroll = Some(Scroll::Centre);
        }
        // Only sub-folders: the cursor on the first, for Enter.
        if self.files.is_empty() && !self.folders.is_empty() && !self.folders_folded && self.folder_focus.is_none() {
            self.folder_focus = Some(0);
        }
        if self.index.is_some() || self.files.is_empty() {
            return;
        }
        // Listed, but not passing the filter: the first that does.
        if self.current.as_deref().is_some_and(|c| folder::position(&self.listed, c).is_some()) {
            self.go(0);
            return;
        }
        match self.current.clone() {
            None => {
                self.go(0);
                // A folder opened: from its top, its sub-folders first,
                // not scrolled down to its first image; returned to, the
                // sub-folder left keeps the scroll asked for.
                if !self.folders.is_empty()
                    && self.folder_focus.is_none()
                    && let Some(gallery) = &mut self.gallery
                {
                    gallery.scroll = None;
                    gallery.scrolled_to = self.current.clone();
                }
            }
            // The file is gone (deleted or renamed, or it never existed):
            // without a place in the folder nothing could be browsed. The
            // next image takes its place, as after a deletion.
            Some(missing) => {
                // Not known among the sub-folders: the names are in
                // order only within a folder.
                let at = if self.mixed() { None } else { folder::insertion_point(&self.files, &missing, self.sort) };
                let at = at.or(self.place);
                let i = at.unwrap_or(0).min(self.files.len() - 1);
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

    /// Play the image on screen if it is animated: its frames replace the
    /// contents of its texture when their time comes (see `anim`). Only
    /// the current image in the viewer plays; it starts again from the
    /// first frame when it comes back.
    /// Start a slideshow in full screen, or stop the one running (Shift+F).
    fn toggle_slideshow(&mut self, ctx: &egui::Context) {
        if self.slideshow.is_some() {
            self.stop_slideshow();
            self.notice(tr!("Slideshow stopped".into(), "Слайд-шоу остановлено".into()));
            return;
        }
        if self.current.is_none() {
            return;
        }
        if self.gallery_open {
            self.leave_gallery();
        }
        if !Self::is_fullscreen(ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        self.slideshow = Some(Slideshow { next_at: None, armed_for: None, paused: false });
        let s = self.slideshow_seconds;
        self.notice(tr!(
            format!("Slideshow: {s} s per image; Space pauses, Esc stops"),
            format!("Слайд-шоу: {s} с на изображение; Space — пауза, Esc — стоп")
        ));
    }

    fn stop_slideshow(&mut self) {
        self.slideshow = None;
    }

    /// Show the next image of a slideshow when the time is up, counted
    /// from when the current one came on screen (a slow file is not
    /// skipped); after the last one, start over or stop.
    fn slideshow_tick(&mut self, ctx: &egui::Context) {
        let (on_screen, gallery_open, current) = (self.current_on_screen(), self.gallery_open, self.current.clone());
        let Some(show) = &mut self.slideshow else { return };
        if show.paused || gallery_open || !on_screen {
            return;
        }
        let interval = Duration::from_secs(self.slideshow_seconds as u64);
        if show.armed_for != current {
            show.armed_for = current;
            show.next_at = Some(Instant::now() + interval);
        }
        let due = show.next_at.unwrap_or_else(Instant::now);
        let now = Instant::now();
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        let last = self.index.is_some_and(|i| i + 1 >= self.files.len());
        if !last {
            self.step(1);
        } else if self.slideshow_loop && self.files.len() > 1 {
            self.go(0);
        } else if self.slideshow_loop {
            // One image: shown on.
            if let Some(show) = &mut self.slideshow {
                show.next_at = Some(now + interval);
            }
        } else {
            self.stop_slideshow();
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.notice(tr!("Slideshow finished".into(), "Слайд-шоу закончено".into()));
        }
    }

    fn animate(&mut self, ctx: &egui::Context) {
        let playing = self
            .shown
            .as_ref()
            .filter(|(p, pic)| pic.meta.animated && !self.gallery_open && self.current.as_ref() == Some(p))
            .map(|(p, _)| p.clone());
        let Some(path) = playing else {
            self.player = None;
            return;
        };
        if self.player.as_ref().is_none_or(|p| p.path != path) {
            // Every frame fills every level: nothing left to complete.
            self.partial.remove(&path);
            let max_side = ctx.input(|i| i.max_texture_side);
            self.player = Some(crate::anim::Player::start(path.clone(), max_side, ctx.clone()));
        }
        let Some(player) = &mut self.player else { return };
        let (frame, wait) = player.poll(Instant::now());
        if let Some(pixels) = frame
            && let Some(Slot::Ready(picture)) = self.cache.get(&path)
        {
            picture.texture.replace(&pixels);
        }
        if let Some(wait) = wait {
            ctx.request_repaint_after(wait);
        }
    }

    /// Put the current image on screen once it is decoded.
    fn sync_shown(&mut self) {
        // Waited for only while it is current (it blocks saving).
        if self.reloading.is_some() && self.reloading != self.current {
            self.reloading = None;
        }
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
                    } else if self.reloading.as_ref() == Some(current) {
                        self.view.turns = 0;
                        self.view.flip = false;
                        self.view.offset = Vec2::ZERO;
                    }
                    if self.reloading.as_ref() == Some(current) {
                        self.reloading = None;
                    }
                    self.shown = Some((current.clone(), picture.clone()));
                    if !self.first_image_logged {
                        self.first_image_logged = true;
                        log::info!("first image on screen at {:.0} ms", crate::since_start_ms());
                    }
                }
            }
            Some(Slot::Failed(_)) => {
                self.shown = None;
                self.reloading = None;
            }
            None => {}
        }
    }

    /// Unmark `gone`, favourites no longer in their folders, with a notice.
    fn forget_gone_favorites(&mut self, gone: &[PathBuf]) {
        if gone.is_empty() {
            return;
        }
        match self.favorites.remove_if(|p| gone.iter().any(|g| folder::same_path(g, p))) {
            Ok(0) => {}
            Ok(n) => self.notice(tr!(
                format!("Not found, removed from the favorites: {n}"),
                format!("Не найдено и убрано из избранного: {n}")
            )),
            Err(e) => self.notice(e),
        }
    }

    /// The folder listed (`dir`) is gone, deleted or renamed outside:
    /// show `up`, the nearest folder above it that is there, in the
    /// gallery, the tree read again; a pinned folder is unpinned.
    fn folder_gone(&mut self, ctx: &egui::Context, up: PathBuf) {
        let name = self.dir.as_deref().map(folder_label).unwrap_or_default();
        let pinned = self.dir.as_deref().is_some_and(|d| self.pinned.contains(d));
        if let Some(gone) = self.dir.clone() {
            self.unpin_gone(&[gone]);
        }
        let shown = up.display();
        self.notice(if pinned {
            tr!(
                format!("Folder not found, unpinned: {name}; showing {shown}"),
                format!("Папка не найдена и откреплена: {name}; показана {shown}")
            )
        } else {
            tr!(format!("Folder not found: {name}; showing {shown}"), format!("Папка не найдена: {name}; показана {shown}"))
        });
        self.set_current(None);
        self.start_scan(ctx, up.clone(), self.deep, None);
        // Nothing is left to view: the folder above, in the gallery.
        if !self.gallery_open {
            self.enter_gallery(ctx);
        }
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.refresh();
            gallery.tree.reveal(&up);
            gallery.scroll = Some(Scroll::Centre);
        }
    }

    /// Look for the pinned folders gone, on a thread (see `poll_pinned_gone`).
    fn check_pinned(&mut self, ctx: &egui::Context) {
        if self.pinned.len() > 0 && self.pinned_gone.is_none() {
            self.pinned_gone = Some(folder::find_gone(self.pinned.paths(), ctx.clone()));
        }
    }

    fn poll_pinned_gone(&mut self) {
        let Some(Ok(gone)) = self.pinned_gone.as_ref().map(mpsc::Receiver::try_recv) else { return };
        self.pinned_gone = None;
        // One may have come back meanwhile.
        let gone: Vec<PathBuf> = gone.into_iter().filter(|p| !p.exists()).collect();
        self.unpin_gone(&gone);
    }

    /// Unpin those of `gone` that are pinned, with a notice; a folder on a
    /// drive that is not there is never passed here (see `folder::is_gone`).
    fn unpin_gone(&mut self, gone: &[PathBuf]) {
        if gone.is_empty() || !gone.iter().any(|g| self.pinned.contains(g)) {
            return;
        }
        match self.pinned.remove_if(|p| gone.iter().any(|g| folder::same_path(g, p))) {
            Ok(0) => {}
            Ok(n) => self.notice(tr!(
                format!("Not found, unpinned from Quick Access: {n}"),
                format!("Не найдено и откреплено от панели быстрого доступа: {n}")
            )),
            Err(e) => self.notice(e),
        }
        self.pinned_changed();
    }

    /// The favourites found gone at start-up.
    fn poll_favorites_gone(&mut self) {
        let Some(Ok(gone)) = self.favorites_gone.as_ref().map(mpsc::Receiver::try_recv) else { return };
        self.favorites_gone = None;
        // One may have come back (or been marked again) meanwhile.
        let gone: Vec<PathBuf> = gone.into_iter().filter(|p| !p.exists()).collect();
        self.forget_gone_favorites(&gone);
    }

    fn poll_delete(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.deleting else { return };
        let Ok((paths, result)) = rx.try_recv() else { return };
        self.deleting = None;
        // The shell may have deleted some and not others.
        let gone: Vec<PathBuf> = paths.into_iter().filter(|p| !p.exists()).collect();
        if let Err(e) = result
            && e != "cancelled"
        {
            self.notice(tr!(format!("Cannot delete: {e}"), format!("Не удалось удалить: {e}")));
        }
        let folders: Vec<PathBuf> =
            std::mem::take(&mut self.deleting_folders).into_iter().filter(|f| gone.iter().any(|g| folder::same_path(g, f))).collect();
        // The files deleted with the folders (chosen together) first.
        let gone: Vec<PathBuf> = gone.into_iter().filter(|g| !folders.iter().any(|f| folder::same_path(g, f))).collect();
        for path in &gone {
            self.cache.remove(path);
        }
        if !gone.is_empty()
            && let Err(e) = self.favorites.remove_if(|p| gone.iter().any(|g| folder::same_path(p, g)))
        {
            self.notice(e);
        }
        for path in &gone {
            self.unlist(path);
        }
        // The folder is listed again, or another shown.
        if !folders.is_empty() {
            self.folders_deleted(ctx, &folders);
            return;
        }
        if gone.is_empty() {
            return;
        }
        // A listing begun before would bring them back.
        if self.scan.is_some() {
            self.list_again(ctx);
        }
    }

    /// `folders` have gone to the Recycle Bin: the favourites and pinned
    /// folders in them are forgotten, the folder shown, if it was one of
    /// them or in one, gives way to the nearest folder above, otherwise it
    /// is listed again (its folder cells); the tree is read again.
    fn folders_deleted(&mut self, ctx: &egui::Context, folders: &[PathBuf]) {
        let within = |p: &Path| folders.iter().any(|f| folder::is_within(p, f));
        if let Err(e) = self.favorites.remove_if(within) {
            self.notice(e);
        }
        match self.pinned.remove_if(within) {
            Ok(0) => {}
            Ok(_) => self.pinned_changed(),
            Err(e) => self.notice(e),
        }
        let name = folder_label(&folders[0]);
        self.notice(match folders.len() {
            1 => tr!(format!("Folder moved to the Recycle Bin: {name}"), format!("Папка перемещена в корзину: {name}")),
            n => tr!(format!("Folders moved to the Recycle Bin: {n}"), format!("Папок перемещено в корзину: {n}")),
        });
        let up = self.dir.as_deref().filter(|d| within(d)).and_then(folder::nearest_folder);
        if let Some(up) = up {
            self.set_current(None);
            self.start_scan(ctx, up.clone(), self.deep, None);
            if let Some(gallery) = &mut self.gallery {
                gallery.tree.reveal(&up);
                gallery.scroll = Some(Scroll::Centre);
            }
        } else {
            self.list_again(ctx);
        }
        if let Some(gallery) = &mut self.gallery {
            gallery.tree.refresh();
        }
    }

    /// Ask to move the folder `dir` to the Recycle Bin (its context menu,
    /// Delete on its cell).
    pub fn ask_delete_folder(&mut self, dir: PathBuf) {
        if self.deleting.is_none() && dir.is_dir() {
            self.confirm_delete = Some(vec![dir]);
            self.delete_folders = 1;
        }
    }

    /// Ask to move the folders chosen in the gallery, and the images chosen
    /// with them, to the Recycle Bin (Delete).
    fn ask_delete_chosen(&mut self) {
        if self.deleting.is_some() {
            return;
        }
        let items: Vec<PathBuf> = self.chosen_items().into_iter().filter(|p| p.exists()).collect();
        let folders = items.iter().take_while(|p| p.is_dir()).count();
        if !items.is_empty() {
            self.confirm_delete = Some(items);
            self.delete_folders = folders;
        }
    }

    /// Ask for the new names of the folders chosen in the gallery and the
    /// images chosen with them (F2): one folder alone as with its menu,
    /// several as several images are (`BatchRename`).
    fn ask_rename_chosen(&mut self) {
        let items: Vec<PathBuf> = self.chosen_items().into_iter().filter(|p| p.exists()).collect();
        let dirs = items.iter().take_while(|p| p.is_dir()).count();
        match &items[..] {
            [] => {}
            [one] if dirs == 1 => self.ask_rename_folder(one.clone()),
            [one] => {
                let name = file_name(one);
                self.rename = Some(Rename { path: one.clone(), name, error: None, focus: true, select: true });
            }
            _ => {
                // The name of the folder shown, as a start.
                let base = items[0].parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.batch_rename = Some(BatchRename { paths: items, dirs, base, start: 1, error: None, focus: true });
            }
        }
    }

    /// Ask for a new name of the folder `dir` (its context menu, F2 on its
    /// cell).
    pub fn ask_rename_folder(&mut self, dir: PathBuf) {
        if dir.is_dir() && dir.parent().is_some() {
            let name = file_name(&dir);
            self.rename = Some(Rename { path: dir, name, error: None, focus: true, select: true });
        }
    }

    /// Take `path` out of `files` (deleted, or no longer a favourite among
    /// the favourites); if it was current, the next image takes its place,
    /// after the last one the one before.
    fn unlist(&mut self, path: &Path) {
        self.selection.remove(path);
        if let Some(pos) = folder::position(&self.listed, path) {
            self.listed.remove(pos);
        }
        let was_current = self.current.as_deref().is_some_and(|c| folder::same_path(c, path));
        match folder::position(&self.files, path) {
            Some(pos) => {
                self.files.remove(pos);
                self.starts = folder::starts(&self.files);
                if was_current {
                    self.index = None;
                    if self.files.is_empty() {
                        self.set_current(None);
                    } else {
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

    /// Move `paths` to the Recycle Bin on a thread; the shell may ask
    /// questions of its own (a file that cannot be recycled).
    pub fn delete(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let owner = self.hwnd;
        self.deleting_folders = paths.iter().filter(|p| p.is_dir()).cloned().collect();
        std::thread::spawn(move || {
            let result = win::recycle(&paths, owner);
            let _ = tx.send((paths, result));
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
        if self.crop.is_some() && self.run_in_crop(ctx, frame, cmd) {
            return;
        }
        if self.gallery_open && self.run_in_gallery(ctx, cmd) {
            return;
        }
        if self.archive && changes_files(cmd) {
            self.notice(tr!("Images in an archive cannot be changed; Save As makes a copy".into(), "Изображения в архиве не изменяются; копию можно сделать через «Сохранить как»".into()));
            return;
        }
        let ppp = ctx.pixels_per_point();
        let viewport = self.viewport;
        let size = self.shown.as_ref().map(|(_, p)| p.size());
        match cmd {
            Cmd::Next | Cmd::PageDown => self.step(1),
            Cmd::Prev | Cmd::PageUp => self.step(-1),
            Cmd::Gallery => self.enter_gallery(ctx),
            // Only with a folder cell under the cursor (`run_in_gallery`).
            Cmd::OpenFolder => {}
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
            Cmd::CopyImage => self.copy_image(ctx),
            Cmd::Wallpaper => self.set_wallpaper(ctx),
            Cmd::Print => self.print(ctx),
            Cmd::Find => {
                if !self.gallery_open {
                    self.enter_gallery(ctx);
                }
                ctx.memory_mut(|m| m.request_focus(egui::Id::new(crate::ui::gallery::FILTER_ID)));
                self.filter_open = true;
                // The `/` that asked for it would be typed into the field,
                // which has the focus in this very frame.
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t == "/")));
            }
            Cmd::Paste => self.paste(ctx),
            Cmd::FlipHorizontal => self.view.mirror(true),
            Cmd::FlipVertical => self.view.mirror(false),
            // A slideshow's pause first; an animation's otherwise.
            Cmd::Pause if self.slideshow.is_some() => {
                let paused = self.slideshow.as_mut().map(|show| {
                    show.paused = !show.paused;
                    // Resumed: the current image gets its full time again.
                    show.armed_for = None;
                    show.paused
                });
                self.notice(if paused == Some(true) {
                    tr!("Slideshow paused".into(), "Слайд-шоу приостановлено".into())
                } else {
                    tr!("Slideshow resumed".into(), "Слайд-шоу продолжается".into())
                });
            }
            Cmd::Pause => {
                if let Some(player) = &mut self.player {
                    player.toggle_pause(Instant::now());
                }
            }
            Cmd::Slideshow => self.toggle_slideshow(ctx),
            Cmd::SlideshowInterval(seconds) => {
                self.slideshow_seconds = seconds.clamp(1, 3600);
                if let Some(show) = &mut self.slideshow {
                    show.armed_for = None;
                }
            }
            Cmd::SlideshowLoop => self.slideshow_loop = !self.slideshow_loop,
            Cmd::NextFrame | Cmd::PrevFrame => {
                if let Some(player) = &mut self.player {
                    player.step(cmd == Cmd::NextFrame);
                }
            }
            Cmd::FullScreen | Cmd::WindowFullScreen => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!Self::is_fullscreen(ctx))),
            Cmd::Escape if self.slideshow.is_some() => {
                self.stop_slideshow();
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                self.notice(tr!("Slideshow stopped".into(), "Слайд-шоу остановлено".into()));
            }
            Cmd::Escape if Self::is_fullscreen(ctx) => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false)),
            // Back to the gallery, as with G; Esc there closes.
            Cmd::Escape => self.enter_gallery(ctx),
            Cmd::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Delete => {
                let files: Vec<PathBuf> = self.targets().into_iter().filter(|p| p.is_file()).collect();
                if self.deleting.is_none() && !files.is_empty() {
                    self.confirm_delete = Some(files);
                    self.delete_folders = 0;
                }
            }
            Cmd::Undo => self.undo(ctx),
            Cmd::Crop => self.start_crop(),
            Cmd::Save => self.save(ctx, frame, false),
            Cmd::SaveAs => self.save(ctx, frame, true),
            Cmd::ConvertTo(format) => self.convert(ctx, format),
            Cmd::Edit => self.open_in_editor(ctx, None),
            Cmd::EditWith(k) => {
                if let Some(editor) = self.menu_editors.get(k).cloned() {
                    self.open_in_editor(ctx, Some(editor));
                }
            }
            Cmd::EditWithOther => {
                let picked = rfd::FileDialog::new()
                    .set_title(tr!("Choose an editor", "Выбор редактора"))
                    .add_filter(tr!("Programs", "Программы"), &["exe"])
                    .set_parent(frame)
                    .pick_file();
                if let Some(program) = picked {
                    self.open_in_editor(ctx, Some(Editor::program(&program)));
                }
            }
            Cmd::Rename => {
                let paths: Vec<PathBuf> = self.targets().into_iter().filter(|p| p.is_file()).collect();
                if paths.len() > 1 {
                    // The folder's name, as a start.
                    // (A drive's root has none.)
                    let base = paths[0].parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.batch_rename = Some(BatchRename { paths, dirs: 0, base, start: 1, error: None, focus: true });
                } else if let Some(path) = paths.into_iter().next() {
                    let name = file_name(&path);
                    self.rename = Some(Rename { path, name, error: None, focus: true, select: true });
                }
            }
            Cmd::Copy => {
                let files = self.targets();
                let n = files.len();
                let result = match &files[..] {
                    [] => return,
                    [one] => win::copy_file(one),
                    _ => win::copy_files(&files),
                };
                let text = match result {
                    Ok(()) if n == 1 => tr!("Copied to the clipboard".into(), "Скопировано в буфер обмена".into()),
                    Ok(()) => tr!(format!("Files copied to the clipboard: {n}"), format!("Скопировано в буфер обмена файлов: {n}")),
                    Err(e) => tr!(format!("Cannot copy: {e}"), format!("Не удалось скопировать: {e}")),
                };
                self.notice(text);
            }
            Cmd::Open => self.pick_file(ctx, frame),
            Cmd::ShowInExplorer => {
                if let Some(path) = &self.current {
                    // An image in an archive: the archive.
                    let path = crate::archive::split(path).map_or_else(|| path.clone(), |(archive, _)| archive);
                    win::show_in_explorer(&path);
                }
            }
            Cmd::Refresh => {
                if let Some(path) = &self.current {
                    // Decoded again; the old picture stays until then.
                    self.cache.remove(path);
                }
                let dir = match &self.current {
                    Some(path) if !self.mixed() => path.parent().map(Path::to_path_buf),
                    _ => self.dir.clone(),
                };
                if let Some(dir) = dir {
                    self.start_scan(ctx, dir, self.deep, self.current.clone());
                }
            }
            Cmd::ToggleToolbar => self.show_toolbar = !self.show_toolbar,
            Cmd::ToggleStatusBar => self.show_status_bar = !self.show_status_bar,
            Cmd::Info => self.show_info = !self.show_info,
            Cmd::SortBy(key) => self.sort_by(ctx, Order { key, ..self.order() }),
            Cmd::SortDescending => self.sort_by(ctx, Order { descending: !self.order().descending, ..self.order() }),
            Cmd::KeepZoom => {
                self.view.keep = !self.view.keep;
                self.notice(if self.view.keep {
                    tr!("The next images keep the zoom and position".into(), "Следующие изображения сохранят масштаб и положение".into())
                } else {
                    tr!("The next images open in the zoom mode".into(), "Следующие изображения откроются в режиме масштаба".into())
                });
            }
            // The gallery's folders and choice.
            Cmd::Back | Cmd::Forward | Cmd::Up | Cmd::SelectTo(_) | Cmd::SelectAll => {}
            Cmd::Favorite => self.toggle_favorite(ctx),
            Cmd::Favorites => {
                self.open_folder(ctx, PathBuf::from(favorites::DIR), self.deep);
                if !self.gallery_open {
                    self.enter_gallery(ctx);
                }
            }
            Cmd::QuickAccess => {
                self.open_folder(ctx, PathBuf::from(favorites::PINNED_DIR), self.deep);
                if !self.gallery_open {
                    self.enter_gallery(ctx);
                }
            }
            Cmd::MoveTo(n) => self.transfer(ctx, n, false),
            Cmd::CopyTo(n) => self.transfer(ctx, n, true),
            Cmd::MoveToListed(k) | Cmd::CopyToListed(k) => {
                if let Some(to) = self.menu_folders.get(k).cloned() {
                    self.transfer_to(ctx, to, cmd == Cmd::CopyToListed(k), None);
                }
            }
            Cmd::MoveToNew | Cmd::CopyToNew => self.ask_new_folder(Some(cmd == Cmd::CopyToNew)),
            Cmd::NewFolder => self.ask_new_folder(None),
            Cmd::MoveToOther | Cmd::CopyToOther => self.transfer_to_picked(ctx, frame, cmd == Cmd::CopyToOther),
            Cmd::CopyFavorites => {
                let files = self.favorite_files();
                let n = files.len();
                let text = match win::copy_files(&files) {
                    _ if n == 0 => tr!("No favorites".into(), "Избранное пусто".into()),
                    Ok(()) => tr!(format!("Files copied to the clipboard: {n}"), format!("Скопировано в буфер обмена файлов: {n}")),
                    Err(e) => tr!(format!("Cannot copy: {e}"), format!("Не удалось скопировать: {e}")),
                };
                self.notice(text);
            }
            Cmd::CopyFavoritesTo => self.copy_favorites_to(ctx, frame),
            Cmd::ClearFavorites => self.confirm_clear_favorites = self.favorites.len() > 0,
            Cmd::UnpinAll => self.confirm_unpin_all = self.pinned.len() > 0,
            Cmd::GoToFolder => self.go_to_folder(ctx),
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
        if self.gallery.is_none() {
            return false;
        }
        // Folders chosen: Delete and F2 act on them and the images chosen
        // with them, Esc chooses nothing; with no image chosen, the other
        // commands of the files have none.
        if self.selection.has_folders() {
            match cmd {
                Cmd::Delete => {
                    self.ask_delete_chosen();
                    return true;
                }
                Cmd::Rename => {
                    self.ask_rename_chosen();
                    return true;
                }
                Cmd::Escape => {
                    self.selection.clear();
                    return true;
                }
                _ if self.selection.is_empty() && acts_on_files(cmd) => return true,
                _ => {}
            }
        }
        // A sub-folder's cell has the cursor: Enter opens it, Esc returns
        // to the current image, and the commands of the files have no file.
        if let Some(dir) = self.focused_folder() {
            match cmd {
                Cmd::OpenFolder => {
                    self.open_subfolder(ctx, dir);
                    return true;
                }
                // Along the cells, as the arrows go.
                Cmd::Next | Cmd::Prev => {
                    if let Some(layout) = self.gallery.as_ref().map(|g| &g.layout)
                        && let Some(k) = self.folder_focus
                    {
                        let target = layout.step(Cell::Folder(k), cmd == Cmd::Next);
                        self.move_to(target);
                    }
                    return true;
                }
                Cmd::Escape => {
                    self.folder_focus = None;
                    if let Some(gallery) = &mut self.gallery {
                        gallery.scroll = Some(Scroll::Visible);
                    }
                    return true;
                }
                Cmd::ShowInExplorer => {
                    win::show_in_explorer(&dir);
                    return true;
                }
                Cmd::Delete => {
                    self.ask_delete_folder(dir);
                    return true;
                }
                Cmd::Rename => {
                    self.ask_rename_folder(dir);
                    return true;
                }
                Cmd::SelectTo(_) => return true,
                _ if acts_on_files(cmd) => return true,
                Cmd::FullScreen | Cmd::SelectAll => self.folder_focus = None,
                _ => {}
            }
        }
        // Moving alone chooses nothing; with Shift, the images on the way.
        let to = match cmd {
            Cmd::Arrow(arrow) => Some(Move::Arrow(arrow)),
            Cmd::PageUp => Some(Move::PageUp),
            Cmd::PageDown => Some(Move::PageDown),
            Cmd::First => Some(Move::First),
            Cmd::Last => Some(Move::Last),
            _ => None,
        };
        if let Some(to) = to {
            if let Some(target) = self.grid_target(to) {
                self.selection.clear();
                self.move_to(target);
            }
            return true;
        }
        match cmd {
            // Among the images only.
            Cmd::SelectTo(to) => {
                if let Some(Cell::Image(i)) = self.grid_target(to) {
                    self.select_to(i);
                }
            }
            Cmd::SelectAll => self.select_all(),
            // First what is chosen, then the program.
            Cmd::Escape if !self.selection.is_empty() => self.selection.clear(),
            Cmd::Back => {
                let here = self.here();
                if let Some(place) = self.history.back(here) {
                    self.return_to(ctx, place);
                }
            }
            Cmd::Forward => {
                let here = self.here();
                if let Some(place) = self.history.forward(here) {
                    self.return_to(ctx, place);
                }
            }
            Cmd::Up => self.go_up(ctx),
            Cmd::ZoomIn | Cmd::ZoomOut => {
                self.thumb_size = gallery::step_size(self.thumb_size, cmd == Cmd::ZoomIn);
                if let Some(gallery) = &mut self.gallery {
                    gallery.scroll = Some(Scroll::Visible);
                }
            }
            // Nothing to zoom, turn or save.
            Cmd::Actual
            | Cmd::Fit
            | Cmd::Fill
            | Cmd::Cover
            | Cmd::RotateLeft
            | Cmd::RotateRight
            | Cmd::FlipHorizontal
            | Cmd::FlipVertical
            | Cmd::Crop
            | Cmd::Save
            | Cmd::SaveAs => {}
            Cmd::Gallery => self.leave_gallery(),
            // F shows the image in full screen; Ctrl+Shift+F
            // (`WindowFullScreen`) turns the gallery's window, as in the viewer.
            Cmd::FullScreen => {
                if self.current.is_some() {
                    self.leave_gallery();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                }
            }
            Cmd::Escape if !Self::is_fullscreen(ctx) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Refresh => {
                // The thumbnails and the tree too; the folder is read again
                // as in the viewer.
                if let Some(gallery) = &mut self.gallery {
                    gallery.forget(&self.files);
                    gallery.tree.refresh();
                }
                self.check_pinned(ctx);
                return false;
            }
            _ => return false,
        }
        true
    }

    /// The cell `to` leads to from the one with the cursor (a sub-folder's
    /// or the current image's) in the gallery's grid, as it was drawn.
    fn grid_target(&self, to: Move) -> Option<Cell> {
        let layout = &self.gallery.as_ref()?.layout;
        let page = self.gallery.as_ref()?.page_rows as isize;
        let here = self.folder_focus.map(Cell::Folder).or(self.index.map(Cell::Image));
        match (to, here) {
            (Move::First, _) => layout.first(),
            (Move::Last, _) => layout.last(),
            (_, None) => layout.first(),
            (Move::Arrow(Arrow::Left), Some(c)) => Some(layout.step(c, false)),
            (Move::Arrow(Arrow::Right), Some(c)) => Some(layout.step(c, true)),
            (Move::Arrow(Arrow::Up), Some(c)) => Some(layout.move_rows(c, -1)),
            (Move::Arrow(Arrow::Down), Some(c)) => Some(layout.move_rows(c, 1)),
            (Move::PageUp, Some(c)) => Some(layout.move_rows(c, -page)),
            (Move::PageDown, Some(c)) => Some(layout.move_rows(c, page)),
        }
    }

    /// Put the gallery's cursor on `c`: a sub-folder's cell, or an image,
    /// which becomes current; the grid follows it.
    fn move_to(&mut self, c: Cell) {
        match c {
            // Not while they are folded: the layout of the last frame may
            // still have them.
            Cell::Folder(k) if k < self.folders.len() && !self.folders_folded => self.folder_focus = Some(k),
            Cell::Folder(_) => return,
            Cell::Image(i) => {
                self.folder_focus = None;
                self.go(i);
            }
        }
        if let Some(gallery) = &mut self.gallery {
            gallery.scroll = Some(Scroll::Visible);
        }
    }

    /// Mark the current image as a favourite, or unmark it; among the
    /// favourites it then leaves the list, the next one taking its place.
    /// Several chosen in the gallery are all marked, or all unmarked when
    /// they all are.
    fn toggle_favorite(&mut self, ctx: &egui::Context) {
        let targets = self.targets();
        if targets.len() > 1 {
            self.toggle_favorites(ctx, &targets);
            return;
        }
        let Some(path) = targets.into_iter().next() else { return };
        let text = match self.favorites.toggle(&path) {
            Ok(true) => tr!("Added to the favorites".into(), "Добавлено в избранное".into()),
            Ok(false) => {
                if self.in_favorites() {
                    if self.scan.is_some() {
                        // The listing under way has it still.
                        self.relist(ctx);
                    }
                    self.unlist(&path);
                }
                tr!("Removed from the favorites".into(), "Убрано из избранного".into())
            }
            Err(e) => e,
        };
        self.notice(text);
    }

    fn toggle_favorites(&mut self, ctx: &egui::Context, paths: &[PathBuf]) {
        let n = paths.len();
        let text = if paths.iter().all(|p| self.favorites.contains(p)) {
            match self.favorites.remove_if(|p| paths.iter().any(|t| folder::same_path(p, t))) {
                Ok(_) => {
                    if self.in_favorites() {
                        if self.scan.is_some() {
                            self.relist(ctx);
                        }
                        for path in paths {
                            self.unlist(path);
                        }
                    }
                    tr!(format!("Removed from the favorites: {n}"), format!("Убрано из избранного: {n}"))
                }
                Err(e) => e,
            }
        } else {
            match self.favorites.add_all(paths) {
                Ok(added) => tr!(format!("Added to the favorites: {added}"), format!("Добавлено в избранное: {added}")),
                Err(e) => e,
            }
        };
        self.notice(text);
    }

    /// The favourites that are there: as listed while they are, otherwise
    /// in the order they were marked.
    fn favorite_files(&self) -> Vec<PathBuf> {
        if self.in_favorites() && self.scan.is_none() {
            return self.files.clone();
        }
        self.favorites.paths().into_iter().filter(|p| p.is_file()).collect()
    }

    /// Copy the favourites to a folder the user picks; the shell shows the
    /// progress and asks about files of the same name.
    fn copy_favorites_to(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.copying.is_some() {
            return;
        }
        let files = self.favorite_files();
        if files.is_empty() {
            self.notice(tr!("No favorites".into(), "Избранное пусто".into()));
            return;
        }
        let picked = rfd::FileDialog::new()
            .set_title(tr!("Copy the favorites to", "Копировать избранное в папку"))
            .set_parent(frame)
            .pick_folder();
        let Some(to) = picked else { return };
        self.copy_files_to(ctx, files, to, None, None);
    }

    /// Copy `files` into `to` by the shell on a thread; `fresh`: the
    /// copies that would be new there, for Ctrl+Z (None for the
    /// favourites). `poll_copy` takes the outcome.
    fn copy_files_to(&mut self, ctx: &egui::Context, files: Vec<PathBuf>, to: PathBuf, fresh: Option<Vec<PathBuf>>, created: Option<PathBuf>) {
        let (tx, done) = mpsc::channel();
        let (ctx, owner, dest, count) = (ctx.clone(), self.hwnd, to.clone(), files.len());
        std::thread::spawn(move || {
            let _ = tx.send(win::copy_to(&files, &dest, owner));
            ctx.request_repaint();
        });
        self.copying = Some(Copying { done, to, count, fresh, created });
    }

    /// Move the current image, or those chosen, into the pinned folder with
    /// the key `n` (Alt+n), or with `copy` copy them there (Shift+Alt+n).
    fn transfer(&mut self, ctx: &egui::Context, n: u8, copy: bool) {
        let Some(to) = self.pinned.with_key(n) else {
            self.notice(tr!(
                format!("No folder has Alt+{n}: give a pinned folder the key in its context menu"),
                format!("Клавиша Alt+{n} не назначена: назначьте её закреплённой папке в её контекстном меню")
            ));
            return;
        };
        self.transfer_to(ctx, to, copy, None);
    }

    /// Move the current image, or those chosen, into the folder `to`, or
    /// with `copy` copy them there; `created`: the folder was made for
    /// them, and Ctrl+Z removes it again if it is empty then. The next
    /// image takes the place of one moved. False when nothing is done.
    fn transfer_to(&mut self, ctx: &egui::Context, to: PathBuf, copy: bool, created: Option<PathBuf>) -> bool {
        let name = folder_label(&to);
        if !to.is_dir() {
            self.notice(tr!(format!("Folder not found: {name}"), format!("Папка не найдена: {name}")));
            return false;
        }
        if self.moving.is_some() || self.copying.is_some() {
            self.notice(tr!("Still moving or copying…".into(), "Перемещение или копирование ещё идёт…".into()));
            return false;
        }
        if self.still_saving() {
            return false;
        }
        let files: Vec<PathBuf> = self.targets().into_iter().filter(|p| p.is_file()).collect();
        if files.is_empty() {
            return false;
        }
        // Already there: nothing to do.
        if files.iter().all(|f| f.parent().is_some_and(|p| folder::same_path(p, &to))) {
            self.notice(tr!(format!("Already in {name}"), format!("Уже в папке {name}")));
            return false;
        }
        let files: Vec<PathBuf> = files.into_iter().filter(|f| f.parent().is_none_or(|p| !folder::same_path(p, &to))).collect();
        let pairs = crate::rename::into_folder(&files, &to);
        if copy {
            // The shell asks about the names taken; the copies that are
            // new can be undone.
            let fresh: Vec<PathBuf> = pairs.iter().filter(|(_, new)| !new.exists()).map(|(_, new)| new.clone()).collect();
            self.copy_files_to(ctx, files, to, Some(fresh), created);
        } else {
            if let Err(e) = crate::rename::check(&pairs) {
                self.notice(tr!(format!("Cannot move: {e}"), format!("Не удалось переместить: {e}")));
                return false;
            }
            self.move_pairs(ctx, pairs, true, created);
        }
        true
    }

    /// The folder of the current image, or of the first chosen (none in
    /// an archive): where a new folder is made, what the Move to Folder
    /// menu lists around.
    fn transfer_base(&self) -> Option<PathBuf> {
        if self.in_archive() {
            return None;
        }
        self.targets().first().and_then(|f| f.parent()).map(Path::to_path_buf)
    }

    /// The folders the Move to Folder menu offers besides the pinned ones:
    /// the parent of the image's folder, if any, and its sub-folders (those
    /// listed, or read now and kept until the next listing).
    pub fn transfer_folders(&mut self) -> (Option<PathBuf>, Vec<PathBuf>) {
        let Some(base) = self.transfer_base() else { return (None, Vec::new()) };
        let parent = base.parent().map(Path::to_path_buf);
        let listed = !self.deep && !self.archive && self.dir.as_deref().is_some_and(|d| folder::same_path(d, &base));
        if listed {
            return (parent, self.listed_folders.clone());
        }
        if !self.sibling_folders.as_ref().is_some_and(|(dir, _)| folder::same_path(dir, &base)) {
            let subfolders = folder::subfolders(&base);
            self.sibling_folders = Some((base, subfolders));
        }
        (parent, self.sibling_folders.as_ref().map(|(_, f)| f.clone()).unwrap_or_default())
    }

    /// Ask for the name of a folder to make: in the folder shown (`transfer`
    /// None; not the favourites, Quick Access or an archive), or beside the
    /// current image, or the chosen ones, to move them into (Alt+N,
    /// `Some(false)`) or copy them into (Shift+Alt+N, `Some(true)`): "New
    /// folder", numbered past one there.
    pub fn ask_new_folder(&mut self, transfer: Option<bool>) {
        let parent = match transfer {
            Some(_) => self.transfer_base(),
            None => self.dir.clone().filter(|d| !favorites::is_virtual(d) && !self.archive),
        };
        let Some(parent) = parent else { return };
        if transfer.is_some() && (self.moving.is_some() || self.copying.is_some()) {
            self.notice(tr!("Still moving or copying…".into(), "Перемещение или копирование ещё идёт…".into()));
            return;
        }
        let base = tr!("New folder", "Новая папка");
        let mut name = base.to_string();
        let mut n = 2;
        while parent.join(&name).exists() {
            name = format!("{base} ({n})");
            n += 1;
        }
        self.new_folder = Some(NewFolder { parent, name, transfer, error: None, focus: true, select: true });
    }

    /// Make the folder `name` in `parent`, and with `transfer` move the
    /// current image, or those chosen, into it (`Some(false)`) or copy them
    /// (`Some(true)`); made alone in the folder shown, its cell gets the
    /// cursor. Why not, if it cannot be made.
    pub fn make_folder(&mut self, ctx: &egui::Context, parent: &Path, name: &str, transfer: Option<bool>) -> Result<(), String> {
        folder::check_name(name).map_err(str::to_string)?;
        let dir = parent.join(name);
        if dir.exists() {
            return Err(tr!("A file or folder with this name already exists", "Файл или папка с таким именем уже существует").into());
        }
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        self.sibling_folders = None;
        match transfer {
            Some(copy) => {
                if !self.transfer_to(ctx, dir.clone(), copy, Some(dir.clone())) {
                    // Refused (a name taken among the files): no folder for nothing.
                    let _ = std::fs::remove_dir(&dir);
                }
            }
            None => {
                self.notice(tr!(format!("Folder made: {name}"), format!("Папка создана: {name}")));
                if self.lists_folder(parent) {
                    self.list_again(ctx);
                    // Its cell has the cursor once listed.
                    if folder::same_path(parent, self.dir.as_deref().unwrap_or(parent)) {
                        self.refocus = Some(dir);
                    }
                }
            }
        }
        Ok(())
    }

    /// Move the current image, or those chosen, into a folder picked in a
    /// dialog, or with `copy` copy them there.
    fn transfer_to_picked(&mut self, ctx: &egui::Context, frame: &eframe::Frame, copy: bool) {
        let Some(base) = self.transfer_base() else { return };
        let title = if copy { tr!("Copy to folder", "Копировать в папку") } else { tr!("Move to folder", "Переместить в папку") };
        let picked = rfd::FileDialog::new().set_title(title).set_directory(&base).set_parent(frame).pick_folder();
        if let Some(to) = picked {
            self.transfer_to(ctx, to, copy, None);
        }
    }

    /// Remove the folder `dir` if it holds nothing (made for a move or a
    /// copy that is undone), and list the folder again, since its cell is
    /// gone.
    fn remove_if_empty(&mut self, ctx: &egui::Context, dir: &Path) {
        let empty = std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none());
        if empty && std::fs::remove_dir(dir).is_ok() {
            self.sibling_folders = None;
            if let Some(parent) = dir.parent()
                && self.lists_folder(parent)
            {
                self.list_again(ctx);
            }
        }
    }

    /// Move each file of `pairs` to its new path: at once by renaming, so
    /// that the next image takes its place now, or by the shell on a
    /// thread when the folder is on another drive (`poll_move` then);
    /// `undoable`: remembered for Ctrl+Z, with `created`, the folder made
    /// for them.
    fn move_pairs(&mut self, ctx: &egui::Context, pairs: Vec<(PathBuf, PathBuf)>, undoable: bool, created: Option<PathBuf>) {
        match crate::rename::move_all(&pairs) {
            Ok(()) => self.files_moved(ctx, pairs, undoable, created),
            Err(crate::rename::MoveError::OtherDrive) => {
                let files: Vec<PathBuf> = pairs.iter().map(|(old, _)| old.clone()).collect();
                let Some(to) = pairs.first().and_then(|(_, new)| new.parent()).map(Path::to_path_buf) else { return };
                let (tx, done) = mpsc::channel();
                let (ctx, owner) = (ctx.clone(), self.hwnd);
                std::thread::spawn(move || {
                    let _ = tx.send(win::move_to(&files, &to, owner));
                    ctx.request_repaint();
                });
                self.moving = Some(Moving { done, pairs, undoable, created, remove_empty: None });
            }
            Err(crate::rename::MoveError::Other(e)) => self.notice(e),
        }
    }

    /// The files of `pairs` are now at their new paths: what the listing
    /// still holds stays, with its texture and thumbnail, the rest leave
    /// it (the next image taking the current one's place).
    fn files_moved(&mut self, ctx: &egui::Context, pairs: Vec<(PathBuf, PathBuf)>, undoable: bool, created: Option<PathBuf>) {
        self.moved(&pairs);
        let mut stays = false;
        for (_, new) in &pairs {
            if self.lists(new) {
                stays = true;
            } else {
                self.unlist(new);
            }
        }
        // The cells of the folders they left and entered show other
        // images now; a folder made for them gets a cell.
        self.sibling_folders = None;
        if let Some(gallery) = &mut self.gallery {
            let dirs: Vec<PathBuf> =
                pairs.iter().flat_map(|(old, new)| [old.parent(), new.parent()]).flatten().map(Path::to_path_buf).collect();
            gallery.forget_previews_of(&dirs);
        }
        let made = created.as_deref().and_then(Path::parent).is_some_and(|p| self.lists_folder(p));
        // Still listed: their place in the order may have changed. A
        // listing begun before the move would bring the others back.
        if stays || made || self.scan.is_some() {
            self.list_again(ctx);
        }
        let n = pairs.len();
        let text = match pairs.first().and_then(|(_, new)| new.parent()).map(folder_label) {
            Some(to) if undoable && n == 1 => tr!(format!("Moved to {to}: {}", file_name(&pairs[0].1)), format!("Перемещено в {to}: {}", file_name(&pairs[0].1))),
            Some(to) if undoable => tr!(format!("Moved to {to}: {n}"), format!("Перемещено в {to}: {n}")),
            _ => tr!("Move undone".into(), "Перемещение отменено".into()),
        };
        self.notice(text);
        if undoable {
            self.push_undo(Undo::Move { pairs, created });
        }
    }

    /// Whether the listing on screen holds `path`, were it there: in its
    /// folder, under it with the sub-folders, or among the favourites.
    fn lists(&self, path: &Path) -> bool {
        if self.in_favorites() {
            return self.favorites.contains(path);
        }
        path.parent().is_some_and(|p| self.lists_folder(p))
    }

    /// Whether the images of `folder` are in the listing on screen: it is
    /// `dir`, or under it with the sub-folders.
    fn lists_folder(&self, folder: &Path) -> bool {
        let Some(dir) = self.dir.as_deref() else { return false };
        if favorites::is_virtual(dir) || self.archive {
            return false;
        }
        folder::same_path(folder, dir) || (self.deep && folder.ancestors().skip(1).any(|a| folder::same_path(a, dir)))
    }

    /// The shell has moved the files (see `move_pairs`).
    fn poll_move(&mut self, ctx: &egui::Context) {
        let Some(moving) = &self.moving else { return };
        let Ok(result) = moving.done.try_recv() else { return };
        let Some(Moving { pairs, undoable, created, remove_empty, .. }) = self.moving.take() else { return };
        // The shell may have moved some and not others.
        let (done, left): (Vec<_>, Vec<_>) = pairs.into_iter().partition(|(old, new)| !old.exists() && new.exists());
        match result {
            Err(e) if e == "cancelled" => self.notice(tr!("Moving cancelled".into(), "Перемещение отменено".into())),
            Err(e) => self.notice(tr!(format!("Cannot move: {e}"), format!("Не удалось переместить: {e}"))),
            Ok(()) if !left.is_empty() => {
                let n = left.len();
                self.notice(tr!(format!("Not moved: {n}"), format!("Не перемещено: {n}")));
            }
            Ok(()) => {}
        }
        if !done.is_empty() {
            self.files_moved(ctx, done, undoable, created);
        }
        if let Some(dir) = remove_empty {
            self.remove_if_empty(ctx, &dir);
        }
    }

    /// What `path` is rendered as for the clipboard, the wallpaper and
    /// printing: the image on screen as it is shown, turned and mirrored in
    /// the viewer, cropped while cropping; any other as it is.
    fn as_shown(&self, path: PathBuf) -> edit::Job {
        // The view's turn counts only for the image it shows.
        let shown = self.editable().filter(|(p, _)| *p == path);
        let (turns, flip) = if shown.is_some() { (self.view.turns, self.view.flip) } else { (0, false) };
        let (size, crop) = match (&shown, &self.crop) {
            (Some((_, picture)), Some(c)) => {
                let turned = self.view.rotated(picture.size());
                let crop = (!crate::crop::is_whole(c.rect, turned)).then(|| crate::crop::pixels(c.rect, turned));
                ([picture.meta.width, picture.meta.height], crop)
            }
            _ => ([0, 0], None),
        };
        edit::Job { src: path, dst: PathBuf::new(), size, turns, flip, crop }
    }

    /// Put the current image on the clipboard as it is shown (`as_shown`),
    /// decoded from its file at full size, on a thread.
    fn copy_image(&mut self, ctx: &egui::Context) {
        if self.clipboard.is_some() {
            self.notice(tr!("Still copying…".into(), "Копирование ещё идёт…".into()));
            return;
        }
        // The file may be the old one or the new one: the view's turns
        // cannot be composed with it until it is on screen again.
        if self.still_saving() {
            return;
        }
        let Some(path) = self.current.clone().filter(|p| crate::archive::is_file(p)) else { return };
        let job = self.as_shown(path);
        self.notice(tr!("Copying the image…".into(), "Копирование изображения…".into()));
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Windows' codecs may decode it.
            let _com = win::com_init();
            let _ = tx.send(Clipped::Copied(crate::clipboard::copy_image(&job)));
            ctx.request_repaint();
        });
        self.clipboard = Some(rx);
    }

    /// Make the current image as it is shown (`as_shown`) the desktop
    /// background, on a thread.
    fn set_wallpaper(&mut self, ctx: &egui::Context) {
        if self.handing.is_some() {
            self.notice(tr!("Still busy with the last one…".into(), "Предыдущее действие ещё выполняется…".into()));
            return;
        }
        if self.still_saving() {
            return;
        }
        let Some(path) = self.current.clone().filter(|p| crate::archive::is_file(p)) else { return };
        let job = self.as_shown(path);
        self.notice(tr!("Setting the desktop background…".into(), "Установка фона рабочего стола…".into()));
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _com = win::com_init();
            let _ = tx.send(Handed::Wallpaper(crate::wallpaper::set(&job)));
            ctx.request_repaint();
        });
        self.handing = Some(rx);
    }

    /// Print the current image, or those chosen, through Windows' Print
    /// Pictures dialog; those it cannot take as they are, or the image as
    /// it is shown (`as_shown`), are rendered on a thread first.
    fn print(&mut self, ctx: &egui::Context) {
        if self.handing.is_some() {
            self.notice(tr!("Still busy with the last one…".into(), "Предыдущее действие ещё выполняется…".into()));
            return;
        }
        if self.still_saving() {
            return;
        }
        let jobs: Vec<edit::Job> =
            self.targets().into_iter().filter(|p| crate::archive::is_file(p)).map(|p| self.as_shown(p)).collect();
        if jobs.is_empty() {
            return;
        }
        if !jobs.iter().all(crate::print::plain) {
            self.notice(tr!("Preparing to print…".into(), "Подготовка к печати…".into()));
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _com = win::com_init();
            let _ = tx.send(Handed::Print(crate::print::prepare(&jobs)));
            ctx.request_repaint();
        });
        self.handing = Some(rx);
    }

    fn poll_handed(&mut self) {
        let Some(Ok(done)) = self.handing.as_ref().map(mpsc::Receiver::try_recv) else { return };
        self.handing = None;
        let error = match done {
            Handed::Wallpaper(Ok(())) => {
                self.notice(tr!("Set as the desktop background".into(), "Установлено фоном рабочего стола".into()));
                return;
            }
            Handed::Wallpaper(Err(e)) => tr!(format!("Cannot set the desktop background: {e}"), format!("Не удалось установить фон: {e}")),
            Handed::Print(Ok(files)) => {
                // The notice of the preparation is over.
                self.notice = None;
                match crate::print::show(&files) {
                    Ok(()) => return,
                    Err(e) => tr!(format!("Cannot print: {e}"), format!("Не удалось напечатать: {e}")),
                }
            }
            Handed::Print(Err(e)) => tr!(format!("Cannot print: {e}"), format!("Не удалось напечатать: {e}")),
        };
        self.notice(error);
    }

    /// Open what the clipboard holds (see `clipboard::paste`), read on a
    /// thread: a large image is saved as a PNG first.
    fn paste(&mut self, ctx: &egui::Context) {
        if self.clipboard.is_some() {
            self.notice(tr!("Still copying…".into(), "Копирование ещё идёт…".into()));
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Clipped::Pasted(crate::clipboard::paste()));
            ctx.request_repaint();
        });
        self.clipboard = Some(rx);
    }

    fn poll_clipboard(&mut self, ctx: &egui::Context) {
        let Some(Ok(done)) = self.clipboard.as_ref().map(mpsc::Receiver::try_recv) else { return };
        self.clipboard = None;
        match done {
            Clipped::Copied(Ok(())) => {
                self.notice(tr!("Image copied to the clipboard".into(), "Изображение скопировано в буфер обмена".into()))
            }
            Clipped::Copied(Err(e)) => {
                self.notice(tr!(format!("Cannot copy the image: {e}"), format!("Не удалось скопировать изображение: {e}")))
            }
            Clipped::Pasted(Ok(path)) => self.open(ctx, path),
            Clipped::Pasted(Err(e)) => self.notice(e),
        }
    }

    fn poll_copy(&mut self, ctx: &egui::Context) {
        let Some(copying) = &self.copying else { return };
        let Ok(result) = copying.done.try_recv() else { return };
        let Some(Copying { to, count: n, fresh, created, .. }) = self.copying.take() else { return };
        let Some(fresh) = fresh else {
            let to = to.display().to_string();
            self.notice(match result {
                Ok(()) => tr!(format!("Favorites copied to {to}: {n}"), format!("Избранное скопировано в {to}: {n}")),
                Err(e) if e == "cancelled" => tr!("Copying cancelled".into(), "Копирование отменено".into()),
                Err(e) => tr!(format!("Cannot copy the favorites: {e}"), format!("Не удалось скопировать избранное: {e}")),
            });
            return;
        };
        // Into a pinned folder: the copies made that were new there can be
        // undone; copied into the folder shown, they are listed.
        let made: Vec<PathBuf> = fresh.into_iter().filter(|p| p.exists()).collect();
        let name = folder_label(&to);
        self.notice(match result {
            Ok(()) if n == 1 => tr!(format!("Copied to {name}"), format!("Скопировано в {name}")),
            Ok(()) => tr!(format!("Copied to {name}: {n}"), format!("Скопировано в {name}: {n}")),
            Err(e) if e == "cancelled" => tr!("Copying cancelled".into(), "Копирование отменено".into()),
            Err(e) => tr!(format!("Cannot copy: {e}"), format!("Не удалось скопировать: {e}")),
        });
        // The cell of the folder shows other images now; a folder made
        // for them gets a cell.
        self.sibling_folders = None;
        if let Some(gallery) = &mut self.gallery {
            gallery.forget_previews_of(std::slice::from_ref(&to));
        }
        let made_folder = created.as_deref().and_then(Path::parent).is_some_and(|p| self.lists_folder(p));
        if self.lists_folder(&to) || made_folder {
            self.list_again(ctx);
        }
        if !made.is_empty() {
            self.push_undo(Undo::Copy { copies: made, created });
        }
    }

    /// Unmark every favourite (after the user's yes).
    pub fn clear_favorites(&mut self, ctx: &egui::Context) {
        if let Err(e) = self.favorites.remove_if(|_| true) {
            self.notice(e);
        }
        if self.in_favorites() {
            self.relist(ctx);
        }
    }

    /// Unpin every folder from Quick Access (after the user's yes).
    pub fn unpin_all(&mut self) {
        match self.pinned.remove_if(|_| true) {
            Ok(n) => self.notice(tr!(format!("Unpinned from Quick Access: {n}"), format!("Откреплено от панели быстрого доступа: {n}"))),
            Err(e) => self.notice(e),
        }
        self.pinned_changed();
    }

    fn pick_file(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let image_extensions: Vec<&str> =
            folder::EXTENSIONS
                .iter()
                .copied()
                .chain(crate::wic::extensions().iter().map(String::as_str))
                .chain(crate::archive::EXTENSIONS.iter().copied())
                .collect();
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr!("Open image", "Открыть изображение"))
            .add_filter(tr!("Images and comic books", "Изображения и комиксы"), &image_extensions)
            .add_filter(tr!("All files", "Все файлы"), &["*"])
            .set_parent(frame);
        let dir = match &self.dir {
            Some(d) if favorites::is_dir(d) => self.current.as_deref().and_then(Path::parent),
            d => d.as_deref(),
        };
        if let Some(dir) = dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(ctx, path);
        }
    }

    /// Where the keys go now.
    fn key_mode(&self) -> Mode {
        if self.gallery_open {
            if self.focused_folder().is_some() { Mode::GalleryFolder } else { Mode::Gallery }
        } else if self.crop.is_some() {
            Mode::Crop
        } else if self.slideshow.is_some() {
            Mode::Slideshow
        } else {
            Mode::Viewer
        }
    }

    /// The image on screen if it can be turned, cropped and saved: the
    /// current one, decoded, in the viewer, not animated.
    pub fn editable(&self) -> Option<(PathBuf, Picture)> {
        let (path, picture) = self.shown.as_ref()?;
        (!self.gallery_open && self.current.as_ref() == Some(path) && !picture.meta.animated)
            .then(|| (path.clone(), picture.clone()))
    }

    /// Start cropping the image on screen (C): the frame in its middle,
    /// the image as large as the window allows.
    fn start_crop(&mut self) {
        // Not yet decoded, the image on screen is another: nothing to crop.
        if self.shown.as_ref().is_some_and(|(p, pic)| self.current.as_ref() == Some(p) && pic.meta.animated) {
            self.notice(tr!("Animated images cannot be edited".into(), "Анимированные изображения не редактируются".into()));
            return;
        }
        let Some((path, picture)) = self.editable() else { return };
        let size = self.view.rotated(picture.size());
        self.crop = Some(Crop::new(path, size, self.view.zoom, self.view.offset));
        self.view.zoom = Zoom::Fill;
        self.view.offset = Vec2::ZERO;
    }

    /// Stop cropping; the zoom and panning are as they were before.
    pub fn end_crop(&mut self) {
        if let Some(crop) = self.crop.take() {
            self.view.zoom = crop.zoom;
            self.view.offset = crop.offset;
        }
    }

    /// Cropping ends when its image is no longer on screen (another file
    /// opened from Explorer, the gallery).
    fn check_crop(&mut self) {
        let Some(crop) = &self.crop else { return };
        if self.editable().is_some_and(|(p, _)| p == crop.path) {
            return;
        }
        let same = self.current.as_ref() == Some(&crop.path);
        self.end_crop();
        if !same {
            // Another image: the cropped one's zoom and panning restored
            // are reset as for any next image.
            self.view.next_image();
        }
    }

    /// Carry out `cmd` while cropping: browsing and the file commands wait
    /// until the frame is saved or given up. False for the commands that
    /// work as in the viewer.
    fn run_in_crop(&mut self, ctx: &egui::Context, frame: &eframe::Frame, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Escape | Cmd::Crop => self.end_crop(),
            Cmd::Save => self.save(ctx, frame, false),
            Cmd::SaveAs => self.save(ctx, frame, true),
            Cmd::RotateLeft | Cmd::RotateRight => {
                // The frame turns with the image; the view turns it.
                if let (Some(crop), Some((_, picture))) = (&mut self.crop, &self.shown) {
                    crop.turn(self.view.rotated(picture.size()), cmd == Cmd::RotateRight);
                }
                return false;
            }
            Cmd::FlipHorizontal | Cmd::FlipVertical => {
                if let (Some(crop), Some((_, picture))) = (&mut self.crop, &self.shown) {
                    crop.mirror(self.view.rotated(picture.size()), cmd == Cmd::FlipHorizontal);
                }
                return false;
            }
            // Panning, never browsing.
            Cmd::Arrow(arrow) => {
                let ppp = ctx.pixels_per_point();
                let can = self.shown.as_ref().map_or([false; 2], |(_, p)| self.view.pannable(p.size(), self.viewport, ppp));
                return !can[usize::from(matches!(arrow, Arrow::Up | Arrow::Down))];
            }
            Cmd::ZoomIn
            | Cmd::ZoomOut
            | Cmd::Actual
            | Cmd::Fit
            | Cmd::Fill
            | Cmd::Cover
            | Cmd::FullScreen
            | Cmd::WindowFullScreen
            | Cmd::Slideshow
            | Cmd::SlideshowInterval(_)
            | Cmd::SlideshowLoop
            | Cmd::ToggleToolbar
            | Cmd::ToggleStatusBar
            | Cmd::Info
            | Cmd::Shortcuts
            | Cmd::About
            | Cmd::CopyImage
            | Cmd::Wallpaper
            | Cmd::Print
            | Cmd::Close => return false,
            _ => {}
        }
        true
    }

    /// Save the image on screen turned, and cropped while cropping: over
    /// its file, or with `as_new` (or in a format that cannot be saved
    /// over) into a file the user picks. On a thread; `poll_save` takes the
    /// outcome.
    fn save(&mut self, ctx: &egui::Context, frame: &eframe::Frame, as_new: bool) {
        if self.still_saving() {
            return;
        }
        if self.shown.as_ref().is_some_and(|(_, p)| p.meta.animated) {
            self.notice(tr!("Animated images cannot be edited".into(), "Анимированные изображения не редактируются".into()));
            return;
        }
        let Some((path, picture)) = self.editable() else { return };
        let turned = self.view.rotated(picture.size());
        let crop = self
            .crop
            .as_ref()
            .filter(|c| !crate::crop::is_whole(c.rect, turned))
            .map(|c| crate::crop::pixels(c.rect, turned));
        let dst = if !as_new && edit::can_overwrite(&path) {
            if !self.view.changed() && crop.is_none() {
                self.notice(tr!(
                    "Nothing to save: the image is neither turned, mirrored nor cropped".into(),
                    "Нечего сохранять: изображение не повёрнуто, не отражено и не обрезано".into()
                ));
                return;
            }
            path.clone()
        } else {
            let suffix = if crop.is_some() {
                "_crop"
            } else if self.view.turns != 0 {
                "_rotate"
            } else if self.view.flip {
                "_flip"
            } else {
                ""
            };
            let Some(dst) = Self::pick_save_path(frame, &path, suffix) else { return };
            dst
        };
        let size = [picture.meta.width, picture.meta.height];
        let job = edit::Job { src: path, dst, size, turns: self.view.turns, flip: self.view.flip, crop };
        self.start_save(ctx, vec![job], false);
    }

    /// A save is under way, or the image saved is not yet on screen again
    /// (until it is, the view's turns are those already saved, and a second
    /// Ctrl+S would turn the file twice): said in a notice.
    fn still_saving(&mut self) -> bool {
        let busy = self.saving.is_some() || self.reloading.is_some();
        if busy {
            self.notice(tr!("Saving…".into(), "Сохранение…".into()));
        }
        busy
    }

    /// The current image, or those chosen in the gallery, can be converted
    /// (File → Convert To), decoded or not.
    pub fn can_convert(&self) -> bool {
        self.current.is_some() && self.crop.is_none() && self.saving.is_none() && !self.archive
    }

    /// Save the current image in `format` beside its file, under its name
    /// with that format's extension (numbered if taken), turned as it is
    /// shown in the viewer; the file itself stays as it is, and current.
    /// Several chosen in the gallery are converted one after another.
    fn convert(&mut self, ctx: &egui::Context, format: edit::Format) {
        if self.still_saving() {
            return;
        }
        let targets = self.targets();
        if targets.len() > 1 {
            self.convert_all(ctx, format, targets);
            return;
        }
        if self.editable().is_none() && self.shown.as_ref().is_some_and(|(p, pic)| self.current.as_ref() == Some(p) && pic.meta.animated) {
            self.notice(tr!("Animated images cannot be edited".into(), "Анимированные изображения не редактируются".into()));
            return;
        }
        let Some(path) = self.current.clone().filter(|p| p.is_file()) else { return };
        // The view's turn counts only for the image it shows.
        let (turns, flip) = if self.editable().is_some() { (self.view.turns, self.view.flip) } else { (0, false) };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = edit::suggested_name(&path, format.extensions()[0], "", |n| dir.join(n).exists());
        let job = edit::Job { src: path, dst: dir.join(name), size: [0, 0], turns, flip, crop: None };
        self.start_save(ctx, vec![job], true);
    }

    /// Convert `paths` to `format`, each beside itself; two of the same name
    /// (`a.png` and `a.heic` into JPEG) do not take the same new name.
    fn convert_all(&mut self, ctx: &egui::Context, format: edit::Format, paths: Vec<PathBuf>) {
        let mut planned = std::collections::HashSet::new();
        let jobs: Vec<edit::Job> = paths
            .into_iter()
            .filter(|p| p.is_file())
            .map(|src| {
                let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
                let name = edit::suggested_name(&src, format.extensions()[0], "", |n| {
                    let dst = dir.join(n);
                    dst.exists() || planned.contains(&dst.to_string_lossy().to_lowercase())
                });
                let dst = dir.join(name);
                planned.insert(dst.to_string_lossy().to_lowercase());
                edit::Job { src, dst, size: [0, 0], turns: 0, flip: false, crop: None }
            })
            .collect();
        let n = jobs.len();
        self.notice(tr!(format!("Converting 0 of {n}…"), format!("Конвертирование: 0 из {n}…")));
        self.start_save(ctx, jobs, true);
    }

    /// Carry out `jobs` one after another on a thread; `poll_save` takes
    /// the outcomes.
    fn start_save(&mut self, ctx: &egui::Context, jobs: Vec<edit::Job>, convert: bool) {
        if jobs.is_empty() {
            return;
        }
        let (tx, done) = mpsc::channel();
        let (ctx, work) = (ctx.clone(), jobs.clone());
        std::thread::spawn(move || {
            // Windows' codecs may decode the originals.
            let _com = win::com_init();
            for job in &work {
                if tx.send(edit::save(job)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        self.saving = Some(Saving { jobs, convert, done, results: Vec::new() });
    }

    /// The file to save `path` as, from the Save As dialog: its own format
    /// first if it can be saved in it, otherwise JPEG; the name suggested
    /// has `suffix` and is not taken (see `edit::suggested_name`).
    fn pick_save_path(frame: &eframe::Frame, path: &Path, suffix: &str) -> Option<PathBuf> {
        // An image in an archive keeps its format, which is only not saved
        // over.
        let own = edit::Format::of(path).filter(|_| edit::can_overwrite(path) || crate::archive::inside(path));
        let first = own.unwrap_or(edit::Format::Jpeg);
        let ext = match own {
            Some(_) => path.extension().unwrap_or_default().to_string_lossy().into_owned(),
            None => "jpg".to_string(),
        };
        // Of an image in an archive, beside the archive.
        let dir = crate::archive::folder_of(path).unwrap_or_default();
        let name = edit::suggested_name(path, &ext, suffix, |name| dir.join(name).exists());
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr!("Save As", "Сохранить как"))
            .set_file_name(name)
            .set_parent(frame);
        for format in std::iter::once(first).chain(edit::Format::ALL.into_iter().filter(|&f| f != first)) {
            dialog = dialog.add_filter(format.name(), format.extensions());
        }
        if !dir.as_os_str().is_empty() {
            dialog = dialog.set_directory(&dir);
        }
        dialog.save_file()
    }

    /// The outcome of `save` and `convert`: a file saved over is decoded
    /// again, one saved as a new file opened, a converted one listed; Ctrl+Z
    /// can undo it. Several converted are counted as they come.
    fn poll_save(&mut self, ctx: &egui::Context) {
        let Some(saving) = &mut self.saving else { return };
        let before = saving.results.len();
        saving.results.extend(saving.done.try_iter());
        let (done, total) = (saving.results.len(), saving.jobs.len());
        if done < total {
            if done > before && total > 1 {
                self.notice(tr!(format!("Converting {done} of {total}…"), format!("Конвертирование: {done} из {total}…")));
            }
            return;
        }
        let Saving { jobs, convert, results, .. } = self.saving.take().expect("checked above");
        if jobs.len() > 1 {
            self.converted_all(ctx, &jobs, results);
            return;
        }
        let (Some(job), Some(result)) = (jobs.into_iter().next(), results.into_iter().next()) else { return };
        let name = file_name(&job.dst);
        match result {
            Ok(saved) => {
                self.push_undo(Undo::Save(vec![(job.dst.clone(), saved.before)]));
                if self.crop.as_ref().is_some_and(|c| c.path == job.src) {
                    self.end_crop();
                }
                self.written(ctx, std::slice::from_ref(&job.dst));
                if convert {
                    self.notice(tr!(format!("Converted: {name}"), format!("Сконвертировано: {name}")));
                    return;
                }
                if !folder::same_path(&job.src, &job.dst) {
                    self.open(ctx, job.dst.clone());
                }
                self.notice(if saved.lossless {
                    tr!(format!("Saved without recompression: {name}"), format!("Сохранено без пересжатия: {name}"))
                } else {
                    tr!(format!("Saved: {name}"), format!("Сохранено: {name}"))
                });
            }
            Err(e) => self.notice(tr!(format!("Cannot save {name}: {e}"), format!("Не удалось сохранить {name}: {e}"))),
        }
    }

    /// The outcome of converting several: the copies listed, one Ctrl+Z
    /// for them all, and how many failed.
    fn converted_all(&mut self, ctx: &egui::Context, jobs: &[edit::Job], results: Vec<Result<edit::Saved, String>>) {
        let total = jobs.len();
        let (mut saved, mut failed) = (Vec::new(), Vec::new());
        for (job, result) in jobs.iter().zip(results) {
            match result {
                Ok(s) => saved.push((job.dst.clone(), s.before)),
                Err(e) => failed.push(format!("{}: {e}", file_name(&job.src))),
            }
        }
        let paths: Vec<PathBuf> = saved.iter().map(|(p, _)| p.clone()).collect();
        let n = saved.len();
        if !saved.is_empty() {
            self.push_undo(Undo::Save(saved));
        }
        self.written(ctx, &paths);
        self.notice(match failed.first() {
            None => tr!(format!("Converted: {n} files"), format!("Сконвертировано файлов: {n}")),
            Some(e) => tr!(
                format!("Converted {n} of {total}; {e}"),
                format!("Сконвертировано {n} из {total}; {e}")
            ),
        });
    }

    /// The editors to choose from for the current image: those Windows
    /// offers for its type, and the one chosen last if it is not among
    /// them, first.
    pub fn editor_choices(&mut self) -> Vec<Editor> {
        let ext = self
            .targets()
            .first()
            .and_then(|f| f.extension())
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        let mut choices = self.editors.entry(ext.clone()).or_insert_with(|| crate::editors::for_extension(&ext)).clone();
        if let Some(editor) = &self.editor
            && !choices.iter().any(|e| e.id.eq_ignore_ascii_case(&editor.id))
        {
            choices.insert(0, editor.clone());
        }
        choices
    }

    /// Open the current image, or those chosen in the gallery, in `editor`,
    /// which is kept as the editor; with none, in the one kept, or with
    /// Windows' "edit" verb. More than `EDIT_WITHOUT_ASKING` files wait for
    /// the user's yes (`confirm_edit`).
    fn open_in_editor(&mut self, ctx: &egui::Context, editor: Option<Editor>) {
        let files: Vec<PathBuf> = self.targets().into_iter().filter(|p| p.is_file()).collect();
        if files.len() > EDIT_WITHOUT_ASKING {
            self.confirm_edit = Some(EditRequest { files, editor });
        } else {
            self.edit_files(ctx, files, editor);
        }
    }

    /// The name of the editor `files` would open in (see `open_in_editor`).
    pub fn editor_name(&self, editor: Option<&Editor>) -> Option<String> {
        editor.or(self.editor.as_ref()).map(|e| e.name.clone())
    }

    /// Open `files` in `editor` (kept as the editor), or in the one kept.
    /// On a thread: a program may take a while to start.
    pub fn edit_files(&mut self, ctx: &egui::Context, files: Vec<PathBuf>, editor: Option<Editor>) {
        if files.is_empty() {
            return;
        }
        if editor.is_some() {
            self.editor = editor;
        }
        let (tx, rx) = mpsc::channel();
        let (ctx, editor) = (ctx.clone(), self.editor.clone());
        std::thread::spawn(move || {
            let _ = tx.send(crate::editors::open(editor.as_ref(), &files));
            ctx.request_repaint();
        });
        self.opening = Some(rx);
    }

    /// A failure to start the editor, shown.
    fn poll_opening(&mut self) {
        let Some(rx) = &self.opening else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.opening = None;
        if let Err(e) = result {
            self.notice(tr!(format!("Cannot open the editor: {e}"), format!("Не удалось открыть редактор: {e}")));
        }
    }

    /// An image is being saved.
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }

    /// The checkerboard under an image placed at `place`, if chosen.
    fn paint_checker(&mut self, painter: &egui::Painter, place: Rect, ppp: f32) {
        if self.checker {
            let texture = self.checker_texture.get_or_insert_with(|| view::checker_texture(painter.ctx()));
            view::paint_checker(painter, texture.id(), place, ppp);
        }
    }

    /// Paint `picture` at `place`, filtered as chosen.
    fn paint_picture(&mut self, painter: &egui::Painter, picture: &Picture, place: Rect, area: Rect, ppp: f32) {
        use view::Filter;
        let texture = &picture.texture;
        // The zoom shown (2.0 is 200%).
        let zoom = self.view.scale(picture.size(), area, ppp);
        let trace = log::log_enabled!(log::Level::Debug);
        // Bilinear is the texture's own, through a program only to be
        // timed; bicubic at 100% gives the pixels as they are, as the mesh
        // does for a sixteenth of the work; pixelated only enlarged.
        let wanted = match self.filter {
            Filter::Bilinear => trace,
            Filter::Bicubic => zoom != 1.0,
            Filter::Pixels => zoom > 1.0,
        };
        let program = match self.programs.get(&self.filter) {
            _ if !wanted => None,
            Some(program) => program.clone(),
            // Made in the frame after the one that wants it (8-85 ms, the
            // first one the longest): the image on screen first, as a mesh.
            None if self.program_due => {
                self.program_due = false;
                let program = crate::filter::Program::new(&self.gl, self.filter);
                self.programs.insert(self.filter, program.clone());
                program
            }
            None => {
                self.program_due = true;
                painter.ctx().request_repaint();
                None
            }
        };
        if trace {
            // Every program polled, not only up to the first with some pending.
            let pending = self.programs.values().flatten().filter(|p| p.poll_timings(&self.gl)).count() > 0;
            if pending {
                painter.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        let (turns, flip) = (self.view.turns, self.view.flip);
        // GL_NEAREST only where the pixelated program cannot be made.
        let failed = matches!(self.programs.get(&Filter::Pixels), Some(None));
        texture.set_smooth(!(self.filter == Filter::Pixels && zoom > 1.0 && failed));
        match program {
            Some(program) => {
                let uv = std::array::from_fn(|i| view::corner_uv(i, turns, flip));
                crate::filter::paint(painter, &program, texture.native(), texture.levels(), place, uv, area, zoom);
            }
            None => view::paint(painter, texture.id(), place, turns, flip),
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
            let place = view.place(size, rect, ppp);
            self.paint_checker(&painter, place, ppp);
            view::paint(&painter, texture, place, 0, false);
        } else if let Some((_, picture)) = self.shown.clone() {
            let size = picture.size();
            if response.dragged_by(PointerButton::Primary) && self.crop.is_none() {
                self.view.pan(response.drag_delta(), size, rect, ppp);
            }
            let place = self.view.place(size, rect, ppp);
            self.paint_checker(&painter, place, ppp);
            self.paint_picture(&painter, &picture, place, rect, ppp);
            if let Some((path, _)) = &self.shown {
                let picked = self.pick(response.hover_pos(), place, path, size);
                if picked != self.picked {
                    // The panel, drawn before the image area, shows it in
                    // the next frame.
                    self.picked = picked;
                    ctx.request_repaint();
                }
            }
            if let Some(crop) = &mut self.crop {
                crate::ui::crop::frame(crop, &response, &painter, place, self.view.rotated(size));
            } else if self.view.pannable(size, rect, ppp).contains(&true) && response.hovered() {
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

        let cropping = self.crop.is_some();
        if crate::input::double_clicked(&response) && self.image_clicked && !cropping {
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
        let modal_open = self.modal_open();
        if !modal_open && !egui::Popup::is_any_open(&ctx) {
            let (browse, zoom) = self.wheel.read(&ctx);
            // Wheel up: the previous image; not while cropping.
            let browse = if cropping { 0 } else { browse };
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
        if response.secondary_clicked() {
            self.menu_picked = self.picked;
        }
        if !cropping {
            response.context_menu(|ui| self.context_menu(ui));
        }
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
            (_, Some(_)) if self.gallery_open && self.in_favorites() => format!("{} - qview", tr!("Favorites", "Избранное")),
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
                area.max.x -= if self.show_info { self.info_width } else { 0.0 };
            }
            self.viewport = area;
        }
        if frame_nr < 8 {
            log::debug!("frame {frame_nr} at {:.0} ms", crate::since_start_ms());
        }
        self.uncloak(&ctx);
        // The previous frame, which may have drawn them, is painted.
        crate::texture::delete_dropped();

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
        self.poll_scan(&ctx);
        self.poll_favorites_gone();
        self.poll_pinned_gone();
        self.poll_decoded(&ctx, frame);
        if let Some(gallery) = &mut self.gallery {
            gallery.poll(&self.gl, frame, &ctx);
        }
        self.poll_delete(&ctx);
        self.poll_copy(&ctx);
        self.poll_move(&ctx);
        self.poll_clipboard(&ctx);
        self.poll_handed();
        self.poll_save(&ctx);
        self.poll_opening();
        self.updates.poll();
        self.handle_drop(&ctx);
        self.sync_shown();

        let modal_open = self.modal_open();
        // Taken in every frame; a dialog's text field pastes for itself.
        let paste = win::take_paste();
        if !modal_open && !egui::Popup::is_any_open(&ctx) {
            // Keys are the viewer's: no widget keeps the focus to take
            // Space or Enter as a click.
            // But the gallery's filter, while it is typed in: the keys are
            // its own then.
            let typing = std::mem::take(&mut self.filter_focused)
                || ctx.memory(|m| m.has_focus(egui::Id::new(crate::ui::gallery::FILTER_ID)));
            if let Some(id) = ctx.memory(|m| m.focused()).filter(|_| !typing) {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
            if paste && !typing {
                self.run(&ctx, frame, Cmd::Paste);
            }
            let cmds = if typing { Vec::new() } else { crate::input::keys(&ctx, self.key_mode()) };
            for cmd in cmds {
                self.run(&ctx, frame, cmd);
            }
            self.sync_shown();
        }
        self.check_crop();
        self.complete_current_if_needed(&ctx);
        self.animate(&ctx);
        self.slideshow_tick(&ctx);

        // The gallery keeps its bars in the frames before full screen is
        // left (`enter_gallery`).
        self.fullscreen = Self::is_fullscreen(&ctx);
        let fullscreen = self.fullscreen && !self.gallery_open;
        self.update_histogram(&ctx, self.show_info && !fullscreen);
        self.update_colour(&ctx, self.show_info && !fullscreen);
        self.update_stats(&ctx, self.show_info && !fullscreen);
        if !fullscreen {
            self.menu_bar(root_ui);
            if self.show_toolbar {
                self.toolbar(root_ui);
            }
            if self.show_status_bar {
                self.status_bar(root_ui);
            }
            if self.show_info {
                self.update_info(&ctx);
                self.info_panel(root_ui);
            }
        }
        // In full screen too: it has the frame's size and buttons.
        if self.crop.is_some() {
            self.crop_bar(root_ui);
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
        self.player = None;
        self.shown = None;
        self.cache.clear();
        self.pending.clear();
        self.partial.clear();
        if let Some(gallery) = &mut self.gallery {
            gallery.clear();
        }
        crate::texture::delete_dropped();
        for program in self.programs.drain().filter_map(|(_, p)| p) {
            program.delete(&self.gl);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(TOOLBAR_KEY, self.show_toolbar.to_string());
        storage.set_string(STATUS_BAR_KEY, self.show_status_bar.to_string());
        storage.set_string(BACKGROUND_KEY, background_to_hex(self.background));
        storage.set_string(CHECKER_KEY, self.checker.to_string());
        storage.set_string(FILTER_KEY, self.filter.name().to_string());
        storage.set_string(ZOOM_KEY, self.view.mode.name().unwrap_or("fit").to_string());
        storage.set_string(THUMB_SIZE_KEY, self.thumb_size.round().to_string());
        storage.set_string(TREE_WIDTH_KEY, self.tree_width.round().to_string());
        storage.set_string(INFO_KEY, self.show_info.to_string());
        storage.set_string(FOLDERS_KEY, self.folders_folded.to_string());
        storage.set_string(INFO_WIDTH_KEY, self.info_width.round().to_string());
        storage.set_string(HISTOGRAM_KEY, self.show_histogram.to_string());
        storage.set_string(COLOUR_KEY, self.show_colour.to_string());
        storage.set_string(HISTOGRAM_CHANNELS_KEY, self.histogram_channels.name().to_string());
        storage.set_string(SLIDESHOW_SECONDS_KEY, self.slideshow_seconds.to_string());
        storage.set_string(SLIDESHOW_LOOP_KEY, self.slideshow_loop.to_string());
        storage.set_string(THUMB_FILL_KEY, self.thumb_fill.to_string());
        let aspect = self.thumb_aspect.map_or(AUTO, gallery::aspect_name);
        storage.set_string(THUMB_ASPECT_KEY, aspect.to_string());
        storage.set_string(LANGUAGE_KEY, self.lang.name().to_string());
        storage.set_string(SORT_KEY, self.sort.key.name().to_string());
        storage.set_string(SORT_DESCENDING_KEY, self.sort.descending.to_string());
        storage.set_string(FAVORITES_SORT_KEY, self.favorites_sort.key.name().to_string());
        storage.set_string(FAVORITES_SORT_DESCENDING_KEY, self.favorites_sort.descending.to_string());
        storage.set_string(BY_FOLDER_KEY, self.by_folder.to_string());
        let editor = self.editor.clone().unwrap_or(Editor { id: String::new(), name: String::new() });
        storage.set_string(EDITOR_KEY, editor.id);
        storage.set_string(EDITOR_NAME_KEY, editor.name);
        storage.set_string(CHECK_UPDATES_KEY, self.updates.enabled.to_string());
        storage.set_string(LAST_UPDATE_CHECK_KEY, self.updates.last_check.to_string());
        if let Some(r) = self.hwnd.filter(|_| !self.fullscreen).and_then(win::normal_rect) {
            storage.set_string(WINDOW_NORMAL_KEY, format!("{},{},{},{}", r[0], r[1], r[2], r[3]));
        }
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
