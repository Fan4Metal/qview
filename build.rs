//! Generates the application icon and embeds it (plus version info) into the
//! Windows executable.

#[path = "src/icon.rs"]
mod icon;
#[path = "src/filetypes.rs"]
mod filetypes;
#[path = "src/type_icon.rs"]
mod type_icon;

/// Sizes in every `.ico`.
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=src/filetypes.rs");
    println!("cargo:rerun-if-changed=src/type_icon.rs");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico_path = out_dir.join("app.ico");
    std::fs::write(&ico_path, icon::ico(&SIZES)).expect("write app.ico");

    let mut res = winresource::WindowsResource::new();
    // The icons of the registered file types, referenced by the registry as
    // `"qview.exe",-<id>`. The app icon keeps ID 1, the lowest, which
    // Explorer shows for the executable.
    for (i, t) in filetypes::FILE_TYPES.iter().enumerate() {
        let path = out_dir.join(format!("type_{}.ico", t.id));
        std::fs::write(&path, type_icon::ico(t, &SIZES)).expect("write a file type icon");
        res.set_icon_with_id(path.to_str().unwrap(), &filetypes::icon_id(i).to_string());
    }
    res.set_icon(ico_path.to_str().unwrap())
        .set("FileDescription", "qview - image viewer")
        .set("ProductName", "qview")
        .set("LegalCopyright", "Copyright (C) 2026 Fan4_Metal");
    if let Err(e) = res.compile() {
        // A missing resource compiler must not break the build.
        println!("cargo:warning=icon not embedded: {e}");
    }
}
