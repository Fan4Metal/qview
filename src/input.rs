//! Keyboard and mouse wheel as commands, and the key bindings.
//!
//! Keys are matched by `Event::Key::key`, which egui-winit fills with the
//! logical key or, for a letter of a non-Latin layout, the physical one:
//! `F` works with the Russian layout too. Numpad `*` has no `egui::Key`
//! and arrives only as text. `Ctrl+C` arrives as `Event::Copy` (with Shift
//! held, Copy Image); `Ctrl+V` is watched for by `win::watch_paste`.

use egui::{Event, Key, Modifiers, MouseWheelUnit, PointerButton};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// Where a key moves in the gallery's grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Arrow(Arrow),
    PageUp,
    PageDown,
    First,
    Last,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Next,
    Prev,
    First,
    Last,
    /// Pans an image larger than the window, otherwise browses.
    Arrow(Arrow),
    ZoomIn,
    ZoomOut,
    Fit,
    /// Fit the window, enlarging a small image too.
    Fill,
    /// Fill the whole window, cropping the edges.
    Cover,
    Actual,
    RotateLeft,
    RotateRight,
    /// Mirror the image as shown left to right (H), or top to bottom (V).
    FlipHorizontal,
    FlipVertical,
    /// Put the image as shown on the clipboard (Ctrl+Shift+C).
    CopyImage,
    /// Make the image as shown the desktop background.
    Wallpaper,
    /// Open Windows' Print Pictures dialog for the image, or those chosen
    /// (Ctrl+P).
    Print,
    /// Type a filter for the gallery's images (Ctrl+F).
    Find,
    /// Open what the clipboard holds: files, a path, an image (Ctrl+V,
    /// Shift+Insert; see `win::watch_paste`).
    Paste,
    /// Pause an animation, or play it on (P).
    Pause,
    /// Show the next frame of an animation (`.`) or the previous one
    /// (`,`), paused.
    NextFrame,
    PrevFrame,
    FullScreen,
    /// Full screen without leaving the gallery (Ctrl+Shift+F there; F
    /// shows the image in full screen).
    WindowFullScreen,
    /// Start a slideshow in full screen, or stop it (Shift+F).
    Slideshow,
    /// Seconds each image of a slideshow stays (View → Slideshow).
    SlideshowInterval(u32),
    /// A slideshow starts over after the last image, or stops.
    SlideshowLoop,
    /// Leaves full screen, or closes the viewer.
    Escape,
    Close,
    Delete,
    /// Rename the current file (F2).
    Rename,
    /// Undo the last rename or save (Ctrl+Z).
    Undo,
    /// Crop the image (C), or stop cropping.
    Crop,
    /// Save the image turned and cropped over its file (Ctrl+S), and while
    /// cropping with Enter.
    Save,
    /// Save it into a file the user picks (Ctrl+Shift+S).
    SaveAs,
    /// Open the current image, or those chosen, in the editor chosen last
    /// (Ctrl+E); with none chosen, with Windows' "edit" verb.
    Edit,
    /// Open them in this one of `App::menu_editors` and keep it as the
    /// editor (File → Edit With).
    EditWith(usize),
    /// Pick a program to open them in, kept as the editor.
    EditWithOther,
    /// Move in the gallery choosing the images on the way (Shift with the
    /// arrows, Page Up/Down, Home and End).
    SelectTo(Move),
    /// Choose every image of the gallery (Ctrl+A).
    SelectAll,
    /// Save it in this format beside its file, which stays current (File →
    /// Convert To).
    ConvertTo(crate::edit::Format),
    Copy,
    Open,
    ShowInExplorer,
    Refresh,
    ToggleToolbar,
    ToggleStatusBar,
    /// Show or hide the information panel (I).
    Info,
    /// The next images keep the zoom and the panning, or no longer do.
    KeepZoom,
    /// Sort the folder by this (View → Sort), in the same direction.
    SortBy(crate::folder::SortKey),
    /// Reverse the order of the folder.
    SortDescending,
    Shortcuts,
    About,
    Associations,
    /// Opens the gallery, or leaves it for the selected image.
    Gallery,
    /// Open the sub-folder whose cell has the gallery's cursor (Enter and
    /// G there; the Open item of its menu).
    OpenFolder,
    /// A page of the gallery up or down (Page Up / Page Down there).
    PageUp,
    PageDown,
    /// The gallery's previous or next folder in its history (Alt+← and
    /// Alt+→ there, the mouse's side buttons), and the folder above.
    Back,
    Forward,
    Up,
    /// Mark the current image as a favourite, or unmark it (S).
    Favorite,
    /// List the favourites in the gallery.
    Favorites,
    /// The favourites to the clipboard, as files.
    CopyFavorites,
    /// The favourites to a folder the user picks.
    CopyFavoritesTo,
    /// Forget them all, once the user says so.
    ClearFavorites,
    /// Unpin every folder from Quick Access, once the user says so.
    UnpinAll,
    /// List the folder of the current image alone, from the favourites or
    /// the sub-folders, the image staying current.
    GoToFolder,
    /// Show Quick Access, the pinned folders, in the gallery.
    QuickAccess,
    /// Move the current image, or those chosen, into the pinned folder
    /// with this key (Alt+1 to Alt+9), or copy them there (Shift+Alt).
    MoveTo(u8),
    CopyTo(u8),
    /// Move them, or copy them, into a folder of the Move to Folder menu's
    /// list (the parent folder and the sub-folders of the image's folder,
    /// `App::menu_folders`), by its index there.
    MoveToListed(usize),
    CopyToListed(usize),
    /// Move them, or copy them, into a new folder, named in a dialog
    /// (Alt+N, Shift+Alt+N).
    MoveToNew,
    CopyToNew,
    /// Move them, or copy them, into a folder picked in a dialog.
    MoveToOther,
    CopyToOther,
}

