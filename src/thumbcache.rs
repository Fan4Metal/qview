//! qview's own cache of the thumbnails it makes itself, on disk.
//!
//! Windows keeps the thumbnails of the files its codecs read; what qview
//! decodes for the gallery instead (the pages of comic books and other
//! archives, SVG, QOI, TGA, PNM, HEIC and AVIF where Windows lacks their
//! codecs, and everything when Windows' thumbnails are turned off) was
//! decoded again every time its folder was opened: a comic book of 200
//! pages took seconds. Those thumbnails go into [`dir`]
//! (`%LOCALAPPDATA%\qview\thumbs`, or `thumbs` beside a portable exe) as
//! one file each, JPEG for opaque ones and PNG for those with alpha, as
//! Windows stores its own.
//!
//! A file is named by an FNV-1a hash of its key, the path as listed (an
//! image in an archive by its virtual path) in lower case, and the side it
//! was made for, in 256 sub-folders by the hash's first byte. Its header
//! holds the key (so a hash collision is a miss, never another image's
//! pixels) and the size and date of the file or the archive it came from,
//! checked against the file on every hit: a file saved over misses, an
//! undo that puts the old bytes and date back hits the old entry. Writing
//! goes through a temporary file and a rename, so a reader never sees half
//! an entry and two running copies of qview (they share the folder) never
//! spoil each other's.
//!
//! The cache is kept within a limit ([`set_limit`]): a sweep on a thread,
//! some time after start-up and after every [`SWEEP_AFTER`] bytes written,
//! removes the oldest entries beyond it. An entry read is dated anew when
//! its date is more than a day old, a coarse least-recently-used order
//! without a write on every hit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use image::DynamicImage;

use crate::loader;
use crate::thumbs::Thumbnail;

/// The limit a fresh installation has, in megabytes.
pub const DEFAULT_LIMIT_MB: u32 = 512;
/// The limit the menu's slider ranges over, in megabytes.
pub const LIMIT_RANGE_MB: std::ops::RangeInclusive<u32> = 64..=8192;
/// A sweep runs after this many bytes written since the last one.
const SWEEP_AFTER: u64 = 64 << 20;
/// The first sweep of a run waits this long after [`start`].
const FIRST_SWEEP: Duration = Duration::from_secs(10);
/// A sweep removes the oldest entries down to this share of the limit.
const KEEP_SHARE: (u64, u64) = (3, 4);
/// An entry read is dated anew when older than this.
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 3600);
/// A temporary file older than this was left by a process that died.
const STALE_TMP: Duration = Duration::from_secs(3600);
/// Quality of the JPEG entries.
const JPEG_QUALITY: u8 = 90;
/// Extension of the entries.
const EXT: &str = "qvt";

const MAGIC: &[u8; 4] = b"QVTH";
/// Raised when the format of the entries changes: older ones miss.
const VERSION: u16 = 1;
const FLAG_PNG: u16 = 1;
/// The fixed part of the header, before the key.
const HEADER: usize = 4 + 2 + 2 + 4 + 4 + 4 + 4 + 4 + 8 + 8 + 8 + 8 + 2;

/// Off until [`configure`]: the tests never write into the real cache.
static ENABLED: AtomicBool = AtomicBool::new(false);
static LIMIT: AtomicU64 = AtomicU64::new((DEFAULT_LIMIT_MB as u64) << 20);
/// Bytes in the cache, [`UNKNOWN`] until a sweep counted them.
static TOTAL: AtomicU64 = AtomicU64::new(UNKNOWN);
const UNKNOWN: u64 = u64::MAX;
/// Bytes written since the last sweep.
static WRITTEN: AtomicU64 = AtomicU64::new(0);
static SWEEPING: AtomicBool = AtomicBool::new(false);

/// Where the entries are kept: `thumbs` beside a portable exe, otherwise
/// `%LOCALAPPDATA%\qview\thumbs`.
pub fn dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = crate::portable_dir().or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("qview")))?;
        Some(base.join("thumbs"))
    })
    .clone()
}

