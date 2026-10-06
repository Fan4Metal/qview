//! How the image is filtered when it is not shown at 100% (View →
//! Filtering, `view::Filter`), by shaders of its own painted through egui
//! paint callbacks, in the order of egui's shapes.
//!
//! - Bilinear is the texture's own filtering: bilinear enlarged, trilinear
//!   between the mip levels reduced. Painted as an egui mesh; through a
//!   program here only with `QVIEW_TRACE`, to be timed like the others.
//! - Bicubic is a Catmull-Rom filter. Enlarged it interpolates 4x4 image
//!   pixels; reduced it is stretched over what a screen pixel covers, on
//!   the largest mip level with at most 2 image pixels to a screen pixel
//!   (up to 8x8 of them): sharper than trilinear filtering, which blends
//!   a level with one of half its size. At 100% it gives the pixels as
//!   they are.
//! - Pixelated ("sharp bilinear", enlarged only): `GL_NEAREST` alone
//!   makes pixels of unequal widths at a zoom that is not whole (at 250%
//!   some are 2 screen pixels wide, some 3), which shimmer while panning.
//!   Sampled with `GL_LINEAR` at moved coordinates instead, each pixel is
//!   flat, blended over one screen pixel at its edges, and as wide as the
//!   zoom says; at a whole zoom it is exactly `GL_NEAREST`.
//!
//! With `QVIEW_TRACE` each painting is timed on the GPU (`GL_TIME_ELAPSED`,
//! read back in a later frame, never waited for) and logged.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui_glow;
use egui::{Pos2, Rect};
use glow::HasContext;

use crate::view::Filter;

const VERTEX: &str = r#"
uniform vec4 u_place;
uniform vec2 u_uv[4];
out vec2 v_tc;
void main() {
    // A strip: top left, top right, bottom left, bottom right.
    vec2 corner = vec2(float(gl_VertexID % 2), float(gl_VertexID / 2));
    gl_Position = vec4(mix(u_place.xy, u_place.zw, corner), 0.0, 1.0);
    v_tc = u_uv[gl_VertexID];
}
"#;

const HEADER: &str = r#"
#ifdef GL_ES
precision highp float;
precision highp int;
#endif
uniform sampler2D u_sampler;
in vec2 v_tc;
out vec4 f_color;
"#;

const BILINEAR: &str = r#"
void main() {
    f_color = texture(u_sampler, v_tc);
}
"#;

const PIXELS: &str = r#"
void main() {
    vec2 size = vec2(textureSize(u_sampler, 0));
    vec2 t = v_tc * size;
    // The nearest edge between pixels, and image pixels per screen pixel.
    vec2 edge = floor(t + 0.5);
    vec2 d = max(fwidth(t), vec2(1e-5));
    t = edge + clamp((t - edge) / d, -0.5, 0.5);
    // Level 0 (the base level): the moved coordinates would pick another.
    f_color = textureLod(u_sampler, t / size, 0.0);
}
"#;

const BICUBIC: &str = r#"
// Mip levels from the base level on.
uniform int u_levels;

// Catmull-Rom (Keys, a = -0.5).
float cubic(float x) {
    x = abs(x);
    if (x < 1.0) return (1.5 * x - 2.5) * x * x + 1.0;
    if (x < 2.0) return ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0;
    return 0.0;
}

