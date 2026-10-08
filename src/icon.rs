//! Procedurally drawn application icon: two photos on top of each other,
//! the one behind tilted, the front one a sunset over mountains mirrored
//! in a lake. Dependency-free so that `build.rs` can include this file to
//! produce the `.ico` embedded in the executable, while the app uses the
//! same pixels for its window icon.

// build.rs and the app use different parts.
#![allow(dead_code)]

/// Sizes in every `.ico` of the executable and in the exported icon.
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Straight RGBA, channels 0..1.
type Rgba = [f32; 4];

fn hex(c: u32) -> Rgba {
    let ch = |s: u32| ((c >> s) & 0xff) as f32 / 255.0;
    [ch(16), ch(8), ch(0), 1.0]
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// Paint `top` over `under`.
fn over(under: Rgba, top: Rgba) -> Rgba {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0, 0.0, 0.0, a];
    for i in 0..3 {
        out[i] = (top[i] * top[3] + under[i] * under[3] * (1.0 - top[3])) / a;
    }
    out
}

fn with_alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], c[3] * a.clamp(0.0, 1.0)]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Colour at `t` along a gradient of `(position, colour)` stops.
fn gradient(stops: &[(f32, u32)], t: f32) -> Rgba {
    let mut c = hex(stops[0].1);
    for w in stops.windows(2) {
        let ((t0, _), (t1, c1)) = (w[0], w[1]);
        if t > t0 {
            c = mix(c, hex(c1), (t - t0) / (t1 - t0));
        }
    }
    c
}

