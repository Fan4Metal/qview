//! What a file's metadata says about its image, for the information panel
//! (I) and the order by date taken: the EXIF fields of a photo (camera,
//! lens, exposure, the date it was taken, the place), the titles Windows
//! writes, and the name of the colour profile.
//!
//! EXIF is TIFF-structured data. It is found in a JPEG's APP1 segment, a
//! PNG's eXIf chunk and a WebP's EXIF chunk (these two through the `image`
//! crate), a HEIF's or AVIF's Exif item (through libheif, `heif::metadata`),
//! and a TIFF file is one itself, as most camera RAW files are (CR2, NEF,
//! ARW, DNG, ORF, RW2, PEF): those are read where their tags point, not
//! whole. Formats only Windows reads (JPEG XR, CR3, RAF…) give nothing.

use std::fs::File;
use std::path::Path;

/// A date and time as EXIF holds it: local time, no time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Where a photo was taken: degrees, north and east positive; metres
/// above the sea.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gps {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
}

/// The EXIF fields the panel shows, those the file has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exif {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub software: Option<String>,
    /// ImageDescription, or Windows' title.
    pub title: Option<String>,
    /// Artist, or Windows' authors.
    pub artist: Option<String>,
    pub copyright: Option<String>,
    /// Windows' comment, or the UserComment.
    pub comment: Option<String>,
    /// Windows' tags.
    pub keywords: Option<String>,
    /// In seconds.
    pub exposure: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    /// In millimetres, as it is and as on 35 mm film.
    pub focal: Option<f64>,
    pub focal_35: Option<u32>,
    /// Exposure compensation, in EV.
    pub bias: Option<f64>,
    /// The Flash tag: bit 0 is set when it fired.
    pub flash: Option<u32>,
    /// DateTimeOriginal, else DateTime.
    pub taken: Option<DateTime>,
    /// 1 for sRGB.
    pub color_space: Option<u32>,
    pub gps: Option<Gps>,
}

/// What the panel shows of a file besides what the decoder found.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub file_size: u64,
    /// FILETIMEs, 0 when unknown.
    pub modified: u64,
    pub created: u64,
    /// The image's size upright, from its header.
    pub size: Option<(u32, u32)>,
    pub exif: Exif,
    /// The ICC profile's description ("sRGB IEC61966-2.1", "Display P3").
    pub profile: Option<String>,
}

/// Read what is known of `path` (a file, or an image in an archive): its
/// size and dates, its image's size and its metadata. Windows' codecs may
/// read the header, so the calling thread has COM initialised.
pub fn read(path: &Path) -> Info {
    use std::os::windows::fs::MetadataExt;
    let (file_size, modified, created) = if crate::archive::inside(path) {
        crate::archive::metadata(path).map_or((0, 0, 0), |(size, modified)| (size, modified, 0))
    } else {
        std::fs::metadata(path).map_or((0, 0, 0), |m| (m.len(), m.last_write_time(), m.creation_time()))
    };
    let size = crate::header::read(path).map(crate::header::Size::upright);
    let (exif, icc) = metadata(path);
    let profile = icc.as_deref().and_then(icc_description);
    Info { file_size, modified, created, size, exif: exif.unwrap_or_default(), profile }
}

/// When the photo in `path` was taken, as a FILETIME (its local time on
/// this computer's clock); None when its metadata does not say.
pub fn taken(path: &Path) -> Option<u64> {
    let taken = metadata(path).0?.taken?;
    crate::win::local_to_filetime(taken)
}

/// The EXIF read and the ICC profile of `path`.
fn metadata(path: &Path) -> (Option<Exif>, Option<Vec<u8>>) {
    if crate::archive::inside(path) {
        let Ok(bytes) = crate::archive::read(path) else { return (None, None) };
        return found_in(&bytes[..], || from_image(std::io::Cursor::new(&bytes[..])), || None);
    }
    let Ok(file) = File::open(path) else { return (None, None) };
    // `seek_read` moves the file's cursor on Windows, and the decoder
    // guesses the format from wherever the cursor is.
    let image = || {
        use std::io::Seek;
        let mut file = &file;
        file.rewind().ok()?;
        from_image(std::io::BufReader::new(file))
    };
    found_in(&file, image, || crate::heif::metadata(path))
}

/// Raw EXIF (TIFF data) and ICC profile, as the `image` crate and libheif
/// give them.
pub type Raw = (Option<Vec<u8>>, Option<Vec<u8>>);

