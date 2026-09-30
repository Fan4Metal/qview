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
    Actual,
    RotateLeft,
    RotateRight,
    FullScreen,
    /// Leaves full screen, or closes the viewer.
    Escape,
    Close,
    Delete,
    Copy,
    Open,
    OpenDefault,
    ShowInExplorer,
    Refresh,
    ToggleToolbar,
    ToggleStatusBar,
    Shortcuts,
    About,
    Associations,
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
        Key::Slash if plain => Actual,
        Key::OpenBracket if plain => RotateLeft,
        Key::CloseBracket if plain => RotateRight,
        Key::F if letter || (m.ctrl && m.shift && !m.alt) => FullScreen,
        Key::T if letter => ToggleToolbar,
        Key::B if letter => ToggleStatusBar,
        Key::E if m.shift && !m.ctrl && !m.alt => OpenDefault,
        Key::W if m.ctrl && !m.alt => Close,
        Key::O if m.ctrl && !m.alt => Open,
        Key::Delete if letter => Delete,
        Key::Escape => Escape,
        Key::F5 => Refresh,
        Key::F1 => Shortcuts,
        _ => return None,
    };
    let repeats = matches!(cmd, Next | Prev | First | Last | Arrow(_) | ZoomIn | ZoomOut);
    (!repeat || repeats).then_some(cmd)
}

/// Commands of this frame's key presses.
pub fn keys(ctx: &egui::Context) -> Vec<Cmd> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed: true, repeat, modifiers, .. } => command(*key, *modifiers, *repeat),
                Event::Text(t) if t == "*" => Some(Cmd::Fit),
                Event::Copy => Some(Cmd::Copy),
                _ => None,
            })
            .collect()
    })
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
        assert_eq!(command(Key::OpenBracket, NONE, false), Some(Cmd::RotateLeft));
        assert_eq!(command(Key::F, NONE, false), Some(Cmd::FullScreen));
        assert_eq!(command(Key::F, CTRL | SHIFT, false), Some(Cmd::FullScreen));
        assert_eq!(command(Key::F, SHIFT, false), None);
        assert_eq!(command(Key::E, SHIFT, false), Some(Cmd::OpenDefault));
        assert_eq!(command(Key::E, NONE, false), None);
        assert_eq!(command(Key::Delete, SHIFT, false), None);
    }

    #[test]
    fn held_keys_repeat_only_browsing_and_zoom() {
        assert_eq!(command(Key::PageDown, NONE, true), Some(Cmd::Next));
        assert_eq!(command(Key::Minus, NONE, true), Some(Cmd::ZoomOut));
        assert_eq!(command(Key::F, NONE, true), None);
        assert_eq!(command(Key::Delete, NONE, true), None);
        assert_eq!(command(Key::Escape, NONE, true), None);
    }
}