/// Apply the settings (`thumb_cache`, `thumb_cache_mb`).
pub fn configure(enabled: bool, limit_mb: u32) {
    ENABLED.store(enabled, Relaxed);
    set_limit(limit_mb);
}

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Relaxed);
}

/// The limit in bytes, from the megabytes of the slider.
pub fn set_limit(limit_mb: u32) {
    LIMIT.store((limit_mb.max(1) as u64) << 20, Relaxed);
}

pub fn limit() -> u64 {
    LIMIT.load(Relaxed)
}

/// Bytes in the cache, if a sweep has counted them yet.
pub fn total() -> Option<u64> {
    let t = TOTAL.load(Relaxed);
    (t != UNKNOWN).then_some(t)
}

/// Sweep [`FIRST_SWEEP`] after this call, on a thread (the count the menu
/// shows comes from it).
pub fn start() {
    std::thread::Builder::new()
        .name("thumbnail cache".into())
        .spawn(|| {
            std::thread::sleep(FIRST_SWEEP);
            sweep();
        })
        .ok();
}

/// Count the entries, on a thread (the menu asks when the count is not
/// known yet).
pub fn measure() {
    if total().is_none() && !SWEEPING.load(Relaxed) {
        std::thread::Builder::new().name("thumbnail cache sweep".into()).spawn(sweep).ok();
    }
}

/// `bytes` as megabytes for the menu: one decimal below 10 MB.
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1 << 20) as f64;
    if mb < 10.0 { format!("{mb:.1}") } else { format!("{mb:.0}") }
}

/// Remove every entry, on a thread; the count shows 0 at once.
pub fn clear() {
    TOTAL.store(0, Relaxed);
    let Some(dir) = dir() else { return };
    std::thread::Builder::new()
        .name("thumbnail cache clear".into())
        .spawn(move || {
            let started = Instant::now();
            let removed = clear_in(&dir);
            log::debug!("thumbnail cache cleared: {removed} files in {:.0} ms", started.elapsed().as_secs_f64() * 1e3);
        })
        .ok();
}

/// The cached thumbnail of `path` at `side`, if the cache holds one for
/// the file as it is now.
pub fn get(path: &Path, side: u32) -> Option<Thumbnail> {
    if !enabled() {
        return None;
    }
    let started = Instant::now();
    let found = get_in(&dir()?, path, side);
    log::trace!(
        "thumbnail cache {} {} at {side}, {:.1} ms",
        if found.is_some() { "hit" } else { "miss" },
        path.display(),
        started.elapsed().as_secs_f64() * 1e3
    );
    found
}

/// A thumbnail made, to be written once it has been handed over (the
/// encoding would hold the next thumbnail back otherwise).
pub struct Pending {
    path: PathBuf,
    side: u32,
    img: DynamicImage,
    width: u32,
    height: u32,
    file_size: u64,
    modified: u64,
    stamp: Stamp,
}

/// The size and date of the file an entry is checked against (the
/// archive of an image in one), taken before the image is decoded: if it
/// changes meanwhile, the thumbnail is of the old contents and is not
/// stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    size: u64,
    modified: u64,
}

/// The [`Stamp`] of `path` now.
pub fn stamp(path: &Path) -> Option<Stamp> {
    stat(&container(path)).map(|(size, modified, _)| Stamp { size, modified })
}

impl Pending {
    /// The thumbnail `img` (fitted into `side`) of `path`, an image of
    /// `width` x `height` in a file of `file_size` bytes last written at
    /// `modified`, decoded from the file `stamp` describes; `None` while
    /// the cache is off.
    #[allow(clippy::too_many_arguments)]
    pub fn new(path: &Path, side: u32, img: &DynamicImage, width: u32, height: u32, file_size: u64, modified: u64, stamp: Stamp) -> Option<Self> {
        enabled().then(|| Self { path: path.to_path_buf(), side, img: img.clone(), width, height, file_size, modified, stamp })
    }

