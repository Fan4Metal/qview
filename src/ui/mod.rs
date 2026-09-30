//! The window around the image: menu bar, toolbar, status bar and dialogs,
//! all as `impl App` methods, in a dark look.

mod dialogs;
mod menu;
mod status;
mod toolbar;

use egui::{Color32, Frame, Margin, TextureHandle, TextureOptions};

pub const MENU_BG: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(0x3e, 0x3e, 0x3e);
pub const STATUS_BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
pub const TEXT: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);

/// The dark style of the whole window.
pub fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals.panel_fill = MENU_BG;
        s.visuals.window_fill = Color32::from_rgb(0x2b, 0x2b, 0x2b);
        s.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
        s.visuals.widgets.inactive.fg_stroke.color = TEXT;
    });
}

/// The app icon as a texture `points` wide, rasterised at the display's
/// pixel density and kept in `cache`; rasterised again when that density
/// changes (the window moved to a monitor with another scale).
pub fn app_icon(ctx: &egui::Context, points: f32, cache: &mut Option<TextureHandle>) -> TextureHandle {
    let px = (points * ctx.pixels_per_point()).round() as usize;
    if cache.as_ref().is_none_or(|t| t.size() != [px, px]) {
        let image = egui::ColorImage::from_rgba_unmultiplied([px, px], &crate::icon::rgba(px as u32));
        *cache = Some(ctx.load_texture(format!("app_icon_{px}"), image, TextureOptions::LINEAR));
    }
    cache.clone().expect("set above")
}

/// Frame of a panel filled with `fill`.
fn panel_frame(fill: Color32, margin: Margin) -> Frame {
    Frame::NONE.fill(fill).inner_margin(margin)
}