/// Signed distance from `(x, y)` to a box of half size `h` centred at the
/// origin, with corners rounded by `r`: negative inside.
fn round_box((x, y): (f32, f32), h: (f32, f32), r: f32) -> f32 {
    let qx = x.abs() - h.0 + r;
    let qy = y.abs() - h.1 + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Height (from the top) of a range of peaks `(x, y, slope)` at `x`.
fn ridge(peaks: &[(f32, f32, f32)], x: f32) -> f32 {
    peaks.iter().map(|&(px, py, k)| py + (x - px).abs() * k).fold(f32::MAX, f32::min)
}

/// The sunset on the front photo. `(u, v)` spans a 3:2 picture, 0..1 from
/// its top left; straight RGBA. The file type icons show it too.
pub fn photo(u: f32, v: f32) -> [f32; 4] {
    // Picture units: 1.5 wide, 1 high, so circles stay round.
    let (x, y) = (u * 1.5, v);
    const HORIZON: f32 = 0.66;
    if y <= HORIZON {
        return above_water(x, y);
    }
    // The lake mirrors the scene, rippled, darker with depth, with a
    // glittering path of sunlight.
    let depth = (y - HORIZON) / (1.0 - HORIZON);
    let ripple = 0.012 * (y * 110.0).sin() * depth.sqrt();
    let m = above_water(x + ripple, 2.0 * HORIZON - y);
    let f = 0.7 - 0.25 * depth;
    let c = mix([m[0] * f, m[1] * f, m[2] * f, 1.0], hex(0x1b1840), 0.2 + 0.4 * depth);
    let width = 0.09 + 0.14 * depth;
    let path = (1.0 - (x - 0.95).abs() / width).max(0.0).sqrt();
    let glitter = 0.55 + 0.45 * (y * 130.0 + x * 9.0).sin();
    mix(c, hex(0xffd98c), path * glitter * (1.0 - 0.5 * depth))
}

fn above_water(x: f32, y: f32) -> Rgba {
    const SUN: (f32, f32, f32) = (0.95, 0.56, 0.17);
    const BACK: [(f32, f32, f32); 3] = [(0.40, 0.20, 0.95), (1.40, 0.30, 0.75), (-0.10, 0.36, 0.5)];
    const FRONT: [(f32, f32, f32); 2] = [(0.08, 0.44, 0.55), (1.52, 0.42, 0.65)];
    if y >= ridge(&FRONT, x) {
        return hex(0x231640);
    }
    let top = ridge(&BACK, x);
    if y >= top {
        // Snow on the peaks in the evening light; haze lightens the far
        // range towards its foot.
        let (_, py, _) = BACK.iter().copied().min_by(|a, b| {
            let h = |p: (f32, f32, f32)| p.1 + (x - p.0).abs() * p.2;
            h(*a).total_cmp(&h(*b))
        }).unwrap();
        if y - py < 0.09 + 0.02 * (x * 41.0).sin() && y - top < 0.07 {
            return mix(hex(0xf4c2cf), hex(0xc9869f), (y - py) / 0.11);
        }
        return mix(hex(0x4a2c6b), hex(0x7a3f78), smoothstep(0.35, 0.66, y));
    }
    let d = (x - SUN.0).hypot(y - SUN.1);
    if d <= SUN.2 {
        return mix(hex(0xfff6d0), hex(0xffd88a), (y - SUN.1 + SUN.2) / (2.0 * SUN.2));
    }
    let sky = gradient(&[(0.0, 0x1b2a63), (0.40, 0x55399a), (0.72, 0xd65c7c), (1.0, 0xffb257)], y / 0.66);
    let glow = (-(d - SUN.2) / 0.16).exp() * 0.65;
    mix(sky, hex(0xffcf85), glow)
}

/// The picture on the photo behind: a summer day with a hill.
fn day(u: f32, v: f32) -> Rgba {
    let (x, y) = (u * 1.5, v);
    if y >= ridge(&[(1.05, 0.50, 0.55)], x) {
        return mix(hex(0x3c9a4e), hex(0x2a7a3c), y);
    }
    if y >= ridge(&[(0.35, 0.40, 0.7)], x) {
        return hex(0x5bb15f);
    }
    gradient(&[(0.0, 0x2f74d0), (1.0, 0x9fd0fb)], y)
}

/// A photo print: a white border around a 3:2 picture.
struct Card {
    centre: (f32, f32),
    /// Half size of the whole print.
    half: (f32, f32),
    /// Clockwise, in radians.
    angle: f32,
    paper: u32,
    picture: fn(f32, f32) -> Rgba,
}

impl Card {
    /// `(x, y)` in the card's own frame, centred.
    fn local(&self, x: f32, y: f32) -> (f32, f32) {
        let (dx, dy) = (x - self.centre.0, y - self.centre.1);
        let (s, c) = self.angle.sin_cos();
        (dx * c + dy * s, -dx * s + dy * c)
    }

    /// Paint the card's shadow and the card over `under`; `px` is the size
    /// of a pixel.
    fn paint(&self, under: Rgba, x: f32, y: f32, px: f32) -> Rgba {
        let radius = 0.04;
        let p = self.local(x - 0.012, y - 0.022);
        let shadow = round_box(p, self.half, radius);
        let blur = 0.03 + px;
        let mut c = over(under, [0.0, 0.0, 0.0, 0.38 * (1.0 - smoothstep(-blur, blur, shadow))]);

        let p = self.local(x, y);
        let d = round_box(p, self.half, radius);
        if d > 0.0 {
            return c;
        }
        // A thin dark rim keeps the print apart from a white background.
        let rim = (0.6 * px).max(0.007);
        c = over(c, if d > -rim { mix(hex(self.paper), hex(0x303030), 0.55) } else { hex(self.paper) });
        let border = 0.045f32.max(1.2 * px);
        let inner = (self.half.0 - border, self.half.1 - border);
        if p.0.abs() <= inner.0 && p.1.abs() <= inner.1 {
            let u = (p.0 + inner.0) / (2.0 * inner.0);
            let v = (p.1 + inner.1) / (2.0 * inner.1);
            c = (self.picture)(u, v);
        }
        c
    }
}

/// Colour of a point in normalised coordinates (0..1, from the top left);
/// `px` is the size of a pixel in the same units.
fn sample(x: f32, y: f32, px: f32) -> Rgba {
    let back = Card { centre: (0.43, 0.41), half: (0.37, 0.265), angle: -0.21, paper: 0xe6e4df, picture: day };
    let front = Card { centre: (0.53, 0.59), half: (0.43, 0.30), angle: 0.0, paper: 0xfbfaf6, picture: photo };
    let c = back.paint([0.0; 4], x, y, px);
    front.paint(c, x, y, px)
}

/// Straight (non-premultiplied) RGBA pixels, `size * size * 4` bytes, row
/// major from the top. 5x5 supersampling gives smooth edges.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 5;
    let n = size as usize;
    let px = 1.0 / size as f32;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..size {
        for x in 0..size {
            // Premultiplied average.
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = (x as f32 + (sx as f32 + 0.5) / SS as f32) * px;
                    let fy = (y as f32 + (sy as f32 + 0.5) / SS as f32) * px;
                    let c = sample(fx, fy, px);
                    for i in 0..3 {
                        acc[i] += c[i] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let i = (y as usize * n + x as usize) * 4;
            if acc[3] > 0.0 {
                let a = acc[3] / (SS * SS) as f32;
                let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                out[i..i + 4].copy_from_slice(&[
                    to_u8(acc[0] / acc[3]),
                    to_u8(acc[1] / acc[3]),
                    to_u8(acc[2] / acc[3]),
                    to_u8(a),
                ]);
            }
        }
    }
    out
}

/// Encode the app icon as a multi-resolution `.ico`.
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    encode_ico(sizes, rgba)
}