/// The metadata of the file `src`: its JPEG segments or TIFF tags read
/// here, or what `heif` (an ISO media file) or `image` (anything else)
/// finds.
fn found_in<S: At + ?Sized>(src: &S, image: impl FnOnce() -> Option<Raw>, heif: impl FnOnce() -> Option<Raw>) -> (Option<Exif>, Option<Vec<u8>>) {
    let Some(head) = src.at(0, 12) else { return (None, None) };
    if head.starts_with(&[0xff, 0xd8]) {
        return jpeg(src);
    }
    if byte_order(&head).is_some() {
        let tiff = Tiff::new(src);
        let icc = tiff.as_ref().and_then(Tiff::icc);
        return (tiff.map(|t| t.exif()), icc);
    }
    let raw = if &head[4..8] == b"ftyp" { heif() } else { image() };
    let (exif, icc) = raw.unwrap_or_default();
    (exif.and_then(|t| Tiff::new(&t[..]).map(|t| t.exif())), icc)
}

/// What the `image` crate's decoder of `reader` finds: PNG's and WebP's
/// chunks.
fn from_image<R: std::io::BufRead + std::io::Seek>(reader: R) -> Option<Raw> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::new(reader).with_guessed_format().ok()?.into_decoder().ok()?;
    Some((decoder.exif_metadata().ok().flatten(), decoder.icc_profile().ok().flatten()))
}

/// The EXIF of a JPEG's APP1 segment and the ICC profile of its APP2
/// segments (a large one is split over several).
fn jpeg<S: At + ?Sized>(src: &S) -> (Option<Exif>, Option<Vec<u8>>) {
    let mut exif = None;
    let mut icc: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut at = 2u64;
    while let Some(marker) = src.at(at, 2) {
        if marker[0] != 0xff {
            break;
        }
        match marker[1] {
            // Fill bytes before a marker.
            0xff => {
                at += 1;
                continue;
            }
            0x01 | 0xd0..=0xd7 => {
                at += 2;
                continue;
            }
            // The image data or its end: no more metadata.
            0xd9 | 0xda => break,
            _ => {}
        }
        let Some(len) = src.at(at + 2, 2).map(|l| u16::from_be_bytes([l[0], l[1]]) as u64) else { break };
        if len < 2 {
            break;
        }
        if (marker[1] == 0xe1 && exif.is_none()) || marker[1] == 0xe2 {
            let data = src.at(at + 4, (len - 2) as usize).unwrap_or_default();
            if let Some(tiff) = data.strip_prefix(b"Exif\0\0") {
                exif = Tiff::new(tiff).map(|t| t.exif());
            } else if let Some(chunk) = data.strip_prefix(b"ICC_PROFILE\0")
                && chunk.len() > 2
            {
                // Its number, then how many there are.
                icc.push((chunk[0], chunk[2..].to_vec()));
            }
        }
        at += 2 + len;
    }
    icc.sort_by_key(|(n, _)| *n);
    let icc = (!icc.is_empty()).then(|| icc.into_iter().flat_map(|(_, c)| c).collect());
    (exif, icc)
}

/// Bytes read at an offset: of a file, or of data in memory.
trait At {
    fn at(&self, offset: u64, len: usize) -> Option<Vec<u8>>;
}

impl At for [u8] {
    fn at(&self, offset: u64, len: usize) -> Option<Vec<u8>> {
        let start = usize::try_from(offset).ok()?;
        self.get(start..start.checked_add(len)?).map(<[u8]>::to_vec)
    }
}

impl At for File {
    fn at(&self, offset: u64, len: usize) -> Option<Vec<u8>> {
        use std::os::windows::fs::FileExt;
        let mut buf = vec![0; len];
        let mut done = 0;
        while done < len {
            match self.seek_read(&mut buf[done..], offset + done as u64) {
                Ok(0) | Err(_) => return None,
                Ok(n) => done += n,
            }
        }
        Some(buf)
    }
}

/// The byte order of TIFF data from its header: big endian for "MM". The
/// RAW files that change the magic number (Olympus' ORF, Panasonic's RW2)
/// count too.
fn byte_order(head: &[u8]) -> Option<bool> {
    match head.get(..4)? {
        [b'M', b'M', 0, 42] => Some(true),
        [b'I', b'I', 42, 0] | [b'I', b'I', b'R', b'O' | b'S'] | [b'I', b'I', 0x55, 0] => Some(false),
        _ => None,
    }
}