/// The digit of a number key, 1 to 9.
fn digit(key: Key) -> Option<u8> {
    Some(match key {
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    })
}

/// The command of a key press with `m` held, if any. `repeat` is set for
/// the presses of a held key, which only browse, pan and zoom.
pub fn command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    use Cmd::*;
    // Shift is part of typing `+` or `*`, so it is ignored for symbols.
    let plain = !m.ctrl && !m.alt;
    let letter = plain && !m.shift;
    // Alt+1 to Alt+9 move the image into a pinned folder, with Shift copy
    // it (not Ctrl+Alt, which is AltGr in some layouts).
    if m.alt && !m.ctrl && !repeat
        && let Some(n) = digit(key)
    {
        return Some(if m.shift { CopyTo(n) } else { MoveTo(n) });
    }
    // Alt+N: into a new folder.
    if m.alt && !m.ctrl && !repeat && key == Key::N {
        return Some(if m.shift { CopyToNew } else { MoveToNew });
    }
    let cmd = match key {
        Key::ArrowRight if m.ctrl && m.alt => RotateRight,
        Key::ArrowLeft if m.ctrl && m.alt => RotateLeft,
        Key::ArrowRight if m.ctrl => Next,
        Key::ArrowLeft if m.ctrl => Prev,
        Key::ArrowRight if plain => Arrow(self::Arrow::Right),
        Key::ArrowLeft if plain => Arrow(self::Arrow::Left),
        Key::ArrowUp if plain => Arrow(self::Arrow::Up),
        Key::ArrowDown if plain => Arrow(self::Arrow::Down),
        Key::PageDown | Key::Space if plain => Next,
        Key::PageUp | Key::Backspace if plain => Prev,
        Key::Home if plain => First,
        Key::End if plain => Last,
        Key::Plus | Key::Equals if !m.alt => ZoomIn,
        Key::Minus if !m.alt => ZoomOut,
        // 1 and 2 side by side; numpad `/` (and `*`, see `keys`) as well.
        Key::Num1 if letter => Actual,
        Key::Num2 if letter => Fit,
        Key::Num3 if letter => Fill,
        Key::Num4 if letter => Cover,
        Key::Slash if plain => Actual,
        Key::OpenBracket if plain => RotateLeft,
        Key::CloseBracket if plain => RotateRight,
        Key::H if letter => FlipHorizontal,
        Key::V if letter => FlipVertical,
        Key::P if letter => Pause,
        Key::P if m.ctrl && !m.alt && !m.shift => Print,
        Key::Period if letter => NextFrame,
        Key::Comma if letter => PrevFrame,
        Key::F if m.ctrl && !m.shift && !m.alt => Find,
        Key::F if plain && m.shift => Slideshow,
        Key::F if letter || (m.ctrl && m.shift && !m.alt) => FullScreen,
        Key::T if letter => ToggleToolbar,
        Key::B if letter => ToggleStatusBar,
        Key::I if letter => Info,
        Key::L if letter => KeepZoom,
        Key::S if m.ctrl && m.shift && !m.alt => SaveAs,
        Key::S if m.ctrl && !m.alt => Save,
        Key::S if letter => Favorite,
        Key::C if letter => Crop,
        Key::G if letter => Gallery,
        Key::Enter if plain && !m.shift => Gallery,
        Key::W if m.ctrl && !m.alt => Close,
        Key::O if m.ctrl && !m.alt => Open,
        Key::E if m.ctrl && !m.alt && !m.shift => Edit,
        Key::Delete if letter => Delete,
        Key::F2 if letter => Rename,
        Key::Z if m.ctrl && !m.alt && !m.shift => Undo,
        Key::Escape => Escape,
        Key::F5 => Refresh,
        Key::F1 => Shortcuts,
        _ => return None,
    };
    let repeats = matches!(cmd, Next | Prev | First | Last | Arrow(_) | ZoomIn | ZoomOut | PageUp | PageDown | NextFrame | PrevFrame);
    (!repeat || repeats).then_some(cmd)
}

