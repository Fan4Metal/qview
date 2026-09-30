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
}

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
    },
];

/// Resource ID of the icon of `FILE_TYPES[i]` in the executable (the app
/// icon is 1).
pub const fn icon_id(i: usize) -> u16 {
    101 + i as u16
}
