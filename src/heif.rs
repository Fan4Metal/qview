//! HEIC/HEIF through libheif with its built-in HEVC decoder (libde265),
//! loaded at run time from `heif.dll` next to qview.exe (with `libde265.dll`
//! beside it; `tools/build_heif.py` builds both). Without them HEIF goes to
//! Windows' codecs (`wic`) as before; with them it opens without the HEIF
//! and HEVC extensions from the Microsoft Store, and a file libheif cannot
//! read still goes to Windows.
//!
//! Both libraries are LGPL: they stay separate DLLs, called through the C
//! API declared here by hand (`heif.h` of libheif 1.23).
//!
//! libheif applies the container's rotation and mirroring itself, so the
//! orientation reported is always 1, as with Windows' HEIF decoder.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::OnceLock;

use crate::wic::Image;

/// `heif_error`, returned by value.
#[repr(C)]
#[derive(Clone, Copy)]
struct Error {
    code: c_int,
    subcode: c_int,
    message: *const c_char,
}

type Context = *mut c_void;
type Handle = *mut c_void;
type HeifImage = *mut c_void;

const COLORSPACE_RGB: c_int = 1;
const CHROMA_INTERLEAVED_RGBA: c_int = 11;
const CHANNEL_INTERLEAVED: c_int = 10;

/// `heif_reader`, version 1: libheif reads the file through it, only the
/// boxes it needs.
#[repr(C)]
struct Reader {
    version: c_int,
    get_position: unsafe extern "C" fn(*mut c_void) -> i64,
    read: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> c_int,
    seek: unsafe extern "C" fn(i64, *mut c_void) -> c_int,
    wait_for_file_size: unsafe extern "C" fn(i64, *mut c_void) -> c_int,
}

/// The functions of heif.dll qview calls.
struct Lib {
    context_alloc: unsafe extern "C" fn() -> Context,
    context_free: unsafe extern "C" fn(Context),
    max_threads: unsafe extern "C" fn(Context, c_int),
    read_from_memory: unsafe extern "C" fn(Context, *const c_void, usize, *const c_void) -> Error,
    read_from_reader: unsafe extern "C" fn(Context, *const Reader, *mut c_void, *const c_void) -> Error,
    primary_handle: unsafe extern "C" fn(Context, *mut Handle) -> Error,
    handle_release: unsafe extern "C" fn(Handle),
    handle_width: unsafe extern "C" fn(Handle) -> c_int,
    handle_height: unsafe extern "C" fn(Handle) -> c_int,
    has_alpha: unsafe extern "C" fn(Handle) -> c_int,
    luma_bits: unsafe extern "C" fn(Handle) -> c_int,
    decode_image: unsafe extern "C" fn(Handle, *mut HeifImage, c_int, c_int, *const c_void) -> Error,
    image_width: unsafe extern "C" fn(HeifImage, c_int) -> c_int,
    image_height: unsafe extern "C" fn(HeifImage, c_int) -> c_int,
    plane: unsafe extern "C" fn(HeifImage, c_int, *mut usize) -> *const u8,
    image_release: unsafe extern "C" fn(HeifImage),
    metadata_ids: unsafe extern "C" fn(Handle, *const c_char, *mut u32, c_int) -> c_int,
    metadata_size: unsafe extern "C" fn(Handle, u32) -> usize,
    metadata: unsafe extern "C" fn(Handle, u32, *mut c_void) -> Error,
    icc_size: unsafe extern "C" fn(Handle) -> usize,
    icc: unsafe extern "C" fn(Handle, *mut c_void) -> Error,
}

/// Where heif.dll is looked for: next to the program, and for the tests
/// where `tools/build_heif.py` puts it.
fn dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).into_iter().collect();
    if cfg!(test) {
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("heif").join("bin"));
    }
    dirs
}

/// heif.dll, loaded the first time it is needed; `None` if it is not there.
fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(|| {
        let started = std::time::Instant::now();
        let lib = dirs().iter().find_map(|dir| load(&dir.join("heif.dll")));
        if lib.is_some() {
            log::debug!("libheif loaded in {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
        }
        lib
    })
    .as_ref()
}

