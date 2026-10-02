//! The image file types qview registers in Windows: one ProgID per format,
//! its extensions, its name in Explorer's Type column and the colours of
//! its icon. Dependency-free: `build.rs` includes this file (with
//! `type_icon.rs`) to draw the icons embedded in the executable.

// build.rs uses the table only for the icons.
#![allow(dead_code)]

pub struct FileType {
    /// ProgID suffix: the type is registered as `qview.<id>`.
    pub id: &'static str,
    /// Text on the icon.
    pub label: &'static str,
    /// Extensions, lowercase, without the dot.
    pub extensions: &'static [&'static str],
    /// Name in Explorer's Type column.
    pub name_en: &'static str,
    pub name_ru: &'static str,
    /// Colour of the icon's label band and of its letters, from the classic
    /// 16-colour palette.
    pub band: [u8; 3],
    pub ink: [u8; 3],
    /// Decoded by Windows' codecs (`wic`), not by the `image` crate.
    pub wic: bool,
}

/// Camera RAW formats, the most common first (the first is the one whose
/// default program the associations dialog shows).
pub const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2", "pef", "srw", "3fr", "ari", "bay", "cap", "crw", "dcr",
    "dcs", "drf", "eip", "erf", "fff", "iiq", "k25", "kdc", "mef", "mos", "mrw", "nrw", "ori", "ptx", "raw", "rwl",
    "sr2", "srf", "x3f",
];

pub const FILE_TYPES: &[FileType] = &[
    FileType {
        id: "jpeg",
        label: "JPG",
        extensions: &["jpg", "jpeg", "jpe", "jfif"],
        name_en: "JPEG Image",
        name_ru: "Изображение JPEG",
        // olive, yellow letters
        band: [0x5c, 0x64, 0x10],
        ink: [0xf0, 0xe8, 0x60],
        wic: false,
    },
    FileType {
        id: "png",
        label: "PNG",
        extensions: &["png"],
        name_en: "PNG Image",
        name_ru: "Изображение PNG",
        // maroon, pink letters
        band: [0x80, 0x04, 0x06],
        ink: [0xfc, 0xd9, 0xe4],
        wic: false,
    },
    FileType {
        id: "gif",
        label: "GIF",
        extensions: &["gif"],
        name_en: "GIF Image",
        name_ru: "Изображение GIF",
        // green
        band: [0x04, 0x86, 0x04],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "bmp",
        label: "BMP",
        extensions: &["bmp", "dib"],
        name_en: "Bitmap Image",
        name_ru: "Точечный рисунок",
        // blue
        band: [0x04, 0x04, 0xe0],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "tiff",
        label: "TIF",
        extensions: &["tif", "tiff"],
        name_en: "TIFF Image",
        name_ru: "Изображение TIFF",
        // navy
        band: [0x00, 0x00, 0x80],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "webp",
        label: "WEBP",
        extensions: &["webp"],
        name_en: "WebP Image",
        name_ru: "Изображение WebP",
        // teal
        band: [0x00, 0x80, 0x80],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "ico",
        label: "ICO",
        extensions: &["ico"],
        name_en: "Icon",
        name_ru: "Значок",
        // purple
        band: [0x80, 0x00, 0x80],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "tga",
        label: "TGA",
        extensions: &["tga"],
        name_en: "Targa Image",
        name_ru: "Изображение Targa",
        // grey
        band: [0x50, 0x50, 0x50],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "qoi",
        label: "QOI",
        extensions: &["qoi"],
        name_en: "QOI Image",
        name_ru: "Изображение QOI",
        // orange
        band: [0xc0, 0x60, 0x00],
        ink: [0xff, 0xff, 0xff],
        wic: false,
    },
    FileType {
        id: "pnm",
        label: "PNM",
        extensions: &["pnm", "pbm", "pgm", "ppm", "pam"],
        name_en: "Netpbm Image",
        name_ru: "Изображение Netpbm",
        // light grey
        band: [0xd8, 0xd8, 0xd8],
        ink: [0x30, 0x30, 0x30],
        wic: false,
    },
    FileType {
        id: "heif",
        label: "HEIC",
        extensions: &["heic", "heif", "hif"],
        name_en: "HEIF Image",
        name_ru: "Изображение HEIF",
        // black
        band: [0x20, 0x20, 0x20],
        ink: [0xff, 0xff, 0xff],
        wic: true,
    },
    FileType {
        id: "avif",
        label: "AVIF",
        extensions: &["avif"],
        name_en: "AVIF Image",
        name_ru: "Изображение AVIF",
        // fuchsia
        band: [0xc0, 0x00, 0xc0],
        ink: [0xff, 0xff, 0xff],
        wic: true,
    },
    FileType {
        id: "raw",
        label: "RAW",
        extensions: RAW_EXTENSIONS,
        name_en: "Camera RAW Image",
        name_ru: "RAW-изображение",
        // yellow, black letters
        band: [0xe8, 0xc8, 0x00],
        ink: [0x20, 0x20, 0x20],
        wic: true,
    },
    FileType {
        id: "jxr",
        label: "JXR",
        extensions: &["jxr", "wdp", "hdp"],
        name_en: "JPEG XR Image",
        name_ru: "Изображение JPEG XR",
        // aqua, dark letters
        band: [0x00, 0xc0, 0xe0],
        ink: [0x10, 0x20, 0x40],
        wic: true,
    },
];

/// Resource ID of the icon of `FILE_TYPES[i]` in the executable (the app
/// icon is 1).
pub const fn icon_id(i: usize) -> u16 {
    101 + i as u16
}
