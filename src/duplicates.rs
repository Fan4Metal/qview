//! Duplicate images among those listed (View → Find Duplicates, in the
//! gallery): groups of similar pictures, each with its recommended original
//! first and the copies of it marked: the same picture in any file (a copy
//! of the file, another format, a re-saved, resized, turned or mirrored
//! copy). The rest of a group is only alike (a frame of the same burst, an
//! edited version), left for the user to judge.
//!
//! Each image gets a [`Print`] of its thumbnail (Windows' cached one,
//! `thumbs::make`, so no file is decoded whole): a perceptual hash, the
//! thumbnail averaged down to a `SIDE` x `SIDE` grid of grey whose bits say
//! which of two neighbours is brighter, along the rows and down the columns
//! (a "difference hash"), in the grid's eight turned and mirrored forms so
//! that a turned copy is as near as the original; the image's proportions;
//! and the mean colours of `ZONES` x `ZONES` zones. Two images are near
//! (`Print::near`) when in one form the bits, the proportions and the
//! colours are all within a `Level`. Images within `SIMILAR` are grouped,
//! transitively (`group`); within a group, those within `SAME` of one
//! another are sets of the same picture, an original and its copies.
//!
//! The original (`keeper`) has the most pixels, then the earliest date
//! modified (a converted or re-saved file is new), then the earliest date
//! created (a copy made in Explorer keeps the date modified), then the
//! largest file when both are of one format (a smaller file of another
//! format may be the better compressed source).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

/// The grid's side: 9 gives 8 differences along each row and column.
const SIDE: usize = 9;
/// Bits of a print along one direction (rows x differences per row).
#[cfg(test)]
const BITS: u32 = (SIDE * (SIDE - 1)) as u32;
/// The side of the grid of colours: the zones of the picture.
const ZONES: usize = 3;
/// The thumbnail side asked of Windows: its smallest cached size, enough
/// for the grids.
const THUMB_SIDE: u32 = 96;

/// How near two prints have to be, in one turned or mirrored form.
#[derive(Clone, Copy, Debug)]
struct Level {
    /// Bits of the difference hash, of 2 x `BITS` (144).
    bits: u32,
    /// Proportions: the gap of their logarithms, in thousandths (30 is 3%).
    aspect: u32,
    /// The colours: the largest difference of a zone's from the other's,
    /// the mean of its channels (0-255).
    colour: u32,
}

/// The same picture: a JPEG saved again or another format differs by 0-1
/// bits on the photos measured; the proportions within rounding, the
/// colours within a re-encoding's.
const SAME: Level = Level { bits: 6, aspect: 30, colour: 8 };
/// Alike: about 15% of the bits, the usual threshold for a difference hash
/// (unrelated photos measured 37-56 bits apart), the proportions within
/// 15%, no zone of other colours. The difference hash alone joined a thin
/// stripe with icons and icons of one layout in other colours: an image
/// with little detail has few bits set, and a centred object on a plain
/// ground gives the same pattern whatever its shape and colour.
const SIMILAR: Level = Level { bits: 22, aspect: 150, colour: 48 };

/// Where `(x, y)` of a `side` x `side` grid is in its form `k` of the
/// eight: the four turns, each as it is and mirrored.
fn form_at(k: usize, side: usize, x: usize, y: usize) -> (usize, usize) {
    let (x, y) = if k & 1 == 1 { (side - 1 - x, y) } else { (x, y) };
    match k >> 1 {
        0 => (x, y),
        1 => (side - 1 - y, x),
        2 => (side - 1 - x, side - 1 - y),
        _ => (y, side - 1 - x),
    }
}

/// Whether form `k` turns a quarter, swapping width and height.
fn turns_quarter(k: usize) -> bool {
    k >> 1 == 1 || k >> 1 == 3
}

/// What an image is compared by: the perceptual hash of its thumbnail in
/// its eight dihedral forms (the first the image as it is, the others its
/// turns and mirrors; each form the row differences and the column
/// differences), its proportions and the mean colours of its zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Print {
    forms: [[u128; 2]; 8],
    /// ln(width / height) in thousandths.
    aspect: i32,
    /// RGB of each zone, row by row.
    zones: [[u8; 3]; ZONES * ZONES],
}