/// The command of a key press in the gallery: Page Up and Page Down move
/// a page there, the other keys are the viewer's (see [`command`]).
pub fn gallery_command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    if m.shift && !m.ctrl && !m.alt {
        let to = match key {
            Key::ArrowLeft => Some(Move::Arrow(Arrow::Left)),
            Key::ArrowRight => Some(Move::Arrow(Arrow::Right)),
            Key::ArrowUp => Some(Move::Arrow(Arrow::Up)),
            Key::ArrowDown => Some(Move::Arrow(Arrow::Down)),
            Key::PageUp => Some(Move::PageUp),
            Key::PageDown => Some(Move::PageDown),
            Key::Home => Some(Move::First),
            Key::End => Some(Move::Last),
            _ => None,
        };
        if let Some(to) = to {
            return Some(Cmd::SelectTo(to));
        }
    }
    match key {
        Key::A if m.ctrl && !m.alt && !m.shift => (!repeat).then_some(Cmd::SelectAll),
        // As in many programs' lists; 100% (the viewer's `/`) means nothing here.
        Key::Slash if !m.ctrl && !m.alt => (!repeat).then_some(Cmd::Find),
        Key::PageUp if !m.ctrl && !m.alt => Some(Cmd::PageUp),
        Key::PageDown if !m.ctrl && !m.alt => Some(Cmd::PageDown),
        Key::F if m.ctrl && m.shift && !m.alt => Some(Cmd::WindowFullScreen),
        // As in Explorer.
        Key::ArrowLeft if m.alt && !m.ctrl && !repeat => Some(Cmd::Back),
        // Held, it does not repeat, nor select the previous image.
        Key::Backspace if !m.alt && !m.ctrl => (!repeat).then_some(Cmd::Back),
        Key::ArrowRight if m.alt && !m.ctrl && !repeat => Some(Cmd::Forward),
        Key::ArrowUp if m.alt && !m.ctrl && !repeat => Some(Cmd::Up),
        _ => command(key, m, repeat),
    }
}

/// The command of a key press while cropping: Enter saves, the other keys
/// are the viewer's (see [`command`]; `App::run_in_crop` decides which of
/// them work).
/// The gallery's keys with a sub-folder's cell under the cursor: Enter and
/// G open the folder.
pub fn folder_command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    match gallery_command(key, m, repeat) {
        Some(Cmd::Gallery) => Some(Cmd::OpenFolder),
        other => other,
    }
}

