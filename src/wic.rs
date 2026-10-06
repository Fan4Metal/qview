//! Windows Imaging Component: Windows' own codecs, for what the `image`
//! crate cannot read: HEIC/HEIF and AVIF (with the HEIF Image Extensions
//! and the HEVC or AV1 video extensions from the Microsoft Store), camera
//! RAW (Raw Image Extension), JPEG XR, DDS, and whatever other codecs are
//! installed. WIC hands out premultiplied BGRA, the order the textures
//! take, so a decoded image needs no conversion.
//!
//! The interfaces are declared by hand, as in `thumbs`: `windows-sys` has
//! no COM interfaces. Every call is made through [`Com::method`] with the
//! index of the method in the interface's vtable (from `wincodec.h`).

use std::ffi::c_void;
use std::path::Path;
use std::ptr::null_mut;
use std::cell::RefCell;
use std::sync::OnceLock;

use windows_sys::core::{GUID, HRESULT};

use crate::filetypes::{FILE_TYPES, RAW_EXTENSIONS as RAW};

const CLSID_WIC_IMAGING_FACTORY: GUID = GUID::from_u128(0xcacaf262_9370_4615_a13b_9f5539da4c0a);
const IID_IWIC_IMAGING_FACTORY: GUID = GUID::from_u128(0xec5ec8a9_c395_4314_9c77_54d7a935ff70);
const IID_IWIC_PIXEL_FORMAT_INFO: GUID = GUID::from_u128(0xe8eda601_3d48_431a_ab44_69059be88bbe);
const IID_IWIC_BITMAP_CODEC_INFO: GUID = GUID::from_u128(0xe87a44c4_b76e_4c47_8b09_298eb12a2714);
const PIXEL_FORMAT_32BPP_PBGRA: GUID = GUID::from_u128(0x6fddc324_4e03_4bfe_b185_3d77768dc910);
const PIXEL_FORMAT_32BPP_RGBA: GUID = GUID::from_u128(0xf5c7ad2d_6a8d_43dd_a7a8_a29935261ae9);
/// No codec takes the file.
const WINCODEC_ERR_COMPONENTNOTFOUND: HRESULT = 0x88982f50_u32 as HRESULT;


/// What the hand-declared vtables are called with.
pub(crate) type Unknown = *mut c_void;
type QueryInterface = unsafe extern "system" fn(Unknown, *const GUID, *mut Unknown) -> HRESULT;
type Release = unsafe extern "system" fn(Unknown) -> u32;
/// A method that returns an interface.
type Create = unsafe extern "system" fn(Unknown, *mut Unknown) -> HRESULT;

/// PROPVARIANT, of which only the small integer types are read.
#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    data: [u64; 2],
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn PropVariantClear(pvar: *mut PropVariant) -> HRESULT;
}

/// A COM interface pointer, released when dropped.
pub(crate) struct Com(pub(crate) Unknown);

impl Com {
    /// Method `index` of the interface's vtable, as a function of type `F`.
    ///
    /// # Safety
    /// `F` must be the method's signature.
    pub(crate) unsafe fn method<F: Copy>(&self, index: usize) -> F {
        unsafe {
            let vtbl = *self.0.cast::<*const usize>();
            std::mem::transmute_copy(&*vtbl.add(index))
        }
    }

    /// The interface `iid` of the same object.
    fn query(&self, iid: &GUID) -> Option<Com> {
        let mut out = null_mut();
        let hr = unsafe { self.method::<QueryInterface>(0)(self.0, iid, &mut out) };
        (hr >= 0 && !out.is_null()).then_some(Com(out))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { self.method::<Release>(2)(self.0) };
    }
}

fn check(hr: HRESULT, what: &str) -> Result<(), String> {
    match hr {
        0.. => Ok(()),
        WINCODEC_ERR_COMPONENTNOTFOUND => Err("Windows has no codec for this file".into()),
        _ => Err(format!("WIC {what} failed ({:#010x})", hr as u32)),
    }
}

/// `IWICImagingFactory`.
fn factory() -> Result<Com, String> {
    use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    let mut out = null_mut();
    let hr = unsafe {
        CoCreateInstance(&CLSID_WIC_IMAGING_FACTORY, null_mut(), CLSCTX_INPROC_SERVER, &IID_IWIC_IMAGING_FACTORY, &mut out)
    };
    check(hr, "CoCreateInstance")?;
    Ok(Com(out))
}

