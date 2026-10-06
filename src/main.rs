#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod anim;
mod app;
mod archive;
mod assoc;
mod clipboard;
mod crop;
mod edit;
mod editors;
mod exif;
mod favorites;
mod filter;
mod filetypes;
mod folder;
mod format;
mod gallery;
mod header;
mod heif;
mod history;
mod icon;
mod input;
mod instance;
mod loader;
mod print;
mod rename;
mod selection;
mod texture;
mod thumbs;
mod tree;
mod type_icon;
mod ui;
mod update;
mod view;
mod wallpaper;
mod wic;
mod win;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Version from Cargo.toml, shown in the About window; a development version
/// ("0.2.0-dev") carries the commit it was built from (`build.rs`).
pub const VERSION: &str = env!("QVIEW_VERSION");
/// eframe app id; also names the settings folder in `%APPDATA%`.
pub const APP_ID: &str = "qview";

/// When the process started, for the start-up timings in the log.
pub static START: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since start-up.
pub fn since_start_ms() -> f64 {
    START.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1e3)
}

/// The app icon rasterised at build time (`build.rs`) as straight RGBA,
/// `size` x `size` pixels for 64, 128 and 256. Drawing it at start-up took
/// 6 ms before the window could be created.
pub fn embedded_icon(size: u32) -> Vec<u8> {
    match size {
        64 => include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_64.rgba")).to_vec(),
        128 => include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_128.rgba")).to_vec(),
        _ => include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_256.rgba")).to_vec(),
    }
}

/// The saved window was maximized: it is created normal (see `main`), and
/// `App::new` maximizes it while it is cloaked.
pub static MAXIMIZE_WHEN_SHOWN: AtomicBool = AtomicBool::new(false);

/// The window's size on the first run, in points.
pub const DEFAULT_SIZE: [f32; 2] = [1024.0, 720.0];

fn main() -> eframe::Result {
    START.get_or_init(Instant::now);
    // QVIEW_TRACE=1 logs start-up and decoding times (to stderr).
    let mut log = env_logger::Builder::from_default_env();
    if std::env::var_os("QVIEW_TRACE").is_some() {
        log.filter_module("qview", log::LevelFilter::Debug);
    }
    log.init();
    log::debug!("main entered {:.0} ms after the process was created", win::ms_since_process_start().unwrap_or(0.0));
    i18n::set_lang(i18n::system_lang());
    install_panic_hook();

    // For the release script: write the app icon for the installer and exit.
    // `args_os`: `args` panics on a path that is not valid Unicode.
    let first = std::env::args_os().nth(1).and_then(|a| a.into_string().ok());
    if first.as_deref() == Some("--export-icon") {
        win::attach_parent_console();
        let Some(path) = std::env::args_os().nth(2) else {
            eprintln!("usage: qview --export-icon <file.ico>");
            std::process::exit(2);
        };
        if let Err(e) = std::fs::write(&path, icon::ico(&icon::ICO_SIZES)) {
            eprintln!("cannot write {}: {e}", path.to_string_lossy());
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    // For an installer: register or unregister the file types and exit.
    if let Some(flag @ ("--register" | "--unregister")) = first.as_deref() {
        let result = match flag {
            "--register" => std::env::current_exe().map_err(|e| e.to_string()).and_then(|e| assoc::register(&e)),
            _ => assoc::unregister(),
        };
        if let Err(e) = &result {
            win::error_box("qview", e);
        }
        std::process::exit(i32::from(result.is_err()));
    }

    let initial = std::env::args_os().nth(1).map(|a| normalize(&a.to_string_lossy()));
    // qview is already open: it shows the file instead.
    if instance::forward(initial.as_deref()) {
        return Ok(());
    }
    // Decoding starts now, while the window is being created.
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 3));
    let loader = loader::Loader::new(workers);
    if let Some(path) = initial.as_ref().filter(|p| p.is_file()) {
        loader.want([path.clone()]);
    }
    // Which formats Windows' codecs add, before the folder is listed.
    let _ = std::thread::Builder::new().name("codecs".into()).spawn(|| wic::extensions().len());

    let has_saved = eframe::storage_dir(APP_ID).is_some_and(|d| d.join("app.ron").is_file());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("qview")
            .with_inner_size(DEFAULT_SIZE)
            .with_min_inner_size([320.0, 240.0])
            .with_drag_and_drop(true)
            .with_icon(egui::IconData { rgba: embedded_icon(64), width: 64, height: 64 }),
        renderer: eframe::Renderer::Glow,
        // Centre only on the first run; later runs restore the saved window.
        centered: !has_saved,
        persist_window: true,
        // eframe creates the window hidden and shows it after the first
        // frame, but winit shows a window created maximized at once, which
        // flashes an empty white window. So a restored maximized window is
        // created normal, and `App::new` maximizes it cloaked, uncloaking it
        // once a maximized frame is painted. A window closed in full screen
        // (saved with the screen's size) opens maximized the same way: a
        // full-screen window would also be shown before it is painted.
        window_builder: Some(Box::new(|mut builder| {
            log::debug!("window builder at {:.0} ms", since_start_ms());
            if builder.fullscreen == Some(true) {
                builder.fullscreen = Some(false);
                builder.maximized = Some(true);
            }
            if builder.maximized == Some(true) {
                builder.maximized = Some(false);
                MAXIMIZE_WHEN_SHOWN.store(true, Ordering::Relaxed);
            }
            builder
        })),
        ..Default::default()
    };
    log::debug!("run_native at {:.0} ms", since_start_ms());
    eframe::run_native(APP_ID, options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, loader, initial)))))
}

/// The path given on the command line, absolute. Explorer passes a drive
/// root as `"C:\"`, which Windows argument parsing turns into `C:"`; the
/// trailing quote is turned back into a backslash. A bare `C:` means the
/// root, not the current directory of C.
fn normalize(arg: &str) -> PathBuf {
    let arg = match arg.strip_suffix('"') {
        Some(stripped) => format!("{}\\", stripped.trim_end_matches('\\')),
        None => arg.to_string(),
    };
    let b = arg.as_bytes();
    let path = if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        PathBuf::from(format!("{arg}\\"))
    } else {
        PathBuf::from(arg)
    };
    std::path::absolute(&path).unwrap_or(path)
}

/// The release build aborts on a panic and has no console, so the window
/// would vanish without a word: say what happened in a message box. The
/// default hook still prints it (seen in a debug build or with a console).
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let text = tr!(
            format!("qview stopped because of an internal error.\n\n{info}"),
            format!("qview остановлен из-за внутренней ошибки.\n\n{info}")
        );
        win::error_box("qview", &text);
    }));
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use std::path::PathBuf;

    #[test]
    fn normalizes_explorer_arguments() {
        assert_eq!(normalize("C:"), PathBuf::from(r"C:\"));
        assert_eq!(normalize("C:\""), PathBuf::from(r"C:\"));
        assert_eq!(normalize(r"D:\Photos\a.jpg"), PathBuf::from(r"D:\Photos\a.jpg"));
        assert_eq!(normalize(r"D:\Photos\.\a.jpg"), PathBuf::from(r"D:\Photos\a.jpg"));
        assert!(normalize("a.jpg").is_absolute());
    }
}