/// An IFD entry: its tag, type, count of values and value field (the
/// values themselves when they fit in 4 bytes, else their offset).
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    field: [u8; 4],
}

/// Longer values than this are not read: no field shown is.
const MAX_VALUE: u64 = 1 << 16;
/// Nor is the ICC profile of a TIFF longer than this.
const MAX_ICC: u64 = 4 << 20;

/// TIFF-structured data at the start of `src`.
struct Tiff<'a, S: At + ?Sized> {
    src: &'a S,
    big: bool,
    first: u32,
}

impl<'a, S: At + ?Sized> Tiff<'a, S> {
    fn new(src: &'a S) -> Option<Self> {
        let head = src.at(0, 8)?;
        let big = byte_order(&head)?;
        let mut tiff = Tiff { src, big, first: 0 };
        tiff.first = tiff.u32(&head[4..8]);
        Some(tiff)
    }

    fn u16(&self, b: &[u8]) -> u16 {
        let b = [b[0], b[1]];
        if self.big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    }

    fn u32(&self, b: &[u8]) -> u32 {
        let b = [b[0], b[1], b[2], b[3]];
        if self.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    }

    /// The entries of the IFD at `offset`.
    fn ifd(&self, offset: u32) -> Vec<Entry> {
        let Some(count) = self.src.at(offset as u64, 2).map(|c| self.u16(&c).min(1000) as usize) else { return Vec::new() };
        let Some(data) = self.src.at(offset as u64 + 2, 12 * count) else { return Vec::new() };
        data.as_chunks::<12>()
            .0
            .iter()
            .map(|e| Entry { tag: self.u16(e), kind: self.u16(&e[2..]), count: self.u32(&e[4..]), field: [e[8], e[9], e[10], e[11]] })
            .collect()
    }