// Each transmute's type is that of the field it fills.
#[allow(clippy::missing_transmute_annotations)]
fn load(path: &Path) -> Option<Lib> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW};
    if !path.is_file() {
        return None;
    }
    // libde265.dll is found next to heif.dll.
    let module = unsafe { LoadLibraryExW(crate::win::wide(path).as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
    if module.is_null() {
        log::warn!("cannot load {}: {}", path.display(), std::io::Error::last_os_error());
        return None;
    }
    macro_rules! get {
        ($name:literal) => {{
            let Some(f) = (unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) }) else {
                log::warn!("{} has no {}", path.display(), $name);
                return None;
            };
            // SAFETY: the type is the field's, declared from heif.h.
            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(f) }
        }};
    }
    let init: unsafe extern "C" fn(*const c_void) -> Error = get!("heif_init");
    let lib = Lib {
        context_alloc: get!("heif_context_alloc"),
        context_free: get!("heif_context_free"),
        max_threads: get!("heif_context_set_max_decoding_threads"),
        read_from_memory: get!("heif_context_read_from_memory_without_copy"),
        read_from_reader: get!("heif_context_read_from_reader"),
        primary_handle: get!("heif_context_get_primary_image_handle"),
        handle_release: get!("heif_image_handle_release"),
        handle_width: get!("heif_image_handle_get_width"),
        handle_height: get!("heif_image_handle_get_height"),
        has_alpha: get!("heif_image_handle_has_alpha_channel"),
        luma_bits: get!("heif_image_handle_get_luma_bits_per_pixel"),
        decode_image: get!("heif_decode_image"),
        image_width: get!("heif_image_get_width"),
        image_height: get!("heif_image_get_height"),
        plane: get!("heif_image_get_plane_readonly2"),
        image_release: get!("heif_image_release"),
        metadata_ids: get!("heif_image_handle_get_list_of_metadata_block_IDs"),
        metadata_size: get!("heif_image_handle_get_metadata_size"),
        metadata: get!("heif_image_handle_get_metadata"),
        icc_size: get!("heif_image_handle_get_raw_color_profile_size"),
        icc: get!("heif_image_handle_get_raw_color_profile"),
    };
    // Never matched by heif_deinit: the library stays for the process.
    check(unsafe { init(std::ptr::null()) }).ok()?;
    Some(lib)
}

fn check(e: Error) -> Result<(), String> {
    if e.code == 0 {
        return Ok(());
    }
    let message = if e.message.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(e.message) }.to_string_lossy().into_owned()
    };
    Err(format!("libheif: {message} ({}.{})", e.code, e.subcode))
}

/// Whether heif.dll is there to read `path`: a HEIF file by its extension.
pub fn takes(path: &Path) -> bool {
    let heif = path.extension().is_some_and(|e| ["heic", "heif", "hif"].iter().any(|x| e.eq_ignore_ascii_case(x)));
    heif && lib().is_some()
}

/// A libheif context with the primary image's handle, freed when dropped.
struct Primary {
    lib: &'static Lib,
    context: Context,
    handle: Handle,
}

impl Primary {
    /// `read` fills the context (from memory or through a reader).
    fn new(lib: &'static Lib, read: impl FnOnce(Context) -> Error) -> Result<Self, String> {
        let context = unsafe { (lib.context_alloc)() };
        if context.is_null() {
            return Err("libheif: no context".into());
        }
        let mut primary = Primary { lib, context, handle: null_mut() };
        // The tiles of a photo decode in parallel: 4 threads by default,
        // 8 took a 12 MP iPhone photo from ~120 to ~95 ms, more gained
        // nothing.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        unsafe { (lib.max_threads)(context, threads as c_int) };
        check(read(context))?;
        check(unsafe { (lib.primary_handle)(context, &mut primary.handle) })?;
        Ok(primary)
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_null() {
                (self.lib.handle_release)(self.handle);
            }
            (self.lib.context_free)(self.context);
        }
    }
}

/// Decode the primary image of `path`, upright: premultiplied BGRA with
/// `bgra`, otherwise straight RGBA, as `wic::decode`. `None` if heif.dll
/// does not take it (see [`takes`]).
pub fn decode(path: &Path, bgra: bool) -> Option<Result<Image, String>> {
    if !takes(path) {
        return None;
    }
    let lib = lib()?;
    Some(decode_with(lib, path, bgra))
}

