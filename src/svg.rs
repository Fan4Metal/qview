//! SVG and SVGZ, drawn by resvg.
//!
//! A drawing has no pixels of its own: its size (`width`/`height`, else
//! the `viewBox`) is taken as the image's, which zoom percentages, the
//! information panel, cropping and saving refer to, and the viewer's
//! texture is drawn larger (`view_scale`: large enough to fill the screen),
//! so that an icon of 24 pixels stays sharp when enlarged. Text needs the
//! system fonts, loaded once, and only for a drawing that has text.

use std::borrow::Cow;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use resvg::{tiny_skia, usvg};

/// The longest side a drawing is taken to have: one that says it is larger
/// is scaled down to it.
pub const MAX_SIDE: u32 = 16384;

/// The most pixels the viewer's texture of a drawing gets (`view_scale`).
const VIEW_PIXELS: f32 = 40e6;

/// Whether `path` is named as an SVG (an image in an archive too).
pub fn takes(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
}

/// Whether `bytes`, read from `path`, are an SVG: named so and text or
/// gzip, or otherwise text with an `<svg` element near its start.
pub fn is_svg(path: &Path, bytes: &[u8]) -> bool {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<');
    if takes(path) {
        return text || bytes.starts_with(&[0x1f, 0x8b]);
    }
    text && contains(&bytes[..bytes.len().min(4096)], b"<svg")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The fonts of this computer, for text (loaded on the first drawing that
/// has some: ~300 files of `C:\Windows\Fonts`, their headers only).
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let started = std::time::Instant::now();
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    log::debug!("{} system fonts loaded in {:.1} ms", fonts.len(), started.elapsed().as_secs_f64() * 1e3);
    Arc::new(fonts)
});

/// A parsed drawing and the size it is taken to have.
pub struct Svg {
    tree: usvg::Tree,
    pub width: u32,
    pub height: u32,
}

impl Svg {
    /// Parse `bytes` (SVG, or SVGZ compressed) read from `path`; images it
    /// links to are looked for beside the file. With `text`, the system
    /// fonts are given to the text it has; without, text is left out (the
    /// size does not depend on it).
    pub fn parse(path: &Path, bytes: &[u8], text: bool) -> Result<Self, String> {
        let data: Cow<[u8]> = if bytes.starts_with(&[0x1f, 0x8b]) {
            usvg::decompress_svgz(bytes).map_err(|e| e.to_string())?.into()
        } else {
            bytes.into()
        };
        let mut options = usvg::Options::default();
        if !crate::archive::inside(path) {
            options.resources_dir = path.parent().map(Path::to_path_buf);
        }
        if text && contains(&data, b"<text") {
            options.fontdb = FONTS.clone();
        }
        let tree = usvg::Tree::from_data(&data, &options).map_err(|e| e.to_string())?;
        let size = tree.size();
        let (w, h) = (size.width(), size.height());
        let fit = (MAX_SIDE as f32 / w.max(h)).min(1.0);
        let side = |v: f32| ((v * fit).round() as u32).clamp(1, MAX_SIDE);
        Ok(Self { width: side(w), height: side(h), tree })
    }

    /// The drawing at `width` x `height` pixels (its proportions are the
    /// caller's), premultiplied RGBA.
    pub fn render(&self, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| format!("cannot draw the image at {width}x{height} pixels"))?;
        let size = self.tree.size();
        let transform = tiny_skia::Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        Ok(pixmap.take())
    }

    /// The part `uv` of the drawing (0 to 1 across and down it) at `width`
    /// x `height` pixels, premultiplied RGBA: what an enlarged drawing
    /// shows of itself (`detail`).
    pub fn render_part(&self, uv: [f32; 4], width: u32, height: u32) -> Result<Vec<u8>, String> {
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| format!("cannot draw the image at {width}x{height} pixels"))?;
        let size = self.tree.size();
        let [u0, v0, u1, v1] = uv;
        let sx = width as f32 / ((u1 - u0) * size.width());
        let sy = height as f32 / ((v1 - v0) * size.height());
        let transform = tiny_skia::Transform::from_row(sx, 0.0, 0.0, sy, -u0 * size.width() * sx, -v0 * size.height() * sy);
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        Ok(pixmap.take())
    }

    /// `render` at `scale` times its size, and the size drawn.
    pub fn render_scaled(&self, scale: f32) -> Result<(Vec<u8>, u32, u32), String> {
        let side = |v: u32| ((v as f32 * scale).round() as u32).max(1);
        let (w, h) = (side(self.width), side(self.height));
        Ok((self.render(w, h)?, w, h))
    }
}

/// How much larger than its size the viewer draws a drawing of `width` x
/// `height`: so that it fills the screen either way up (Fill, or zoomed
/// in), never smaller than itself, within `max_side` and `VIEW_PIXELS`.
pub fn view_scale(width: u32, height: u32, max_side: usize) -> f32 {
    let (w, h) = (width as f32, height as f32);
    let (sw, sh) = crate::win::screen_size();
    let fill = |a: f32, b: f32| (sw / a).min(sh / b);
    let scale = fill(w, h).max(fill(h, w)).max(1.0);
    scale.min(max_side as f32 / w.max(h)).min((VIEW_PIXELS / (w * h)).sqrt())
}