/// The layers from this size up are stored as PNG, which Windows reads in
/// icons since Vista: the 256 px layer as a 32-bit BMP took 264 KB of each
/// icon's 306 (17 icons: 5.2 MB of the exe, 0.9 MB with PNG). The smaller
/// ones stay BMP, which Windows draws without decoding.
const PNG_FROM: u32 = 256;

/// Encode the images `render(size)` (straight RGBA) as a multi-resolution
/// `.ico`: 32-bit BMP entries, PNG from `PNG_FROM` up.
pub fn encode_ico(sizes: &[u32], render: impl Fn(u32) -> Vec<u8>) -> Vec<u8> {
    let images: Vec<Vec<u8>> =
        sizes.iter().map(|&s| if s >= PNG_FROM { png_entry(s, &render(s)) } else { bmp_entry(s, &render(s)) }).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type = icon
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// A whole PNG file of straight RGBA pixels, compressed hard (built once).
fn png_entry(size: u32, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(px).expect("PNG data");
    writer.finish().expect("PNG end");
    out
}

/// BITMAPINFOHEADER + bottom-up BGRA pixels + 1-bpp AND mask.
fn bmp_entry(size: u32, px: &[u8]) -> Vec<u8> {
    let n = size as usize;
    let mask_stride = n.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + n * n * 4 + mask_stride * n);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes()); // XOR + AND
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]); // compression .. colours important
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            out.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
        }
    }
    // Alpha channel carries transparency; an all-zero AND mask is correct.
    out.resize(out.len() + mask_stride * n, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape() {
        let px = rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        // Corners are transparent, the middle of the picture is opaque.
        assert_eq!(px[3], 0);
        let c = (18 * 32 + 16) * 4;
        assert_eq!(px[c + 3], 255);
        let ico = ico(&[16, 32]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
    }

    /// Writes the icon at all sizes on a light and a dark strip, and the
    /// small sizes enlarged 8 times, into the PNG named by
    /// `QVIEW_APP_ICON_SHEET`, for a look at the design.
    /// The 256 px layer is a PNG of the same pixels, the smaller ones BMP.
    #[test]
    fn large_layer_is_png() {
        let ico = encode_ico(&[16, 256], rgba);
        let entry = |k: usize| {
            let at = 6 + 16 * k;
            let size = u32::from_le_bytes(ico[at + 8..at + 12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(ico[at + 12..at + 16].try_into().unwrap()) as usize;
            (ico[at], &ico[offset..offset + size])
        };
        let (dim, small) = entry(0);
        assert_eq!((dim, &small[..4]), (16, &40u32.to_le_bytes()[..]));
        let (dim, large) = entry(1);
        assert_eq!(dim, 0);
        assert!(large.starts_with(b"\x89PNG"));
        let decoded = image::load_from_memory(large).unwrap().to_rgba8();
        assert_eq!(decoded.into_raw(), rgba(256));
        assert!(large.len() < 256 * 256 * 4 / 2, "{}", large.len());
    }

    #[test]
    #[ignore]
    fn preview_sheet() {
        let path = std::env::var("QVIEW_APP_ICON_SHEET").expect("QVIEW_APP_ICON_SHEET");
        let sizes = [16u32, 20, 24, 32, 48, 64, 256];
        let w = sizes.iter().map(|s| s + 16).sum::<u32>() + 16;
        let zoomed = [16u32, 20, 24, 32];
        let zw = zoomed.iter().map(|s| s * 8 + 16).sum::<u32>() + 16;
        let (w, h) = (w.max(zw), 2 * 272 + 32 * 8 + 32);
        let mut sheet = image::RgbaImage::from_fn(w, h, |_, y| {
            if !(272..544).contains(&y) { image::Rgba([0xf0, 0xf0, 0xf0, 255]) } else { image::Rgba([0x20, 0x20, 0x20, 255]) }
        });
        for row in 0..2 {
            let mut x = 16;
            for &n in &sizes {
                let icon = image::RgbaImage::from_raw(n, n, rgba(n)).unwrap();
                image::imageops::overlay(&mut sheet, &icon, x as i64, (row * 272 + 8) as i64);
                x += n + 16;
            }
        }
        let mut x = 16;
        for &n in &zoomed {
            let icon = image::RgbaImage::from_raw(n, n, rgba(n)).unwrap();
            let big = image::imageops::resize(&icon, n * 8, n * 8, image::imageops::FilterType::Nearest);
            image::imageops::overlay(&mut sheet, &big, x as i64, 560);
            x += n * 8 + 16;
        }
        sheet.save(path).unwrap();
    }
}