    /// The bytes of `e`'s values, up to `max`.
    fn data(&self, e: &Entry, max: u64) -> Option<Vec<u8>> {
        let size = match e.kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => return None,
        } * e.count as u64;
        if size > max {
            return None;
        }
        if size <= 4 {
            return Some(e.field[..size as usize].to_vec());
        }
        self.src.at(self.u32(&e.field) as u64, size as usize)
    }

    /// The text of `e`: ASCII (UTF-8 in practice), up to its first NUL,
    /// trimmed; None when empty.
    fn text(&self, e: &Entry) -> Option<String> {
        let data = self.data(e, MAX_VALUE)?;
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        nonempty(String::from_utf8_lossy(&data[..end]).into_owned())
    }

    /// The text of a Windows field (XPTitle and the like): UTF-16LE, in
    /// bytes, whatever the TIFF's byte order.
    fn windows_text(&self, e: &Entry) -> Option<String> {
        let data = self.data(e, MAX_VALUE)?;
        let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).take_while(|&u| u != 0).collect();
        nonempty(String::from_utf16_lossy(&units))
    }

    /// The UserComment: an 8-byte character code, then the text.
    fn user_comment(&self, e: &Entry) -> Option<String> {
        let data = self.data(e, MAX_VALUE)?;
        let (code, text) = (data.get(..8)?, &data[8..]);
        let text = match code {
            b"UNICODE\0" => {
                let unit = |&c: &[u8; 2]| if self.big { u16::from_be_bytes(c) } else { u16::from_le_bytes(c) };
                String::from_utf16_lossy(&text.as_chunks::<2>().0.iter().map(unit).take_while(|&u| u != 0).collect::<Vec<_>>())
            }
            _ => String::from_utf8_lossy(text.split(|&b| b == 0).next().unwrap_or_default()).into_owned(),
        };
        nonempty(text)
    }

    /// The first value of `e` as a whole number (BYTE, SHORT, LONG, or
    /// IFD, the type TIFF-EP and DNG may give the sub-IFD pointers).
    fn number(&self, e: &Entry) -> Option<u32> {
        if e.count == 0 {
            return None;
        }
        match e.kind {
            1 | 7 => Some(e.field[0] as u32),
            3 => Some(self.u16(&e.field) as u32),
            4 | 13 => Some(self.u32(&e.field)),
            _ => None,
        }
    }

    /// The value `i` of `e`, a RATIONAL or SRATIONAL; None if divided by 0.
    fn rational(&self, e: &Entry, i: usize) -> Option<f64> {
        let data = self.data(e, MAX_VALUE)?;
        let v = data.get(8 * i..8 * i + 8)?;
        let (n, d) = (self.u32(v), self.u32(&v[4..]));
        let (n, d) = match e.kind {
            5 => (n as f64, d as f64),
            10 => (n as i32 as f64, d as i32 as f64),
            _ => return None,
        };
        (d != 0.0).then(|| n / d)
    }

    /// Degrees, minutes and seconds as degrees.
    fn degrees(&self, e: &Entry) -> Option<f64> {
        let (d, m) = (self.rational(e, 0)?, self.rational(e, 1).unwrap_or(0.0));
        Some(d + m / 60.0 + self.rational(e, 2).unwrap_or(0.0) / 3600.0)
    }

    /// The fields the panel shows: the first IFD's, its Exif IFD's and
    /// its GPS IFD's.
    fn exif(&self) -> Exif {
        let mut x = Exif::default();
        let (mut exif_ifd, mut gps_ifd, mut modified) = (None, None, None);
        let set = |field: &mut Option<String>, value: Option<String>| {
            if field.is_none() {
                *field = value;
            }
        };
        for e in self.ifd(self.first) {
            match e.tag {
                0x010e => set(&mut x.title, self.text(&e)),
                0x010f => x.make = self.text(&e),
                0x0110 => x.model = self.text(&e),
                0x0131 => x.software = self.text(&e),
                0x0132 => modified = self.text(&e).as_deref().and_then(parse_date),
                0x013b => set(&mut x.artist, self.text(&e)),
                0x8298 => x.copyright = self.text(&e),
                0x8769 => exif_ifd = self.number(&e),
                0x8825 => gps_ifd = self.number(&e),
                0x9c9b => set(&mut x.title, self.windows_text(&e)),
                0x9c9c => set(&mut x.comment, self.windows_text(&e)),
                0x9c9d => set(&mut x.artist, self.windows_text(&e)),
                0x9c9e => x.keywords = self.windows_text(&e),
                _ => {}
            }
        }
        let mut lens_make = None;
        for e in exif_ifd.map(|o| self.ifd(o)).unwrap_or_default() {
            match e.tag {
                0x829a => x.exposure = self.rational(&e, 0),
                0x829d => x.f_number = self.rational(&e, 0),
                0x8827 => x.iso = self.number(&e),
                0x9003 => x.taken = self.text(&e).as_deref().and_then(parse_date),
                0x9204 => x.bias = self.rational(&e, 0),
                0x9209 => x.flash = self.number(&e),
                0x920a => x.focal = self.rational(&e, 0),
                0x9286 => set(&mut x.comment, self.user_comment(&e)),
                0xa001 => x.color_space = self.number(&e),
                0xa405 => x.focal_35 = self.number(&e).filter(|&f| f > 0),
                0xa433 => lens_make = self.text(&e),
                0xa434 => x.lens = self.text(&e),
                _ => {}
            }
        }
        if x.lens.is_none() {
            x.lens = lens_make;
        }
        x.taken = x.taken.or(modified);
        x.gps = gps_ifd.and_then(|o| self.gps(o));
        x
    }

    fn gps(&self, offset: u32) -> Option<Gps> {
        let (mut lat, mut lon, mut alt) = (None, None, None);
        let (mut south, mut west, mut below) = (false, false, false);
        for e in self.ifd(offset) {
            match e.tag {
                1 => south = self.text(&e).is_some_and(|r| r.eq_ignore_ascii_case("S")),
                2 => lat = self.degrees(&e),
                3 => west = self.text(&e).is_some_and(|r| r.eq_ignore_ascii_case("W")),
                4 => lon = self.degrees(&e),
                5 => below = self.number(&e) == Some(1),
                6 => alt = self.rational(&e, 0),
                _ => {}
            }
        }
        let (lat, lon) = (lat?, lon?);
        // 0, 0 is what some phones write without a fix.
        if !(lat.abs() <= 90.0 && lon.abs() <= 180.0) || (lat == 0.0 && lon == 0.0) {
            return None;
        }
        let sign = |negative: bool, v: f64| if negative { -v } else { v };
        Some(Gps { latitude: sign(south, lat), longitude: sign(west, lon), altitude: alt.map(|a| sign(below, a)) })
    }

    /// The ICC profile of the first IFD (InterColorProfile).
    fn icc(&self) -> Option<Vec<u8>> {
        let e = self.ifd(self.first).into_iter().find(|e| e.tag == 0x8773)?;
        self.data(&e, MAX_ICC)
    }
}