/// The first frame (`IWICBitmapFrameDecode`) of `path`.
fn first_frame(factory: &Com, path: &Path) -> Result<Com, String> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const METADATA_CACHE_ON_DEMAND: i32 = 0;
    type CreateDecoderFromFilename =
        unsafe extern "system" fn(Unknown, *const u16, *const GUID, u32, i32, *mut Unknown) -> HRESULT;
    type GetFrame = unsafe extern "system" fn(Unknown, u32, *mut Unknown) -> HRESULT;
    let wide = crate::win::wide(path);
    let mut decoder = null_mut();
    let hr = unsafe {
        factory.method::<CreateDecoderFromFilename>(3)(
            factory.0,
            wide.as_ptr(),
            std::ptr::null(),
            GENERIC_READ,
            METADATA_CACHE_ON_DEMAND,
            &mut decoder,
        )
    };
    check(hr, "CreateDecoderFromFilename")?;
    let decoder = Com(decoder);
    keep_codec(factory, &decoder);
    let mut frame = null_mut();
    check(unsafe { decoder.method::<GetFrame>(13)(decoder.0, 0, &mut frame) }, "GetFrame")?;
    Ok(Com(frame))
}

/// A container format's GUID, comparable.
type Container = (u32, u16, u16, [u8; 8]);

thread_local! {
    /// An instance of each codec this thread has used, with no file: see
    /// [`keep_codec`].
    static KEPT: RefCell<Vec<(Container, Com)>> = const { RefCell::new(Vec::new()) };
}

/// Keep an instance of the codec of `decoder` while this thread uses COM:
/// releasing the last instance of some codecs is slow (HEIF's takes
/// ~260 ms, its video decoder shut down), and a thread decoding one file
/// after another would pay it for each. An uninitialised decoder (no file,
/// nothing locked) is enough, and makes creating the next one faster
/// (~2 ms instead of ~25 for HEIF). Only on a thread whose own COM guard
/// releases them (`win::com_init`, nested in the caller's).
fn keep_codec(factory: &Com, decoder: &Com) {
    type GetContainerFormat = unsafe extern "system" fn(Unknown, *mut GUID) -> HRESULT;
    type CreateDecoder = unsafe extern "system" fn(Unknown, *const GUID, *const GUID, *mut Unknown) -> HRESULT;
    if crate::win::com_depth() < 2 {
        return;
    }
    let mut container = GUID::from_u128(0);
    if unsafe { decoder.method::<GetContainerFormat>(5)(decoder.0, &mut container) } < 0 {
        return;
    }
    let key = (container.data1, container.data2, container.data3, container.data4);
    if KEPT.with_borrow(|kept| kept.iter().any(|(k, _)| *k == key)) {
        return;
    }
    let mut instance = null_mut();
    if unsafe { factory.method::<CreateDecoder>(7)(factory.0, &container, std::ptr::null(), &mut instance) } >= 0
        && !instance.is_null()
    {
        KEPT.with_borrow_mut(|kept| kept.push((key, Com(instance))));
    }
}

/// Release the codec instances this thread keeps; called by the outermost
/// `win::com_init` guard, before COM is uninitialised.
pub fn release_kept() {
    // Not when the thread is ending and its locals are gone.
    let kept = KEPT.try_with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    drop(kept);
}

/// Size of a bitmap source (`IWICBitmapSource::GetSize`).
fn size_of_source(source: &Com) -> Result<(u32, u32), String> {
    type GetSize = unsafe extern "system" fn(Unknown, *mut u32, *mut u32) -> HRESULT;
    let (mut w, mut h) = (0, 0);
    check(unsafe { source.method::<GetSize>(3)(source.0, &mut w, &mut h) }, "GetSize")?;
    Ok((w, h))
}

/// The EXIF orientation of `frame` (1 to 8), through the photo metadata
/// policy, whatever the format keeps it in; 1 if it has none.
fn orientation(frame: &Com) -> u16 {
    const VT_UI1: u16 = 17;
    const VT_UI2: u16 = 18;
    const VT_UI4: u16 = 19;
    type GetMetadataByName = unsafe extern "system" fn(Unknown, *const u16, *mut PropVariant) -> HRESULT;
    let mut reader = null_mut();
    // IWICBitmapFrameDecode::GetMetadataQueryReader
    if unsafe { frame.method::<Create>(8)(frame.0, &mut reader) } < 0 || reader.is_null() {
        return 1;
    }
    let reader = Com(reader);
    let name: Vec<u16> = "System.Photo.Orientation".encode_utf16().chain([0]).collect();
    let mut value = PropVariant { vt: 0, reserved: [0; 3], data: [0; 2] };
    let hr = unsafe { reader.method::<GetMetadataByName>(5)(reader.0, name.as_ptr(), &mut value) };
    let found = match value.vt {
        VT_UI1 | VT_UI2 | VT_UI4 if hr >= 0 => value.data[0] as u32 & 0xffff,
        _ => 1,
    };
    unsafe { PropVariantClear(&mut value) };
    if (1..=8).contains(&found) { found as u16 } else { 1 }
}

