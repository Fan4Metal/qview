//! The histogram the information panel shows: how many pixels have each
//! level, 0 to 255, of red, green, blue and luma. Counted only while the
//! panel's Histogram section is open: by the decoder threads from the
//! pixels they make (`Histogram::of_bgra`, see `loader`), or for an image
//! decoded before it was opened from the file read again
//! (`Histogram::of_image`, on a thread of its own, see `App::update_histogram`).

use image::DynamicImage;

/// Levels of an image, of its pixels that are not fully transparent;
/// partly transparent ones count with their colour as it is, not as
/// blended with anything.
#[derive(Clone, Debug)]
pub struct Histogram {
    /// Pixels at each level of red, green, blue and luma (Rec. 709
    /// weights of the stored values, as cameras and editors show it).
    pub channels: [[u32; 256]; 4],
    /// Pixels counted.
    pub pixels: u64,
}

pub const RED: usize = 0;
pub const GREEN: usize = 1;
pub const BLUE: usize = 2;
pub const LUMA: usize = 3;

impl Default for Histogram {
    fn default() -> Self {
        Self { channels: [[0; 256]; 4], pixels: 0 }
    }
}

impl Histogram {
    /// Of premultiplied BGRA pixels, as the decoders make them.
    pub fn of_bgra(pixels: &[u8]) -> Self {
        in_parts(pixels.as_chunks::<4>().0, |&[b, g, r, a]| match a {
            255 => Some([r, g, b]),
            0 => None,
            a => Some([unpremultiply(r, a), unpremultiply(g, a), unpremultiply(b, a)]),
        })
    }

    /// Of a decoded image, in straight (not premultiplied) colours; more
    /// than 8 bits per channel are taken down to 8.
    pub fn of_image(img: &DynamicImage) -> Self {
        match img {
            DynamicImage::ImageRgb8(rgb) => in_parts(rgb.as_raw().as_chunks::<3>().0, |&p| Some(p)),
            DynamicImage::ImageRgba8(rgba) => in_parts(rgba.as_raw().as_chunks::<4>().0, straight),
            other => in_parts(other.to_rgba8().as_raw().as_chunks::<4>().0, straight),
        }
    }

    fn merge(mut self, other: Self) -> Self {
        for (a, b) in self.channels.iter_mut().flatten().zip(other.channels.iter().flatten()) {
            *a += b;
        }
        self.pixels += other.pixels;
        self
    }

    /// The mean level of `channel`, 0 to 255; None with no pixels.
    pub fn mean(&self, channel: usize) -> Option<f64> {
        let sum: u64 = self.channels[channel].iter().enumerate().map(|(level, &n)| level as u64 * n as u64).sum();
        (self.pixels > 0).then(|| sum as f64 / self.pixels as f64)
    }

    /// The level at the middle of `channel`'s pixels; None with no pixels.
    pub fn median(&self, channel: usize) -> Option<u8> {
        let half = self.pixels.div_ceil(2);
        let mut seen = 0;
        for (level, &n) in self.channels[channel].iter().enumerate() {
            seen += n as u64;
            if seen >= half && seen > 0 {
                return Some(level as u8);
            }
        }
        None
    }

    /// The share of the pixels, 0 to 1, of `n` of them.
    pub fn share(&self, n: u64) -> f64 {
        if self.pixels == 0 { 0.0 } else { n as f64 / self.pixels as f64 }
    }

    /// The count the graph's full height stands for, of `channels`: the
    /// largest count between the ends, but at most `PEAK` times the mean
    /// of the levels used between them. A few spikes (a flat area of one
    /// colour, a black border, a blown sky) would otherwise flatten the
    /// rest: a wallpaper with 31% of its pixels in one colour and the
    /// other levels under 3% showed them a few pixels high. Spikes are cut
    /// at the top then, as browsers' histograms do. The largest at all
    /// when the ends are all there is.
    pub fn scale(&self, channels: &[usize]) -> u32 {
        let (mut max, mut sum, mut used) = (0u32, 0u64, 0u64);
        for &c in channels {
            for &n in &self.channels[c][1..255] {
                max = max.max(n);
                sum += n as u64;
                used += (n > 0) as u64;
            }
        }
        if used == 0 {
            return channels.iter().flat_map(|&c| self.channels[c]).max().unwrap_or(0);
        }
        let cap = (sum as f64 / used as f64 * PEAK).ceil() as u32;
        max.min(cap.max(1))
    }
}