fn decode_with(lib: &'static Lib, path: &Path, bgra: bool) -> Result<Image, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let primary = Primary::new(lib, |context| unsafe {
        (lib.read_from_memory)(context, bytes.as_ptr().cast(), bytes.len(), std::ptr::null())
    })?;
    let alpha = unsafe { (lib.has_alpha)(primary.handle) } != 0;
    let luma = unsafe { (lib.luma_bits)(primary.handle) }.max(8) as u16;
    // libheif's own limit is a gigapixel; its RGBA plane and the copy here
    // would take 8 GB. The `image` crate's limit holds for it too.
    let (w, h) = unsafe { ((lib.handle_width)(primary.handle), (lib.handle_height)(primary.handle)) };
    if w.max(0) as u64 * h.max(0) as u64 * 4 > crate::loader::MAX_ALLOC {
        return Err(format!("libheif: the image is too large ({w}x{h})"));
    }
    let mut decoded = null_mut();
    let e = unsafe {
        (lib.decode_image)(primary.handle, &mut decoded, COLORSPACE_RGB, CHROMA_INTERLEAVED_RGBA, std::ptr::null())
    };
    struct Decoded(&'static Lib, HeifImage);
    impl Drop for Decoded {
        fn drop(&mut self) {
            if !self.1.is_null() {
                unsafe { (self.0.image_release)(self.1) };
            }
        }
    }
    let decoded = Decoded(lib, decoded);
    check(e)?;
    let width = unsafe { (lib.image_width)(decoded.1, CHANNEL_INTERLEAVED) };
    let height = unsafe { (lib.image_height)(decoded.1, CHANNEL_INTERLEAVED) };
    let mut stride = 0usize;
    let plane = unsafe { (lib.plane)(decoded.1, CHANNEL_INTERLEAVED, &mut stride) };
    let row = width.max(0) as usize * 4;
    if width <= 0 || height <= 0 || plane.is_null() || stride < row {
        return Err(format!("libheif: unexpected image {width}x{height}, stride {stride}"));
    }
    let height = height as usize;
    let mut pixels = vec![0u8; row * height];
    for (y, out) in pixels.chunks_exact_mut(row).enumerate() {
        let src = unsafe { std::slice::from_raw_parts(plane.add(y * stride), row) };
        if bgra {
            to_premultiplied_bgra(src, out, alpha);
        } else {
            out.copy_from_slice(src);
        }
    }
    let bits = luma * if alpha { 4 } else { 3 };
    Ok(Image { width: width as u32, height: height as u32, pixels, orientation: 1, bits })
}

/// Straight RGBA `src` to premultiplied BGRA in `out`, of the same length.
fn to_premultiplied_bgra(src: &[u8], out: &mut [u8], alpha: bool) {
    for (s, o) in src.as_chunks::<4>().0.iter().zip(out.as_chunks_mut::<4>().0) {
        let a = s[3];
        *o = if !alpha || a == 255 {
            [s[2], s[1], s[0], a]
        } else {
            let m = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            [m(s[2]), m(s[1]), m(s[0]), a]
        };
    }
}

/// The upright size of the primary image of `path`, reading only the boxes
/// that describe it. `None` if heif.dll does not take it or cannot read it.
pub fn size(path: &Path) -> Option<(u32, u32)> {
    if !takes(path) {
        return None;
    }
    let lib = lib()?;
    with_primary(lib, path, |primary| {
        let width = unsafe { (lib.handle_width)(primary.handle) };
        let height = unsafe { (lib.handle_height)(primary.handle) };
        (width > 0 && height > 0).then_some((width as u32, height as u32))
    })
}

/// The EXIF (its TIFF data, without the offset in front of it) and the ICC
/// profile of the primary image of a HEIF or AVIF file (`info`), reading
/// only the boxes they are in. None if heif.dll is not there or cannot
/// read the file.
pub fn metadata(path: &Path) -> Option<crate::info::Raw> {
    const MAX: usize = 16 << 20;
    let lib = lib()?;
    with_primary(lib, path, |primary| {
        let handle = primary.handle;
        let mut ids = [0u32; 4];
        let n = unsafe { (lib.metadata_ids)(handle, c"Exif".as_ptr(), ids.as_mut_ptr(), ids.len() as c_int) };
        let exif = ids[..n.clamp(0, 4) as usize].iter().find_map(|&id| {
            let size = unsafe { (lib.metadata_size)(handle, id) };
            if !(8..MAX).contains(&size) {
                return None;
            }
            let mut data = vec![0u8; size];
            check(unsafe { (lib.metadata)(handle, id, data.as_mut_ptr().cast()) }).ok()?;
            // The offset of the TIFF header from the end of this number.
            let offset = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
            data.get(4usize.checked_add(offset)?..).filter(|t| t.len() >= 8).map(<[u8]>::to_vec)
        });
        let size = unsafe { (lib.icc_size)(handle) };
        let icc = (1..MAX).contains(&size).then(|| {
            let mut data = vec![0u8; size];
            check(unsafe { (lib.icc)(handle, data.as_mut_ptr().cast()) }).ok().map(|()| data)
        });
        Some((exif, icc.flatten()))
    })
}

/// `f` of the primary image of `path`, which libheif reads through a
/// buffered reader: only the boxes it needs.
fn with_primary<T>(lib: &'static Lib, path: &Path, f: impl FnOnce(&Primary) -> Option<T>) -> Option<T> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len() as i64;
    // libheif reads box by box, a few bytes at a time.
    let mut state: ReaderState = (BufReader::with_capacity(64 * 1024, file), len);
    let reader = Reader { version: 1, get_position, read, seek, wait_for_file_size };
    let primary = Primary::new(lib, |context| unsafe {
        (lib.read_from_reader)(context, &reader, (&raw mut state).cast(), std::ptr::null())
    })
    .ok()?;
    f(&primary)
}

