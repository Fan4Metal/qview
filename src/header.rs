//! The size of an image from its file's header, without decoding it.
//!
//! The `image` crate's JPEG decoder reads the whole file even for its
//! dimensions (several megabytes for a photo), so JPEG is read here
//! segment by segment up to the frame header: the EXIF orientation and the
//! size are in the first few kilobytes. Other formats go through `image`,
//! whose decoders read only their headers.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use image::metadata::Orientation;

/// Size of an image as stored, and whether its orientation turns it a
/// quarter (the sides swap when it is shown upright).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
    pub turned: bool,
}

impl Size {
    /// Width and height as the image is shown, upright.
    pub fn upright(self) -> (u32, u32) {
        if self.turned { (self.height, self.width) } else { (self.width, self.height) }
    }
}

/// The size of the image in `path`, from its header.
pub fn read(path: &Path) -> Option<Size> {
    let mut file = BufReader::with_capacity(16 * 1024, File::open(path).ok()?);
    if file.fill_buf().ok()?.starts_with(&[0xff, 0xd8]) {
        return jpeg(&mut file);
    }
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut decoder = image::ImageReader::new(file).with_guessed_format().ok()?.into_decoder().ok()?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    let orientation = image::ImageDecoder::orientation(&mut decoder).unwrap_or(Orientation::NoTransforms);
    Some(Size { width, height, turned: turns(orientation) })
}

fn turns(o: Orientation) -> bool {
    matches!(o, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH)
}

/// The JPEG segments up to the frame header: the EXIF orientation from
/// APP1 (which comes first) and the size from SOFn.
fn jpeg(r: &mut BufReader<File>) -> Option<Size> {
    fn read_u16(r: &mut impl Read) -> Option<u16> {
        let mut b = [0u8; 2];
        r.read_exact(&mut b).ok()?;
        Some(u16::from_be_bytes(b))
    }
    let mut turned = false;
    let mut byte = [0u8; 1];
    r.consume(2);
    loop {
        // A marker: 0xFF (any number of them), then its code.
        r.read_exact(&mut byte).ok()?;
        if byte[0] != 0xff {
            return None;
        }
        let mut code = 0xff;
        while code == 0xff {
            r.read_exact(&mut byte).ok()?;
            code = byte[0];
        }
        // Markers without a length.
        if matches!(code, 0x01 | 0xd0..=0xd8) {
            continue;
        }
        // Start of scan or end of image before a frame header: none.
        if matches!(code, 0xd9 | 0xda) {
            return None;
        }
        let len = read_u16(r)?.checked_sub(2)? as usize;
        match code {
            // SOF0-15 except DHT (C4), JPG (C8) and DAC (CC).
            0xc0..=0xcf if !matches!(code, 0xc4 | 0xc8 | 0xcc) => {
                let mut sof = [0u8; 5];
                r.read_exact(&mut sof).ok()?;
                let height = u16::from_be_bytes([sof[1], sof[2]]) as u32;
                let width = u16::from_be_bytes([sof[3], sof[4]]) as u32;
                return (width > 0 && height > 0).then_some(Size { width, height, turned });
            }
            0xe1 => {
                let mut data = vec![0u8; len];
                r.read_exact(&mut data).ok()?;
                if let Some(o) = exif_orientation(&data) {
                    turned = turns(o);
                }
            }
            // Within the buffer, unlike `seek`, which drops it.
            _ => r.seek_relative(len as i64).ok()?,
        }
    }
}