/// How many times the mean count of the used levels the graph's height
/// stands for at most (`Histogram::scale`); a smooth photo's peak is
/// usually within it.
const PEAK: f64 = 5.0;

/// The channels the graph shows (persisted by name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    Luma,
    Rgb,
    Red,
    Green,
    Blue,
}

impl Channels {
    pub const ALL: [Channels; 5] = [Channels::Luma, Channels::Rgb, Channels::Red, Channels::Green, Channels::Blue];

    /// The indices into `Histogram::channels`.
    pub fn indices(self) -> &'static [usize] {
        match self {
            Channels::Luma => &[LUMA],
            Channels::Rgb => &[RED, GREEN, BLUE],
            Channels::Red => &[RED],
            Channels::Green => &[GREEN],
            Channels::Blue => &[BLUE],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Channels::Luma => "luma",
            Channels::Rgb => "rgb",
            Channels::Red => "r",
            Channels::Green => "g",
            Channels::Blue => "b",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// Pixels per part counted on a thread of its own: below it a thread
/// would cost more than it saves.
const PART: usize = 1 << 21;
/// Threads at most for one image (the decoder threads run beside them).
const THREADS: usize = 4;

/// The histogram of `pixels`, whose colour `rgb` gives (None: not
/// counted), a large image in parts on threads.
fn in_parts<const N: usize>(pixels: &[[u8; N]], rgb: impl Fn(&[u8; N]) -> Option<[u8; 3]> + Sync) -> Histogram {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(THREADS);
    let parts = pixels.len().div_ceil(PART).clamp(1, threads);
    if parts == 1 {
        return count(pixels, &rgb);
    }
    let rgb = &rgb;
    std::thread::scope(|s| {
        let counting: Vec<_> =
            pixels.chunks(pixels.len().div_ceil(parts)).map(|part| s.spawn(move || count(part, rgb))).collect();
        counting.into_iter().map(|t| t.join().expect("histogram thread")).fold(Histogram::default(), Histogram::merge)
    })
}

/// Sets of counts `count` adds alternate pixels to.
const SETS: usize = 4;

/// The histogram of `pixels` on this thread. Neighbours are often alike
/// (a smooth sky, a flat background), and adding to the count just
/// written waits for that write: alternate pixels go to `SETS` sets of
/// counts, added up at the end (a gradient 65 ms for 14.7 MP with one
/// set). Counting the pixels with any channel at 0 or 255 took half as
/// long again, so clipping is told per channel, from the ends.
fn count<const N: usize>(pixels: &[[u8; N]], rgb: &impl Fn(&[u8; N]) -> Option<[u8; 3]>) -> Histogram {
    let mut sets = [[[0u32; 256]; 4]; SETS];
    let (groups, rest) = pixels.as_chunks::<SETS>();
    for group in groups {
        for (set, p) in sets.iter_mut().zip(group) {
            if let Some([r, g, b]) = rgb(p) {
                set[RED][r as usize] += 1;
                set[GREEN][g as usize] += 1;
                set[BLUE][b as usize] += 1;
                set[LUMA][luma(r, g, b) as usize] += 1;
            }
        }
    }
    for (set, p) in sets.iter_mut().zip(rest) {
        if let Some([r, g, b]) = rgb(p) {
            set[RED][r as usize] += 1;
            set[GREEN][g as usize] += 1;
            set[BLUE][b as usize] += 1;
            set[LUMA][luma(r, g, b) as usize] += 1;
        }
    }
    let mut channels = [[0u32; 256]; 4];
    for set in &sets {
        for (a, b) in channels.iter_mut().flatten().zip(set.iter().flatten()) {
            *a += b;
        }
    }
    let pixels = channels[LUMA].iter().map(|&n| n as u64).sum();
    Histogram { channels, pixels }
}

/// The colour of a straight RGBA pixel, unless it is fully transparent.
fn straight(&[r, g, b, a]: &[u8; 4]) -> Option<[u8; 3]> {
    (a != 0).then_some([r, g, b])
}

/// A premultiplied channel `c` of alpha `a` (not 0) back to its colour.
fn unpremultiply(c: u8, a: u8) -> u8 {
    ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
}

/// Rec. 709 luma of 8-bit values, with integer weights that add up to 256,
/// so white stays 255.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((54 * r as u32 + 183 * g as u32 + 19 * b as u32 + 128) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_levels() {
        // Opaque white, opaque pure red, fully transparent (not counted),
        // half-transparent grey 100 (premultiplied to 50).
        let bgra = [255, 255, 255, 255, 0, 0, 255, 255, 9, 9, 9, 0, 50, 50, 50, 128];
        let h = Histogram::of_bgra(&bgra);
        assert_eq!(h.pixels, 3);
        assert_eq!(h.channels[RED][255], 2);
        assert_eq!(h.channels[GREEN][0], 1);
        assert_eq!(h.channels[GREEN][100], 1);
        assert_eq!(h.channels[LUMA][255], 1);
        assert_eq!(h.channels[LUMA][luma(255, 0, 0) as usize], 1);
        assert_eq!(h.channels[LUMA][100], 1);
        assert_eq!(h.share(h.channels[RED][255] as u64), 2.0 / 3.0);
    }