/// The reader's state: the file and its length.
type ReaderState = (BufReader<File>, i64);

unsafe extern "C" fn get_position(state: *mut c_void) -> i64 {
    let (file, _) = unsafe { &mut *state.cast::<ReaderState>() };
    file.stream_position().map_or(-1, |p| p as i64)
}

unsafe extern "C" fn read(data: *mut c_void, size: usize, state: *mut c_void) -> c_int {
    let (file, _) = unsafe { &mut *state.cast::<ReaderState>() };
    let out = unsafe { std::slice::from_raw_parts_mut(data.cast::<u8>(), size) };
    if file.read_exact(out).is_ok() { 0 } else { 1 }
}

unsafe extern "C" fn seek(position: i64, state: *mut c_void) -> c_int {
    let (file, _) = unsafe { &mut *state.cast::<ReaderState>() };
    // Relative, so that a seek within the buffer keeps it.
    let Ok(current) = file.stream_position() else { return 1 };
    if position >= 0 && file.seek_relative(position - current as i64).is_ok() { 0 } else { 1 }
}

/// `heif_reader_grow_status`: 0 size reached, 2 beyond the end of the file.
unsafe extern "C" fn wait_for_file_size(target: i64, state: *mut c_void) -> c_int {
    let (_, len) = unsafe { &*state.cast::<ReaderState>() };
    if target <= *len { 0 } else { 2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/heif/src/libheif/examples/example.heic")
    }

    #[test]
    fn premultiplies() {
        let mut out = [0; 12];
        to_premultiplied_bgra(&[200, 100, 50, 255, 200, 100, 50, 128, 10, 20, 30, 0], &mut out, true);
        assert_eq!(out, [50, 100, 200, 255, 25, 50, 100, 128, 0, 0, 0, 0]);
        let mut out = [0; 4];
        to_premultiplied_bgra(&[1, 2, 3, 255], &mut out, false);
        assert_eq!(out, [3, 2, 1, 255]);
    }

    /// Needs `python tools/build_heif.py`; passes without it.
    #[test]
    fn decodes_through_libheif() {
        let path = example();
        if lib().is_none() || !path.exists() {
            eprintln!("heif.dll or {} missing: run tools/build_heif.py", path.display());
            return;
        }
        let (w, h) = size(&path).expect("size");
        let image = decode(&path, true).expect("taken").expect("decoded");
        assert_eq!((image.width, image.height), (w, h));
        assert_eq!(image.pixels.len(), (w * h * 4) as usize);
        assert!(image.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // Not a HEIF file: an error, not a crash.
        let dir = std::env::temp_dir().join(format!("qview_heif_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let broken = dir.join("broken.heic");
        std::fs::write(&broken, b"not an image").unwrap();
        assert!(decode(&broken, true).unwrap().is_err());
        assert!(size(&broken).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `$env:QVIEW_HEIF_FILE="<file>"; cargo test --release heif_file -- --ignored --nocapture`:
    /// libheif against Windows' codec on one file.
    #[test]
    #[ignore]
    fn heif_file() {
        let path = PathBuf::from(std::env::var("QVIEW_HEIF_FILE").expect("QVIEW_HEIF_FILE"));
        let _com = crate::win::com_init();
        for round in 0..3 {
            let t = std::time::Instant::now();
            let size = size(&path);
            let header = t.elapsed();
            let t = std::time::Instant::now();
            let image = decode(&path, true).expect("taken");
            let decoded = t.elapsed();
            let t = std::time::Instant::now();
            let wic = crate::wic::decode(&path, true);
            let windows = t.elapsed();
            println!(
                "round {round}: libheif header {:.1} ms {size:?}, decode {:.1} ms {:?}; WIC {:.1} ms {:?}",
                header.as_secs_f64() * 1e3,
                decoded.as_secs_f64() * 1e3,
                image.as_ref().map(|i| (i.width, i.height, i.bits)),
                windows.as_secs_f64() * 1e3,
                wic.as_ref().map(|i| (i.width, i.height, i.orientation)),
            );
            if round == 0
                && let (Ok(a), Ok(b)) = (&image, &wic)
                && a.pixels.len() == b.pixels.len()
            {
                let diffs = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y) as u64);
                let (sum, max) = diffs.fold((0, 0), |(s, m), d| (s + d, m.max(d)));
                println!("against WIC: mean difference {:.2}, largest {max}", sum as f64 / a.pixels.len() as f64);
            }
        }
    }

    /// libheif's fuzzing corpus: every file read or refused, none crashes.
    #[test]
    #[ignore]
    fn fuzz_corpus() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/heif/src/libheif/fuzzing/data/corpus");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if takes(&path) {
                let result = decode(&path, true).unwrap();
                println!("{}: {:?} {:?}", path.display(), size(&path), result.map(|i| (i.width, i.height)));
            }
        }
    }
}
