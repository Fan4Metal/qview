//! Playing animated GIF and WebP images.
//!
//! The first frame is shown like any image (decoded ahead, `loader`).
//! While an animated image is on screen, a thread decodes its frames one
//! after another, ready for the GPU (`loader::Pixels`), into a channel a
//! few frames deep, so a long animation never sits in memory whole; the UI
//! takes each frame when the previous one's delay is over and uploads it
//! into the image's texture ([`crate::texture::Texture::replace`]). The
//! thread starts over for every loop and stops when the player is dropped
//! (the channel is closed) or after the file's number of loops.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::{LoopCount, Orientation};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat};

use crate::loader::{self, Pixels};

/// Frames decoded ahead of the one on screen.
const AHEAD: usize = 2;
/// Delays shorter than this are taken as `DEFAULT_DELAY`, as browsers do:
/// many GIFs say 0 and mean "as browsers show it".
const MIN_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);

/// A frame, ready for the texture, and how long it stays on screen.
pub struct Frame {
    pub pixels: Pixels,
    pub delay: Duration,
}

/// The animation of `path` being played.
pub struct Player {
    pub path: PathBuf,
    rx: mpsc::Receiver<Frame>,
    /// The frame received, waiting for its time.
    next: Option<Frame>,
    /// When the frame on screen ends; `None` before the first one.
    due: Option<Instant>,
}

impl Player {
    /// Start decoding the frames of `path` (larger than `max_side`
    /// shrunk); `ctx` is repainted when one is ready.
    pub fn start(path: PathBuf, max_side: usize, ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        let file = path.clone();
        let spawned = std::thread::Builder::new().name("animation".into()).spawn(move || {
            if let Err(e) = play(&file, max_side, &tx, &ctx) {
                log::warn!("cannot play {}: {e}", file.display());
            }
        });
        if let Err(e) = spawned {
            log::warn!("cannot start a thread for the animation: {e}");
        }
        Self { path, rx, next: None, due: None }
    }

    /// The frame to show now, if its time has come, and how long until the
    /// next one is due (`None`: when the decoder sends it, which repaints,
    /// or never, after the last loop).
    pub fn poll(&mut self, now: Instant) -> (Option<Pixels>, Option<Duration>) {
        if self.next.is_none() {
            self.next = self.rx.try_recv().ok();
        }
        if self.next.is_none() {
            return (None, None);
        }
        if let Some(due) = self.due
            && now < due
        {
            return (None, Some(due - now));
        }
        let frame = self.next.take().expect("checked above");
        // On time: the next frame follows this one's delay exactly, so the
        // animation does not drift; late by more than a frame: from now.
        self.due = Some(match self.due {
            Some(due) if now - due < frame.delay => due + frame.delay,
            _ => now + frame.delay,
        });
        (Some(frame.pixels), self.due.map(|d| d.saturating_duration_since(now)))
    }
}

/// The delay of a frame as shown (see `MIN_DELAY`).
fn shown_delay(delay: image::Delay) -> Duration {
    let delay = Duration::from(delay);
    if delay < MIN_DELAY { DEFAULT_DELAY } else { delay }
}

/// Whether the GIF or WebP `bytes` has more than one frame.
pub fn is_animated(bytes: &[u8], format: ImageFormat) -> bool {
    match format {
        ImageFormat::WebP => WebPDecoder::new(Cursor::new(bytes)).is_ok_and(|d| d.has_animation()),
        // GIF says nothing about its frames up front: a second one is
        // looked for (frames are small and quick to decode).
        ImageFormat::Gif => GifDecoder::new(Cursor::new(bytes)).is_ok_and(|d| d.into_frames().take(2).count() == 2),
        _ => false,
    }
}

/// The frames of the GIF or WebP `bytes`, the orientation to turn them by
/// and how many times to play them.
fn frames(bytes: &[u8], format: ImageFormat) -> image::ImageResult<(image::Frames<'_>, Orientation, LoopCount)> {
    match format {
        ImageFormat::WebP => {
            let mut d = WebPDecoder::new(Cursor::new(bytes))?;
            let orientation = d.orientation().unwrap_or(Orientation::NoTransforms);
            let loops = d.loop_count();
            Ok((d.into_frames(), orientation, loops))
        }
        _ => {
            let d = GifDecoder::new(Cursor::new(bytes))?;
            let loops = d.loop_count();
            Ok((d.into_frames(), Orientation::NoTransforms, loops))
        }
    }
}