    #[test]
    fn straight_and_premultiplied_agree() {
        let rgba = image::RgbaImage::from_fn(16, 16, |x, y| image::Rgba([x as u8 * 16, y as u8 * 16, 77, 255]));
        let mut bgra = Vec::new();
        for p in rgba.pixels() {
            bgra.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        let a = Histogram::of_image(&DynamicImage::ImageRgba8(rgba.clone()));
        let b = Histogram::of_bgra(&bgra);
        let c = Histogram::of_image(&DynamicImage::from(DynamicImage::ImageRgba8(rgba).into_rgb8()));
        assert_eq!(a.channels, b.channels);
        assert_eq!(a.channels, c.channels);
        assert_eq!(a.pixels, 256);
    }

    #[test]
    fn luma_weights() {
        assert_eq!(luma(255, 255, 255), 255);
        assert_eq!(luma(0, 0, 0), 0);
        assert_eq!(luma(128, 128, 128), 128);
        assert_eq!(luma(0, 255, 0), 182);
        assert_eq!(unpremultiply(50, 128), 100);
        assert_eq!(unpremultiply(255, 254), 255);
    }

    #[test]
    fn statistics() {
        let grey = image::RgbImage::from_fn(4, 1, |x, _| image::Rgb([[10, 20, 30, 200][x as usize]; 3]));
        let h = Histogram::of_image(&DynamicImage::ImageRgb8(grey));
        assert_eq!(h.mean(LUMA), Some(65.0));
        assert_eq!(h.median(LUMA), Some(20));
        assert_eq!(Histogram::default().median(LUMA), None);
        assert_eq!(h.share(1), 0.25);
        assert_eq!(h.scale(&[LUMA]), 1);
        assert_eq!(Channels::from_name("rgb"), Some(Channels::Rgb));
        assert_eq!(Channels::from_name("x"), None);
    }

    #[test]
    fn spikes_cut() {
        // One level holding most pixels, ten others a few each: the
        // height stands for five times their mean, not for the spike.
        let mut h = Histogram::default();
        h.channels[RED][40] = 1000;
        for level in 100..110 {
            h.channels[RED][level] = 10;
        }
        assert_eq!(h.scale(&[RED]), ((1000 + 100) as f64 / 11.0 * PEAK).ceil() as u32);
        // A smooth one keeps its peak.
        let mut smooth = Histogram::default();
        for level in 1..255 {
            smooth.channels[GREEN][level] = 100 + (level as u32).min(254 - level as u32);
        }
        assert_eq!(smooth.scale(&[GREEN]), 100 + 127);
        // Only the ends: the largest of them.
        let mut ends = Histogram::default();
        ends.channels[BLUE][255] = 7;
        assert_eq!(ends.scale(&[BLUE]), 7);
    }

    /// The histogram of `QVIEW_BENCH_FILE` as the decoder threads count
    /// it, on one thread and in parts, and what it adds to decoding (it is
    /// counted while the mip levels are made), printed with `--nocapture`.
    #[test]
    #[ignore]
    fn histogram_timings() {
        let path = std::path::PathBuf::from(std::env::var("QVIEW_BENCH_FILE").expect("QVIEW_BENCH_FILE"));
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
        let (pixels, _) = crate::loader::decode(&path, 16384, false).unwrap();
        let bgra = pixels.levels[0].as_chunks::<4>().0;
        let opaque = |&[b, g, r, a]: &[u8; 4]| match a {
            255 => Some([r, g, b]),
            0 => None,
            a => Some([unpremultiply(r, a), unpremultiply(g, a), unpremultiply(b, a)]),
        };
        for round in 0..5 {
            let t = std::time::Instant::now();
            let one = count(bgra, &opaque);
            let single = ms(t);
            let t = std::time::Instant::now();
            let parts = Histogram::of_bgra(&pixels.levels[0]);
            let threaded = ms(t);
            assert_eq!(one.channels, parts.channels);
            let t = std::time::Instant::now();
            let _ = crate::loader::decode(&path, 16384, false).unwrap();
            let plain = ms(t);
            let t = std::time::Instant::now();
            let (_, meta) = crate::loader::decode(&path, 16384, true).unwrap();
            let counting = ms(t);
            assert_eq!(meta.histogram.unwrap().channels, one.channels);
            let mp = bgra.len() as f64 / 1e6;
            println!(
                "round {round}: {mp:.1} MP, histogram on one thread {single:.1} ms, in parts {threaded:.1} ms; decode {plain:.1} ms, with the histogram {counting:.1} ms"
            );
        }
    }

    #[test]
    #[ignore]
    fn print_levels() {
        let path = std::path::PathBuf::from(std::env::var("QVIEW_BENCH_FILE").expect("QVIEW_BENCH_FILE"));
        let (_, meta) = crate::loader::decode(&path, 16384, true).unwrap();
        let h = meta.histogram.unwrap();
        let max = h.channels[..3].iter().flat_map(|c| &c[1..255]).max().unwrap();
        println!("pixels {}, largest {max}, scale {}", h.pixels, h.scale(&[RED, GREEN, BLUE]));
        for (c, name) in ["R", "G", "B", "L"].iter().enumerate() {
            let used: Vec<(usize, u32)> = h.channels[c].iter().copied().enumerate().filter(|&(_, n)| n > 0).collect();
            let mut top = used.clone();
            top.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
            let top: Vec<String> = top.iter().take(6).map(|&(l, n)| format!("{l}:{:.2}%", n as f64 * 100.0 / h.pixels as f64)).collect();
            let gaps = used.windows(2).filter(|w| w[1].0 - w[0].0 > 1).count();
            let mean = h.mean(c).unwrap();
            println!("{name}: mean {mean:.2}, {} levels from {} to {}, {gaps} gaps, top {}", used.len(), used[0].0, used.last().unwrap().0, top.join(" "));
        }
        // Luma is linear in the channels: its mean is theirs weighted, up
        // to the rounding of each pixel's luma.
        let [r, g, b] = [RED, GREEN, BLUE].map(|c| h.mean(c).unwrap());
        println!("weighted mean of R, G, B: {:.2}", (54.0 * r + 183.0 * g + 19.0 * b) / 256.0);
    }

    #[test]
    #[ignore]
    fn read_again_timings() {
        // What the panel waits for when the histogram is opened on an
        // image decoded without it: the file read again, then counted.
        let path = std::path::PathBuf::from(std::env::var("QVIEW_BENCH_FILE").expect("QVIEW_BENCH_FILE"));
        let _com = crate::win::com_init();
        for round in 0..3 {
            let t = std::time::Instant::now();
            let (img, _) = crate::loader::read(&path).unwrap();
            let read = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let h = Histogram::of_image(&img);
            let counted = t.elapsed().as_secs_f64() * 1e3;
            println!("round {round}: read {read:.1} ms, counted {counted:.1} ms ({} pixels)", h.pixels);
        }
    }

    #[test]
    fn parts_add_up() {
        // Larger than a part, with an odd pixel at the end.
        let n = PART * 2 + 3;
        let bgra: Vec<u8> = (0..n * 4).map(|i| (i * 7 % 251) as u8 | 1).collect();
        let whole = in_parts(bgra.as_chunks::<4>().0, |&[b, g, r, _]| Some([r, g, b]));
        let alone = count(bgra.as_chunks::<4>().0, &|&[b, g, r, _]: &[u8; 4]| Some([r, g, b]));
        assert_eq!(whole.channels, alone.channels);
        assert_eq!(whole.pixels, n as u64);
    }
}