    /// Encode and write it.
    pub fn write(self) {
        let Some(dir) = dir() else { return };
        let started = Instant::now();
        match put_in(&dir, &self.path, self.side, &self.img, (self.width, self.height), (self.file_size, self.modified), self.stamp) {
            Ok(bytes) => {
                log::trace!("thumbnail cache put {} at {}, {bytes} bytes, {:.1} ms", self.path.display(), self.side, started.elapsed().as_secs_f64() * 1e3);
                if TOTAL.load(Relaxed) != UNKNOWN {
                    TOTAL.fetch_add(bytes, Relaxed);
                }
                if WRITTEN.fetch_add(bytes, Relaxed) + bytes >= SWEEP_AFTER {
                    WRITTEN.store(0, Relaxed);
                    std::thread::Builder::new().name("thumbnail cache sweep".into()).spawn(sweep).ok();
                }
            }
            Err(e) => log::debug!("thumbnail cache cannot write {}: {e}", self.path.display()),
        }
    }
}

/// The entry's file for `path` at `side` under `root`.
fn entry_path(root: &Path, key: &str, side: u32) -> PathBuf {
    let hash = fnv1a(key.as_bytes());
    root.join(format!("{:02x}", hash >> 56)).join(format!("{hash:016x}-{side}.{EXT}"))
}

/// The key of `path`: the path as text, lower case, `\` between folders.
fn key_of(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// FNV-1a, 64 bits: the same in every build, unlike `DefaultHasher`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The file whose size and date tell whether an entry is current: the
/// archive of an image in one, else the file itself.
fn container(path: &Path) -> PathBuf {
    crate::archive::split(path).map_or_else(|| path.to_path_buf(), |(archive, _)| archive)
}

/// Size, last write time and creation time (FILETIME) of `path`.
fn stat(path: &Path) -> Option<(u64, u64, u64)> {
    use std::os::windows::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.last_write_time(), m.creation_time()))
}

/// What an entry's header says.
struct Header {
    flags: u16,
    side: u32,
    image_width: u32,
    image_height: u32,
    container_size: u64,
    container_modified: u64,
    file_size: u64,
    modified: u64,
    key: String,
    /// Where the payload starts.
    payload: usize,
}

fn read_header(bytes: &[u8]) -> Option<Header> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
    if u16_at(4) != VERSION {
        return None;
    }
    let key_len = u16_at(HEADER - 2) as usize;
    let payload = HEADER + key_len;
    if bytes.len() < payload {
        return None;
    }
    let key = std::str::from_utf8(&bytes[HEADER..payload]).ok()?.to_owned();
    Some(Header {
        flags: u16_at(6),
        side: u32_at(8),
        // 12, 16: the thumbnail's size, which the payload says itself.
        image_width: u32_at(20),
        image_height: u32_at(24),
        container_size: u64_at(28),
        container_modified: u64_at(36),
        file_size: u64_at(44),
        modified: u64_at(52),
        key,
        payload,
    })
}

#[allow(clippy::too_many_arguments)]
fn write_header(out: &mut Vec<u8>, flags: u16, side: u32, thumb: (u32, u32), image: (u32, u32), container: (u64, u64), file: (u64, u64), key: &str) {
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&side.to_le_bytes());
    out.extend_from_slice(&thumb.0.to_le_bytes());
    out.extend_from_slice(&thumb.1.to_le_bytes());
    out.extend_from_slice(&image.0.to_le_bytes());
    out.extend_from_slice(&image.1.to_le_bytes());
    out.extend_from_slice(&container.0.to_le_bytes());
    out.extend_from_slice(&container.1.to_le_bytes());
    out.extend_from_slice(&file.0.to_le_bytes());
    out.extend_from_slice(&file.1.to_le_bytes());
    let key = &key.as_bytes()[..key.len().min(u16::MAX as usize)];
    out.extend_from_slice(&(key.len() as u16).to_le_bytes());
    out.extend_from_slice(key);
}