impl Print {
    /// The forms of a `SIDE` x `SIDE` grid of grey, row by row.
    fn forms_of(grid: &[u8; SIDE * SIDE]) -> [[u128; 2]; 8] {
        let mut forms = [[0u128; 2]; 8];
        for (k, form) in forms.iter_mut().enumerate() {
            let at = |x: usize, y: usize| {
                let (x, y) = form_at(k, SIDE, x, y);
                grid[y * SIDE + x]
            };
            let (mut rows, mut columns) = (0u128, 0u128);
            for y in 0..SIDE {
                for x in 0..SIDE - 1 {
                    rows = (rows << 1) | u128::from(at(x, y) < at(x + 1, y));
                    columns = (columns << 1) | u128::from(at(y, x) < at(y, x + 1));
                }
            }
            *form = [rows, columns];
        }
        forms
    }

    /// Of a thumbnail: premultiplied BGRA, `width` x `height`, of an image
    /// `upright` (width, height) when known, else as the thumbnail. `None`
    /// for an empty image.
    pub fn of_bgra(bgra: &[u8], width: usize, height: usize, upright: Option<(u32, u32)>) -> Option<Print> {
        if width == 0 || height == 0 || bgra.len() < width * height * 4 {
            return None;
        }
        // The sums of B, G, R over a part of the thumbnail, `side` x `side`
        // parts covering it whole, each at least a pixel.
        let sums = |side: usize, j: usize| {
            let (gx, gy) = (j % side, j / side);
            let (x0, x1) = (gx * width / side, ((gx + 1) * width / side).max(gx * width / side + 1).min(width));
            let (y0, y1) = (gy * height / side, ((gy + 1) * height / side).max(gy * height / side + 1).min(height));
            let (mut sum, mut n) = ([0u64; 3], 0u64);
            for y in y0..y1 {
                for p in bgra[(y * width + x0) * 4..(y * width + x1) * 4].as_chunks::<4>().0 {
                    for c in 0..3 {
                        sum[c] += u64::from(p[c]);
                    }
                    n += 1;
                }
            }
            (sum, n.max(1))
        };
        let mut grid = [0u8; SIDE * SIDE];
        for (j, cell) in grid.iter_mut().enumerate() {
            let ([b, g, r], n) = sums(SIDE, j);
            // Rec. 601 luma.
            *cell = ((r * 77 + g * 150 + b * 29) / (n * 256)) as u8;
        }
        let mut zones = [[0u8; 3]; ZONES * ZONES];
        for (j, zone) in zones.iter_mut().enumerate() {
            let ([b, g, r], n) = sums(ZONES, j);
            *zone = [(r / n) as u8, (g / n) as u8, (b / n) as u8];
        }
        let (w, h) = upright.filter(|&(w, h)| w > 0 && h > 0).unwrap_or((width as u32, height as u32));
        let aspect = ((f64::from(w) / f64::from(h)).ln() * 1000.0).round() as i32;
        Some(Print { forms: Print::forms_of(&grid), aspect, zones })
    }

    /// Bits of the difference hash that differ between this image and form
    /// `k` of `other`.
    fn bits(&self, other: &Print, k: usize) -> u32 {
        let ([rows, columns], [r, c]) = (self.forms[0], other.forms[k]);
        (rows ^ r).count_ones() + (columns ^ c).count_ones()
    }

    /// Bits that differ between this image and the nearest form of
    /// `other`: 0 to 2 x `BITS` (`near` is what groups them).
    #[cfg(test)]
    pub fn distance(&self, other: &Print) -> u32 {
        (0..8).map(|k| self.bits(other, k)).min().unwrap_or(2 * BITS)
    }

    /// Whether `other`, in one of its forms, is as near as `level` says.
    fn near(&self, other: &Print, level: Level) -> bool {
        (0..8).any(|k| {
            // A quarter turn makes its proportions the inverse.
            let theirs = if turns_quarter(k) { -other.aspect } else { other.aspect };
            self.aspect.abs_diff(theirs) <= level.aspect
                && self.bits(other, k) <= level.bits
                && self.colour_gap(other, k) <= level.colour
        })
    }

