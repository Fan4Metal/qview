//! Icons of the registered file types (`filetypes::FILE_TYPES`): a white
//! page with a folded corner, a small landscape (the app icon's motif) and,
//! across the page, a band in the format's colour with its extension in a
//! pixel font. Dependency-free, drawn at every size (not scaled), so that
//! the letters stay sharp at 16 x 16: they sit on whole pixels at a whole
//! scale, while the shapes are supersampled.

#![allow(dead_code)]

use crate::filetypes::FileType;

/// 3 or more columns by 5 rows, `#` for ink.
fn glyph(c: char) -> &'static [&'static str; 5] {
    match c {
        'A' => &[".#.", "#.#", "###", "#.#", "#.#"],
        'B' => &["##.", "#.#", "###", "#.#", "##."],
        'C' => &["###", "#..", "#..", "#..", "###"],
        'E' => &["###", "#..", "##.", "#..", "###"],
        'F' => &["###", "#..", "##.", "#..", "#.."],
        'G' => &["###", "#..", "#.#", "#.#", "###"],
        'I' => &["###", ".#.", ".#.", ".#.", "###"],
        'J' => &["..#", "..#", "..#", "#.#", "###"],
        'M' => &["#...#", "##.##", "#.#.#", "#...#", "#...#"],
        'N' => &["#..#", "##.#", "#.##", "#..#", "#..#"],
        'O' => &["###", "#.#", "#.#", "#.#", "###"],
        'P' => &["###", "#.#", "###", "#..", "#.."],
        'Q' => &["###", "#.#", "#.#", "##.", ".##"],
        'T' => &["###", ".#.", ".#.", ".#.", ".#."],
        'W' => &["#...#", "#...#", "#.#.#", "##.##", "#...#"],
        _ => &["###", "#.#", "#.#", "#.#", "###"],
    }
}

/// Width of `text` in font pixels (one column between letters).
fn text_width(text: &str) -> u32 {
    let letters: u32 = text.chars().map(|c| glyph(c)[0].len() as u32).sum();
    letters + text.chars().count().saturating_sub(1) as u32
}

/// Whether font pixel `(x, y)` of `text` is ink.
fn text_ink(text: &str, x: u32, y: u32) -> bool {
    let mut left = 0;
    for c in text.chars() {
        let rows = glyph(c);
        let w = rows[0].len() as u32;
        if x >= left && x < left + w {
            return rows[y as usize].as_bytes()[(x - left) as usize] == b'#';
        }
        left += w + 1;
    }
    false
}

type Rgba = [f32; 4];

fn rgb(c: [u8; 3]) -> Rgba {
    [c[0] as f32, c[1] as f32, c[2] as f32, 255.0]
}

fn shade(c: [u8; 3], f: f32) -> Rgba {
    [c[0] as f32 * f, c[1] as f32 * f, c[2] as f32 * f, 255.0]
}

/// Paint `top` over `under`.
fn over(under: Rgba, top: Rgba) -> Rgba {
    let a = top[3] / 255.0;
    let out_a = a + under[3] / 255.0 * (1.0 - a);
    if out_a <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0; 4];
    for i in 0..3 {
        out[i] = (top[i] * a + under[i] * (under[3] / 255.0) * (1.0 - a)) / out_a;
    }
    out[3] = out_a * 255.0;
    out
}