fn nonempty(s: String) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// "2024:05:01 14:30:22"; None for the "0000:00:00 00:00:00" of cameras
/// whose clock was not set.
fn parse_date(s: &str) -> Option<DateTime> {
    let n = |range: std::ops::Range<usize>| s.get(range)?.trim().parse::<u16>().ok();
    let date = DateTime {
        year: n(0..4)?,
        month: n(5..7)? as u8,
        day: n(8..10)? as u8,
        hour: n(11..13).unwrap_or(0) as u8,
        minute: n(14..16).unwrap_or(0) as u8,
        second: n(17..19).unwrap_or(0) as u8,
    };
    let valid = date.year >= 1800
        && (1..=12).contains(&date.month)
        && (1..=31).contains(&date.day)
        && date.hour < 24
        && date.minute < 60
        && date.second < 61;
    valid.then_some(date)
}

/// The description of an ICC profile: its 'desc' tag, of type 'desc'
/// (version 2: ASCII) or 'mluc' (version 4: UTF-16, the first language).
pub fn icc_description(icc: &[u8]) -> Option<String> {
    let u32_at = |i: usize| icc.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let count = u32_at(128)?.min(200);
    let (offset, size) = (0..count).map(|k| 132 + 12 * k).find(|&e| icc.get(e..e + 4) == Some(b"desc")).map(|e| (u32_at(e + 4), u32_at(e + 8)))?;
    let (offset, size) = (offset?, size?);
    let tag = icc.get(offset..offset.checked_add(size)?)?;
    let at = |i: usize| tag.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let text = match tag.get(..4)? {
        b"desc" => {
            let len = at(8)?;
            let ascii = tag.get(12..12usize.checked_add(len)?)?;
            String::from_utf8_lossy(ascii.split(|&b| b == 0).next().unwrap_or_default()).into_owned()
        }
        b"mluc" => {
            let (records, record) = (at(8)?, at(12)?);
            if records == 0 || record < 12 {
                return None;
            }
            let (len, start) = (at(16 + 4)?, at(16 + 8)?);
            let utf16 = tag.get(start..start.checked_add(len)?)?;
            let units: Vec<u16> = utf16.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).take_while(|&u| u != 0).collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    nonempty(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TIFF data, little endian or big, made of IFDs given as (tag, type,
    /// values as bytes in the TIFF's order); the first IFD's entries
    /// 0x8769 and 0x8825 point at the second and third.
    struct Builder {
        big: bool,
    }

    impl Builder {
        fn u16(&self, v: u16) -> [u8; 2] {
            if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
        }
        fn u32(&self, v: u32) -> [u8; 4] {
            if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
        }
        fn rational(&self, n: u32, d: u32) -> Vec<u8> {
            [self.u32(n), self.u32(d)].concat()
        }
        fn ascii(s: &str) -> Vec<u8> {
            [s.as_bytes(), b"\0"].concat()
        }

        /// The IFDs one after another from offset 8, each entry's long
        /// values after all the IFDs.
        fn build(&self, ifds: &[Vec<(u16, u16, Vec<u8>)>]) -> Vec<u8> {
            let size = |ifd: &Vec<(u16, u16, Vec<u8>)>| 2 + 12 * ifd.len() as u32 + 4;
            let mut starts = vec![8u32];
            for ifd in ifds {
                starts.push(starts.last().unwrap() + size(ifd));
            }
            let mut extra_at = *starts.last().unwrap();
            let mut out = if self.big { b"MM\0\x2a".to_vec() } else { b"II\x2a\0".to_vec() };
            out.extend(self.u32(8));
            let mut extra = Vec::new();
            for ifd in ifds {
                out.extend(self.u16(ifd.len() as u16));
                for (tag, kind, value) in ifd {
                    let unit = match kind {
                        1 | 2 | 7 => 1,
                        3 => 2,
                        4 => 4,
                        _ => 8,
                    };
                    let mut value = value.clone();
                    // A pointer to the next IFDs.
                    if *tag == 0x8769 {
                        value = self.u32(starts[1]).to_vec();
                    } else if *tag == 0x8825 {
                        value = self.u32(starts[2]).to_vec();
                    }
                    out.extend(self.u16(*tag));
                    out.extend(self.u16(*kind));
                    out.extend(self.u32((value.len() / unit) as u32));
                    if value.len() <= 4 {
                        value.resize(4, 0);
                        out.extend(value);
                    } else {
                        out.extend(self.u32(extra_at));
                        extra_at += value.len() as u32;
                        extra.extend(value);
                    }
                }
                out.extend(self.u32(0));
            }
            out.extend(extra);
            out
        }
    }

    fn sample(big: bool) -> Vec<u8> {
        let b = Builder { big };
        let short = |v: u16| b.u16(v).to_vec();
        b.build(&[
            vec![
                (0x010f, 2, Builder::ascii("Canon")),
                (0x0110, 2, Builder::ascii("Canon EOS R6")),
                (0x0132, 2, Builder::ascii("2024:05:02 10:00:00")),
                (0x8769, 4, vec![0; 4]),
                (0x8825, 4, vec![0; 4]),
                (0x9c9b, 1, "Закат\0".encode_utf16().flat_map(u16::to_le_bytes).collect()),
            ],
            vec![
                (0x829a, 5, b.rational(1, 250)),
                (0x829d, 5, b.rational(28, 10)),
                (0x8827, 3, short(400)),
                (0x9003, 2, Builder::ascii("2024:05:01 14:30:22")),
                (0x9204, 10, [b.u32(-2i32 as u32), b.u32(3)].concat()),
                (0x9209, 3, short(16)),
                (0x920a, 5, b.rational(50, 1)),
                (0xa001, 3, short(1)),
                (0xa405, 3, short(80)),
                (0xa434, 2, Builder::ascii("RF50mm F1.8 STM")),
            ],
            vec![
                (1, 2, Builder::ascii("N")),
                (2, 5, [b.rational(55, 1), b.rational(45, 1), b.rational(2088, 100)].concat()),
                (3, 2, Builder::ascii("W")),
                (4, 5, [b.rational(37, 1), b.rational(37, 1), b.rational(0, 1)].concat()),
                (5, 1, vec![1]),
                (6, 5, b.rational(15, 1)),
            ],
        ])
    }

    #[test]
    fn reads_the_fields() {
        for big in [false, true] {
            let tiff = sample(big);
            let x = Tiff::new(&tiff[..]).unwrap().exif();
            assert_eq!(x.make.as_deref(), Some("Canon"));
            assert_eq!(x.model.as_deref(), Some("Canon EOS R6"));
            assert_eq!(x.lens.as_deref(), Some("RF50mm F1.8 STM"));
            assert_eq!(x.title.as_deref(), Some("Закат"));
            assert_eq!(x.exposure, Some(1.0 / 250.0));
            assert_eq!(x.f_number, Some(2.8));
            assert_eq!(x.iso, Some(400));
            assert_eq!(x.focal, Some(50.0));
            assert_eq!(x.focal_35, Some(80));
            assert!((x.bias.unwrap() + 2.0 / 3.0).abs() < 1e-9);
            assert_eq!(x.flash, Some(16));
            assert_eq!(x.color_space, Some(1));
            // DateTimeOriginal, not DateTime.
            let taken = DateTime { year: 2024, month: 5, day: 1, hour: 14, minute: 30, second: 22 };
            assert_eq!(x.taken, Some(taken));
            let gps = x.gps.unwrap();
            assert!((gps.latitude - (55.0 + 45.0 / 60.0 + 20.88 / 3600.0)).abs() < 1e-9);
            assert!((gps.longitude + (37.0 + 37.0 / 60.0)).abs() < 1e-9);
            assert_eq!(gps.altitude, Some(-15.0));
        }
    }

    #[test]
    fn reads_a_jpeg_and_a_tiff_file() {
        let tiff = sample(false);
        // A JPEG: SOI, APP0, APP1 with the EXIF, APP2 with an ICC profile in
        // two chunks, out of order, then SOS.
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
        let segment = |jpeg: &mut Vec<u8>, code: u8, payload: &[u8]| {
            jpeg.extend([0xff, code]);
            jpeg.extend(((payload.len() + 2) as u16).to_be_bytes());
            jpeg.extend(payload);
        };
        segment(&mut jpeg, 0xe1, &[b"Exif\0\0".as_slice(), &tiff].concat());
        segment(&mut jpeg, 0xe2, &[b"ICC_PROFILE\0\x02\x02".as_slice(), b"def"].concat());
        segment(&mut jpeg, 0xe2, &[b"ICC_PROFILE\0\x01\x02".as_slice(), b"abc"].concat());
        jpeg.extend([0xff, 0xda, 0, 2, 0xff, 0xd9]);
        let (exif, icc) = found_in(&jpeg[..], || None, || None);
        assert_eq!(exif.unwrap().iso, Some(400));
        assert_eq!(icc.as_deref(), Some(&b"abcdef"[..]));
        // The TIFF as a file on disk, read where its tags point.
        let dir = std::env::temp_dir().join(format!("qview_info_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("photo.tif");
        std::fs::write(&path, &tiff).unwrap();
        let exif = metadata(&path).0.unwrap();
        assert_eq!(exif.lens.as_deref(), Some("RF50mm F1.8 STM"));
        assert!(taken(&path).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
        // Nonsense is no metadata, and no panic.
        assert_eq!(found_in(&b"II\x2a\0\xff\xff\xff\xff\0\0\0\0"[..], || None, || None).0, Some(Exif::default()));
        assert_eq!(found_in(&b"short"[..], || None, || None).0, None);
    }

    #[test]
    fn png_exif_through_image() {
        let tiff = sample(true);
        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
            encoder.set_exif_metadata(tiff).unwrap();
            encoder.write_image(&[0, 0, 0], 1, 1, image::ExtendedColorType::Rgb8).unwrap();
        }
        let (exif, _) = found_in(&png[..], || from_image(std::io::Cursor::new(&png[..])), || None);
        assert_eq!(exif.unwrap().model.as_deref(), Some("Canon EOS R6"));
        // From a file on disk too: the header is read first, and the
        // decoder must start from the beginning.
        let dir = std::env::temp_dir().join(format!("qview_info_png_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("camera.png");
        std::fs::write(&path, &png).unwrap();
        assert_eq!(metadata(&path).0.unwrap().model.as_deref(), Some("Canon EOS R6"));
        assert!(taken(&path).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `$env:QVIEW_INFO_FILE="<file>"; cargo test --release info_file -- --ignored --nocapture`:
    /// what the panel reads of one file, and how long it takes.
    #[test]
    #[ignore]
    fn info_file() {
        let path = std::path::PathBuf::from(std::env::var("QVIEW_INFO_FILE").expect("QVIEW_INFO_FILE"));
        let _com = crate::win::com_init();
        for _ in 0..2 {
            let t = std::time::Instant::now();
            let info = read(&path);
            let read = t.elapsed();
            let t = std::time::Instant::now();
            let taken = taken(&path);
            println!("{info:#?}\nread in {:.1} ms; taken {taken:?} in {:.1} ms", read.as_secs_f64() * 1e3, t.elapsed().as_secs_f64() * 1e3);
        }
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("2024:05:01 14:30:22"), Some(DateTime { year: 2024, month: 5, day: 1, hour: 14, minute: 30, second: 22 }));
        assert_eq!(parse_date("0000:00:00 00:00:00"), None);
        assert_eq!(parse_date("    :  :     :  :  "), None);
        assert_eq!(parse_date("2024:05:01"), Some(DateTime { year: 2024, month: 5, day: 1, hour: 0, minute: 0, second: 0 }));
    }

    #[test]
    fn icc_descriptions() {
        // A version 2 'desc' and a version 4 'mluc'.
        let profile = |tag: Vec<u8>| {
            let mut icc = vec![0u8; 128];
            icc.extend(1u32.to_be_bytes());
            icc.extend(b"desc");
            icc.extend(144u32.to_be_bytes());
            icc.extend((tag.len() as u32).to_be_bytes());
            icc.extend(tag);
            icc
        };
        let mut desc = b"desc\0\0\0\0".to_vec();
        desc.extend(18u32.to_be_bytes());
        desc.extend(b"sRGB IEC61966-2.1\0");
        assert_eq!(icc_description(&profile(desc)).as_deref(), Some("sRGB IEC61966-2.1"));
        let text: Vec<u8> = "Display P3".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut mluc = b"mluc\0\0\0\0".to_vec();
        for v in [1u32, 12] {
            mluc.extend(v.to_be_bytes());
        }
        mluc.extend(b"enUS");
        mluc.extend((text.len() as u32).to_be_bytes());
        mluc.extend(28u32.to_be_bytes());
        mluc.extend(text);
        assert_eq!(icc_description(&profile(mluc)).as_deref(), Some("Display P3"));
        assert_eq!(icc_description(b"nonsense"), None);
    }
}