/// The orientation in an APP1 segment's payload, if it is EXIF and has
/// one: tag 0x0112 of the first IFD.
fn exif_orientation(data: &[u8]) -> Option<Orientation> {
    let tiff = data.strip_prefix(b"Exif\0\0")?;
    let big = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(i..i + 2)?.try_into().ok()?;
        Some(if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(i..i + 4)?.try_into().ok()?;
        Some(if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count).map(|k| ifd + 2 + 12 * k).find(|&e| u16_at(e) == Some(0x0112)).and_then(|e| {
        // A SHORT, in the first two bytes of the value field.
        Orientation::from_exif(u16_at(e + 8)?.try_into().ok()?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_header_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A JPEG of `w` x `h` whose EXIF says `orientation`, in `big` or
    /// little endian, the EXIF segment after an APP0 and a comment.
    fn jpeg_with_exif(w: u16, h: u16, orientation: u16, big: bool) -> Vec<u8> {
        let mut img = Vec::new();
        image::RgbImage::new(w as u32, h as u32)
            .write_to(&mut std::io::Cursor::new(&mut img), image::ImageFormat::Jpeg)
            .unwrap();
        let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut tiff = Vec::new();
        tiff.extend_from_slice(if big { b"MM" } else { b"II" });
        tiff.extend(u16b(42));
        tiff.extend(u32b(8));
        tiff.extend(u16b(2));
        // An unrelated tag first, then Orientation.
        for (tag, value) in [(0x010f_u16, 0_u16), (0x0112, orientation)] {
            tiff.extend(u16b(tag));
            tiff.extend(u16b(3));
            tiff.extend(u32b(1));
            tiff.extend(u16b(value));
            tiff.extend([0, 0]);
        }
        tiff.extend(u32b(0));
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut out = vec![0xff, 0xd8];
        out.extend([0xff, 0xfe, 0x00, 0x05, b'h', b'i', b'!']);
        out.extend([0xff, 0xe1]);
        out.extend(((app1.len() + 2) as u16).to_be_bytes());
        out.extend(app1);
        // The encoder's segments after its SOI.
        out.extend(&img[2..]);
        out
    }

    /// Header reads of every image in `QVIEW_THUMB_DIR` against the
    /// `image` crate's `into_dimensions`, printed with `--nocapture`.
    #[test]
    #[ignore]
    fn header_timings() {
        use std::time::Instant;
        let dir = PathBuf::from(std::env::var("QVIEW_THUMB_DIR").expect("QVIEW_THUMB_DIR"));
        let files = crate::folder::list(&dir, None, Default::default()).unwrap();
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let ours: Vec<_> = files.iter().map(|f| read(f).map(|s| (s.width, s.height))).collect();
        let ours_ms = ms(t);
        let t = Instant::now();
        let theirs: Vec<_> = files
            .iter()
            .map(|f| image::ImageReader::open(f).ok()?.with_guessed_format().ok()?.into_dimensions().ok())
            .collect();
        let theirs_ms = ms(t);
        for ((f, a), b) in files.iter().zip(&ours).zip(&theirs) {
            if a != b {
                println!("differs: {} {a:?} {b:?}", f.display());
            }
        }
        println!("{} files: headers {ours_ms:.1} ms, into_dimensions {theirs_ms:.1} ms", files.len());
    }

    #[test]
    fn reads_sizes_and_orientation() {
        let dir = temp_dir("sizes");
        let png = dir.join("a.png");
        image::RgbImage::new(30, 20).save(&png).unwrap();
        assert_eq!(read(&png), Some(Size { width: 30, height: 20, turned: false }));

        let plain = dir.join("plain.jpg");
        image::RgbImage::new(40, 10).save(&plain).unwrap();
        assert_eq!(read(&plain).unwrap().upright(), (40, 10));

        for big in [false, true] {
            let turned = dir.join("turned.jpg");
            std::fs::write(&turned, jpeg_with_exif(40, 10, 6, big)).unwrap();
            assert_eq!(read(&turned), Some(Size { width: 40, height: 10, turned: true }), "big endian: {big}");
            assert_eq!(read(&turned).unwrap().upright(), (10, 40));
            let upside_down = dir.join("upside_down.jpg");
            std::fs::write(&upside_down, jpeg_with_exif(40, 10, 3, big)).unwrap();
            assert_eq!(read(&upside_down).unwrap().upright(), (40, 10));
        }

        let broken = dir.join("broken.jpg");
        std::fs::write(&broken, [0xff, 0xd8, 0xff, 0xe0, 0x00]).unwrap();
        assert_eq!(read(&broken), None);
        assert_eq!(read(&dir.join("missing.png")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