void main() {
    // Image pixels of the base level per screen pixel, along each axis of
    // the image (turned a quarter, the screen's axes swap).
    vec2 r = max(fwidth(v_tc * vec2(textureSize(u_sampler, 0))), vec2(1e-5));
    int level = int(clamp(floor(log2(max(r.x, r.y))), 0.0, float(u_levels - 1)));
    ivec2 size = textureSize(u_sampler, level);
    r = max(r / exp2(float(level)), vec2(1.0));
    vec2 p = v_tc * vec2(size);
    // The pixels whose centres are within 2 r of p.
    ivec2 lo = ivec2(ceil(p - 2.0 * r - 0.5));
    ivec2 hi = min(ivec2(floor(p + 2.0 * r - 0.5)), lo + 8);
    vec4 sum = vec4(0.0);
    float total = 0.0;
    for (int y = lo.y; y <= hi.y; y++) {
        float wy = cubic((float(y) + 0.5 - p.y) / r.y);
        for (int x = lo.x; x <= hi.x; x++) {
            float w = wy * cubic((float(x) + 0.5 - p.x) / r.x);
            sum += w * texelFetch(u_sampler, clamp(ivec2(x, y), ivec2(0), size - 1), level);
            total += w;
        }
    }
    vec4 c = sum / total;
    // The negative lobes overshoot; colours are premultiplied.
    c.a = clamp(c.a, 0.0, 1.0);
    c.rgb = clamp(c.rgb, 0.0, c.a);
    f_color = c;
}
"#;

/// A filter's program, made once on the UI thread with the painting
/// context.
pub struct Program {
    filter: Filter,
    program: glow::Program,
    /// Empty: the corners come from `gl_VertexID`, but a core profile
    /// draws nothing without a vertex array bound.
    vao: glow::VertexArray,
    place: glow::UniformLocation,
    uv: glow::UniformLocation,
    sampler: glow::UniformLocation,
    levels: Option<glow::UniformLocation>,
    timing: Mutex<Timing>,
}

/// GPU timings under way, with `QVIEW_TRACE`.
#[derive(Default)]
struct Timing {
    /// Queries not yet read, with what they timed.
    pending: VecDeque<(glow::Query, String)>,
    /// What was timed last: the same painting again is not timed.
    last: String,
}

impl Program {
    /// None if the context has no GLSL 1.40 or ES 3.0 (in/out,
    /// gl_VertexID, textureSize, texelFetch) or the shaders fail; the
    /// caller then paints a mesh (with `GL_NEAREST` for Pixelated).
    pub fn new(gl: &glow::Context, filter: Filter) -> Option<Arc<Self>> {
        let started = Instant::now();
        let version = egui_glow::ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            log::warn!("filter {}: {version:?} has no in/out", filter.name());
            return None;
        }
        let fragment = match filter {
            Filter::Bilinear => BILINEAR,
            Filter::Bicubic => BICUBIC,
            Filter::Pixels => PIXELS,
        };
        let declaration = version.version_declaration();
        let sources = [(glow::VERTEX_SHADER, format!("{declaration}{VERTEX}")), (glow::FRAGMENT_SHADER, format!("{declaration}{HEADER}{fragment}"))];
        let this = unsafe {
            let program = gl.create_program().ok()?;
            let mut shaders = Vec::new();
            let mut failed = false;
            for (kind, source) in sources {
                let Ok(shader) = gl.create_shader(kind) else {
                    failed = true;
                    break;
                };
                gl.shader_source(shader, &source);
                gl.compile_shader(shader);
                shaders.push(shader);
                if !gl.get_shader_compile_status(shader) {
                    log::warn!("filter {}: {}", filter.name(), gl.get_shader_info_log(shader));
                    failed = true;
                    break;
                }
                gl.attach_shader(program, shader);
            }
            if !failed {
                gl.link_program(program);
                if !gl.get_program_link_status(program) {
                    log::warn!("filter {}: {}", filter.name(), gl.get_program_info_log(program));
                    failed = true;
                }
            }
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            let uniform = |name| gl.get_uniform_location(program, name);
            let vao = if failed { None } else { gl.create_vertex_array().ok() };
            match (vao, uniform("u_place"), uniform("u_uv"), uniform("u_sampler")) {
                (Some(vao), Some(place), Some(uv), Some(sampler)) => {
                    let levels = uniform("u_levels");
                    Self { filter, program, vao, place, uv, sampler, levels, timing: Mutex::default() }
                }
                (vao, ..) => {
                    if let Some(vao) = vao {
                        gl.delete_vertex_array(vao);
                    }
                    gl.delete_program(program);
                    return None;
                }
            }
        };
        log::debug!("filter {}: program made in {:.1} ms", filter.name(), started.elapsed().as_secs_f64() * 1e3);
        Some(Arc::new(this))
    }

    /// Free it while the context lives (`App::on_exit`).
    pub fn delete(&self, gl: &glow::Context) {
        let timing = self.timing.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            for (query, _) in &timing.pending {
                gl.delete_query(*query);
            }
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
    }

    /// Log the GPU timings that are ready; whether some are still on their
    /// way (a later frame reads them).
    pub fn poll_timings(&self, gl: &glow::Context) -> bool {
        let mut timing = self.timing.lock().unwrap_or_else(|e| e.into_inner());
        while let Some((query, what)) = timing.pending.front() {
            let ready = unsafe { gl.get_query_parameter_u32(*query, glow::QUERY_RESULT_AVAILABLE) } != 0;
            if !ready {
                break;
            }
            let ns = unsafe { gl.get_query_parameter_u64(*query, glow::QUERY_RESULT) };
            log::debug!("filter {}: {:.3} ms on the GPU, {what}", self.filter.name(), ns as f64 / 1e6);
            unsafe { gl.delete_query(*query) };
            timing.pending.pop_front();
        }
        !timing.pending.is_empty()
    }
}

