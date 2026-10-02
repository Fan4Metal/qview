//! Keyboard and mouse wheel as commands, and the key bindings.
//!
//! Keys are matched by `Event::Key::key`, which egui-winit fills with the
//! logical key or, for a letter of a non-Latin layout, the physical one:
//! `F` works with the Russian layout too. Numpad `*` has no `egui::Key`
//! and arrives only as text. `Ctrl+C` arrives as `Event::Copy`.

use egui::{Event, Key, Modifiers, MouseWheelUnit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
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
    FullScreen,
    /// Full screen without leaving the gallery (Ctrl+Shift+F there; F
    /// shows the image in full screen).
    WindowFullScreen,
    /// Leaves full screen, or closes the viewer.
    Escape,
    Close,
    Delete,
    /// Rename the current file (F2).
    Rename,
    /// Undo the last rename (Ctrl+Z).
    Undo,
    Copy,
    Open,
    ShowInExplorer,
    Refresh,
    ToggleToolbar,
    ToggleStatusBar,
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
    /// A page of the gallery up or down (Page Up / Page Down there).
    PageUp,
    PageDown,
}

/// The command of a key press with `m` held, if any. `repeat` is set for
/// the presses of a held key, which only browse, pan and zoom.
pub fn command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    use Cmd::*;
    // Shift is part of typing `+` or `*`, so it is ignored for symbols.
    let plain = !m.ctrl && !m.alt;
    let letter = plain && !m.shift;
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
        Key::F if letter || (m.ctrl && m.shift && !m.alt) => FullScreen,
        Key::T if letter => ToggleToolbar,
        Key::B if letter => ToggleStatusBar,
        Key::L if letter => KeepZoom,
        Key::G if letter => Gallery,
        Key::Enter if plain && !m.shift => Gallery,
        Key::W if m.ctrl && !m.alt => Close,
        Key::O if m.ctrl && !m.alt => Open,
        Key::Delete if letter => Delete,
        Key::F2 if letter => Rename,
        Key::Z if m.ctrl && !m.alt && !m.shift => Undo,
        Key::Escape => Escape,
        Key::F5 => Refresh,
        Key::F1 => Shortcuts,
        _ => return None,
    };
    let repeats = matches!(cmd, Next | Prev | First | Last | Arrow(_) | ZoomIn | ZoomOut | PageUp | PageDown);
    (!repeat || repeats).then_some(cmd)
}

/// The command of a key press in the gallery: Page Up and Page Down move
/// a page there, the other keys are the viewer's (see [`command`]).
pub fn gallery_command(key: Key, m: Modifiers, repeat: bool) -> Option<Cmd> {
    match key {
        Key::PageUp if !m.ctrl && !m.alt => Some(Cmd::PageUp),
        Key::PageDown if !m.ctrl && !m.alt => Some(Cmd::PageDown),
        Key::F if m.ctrl && m.shift && !m.alt => Some(Cmd::WindowFullScreen),
        _ => command(key, m, repeat),
    }
}

/// Commands of this frame's key presses, in the gallery or the viewer.
pub fn keys(ctx: &egui::Context, gallery: bool) -> Vec<Cmd> {
    let map = if gallery { gallery_command } else { command };
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed: true, repeat, modifiers, .. } => map(*key, *modifiers, *repeat),
                Event::Text(t) if t == "*" => Some(Cmd::Fit),
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
        assert_eq!(command(Key::OpenBracket, NONE, false), Some(Cmd::RotateLeft));
        assert_eq!(command(Key::F, NONE, false), Some(Cmd::FullScreen));
        assert_eq!(command(Key::F, CTRL | SHIFT, false), Some(Cmd::FullScreen));
        assert_eq!(command(Key::F, SHIFT, false), None);
        assert_eq!(command(Key::E, SHIFT, false), None);
        assert_eq!(command(Key::Delete, SHIFT, false), None);
        assert_eq!(command(Key::G, NONE, false), Some(Cmd::Gallery));
        assert_eq!(command(Key::Enter, NONE, false), Some(Cmd::Gallery));
        assert_eq!(command(Key::Enter, NONE, true), None);
        assert_eq!(command(Key::L, NONE, false), Some(Cmd::KeepZoom));
        assert_eq!(command(Key::L, NONE, true), None);
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