/// The viewer's keys during a slideshow: Space pauses and resumes it.
pub fn slideshow_command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    match key {
        Key::Space if !m.ctrl && !m.alt && !m.shift => (!repeat).then_some(Cmd::Pause),
        _ => command(key, m, repeat),
    }
}

pub fn crop_command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    match key {
        Key::Enter if !m.ctrl && !m.alt && !m.shift => (!repeat).then_some(Cmd::Save),
        _ => command(key, m, repeat),
    }
}

/// Where the keys go.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Viewer,
    Gallery,
    /// The gallery with a sub-folder's cell under the cursor: Enter and G
    /// open it, not the image.
    GalleryFolder,
    Crop,
    /// A slideshow: Space pauses it instead of browsing.
    Slideshow,
}

/// Commands of this frame's key presses in `mode`.
pub fn keys(ctx: &egui::Context, mode: Mode) -> Vec<Cmd> {
    let gallery = matches!(mode, Mode::Gallery | Mode::GalleryFolder);
    let map = match mode {
        Mode::Viewer => command,
        Mode::Gallery => gallery_command,
        Mode::GalleryFolder => folder_command,
        Mode::Crop => crop_command,
        Mode::Slideshow => slideshow_command,
    };
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed: true, repeat, modifiers, .. } => map(*key, *modifiers, *repeat),
                // The mouse's side buttons go back and forward, as in Explorer.
                Event::PointerButton { button: PointerButton::Extra1, pressed: true, .. } if gallery => Some(Cmd::Back),
                Event::PointerButton { button: PointerButton::Extra2, pressed: true, .. } if gallery => Some(Cmd::Forward),
                Event::Text(t) if t == "*" => Some(Cmd::Fit),
                Event::Copy if i.modifiers.shift => Some(Cmd::CopyImage),
                Event::Copy => Some(Cmd::Copy),
                _ => None,
            })
            .collect()
    })
}

/// A double click on `response`. egui counts a click soon after a double
/// click as a triple click, which is not a double click: a click that
/// selects a thumbnail followed at once by a double click on it would be
/// lost. Any click of a series after the first counts here.
pub fn double_clicked(response: &egui::Response) -> bool {
    response.double_clicked() || response.triple_clicked()
}

/// Mouse wheel notches, counted whole across frames: a precise wheel or a
/// touchpad sends fractions.
#[derive(Default)]
pub struct Wheel {
    browse: f32,
    zoom: f32,
}

/// Points of touchpad scrolling that count as one notch.
const POINTS_PER_NOTCH: f32 = 50.0;

