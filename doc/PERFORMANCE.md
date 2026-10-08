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
- The icons (the app's and 16 file types', 8 sizes each) were stored as 32-bit BMP layers: 306 KB each, 264 KB of it the 256 px layer, 5.2 MB of an 18 MB exe. The 256 px layer is now a PNG (Windows reads PNG layers in icons since Vista; checked with `PrivateExtractIconsW` from the exe): ~50 KB an icon, 0.85 MB in all, the exe 13.2 MB. The smaller layers stay BMP, which Windows draws without decoding.
- The C runtime is linked statically (`crt-static`, `.cargo/config.toml`), so that qview needs no Visual C++ Redistributable; it adds ~180 KB to the exe. For start-up it is slightly in favour: an empty program started and ended in 6.31 ms (median of 300 runs) against 6.70 ms with `VCRUNTIME140.dll` and the `api-ms-win-crt-*` DLLs loaded, ~0.4 ms less.

## Decoding

Measured for a 14.7 MP JPEG on a decoder thread:

| Phase | Time |
|---|---|
| Decoding | ~80 ms |
| Conversion to premultiplied BGRA | ~14 ms |
| All mip levels, in linear light (sRGB decoded through a table, averaged, encoded again; the default) | ~12 ms |
| All mip levels, of the stored values (2x2 box, packed-byte averages; View → Filtering → Reduce in Linear Light off) | ~4 ms |

- Two or three worker threads take files from a priority queue that the UI replaces as a whole, so files the user has skipped are never decoded.
- Everything the GPU needs (BGRA in the driver's native order and every mip level) is prepared on the worker; the UI thread converts nothing.
- A level of a million pixels or more (the first two of a 14.7 MP photo) is made in bands of rows on up to four threads: in one thread the levels took 5 ms of the stored values and 20 ms in linear light. `cargo test --release mip_timings -- --ignored --nocapture` measures both ways on a synthetic 14.7 MP image.
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

### RAR archives

Measured with `archive_timings` on a real comic book (RAR 1.5-4, stored, 68 pages of ~0.7 MB) and on 120 pages of 2.9 MB packed by WinRAR 7 as RAR 5, ordinary and solid.

| Archive | List | Page 1 | Middle page | Last page | All in order | All, 3 threads |
|---|---|---|---|---|---|---|
| Comic book, 68 pages | 0.9 ms | 3.5 ms | 2.7 ms | 2.9 ms | 165 ms | 84 ms |
| RAR 5, 120 pages | 0.5 ms | 10 ms | 8.4 ms | 8.1 ms | 1067 ms | 409 ms |
| RAR 5 solid, 120 pages, without the cursor | 0.6 ms | 8.8 ms | 300 ms | 571 ms | ~35 s (estimate) | |
| RAR 5 solid, 120 pages | 0.5 ms | 9.2 ms | 311 ms | 303 ms (after the middle one) | 778 ms | 757 ms |

- An entry is found by reading the headers before it; in an ordinary archive their data is skipped, so any page costs about the same. The format is told by the first 6 bytes (~0.07 ms).
- In a solid archive every page before the one wanted is unpacked, so without help the last page cost ~570 ms and the gallery's thumbnails about half a minute. The archive read last keeps an UnRAR cursor after the page read last and the pages unpacked on the way (up to 256 MB): reading on costs only the pages between, the thumbnails of the whole book about one unpacking, and threads wait for the one that unpacks instead of each starting over.
- UnRAR adds ~290 KB to the exe (with the static C runtime it needs no `MSVCP140.dll`), the CBR file type's icon ~50 KB more (306 KB before the icons' 256 px layers were stored as PNG, see below). The `unrar-ng` wrapper was tried first: it depends on `regex` with its default features, which put `regex`'s Unicode tables into the exe (~360 KB), so UnRAR's C API is called directly through `unrar-ng-sys`.

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
- **The mip levels are averaged in linear light.** Averaging sRGB values as stored darkens a reduced image wherever bright and dark pixels meet: thin bright lines, fine texture and halftones lose weight at 25–70%, where levels 1 and 2 are shown. Each level is now made from the one above with its bytes decoded to linear light through a table, averaged and encoded again (opaque pixels take the short way; a half-transparent pixel is unpremultiplied first, so its colour is weighted by its alpha). It costs ~7 ms more per 14.7 MP photo on the worker, and can be turned off in View → Filtering (the decoded images are decoded again) to compare, or for the old look.
- **Nothing is repainted without a reason.** egui draws a frame only on input, animation or loading, so a filter's cost is paid while panning, zooming, browsing or playing an animation, not while an image is merely shown.

### Possible further work

- Bicubic enlargement with 9 bilinear reads instead of 16 point reads, and reduction with part of its reads through hardware filtering: about half the cost. Worthwhile only for weak GPUs.
- Pixel-art scalers (xBR, ScaleFX, MMPX) and Lanczos were considered: the former are long shaders for a niche use, the latter is barely distinguishable from Catmull-Rom for enlargement and rings more.
- A better kernel for the mip levels (Lanczos or Kaiser instead of the 2×2 box) would sharpen Bilinear's reduction too, at an estimated 15–30 ms more decoding time per 15 MP photo on the worker (the linear-light averaging already takes 7 ms of it).
- A negative `GL_TEXTURE_LOD_BIAS` (about −0.3…−0.5) would make Bilinear's trilinear blend between 50% and 100% favour the larger level, sharper at no decoding cost, with some aliasing on regular textures.
- The integrated-GPU figures above are estimates, not measurements; the test machine has no integrated GPU.

## Histogram