/// Decode the frames of `path` into `tx`, loop after loop, until the
/// player is gone or the file's loops are played.
fn play(path: &Path, max_side: usize, tx: &mpsc::SyncSender<Frame>, ctx: &egui::Context) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    let mut loops_played = 0u32;
    loop {
        let (frames, orientation, loops) = frames(&bytes, format).map_err(|e| e.to_string())?;
        let mut count = 0;
        for frame in frames {
            // A broken frame ends the animation on the last good one.
            let frame = frame.map_err(|e| e.to_string())?;
            let delay = shown_delay(frame.delay());
            let mut img = DynamicImage::ImageRgba8(frame.into_buffer());
            img.apply_orientation(orientation);
            if img.width() as usize > max_side || img.height() as usize > max_side {
                img = img.thumbnail(max_side as u32, max_side as u32);
            }
            if tx.send(Frame { pixels: loader::to_pixels(img), delay }).is_err() {
                // The player is gone.
                return Ok(());
            }
            ctx.request_repaint();
            count += 1;
        }
        loops_played += 1;
        let done = match loops {
            LoopCount::Infinite => false,
            LoopCount::Finite(n) => loops_played >= n.get(),
        };
        if count < 2 || done {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Rgba, RgbaImage};

    /// A GIF of three 8x6 frames, red, green and blue, shown for 50, 10
    /// (too short: 100) and 70 ms, played `repeat` times.
    pub fn three_frames(repeat: Repeat) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(repeat).unwrap();
            let frames = [([255, 0, 0], 50), ([0, 255, 0], 10), ([0, 0, 255], 70)].map(|([r, g, b], ms)| {
                image::Frame::from_parts(RgbaImage::from_pixel(8, 6, Rgba([r, g, b, 255])), 0, 0, Delay::from_numer_denom_ms(ms, 1))
            });
            encoder.encode_frames(frames).unwrap();
        }
        bytes
    }

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_anim_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn detects_animation() {
        assert!(is_animated(&three_frames(Repeat::Infinite), ImageFormat::Gif));
        let mut still = Vec::new();
        RgbaImage::new(4, 4).write_to(&mut Cursor::new(&mut still), ImageFormat::Gif).unwrap();
        assert!(!is_animated(&still, ImageFormat::Gif));
        let mut webp = Vec::new();
        RgbaImage::new(4, 4).write_to(&mut Cursor::new(&mut webp), ImageFormat::WebP).unwrap();
        assert!(!is_animated(&webp, ImageFormat::WebP));
    }

    /// The frames of a GIF played twice come in order, as BGRA, with their
    /// delays, and then the channel closes.
    #[test]
    fn plays_the_frames_and_the_loops() {
        let path = temp_file("twice.gif", &three_frames(Repeat::Finite(2)));
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        let ctx = egui::Context::default();
        let file = path.clone();
        let thread = std::thread::spawn(move || play(&file, 16384, &tx, &ctx));
        let got: Vec<Frame> = rx.iter().collect();
        thread.join().unwrap().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        let summary: Vec<([u8; 4], u128)> = got
            .iter()
            .map(|f| (f.pixels.levels[0][..4].try_into().unwrap(), f.delay.as_millis()))
            .collect();
        let once = [([0, 0, 255, 255], 50), ([0, 255, 0, 255], 100), ([255, 0, 0, 255], 70)];
        assert_eq!(summary, [once, once].concat());
        assert_eq!((got[0].pixels.width, got[0].pixels.height), (8, 6));
    }

    /// The frames of the GIF or WebP named by `QVIEW_ANIM_FILE`: decoding
    /// times and delays, printed with `--nocapture`.
    #[test]
    #[ignore]
    fn file_frames() {
        let path = PathBuf::from(std::env::var("QVIEW_ANIM_FILE").expect("QVIEW_ANIM_FILE"));
        let (pixels, meta) = loader::decode(&path, 16384).unwrap();
        println!("{}x{} {} animated: {}", pixels.width, pixels.height, meta.format, meta.animated);
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        let ctx = egui::Context::default();
        let thread = std::thread::spawn(move || play(&path, 16384, &tx, &ctx));
        let started = Instant::now();
        let mut total = Duration::ZERO;
        // One loop at most of a looping animation.
        for (i, f) in rx.iter().enumerate().take(500) {
            total += f.delay;
            println!("frame {i}: {}x{}, {} ms, ready at {:.1} ms", f.pixels.width, f.pixels.height, f.delay.as_millis(), started.elapsed().as_secs_f64() * 1e3);
            if i > 0 && f.pixels.levels[0] == pixels.levels[0] {
                println!("(the first frame again)");
                break;
            }
        }
        println!("{:.0} ms of animation", total.as_secs_f64() * 1e3);
        drop(thread);
    }

    /// Frames are taken when the previous one's delay is over.
    #[test]
    fn frames_wait_for_their_time() {
        let (tx, rx) = mpsc::sync_channel(4);
        let mut player = Player { path: PathBuf::new(), rx, next: None, due: None };
        let frame = |ms| Frame { pixels: Pixels { width: 1, height: 1, levels: vec![vec![0; 4]] }, delay: Duration::from_millis(ms) };
        let t0 = Instant::now();
        assert!(matches!(player.poll(t0), (None, None)));
        tx.send(frame(50)).unwrap();
        tx.send(frame(70)).unwrap();
        // The first frame at once, the next in 50 ms.
        let (shown, wait) = player.poll(t0);
        assert!(shown.is_some());
        assert_eq!(wait, Some(Duration::from_millis(50)));
        let (shown, wait) = player.poll(t0 + Duration::from_millis(20));
        assert!(shown.is_none());
        assert_eq!(wait, Some(Duration::from_millis(30)));
        // A little late: the next one keeps to the schedule.
        let (shown, wait) = player.poll(t0 + Duration::from_millis(55));
        assert!(shown.is_some());
        assert_eq!(wait, Some(Duration::from_millis(65)));
        // Nothing more: nothing to wait for.
        drop(tx);
        assert!(matches!(player.poll(t0 + Duration::from_millis(200)), (None, None)));
    }
}