fn inside_rounded(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// Geometry of the icon at `n` x `n` pixels.
struct Layout {
    n: f32,
    /// The page, and the size of its folded corner.
    page: (f32, f32, f32, f32),
    fold: f32,
    /// Outline width.
    line: f32,
    /// The band, in whole pixels.
    band: (u32, u32, u32, u32),
    /// Font scale and the top left corner of the text, in whole pixels.
    scale: u32,
    text: String,
    text_at: (u32, u32),
    /// The landscape above the band, if there is room for it.
    picture: Option<(f32, f32, f32, f32)>,
}

fn layout(t: &FileType, n: u32) -> Layout {
    let nf = n as f32;
    // A label too wide for its size is drawn smaller; at the smallest sizes
    // it loses its last letters.
    let fits = |text: &str, scale: u32| text_width(text) * scale <= n - 2;
    let mut text: String = t.label.into();
    let mut scale = (n / 16).max(1);
    while scale > 1 && !fits(&text, scale) {
        scale -= 1;
    }
    while !fits(&text, scale) && text.len() > 3 {
        text.pop();
    }
    let pad = scale.max(1);
    let band_h = 5 * scale + 2 * pad;
    let band_y = ((nf * 0.62) as u32).saturating_sub(band_h / 2).min(n - band_h - 1);
    let text_w = text_width(&text) * scale;
    let band_w = (text_w + 2 * pad.max(2)).max((nf * 0.8) as u32).min(n);
    let band_x = (n - band_w) / 2;
    let page = (nf * 0.17, nf * 0.04, nf * 0.83, nf * 0.96);
    let line = (nf / 32.0).max(1.0);
    let picture = (n >= 32).then(|| {
        let m = nf * 0.1;
        (page.0 + m, page.1 + nf * 0.12, page.2 - m, band_y as f32 - nf * 0.05)
    });
    Layout {
        n: nf,
        page,
        fold: nf * 0.2,
        line,
        band: (band_x, band_y, band_x + band_w, band_y + band_h),
        scale,
        text_at: (band_x + (band_w - text_w) / 2, band_y + pad),
        text,
        picture,
    }
}

/// Colour of a sample point of the shapes (page, fold, picture, band).
fn shapes(t: &FileType, l: &Layout, x: f32, y: f32) -> Rgba {
    let mut c = [0.0; 4];
    let (x0, y0, x1, y1) = l.page;
    // The page without its top right corner, cut along the diagonal.
    let cut = x - (x1 - l.fold) + (y0 + l.fold - y);
    let on_page = inside_rounded(x, y, l.page, l.n * 0.03) && cut <= l.fold;
    if on_page {
        let inner = (x0 + l.line, y0 + l.line, x1 - l.line, y1 - l.line);
        let edge = !inside_rounded(x, y, inner, l.n * 0.02) || cut > l.fold - l.line * 1.4;
        c = if edge { rgb([0x80, 0x84, 0x88]) } else { rgb([0xfb, 0xfb, 0xfb]) };
        // The fold: a small triangle below the cut.
        let fx = x - (x1 - l.fold);
        let fy = y - y0;
        if fx >= 0.0 && fy <= l.fold && fx <= fy + 0.001 && cut <= l.fold {
            let border = fx < l.line || fy > l.fold - l.line;
            c = if border { rgb([0x80, 0x84, 0x88]) } else { rgb([0xd8, 0xdc, 0xe0]) };
        }
        if let Some((px0, py0, px1, py1)) = l.picture
            && x >= px0 && x <= px1 && y >= py0 && y <= py1
        {
            c = picture(x, y, (px0, py0, px1, py1));
        }
    }
    let (bx0, by0, bx1, by1) = l.band;
    let band = (bx0 as f32, by0 as f32, bx1 as f32, by1 as f32);
    if inside_rounded(x, y, band, l.n * 0.03) {
        let inner = (band.0 + l.line, band.1 + l.line, band.2 - l.line, band.3 - l.line);
        c = if inside_rounded(x, y, inner, l.n * 0.02) { rgb(t.band) } else { shade(t.band, 0.55) };
    }
    c
}

/// The app icon's sunset (`icon::photo`), in `r`.
fn picture(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> Rgba {
    let c = crate::icon::photo((x - x0) / (x1 - x0), (y - y0) / (y1 - y0));
    [c[0] * 255.0, c[1] * 255.0, c[2] * 255.0, 255.0]
}

/// Straight RGBA pixels of the icon of `t`, `n * n * 4` bytes.
pub fn rgba(t: &FileType, n: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let l = layout(t, n);
    let mut out = vec![0u8; (n * n * 4) as usize];
    for py in 0..n {
        for px in 0..n {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let c = shapes(
                        t,
                        &l,
                        px as f32 + (sx as f32 + 0.5) / SS as f32,
                        py as f32 + (sy as f32 + 0.5) / SS as f32,
                    );
                    // Premultiplied average.
                    for i in 0..3 {
                        acc[i] += c[i] * c[3] / 255.0;
                    }
                    acc[3] += c[3];
                }
            }
            let a = acc[3] / (SS * SS) as f32;
            let mut c = if a > 0.0 {
                [acc[0] / acc[3] * 255.0, acc[1] / acc[3] * 255.0, acc[2] / acc[3] * 255.0, a]
            } else {
                [0.0; 4]
            };
            // Letters on whole pixels, not supersampled.
            let (tx, ty) = l.text_at;
            if px >= tx && py >= ty {
                let (fx, fy) = ((px - tx) / l.scale, (py - ty) / l.scale);
                if fy < 5 && fx < text_width(&l.text) && text_ink(&l.text, fx, fy) {
                    c = over(c, rgb(t.ink));
                }
            }
            let i = ((py * n + px) * 4) as usize;
            for k in 0..4 {
                out[i + k] = c[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The icon of `t` as a multi-size `.ico`.
pub fn ico(t: &FileType, sizes: &[u32]) -> Vec<u8> {
    crate::icon::encode_ico(sizes, |n| rgba(t, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filetypes::FILE_TYPES;

    #[test]
    fn labels_fit_and_letters_are_known() {
        for t in FILE_TYPES {
            for n in [16, 20, 24, 32, 48, 256] {
                let l = layout(t, n);
                assert!(l.band.2 <= n && l.band.3 < n, "{} at {n}", t.label);
                assert!(l.text_at.0 + text_width(&l.text) * l.scale <= l.band.2, "{} at {n}", t.label);
            }
            for c in t.label.chars() {
                assert!(matches!(c, 'A' | 'B' | 'C' | 'E' | 'F' | 'G' | 'I' | 'J' | 'M' | 'N' | 'O' | 'P' | 'Q' | 'T' | 'W'), "{c}");
            }
        }
    }

    /// Writes all icons at all sizes on a grey and a white strip into the
    /// PNG named by `QVIEW_ICON_SHEET`, for a look at the design.
    #[test]
    #[ignore]
    fn preview_sheet() {
        let path = std::env::var("QVIEW_ICON_SHEET").expect("QVIEW_ICON_SHEET");
        let sizes = [16u32, 20, 24, 32, 48, 64, 256];
        let cell = 272u32;
        let (w, h) = (cell * sizes.len() as u32, cell * FILE_TYPES.len() as u32);
        let mut sheet = image::RgbaImage::from_fn(w, h, |x, _| {
            if x % cell < cell / 2 { image::Rgba([0xf0, 0xf0, 0xf0, 255]) } else { image::Rgba([0x2b, 0x2b, 0x2b, 255]) }
        });
        for (row, t) in FILE_TYPES.iter().enumerate() {
            for (col, &n) in sizes.iter().enumerate() {
                let icon = image::RgbaImage::from_raw(n, n, rgba(t, n)).unwrap();
                let (x, y) = (col as u32 * cell + 8, row as u32 * cell + 8);
                image::imageops::overlay(&mut sheet, &icon, x as i64, y as i64);
            }
        }
        sheet.save(path).unwrap();
    }

    #[test]
    fn letters_are_sharp_at_16() {
        let t = &FILE_TYPES[0];
        let px = rgba(t, 16);
        let l = layout(t, 16);
        // The top left pixel of "J" is empty, its top right one is ink.
        let at = |x: u32, y: u32| {
            let i = ((y * 16 + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2]]
        };
        let (tx, ty) = l.text_at;
        assert_eq!(at(tx + 2, ty), t.ink);
        assert_eq!(at(tx, ty), t.band);
    }
}
