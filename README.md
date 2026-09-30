# qview

English | [Русский](README.ru.md)

qview is a fast and simple image viewer for Windows. It opens an image in a fraction of a second, browses the other images of its folder without delays and has no settings dialog: the few options are in the menus.

## Features

- Start-up of about 0.2 s to the first image: decoding begins before the window is created and runs on background threads.
- Instant browsing: the neighbours of the current image are decoded in advance.
- Images are listed in the same order as in Explorer (numbers are compared as numbers).
- Downscaled images are smoothed with GPU mipmaps; 100% shows one image pixel per screen pixel at any Windows display scaling.
- The EXIF orientation of photos is applied.
- The status bar shows the position in the folder, the file name, size, dimensions, colour depth, format, modification date and zoom.
- Deletion moves the file to the Recycle Bin after a confirmation.
- The background of the image area is chosen in View → Background: dark (the default), black, grey, white or any other colour.
- The interface is in Russian when Windows is in Russian, and in English otherwise.

Supported formats: JPEG, PNG, GIF (first frame), WebP, BMP, TIFF, ICO, TGA, QOI, PNM (PBM, PGM, PPM, PAM).

## Controls

| Action | Keys and mouse |
|---|---|
| Next image | `→`, `Page Down`, `Space`, `Ctrl+→`, wheel down |
| Previous image | `←`, `Page Up`, `Backspace`, `Ctrl+←`, wheel up |
| First / last image | `Home` / `End` |
| Zoom in / out | `+`, `=`, `-`; `Ctrl+wheel` zooms at the pointer |
| Fit to window (shrink only) | numpad `*` |
| Actual size (100%) | numpad `/` |
| Scroll a zoomed image | arrow keys, dragging with the left button |
| Rotate left / right (view only) | `[` / `]`, `Ctrl+Alt+←` / `Ctrl+Alt+→` |
| Full screen | `F`, `Ctrl+Shift+F`, double click, middle click |
| Leave full screen / close | `Esc`; `Ctrl+W` and `Alt+F4` close |
| Move to the Recycle Bin | `Delete` (`Enter` confirms, `Esc` cancels) |
| Copy the file to the clipboard | `Ctrl+C` |
| Open a file | `Ctrl+O`, or dropping a file onto the window |
| Open with the default program | `Shift+E` |
| Reload the image and the folder | `F5` |
| Show or hide the toolbar / status bar | `T` / `B` |
| List of shortcuts | `F1` |

The arrow keys scroll an image that is larger than the window in that direction; otherwise they browse. Letter keys also work with the Russian keyboard layout. Double click switches to full screen.

## Usage

```
qview.exe [FILE | FOLDER]
```

A file is shown together with the other images of its folder; a folder is opened at its first image.

## File associations

File → File Associations… registers qview with Windows for the current user (no administrator rights are needed): each supported format gets its own file type with an icon, qview appears in "Open with" and in Settings → Default apps. Each icon is a page with a band in the format's colour showing the extension.

Windows lets only the user choose the default program, so registration offers qview but does not take over the file types by itself. The "Choose as Default…" button opens Settings, where qview is selected under "Set defaults by app"; Windows also offers qview the next time an image is opened. The dialog shows which types qview opens by default, and "Unregister" removes everything the registration wrote. The same can be done from the command line, for example by an installer:

```
qview.exe --register
qview.exe --unregister
```

The registration refers to the location of `qview.exe`; after the program is moved, it is registered again.

The window position and size, the visibility of the toolbar and the status bar, and the background colour are kept in `%APPDATA%\qview\data\app.ron`.

## Building

Requirements: Windows 10 or later, stable Rust with the MSVC toolchain, Visual Studio 2022 Build Tools and [uv](https://docs.astral.sh/uv/).

```
uv run build.py          # target\release\qview.exe
cargo test
```

`build.py` first closes a `qview.exe` running from the project's `target` folder, which would otherwise lock the file, and then runs `cargo build --release`; extra arguments are passed to cargo. Its only dependency, `psutil`, is installed by uv. When qview is not running, plain `cargo build --release` works as well.

The window is drawn by egui/eframe through OpenGL (glow), images are decoded by the `image` crate. Setting the environment variable `QVIEW_TRACE=1` writes start-up and decoding times to stderr.

## License

MIT, see [LICENSE](LICENSE).