/// Bits per pixel of the pixel format `format`; 0 if unknown.
fn bits_per_pixel(factory: &Com, format: &GUID) -> u16 {
    type CreateComponentInfo = unsafe extern "system" fn(Unknown, *const GUID, *mut Unknown) -> HRESULT;
    type GetBitsPerPixel = unsafe extern "system" fn(Unknown, *mut u32) -> HRESULT;
    let mut info = null_mut();
    if unsafe { factory.method::<CreateComponentInfo>(6)(factory.0, format, &mut info) } < 0 || info.is_null() {
        return 0;
    }
    let Some(info) = Com(info).query(&IID_IWIC_PIXEL_FORMAT_INFO) else { return 0 };
    let mut bits = 0;
    let hr = unsafe { info.method::<GetBitsPerPixel>(13)(info.0, &mut bits) };
    if hr >= 0 { bits as u16 } else { 0 }
}

/// A decoded image, as stored (not turned by its orientation).
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Four bytes a pixel, top row first.
    pub pixels: Vec<u8>,
    /// EXIF orientation, 1 to 8.
    pub orientation: u16,
    /// Bits per pixel in the file.
    pub bits: u16,
}

/// Decode the first frame of `path`: premultiplied BGRA with `bgra`,
/// otherwise straight RGBA. Call it on a thread where COM is initialised
/// (`win::com_init`), so that the codecs stay loaded between calls.
pub fn decode(path: &Path, bgra: bool) -> Result<Image, String> {
    type GetPixelFormat = unsafe extern "system" fn(Unknown, *mut GUID) -> HRESULT;
    type Initialize = unsafe extern "system" fn(Unknown, Unknown, *const GUID, i32, Unknown, f64, i32) -> HRESULT;
    type CopyPixels = unsafe extern "system" fn(Unknown, *const c_void, u32, u32, *mut u8) -> HRESULT;
    let _com = crate::win::com_init();
    let factory = factory()?;
    let frame = first_frame(&factory, path)?;
    let (width, height) = size_of_source(&frame)?;
    let mut format = GUID::from_u128(0);
    check(unsafe { frame.method::<GetPixelFormat>(4)(frame.0, &mut format) }, "GetPixelFormat")?;
    let bits = bits_per_pixel(&factory, &format);
    let orientation = orientation(&frame);
    let stride = width as usize * 4;
    let len = stride * height as usize;
    if width == 0 || height == 0 || len > u32::MAX as usize / 2 {
        return Err(format!("unsupported image size {width}x{height}"));
    }
    let mut converter = null_mut();
    // IWICImagingFactory::CreateFormatConverter
    check(unsafe { factory.method::<Create>(10)(factory.0, &mut converter) }, "CreateFormatConverter")?;
    let converter = Com(converter);
    let target = if bgra { PIXEL_FORMAT_32BPP_PBGRA } else { PIXEL_FORMAT_32BPP_RGBA };
    // No dithering, no palette.
    let hr = unsafe { converter.method::<Initialize>(8)(converter.0, frame.0, &target, 0, null_mut(), 0.0, 0) };
    check(hr, "format conversion")?;
    let mut pixels = vec![0u8; len];
    let hr = unsafe {
        converter.method::<CopyPixels>(7)(converter.0, std::ptr::null(), stride as u32, len as u32, pixels.as_mut_ptr())
    };
    check(hr, "CopyPixels")?;
    Ok(Image { width, height, pixels, orientation, bits })
}

/// The size of the image in `path` as stored and its EXIF orientation,
/// without decoding the pixels.
pub fn size(path: &Path) -> Option<(u32, u32, u16)> {
    let _com = crate::win::com_init();
    let factory = factory().ok()?;
    let frame = first_frame(&factory, path).ok()?;
    let (w, h) = size_of_source(&frame).ok()?;
    Some((w, h, orientation(&frame)))
}

