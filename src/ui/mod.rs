//! The window around the image: menu bar, toolbar, status bar and dialogs,
//! all as `impl App` methods, in a dark look.

pub mod crop;
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
/// Fill of the buttons of destructive actions.
pub const DANGER: Color32 = Color32::from_rgb(200, 40, 40);
/// The star of a favourite image.
pub const STAR: Color32 = Color32::from_rgb(0xf2, 0xc0, 0x3c);

/// A five-pointed star of radius `r` around `c`, point up: filled with
/// `fill`, outlined with `outline`. egui fills only convex shapes, so the
/// fill is the inner pentagon and a triangle for each point.
pub fn paint_star(painter: &egui::Painter, c: egui::Pos2, r: f32, fill: Option<Color32>, outline: Color32) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let point = |k: usize, radius: f32| {
        let a = -FRAC_PI_2 + k as f32 * PI / 5.0;
        c + radius * egui::vec2(a.cos(), a.sin())
    };
    // The inner corners of a regular star.
    let inner = r * 0.382;
    let points: Vec<egui::Pos2> = (0..10).map(|k| point(k, if k % 2 == 0 { r } else { inner })).collect();
    if let Some(fill) = fill {
        let pentagon: Vec<egui::Pos2> = (0..5).map(|k| points[2 * k + 1]).collect();
        painter.add(egui::Shape::convex_polygon(pentagon, fill, egui::Stroke::NONE));
        for k in 0..5 {
            let tip = vec![points[(2 * k + 9) % 10], points[2 * k], points[2 * k + 1]];
            painter.add(egui::Shape::convex_polygon(tip, fill, egui::Stroke::NONE));
        }
    }
    painter.add(egui::Shape::closed_line(points, egui::Stroke::new(1.3, outline)));
}

/// The dark style of the whole window.
pub fn style(ctx: &egui::Context) {
    // The proportional font and its emoji fallbacks have no arrows (← → ↑
    // in the tooltips); the monospace one, loaded anyway, has them.
    let mut fonts = egui::FontDefinitions::default();
    if let Some(mono) = fonts.families.get(&egui::FontFamily::Monospace).and_then(|f| f.first()).cloned()
        && let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional)
    {
        proportional.push(mono);
    }
    ctx.set_fonts(fonts);
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
