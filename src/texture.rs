//! Image textures uploaded to OpenGL directly with glow, not through egui's
//! texture manager. egui_glow uploads RGBA with `glTexImage2D` and then
//! calls `glGenerateMipmap`, which took 70-150 ms of the UI thread for a
//! 15-megapixel photo on an RTX 4070 (the driver converts the pixels and
//! builds the mipmaps itself). Here the decoder thread prepares BGRA pixels
//! in the driver's native order and every mip level (`loader::Pixels`),
//! so the UI thread only hands them to the driver.
//!
//! Even so, the driver's first pass over new memory is slow (about 25 ms
//! plus 1 ms per megabyte, measured), so a texture can start at the level
//! the image is shown at (`GL_TEXTURE_BASE_LEVEL`): for a photo fitted to
//! the window that is level 1 or 2, a quarter of the bytes or less. The
//! levels below are uploaded later ([`Texture::complete`]), when a frame
//! has nothing else to upload or when the user zooms in.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;

use glow::HasContext;

use crate::loader::Pixels;

/// A texture of the GL context `gl`, registered with egui as `id`. Deleted
/// when dropped, which must happen on the UI thread while the context
/// lives (see `App::on_exit`).
pub struct Texture {
    gl: Arc<glow::Context>,
    native: glow::Texture,
    id: egui::TextureId,
    /// Size of level 0 and the number of levels.
    size: (u32, u32, usize),
    /// The smallest level uploaded so far; the texture is shown from it.
    base_level: Cell<u32>,
}

impl Texture {
    /// Upload the levels of `pixels` from `first` on, and register the
    /// texture with egui through `register`
    /// (`eframe::Frame::register_native_glow_texture`).
    pub fn new(
        gl: Arc<glow::Context>,
        pixels: &Pixels,
        first: u32,
        register: impl FnOnce(glow::Texture) -> egui::TextureId,
    ) -> Result<Self, String> {
        let first = first.min(pixels.levels.len() as u32 - 1);
        let started = Instant::now();
        let native = unsafe {
            let native = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(native));
            let param = |name: u32, value: u32| gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            param(glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR);
            param(glow::TEXTURE_MAG_FILTER, glow::LINEAR);
            param(glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE);
            param(glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE);
            param(glow::TEXTURE_MAX_LEVEL, pixels.levels.len() as u32 - 1);
            param(glow::TEXTURE_BASE_LEVEL, first);
            upload_levels(&gl, pixels, first..pixels.levels.len() as u32);
            gl.bind_texture(glow::TEXTURE_2D, None);
            // glGetError waits for the driver's worker thread to finish the
            // upload (20-60 ms): only in debug builds.
            let error = if cfg!(debug_assertions) { gl.get_error() } else { glow::NO_ERROR };
            if error != glow::NO_ERROR {
                gl.delete_texture(native);
                return Err(format!("OpenGL error {error:#x} while uploading the texture"));
            }
            native
        };
        log::debug!(
            "uploaded {}x{} from level {first} in {:.1} ms",
            pixels.width,
            pixels.height,
            started.elapsed().as_secs_f64() * 1e3
        );
        let id = register(native);
        let size = (pixels.width, pixels.height, pixels.levels.len());
        Ok(Self { gl, native, id, size, base_level: Cell::new(first) })
    }

    pub fn id(&self) -> egui::TextureId {
        self.id
    }

    /// The smallest level uploaded so far, 0 when complete.
    pub fn base_level(&self) -> u32 {
        self.base_level.get()
    }

    /// Upload the levels below the base level, so that the texture is
    /// shown at full resolution.
    pub fn complete(&self, pixels: &Pixels) {
        let base = self.base_level.get();
        if base == 0 {
            return;
        }
        let started = Instant::now();
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.native));
            upload_levels(&self.gl, pixels, 0..base);
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.base_level.set(0);
        log::debug!(
            "completed {}x{} (levels below {base}) in {:.1} ms",
            pixels.width,
            pixels.height,
            started.elapsed().as_secs_f64() * 1e3
        );
    }
}

impl Texture {
    /// Replace the picture with `pixels`, every level (the next frame of
    /// an animation). The same size is written into the texture's memory,
    /// which is not allocated anew.
    pub fn replace(&self, pixels: &Pixels) {
        let size = (pixels.width, pixels.height, pixels.levels.len());
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.native));
            if size == self.size {
                self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                for (level, data) in pixels.levels.iter().enumerate() {
                    self.gl.tex_sub_image_2d(
                        glow::TEXTURE_2D,
                        level as i32,
                        0,
                        0,
                        (pixels.width >> level).max(1) as i32,
                        (pixels.height >> level).max(1) as i32,
                        glow::BGRA,
                        glow::UNSIGNED_INT_8_8_8_8_REV,
                        glow::PixelUnpackData::Slice(Some(data)),
                    );
                }
            } else {
                log::warn!("a frame of {}x{} for a texture of {}x{}", size.0, size.1, self.size.0, self.size.1);
                upload_levels(&self.gl, pixels, 0..pixels.levels.len() as u32);
                self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, pixels.levels.len() as i32 - 1);
            }
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.base_level.set(0);
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe { self.gl.delete_texture(self.native) };
    }
}

/// Upload `levels` of `pixels` into the bound texture. BGRA with reversed
/// 8_8_8_8 is the layout Windows drivers keep textures in, so nothing is
/// converted on the way.
unsafe fn upload_levels(gl: &glow::Context, pixels: &Pixels, levels: std::ops::Range<u32>) {
    unsafe {
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        for level in levels {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                level as i32,
                glow::RGBA8 as i32,
                (pixels.width >> level).max(1) as i32,
                (pixels.height >> level).max(1) as i32,
                0,
                glow::BGRA,
                glow::UNSIGNED_INT_8_8_8_8_REV,
                glow::PixelUnpackData::Slice(Some(&pixels.levels[level as usize])),
            );
        }
    }
}