/// Paint `texture` (`levels` mip levels from its base level on) into
/// `place` with `program`'s filter, showing `uv` at its corners (top left,
/// top right, bottom right, bottom left; see `view::paint`), clipped to
/// `area`; `zoom` only for the log.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &egui::Painter,
    program: &Arc<Program>,
    texture: glow::Texture,
    levels: u32,
    place: Rect,
    uv: [Pos2; 4],
    area: Rect,
    zoom: f32,
) {
    let program = program.clone();
    let trace = log::log_enabled!(log::Level::Debug);
    let callback = egui_glow::CallbackFn::new(move |info, painter| {
        let gl = painter.gl();
        // The viewport is `area` in whole screen pixels; the image's corner
        // is on a whole pixel (`View::place`).
        let v = info.viewport_in_pixels();
        let ppp = info.pixels_per_point;
        let x = |p: f32| (p * ppp - v.left_px as f32) / v.width_px as f32 * 2.0 - 1.0;
        let y = |p: f32| 1.0 - (p * ppp - v.top_px as f32) / v.height_px as f32 * 2.0;
        // In strip order.
        let strip = [uv[0], uv[1], uv[3], uv[2]];
        let uv: Vec<f32> = strip.iter().flat_map(|p| [p.x, p.y]).collect();
        let query = trace.then(|| timed(&program, gl, place.intersect(area), ppp, zoom)).flatten();
        unsafe {
            gl.use_program(Some(program.program));
            gl.bind_vertex_array(Some(program.vao));
            gl.uniform_4_f32(Some(&program.place), x(place.min.x), y(place.min.y), x(place.max.x), y(place.max.y));
            gl.uniform_2_f32_slice(Some(&program.uv), &uv);
            gl.uniform_1_i32(Some(&program.sampler), 0);
            if let Some(levels_at) = &program.levels {
                gl.uniform_1_i32(Some(levels_at), levels.max(1) as i32);
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
            if query.is_some() {
                gl.end_query(glow::TIME_ELAPSED);
            }
        }
    });
    painter.add(egui::PaintCallback { rect: area, callback: Arc::new(callback) });
}

/// Start timing a painting of `shown` (points) unless it is the same as
/// the last one timed: a frame repainted to read the timing back is not
/// timed again.
fn timed(program: &Program, gl: &glow::Context, shown: Rect, ppp: f32, zoom: f32) -> Option<glow::Query> {
    let what = format!(
        "{}x{} screen pixels of image at {}",
        (shown.width() * ppp).round(),
        (shown.height() * ppp).round(),
        crate::format::zoom(zoom)
    );
    let mut timing = program.timing.lock().unwrap_or_else(|e| e.into_inner());
    if timing.last == what {
        return None;
    }
    let query = unsafe { gl.create_query() }.ok()?;
    unsafe { gl.begin_query(glow::TIME_ELAPSED, query) };
    timing.last = what.clone();
    timing.pending.push_back((query, what));
    Some(query)
}