    /// How far the zones' colours are from those of form `k` of `other`:
    /// the largest difference of a zone, the mean of its channels (a mean
    /// over the zones let an object of another colour in the middle pass).
    fn colour_gap(&self, other: &Print, k: usize) -> u32 {
        let mut gap = 0u32;
        for y in 0..ZONES {
            for x in 0..ZONES {
                let (ox, oy) = form_at(k, ZONES, x, y);
                let (a, b) = (self.zones[y * ZONES + x], other.zones[oy * ZONES + ox]);
                gap = gap.max((0..3).map(|c| u32::from(a[c].abs_diff(b[c]))).sum::<u32>() / 3);
            }
        }
        gap
    }
}

/// What is known of an image for grouping and for choosing the original.
#[derive(Clone, Debug)]
struct Entry {
    /// Its place in the list given.
    index: usize,
    print: Print,
    /// The image's pixels, when known.
    pixels: u64,
    file_size: u64,
    /// FILETIMEs, 0 when unknown.
    modified: u64,
    created: u64,
    /// The extension, lowercase: file sizes are compared within a format.
    format: String,
}

/// What an image is in its group. The number tells the sets of the same
/// picture in a group apart (1, 2…), 0 when it holds one set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The one of the same picture recommended to keep.
    Original(u16),
    /// The same picture as the original of its number.
    Copy(u16),
    /// Only alike: no other image of the group is the same picture.
    Alike,
}

/// Images found alike, as sets of the same picture: each set's original
/// first, then its copies; the set of the group's best image first, then
/// the others and the images only alike, in the order listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub members: Vec<PathBuf>,
    /// What each member is.
    pub kinds: Vec<Kind>,
}

/// Where each group starts in `files`, the groups' members as listed (some
/// may be filtered out or deleted): the gallery's sections.
pub fn starts(files: &[PathBuf], groups: &[Group]) -> Vec<usize> {
    let of: HashMap<&Path, usize> =
        groups.iter().enumerate().flat_map(|(k, g)| g.members.iter().map(move |m| (m.as_path(), k))).collect();
    let group = |i: usize| of.get(files[i].as_path());
    (0..files.len()).filter(|&i| i == 0 || group(i) != group(i - 1)).collect()
}

/// The search, on threads; dropping it stops them.
pub struct Finder {
    rx: mpsc::Receiver<Vec<Group>>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    total: usize,
}

impl Drop for Finder {
    fn drop(&mut self) {
        self.cancel.store(true, Relaxed);
    }
}

impl Finder {
    /// Look for duplicates among `files`, in their order; `ctx` is
    /// repainted when they are found.
    pub fn start(files: Vec<PathBuf>, ctx: egui::Context) -> Finder {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let total = files.len();
        let (stop, counted) = (cancel.clone(), done.clone());
        let spawned = std::thread::Builder::new().name("duplicates".into()).spawn(move || {
            let started = Instant::now();
            let groups = find(&files, &stop, &counted);
            if stop.load(Relaxed) {
                return;
            }
            log::debug!("{} groups of duplicates among {} images in {:.0} ms", groups.len(), files.len(), started.elapsed().as_secs_f64() * 1e3);
            if tx.send(groups).is_ok() {
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("cannot start the duplicates thread: {e}");
            cancel.store(true, Relaxed);
        }
        Finder { rx, cancel, done, total }
    }

    /// The groups, once found.
    pub fn poll(&self) -> Option<Vec<Group>> {
        self.rx.try_recv().ok()
    }

    /// Images looked at so far, and how many there are.
    pub fn progress(&self) -> (usize, usize) {
        (self.done.load(Relaxed).min(self.total), self.total)
    }
}

/// How many threads make the prints: half the processors, 2 to 4, as the
/// thumbnails.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(2, 4))
}

/// The groups among `files`; `done` counts the files looked at.
fn find(files: &[PathBuf], cancel: &AtomicBool, done: &AtomicUsize) -> Vec<Group> {
    let next = AtomicUsize::new(0);
    let entries = Mutex::new(Vec::with_capacity(files.len()));
    std::thread::scope(|s| {
        for _ in 0..workers().min(files.len().max(1)) {
            s.spawn(|| {
                // Windows' thumbnails are COM objects.
                let _com = crate::win::com_init();
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(path) = files.get(i) else { break };
                    if cancel.load(Relaxed) {
                        break;
                    }
                    if let Some(entry) = entry(path, i) {
                        entries.lock().unwrap().push(entry);
                    }
                    done.fetch_add(1, Relaxed);
                }
            });
        }
    });
    if cancel.load(Relaxed) {
        return Vec::new();
    }
    let mut entries = entries.into_inner().unwrap();
    entries.sort_by_key(|e| e.index);
    group(&entries, SIMILAR, cancel)
        .into_iter()
        .map(|members| {
            let mut group = Group { members: Vec::new(), kinds: Vec::new() };
            for (k, kind) in arrange(&entries, &members) {
                group.members.push(files[entries[k].index].clone());
                group.kinds.push(kind);
            }
            group
        })
        .collect()
}

