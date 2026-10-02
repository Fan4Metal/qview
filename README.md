<p align="center">
  <img src="images/icon.png" width="128" height="128" alt="qview icon">
</p>

<h1 align="center">qview</h1>

<p align="center">A fast and simple image viewer for Windows</p>

<p align="center">
  <a href="https://github.com/Fan4Metal/qview/releases/latest"><img src="https://img.shields.io/github/v/release/Fan4Metal/qview?label=release" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/Windows-10%2B-0078D6" alt="Windows 10 or later">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/Fan4Metal/qview?label=license" alt="MIT license"></a>
</p>

<p align="center"><b>English</b> | <a href="README.ru.md">Русский</a></p>

qview is a fast and simple image viewer for Windows. It opens an image in a fraction of a second, browses the other images of its folder without delays and has no settings dialog: the few options are in the menus.

![qview showing a photo with its context menu open, under the menu bar and the toolbar, above the status bar](images/screenshot.png)

## Features

- Start-up of about 0.2 s to the first image: decoding begins before the window is created and runs on background threads.
- Instant browsing: the neighbours of the current image are decoded in advance.
- Images are listed in the same order as in Explorer (numbers are compared as numbers); View → Sort orders them by date modified or size instead, ascending or descending.
- The gallery shows the folder tree and the images of a folder as thumbnails (see [Gallery](#gallery)).
- Downscaled images are smoothed with mipmaps; 100% shows one image pixel per screen pixel at any Windows display scaling.
- The EXIF orientation of photos is applied.
- The status bar shows the position in the folder, the file name, size, dimensions, colour depth, format, modification date and zoom.
- Deletion moves the file to the Recycle Bin after a confirmation.
- The background of the image area is chosen in View → Background: dark (the default), black, grey, white or any other colour.
- The interface is in Russian when Windows is in Russian, and in English otherwise; another language is chosen in Help → About qview.

Supported formats: JPEG, PNG, GIF, WebP, BMP, TIFF, ICO, TGA, QOI, PNM (PBM, PGM, PPM, PAM). In addition, every format for which Windows has a codec (Windows Imaging Component) is opened through it: HEIC/HEIF (with the HEIF Image Extensions and the HEVC Video Extensions from the Microsoft Store), AVIF (with the AV1 Video Extension), camera RAW files such as CR2, CR3, NEF, ARW and DNG (with the Raw Image Extension, included in Windows 11), JPEG XR, DDS and the formats of other installed codecs. HEIC, HEIF and AVIF files are always listed; when the extension they need is missing, the image area names it. Windows' codecs also take over the files of the formats above that qview's own decoders cannot read. Animated GIF and WebP images are played in the viewer, in a loop or as many times as the file specifies; the gallery shows their first frame, and the status bar marks them as animated.

## Installation

The installer and the portable archive are published on the [Releases](https://github.com/Fan4Metal/qview/releases) page.

The installer, `qview_<version>_Setup.exe`, needs no administrator rights: the program is placed in `%LOCALAPPDATA%\Programs\qview`. The "Register qview for image files" option, selected by default, registers the supported file types (see [File associations](#file-associations)); the last page of the installer then offers to open Windows Settings, where qview is chosen as the default program. Uninstallation removes the registration.

The portable archive, `qview_<version>_portable.zip`, contains the program in a `qview` folder and runs without installation; the file types are registered from File → File Associations… if needed. In both cases the settings are kept in `%APPDATA%\qview`.

## Controls

| Action | Keys and mouse |
|---|---|
| Next image | `→`, `Page Down`, `Space`, `Ctrl+→`, wheel down |
| Previous image | `←`, `Page Up`, `Backspace`, `Ctrl+←`, wheel up |
| First / last image | `Home` / `End` |
| Zoom in / out | `+`, `=`, `-`; `Ctrl+wheel` zooms at the pointer |
| Actual size (100%) | `1`, numpad `/` |
| Fit to window (shrink only) | `2`, numpad `*` |
| Fill the window (enlarge too, proportions kept) | `3` |
| Fill the entire window (the edges are cropped) | `4` |
| Keep the zoom and position for the next images (on / off) | `L` |
| Scroll a zoomed image | arrow keys, dragging with the left button |
| Rotate left / right (view only) | `[` / `]`, `Ctrl+Alt+←` / `Ctrl+Alt+→` |
| Full screen | `F`, `Ctrl+Shift+F`, middle click |
| Gallery | `G`, `Enter`, `Esc`, double click |
| Leave full screen | `Esc` |
| Close | `Ctrl+W`, `Alt+F4`; `Esc` in the gallery |
| Move to the Recycle Bin | `Delete` (`Enter` confirms, `Esc` cancels) |
| Copy the file to the clipboard | `Ctrl+C` |
| Open a file | `Ctrl+O`, or dropping a file onto the window |
| Reload the image and the folder | `F5` |
| Show or hide the toolbar / status bar | `T` / `B` |
| List of shortcuts | `F1` |

The arrow keys scroll an image that is larger than the window in that direction; otherwise they browse. Letter keys also work with the Russian keyboard layout.

## Gallery

![The gallery: the folder tree on the left, the thumbnails of the folder on the right](images/gallery.png)

The gallery is opened with `G`, `Enter`, `Esc`, a double click on the image or the toolbar button, and shows the folder of the current image: the folder tree on the left and the images as a grid of thumbnails on the right, under a bar with the path of the folder and the thumbnail size slider. The tree lists the Pictures and Desktop folders and the drives; a folder is shown by clicking it and expanded by clicking its arrow or double-clicking it. With the "Sub-folders" check box above the grid selected, the images of the folder and of all its sub-folders, hidden ones excepted, are shown as one list in the chosen sort order. The folders chosen in the tree are then shown the same way, until the check box is cleared; opening a file shows its folder alone. In this mode the status bar shows the path of the image from the folder, and so does the tooltip of a thumbnail, and the "By folder" check box next to "Sub-folders", inactive otherwise, takes effect: the images are then shown folder by folder in the order of the tree (the images of a folder before those of its sub-folders), sorted within each folder, and each folder starts a new row of the grid under a header with its path, the number of its images and a horizontal line. The "By folder" choice is kept between runs.

| Action | Keys and mouse |
|---|---|
| Select an image | click, arrow keys, `Page Up` / `Page Down`, `Home` / `End` |
| Show the selected image | `Enter`, `G`, double click on a thumbnail |
| Thumbnail size | the slider above the grid, `+` / `-`, `Ctrl+wheel` |
| Show the images of the sub-folders too | the "Sub-folders" check box above the grid |
| Read the folder, the thumbnails and the folder tree again | `F5` |
| Close the program | `Esc` |

The selected thumbnail is the current image: deletion, copying and Show in Explorer apply to it, also from the context menu of a thumbnail. That menu, and the one of the empty space of the grid, also offers the sort order. Thumbnails are taken from the Windows thumbnail cache, which Explorer fills too, so a folder seen before appears at once; for formats Windows has no thumbnails of, qview makes them itself. The image under the pointer and the selected one are decoded in advance, so a double click shows the image without delay. The list above the grid sets the proportions of the cells: 1:1, 4:3, 3:2, 16:9, the portrait 3:4, 2:3, 9:16, or Auto, which takes the proportions most images of the folder have (read from the headers of the files when the folder is opened; for a large folder, of 200 of them); the slider sets their long side. The "Fill cells" check box makes the thumbnails fill their cells, with the edges of the images cropped; otherwise each image is shown whole. The thumbnail size, these choices and the width of the tree are kept between runs.

## Usage

```
qview.exe [FILE | FOLDER]
```

A file is shown together with the other images of its folder; a folder is opened in the gallery.

Only one window of qview is open at a time: when qview is already running, a file opened later (from Explorer or the command line) is shown in the existing window, which is brought to the foreground. Copies of qview started from different folders work independently.

## File associations

File → File Associations… registers qview with Windows for the current user (no administrator rights are needed): each supported format gets its own file type with an icon (HEIC/HEIF, AVIF, camera RAW and JPEG XR included, opened through Windows' codecs), qview appears in "Open with" and in Settings → Default apps. Each icon is a page with a band in the format's colour showing the extension.

<p align="center">
  <img src="images/file_types.png" width="520" alt="The file type icons: JPG, PNG, GIF, BMP, TIF, WEBP, ICO, TGA, QOI, PNM, HEIC, AVIF, RAW and JXR">
</p>

Windows lets only the user choose the default program, so registration offers qview but does not take over the file types by itself; only an extension no other program handles (QOI, for example) gets qview as its default program. Files that were opened with qview through "Open with" before the registration get the icon and the name of their type too. The "Choose as Default…" button opens Settings, where qview is selected under "Set defaults by app"; Windows also offers qview the next time an image is opened. The dialog shows which types qview opens by default, and "Unregister" removes everything the registration wrote. The same can be done from the command line, for example by an installer:

```
qview.exe --register
qview.exe --unregister
```

The registration refers to the location of `qview.exe`; after the program is moved, it is registered again.

The zoom mode chosen with `1`–`4` applies to the following images as well, also after zooming in or out by steps. With `L` (View → Keep Zoom and Position) the following images keep the zoom set by steps and the scrolled position instead, so that a series of photos is compared at the same place and scale; the status bar marks the zoom as kept. This choice is not kept between runs. The window position and size, the visibility of the toolbar and the status bar, the background colour, the zoom mode and the sort order are kept in `%APPDATA%\qview\data\app.ron`.

## Building

Requirements: Windows 10 or later, stable Rust with the MSVC toolchain, Visual Studio 2022 Build Tools and [uv](https://docs.astral.sh/uv/).

```
uv run build.py          # target\release\qview.exe
cargo test
```

`build.py` first closes a `qview.exe` running from the project's `target` folder, which would otherwise lock the file, and then runs `cargo build --release`; extra arguments are passed to cargo. Its only dependency, `psutil`, is installed by uv. When qview is not running, plain `cargo build --release` works as well.

The window is drawn by egui/eframe through OpenGL (glow), images are decoded by the `image` crate. Setting the environment variable `QVIEW_TRACE=1` writes start-up and decoding times to stderr.

## Installer and release

The release script builds the executable, the installer and the portable archive (Inno Setup 6 is required):

```
python tools/make_release.py              # tests, release build, installer, archive
python tools/make_release.py --no-tests   # the same without cargo test
python tools/make_release.py --install    # then a silent installation over the installed copy
```

The files are written to `dist`; the version is taken from `Cargo.toml`. Like `build.py`, the script first closes a `qview.exe` running from `target\release`. The installer script is `tools/setup.iss`; the installer icon is written by `qview.exe --export-icon <file.ico>`.

Releases on GitHub are built by the **Release** workflow (`.github/workflows/release.yml`). After the version in `Cargo.toml` is updated and committed, pushing a matching tag publishes a release with the installer and the portable archive:

```
git tag v0.1.0
git push origin v0.1.0
```

Running the workflow manually (Actions → Release → Run workflow) only builds the files and attaches them to the run, without a release.

## License

MIT, see [LICENSE](LICENSE).