/// [`get`] under `root`.
fn get_in(root: &Path, path: &Path, side: u32) -> Option<Thumbnail> {
    let key = key_of(path);
    let file = entry_path(root, &key, side);
    let bytes = std::fs::read(&file).ok()?;
    let header = read_header(&bytes)?;
    if header.key != key || header.side != side {
        return None;
    }
    let (size, modified, created) = stat(&container(path))?;
    if header.container_size != size || header.container_modified != modified {
        return None;
    }
    let format = if header.flags & FLAG_PNG != 0 { image::ImageFormat::Png } else { image::ImageFormat::Jpeg };
    let img = image::load_from_memory_with_format(&bytes[header.payload..], format).ok()?;
    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    touch(&file);
    Some(Thumbnail {
        pixels: loader::to_pixels(img),
        width: header.image_width,
        height: header.image_height,
        file_size: header.file_size,
        modified: header.modified,
        created: if crate::archive::inside(path) { 0 } else { created },
    })
}

/// Date `file` anew when it is more than [`TOUCH_AFTER`] old, so that the
/// sweep removes what has not been looked at for longest.
fn touch(file: &Path) {
    let Ok(f) = std::fs::File::options().write(true).open(file) else { return };
    let now = SystemTime::now();
    let old = f.metadata().and_then(|m| m.modified()).ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > TOUCH_AFTER);
    if old {
        f.set_modified(now).ok();
    }
}

/// [`Pending::write`] under `root`: the bytes written. `image` is the
/// image's size, `file` the file's size and date, `stamp` what the
/// container was when the image was read.
fn put_in(root: &Path, path: &Path, side: u32, img: &DynamicImage, image: (u32, u32), file: (u64, u64), stamp: Stamp) -> Result<u64, String> {
    let key = key_of(path);
    if self::stamp(path) != Some(stamp) {
        return Err("the file changed while it was decoded".into());
    }
    let mut out = Vec::with_capacity(HEADER + key.len() + (64 << 10));
    let flags = if img.color().has_alpha() { FLAG_PNG } else { 0 };
    write_header(&mut out, flags, side, (img.width(), img.height()), image, (stamp.size, stamp.modified), file, &key);
    if flags & FLAG_PNG != 0 {
        use image::codecs::png::{CompressionType, FilterType, PngEncoder};
        let rgba = img.to_rgba8();
        rgba.write_with_encoder(PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)).map_err(|e| e.to_string())?;
    } else {
        let rgb = img.to_rgb8();
        rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)).map_err(|e| e.to_string())?;
    }
    let file = entry_path(root, &key, side);
    let shard = file.parent().ok_or("no folder")?;
    std::fs::create_dir_all(shard).map_err(|e| e.to_string())?;
    let tmp = shard.join(format!("{}.{}.tmp", file.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, &out).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&tmp, &file) {
        std::fs::remove_file(&tmp).ok();
        return Err(e.to_string());
    }
    Ok(out.len() as u64)
}

