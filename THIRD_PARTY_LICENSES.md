# Third-party licenses

qview's own source code is licensed under the MIT License (see [LICENSE](LICENSE)).

## libheif, libde265 and dav1d

The installer and the portable archive include `heif.dll` and `libde265.dll`, with which the program reads HEIC, HEIF and AVIF images: [libheif](https://github.com/strukturag/libheif) 1.23.5 with [libde265](https://github.com/strukturag/libde265) 1.1.3 (the HEVC decoder) and [dav1d](https://code.videolan.org/videolan/dav1d) 1.5.4 (the AV1 decoder) built in. libheif and libde265 are licensed under the **GNU Lesser General Public License, version 3** (see [licenses/LGPL-3.0.txt](licenses/LGPL-3.0.txt), which also holds the GNU GPL version 3 that the LGPL builds on). The program loads them from its own folder at run time and is not linked with them, so they can be replaced with other builds of the same libraries; without them HEIC, HEIF and AVIF are read by the Windows codecs instead.

dav1d, by VideoLAN and the dav1d authors, is linked into `heif.dll` and licensed under the **BSD 2-Clause License**; the AV1 format is covered by the Alliance for Open Media Patent License 1.0 (both in [licenses/dav1d-license.txt](licenses/dav1d-license.txt)).

The source code of the included builds:

- libheif: <https://github.com/strukturag/libheif/tree/v1.23.5>;
- libde265: <https://github.com/strukturag/libde265/tree/v1.1.3>;
- dav1d: <https://code.videolan.org/videolan/dav1d/-/tree/1.5.4>;
- the build script, which fetches these tags and builds them unmodified, libheif and libde265 as DLLs with the C runtime linked in and dav1d as a static library linked into `heif.dll`: [`tools/build_heif.py`](https://github.com/Fan4Metal/qview/blob/main/tools/build_heif.py) (in the repository; the installer and the portable archive do not include it).

## UnRAR

RAR archives and CBR comic books are read with the UnRAR source code by Alexander Roshal, compiled into `qview.exe` unmodified (through the crate `unrar-ng-sys`). It is freeware under its own licence, which allows its use for reading RAR archives but not for re-creating the RAR compression algorithm (see [licenses/unrar-license.txt](licenses/unrar-license.txt)).

## egui / eframe and the other Rust crates

The user interface uses [egui and eframe](https://github.com/emilk/egui), licensed under the MIT License or the Apache License 2.0, and the program further Rust crates under permissive licenses (MIT, Apache 2.0, BSD, ISC, Zlib, BSL, Unicode), among them [image](https://github.com/image-rs/image) for decoding and [resvg](https://github.com/linebender/resvg) for SVG, compiled into `qview.exe`, together with the fonts egui embeds (Ubuntu Font Licence, SIL Open Font License 1.1, MIT). Their licence texts and copyright notices are in [licenses/THIRD_PARTY_NOTICES.txt](licenses/THIRD_PARTY_NOTICES.txt), written by `tools/notices.py` from `Cargo.lock` and `Cargo.toml`; the program also shows them in its About window (**Licenses…**).