/// The extensions (lowercase, without the dot) of all WIC decoders
/// installed.
fn decoder_extensions() -> Result<Vec<String>, String> {
    const WIC_DECODER: u32 = 1;
    const ENUMERATE_DEFAULT: u32 = 0;
    type CreateComponentEnumerator = unsafe extern "system" fn(Unknown, u32, u32, *mut Unknown) -> HRESULT;
    type Next = unsafe extern "system" fn(Unknown, u32, *mut Unknown, *mut u32) -> HRESULT;
    type GetFileExtensions = unsafe extern "system" fn(Unknown, u32, *mut u16, *mut u32) -> HRESULT;
    let _com = crate::win::com_init();
    let factory = factory()?;
    let mut list = null_mut();
    let hr = unsafe {
        factory.method::<CreateComponentEnumerator>(23)(factory.0, WIC_DECODER, ENUMERATE_DEFAULT, &mut list)
    };
    check(hr, "CreateComponentEnumerator")?;
    let list = Com(list);
    let mut found = Vec::new();
    loop {
        let (mut item, mut fetched) = (null_mut(), 0);
        // IEnumUnknown::Next: S_FALSE (1) at the end.
        if unsafe { list.method::<Next>(3)(list.0, 1, &mut item, &mut fetched) } != 0 || fetched == 0 {
            break;
        }
        let Some(info) = Com(item).query(&IID_IWIC_BITMAP_CODEC_INFO) else { continue };
        let get = unsafe { info.method::<GetFileExtensions>(17) };
        let mut len = 0;
        if unsafe { get(info.0, 0, null_mut(), &mut len) } < 0 || len == 0 {
            continue;
        }
        let mut text = vec![0u16; len as usize];
        if unsafe { get(info.0, len, text.as_mut_ptr(), &mut len) } < 0 {
            continue;
        }
        let text = String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())]);
        found.extend(text.split(',').map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()));
    }
    Ok(found)
}

/// Extensions (lowercase, without the dot) that go to Windows' codecs, not
/// to the `image` crate: those of the installed WIC decoders that are not
/// among `folder::EXTENSIONS`, and those of the file types qview registers
/// for them (`filetypes`, `wic: true`) even without their codec: the image
/// then says which extension Windows needs (`needs`). Found the first time
/// it is asked (`main` asks early, on a thread).
pub fn extensions() -> &'static [String] {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(|| {
        let started = std::time::Instant::now();
        let mut list = decoder_extensions().unwrap_or_else(|e| {
            log::warn!("cannot list the WIC codecs: {e}");
            Vec::new()
        });
        let registered = FILE_TYPES.iter().filter(|t| t.kind == crate::filetypes::Kind::Wic).flat_map(|t| t.extensions);
        list.extend(registered.map(|e| e.to_string()));
        list.retain(|e| !crate::folder::EXTENSIONS.contains(&e.as_str()));
        list.sort();
        list.dedup();
        log::debug!("WIC codecs listed in {:.1} ms: {}", started.elapsed().as_secs_f64() * 1e3, list.join(" "));
        list
    })
}

/// The extension of `path`, lowercase.
fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// Whether `path` goes to Windows' codecs rather than to the `image` crate
/// (see [`extensions`]). Camera RAW files are TIFF inside: `image` would
/// show the small preview most of them start with.
pub fn takes(path: &Path) -> bool {
    match extension(path) {
        Some(ext) if !crate::folder::EXTENSIONS.contains(&ext.as_str()) => extensions().contains(&ext),
        _ => false,
    }
}

/// The name of the format of `path` for the status bar.
pub fn format_name(path: &Path) -> &'static str {
    match extension(path).as_deref() {
        Some("heic" | "heif" | "hif") => "HEIF",
        Some("avif") => "AVIF",
        Some("jxr" | "wdp" | "hdp") => "JPEG XR",
        Some("dds") => "DDS",
        Some("dng") => "DNG",
        Some(ext) if RAW.contains(&ext) => "RAW",
        _ => "WIC",
    }
}

