//! The window around the image: menu bar, toolbar, status bar and dialogs,
//! all as `impl App` methods, in a dark look.

mod dialogs;
pub mod gallery;
mod menu;
mod status;
mod toolbar;

use egui::{Color32, Frame, Margin, TextureHandle, TextureOptions};

pub const MENU_BG: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(0x3e, 0x3e, 0x3e);
pub const STATUS_BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
pub const TEXT: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
/// Fill of the buttons of destructive actions, as in disk_flashlight.
pub const DANGER: Color32 = Color32::from_rgb(200, 40, 40);

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

/// The app icon as a texture drawn `points` wide: the smallest of the
/// sizes rasterised at build time (`main::embedded_icon`) that is not
/// smaller than that on this display, minified with mipmaps. Kept in
/// `cache`, made again when the display's scale changes (the window moved
/// to a monitor with another scale).
pub fn app_icon(ctx: &egui::Context, points: f32, cache: &mut Option<TextureHandle>) -> TextureHandle {
    let px = (points * ctx.pixels_per_point()).round() as u32;
    let size = [64u32, 128, 256].into_iter().find(|&s| s >= px).unwrap_or(256);
    let n = size as usize;
    if cache.as_ref().is_none_or(|t| t.size() != [n, n]) {
        let image = egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::embedded_icon(size));
        let options = TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
        *cache = Some(ctx.load_texture(format!("app_icon_{size}"), image, options));
    }
    cache.clone().expect("set above")
}

/// Frame of a panel filled with `fill`.
fn panel_frame(fill: Color32, margin: Margin) -> Frame {
    Frame::NONE.fill(fill).inner_margin(margin)
}