/// A group's `members` (indices into `entries`, in the order listed) as
/// shown: sets of the same picture (within `SAME` of one another,
/// transitively), each its original (`keeper`) then its copies; the set
/// of the group's best image first, the others in the order of their
/// first members; an image only alike is a set of its own. The sets of
/// two or more are numbered when there are several (see `Kind`).
fn arrange(entries: &[Entry], members: &[usize]) -> Vec<(usize, Kind)> {
    // Sets by the same joining as the groups, within the group.
    let within: Vec<Entry> = members.iter().map(|&k| entries[k].clone()).collect();
    let sets = group(&within, SAME, &AtomicBool::new(false));
    let set_of = |j: usize| sets.iter().position(|s| s.contains(&j));
    let mut ordered: Vec<Vec<usize>> = Vec::new();
    for j in 0..members.len() {
        match set_of(j) {
            Some(s) if sets[s][0] == j => ordered.push(sets[s].clone()),
            Some(_) => {}
            None => ordered.push(vec![j]),
        }
    }
    let best = keeper(&within.iter().collect::<Vec<_>>());
    if let Some(first) = ordered.iter().position(|s| s.contains(&best)) {
        let set = ordered.remove(first);
        ordered.insert(0, set);
    }
    let numbered = ordered.iter().filter(|s| s.len() > 1).count() > 1;
    let mut number = 0u16;
    let mut out = Vec::with_capacity(members.len());
    for mut set in ordered {
        if set.len() == 1 {
            out.push((members[set[0]], Kind::Alike));
            continue;
        }
        number += 1;
        let n = if numbered { number } else { 0 };
        let original = set.remove(keeper(&set.iter().map(|&j| &within[j]).collect::<Vec<_>>()));
        out.push((members[original], Kind::Original(n)));
        out.extend(set.into_iter().map(|j| (members[j], Kind::Copy(n))));
    }
    out
}

/// The entry of an image from its thumbnail; `None` when it has none.
fn entry(path: &Path, index: usize) -> Option<Entry> {
    use std::os::windows::fs::MetadataExt;
    let thumb = match crate::thumbs::make(path, THUMB_SIDE) {
        Ok(t) => t,
        Err(e) => {
            log::debug!("no print of {}: {e}", path.display());
            return None;
        }
    };
    let p = &thumb.pixels;
    let print = Print::of_bgra(&p.levels[0], p.width as usize, p.height as usize, Some((thumb.width, thumb.height)))?;
    // An image in an archive has no file of its own: its dates are the
    // thumbnail's.
    let created = std::fs::metadata(path).map_or(thumb.modified, |m| m.creation_time());
    let format = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    Some(Entry {
        index,
        print,
        pixels: u64::from(thumb.width) * u64::from(thumb.height),
        file_size: thumb.file_size,
        modified: thumb.modified,
        created,
        format,
    })
}

