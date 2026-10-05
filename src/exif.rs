//! EXIF metadata (TIFF-structured) read and patched: the orientation, the
//! pixel dimensions and the link to the thumbnail, which change when an
//! image is turned or cropped. Everything else is kept byte for byte.
//!
//! Functions take the TIFF data, which starts with "II" or "MM": what
//! follows "Exif\0\0" in a JPEG's APP1 segment, and what the `image` crate's
//! decoders return as `exif_metadata`.

const ORIENTATION: u16 = 0x0112;
/// The pointer to the Exif sub-IFD, which holds the pixel dimensions.
const EXIF_IFD: u16 = 0x8769;
const PIXEL_X: u16 = 0xa002;
const PIXEL_Y: u16 = 0xa003;
const SHORT: u16 = 3;
const LONG: u16 = 4;

/// Byte order: big endian for "MM", little for "II".
fn big_endian(tiff: &[u8]) -> Option<bool> {
    match tiff.get(..4)? {
        [b'M', b'M', 0, 42] => Some(true),
        [b'I', b'I', 42, 0] => Some(false),
        _ => None,
    }
}

fn u16_at(d: &[u8], big: bool, i: usize) -> Option<u16> {
    let b: [u8; 2] = d.get(i..i + 2)?.try_into().ok()?;
    Some(if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
}

fn u32_at(d: &[u8], big: bool, i: usize) -> Option<u32> {
    let b: [u8; 4] = d.get(i..i + 4)?.try_into().ok()?;
    Some(if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
}

fn u16_bytes(big: bool, v: u16) -> [u8; 2] {
    if big { v.to_be_bytes() } else { v.to_le_bytes() }
}

fn u32_bytes(big: bool, v: u32) -> [u8; 4] {
    if big { v.to_be_bytes() } else { v.to_le_bytes() }
}

/// Offset of the IFD at `ifd` with its entries checked to be in `d`, and
/// their number.
fn ifd_entries(d: &[u8], big: bool, ifd: usize) -> Option<usize> {
    let count = u16_at(d, big, ifd)? as usize;
    // The entries and the offset of the next IFD after them.
    (ifd + 2 + 12 * count + 4 <= d.len()).then_some(count)
}

/// Offset of the 12-byte entry of `tag` in the IFD at `ifd`.
fn entry(d: &[u8], big: bool, ifd: usize, tag: u16) -> Option<usize> {
    let count = ifd_entries(d, big, ifd)?;
    (0..count).map(|k| ifd + 2 + 12 * k).find(|&e| u16_at(d, big, e) == Some(tag))
}

/// The entry at `e` as one value of `kind` (SHORT or LONG), held in the
/// entry itself.
fn set_entry(d: &mut [u8], big: bool, e: usize, kind: u16, value: u32) {
    d[e + 2..e + 4].copy_from_slice(&u16_bytes(big, kind));
    d[e + 4..e + 8].copy_from_slice(&u32_bytes(big, 1));
    let field = if kind == SHORT {
        let [a, b] = u16_bytes(big, value as u16);
        [a, b, 0, 0]
    } else {
        u32_bytes(big, value)
    };
    d[e + 8..e + 12].copy_from_slice(&field);
}

/// The orientation, 1 to 8, if the first IFD has one.
pub fn orientation(tiff: &[u8]) -> Option<u8> {
    let big = big_endian(tiff)?;
    let ifd = u32_at(tiff, big, 4)? as usize;
    let e = entry(tiff, big, ifd, ORIENTATION)?;
    // A SHORT, in the first two bytes of the value field.
    let value = u16_at(tiff, big, e + 8)?;
    (1..=8).contains(&value).then_some(value as u8)
}

/// `tiff` with the orientation `value`. A first IFD without one is copied
/// to the end with the tag added and the header pointed at the copy: every
/// offset in the data stays valid, since nothing moves. None if `tiff` is
/// not readable.
pub fn with_orientation(tiff: &[u8], value: u8) -> Option<Vec<u8>> {
    let big = big_endian(tiff)?;
    let ifd = u32_at(tiff, big, 4)? as usize;
    let count = ifd_entries(tiff, big, ifd)?;
    let mut out = tiff.to_vec();
    if let Some(e) = entry(tiff, big, ifd, ORIENTATION) {
        set_entry(&mut out, big, e, SHORT, value as u32);
        return Some(out);
    }
    let mut entries: Vec<[u8; 12]> =
        (0..count).map(|k| tiff[ifd + 2 + 12 * k..ifd + 14 + 12 * k].try_into().expect("12 bytes")).collect();
    let mut added = [0u8; 12];
    added[..2].copy_from_slice(&u16_bytes(big, ORIENTATION));
    set_entry(&mut added, big, 0, SHORT, value as u32);
    // Entries are sorted by tag.
    let at = entries.iter().position(|e| u16_at(e, big, 0).is_some_and(|t| t > ORIENTATION)).unwrap_or(count);
    entries.insert(at, added);
    let next = u32_at(tiff, big, ifd + 2 + 12 * count)?;
    // IFDs start on a word boundary.
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let moved = u32::try_from(out.len()).ok()?;
    out.extend(u16_bytes(big, entries.len() as u16));
    out.extend(entries.iter().flatten());
    out.extend(u32_bytes(big, next));
    out[4..8].copy_from_slice(&u32_bytes(big, moved));
    Some(out)
}

/// The smallest EXIF with an orientation: for a JPEG that has none.
pub fn minimal(value: u8) -> Vec<u8> {
    let mut out = b"II\x2a\x00\x08\x00\x00\x00\x01\x00".to_vec();
    let mut e = [0u8; 12];
    e[..2].copy_from_slice(&ORIENTATION.to_le_bytes());
    set_entry(&mut e, false, 0, SHORT, value as u32);
    out.extend(e);
    out.extend([0, 0, 0, 0]);
    out
}

/// `tiff` for an image saved anew at `width` x `height`, upright: the
/// orientation 1, the pixel dimensions of the new image, and no link to the
/// thumbnail, which shows the old picture. None if it is not readable.
pub fn for_new_pixels(tiff: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let big = big_endian(tiff)?;
    let ifd = u32_at(tiff, big, 4)? as usize;
    let count = ifd_entries(tiff, big, ifd)?;
    let mut out = tiff.to_vec();
    if let Some(e) = entry(tiff, big, ifd, ORIENTATION) {
        set_entry(&mut out, big, e, SHORT, 1);
    }
    let next = ifd + 2 + 12 * count;
    out[next..next + 4].copy_from_slice(&[0; 4]);
    if let Some(e) = entry(tiff, big, ifd, EXIF_IFD)
        && let Some(sub) = u32_at(tiff, big, e + 8)
    {
        for (tag, value) in [(PIXEL_X, width), (PIXEL_Y, height)] {
            if let Some(e) = entry(tiff, big, sub as usize, tag) {
                // Either type is allowed; a SHORT only while the value fits.
                let short = u16_at(tiff, big, e + 2) == Some(SHORT) && value <= u16::MAX as u32;
                set_entry(&mut out, big, e, if short { SHORT } else { LONG }, value);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// TIFF data with the given first-IFD entries (SHORT values) and,
    /// with `sub`, an Exif sub-IFD holding the pixel dimensions as LONGs.
    pub fn tiff(big: bool, tags: &[(u16, u16)], sub: Option<(u32, u32)>) -> Vec<u8> {
        let mut out = if big { b"MM\0\x2a".to_vec() } else { b"II\x2a\0".to_vec() };
        out.extend(u32_bytes(big, 8));
        let n = tags.len() + usize::from(sub.is_some());
        out.extend(u16_bytes(big, n as u16));
        let sub_at = 8 + 2 + 12 * n as u32 + 4;
        let mut all: Vec<(u16, u16, u32)> = tags.iter().map(|&(t, v)| (t, SHORT, v as u32)).collect();
        if sub.is_some() {
            all.push((EXIF_IFD, LONG, sub_at));
        }
        all.sort_by_key(|e| e.0);
        for (tag, kind, value) in all {
            let mut e = [0u8; 12];
            e[..2].copy_from_slice(&u16_bytes(big, tag));
            set_entry(&mut e, big, 0, kind, value);
            out.extend(e);
        }
        // A thumbnail IFD would follow; pointed at, though absent.
        out.extend(u32_bytes(big, 0x1234));
        if let Some((w, h)) = sub {
            out.extend(u16_bytes(big, 2));
            for (tag, value) in [(PIXEL_X, w), (PIXEL_Y, h)] {
                let mut e = [0u8; 12];
                e[..2].copy_from_slice(&u16_bytes(big, tag));
                set_entry(&mut e, big, 0, LONG, value);
                out.extend(e);
            }
            out.extend([0; 4]);
        }
        out
    }

    #[test]
    fn orientation_is_read_and_set() {
        for big in [false, true] {
            let t = tiff(big, &[(0x010f, 7), (ORIENTATION, 6)], None);
            assert_eq!(orientation(&t), Some(6));
            let set = with_orientation(&t, 3).unwrap();
            assert_eq!(set.len(), t.len());
            assert_eq!(orientation(&set), Some(3));
            // The other tag is untouched.
            assert_eq!(entry(&set, big, 8, 0x010f).and_then(|e| u16_at(&set, big, e + 8)), Some(7));
        }
        assert_eq!(orientation(b"nonsense"), None);
        assert_eq!(with_orientation(b"nonsense", 1), None);
    }

    #[test]
    fn a_missing_orientation_is_added_in_a_moved_ifd() {
        for big in [false, true] {
            let t = tiff(big, &[(0x010f, 7), (0x0131, 9)], Some((4000, 3000)));
            assert_eq!(orientation(&t), None);
            let set = with_orientation(&t, 8).unwrap();
            assert_eq!(orientation(&set), Some(8));
            // The old data is where it was, the IFD copied after it.
            assert_eq!(&set[8..t.len()], &t[8..]);
            let ifd = u32_at(&set, big, 4).unwrap() as usize;
            assert!(ifd >= t.len() && ifd.is_multiple_of(2));
            let tags: Vec<u16> = (0..4).map(|k| u16_at(&set, big, ifd + 2 + 12 * k).unwrap()).collect();
            assert_eq!(tags, [0x010f, 0x0112, 0x0131, EXIF_IFD]);
            // The link to the next IFD and the sub-IFD's offset are kept.
            assert_eq!(u32_at(&set, big, ifd + 2 + 48), Some(0x1234));
            let sub = entry(&set, big, ifd, EXIF_IFD).and_then(|e| u32_at(&set, big, e + 8)).unwrap();
            assert!(entry(&set, big, sub as usize, PIXEL_X).is_some());
        }
    }

    #[test]
    fn new_pixels_reset_the_orientation_and_dimensions() {
        for big in [false, true] {
            let t = tiff(big, &[(ORIENTATION, 6)], Some((4000, 3000)));
            let new = for_new_pixels(&t, 1200, 70000).unwrap();
            assert_eq!(orientation(&new), Some(1));
            let ifd = 8;
            let count = ifd_entries(&new, big, ifd).unwrap();
            assert_eq!(u32_at(&new, big, ifd + 2 + 12 * count), Some(0), "the thumbnail is unlinked");
            let sub = entry(&new, big, ifd, EXIF_IFD).and_then(|e| u32_at(&new, big, e + 8)).unwrap() as usize;
            let value = |tag| entry(&new, big, sub, tag).and_then(|e| u32_at(&new, big, e + 8));
            assert_eq!((value(PIXEL_X), value(PIXEL_Y)), (Some(1200), Some(70000)));
        }
    }

    #[test]
    fn minimal_exif_reads_back() {
        assert_eq!(orientation(&minimal(6)), Some(6));
        assert_eq!(image::metadata::Orientation::from_exif_chunk(&minimal(6)), Some(image::metadata::Orientation::Rotate90));
    }
}