/// Premultiplied RGBA as straight RGBA, in place.
pub fn unpremultiply(rgba: &mut [u8]) {
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &[u8] = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 4 2">
  <rect x="0" y="0" width="2" height="2" fill="#ff0000"/>
  <rect x="2" y="0" width="2" height="2" fill="#0000ff" fill-opacity="0.5"/>
</svg>"##;

    #[test]
    fn tells_svg_by_name_and_content() {
        let named = Path::new("a.SVG");
        assert!(takes(named) && takes(Path::new("b.svgz")) && !takes(Path::new("c.png")));
        assert!(is_svg(named, SQUARE));
        assert!(is_svg(Path::new("misnamed.png"), SQUARE));
        assert!(is_svg(Path::new("x.svgz"), &[0x1f, 0x8b, 8, 0]));
        assert!(!is_svg(Path::new("x.png"), b"\x89PNG\r\n\x1a\n"));
        assert!(!is_svg(Path::new("x.xml"), b"<?xml version=\"1.0\"?><html/>"));
        // A PNG named .svg is not text.
        assert!(!is_svg(named, b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn draws_at_any_size() {
        let svg = Svg::parse(Path::new("a.svg"), SQUARE, true).unwrap();
        assert_eq!((svg.width, svg.height), (40, 20));
        let (mut rgba, w, h) = svg.render_scaled(2.0).unwrap();
        assert_eq!((w, h, rgba.len()), (80, 40, 80 * 40 * 4));
        // Red on the left, half-transparent blue (premultiplied) on the right.
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        let right = (20 * 80 + 70) * 4;
        assert_eq!(&rgba[right..right + 4], &[0, 0, 128, 128]);
        unpremultiply(&mut rgba);
        assert_eq!(&rgba[right..right + 4], &[0, 0, 255, 128]);
        // The right half alone, at the scale of the whole at 2x: the
        // same pixels as the right half of that.
        let whole = svg.render_scaled(2.0).unwrap().0;
        let part = svg.render_part([0.5, 0.0, 1.0, 1.0], 40, 40).unwrap();
        for y in 0..40 {
            assert_eq!(&part[y * 160..(y + 1) * 160], &whole[y * 320 + 160..(y + 1) * 320], "row {y}");
        }
        // Too large a size is scaled down to `MAX_SIDE`, proportions kept.
        let huge = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="50000"/>"#;
        let svg = Svg::parse(Path::new("h.svg"), huge, false).unwrap();
        assert_eq!((svg.width, svg.height), (MAX_SIDE, MAX_SIDE / 2));
        assert!(Svg::parse(Path::new("bad.svg"), b"<svg", false).is_err());
    }

    /// Text is drawn with the system fonts (Arial is in every Windows).
    #[test]
    fn draws_text() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="30">
            <text x="2" y="24" font-family="Arial" font-size="24" fill="black">Ag</text></svg>"#;
        let inked = |text: bool| {
            let svg = Svg::parse(Path::new("t.svg"), svg, text).unwrap();
            svg.render(svg.width, svg.height).unwrap().as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
        };
        assert!(inked(true) > 50);
        assert_eq!(inked(false), 0);
    }

    /// Parsing and drawing the file named by `QVIEW_SVG_FILE`, printed
    /// with `--nocapture`.
    #[test]
    #[ignore]
    fn svg_file() {
        let path = std::path::PathBuf::from(std::env::var("QVIEW_SVG_FILE").expect("QVIEW_SVG_FILE"));
        let bytes = std::fs::read(&path).unwrap();
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
        let t = std::time::Instant::now();
        LazyLock::force(&FONTS);
        println!("system fonts: {} in {:.1} ms", FONTS.len(), ms(t));
        for round in 0..3 {
            let t = std::time::Instant::now();
            let svg = Svg::parse(&path, &bytes, false).unwrap();
            let size_only = ms(t);
            let t = std::time::Instant::now();
            let svg_text = Svg::parse(&path, &bytes, true).unwrap();
            let parsed = ms(t);
            let t = std::time::Instant::now();
            svg.render(svg.width, svg.height).unwrap();
            let own = ms(t);
            let scale = view_scale(svg.width, svg.height, 16384);
            let t = std::time::Instant::now();
            let (_, w, h) = svg_text.render_scaled(scale).unwrap();
            let view = ms(t);
            // Zoomed in 4x past that: the screen with the margins of
            // `detail` (a quarter more each way), from the middle.
            let (sw, sh) = crate::win::screen_size();
            let (pw, ph) = ((sw * 1.25) as u32, (sh * 1.25) as u32);
            let (du, dv) = (pw as f32 / (w as f32 * 4.0), ph as f32 / (h as f32 * 4.0));
            let t = std::time::Instant::now();
            svg_text.render_part([0.5 - du / 2.0, 0.5 - dv / 2.0, 0.5 + du / 2.0, 0.5 + dv / 2.0], pw, ph).unwrap();
            let part = ms(t);
            println!(
                "round {round}: {}x{}, parsed {size_only:.1} ms (with fonts {parsed:.1}), drawn at its size {own:.1} ms, at {w}x{h} {view:.1} ms, {pw}x{ph} of it at 4x {part:.1} ms",
                svg.width, svg.height
            );
        }
    }

    #[test]
    fn viewer_draws_large_enough_to_fill_the_screen() {
        let (sw, sh) = crate::win::screen_size();
        let scale = view_scale(24, 24, 16384);
        assert_eq!(scale, sw.min(sh) / 24.0);
        // Never smaller than itself, never larger than the GPU takes.
        assert_eq!(view_scale(10000, 100, 16384), 1.0);
        assert_eq!(view_scale(24, 24, 240), 10.0);
    }
}