/// Entries within `level` of one another, transitively (a chain
/// of near images is one group), as indices into `entries`, groups of two
/// or more in the order of their first members. Every pair is compared,
/// the rows shared among the processors (`group_timings`: 20 000 prints
/// took 4.9 s on one thread).
fn group(entries: &[Entry], level: Level, cancel: &AtomicBool) -> Vec<Vec<usize>> {
    let n = entries.len();
    // One thread for a group's own sets (`arrange`), or a small folder.
    let threads = if n < 512 { 1 } else { std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16) };
    // The pairs near enough; few, so gathered and joined afterwards.
    let next = AtomicUsize::new(0);
    let pairs = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut found = Vec::new();
                loop {
                    // Rows taken one by one: the first are the longest.
                    let a = next.fetch_add(1, Relaxed);
                    if a >= n || cancel.load(Relaxed) {
                        break;
                    }
                    let pa = &entries[a].print;
                    for (b, eb) in entries.iter().enumerate().skip(a + 1) {
                        if pa.near(&eb.print, level) {
                            found.push((a, b));
                        }
                    }
                }
                pairs.lock().unwrap().extend(found);
            });
        }
    });
    if cancel.load(Relaxed) {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut k: usize) -> usize {
        while parent[k] != k {
            parent[k] = parent[parent[k]];
            k = parent[k];
        }
        k
    }
    for (a, b) in pairs.into_inner().unwrap() {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent[rb.max(ra)] = rb.min(ra);
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut of_root: HashMap<usize, usize> = HashMap::new();
    for k in 0..n {
        let r = root(&mut parent, k);
        match of_root.get(&r) {
            Some(&g) => groups[g].push(k),
            None => {
                of_root.insert(r, groups.len());
                groups.push(vec![k]);
            }
        }
    }
    groups.retain(|g| g.len() > 1);
    groups
}

