# qview performance

[Русская версия](PERFORMANCE.ru.md) · [README](../README.md)

This document collects the measurements behind qview's design: where the time goes from start-up to a browsed image, the estimates made before a change, and the decisions taken from them. Every figure comes from the test machine below unless stated otherwise; figures from other hardware are estimates and are marked as such.

## Test machine

| | |
|---|---|
| CPU | Intel Core i7-13700KF |
| GPU | NVIDIA GeForce RTX 4070 Ti SUPER (driver 32.0.16.1074, NVIDIA 610) |
| Display | 3840×2160 at 240 Hz, 150% Windows scaling |
| Memory | 32 GB |
| OS | Windows 10 Pro 22H2 |

At 240 Hz a frame lasts 4.2 ms; at 60 Hz, 16.7 ms. GPU timings below are compared with both budgets.

## Start-up

| Moment | Time after process creation |
|---|---|
| `main` entered | ~20 ms |
| Window created | ~145 ms |
| First frame | ~160 ms |
| 14.7 MP JPEG decoded (in parallel) | ~80 ms of decoding |

- The loader is created and the file from the command line is requested before `eframe::run_native`, so decoding overlaps window creation; window creation, not decoding, is the limit.
- The renderer is glow, not wgpu, and eframe's default features (accesskit, wgpu) are off.
- The window icon is rasterised by `build.rs` at build time; rasterising it at start-up took 6 ms.
- A window saved maximized is created normal, cloaked, maximized and shown once a maximized frame is presented, which avoids a white flash rather than saving time.
- The WIC decoder list (`wic::extensions`) is warmed by a thread in `main`.
- A filter shader (see [Filtering](#filtering)) is never built before the first image is on screen.

## Decoding

Measured for a 14.7 MP JPEG on a decoder thread:

| Phase | Time |
|---|---|
| Decoding | ~80 ms |
| Conversion to premultiplied BGRA | ~14 ms |
| All mip levels (2x2 box, packed-byte averages) | ~5 ms |

- Two or three worker threads take files from a priority queue that the UI replaces as a whole, so files the user has skipped are never decoded.
- Everything the GPU needs (BGRA in the driver's native order and every mip level) is prepared on the worker; the UI thread converts nothing.
- `cargo test --release phase_timings -- --ignored --nocapture` with `QVIEW_BENCH_FILE` measures these phases for any file.

### HEIC and HEIF

| Path | 12 MP iPhone HEIC |
|---|---|
| WIC, decoder released after each file | ~280 ms header, ~500 ms decode |
| WIC, decoder kept per thread (`wic::keep_codec`) | ~5 ms header, ~160 ms decode |
| libheif, 4 threads (its default) | ~120 ms decode |
| libheif, 8 threads | ~95 ms decode |
| libheif header, unbuffered reads | ~25 ms (~30,000 small reads) |
| libheif header through a 64 KB `BufReader` | ~1 ms |

- Releasing the last HEIF decoder of WIC shut its HEVC pipeline down (~260 ms), so each thread keeps one decoder of every container it has used.
- libheif with 8 decoding threads is preferred for HEIC/HEIF; WIC remains the fallback.

## Texture upload

| Method | UI thread time per 15 MP photo |
|---|---|
| egui's `load_texture` (RGBA `glTexImage2D` + `glGenerateMipmap`) | 70–150 ms |
| Direct glow upload, BGRA `UNSIGNED_INT_8_8_8_8_REV`, levels from the worker | 3–10 ms (warm driver memory pool) |

- The driver's first pass over new memory costs about 25 ms plus 1 ms per MB, whatever is done; a warm-up only moves the cost.
- A texture therefore starts at the mip level the image is shown at (`GL_TEXTURE_BASE_LEVEL`) and the levels below are uploaded in a later idle frame, or at once when the view needs them: the first 14.7 MP photo is on screen about 85 ms sooner.
- One texture is made per frame (the current image first), because a large upload stalls the frame.
- `glGetError` waits for the driver thread (20–60 ms), so release builds never call it.

## Browsing cache

- Textures are kept for the current image, the next one in the direction of travel, the previous one, and up to three more ahead within 100 million pixels (four 24 MP photos, about half a gigabyte of texture memory).
- Every image is counted as large as the current one, so the set does not change while neighbours decode.
- The previous picture stays on screen until the current one is decoded: there are no blank frames.

## Gallery thumbnails

| Source | 12 MP JPEG |
|---|---|
| Windows' thumbnail cache, cached | ~3 ms |
| Windows' thumbnail cache, not cached | ~30 ms |

- Thumbnails come from `IShellItemImageFactory` at Windows' cache sizes (96, 256, 768, 1280); qview decodes them itself only for formats Windows cannot (QOI, TGA, PNM).
- A thumbnail may be enlarged up to 1.5 times before a larger size is requested, because 768 is rarely in Windows' cache and is made anew with nine times the pixels.
- Image sizes are read from file headers; for JPEG qview parses segments up to SOFn itself, because the `image` crate's JPEG decoder reads the whole file even for its dimensions.
- Uploads are limited per frame, and thumbnails beyond a pixel budget are evicted least recently used first, never those drawn in the last frame.
- `cargo test --release thumbnail_timings -- --ignored --nocapture` with `QVIEW_THUMB_DIR` measures a folder.

## Filtering

View → Filtering chooses how an image not shown at 100% is filtered. Bilinear is the texture's own filtering and is drawn as an egui mesh; Bicubic and Pixelated are fragment shaders of qview's own (`src/filter.rs`), drawn through egui_glow paint callbacks.

### Estimates made beforehand

| Filter | Texture reads per screen pixel | RTX-class GPU, 2560×1440 image area | Integrated Intel UHD 620, same (estimate) |
|---|---|---|---|
| Bilinear | 1–2 (trilinear) | ~0.02 ms | ~0.3 ms |
| Pixelated | 1 | ~0.02 ms | ~0.3 ms |
| Bicubic, enlarged | 16 | ~0.1–0.2 ms | ~2–3 ms |
| Bicubic, reduced | 16–64, by zoom | ~0.1–0.5 ms | ~3–10 ms |

### Measured

A 6000×4000 test image on a 2827×1652 image area (4.7 million screen pixels), timed on the GPU with `GL_TIME_ELAPSED` queries under `QVIEW_TRACE`:

| Filter | Reduced (41%) | 100% | Enlarged (125–1600%) | Program built in |
|---|---|---|---|---|
| Bilinear | ~0.05 ms | ~0.03 ms | 0.06–0.08 ms | — (none needed) |
| Bicubic | 1.0–1.4 ms | 0.8–1.7 ms (now drawn as a mesh) | 0.43–0.9 ms | 17.6 ms |
| Pixelated | as Bilinear | as Bilinear | 0.13–0.48 ms | 8.1 ms |

- The first program built in a run took 85 ms (the driver's shader compiler starting); later ones 8–18 ms.
- Bilinear showed 1–8 ms in the first seconds after the image was opened while the full-resolution levels were still being uploaded; once they were, it was steady at 0.03–0.05 ms.
- Single frames after idle run at reduced GPU clocks, so these figures are on the high side.
- Bicubic costs 20–30 times as much as Bilinear, but at 1–1.7 ms it takes 6–10% of a 60 Hz frame and 25–40% of a 240 Hz frame. The worst case, reduction to just above 50% (up to 64 reads per pixel), is expected at about 3 ms on this GPU, still within a 240 Hz frame.
- On a weak integrated GPU Bicubic is expected at 20–40 ms per frame on a large window: panning and animations would stutter.

### Decisions

- **Bilinear stays the default.** It costs nothing on any GPU and is what earlier versions showed; Bicubic is a choice for those who want sharper images and have the GPU for it.
- **Pixelated uses a "sharp bilinear" shader, not `GL_NEAREST`.** With `GL_NEAREST` alone, pixels at a zoom that is not whole have unequal widths (at 250% some are 2 screen pixels wide, some 3) and shimmer while panning; the shader keeps every pixel flat and equally wide, blending over one screen pixel at its edges, and is exactly `GL_NEAREST` at whole zooms. `GL_NEAREST` remains the fallback if the shader cannot be built.
- **Bicubic reduction samples one mip level, not two.** Trilinear filtering blends a level with one of half its size, which softens images shown between 50% and 100%. Bicubic instead takes the largest level with at most two image pixels per screen pixel and stretches a Catmull-Rom kernel over the screen pixel's footprint (up to 8×8 reads).
- **At 100% Bicubic is drawn as a mesh.** Its result there equals the image's pixels, and the mesh does it for a sixteenth of the work.
- **A program is built in the frame after the one that first wants it.** The image is drawn as a mesh in that first frame, so a start-up with Bicubic chosen shows the image as soon as before; the 8–85 ms of building follow once it is on screen.
- **Nothing is repainted without a reason.** egui draws a frame only on input, animation or loading, so a filter's cost is paid while panning, zooming, browsing or playing an animation, not while an image is merely shown.

### Possible further work

- Bicubic enlargement with 9 bilinear reads instead of 16 point reads, and reduction with part of its reads through hardware filtering: about half the cost. Worthwhile only for weak GPUs.
- Pixel-art scalers (xBR, ScaleFX, MMPX) and Lanczos were considered: the former are long shaders for a niche use, the latter is barely distinguishable from Catmull-Rom for enlargement and rings more.
- Better mip levels (Lanczos or Kaiser instead of the 2×2 box) would sharpen Bilinear's reduction too, at an estimated 15–30 ms more decoding time per 15 MP photo on the worker.

## Measuring

| What | How |
|---|---|
| Start-up, decoding and upload timings; filter build and GPU times | `$env:QVIEW_TRACE=1; .\target\release\qview.exe <file>` (stderr) |
| Decoding phases of one file | `$env:QVIEW_BENCH_FILE="<file>"; cargo test --release phase_timings -- --ignored --nocapture` |
| Gallery thumbnails of a folder | `$env:QVIEW_THUMB_DIR="<folder>"; cargo test --release thumbnail_timings -- --ignored --nocapture` |
| One file through WIC | `$env:QVIEW_WIC_FILE="<file>"; cargo test --release wic_file -- --ignored --nocapture` |
| HEIC through libheif and WIC | `$env:QVIEW_HEIF_FILE="<heic>"; cargo test --release heif_file -- --ignored --nocapture` |
| Header reading of a folder, qview's against the `image` crate's | `$env:QVIEW_THUMB_DIR="<folder>"; cargo test --release header_timings -- --ignored --nocapture` |

A GUI program started from PowerShell gets no console; with `Start-Process -RedirectStandardError <log>` the trace goes to a file.