impl Wheel {
    /// Whole notches of this frame: `(browse, zoom)`, positive away from
    /// the user (up). Ctrl turns browsing into zooming.
    pub fn read(&mut self, ctx: &egui::Context) -> (i32, i32) {
        ctx.input(|i| {
            for e in &i.events {
                if let Event::MouseWheel { unit, delta, modifiers, .. } = e {
                    let notches = match unit {
                        MouseWheelUnit::Point => delta.y / POINTS_PER_NOTCH,
                        MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
                    };
                    let acc = if modifiers.ctrl { &mut self.zoom } else { &mut self.browse };
                    // Turning the other way starts afresh.
                    if *acc * notches < 0.0 {
                        *acc = 0.0;
                    }
                    *acc += notches;
                }
            }
        });
        let take = |acc: &mut f32| {
            let whole = acc.trunc();
            *acc -= whole;
            whole as i32
        };
        (take(&mut self.browse), take(&mut self.zoom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const CTRL: Modifiers = Modifiers::CTRL;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const ALT: Modifiers = Modifiers::ALT;

    #[test]
    fn key_bindings() {
        assert_eq!(command(Key::Space, NONE, false), Some(Cmd::Next));
        assert_eq!(command(Key::Backspace, NONE, false), Some(Cmd::Prev));
        assert_eq!(command(Key::ArrowRight, CTRL, false), Some(Cmd::Next));
        assert_eq!(command(Key::ArrowRight, CTRL | ALT, false), Some(Cmd::RotateRight));
        assert_eq!(command(Key::ArrowLeft, NONE, false), Some(Cmd::Arrow(Arrow::Left)));
        // `+` is typed with Shift on the main keyboard.
        assert_eq!(command(Key::Plus, SHIFT, false), Some(Cmd::ZoomIn));
        assert_eq!(command(Key::Slash, NONE, false), Some(Cmd::Actual));
        assert_eq!(command(Key::Num1, NONE, false), Some(Cmd::Actual));
        assert_eq!(command(Key::Num2, NONE, false), Some(Cmd::Fit));
        assert_eq!(command(Key::Num3, NONE, false), Some(Cmd::Fill));
        assert_eq!(command(Key::Num4, NONE, false), Some(Cmd::Cover));
        assert_eq!(command(Key::Num1, CTRL, false), None);
        // Into the pinned folders, in the gallery too; not when held.
        assert_eq!(command(Key::Num1, ALT, false), Some(Cmd::MoveTo(1)));
        assert_eq!(command(Key::Num9, ALT | SHIFT, false), Some(Cmd::CopyTo(9)));
        assert_eq!(gallery_command(Key::Num5, ALT, false), Some(Cmd::MoveTo(5)));
        assert_eq!(command(Key::Num1, ALT, true), None);
        assert_eq!(command(Key::Num1, CTRL | ALT, false), None);
        assert_eq!(command(Key::Num0, ALT, false), None);
        assert_eq!(command(Key::N, ALT, false), Some(Cmd::MoveToNew));
        assert_eq!(command(Key::N, ALT | SHIFT, false), Some(Cmd::CopyToNew));
        assert_eq!(gallery_command(Key::N, ALT, false), Some(Cmd::MoveToNew));
        assert_eq!(command(Key::N, ALT, true), None);
        assert_eq!(command(Key::N, CTRL | ALT, false), None);
        assert_eq!(command(Key::OpenBracket, NONE, false), Some(Cmd::RotateLeft));
        assert_eq!(command(Key::H, NONE, false), Some(Cmd::FlipHorizontal));
        assert_eq!(command(Key::V, NONE, false), Some(Cmd::FlipVertical));
        assert_eq!(command(Key::V, CTRL, false), None);
        assert_eq!(command(Key::P, NONE, false), Some(Cmd::Pause));
        assert_eq!(command(Key::P, CTRL, false), Some(Cmd::Print));
        assert_eq!(command(Key::I, NONE, false), Some(Cmd::Info));
        assert_eq!(command(Key::F, SHIFT, false), Some(Cmd::Slideshow));
        assert_eq!(slideshow_command(Key::Space, NONE, false), Some(Cmd::Pause));
        assert_eq!(slideshow_command(Key::Space, NONE, true), None);
        assert_eq!(slideshow_command(Key::PageDown, NONE, false), Some(Cmd::Next));
        assert_eq!(command(Key::Slash, NONE, false), Some(Cmd::Actual));
        assert_eq!(gallery_command(Key::Slash, NONE, false), Some(Cmd::Find));
        assert_eq!(command(Key::F, CTRL, false), Some(Cmd::Find));
        assert_eq!(command(Key::Period, NONE, true), Some(Cmd::NextFrame));
        assert_eq!(command(Key::Comma, NONE, true), Some(Cmd::PrevFrame));
        assert_eq!(command(Key::F, NONE, false), Some(Cmd::FullScreen));
        assert_eq!(command(Key::F, CTRL | SHIFT, false), Some(Cmd::FullScreen));
        assert_eq!(command(Key::F, SHIFT, false), Some(Cmd::Slideshow));
        assert_eq!(command(Key::E, SHIFT, false), None);
        assert_eq!(command(Key::Delete, SHIFT, false), None);
        assert_eq!(command(Key::G, NONE, false), Some(Cmd::Gallery));
        assert_eq!(command(Key::Enter, NONE, false), Some(Cmd::Gallery));
        assert_eq!(command(Key::Enter, NONE, true), None);
        assert_eq!(command(Key::L, NONE, false), Some(Cmd::KeepZoom));
        assert_eq!(command(Key::L, NONE, true), None);
        assert_eq!(command(Key::S, NONE, false), Some(Cmd::Favorite));
        assert_eq!(command(Key::S, NONE, true), None);
        assert_eq!(command(Key::S, CTRL, false), Some(Cmd::Save));
        assert_eq!(command(Key::S, CTRL | SHIFT, false), Some(Cmd::SaveAs));
        assert_eq!(command(Key::S, CTRL, true), None);
        assert_eq!(command(Key::C, NONE, false), Some(Cmd::Crop));
        assert_eq!(command(Key::E, CTRL, false), Some(Cmd::Edit));
        assert_eq!(command(Key::C, SHIFT, false), None);
        assert_eq!(crop_command(Key::Enter, NONE, false), Some(Cmd::Save));
        assert_eq!(crop_command(Key::Enter, NONE, true), None);
        assert_eq!(crop_command(Key::Escape, NONE, false), Some(Cmd::Escape));
    }

    #[test]
    fn gallery_keys() {
        assert_eq!(gallery_command(Key::PageDown, NONE, true), Some(Cmd::PageDown));
        assert_eq!(gallery_command(Key::PageUp, NONE, false), Some(Cmd::PageUp));
        assert_eq!(gallery_command(Key::ArrowDown, NONE, false), Some(Cmd::Arrow(Arrow::Down)));
        assert_eq!(gallery_command(Key::Enter, NONE, false), Some(Cmd::Gallery));
        assert_eq!(gallery_command(Key::F, NONE, false), Some(Cmd::FullScreen));
        assert_eq!(gallery_command(Key::F, CTRL | SHIFT, false), Some(Cmd::WindowFullScreen));
        assert_eq!(command(Key::PageDown, NONE, false), Some(Cmd::Next));
        assert_eq!(gallery_command(Key::ArrowLeft, ALT, false), Some(Cmd::Back));
        assert_eq!(gallery_command(Key::ArrowRight, ALT, false), Some(Cmd::Forward));
        assert_eq!(gallery_command(Key::ArrowUp, ALT, false), Some(Cmd::Up));
        assert_eq!(gallery_command(Key::ArrowLeft, ALT, true), None);
        assert_eq!(gallery_command(Key::Backspace, NONE, false), Some(Cmd::Back));
        assert_eq!(gallery_command(Key::Backspace, NONE, true), None);
        assert_eq!(gallery_command(Key::ArrowLeft, CTRL | ALT, false), Some(Cmd::RotateLeft));
        assert_eq!(command(Key::ArrowLeft, ALT, false), None);
        assert_eq!(gallery_command(Key::ArrowDown, SHIFT, true), Some(Cmd::SelectTo(Move::Arrow(Arrow::Down))));
        assert_eq!(gallery_command(Key::End, SHIFT, false), Some(Cmd::SelectTo(Move::Last)));
        assert_eq!(gallery_command(Key::A, CTRL, false), Some(Cmd::SelectAll));
        assert_eq!(command(Key::A, CTRL, false), None);
    }

    #[test]
    fn held_keys_repeat_only_browsing_and_zoom() {
        assert_eq!(command(Key::PageDown, NONE, true), Some(Cmd::Next));
        assert_eq!(command(Key::Minus, NONE, true), Some(Cmd::ZoomOut));
        assert_eq!(command(Key::F, NONE, true), None);
        assert_eq!(command(Key::Delete, NONE, true), None);
        assert_eq!(command(Key::F2, NONE, false), Some(Cmd::Rename));
        assert_eq!(command(Key::F2, NONE, true), None);
        assert_eq!(command(Key::Z, Modifiers::CTRL, false), Some(Cmd::Undo));
        assert_eq!(command(Key::Z, NONE, false), None);
        assert_eq!(command(Key::Escape, NONE, true), None);
    }
}