/// Which of a group's `members` to keep (see the module's description);
/// the first listed when nothing tells them apart.
fn keeper(members: &[&Entry]) -> usize {
    // Greater is better.
    let rank = |a: &Entry, b: &Entry| -> Ordering {
        let older = |x: u64, y: u64| if x == 0 || y == 0 { Ordering::Equal } else { y.cmp(&x) };
        let larger = if a.format == b.format { a.file_size.cmp(&b.file_size) } else { Ordering::Equal };
        a.pixels
            .cmp(&b.pixels)
            .then(older(a.modified, b.modified))
            .then(older(a.created, b.created))
            .then(larger)
            .then(b.index.cmp(&a.index))
    };
    (0..members.len()).reduce(|best, k| if rank(members[k], members[best]) == Ordering::Greater { k } else { best }).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A print of a grid alone: square, its zones black.
    fn print_of(grid: &[u8; SIDE * SIDE]) -> Print {
        Print { forms: Print::forms_of(grid), aspect: 0, zones: [[0; 3]; ZONES * ZONES] }
    }

    /// `SAME` with this many bits.
    fn bits(bits: u32) -> Level {
        Level { bits, ..SAME }
    }

    fn entry_of(index: usize, print: Print) -> Entry {
        Entry { index, print, pixels: 0, file_size: 0, modified: 0, created: 0, format: String::new() }
    }

    /// A gradient with a bright square: `w` x `h` premultiplied BGRA.
    fn picture(w: usize, h: usize, square: (usize, usize)) -> Vec<u8> {
        let mut bgra = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = (x * 200 / w) as u8;
                let bright = x >= square.0 && x < square.0 + w / 4 && y >= square.1 && y < square.1 + h / 4;
                let v = if bright { 255 } else { v };
                bgra[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        bgra
    }

    /// Blocks of 8x8 pixels of pseudo-random grey: `w` x `h` BGRA.
    fn blocks(w: usize, h: usize, seed: u32) -> Vec<u8> {
        let mut bgra = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let k = ((x / 8) as u32 * 7919 + (y / 8) as u32 * 104_729 + seed).wrapping_mul(2_654_435_761);
                let v = (k >> 24) as u8;
                bgra[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        bgra
    }

    #[test]
    fn prints_tell_pictures_apart() {
        let a = Print::of_bgra(&picture(64, 48, (8, 8)), 64, 48, None).unwrap();
        let same = Print::of_bgra(&picture(64, 48, (8, 8)), 64, 48, None).unwrap();
        // The same picture at another size: the grid averages it the same.
        let resized = Print::of_bgra(&picture(128, 96, (16, 16)), 128, 96, None).unwrap();
        // The square moved a little: alike, not the same.
        let moved = Print::of_bgra(&picture(64, 48, (11, 10)), 64, 48, None).unwrap();
        // Moved to the other corner: a zone of another colour, not alike.
        let away = Print::of_bgra(&picture(64, 48, (40, 30)), 64, 48, None).unwrap();
        let other = Print::of_bgra(&blocks(72, 72, 1), 72, 72, None).unwrap();
        assert_eq!(a.distance(&same), 0);
        assert!(a.distance(&resized) <= SAME.bits, "{}", a.distance(&resized));
        assert!(a.near(&resized, SAME));
        assert!(a.near(&moved, SIMILAR) && !a.near(&moved, SAME), "{}", a.distance(&moved));
        assert!(!a.near(&away, SIMILAR));
        assert!(a.distance(&other) > SIMILAR.bits, "{}", a.distance(&other));
        assert!(!a.near(&other, SIMILAR));
        let another = Print::of_bgra(&blocks(72, 72, 2), 72, 72, None).unwrap();
        assert!(other.distance(&another) > SIMILAR.bits, "{}", other.distance(&another));
        assert!(Print::of_bgra(&[], 0, 0, None).is_none());
    }

    #[test]
    fn turned_and_mirrored_copies_are_near() {
        // Not square: the turned copy's proportions are the inverse.
        let (w, h) = (64, 48);
        let original = picture(w, h, (4, 4));
        // Turned a quarter clockwise: (x, y) -> (h - 1 - y, x).
        let mut turned = vec![0u8; w * h * 4];
        let mut mirrored = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let p = &original[(y * w + x) * 4..][..4];
                let (tx, ty) = (h - 1 - y, x);
                turned[(ty * h + tx) * 4..][..4].copy_from_slice(p);
                mirrored[(y * w + (w - 1 - x)) * 4..][..4].copy_from_slice(p);
            }
        }
        let a = Print::of_bgra(&original, w, h, None).unwrap();
        let t = Print::of_bgra(&turned, h, w, None).unwrap();
        let m = Print::of_bgra(&mirrored, w, h, None).unwrap();
        assert_eq!(a.distance(&t), 0);
        assert_eq!(t.distance(&a), 0);
        assert_eq!(a.distance(&m), 0);
        assert!(a.near(&t, SAME) && t.near(&a, SAME) && a.near(&m, SAME));
        // The grid's own forms are distinct: a turn is not the identity.
        assert_ne!(a.forms[0], a.forms[2]);
    }

    /// What the difference hash alone took for alike: a stripe and a
    /// square, and one layout in other colours.
    #[test]
    fn proportions_and_colours_tell_apart() {
        let solid = |w: usize, h: usize, rgb: [u8; 3]| {
            let mut bgra = vec![0u8; w * h * 4];
            for p in bgra.as_chunks_mut::<4>().0 {
                *p = [rgb[2], rgb[1], rgb[0], 255];
            }
            bgra
        };
        // Plain images: no bit set, so the bits alone call them the same.
        let stripe = Print::of_bgra(&solid(1, 96, [255, 255, 255]), 1, 96, None).unwrap();
        let square = Print::of_bgra(&solid(96, 96, [255, 255, 255]), 96, 96, None).unwrap();
        let orange = Print::of_bgra(&solid(96, 96, [240, 150, 40]), 96, 96, None).unwrap();
        assert_eq!(stripe.distance(&square), 0);
        assert!(!stripe.near(&square, SIMILAR));
        assert_eq!(square.distance(&orange), 0);
        assert!(!square.near(&orange, SIMILAR));
        // A centred object on a dark ground, green and orange.
        let icon = |rgb: [u8; 3]| {
            let mut bgra = solid(96, 96, [20, 20, 20]);
            for y in 24..72 {
                for x in 24..72 {
                    bgra[(y * 96 + x) * 4..][..4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
                }
            }
            Print::of_bgra(&bgra, 96, 96, None).unwrap()
        };
        let (green, amber) = (icon([60, 190, 80]), icon([230, 140, 90]));
        assert!(green.distance(&amber) <= SAME.bits);
        assert!(!green.near(&amber, SIMILAR));
        // The image's own proportions count, not the thumbnail's (a
        // thumbnail may be padded).
        let padded = Print::of_bgra(&solid(96, 96, [255, 255, 255]), 96, 96, Some((1, 400))).unwrap();
        let long = Print::of_bgra(&solid(1, 96, [255, 255, 255]), 1, 96, Some((1, 400))).unwrap();
        assert!(padded.near(&long, SAME));
        assert!(!padded.near(&square, SIMILAR));
    }

    #[test]
    fn groups_are_transitive_and_ordered() {
        let mut grid = [0u8; SIDE * SIDE];
        for (k, g) in grid.iter_mut().enumerate() {
            *g = ((k * 37) % 251) as u8;
        }
        let a = print_of(&grid);
        // A print a few bits from `a`, and one a few bits from that.
        let mut near = a;
        near.forms[0][0] ^= 0b111;
        let mut further = near;
        further.forms[0][1] ^= 0b111;
        let mut far = a;
        far.forms[0][0] = !a.forms[0][0];
        let entries = vec![entry_of(0, far), entry_of(1, a), entry_of(3, further), entry_of(4, near)];
        let cancel = AtomicBool::new(false);
        // Within 3 bits: a-near and near-further chain into one group.
        assert_eq!(group(&entries, bits(3), &cancel), vec![vec![1, 2, 3]]);
        // Within 0: nothing.
        assert!(group(&entries, bits(0), &cancel).is_empty());
        cancel.store(true, Relaxed);
        assert!(group(&entries, bits(3), &cancel).is_empty());
    }

    #[test]
    fn sections_follow_the_groups() {
        let p = |s: &str| PathBuf::from(s);
        let group = |m: &[&str]| Group { members: m.iter().map(|s| p(s)).collect(), kinds: vec![Kind::Alike; m.len()] };
        let groups = vec![group(&["a", "b"]), group(&["c", "d", "e"])];
        assert_eq!(starts(&[p("a"), p("b"), p("c"), p("d"), p("e")], &groups), vec![0, 2]);
        // The first group's original filtered out, the second's copies too.
        assert_eq!(starts(&[p("b"), p("c")], &groups), vec![0, 1]);
        assert!(starts(&[], &groups).is_empty());
    }

    #[test]
    fn the_original_is_the_largest_then_the_oldest() {
        let print = print_of(&[0; SIDE * SIDE]);
        let at = |index, pixels, modified, created, file_size, format: &str| Entry {
            index,
            print,
            pixels,
            file_size,
            modified,
            created,
            format: format.into(),
        };
        // A JPEG made from an AVIF later: larger, newer, of the same size.
        let avif = at(0, 100, 10, 10, 44, "avif");
        let jpeg = at(1, 100, 20, 20, 302, "jpg");
        assert_eq!(keeper(&[&jpeg, &avif]), 1);
        // A downscaled copy, older or not: the most pixels win.
        let small = at(2, 50, 1, 1, 9, "jpg");
        assert_eq!(keeper(&[&small, &jpeg]), 1);
        // A copy made in Explorer: the date modified kept, created later.
        let copy = at(3, 100, 10, 30, 44, "avif");
        assert_eq!(keeper(&[&copy, &avif]), 1);
        // Within a format and of one date, the larger file.
        let better = at(4, 100, 10, 10, 60, "avif");
        assert_eq!(keeper(&[&avif, &better]), 1);
        // Nothing tells them apart: the first listed.
        assert_eq!(keeper(&[&avif, &avif.clone()]), 0);
        // Unknown dates do not count as the oldest: listed later, it loses.
        let undated = at(5, 100, 0, 0, 1, "png");
        assert_eq!(keeper(&[&undated, &avif]), 1);
    }

    /// A group of five alike, as found in a real folder: two pictures
    /// saved twice each and one only alike.
    #[test]
    fn a_group_is_sets_of_the_same_picture() {
        let mut grid = [0u8; SIDE * SIDE];
        for (k, g) in grid.iter_mut().enumerate() {
            *g = ((k * 37) % 251) as u8;
        }
        let base = print_of(&grid);
        // Bits flipped in the first form: `n` apart from `base`.
        let off = |n: u32| {
            let mut p = base;
            p.forms[0][0] ^= (1u128 << n) - 1;
            p
        };
        let (a, b) = (off(0), off(1));
        let (c, d) = (off(12), off(13));
        let e = off(20);
        let mut entries: Vec<Entry> = [c, a, e, d, b].into_iter().enumerate().map(|(i, p)| entry_of(i, p)).collect();
        // The best image of the group is b, of the set {a, b}; d beats c.
        entries[4].pixels = 10;
        entries[3].pixels = 5;
        let arranged = arrange(&entries, &[0, 1, 2, 3, 4]);
        assert_eq!(arranged, vec![(4, Kind::Original(1)), (1, Kind::Copy(1)), (3, Kind::Original(2)), (0, Kind::Copy(2)), (2, Kind::Alike)]);
    }

    #[test]
    fn copies_on_disk_are_found() {
        let dir = std::env::temp_dir().join(format!("qview_dup_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let encode = |img: image::RgbaImage| {
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            png
        };
        let a = encode(image::RgbaImage::from_fn(90, 60, |x, y| image::Rgba([(x * 2) as u8, (y * 4) as u8, ((x * y) % 256) as u8, 255])));
        let other = encode(image::RgbaImage::from_fn(90, 60, |x, y| image::Rgba([((x / 10 * 97 + y / 10 * 31) % 256) as u8, 40, 200, 255])));
        let files: Vec<PathBuf> = ["a.png", "b.png", "c.png"].iter().map(|n| dir.join(n)).collect();
        std::fs::write(&files[0], &a).unwrap();
        std::fs::write(&files[1], &other).unwrap();
        std::fs::write(&files[2], &a).unwrap();
        let (cancel, done) = (AtomicBool::new(false), AtomicUsize::new(0));
        let groups = find(&files, &cancel, &done);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].members.len(), 2);
        assert!(groups[0].members.contains(&files[0]) && groups[0].members.contains(&files[2]));
        // One set: no number.
        assert_eq!(groups[0].kinds, vec![Kind::Original(0), Kind::Copy(0)]);
        assert_eq!(done.load(Relaxed), 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `cargo test --release group_timings -- --ignored --nocapture`: every
    /// pair of N random prints compared.
    #[test]
    #[ignore]
    fn group_timings() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for n in [1000, 5000, 20000] {
            let entries: Vec<Entry> = (0..n)
                .map(|i| {
                    let mut grid = [0u8; SIDE * SIDE];
                    grid.iter_mut().for_each(|g| *g = next() as u8);
                    entry_of(i, print_of(&grid))
                })
                .collect();
            let t = Instant::now();
            let groups = group(&entries, SIMILAR, &AtomicBool::new(false));
            println!("{n} prints: {:.0} ms, {} groups", t.elapsed().as_secs_f64() * 1e3, groups.len());
        }
    }

    /// `$env:QVIEW_THUMB_DIR="<folder>"; cargo test --release duplicates_folder -- --ignored --nocapture`
    /// (`$env:QVIEW_DUP_DEEP=1` with the sub-folders):
    /// the groups among a folder's images, timed, the copies marked, and
    /// each image's nearest other image.
    #[test]
    #[ignore]
    fn duplicates_folder() {
        let dir = PathBuf::from(std::env::var("QVIEW_THUMB_DIR").expect("QVIEW_THUMB_DIR"));
        let (cancel, done) = (AtomicBool::new(false), AtomicUsize::new(0));
        // With QVIEW_DUP_DEEP, its sub-folders too.
        let files = if std::env::var_os("QVIEW_DUP_DEEP").is_some() {
            crate::folder::list_deep(&dir, Default::default(), true, &AtomicUsize::new(0), &cancel).unwrap()
        } else {
            crate::folder::list(&dir, None, Default::default()).unwrap()
        };
        let t = Instant::now();
        let groups = find(&files, &cancel, &done);
        println!("{} groups among {} images in {:.0} ms", groups.len(), files.len(), t.elapsed().as_secs_f64() * 1e3);
        for g in &groups {
            let names: Vec<String> = g
                .members
                .iter()
                .zip(&g.kinds)
                .map(|(p, kind)| {
                    let mark = match kind {
                        Kind::Original(n) => format!("original {n} "),
                        Kind::Copy(n) => format!("copy {n} "),
                        Kind::Alike => String::new(),
                    };
                    format!("{mark}{}", p.file_name().unwrap().to_string_lossy())
                })
                .collect();
            println!("  {}", names.join("  |  "));
        }
        let _com = crate::win::com_init();
        let prints: Vec<(String, Print)> =
            files.iter().filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), entry(p, 0)?.print))).collect();
        for (name, print) in &prints {
            let nearest = prints.iter().filter(|(n, _)| n != name).map(|(n, q)| (print.distance(q), n)).min();
            if let Some((d, n)) = nearest {
                println!("{d:4}  {name}  ~  {n}");
            }
        }
    }
}