The information panel's Histogram section counts the levels of red, green, blue and luma of the current image (`src/histogram.rs`). Folded, it costs nothing: no pixel is counted and nothing is kept, only a flag is stored once per frame. Open, the decoder threads count every image they decode (`Meta::histogram`, 4 KB per image), and an image decoded before the section was opened is read again and counted on a thread of its own.

### Measured

Two 5120×2880 (14.7 MP) images, a photo (JPEG) and a smooth gradient (PNG), counted from the decoder's premultiplied BGRA:

| Counting | Photo | Gradient |
|---|---|---|
| One set of counts, any channel at 0 or 255 counted per pixel | 23–26 ms | — |
| One set of counts, clipping read from the end levels | 15–23 ms | 65 ms |
| Four sets of counts for alternate pixels, one thread | 18–20 ms | 19–20 ms |
| The same in up to four parts on threads (as used) | 7–9.6 ms | 6–9.5 ms |

What the panel adds to opening an image:

| | Photo | Gradient | 49.8 MP JPEG (1520×32768, shrunk to 12.5 MP) |
|---|---|---|---|
| Decoding, folded | 104–107 ms | 56–58 ms | 325–338 ms |
| Decoding, open (counted beside the mip levels) | 109–113 ms | 61–67 ms | 330–337 ms |
| Read again and counted, when opened on an image already decoded | 80 + 6 ms | 34–41 + 6–7 ms | 135 + 17–22 ms |
| Building the graph (three translucent areas of 512 vertices and three lines), per painted frame | 6–9 µs | | |

- Open, decoding takes 3–10 ms longer, about 5%; in the 49.8 MP case the difference is within the noise. The image appears that much later; neighbours are decoded ahead anyway.
- The figures of one run of the same test vary by about 20%, since the processor's cores differ (performance and efficiency cores) and run at changing clocks.

### Decisions

- **Counted on the decoder thread, not on the UI thread, and not from the texture.** Reading pixels back from the GPU would wait for the driver; the decoder has them at hand.
- **Counted while the mip levels are made.** The histogram is counted on other threads while the decoder thread makes the levels (~5 ms for 14.7 MP), so only the difference delays the image; counted after them, it would add all of its 7–10 ms.
- **Four sets of counts.** Neighbouring pixels of a smooth area are often equal, and adding to the count just written waits for that write: a gradient took 65 ms with one set, 20 ms with four.
- **Clipping per channel.** Counting the pixels with any channel at 0 or 255 took half as long again as the four histograms; the share of each channel at 0 and 255 is read from the end levels instead.
- **From the full image, not from a mip level.** A level of 512 pixels would be counted in well under a millisecond, but averaging smooths the peaks, those at 0 and 255 in particular, which are what a histogram is looked at for.
- **One reading again at a time.** As for the panel's other data, while browsing fast with the section open only the image stopped at is read again; images decoded with the section open need no reading.

## Colour under the pointer

The information panel's Colour section shows the pixel under the pointer. Folded, it costs nothing. Open, the current image is read again from its file on a thread and kept decoded at full size; the pixel is then looked up on every pointer move.

| | 14.7 MP photo (JPEG) | 14.7 MP gradient (PNG) | 49.8 MP JPEG |
|---|---|---|---|
| Reading again, on a thread, per image browsed to | 81 ms | 38 ms | 134 ms |
| Kept in memory (RGB, 3 bytes a pixel) | ~44 MB | ~44 MB | ~149 MB |
| Finding the pixel under the pointer | ~8 ns | ~8 ns | ~8 ns |

- **The file is read again rather than the decoder's pixels kept.** The neighbours of the current image are decoded ahead, so by the time one becomes current its pixels have been uploaded and dropped; keeping them for every cached image would cost several times the memory. Reading again also gives the full image when the GPU shows it shrunk.
- **One reading at a time.** While browsing fast with the section open, only the image stopped at is read; the image read before is dropped when the next reading starts.

## Measuring

| What | How |
|---|---|
| Start-up, decoding and upload timings; filter build and GPU times | `$env:QVIEW_TRACE=1; .\target\release\qview.exe <file>` (stderr) |
| Decoding phases of one file | `$env:QVIEW_BENCH_FILE="<file>"; cargo test --release phase_timings -- --ignored --nocapture` |
| Gallery thumbnails of a folder | `$env:QVIEW_THUMB_DIR="<folder>"; cargo test --release thumbnail_timings -- --ignored --nocapture` |
| One file through WIC | `$env:QVIEW_WIC_FILE="<file>"; cargo test --release wic_file -- --ignored --nocapture` |
| HEIC through libheif and WIC | `$env:QVIEW_HEIF_FILE="<heic>"; cargo test --release heif_file -- --ignored --nocapture` |
| Histogram counting of one file, and decoding with and without it | `$env:QVIEW_BENCH_FILE="<file>"; cargo test --release histogram_timings -- --ignored --nocapture` |
| A histogram read again (the section opened on a decoded image) | `$env:QVIEW_BENCH_FILE="<file>"; cargo test --release read_again_timings -- --ignored --nocapture` |
| Finding the pixel under the pointer | `cargo test --release pick_timings -- --ignored --nocapture` |
| Building the histogram's graph | `$env:QVIEW_BENCH_FILE="<file>"; cargo test --release graph_timings -- --ignored --nocapture` |
| Header reading of a folder, qview's against the `image` crate's | `$env:QVIEW_THUMB_DIR="<folder>"; cargo test --release header_timings -- --ignored --nocapture` |

A GUI program started from PowerShell gets no console; with `Start-Process -RedirectStandardError <log>` the trace goes to a file.