/// Remove every entry under `root`: how many files went.
fn clear_in(root: &Path) -> usize {
    let mut removed = 0;
    for (file, _, _) in entries(root) {
        if std::fs::remove_file(file).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Count the cache and remove the oldest entries beyond the limit; one
/// sweep at a time.
fn sweep() {
    let Some(dir) = dir() else { return };
    if SWEEPING.swap(true, Relaxed) {
        return;
    }
    let started = Instant::now();
    let (total, removed) = sweep_in(&dir, limit());
    TOTAL.store(total, Relaxed);
    SWEEPING.store(false, Relaxed);
    log::debug!(
        "thumbnail cache swept: {:.1} MB kept, {removed} files removed, {:.0} ms",
        total as f64 / (1 << 20) as f64,
        started.elapsed().as_secs_f64() * 1e3
    );
}

/// [`sweep`] of `root` within `limit` bytes: the bytes kept and the files
/// removed.
fn sweep_in(root: &Path, limit: u64) -> (u64, usize) {
    let mut entries = entries(root);
    let mut total: u64 = entries.iter().map(|(_, len, _)| *len).sum();
    let mut removed = 0;
    if total > limit {
        entries.sort_by_key(|(_, _, modified)| *modified);
        let keep = limit / KEEP_SHARE.1 * KEEP_SHARE.0;
        for (file, len, _) in entries {
            if total <= keep {
                break;
            }
            if std::fs::remove_file(&file).is_ok() {
                total -= len;
                removed += 1;
            }
        }
    }
    (total, removed)
}

/// The entries under `root`: path, size, date. Temporary files left by a
/// process that died are removed on the way.
fn entries(root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut out = Vec::new();
    let Ok(shards) = std::fs::read_dir(root) else { return out };
    let now = SystemTime::now();
    for shard in shards.flatten() {
        let Ok(files) = std::fs::read_dir(shard.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            let Ok(meta) = f.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            match path.extension().and_then(|e| e.to_str()) {
                Some(e) if e == EXT => out.push((path, meta.len(), modified)),
                Some("tmp") if now.duration_since(modified).is_ok_and(|age| age > STALE_TMP) => {
                    std::fs::remove_file(&path).ok();
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::MetadataExt;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_thumbcache_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put_png(root: &Path, path: &Path, side: u32, img: &DynamicImage) -> u64 {
        let m = std::fs::metadata(path).unwrap();
        let stamp = stamp(path).unwrap();
        put_in(root, path, side, img, (img.width() * 4, img.height() * 4), (m.len(), m.last_write_time()), stamp).unwrap()
    }

    /// An opaque thumbnail comes back as JPEG, one with alpha as PNG with
    /// its alpha; the image's size and the file's are the header's.
    #[test]
    fn round_trip() {
        let dir = temp_dir("round");
        let (root, file) = (dir.join("cache"), dir.join("a.png"));
        image::RgbImage::new(8, 8).save(&file).unwrap();
        let opaque = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 32, image::Rgb([200, 30, 30])));
        let bytes = put_png(&root, &file, 96, &opaque);
        let t = get_in(&root, &file, 96).unwrap();
        assert_eq!((t.pixels.width, t.pixels.height), (64, 32));
        assert_eq!((t.width, t.height), (256, 128));
        assert_eq!(t.file_size, std::fs::metadata(&file).unwrap().len());
        assert!(t.created != 0);
        let p = &t.pixels.levels[0][(16 * 64 + 32) * 4..][..4];
        assert!(p[2] > 180 && p[1] < 60 && p[3] == 255, "{p:?}");
        assert!(bytes < 4000, "JPEG of {bytes} bytes");
        assert!(get_in(&root, &file, 256).is_none(), "another side");

        let mut rgba = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 255, 128]));
        rgba.put_pixel(0, 0, image::Rgba([255, 255, 255, 0]));
        put_png(&root, &file, 256, &DynamicImage::ImageRgba8(rgba));
        let t = get_in(&root, &file, 256).unwrap();
        let p = &t.pixels.levels[0][(8 * 16 + 8) * 4..][..4];
        // Premultiplied BGRA.
        assert_eq!(p, [128, 0, 0, 128]);
        assert_eq!(t.pixels.levels[0][3], 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file written since, or of another size, misses; so does an entry
    /// whose key is another path's (a hash collision) and a damaged one.
    #[test]
    fn stale_and_damaged_entries_miss() {
        let dir = temp_dir("stale");
        let (root, file) = (dir.join("cache"), dir.join("a.png"));
        image::RgbImage::new(8, 8).save(&file).unwrap();
        let img = DynamicImage::ImageRgb8(image::RgbImage::new(4, 4));
        put_png(&root, &file, 96, &img);
        assert!(get_in(&root, &file, 96).is_some());
        let entry = entry_path(&root, &key_of(&file), 96);

        let bytes = std::fs::read(&entry).unwrap();
        let other = dir.join("b.png");
        std::fs::copy(&file, &other).unwrap();
        let other_entry = entry_path(&root, &key_of(&other), 96);
        std::fs::create_dir_all(other_entry.parent().unwrap()).unwrap();
        std::fs::write(&other_entry, &bytes).unwrap();
        assert!(get_in(&root, &other, 96).is_none(), "another path's entry");

        std::fs::write(&entry, &bytes[..HEADER - 1]).unwrap();
        assert!(get_in(&root, &file, 96).is_none(), "cut header");
        std::fs::write(&entry, &bytes[..bytes.len() - 20]).unwrap();
        assert!(get_in(&root, &file, 96).is_none(), "cut payload");
        std::fs::write(&entry, b"").unwrap();
        assert!(get_in(&root, &file, 96).is_none(), "empty");

        std::fs::write(&entry, &bytes).unwrap();
        assert!(get_in(&root, &file, 96).is_some());
        std::thread::sleep(Duration::from_millis(20));
        image::RgbImage::new(8, 8).save(&file).unwrap();
        assert!(get_in(&root, &file, 96).is_none(), "written since");

        // Written while it was decoded: the old picture is not stored.
        let before = stamp(&file).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        image::RgbImage::new(9, 9).save(&file).unwrap();
        assert!(put_in(&root, &file, 96, &img, (9, 9), (0, 0), before).is_err());
        assert!(get_in(&root, &file, 96).is_none(), "decoded before a write");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Over the limit, the oldest entries go until three quarters of it
    /// are left; a stale temporary file goes too.
    #[test]
    fn sweep_removes_the_oldest() {
        let dir = temp_dir("sweep");
        let root = dir.join("cache");
        let file = dir.join("a.png");
        image::RgbImage::new(8, 8).save(&file).unwrap();
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([90, 120, 200])));
        let mut sizes = Vec::new();
        for i in 0..8u32 {
            let p = dir.join(format!("{i}.png"));
            std::fs::copy(&file, &p).unwrap();
            sizes.push(put_png(&root, &p, 96, &img));
            let entry = entry_path(&root, &key_of(&p), 96);
            let f = std::fs::File::options().write(true).open(&entry).unwrap();
            f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + i as u64 * 1000)).unwrap();
        }
        let tmp = root.join("00").join("x.tmp");
        std::fs::create_dir_all(tmp.parent().unwrap()).unwrap();
        std::fs::write(&tmp, b"x").unwrap();
        std::fs::File::options().write(true).open(&tmp).unwrap().set_modified(SystemTime::UNIX_EPOCH).unwrap();
        let total: u64 = sizes.iter().sum();
        let (kept, removed) = sweep_in(&root, total + 1);
        assert_eq!((kept, removed), (total, 0));
        assert!(!tmp.exists(), "stale temporary file");

        let limit = total / 2;
        let (kept, removed) = sweep_in(&root, limit);
        assert!(kept <= limit / 4 * 3 && removed > 0, "{kept} {removed}");
        // The newest stayed, the oldest went.
        assert!(get_in(&root, &dir.join("7.png"), 96).is_some());
        assert!(get_in(&root, &dir.join("0.png"), 96).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keys_and_names() {
        let a = key_of(Path::new(r"D:\Pics\Book.cbz/ch1/001.JPG"));
        assert_eq!(a, r"d:\pics\book.cbz\ch1\001.jpg");
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        let p = entry_path(Path::new(r"C:\cache"), "k", 256);
        assert_eq!(p.parent().unwrap().file_name().unwrap().len(), 2);
        assert!(p.to_string_lossy().ends_with("-256.qvt"));
    }
}
