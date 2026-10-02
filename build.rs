//! Generates the application icon and embeds it (plus version info) into the
//! Windows executable.

#[path = "src/icon.rs"]
mod icon;
#[path = "src/filetypes.rs"]
mod filetypes;
#[path = "src/type_icon.rs"]
mod type_icon;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=src/filetypes.rs");
    println!("cargo:rerun-if-changed=src/type_icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=QVIEW_VERSION={}", version());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico_path = out_dir.join("app.ico");
    std::fs::write(&ico_path, icon::ico(&icon::ICO_SIZES)).expect("write app.ico");
    // The icon as raw RGBA for the window icon and the start screen (see
    // `main::embedded_icon`), so that nothing is rasterised at start-up.
    for size in [64u32, 128, 256] {
        std::fs::write(out_dir.join(format!("app_icon_{size}.rgba")), icon::rgba(size)).expect("write an icon");
    }

    let mut res = winresource::WindowsResource::new();
    // The icons of the registered file types, referenced by the registry as
    // `"qview.exe",-<id>`. The app icon keeps ID 1, the lowest, which
    // Explorer shows for the executable.
    for (i, t) in filetypes::FILE_TYPES.iter().enumerate() {
        let path = out_dir.join(format!("type_{}.ico", t.id));
        std::fs::write(&path, type_icon::ico(t, &icon::ICO_SIZES)).expect("write a file type icon");
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

/// The version shown in the UI (`main::VERSION`): Cargo's, plus the commit
/// for a development version ("0.2.0-dev (v0.1.0-2-g4bd494d)"), so that builds
/// between releases can be told apart. A release version is shown as is.
fn version() -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    if !version.contains('-') {
        return version;
    }
    // `--always` falls back to the abbreviated hash when no tag is reachable
    // (a shallow checkout fetches none). No `--dirty`: this script is not rerun
    // when only sources change, so the mark would be stale.
    match git(&["describe", "--tags", "--always"]) {
        Some(describe) => {
            watch_git();
            format!("{version} ({describe})")
        }
        None => version,
    }
}

/// Reruns this script when a commit is made, the branch is switched or a tag
/// is added: HEAD, the branch it points to, packed refs and the tags.
fn watch_git() {
    let Some(dirs) = git(&["rev-parse", "--git-dir", "--git-common-dir"]) else { return };
    let mut dirs = dirs.lines().map(std::path::PathBuf::from);
    let (Some(git_dir), Some(common)) = (dirs.next(), dirs.next()) else { return };
    let head = git_dir.join("HEAD");
    let mut paths = vec![head.clone(), common.join("packed-refs"), common.join("refs").join("tags")];
    if let Some(branch) = std::fs::read_to_string(&head).ok().and_then(|h| h.strip_prefix("ref: ").map(|r| r.trim().to_owned())) {
        paths.push(common.join(branch));
    }
    // A path that does not exist would make Cargo rerun the script every time.
    for path in paths.iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Output of a git command, trimmed; `None` when git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}