/// What Windows needs to decode `path`, if it is a format whose codec
/// comes from the Microsoft Store.
pub fn needs(path: &Path) -> Option<&'static str> {
    match extension(path).as_deref() {
        Some("heic" | "heif" | "hif") => Some(tr!(
            "HEIC needs the HEIF Image Extensions and the HEVC Video Extensions from the Microsoft Store",
            "Для HEIC нужны расширения HEIF Image Extensions и HEVC Video Extensions из Microsoft Store"
        )),
        Some("avif") => Some(tr!(
            "AVIF needs the AV1 Video Extension from the Microsoft Store",
            "Для AVIF нужно расширение AV1 Video Extension из Microsoft Store"
        )),
        Some(ext) if RAW.contains(&ext) => Some(tr!(
            "Camera RAW files need the Raw Image Extension from the Microsoft Store",
            "Для RAW-файлов нужно расширение Raw Image Extension из Microsoft Store"
        )),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qview_wic_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `jpeg` with an EXIF APP1 segment giving `orientation`.
    pub fn with_orientation(jpeg: &[u8], orientation: u8) -> Vec<u8> {
        let tiff = [
            b'I', b'I', 42, 0, 8, 0, 0, 0, // little-endian header, IFD at 8
            1, 0, // one entry
            0x12, 0x01, 3, 0, 1, 0, 0, 0, orientation, 0, 0, 0, // 0x0112 Orientation, SHORT, 1
            0, 0, 0, 0, // no next IFD
        ];
        let len = (2 + 6 + tiff.len()) as u16;
        let mut out = jpeg[..2].to_vec();
        out.extend([0xff, 0xe1]);
        out.extend(len.to_be_bytes());
        out.extend(b"Exif\0\0");
        out.extend(tiff);
        out.extend(&jpeg[2..]);
        out
    }

    #[test]
    fn decodes_through_windows() {
        let _com = crate::win::com_init();
        let dir = temp_dir("decode");
        // Red at half opacity, then opaque blue.
        let png = dir.join("a.png");
        let mut img = image::RgbaImage::new(3, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 128]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        img.save(&png).unwrap();
        let bgra = decode(&png, true).unwrap();
        assert_eq!((bgra.width, bgra.height, bgra.orientation, bgra.bits), (3, 2, 1, 32));
        assert_eq!(&bgra.pixels[..8], &[0, 0, 128, 128, 255, 0, 0, 255]);
        let rgba = decode(&png, false).unwrap();
        assert_eq!(&rgba.pixels[..8], &[255, 0, 0, 128, 0, 0, 255, 255]);
        assert_eq!(size(&png), Some((3, 2, 1)));

        // The EXIF orientation is reported, not applied.
        let mut jpeg = Vec::new();
        image::RgbImage::new(4, 2).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        let turned = dir.join("turned.jpg");
        std::fs::write(&turned, with_orientation(&jpeg, 6)).unwrap();
        let img = decode(&turned, true).unwrap();
        assert_eq!((img.width, img.height, img.orientation, img.bits), (4, 2, 6, 24));
        assert_eq!(size(&turned), Some((4, 2, 6)));

        let broken = dir.join("broken.heic");
        std::fs::write(&broken, b"not an image").unwrap();
        assert!(decode(&broken, true).is_err());
        assert!(decode(&dir.join("missing.png"), true).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The extensions of the installed decoders and how long listing them
    /// takes, printed with `--nocapture`.
    #[test]
    #[ignore]
    fn codec_list() {
        let started = std::time::Instant::now();
        let all = decoder_extensions().unwrap();
        println!("{} extensions in {:.1} ms: {}", all.len(), started.elapsed().as_secs_f64() * 1e3, all.join(" "));
        println!("taken by Windows' codecs: {}", extensions().join(" "));
    }

    /// Decode the file named by `QVIEW_WIC_FILE` through Windows' codecs
    /// and print its size, orientation and timings, with `--nocapture`.
    /// The first round loads the codec; from the second its instance kept
    /// by the thread (`keep_codec`) saves its slow release.
    #[test]
    #[ignore]
    fn wic_file() {
        let _com = crate::win::com_init();
        let path = std::path::PathBuf::from(std::env::var("QVIEW_WIC_FILE").expect("QVIEW_WIC_FILE"));
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
        for round in 0..3 {
            let t = std::time::Instant::now();
            let head = size(&path);
            let header = ms(t);
            let t = std::time::Instant::now();
            match decode(&path, true) {
                Ok(img) => println!(
                    "round {round}: header {head:?} in {header:.1} ms; {}x{}, orientation {}, {} bits, decoded in {:.1} ms",
                    img.width,
                    img.height,
                    img.orientation,
                    img.bits,
                    ms(t)
                ),
                Err(e) => println!("round {round}: header {head:?}; {e}"),
            }
        }
    }

    #[test]
    fn lists_the_codecs() {
        let all = decoder_extensions().unwrap();
        for ext in ["jpg", "png", "tif", "bmp"] {
            assert!(all.iter().any(|e| e == ext), "{ext} in {all:?}");
        }
        let taken = extensions();
        assert!(taken.iter().any(|e| e == "heic"), "{taken:?}");
        assert!(!taken.iter().any(|e| e == "jpg" || e == "png"), "{taken:?}");
        assert!(takes(Path::new(r"C:\a\IMG_0001.HEIC")));
        assert!(!takes(Path::new("a.jpg")));
        assert!(!takes(Path::new("a.txt")));
        assert_eq!(format_name(Path::new("a.NEF")), "RAW");
        assert!(needs(Path::new("a.heic")).is_some());
        assert!(needs(Path::new("a.jxr")).is_none());
    }
}
