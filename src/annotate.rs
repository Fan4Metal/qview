//! Marks drawn over an image (A, `App::annotating`): frames, ellipses,
//! arrows, pixelated areas and text, in pixels of the image as shown
//! (turned by the view), like the crop's frame (`crop`). `ui::annotate`
//! draws them on screen and takes the mouse; saving draws them into the
//! pixels (`draw`, from `edit::transform`), as do Copy Image, printing and
//! the desktop background.
//!
//! What is saved is what was shown: frames, ellipses and arrows are the
//! same shapes in both (egui's painter on screen, tiny-skia in the file),
//! text is drawn by resvg into a small image for both, and a pixelated
//! area is the averages of the image's blocks, computed once per block
//! size from the image as `edit` decodes it (`mosaic`) for the screen and
//! again for the file. Pixelated areas lie under everything else, as they
//! cover what is under them, not the marks.

use std::sync::{LazyLock, mpsc};

use egui::{Color32, Pos2, Rect, TextureHandle, Vec2, vec2};
use image::DynamicImage;
use resvg::{tiny_skia, usvg};

use crate::view::Zoom;

/// The colours to choose from (1 to 6 while marking).
pub const COLOURS: [Color32; 6] = [
    Color32::from_rgb(0xe5, 0x2b, 0x2b),
    Color32::from_rgb(0xff, 0xc4, 0x00),
    Color32::from_rgb(0x2e, 0xb8, 0x4d),
    Color32::from_rgb(0x1e, 0x7c, 0xf0),
    Color32::WHITE,
    Color32::BLACK,
];

/// What a mark's size measures, in `unit`s of the image: the width of a
/// line, the height of text, or the side of a pixelated area's blocks.
/// Each is set continuously (the bar's slider; [ and ] step it) and kept
/// apart, so that thick lines do not make large text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    Line,
    Text,
    Block,
}

impl Measure {
    pub fn of_tool(tool: Tool) -> Measure {
        match tool {
            Tool::Text => Measure::Text,
            Tool::Pixelate => Measure::Block,
            _ => Measure::Line,
        }
    }

    pub fn of(shape: &Shape) -> Measure {
        match shape {
            Shape::Text(..) => Measure::Text,
            Shape::Pixelate(_) => Measure::Block,
            _ => Measure::Line,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            Measure::Line => 0.25..=40.0,
            Measure::Text => 2.0..=150.0,
            Measure::Block => 0.7..=160.0,
        }
    }

    /// The size marks start with: 2 units wide lines (3.2 pixels on a 1920
    /// pixel screenshot), text 9.6 units high, blocks of 6.8 units.
    pub fn default_size(self) -> f32 {
        match self {
            Measure::Line => 2.0,
            Measure::Text => 9.6,
            Measure::Block => 6.8,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Measure::Line => tr!("Width", "Толщина"),
            Measure::Text => tr!("Text", "Текст"),
            Measure::Block => tr!("Blocks", "Блоки"),
        }
    }

    /// `size` within the range.
    pub fn clamped(self, size: f32) -> f32 {
        let r = self.range();
        clamp(size, *r.start(), *r.end())
    }

    /// `size` in pixels of an image of `unit`, as the marks are drawn.
    pub fn pixels(self, unit: f32, size: f32) -> f32 {
        match self {
            Measure::Line => line_width(unit, size),
            Measure::Text => font_size(unit, size),
            Measure::Block => block(unit, size) as f32,
        }
    }

    /// One step of [ or ] from `size`: a quarter, a power of two for
    /// blocks.
    pub fn stepped(self, size: f32, up: bool) -> f32 {
        let factor = if self == Measure::Block { 2.0 } else { 1.25 };
        self.clamped(if up { size * factor } else { size / factor })
    }
}

/// How text is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextStyle {
    /// In its colour, outlined in black or white, whichever stands out.
    Outline,
    /// In black or white on a rounded plate of its colour.
    Plate,
    /// In its colour alone.
    Plain,
}

impl TextStyle {
    pub const ALL: [TextStyle; 3] = [TextStyle::Outline, TextStyle::Plate, TextStyle::Plain];

    pub fn label(self) -> &'static str {
        match self {
            TextStyle::Outline => tr!("Outline", "Обводка"),
            TextStyle::Plate => tr!("Plate", "Плашка"),
            TextStyle::Plain => tr!("Plain", "Без обводки"),
        }
    }

    fn id(self) -> &'static str {
        match self {
            TextStyle::Outline => "outline",
            TextStyle::Plate => "plate",
            TextStyle::Plain => "plain",
        }
    }
}

/// How sharp an arrow's head is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrowHead {
    /// Short and wide.
    Blunt,
    Normal,
    /// Long and narrow.
    Sharp,
}

impl ArrowHead {
    pub const ALL: [ArrowHead; 3] = [ArrowHead::Blunt, ArrowHead::Normal, ArrowHead::Sharp];

    pub fn label(self) -> &'static str {
        match self {
            ArrowHead::Blunt => tr!("Blunt", "Тупая"),
            ArrowHead::Normal => tr!("Normal", "Обычная"),
            ArrowHead::Sharp => tr!("Sharp", "Острая"),
        }
    }

    /// Its name in the settings (`Preset::to_line`).
    fn id(self) -> &'static str {
        match self {
            ArrowHead::Blunt => "blunt",
            ArrowHead::Normal => "normal",
            ArrowHead::Sharp => "sharp",
        }
    }

    /// The head's length in widths of the line, and its half width in
    /// lengths: 74°, 53° and 30° at the tip.
    fn proportions(self) -> (f32, f32) {
        match self {
            ArrowHead::Blunt => (3.5, 0.75),
            ArrowHead::Normal => (4.5, 0.5),
            ArrowHead::Sharp => (6.5, 0.27),
        }
    }
}

/// The font of text: a family of the fonts installed, bold, italic. A
/// family that is not installed (a preset from another computer) falls
/// back to Segoe UI.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextFont {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
}

impl Default for TextFont {
    fn default() -> Self {
        TextFont { family: FALLBACK_FAMILY.into(), bold: false, italic: false }
    }
}

/// The family text falls back to.
const FALLBACK_FAMILY: &str = "Segoe UI";

/// The families of the fonts installed, sorted, for the bar's list
/// (loaded with the fonts, ~20 ms, on the thread that reads the mosaics).
pub fn families() -> &'static [String] {
    static FAMILIES: LazyLock<Vec<String>> = LazyLock::new(|| {
        let fonts = crate::svg::system_fonts();
        let mut names: Vec<String> = fonts
            .faces()
            .filter_map(|f| f.families.first())
            .map(|(name, _)| name.clone())
            // Vertical writing's faces.
            .filter(|n| !n.starts_with('@'))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    });
    &FAMILIES
}

/// A tool with its colour, size, text style, font and arrow head, chosen in
/// one click (the bar's Presets), the user's own, kept between runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub tool: Tool,
    pub colour: Color32,
    /// Of the tool's `Measure`.
    pub size: f32,
    pub text: TextStyle,
    pub font: TextFont,
    pub head: ArrowHead,
    /// Of text (`Mark::contrast`).
    pub contrast: Option<Color32>,
}

impl Preset {
    /// As kept in the settings: `text #e52b2b 9.6 plate b- Segoe UI`, the
    /// font's bold and italic, then its family to the end of the line; an
    /// arrow's head after the text style (`arrow #e52b2b 2 outline sharp
    /// b- Segoe UI`), text's second colour there when it is chosen
    /// (`text #ffc400 9.6 outline #1e7cf0 -- Segoe UI`).
    pub fn to_line(&self) -> String {
        let flags = format!("{}{}", if self.font.bold { 'b' } else { '-' }, if self.font.italic { 'i' } else { '-' });
        let head = if self.tool == Tool::Arrow { format!(" {}", self.head.id()) } else { String::new() };
        let contrast = match self.contrast {
            Some(c) if self.tool == Tool::Text => format!(" {}", hex(c)),
            _ => String::new(),
        };
        format!("{} {} {} {}{head}{contrast} {flags} {}", self.tool.id(), hex(self.colour), self.size, self.text.id(), self.font.family)
    }

    /// A line of `to_line`; those of before fonts and heads (without them)
    /// too.
    pub fn parse(line: &str) -> Option<Preset> {
        let mut parts = line.split_whitespace();
        let (tool, colour, size, text) = (parts.next()?, parts.next()?, parts.next()?, parts.next());
        let tool = Tool::ALL.into_iter().find(|t| t.id() == tool)?;
        let colour = Color32::from_hex(colour).ok()?;
        let size = Measure::of_tool(tool).clamped(size.parse().ok()?);
        let text = TextStyle::ALL.into_iter().find(|t| Some(t.id()) == text).unwrap_or(TextStyle::Outline);
        let mut rest: Vec<&str> = parts.collect();
        let head = match rest.first().and_then(|p| ArrowHead::ALL.into_iter().find(|h| h.id() == *p)) {
            Some(head) => {
                rest.remove(0);
                head
            }
            None => ArrowHead::Normal,
        };
        let contrast = rest.first().filter(|p| p.starts_with('#')).and_then(|p| Color32::from_hex(p).ok());
        if contrast.is_some() {
            rest.remove(0);
        }
        let font = match rest.split_first() {
            Some((flags, family)) if flags.len() == 2 && !family.is_empty() => {
                TextFont { family: family.join(" "), bold: flags.starts_with('b'), italic: flags.ends_with('i') }
            }
            _ => TextFont::default(),
        };
        Some(Preset { tool, colour, size, text, font, head, contrast })
    }

    /// What it is, for the menu: the tool, the text style or the head, the
    /// size in pixels of an image of `unit`.
    pub fn label(&self, unit: f32) -> String {
        let px = Measure::of_tool(self.tool).pixels(unit, self.size).round();
        match self.tool {
            Tool::Text => format!("{}, {}, {}, {px} px", self.tool.label(), self.text.label().to_lowercase(), self.font.family),
            Tool::Arrow => format!("{}, {}, {px} px", self.tool.label(), self.head.label().to_lowercase()),
            _ => format!("{}, {px} px", self.tool.label()),
        }
    }
}

/// What a drag on the image draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Rect,
    Ellipse,
    Line,
    Arrow,
    /// Freehand.
    Pen,
    Pixelate,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 7] = [Tool::Rect, Tool::Ellipse, Tool::Line, Tool::Arrow, Tool::Pen, Tool::Pixelate, Tool::Text];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Rect => tr!("Frame", "Рамка"),
            Tool::Ellipse => tr!("Ellipse", "Овал"),
            Tool::Line => tr!("Line", "Линия"),
            Tool::Arrow => tr!("Arrow", "Стрелка"),
            Tool::Pen => tr!("Pen", "Карандаш"),
            Tool::Pixelate => tr!("Pixelate", "Пикселизация"),
            Tool::Text => tr!("Text", "Текст"),
        }
    }

    /// Its name in the settings (`Preset::to_line`).
    fn id(self) -> &'static str {
        match self {
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Line => "line",
            Tool::Arrow => "arrow",
            Tool::Pen => "pen",
            Tool::Pixelate => "pixelate",
            Tool::Text => "text",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Rect => "R",
            Tool::Ellipse => "E",
            Tool::Line => "L",
            Tool::Arrow => "A",
            Tool::Pen => "D",
            Tool::Pixelate => "P",
            Tool::Text => "T",
        }
    }
}

/// A mark's place on the image, in its pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect(Rect),
    Ellipse(Rect),
    Line(Pos2, Pos2),
    /// From the first point to the second, the head at the second.
    Arrow(Pos2, Pos2),
    /// Drawn freehand: the points the pointer went through.
    Pen(Vec<Pos2>),
    Pixelate(Rect),
    /// The top left of the first line, in whole pixels, and the lines.
    Text(Pos2, String),
}

impl Shape {
    /// The tool that draws it.
    pub fn tool(&self) -> Tool {
        match self {
            Shape::Rect(_) => Tool::Rect,
            Shape::Ellipse(_) => Tool::Ellipse,
            Shape::Line(..) => Tool::Line,
            Shape::Arrow(..) => Tool::Arrow,
            Shape::Pen(_) => Tool::Pen,
            Shape::Pixelate(_) => Tool::Pixelate,
            Shape::Text(..) => Tool::Text,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub shape: Shape,
    pub colour: Color32,
    /// In units of the image, of its `Measure`.
    pub size: f32,
    /// Of text.
    pub text: TextStyle,
    pub font: TextFont,
    /// Of an arrow.
    pub head: ArrowHead,
    /// Of text: the outline's colour, or the letters' on a plate; None
    /// for black or white, whichever stands out (`outline`).
    pub contrast: Option<Color32>,
}

impl Mark {
    /// Where it lies on the image, in its pixels (text's from `text_box`,
    /// its box measured as drawn).
    pub fn bounds(&self, text_box: Option<Rect>) -> Rect {
        match &self.shape {
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Pixelate(r) => *r,
            Shape::Line(a, b) | Shape::Arrow(a, b) => Rect::from_two_pos(*a, *b),
            Shape::Pen(points) => Rect::from_points(points),
            Shape::Text(p, _) => text_box.unwrap_or(Rect::from_min_max(*p, *p)),
        }
    }

    /// The frame of a frame, an ellipse or a pixelated area.
    pub fn frame(&self) -> Option<Rect> {
        match self.shape {
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Pixelate(r) => Some(r),
            _ => None,
        }
    }

    pub fn set_frame(&mut self, frame: Rect) {
        if let Shape::Rect(r) | Shape::Ellipse(r) | Shape::Pixelate(r) = &mut self.shape {
            *r = frame;
        }
    }

    /// The ends of a line or an arrow.
    pub fn ends(&self) -> Option<(Pos2, Pos2)> {
        match self.shape {
            Shape::Line(a, b) | Shape::Arrow(a, b) => Some((a, b)),
            _ => None,
        }
    }

    /// Moved by `d`, kept on an image of `size` as far as it fits.
    pub fn moved(&self, d: Vec2, size: Vec2) -> Mark {
        let bounds = self.bounds(None);
        let d = vec2(clamp(d.x, -bounds.min.x, size.x - bounds.max.x), clamp(d.y, -bounds.min.y, size.y - bounds.max.y));
        let shape = match &self.shape {
            Shape::Rect(r) => Shape::Rect(r.translate(d)),
            Shape::Ellipse(r) => Shape::Ellipse(r.translate(d)),
            Shape::Pixelate(r) => Shape::Pixelate(r.translate(d)),
            Shape::Line(a, b) => Shape::Line(*a + d, *b + d),
            Shape::Arrow(a, b) => Shape::Arrow(*a + d, *b + d),
            Shape::Pen(points) => Shape::Pen(points.iter().map(|p| *p + d).collect()),
            Shape::Text(p, t) => Shape::Text((*p + d).round(), t.clone()),
        };
        Mark { shape, ..self.clone() }
    }

    /// Too small to keep, `min` image pixels across.
    pub fn is_tiny(&self, min: f32) -> bool {
        match &self.shape {
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Pixelate(r) => r.width() < min || r.height() < min,
            Shape::Line(a, b) | Shape::Arrow(a, b) => a.distance(*b) < min,
            Shape::Pen(points) => {
                let r = Rect::from_points(points);
                points.len() < 2 || r.width().max(r.height()) < min
            }
            Shape::Text(_, t) => t.trim().is_empty(),
        }
    }
}

/// The unit of the marks' sizes on an image of `size`: a 600th of its
/// longer side, at least 1.5 pixels, so that marks look alike on a
/// screenshot and on a photo.
pub fn unit(size: Vec2) -> f32 {
    (size.x.max(size.y) / 600.0).max(1.5)
}

/// The width of a mark's line.
pub fn line_width(unit: f32, size: f32) -> f32 {
    unit * Measure::Line.clamped(size)
}

/// The height of text (its font size).
pub fn font_size(unit: f32, size: f32) -> f32 {
    unit * Measure::Text.clamped(size)
}

/// The side of a pixelated area's blocks: a power of two, so that the
/// blocks are those of the image's grid from its corner, 4 to 1024.
pub fn block(unit: f32, size: f32) -> u32 {
    let b = (unit * Measure::Block.clamped(size)).clamp(4.0, 1024.0);
    1 << (b.log2().floor() as u32)
}

/// Every block side the sizes give on an image of `unit`, smallest first:
/// the mosaics read when marking starts.
pub fn blocks(unit: f32) -> Vec<u32> {
    let range = Measure::Block.range();
    let (first, last) = (block(unit, *range.start()), block(unit, *range.end()));
    std::iter::successors(Some(first), |b| (*b < last).then_some(b * 2)).collect()
}

/// The line of an arrow from `a` to `b` of width `width`, as far as it
/// goes (into its head, so that no seam shows), and its head of `kind`:
/// the tip and the two corners.
pub fn arrow(a: Pos2, b: Pos2, width: f32, kind: ArrowHead) -> (Pos2, [Pos2; 3]) {
    let d = b - a;
    let len = d.length();
    if len < 1e-3 {
        return (b, [b; 3]);
    }
    let dir = d / len;
    let (length, half) = kind.proportions();
    let head = (width * length).min(len);
    let base = b - dir * head;
    let across = dir.rot90() * head * half;
    (base + dir * head * 0.3, [b, base + across, base - across])
}

/// How far `p` is from the segment `a`-`b`.
pub fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = if d.length_sq() > 0.0 { ((p - a).dot(d) / d.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + d * t)
}

/// `p` is within `reach` of the outline of the frame `r`.
pub fn near_frame(r: Rect, p: Pos2, reach: f32) -> bool {
    r.expand(reach).contains(p) && !r.shrink(reach).contains(p)
}

/// `p` is within `reach` of the outline of the ellipse in `r`.
pub fn near_ellipse(r: Rect, p: Pos2, reach: f32) -> bool {
    let radius = r.size() / 2.0;
    if radius.x <= reach || radius.y <= reach {
        return r.expand(reach).contains(p);
    }
    let q = p - r.center();
    let at = vec2(q.x / radius.x, q.y / radius.y).length();
    (at - 1.0).abs() * radius.min_elem() <= reach
}

/// `v` within `lo`..`hi`, `lo` when the bounds cross (`f32::clamp` panics).
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.min(hi).max(lo)
}

// ---- Pixelation ----

/// The average of the block at cell (`cx`, `cy`) of `block` pixels of the
/// RGBA `data` of `w` x `h`, cut at the image's edges.
fn average(data: &[u8], w: u32, h: u32, cx: u32, cy: u32, block: u32) -> [u8; 4] {
    let (x0, y0) = (cx * block, cy * block);
    let (x1, y1) = ((x0 + block).min(w), (y0 + block).min(h));
    let mut sum = [0u64; 4];
    for y in y0..y1 {
        let row = &data[((y * w + x0) * 4) as usize..((y * w + x1) * 4) as usize];
        for p in row.as_chunks::<4>().0 {
            for c in 0..4 {
                sum[c] += u64::from(p[c]);
            }
        }
    }
    let n = (u64::from(x1 - x0) * u64::from(y1 - y0)).max(1);
    sum.map(|s| ((s + n / 2) / n) as u8)
}

/// Every block's average: the mosaic of the whole image, a pixel per
/// block, and its width and height. The screen shows a pixelated area as
/// this, enlarged and cut to the area.
pub fn mosaic(data: &[u8], w: u32, h: u32, block: u32) -> (Vec<u8>, u32, u32) {
    let (cw, ch) = (w.div_ceil(block), h.div_ceil(block));
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for cy in 0..ch {
        for cx in 0..cw {
            out.extend(average(data, w, h, cx, cy, block));
        }
    }
    (out, cw, ch)
}

/// The blocks over `area` (x, y, width, height) of `data` and their
/// averages, as `pixelate` fills them.
fn pixelated(data: &[u8], w: u32, h: u32, area: [u32; 4], block: u32) -> Vec<([u32; 4], [u8; 4])> {
    let [x, y, aw, ah] = area;
    let (x1, y1) = ((x + aw).min(w), (y + ah).min(h));
    let mut out = Vec::new();
    if x >= x1 || y >= y1 {
        return out;
    }
    for cy in y / block..=(y1 - 1) / block {
        for cx in x / block..=(x1 - 1) / block {
            let colour = average(data, w, h, cx, cy, block);
            let (bx0, by0) = ((cx * block).max(x), (cy * block).max(y));
            let (bx1, by1) = ((cx * block + block).min(x1), (cy * block + block).min(y1));
            out.push(([bx0, by0, bx1, by1], colour));
        }
    }
    out
}

fn fill(data: &mut [u8], w: u32, [x0, y0, x1, y1]: [u32; 4], colour: [u8; 4]) {
    for y in y0..y1 {
        for p in data[((y * w + x0) * 4) as usize..((y * w + x1) * 4) as usize].as_chunks_mut::<4>().0 {
            *p = colour;
        }
    }
}

// ---- Text ----


/// The colour of the outline around text of `colour`: black around light
/// colours, white around dark ones.
pub fn outline(colour: Color32) -> Color32 {
    let [r, g, b, _] = colour.to_array();
    let luma = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
    if luma > 140.0 { Color32::BLACK } else { Color32::WHITE }
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The SVG of `text` in `font` with its point at the origin: each line a
/// `tspan`, the first line's top at 0; outlined in `stroke`, if any.
fn text_svg(text: &str, px: f32, colour: Color32, font: &TextFont, stroke: Option<Color32>) -> String {
    let line = px * 1.25;
    let mut spans = String::new();
    let mut dy = 0.0;
    for (i, l) in text.split('\n').enumerate() {
        let l = l.trim_end_matches('\r');
        if i > 0 {
            dy += line;
        }
        if l.trim().is_empty() {
            continue;
        }
        spans += &format!("<tspan x=\"0\" y=\"{}\">{}</tspan>", px * 0.8 + dy, escape(l));
    }
    let stroke = match stroke {
        None => String::new(),
        Some(s) => format!(" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\" paint-order=\"stroke\"", hex(s), px * 0.14),
    };
    // The family quoted as a CSS string (names may have commas), Segoe UI
    // after it for one not installed: usvg's own fallback is a serif.
    let family = font.family.replace('\\', "").replace('\'', "\\'");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><text font-family=\"'{}', '{FALLBACK_FAMILY}'\" font-weight=\"{}\" font-style=\"{}\" font-size=\"{px}\" fill=\"{}\"{stroke} xml:space=\"preserve\">{spans}</text></svg>",
        escape(&family),
        if font.bold { "bold" } else { "normal" },
        if font.italic { "italic" } else { "normal" },
        hex(colour),
    )
}

/// `text` drawn as a mark in `style` and `font`, `contrast` the outline's
/// or the plate's letters' colour (black or white when None): its pixels,
/// premultiplied RGBA, and where their top left corner is from the mark's
/// point. None for no text.
pub fn render_text(
    text: &str,
    px: f32,
    colour: Color32,
    style: TextStyle,
    contrast: Option<Color32>,
    font: &TextFont,
) -> Option<(tiny_skia::Pixmap, [i32; 2])> {
    let contrast = contrast.unwrap_or_else(|| outline(colour));
    match style {
        TextStyle::Outline => glyphs(text, px, colour, font, Some(contrast)),
        TextStyle::Plain => glyphs(text, px, colour, font, None),
        TextStyle::Plate => plate(text, px, colour, contrast, font),
    }
}

/// The text in `letters` on a rounded plate of `colour`, as tall as its
/// lines.
fn plate(text: &str, px: f32, colour: Color32, letters: Color32, font: &TextFont) -> Option<(tiny_skia::Pixmap, [i32; 2])> {
    let (glyphs, [gx, gy]) = glyphs(text, px, letters, font, None)?;
    let lines = text.trim_end().split('\n').count().max(1) as f32;
    let (pad_x, pad_y) = (px * 0.35, px * 0.2);
    // From the capitals' top of the first line to the descenders of the
    // last (the baseline is at 0.8 of the size, see `text_svg`), or the
    // glyphs' own reach in a font that goes beyond.
    let (x0, x1) = (gx as f32 - pad_x, (gx + glyphs.width() as i32) as f32 + pad_x);
    let top = (px * 0.1).min(gy as f32);
    let bottom = (px * 1.02 + (lines - 1.0) * px * 1.25).max((gy + glyphs.height() as i32) as f32);
    let (y0, y1) = (top - pad_y, bottom + pad_y);
    let (ox, oy) = (x0.floor(), y0.min(gy as f32).floor());
    let w = (x1.ceil() - ox).max(1.0) as u32;
    let h = (y1.max((gy + glyphs.height() as i32) as f32).ceil() - oy).max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let r = (px * 0.25).min((y1 - y0) / 2.0);
    let (l, t, rt, b) = (x0 - ox, y0 - oy, x1 - ox, y1 - oy);
    let mut path = tiny_skia::PathBuilder::new();
    path.move_to(l + r, t);
    path.line_to(rt - r, t);
    path.quad_to(rt, t, rt, t + r);
    path.line_to(rt, b - r);
    path.quad_to(rt, b, rt - r, b);
    path.line_to(l + r, b);
    path.quad_to(l, b, l, b - r);
    path.line_to(l, t + r);
    path.quad_to(l, t, l + r, t);
    path.close();
    let identity = tiny_skia::Transform::identity();
    pixmap.fill_path(&path.finish()?, &paint(colour), tiny_skia::FillRule::Winding, identity, None);
    pixmap.draw_pixmap(gx - ox as i32, gy - oy as i32, glyphs.as_ref(), &tiny_skia::PixmapPaint::default(), identity, None);
    Some((pixmap, [ox as i32, oy as i32]))
}

/// The glyphs of `text` alone, outlined in `stroke`, if any.
fn glyphs(text: &str, px: f32, colour: Color32, font: &TextFont, stroke: Option<Color32>) -> Option<(tiny_skia::Pixmap, [i32; 2])> {
    let options = usvg::Options { fontdb: crate::svg::system_fonts(), ..Default::default() };
    let tree = usvg::Tree::from_str(&text_svg(text, px, colour, font, stroke), &options).ok()?;
    let bounds = tree.root().abs_stroke_bounding_box();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let (x0, y0) = (bounds.left().floor(), bounds.top().floor());
    let w = (bounds.right().ceil() - x0).max(1.0) as u32;
    let h = (bounds.bottom().ceil() - y0).max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, tiny_skia::Transform::from_translate(-x0, -y0), &mut pixmap.as_mut());
    Some((pixmap, [x0 as i32, y0 as i32]))
}

// ---- Drawing into the pixels ----

fn paint(colour: Color32) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    let [r, g, b, a] = colour.to_srgba_unmultiplied();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

fn skia_rect(r: Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x, r.max.y)
}

fn premultiply(rgba: &mut [u8]) {
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * a + 127) / 255) as u8;
            }
        }
    }
}

/// `img` (the image as shown, turned, the size the marks were placed on)
/// with `marks` drawn into it: the pixelated areas first, then the rest
/// in order. An image without transparency stays without.
pub fn draw(img: DynamicImage, marks: &[Mark]) -> Result<DynamicImage, String> {
    if marks.is_empty() {
        return Ok(img);
    }
    let alpha = img.color().has_alpha();
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let size = vec2(w as f32, h as f32);
    let unit = unit(size);
    let mut data = rgba.into_raw();
    premultiply(&mut data);
    // Every area's blocks from the image as it was, then filled.
    let blocks: Vec<_> = marks
        .iter()
        .filter_map(|m| match m.shape {
            Shape::Pixelate(r) => Some(pixelated(&data, w, h, crate::crop::pixels(r, size), block(unit, m.size))),
            _ => None,
        })
        .flatten()
        .collect();
    for (area, colour) in blocks {
        fill(&mut data, w, area, colour);
    }
    let too_large = || format!("cannot draw on an image of {w}x{h} pixels");
    let mut pixmap = tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h).ok_or_else(too_large)?).ok_or_else(too_large)?;
    for mark in marks {
        let stroke = tiny_skia::Stroke { width: line_width(unit, mark.size), ..Default::default() };
        let identity = tiny_skia::Transform::identity();
        match &mark.shape {
            Shape::Pixelate(_) => {}
            Shape::Rect(r) => {
                if let Some(r) = skia_rect(*r) {
                    pixmap.stroke_path(&tiny_skia::PathBuilder::from_rect(r), &paint(mark.colour), &stroke, identity, None);
                }
            }
            Shape::Ellipse(r) => {
                if let Some(path) = skia_rect(*r).and_then(tiny_skia::PathBuilder::from_oval) {
                    pixmap.stroke_path(&path, &paint(mark.colour), &stroke, identity, None);
                }
            }
            Shape::Line(a, b) => {
                let mut line = tiny_skia::PathBuilder::new();
                line.move_to(a.x, a.y);
                line.line_to(b.x, b.y);
                if let Some(path) = line.finish() {
                    pixmap.stroke_path(&path, &paint(mark.colour), &stroke, identity, None);
                }
            }
            Shape::Pen(points) => {
                let mut line = tiny_skia::PathBuilder::new();
                if let Some(first) = points.first() {
                    line.move_to(first.x, first.y);
                }
                for p in &points[1.min(points.len())..] {
                    line.line_to(p.x, p.y);
                }
                let round = tiny_skia::Stroke { line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..stroke.clone() };
                if let Some(path) = line.finish() {
                    pixmap.stroke_path(&path, &paint(mark.colour), &round, identity, None);
                }
            }
            Shape::Arrow(a, b) => {
                let (end, head) = arrow(*a, *b, stroke.width, mark.head);
                let mut line = tiny_skia::PathBuilder::new();
                line.move_to(a.x, a.y);
                line.line_to(end.x, end.y);
                if let Some(path) = line.finish() {
                    pixmap.stroke_path(&path, &paint(mark.colour), &stroke, identity, None);
                }
                let mut tip = tiny_skia::PathBuilder::new();
                tip.move_to(head[0].x, head[0].y);
                tip.line_to(head[1].x, head[1].y);
                tip.line_to(head[2].x, head[2].y);
                tip.close();
                if let Some(path) = tip.finish() {
                    pixmap.fill_path(&path, &paint(mark.colour), tiny_skia::FillRule::Winding, identity, None);
                }
            }
            Shape::Text(p, text) => {
                if let Some((image, [dx, dy])) = render_text(text, font_size(unit, mark.size), mark.colour, mark.text, mark.contrast, &mark.font) {
                    let at = p.round();
                    let pixmap_paint = tiny_skia::PixmapPaint::default();
                    pixmap.draw_pixmap(at.x as i32 + dx, at.y as i32 + dy, image.as_ref(), &pixmap_paint, identity, None);
                }
            }
        }
    }
    let mut data = pixmap.take();
    crate::svg::unpremultiply(&mut data);
    let rgba = image::RgbaImage::from_raw(w, h, data).ok_or_else(too_large)?;
    Ok(if alpha { DynamicImage::ImageRgba8(rgba) } else { DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).into_rgb8()) })
}

// ---- The state while marking ----

/// The mosaics of the image for each of `blocks`: the block's side, the
/// averages and the mosaic's width and height, read on a thread.
pub type Mosaics = Vec<(u32, Vec<u8>, u32, u32)>;

/// Read the image of `job` (as shown, no marks) and make its mosaics for
/// `blocks`; on a thread, which has COM initialised.
pub fn read_mosaics(job: &crate::edit::Job, blocks: &[u32]) -> Result<Mosaics, String> {
    // The fonts, for the bar's list and the first text, loaded here rather
    // than on the UI thread.
    let _ = families();
    let img = crate::edit::render(job)?;
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut data = rgba.into_raw();
    premultiply(&mut data);
    Ok(blocks
        .iter()
        .map(|&b| {
            let (m, cw, ch) = mosaic(&data, w, h, b);
            (b, m, cw, ch)
        })
        .collect())
}

/// Text drawn for the screen, by its text, size (`f32::to_bits`), colour,
/// style, second colour and font.
pub type TextKey = (String, u32, Color32, TextStyle, Option<Color32>, TextFont);
/// Its texture, the offset of its top left corner from the mark's point
/// and its size, in image pixels.
pub type DrawnText = (TextureHandle, [i32; 2], [u32; 2]);

/// A drag on the image while marking.
#[derive(Clone, Debug)]
pub enum Drag {
    /// Drawing the mark `index` from the point where the drag started.
    New { index: usize, from: Pos2 },
    /// Moving the mark `index`, as it was, from that point.
    Move { index: usize, start: Mark, from: Pos2 },
    /// An edge or corner of the frame of `index`.
    Edges { index: usize, grip: crate::crop::Grip, start: Rect, from: Pos2 },
    /// An end of the line or arrow `index`: the second (the head) if `head`.
    End { index: usize, head: bool },
}

/// Text being typed: of the mark `index`, or of a new one at `at`.
pub struct Typing {
    pub index: Option<usize>,
    pub at: Pos2,
    pub text: String,
    /// The field takes the focus in its first frame.
    pub focus: bool,
}

/// The image being marked and its marks.
pub struct Annotating {
    pub path: std::path::PathBuf,
    pub tool: Tool,
    /// The colour of new shapes and of new text (`slot`), kept apart: text
    /// starts black, the shapes red.
    pub colours: [Color32; 2],
    /// The size of each `Measure` new marks take.
    pub sizes: [f32; 3],
    pub text_style: TextStyle,
    pub font: TextFont,
    /// Text's outline or plate letters (`Mark::contrast`).
    pub contrast: Option<Color32>,
    pub head: ArrowHead,
    /// What the font list is narrowed to.
    pub font_filter: String,
    pub marks: Vec<Mark>,
    pub selected: Option<usize>,
    /// The mark whose row in the list is under the pointer, outlined on
    /// the image.
    pub hovered: Option<usize>,
    /// The marks before each change, for Ctrl+Z.
    pub undo: Vec<Vec<Mark>>,
    pub drag: Option<Drag>,
    pub typing: Option<Typing>,
    /// The view's zoom and panning before marking, given back after.
    pub zoom: Zoom,
    pub offset: Vec2,
    /// When Esc last asked to discard the marks: a second one does.
    pub discard_asked: Option<std::time::Instant>,
    /// The mosaics being read, then their blocks, textures and sizes in
    /// cells.
    pub mosaics_rx: Option<mpsc::Receiver<Result<Mosaics, String>>>,
    pub mosaics: Vec<(u32, TextureHandle, [u32; 2])>,
    /// The unit of the marks' sizes on this image (`unit`).
    pub unit: f32,
    /// The blocks of the sizes (`blocks`).
    pub blocks: Vec<u32>,
    /// The mark whose colour or size is being changed: a slider dragged
    /// or the picker moved is one change for Ctrl+Z.
    restyling: Option<usize>,
    /// Text drawn for the screen, by what it was drawn of; those not drawn
    /// in a frame are dropped.
    pub texts: std::collections::HashMap<TextKey, DrawnText>,
}

/// The tool, colours, sizes, text style, font, text's second colour and
/// arrow head marking starts with.
pub type Style = (Tool, [Color32; 2], [f32; 3], TextStyle, TextFont, Option<Color32>, ArrowHead);

/// The style of the first marking.
pub fn default_style() -> Style {
    let sizes = [Measure::Line, Measure::Text, Measure::Block].map(Measure::default_size);
    (Tool::Rect, DEFAULT_COLOURS, sizes, TextStyle::Plain, TextFont::default(), None, ArrowHead::Normal)
}

/// The colours marks start with: red shapes, black text.
const DEFAULT_COLOURS: [Color32; 2] = [COLOURS[0], Color32::BLACK];

/// Which of `Annotating::colours` marks of `tool` take.
fn slot(tool: Tool) -> usize {
    usize::from(tool == Tool::Text)
}

/// Changes kept for Ctrl+Z.
const UNDO_STEPS: usize = 100;

impl Annotating {
    pub fn new(path: std::path::PathBuf, size: Vec2, zoom: Zoom, offset: Vec2) -> Self {
        let unit = unit(size);
        Self {
            path,
            tool: Tool::Rect,
            colours: DEFAULT_COLOURS,
            sizes: [Measure::Line, Measure::Text, Measure::Block].map(Measure::default_size),
            text_style: TextStyle::Plain,
            font: TextFont::default(),
            contrast: None,
            head: ArrowHead::Normal,
            font_filter: String::new(),
            marks: Vec::new(),
            selected: None,
            hovered: None,
            undo: Vec::new(),
            drag: None,
            typing: None,
            zoom,
            offset,
            discard_asked: None,
            mosaics_rx: None,
            mosaics: Vec::new(),
            unit,
            blocks: blocks(unit),
            restyling: None,
            texts: Default::default(),
        }
    }

    /// Keep the marks as they are now for Ctrl+Z, before changing them.
    pub fn remember(&mut self) {
        self.restyling = None;
        self.undo.push(self.marks.clone());
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
    }

    /// The marks before the last change.
    pub fn undo(&mut self) -> bool {
        let Some(marks) = self.undo.pop() else { return false };
        self.marks = marks;
        self.restyling = None;
        self.selected = None;
        self.drag = None;
        self.typing = None;
        true
    }

    /// The colour shown: the chosen mark's, else the tool's.
    pub fn colour(&self) -> Color32 {
        self.colours[slot(self.kind())]
    }

    /// The colour new marks of `tool` take.
    pub fn colour_of(&self, tool: Tool) -> Color32 {
        self.colours[slot(tool)]
    }

    /// Choose `colour` for the tool's kind of marks; the mark chosen takes
    /// it too.
    pub fn set_colour(&mut self, colour: Color32) {
        self.colours[slot(self.kind())] = colour;
        self.restyle(|m| m.colour = colour);
    }

    /// The mark chosen.
    pub fn chosen(&self) -> Option<&Mark> {
        self.selected.and_then(|i| self.marks.get(i))
    }

    /// What the bar's style is of: the chosen mark's kind, else the tool.
    pub fn kind(&self) -> Tool {
        self.chosen().map_or(self.tool, |m| m.shape.tool())
    }

    /// What the size shown measures: the chosen mark's, else the tool's.
    pub fn measure(&self) -> Measure {
        Measure::of_tool(self.kind())
    }

    /// The size new marks of `measure` take.
    pub fn size_of(&self, measure: Measure) -> f32 {
        self.sizes[measure.index()]
    }

    /// The size shown: the chosen mark's, else the tool's.
    pub fn size(&self) -> f32 {
        match self.selected.and_then(|i| self.marks.get(i)) {
            Some(m) => m.size,
            None => self.sizes[Measure::of_tool(self.tool).index()],
        }
    }

    /// Set the size shown; the mark chosen takes it.
    pub fn set_size(&mut self, size: f32) {
        let measure = self.measure();
        let size = measure.clamped(size);
        self.sizes[measure.index()] = size;
        self.restyle(|m| m.size = size);
    }

    /// One step of [ or ].
    pub fn step_size(&mut self, up: bool) {
        let size = self.measure().stepped(self.size(), up);
        self.set_size(size);
    }

    /// Choose how text is drawn; the text chosen takes it.
    pub fn set_text_style(&mut self, style: TextStyle) {
        self.text_style = style;
        self.restyle(|m| {
            if matches!(m.shape, Shape::Text(..)) {
                m.text = style;
            }
        });
    }

    /// Choose the font of text; the text chosen takes it.
    pub fn set_font(&mut self, font: TextFont) {
        self.font = font.clone();
        self.restyle(|m| {
            if matches!(m.shape, Shape::Text(..)) {
                m.font = font.clone();
            }
        });
    }

    /// Choose text's second colour (None: black or white); the text
    /// chosen takes it.
    pub fn set_contrast(&mut self, contrast: Option<Color32>) {
        self.contrast = contrast;
        self.restyle(|m| {
            if matches!(m.shape, Shape::Text(..)) {
                m.contrast = contrast;
            }
        });
    }

    /// Choose the arrow's head; the arrow chosen takes it.
    pub fn set_head(&mut self, head: ArrowHead) {
        self.head = head;
        self.restyle(|m| {
            if matches!(m.shape, Shape::Arrow(..)) {
                m.head = head;
            }
        });
    }

    /// A new mark of `shape` in the style chosen.
    pub fn new_mark(&self, shape: Shape) -> Mark {
        let size = self.sizes[Measure::of(&shape).index()];
        Mark { colour: self.colour_of(shape.tool()), shape, size, text: self.text_style, font: self.font.clone(), head: self.head, contrast: self.contrast }
    }

    /// Choose the mark `i`: the bar shows its colour, size, text style or
    /// head, which new marks then take.
    pub fn select(&mut self, i: usize) {
        let Some(m) = self.marks.get(i) else { return };
        self.selected = Some(i);
        self.colours[slot(m.shape.tool())] = m.colour;
        self.sizes[Measure::of(&m.shape).index()] = m.size;
        match m.shape {
            Shape::Text(..) => {
                self.text_style = m.text;
                self.font = m.font.clone();
                self.contrast = m.contrast;
            }
            Shape::Arrow(..) => self.head = m.head,
            _ => {}
        }
    }

    /// Edit the text of mark `i` in the field below it.
    pub fn edit_text(&mut self, i: usize) {
        let Some(Shape::Text(at, text)) = self.marks.get(i).map(|m| &m.shape) else { return };
        let (at, text) = (*at, text.clone());
        self.select(i);
        self.typing = Some(Typing { index: Some(i), at, text, focus: true });
    }

    /// Take out the mark `i`.
    pub fn delete(&mut self, i: usize) {
        self.finish_typing();
        if i >= self.marks.len() {
            return;
        }
        self.remember();
        self.marks.remove(i);
        self.selected = match self.selected {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
    }

    /// Mark `i` one place later in the order (drawn over the next one), or
    /// earlier; the choice stays on the same marks.
    pub fn reorder(&mut self, i: usize, later: bool) {
        self.finish_typing();
        let Some(j) = (if later { i.checked_add(1) } else { i.checked_sub(1) }).filter(|&j| j < self.marks.len()) else { return };
        if i >= self.marks.len() {
            return;
        }
        self.remember();
        self.marks.swap(i, j);
        self.selected = match self.selected {
            Some(s) if s == i => Some(j),
            Some(s) if s == j => Some(i),
            s => s,
        };
    }

    /// The tool and its style, as a preset.
    pub fn preset(&self) -> Preset {
        let size = self.sizes[Measure::of_tool(self.tool).index()];
        Preset { tool: self.tool, colour: self.colour_of(self.tool), size, text: self.text_style, font: self.font.clone(), head: self.head, contrast: self.contrast }
    }

    /// Take the tool and the style of `preset`.
    pub fn apply(&mut self, preset: &Preset) {
        self.finish_typing();
        self.selected = None;
        self.tool = preset.tool;
        self.colours[slot(preset.tool)] = preset.colour;
        self.sizes[Measure::of_tool(preset.tool).index()] = Measure::of_tool(preset.tool).clamped(preset.size);
        match preset.tool {
            Tool::Text => {
                self.text_style = preset.text;
                self.font = preset.font.clone();
                self.contrast = preset.contrast;
            }
            Tool::Arrow => self.head = preset.head,
            _ => {}
        }
    }

    /// The tool and the style kept for the next image.
    pub fn style(&self) -> Style {
        (self.tool, self.colours, self.sizes, self.text_style, self.font.clone(), self.contrast, self.head)
    }

    pub fn set_style(&mut self, (tool, colours, sizes, text, font, contrast, head): Style) {
        (self.tool, self.colours, self.sizes, self.text_style, self.font, self.contrast, self.head) = (tool, colours, sizes, text, font, contrast, head);
    }

    /// The mark chosen changed by `change`: kept for Ctrl+Z once for a run
    /// of changes to it (a slider dragged).
    fn restyle(&mut self, change: impl Fn(&mut Mark)) {
        let Some(i) = self.selected.filter(|&i| i < self.marks.len()) else { return };
        let mut mark = self.marks[i].clone();
        change(&mut mark);
        if mark != self.marks[i] {
            if self.restyling != Some(i) {
                self.remember();
                self.restyling = Some(i);
            }
            self.marks[i] = mark;
        }
    }

    /// Take out the mark chosen.
    pub fn delete_selected(&mut self) -> bool {
        let Some(i) = self.selected.take().filter(|&i| i < self.marks.len()) else { return false };
        self.remember();
        self.marks.remove(i);
        true
    }

    /// Take out every mark, in one step for Ctrl+Z.
    pub fn clear(&mut self) -> bool {
        self.typing = None;
        if self.marks.is_empty() {
            return false;
        }
        self.remember();
        self.marks.clear();
        self.selected = None;
        self.drag = None;
        true
    }

    /// The text typed made a mark (or changed one, or took an emptied one
    /// out).
    pub fn finish_typing(&mut self) {
        let Some(typing) = self.typing.take() else { return };
        let text = typing.text.trim_end().to_string();
        match typing.index.filter(|&i| i < self.marks.len()) {
            Some(i) => {
                let Shape::Text(at, old) = &self.marks[i].shape else { return };
                if *old == text {
                    return;
                }
                let at = *at;
                self.remember();
                if text.trim().is_empty() {
                    self.marks.remove(i);
                    self.selected = None;
                } else {
                    self.marks[i].shape = Shape::Text(at, text);
                }
            }
            None if !text.trim().is_empty() => {
                self.remember();
                self.marks.push(self.new_mark(Shape::Text(typing.at.round(), text)));
                self.select(self.marks.len() - 1);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn sizes_follow_the_image() {
        assert_eq!(unit(vec2(600.0, 400.0)), 1.5);
        assert_eq!(unit(vec2(4032.0, 3024.0)), 6.72);
        // Blocks are powers of two, at least 4.
        assert_eq!(block(1.5, 0.5), 4);
        assert_eq!([3.4, 6.8, 13.6].map(|s| block(3.2, s)), [8, 16, 32]);
        assert_eq!(blocks(3.2), vec![4, 8, 16, 32, 64, 128, 256, 512]);
        assert_eq!(blocks(10.0).last(), Some(&1024));
        assert_eq!(line_width(3.0, 2.0), 6.0);
        assert_eq!(font_size(2.0, 900.0), 300.0);
        assert_eq!(Measure::Line.stepped(40.0, true), 40.0);
        assert_eq!(Measure::Line.stepped(2.0, true), 2.5);
        assert_eq!(Measure::Block.stepped(6.8, true), 13.6);
        assert_eq!(Measure::Block.pixels(3.2, 6.8), 16.0);
    }

    #[test]
    fn arrows_have_their_head_at_the_end() {
        let (end, head) = arrow(pos2(0.0, 0.0), pos2(100.0, 0.0), 4.0, ArrowHead::Normal);
        assert_eq!(head[0], pos2(100.0, 0.0));
        let mut corners = [head[1], head[2]].map(|p| (p.x, p.y));
        corners.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(corners, [(82.0, -9.0), (82.0, 9.0)]);
        assert!(end.x > 82.0 && end.x < 100.0);
        // A short arrow is all head.
        let (_, head) = arrow(pos2(0.0, 0.0), pos2(5.0, 0.0), 4.0, ArrowHead::Normal);
        assert_eq!(head[1].x, 0.0);
        assert_eq!(arrow(pos2(1.0, 1.0), pos2(1.0, 1.0), 4.0, ArrowHead::Normal).1, [pos2(1.0, 1.0); 3]);
        // Sharper heads are longer and narrower.
        let shape = |kind| {
            let (_, [_, c1, c2]) = arrow(pos2(0.0, 0.0), pos2(100.0, 0.0), 4.0, kind);
            (100.0 - c1.x, c1.distance(c2))
        };
        let [blunt, normal, sharp] = ArrowHead::ALL.map(shape);
        assert!(blunt.0 < normal.0 && normal.0 < sharp.0, "{blunt:?} {normal:?} {sharp:?}");
        assert!(blunt.1 > normal.1 && normal.1 > sharp.1, "{blunt:?} {normal:?} {sharp:?}");
    }

    /// Text starts black and plain, and keeps its colour apart from the
    /// shapes'.
    #[test]
    fn text_has_its_own_colour() {
        let mut a = Annotating::new(Default::default(), vec2(400.0, 300.0), Zoom::Fit, Vec2::ZERO);
        a.set_style(default_style());
        let text = a.new_mark(Shape::Text(Pos2::ZERO, "a".into()));
        assert_eq!((text.colour, text.text), (Color32::BLACK, TextStyle::Plain));
        assert_eq!(a.new_mark(Shape::Rect(Rect::ZERO)).colour, COLOURS[0]);
        a.tool = Tool::Text;
        a.set_colour(COLOURS[3]);
        a.tool = Tool::Arrow;
        assert_eq!((a.colour(), a.colour_of(Tool::Text)), (COLOURS[0], COLOURS[3]));
    }

    /// Moving a mark in the order and deleting one keep the choice on the
    /// same mark.
    #[test]
    fn the_list_reorders_and_deletes() {
        let mut a = Annotating::new(Default::default(), vec2(400.0, 300.0), Zoom::Fit, Vec2::ZERO);
        for x in [10.0, 20.0, 30.0] {
            a.marks.push(a.new_mark(Shape::Line(pos2(x, 0.0), pos2(x, 50.0))));
        }
        let at = |a: &Annotating, i: usize| a.marks[i].ends().unwrap().0.x;
        a.select(0);
        a.reorder(0, true);
        assert_eq!((at(&a, 0), at(&a, 1), a.selected), (20.0, 10.0, Some(1)));
        // The top cannot go higher, nor the bottom lower.
        a.reorder(2, true);
        a.reorder(0, false);
        assert_eq!(a.undo.len(), 1);
        a.delete(0);
        assert_eq!((a.marks.len(), a.selected, at(&a, 0)), (2, Some(0), 10.0));
        a.delete(0);
        assert_eq!(a.selected, None);
        assert!(a.undo());
        assert_eq!(a.marks.len(), 2);
        // Clearing is one step back.
        a.select(1);
        assert!(a.clear());
        assert_eq!((a.marks.len(), a.selected), (0, None));
        assert!(!a.clear());
        assert!(a.undo());
        assert_eq!(a.marks.len(), 2);
    }

    #[test]
    fn hits() {
        let r = Rect::from_min_max(pos2(10.0, 10.0), pos2(50.0, 30.0));
        assert!(near_frame(r, pos2(10.0, 20.0), 2.0));
        assert!(!near_frame(r, pos2(30.0, 20.0), 2.0));
        assert!(near_ellipse(r, pos2(30.0, 10.5), 2.0));
        assert!(!near_ellipse(r, pos2(30.0, 20.0), 2.0));
        assert!(!near_ellipse(r, pos2(11.0, 11.0), 2.0));
        assert_eq!(segment_distance(pos2(5.0, 3.0), pos2(0.0, 0.0), pos2(10.0, 0.0)), 3.0);
        assert_eq!(segment_distance(pos2(-4.0, 3.0), pos2(0.0, 0.0), pos2(10.0, 0.0)), 5.0);
    }

    #[test]
    fn marks_move_within_the_image() {
        let m = Mark { shape: Shape::Rect(Rect::from_min_max(pos2(10.0, 10.0), pos2(20.0, 20.0))), colour: COLOURS[0], size: 2.0, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None };
        let moved = m.moved(vec2(-50.0, 5.0), vec2(100.0, 100.0));
        assert_eq!(moved.frame(), Some(Rect::from_min_max(pos2(0.0, 15.0), pos2(10.0, 25.0))));
        let arrow = Mark { shape: Shape::Arrow(pos2(90.0, 50.0), pos2(95.0, 60.0)), ..m.clone() };
        assert_eq!(arrow.moved(vec2(20.0, 0.0), vec2(100.0, 100.0)).shape, Shape::Arrow(pos2(95.0, 50.0), pos2(100.0, 60.0)));
        assert!(Mark { shape: Shape::Text(Pos2::ZERO, " \n".into()), ..m }.is_tiny(1.0));
    }

    /// A mosaic's blocks are the averages of the image's blocks from its
    /// corner, those at the edges cut; an area takes the blocks it touches.
    #[test]
    fn pixelation_follows_the_image_grid() {
        let (w, h) = (6u32, 5u32);
        let data: Vec<u8> = (0..w * h).flat_map(|i| [(i * 8) as u8, 0, 0, 255]).collect();
        let (m, cw, ch) = mosaic(&data, w, h, 4);
        assert_eq!((cw, ch), (2, 2));
        // The first block: rows 0-3, columns 0-3: values 8 * (0..4 + 6 * 0..4).
        let first = (0..4).flat_map(|y| (0..4).map(move |x| (x + 6 * y) * 8)).sum::<u32>() as f32 / 16.0;
        assert_eq!(m[0], first.round() as u8);
        // The last block is the one pixel at (4..6, 4).
        assert_eq!(m[12], (((4 + 24) * 8 + (5 + 24) * 8) as f32 / 2.0).round() as u8);
        let blocks = pixelated(&data, w, h, [3, 1, 2, 2], 4);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], ([3, 1, 4, 3], [m[0], 0, 0, 255]));
        assert_eq!(blocks[1].0, [4, 1, 5, 3]);
    }

    #[test]
    fn drawing_keeps_the_image_and_its_kind() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(200, 100, image::Rgb([10, 20, 30])));
        let marks = [
            Mark { shape: Shape::Rect(Rect::from_min_max(pos2(20.0, 20.0), pos2(60.0, 60.0))), colour: COLOURS[0], size: 2.0, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Arrow(pos2(100.0, 80.0), pos2(180.0, 20.0)), colour: COLOURS[3], size: 3.5, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Pixelate(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0))), colour: COLOURS[0], size: 3.4, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
        ];
        let out = draw(img, &marks).unwrap();
        let rgb = out.as_rgb8().expect("still RGB");
        assert_eq!(rgb.dimensions(), (200, 100));
        // On the frame's line, inside it, beyond it.
        assert_eq!(rgb.get_pixel(20, 40).0, [0xe5, 0x2b, 0x2b]);
        assert_eq!(rgb.get_pixel(40, 40).0, [10, 20, 30]);
        assert_eq!(rgb.get_pixel(5, 95).0, [10, 20, 30]);
        // The arrow's head, blue.
        assert_eq!(rgb.get_pixel(176, 24).0, [0x1e, 0x7c, 0xf0]);
    }

    /// Marks of every kind on `QVIEW_BENCH_FILE`, as saving draws them,
    /// into `QVIEW_MARKS_OUT` (a PNG), timed.
    #[test]
    #[ignore]
    fn marks_sheet() {
        let src = std::path::PathBuf::from(std::env::var("QVIEW_BENCH_FILE").expect("QVIEW_BENCH_FILE"));
        let _com = crate::win::com_init();
        let job = crate::edit::Job {
            src: src.clone(),
            dst: Default::default(),
            size: [0, 0],
            turns: 0,
            flip: false,
            crop: None,
            marks: Vec::new(),
        };
        let t = std::time::Instant::now();
        let img = crate::edit::render(&job).unwrap();
        let decoded = t.elapsed().as_secs_f64() * 1e3;
        let (w, h) = (img.width() as f32, img.height() as f32);
        let blocks = blocks(unit(vec2(w, h)));
        let mut data = img.to_rgba8().into_raw();
        let t = std::time::Instant::now();
        premultiply(&mut data);
        for &b in &blocks {
            let _ = mosaic(&data, w as u32, h as u32, b);
        }
        println!("decoded in {decoded:.0} ms, mosaics of {blocks:?} in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let t = std::time::Instant::now();
        let first = render_text("Текст", 40.0, COLOURS[0], TextStyle::Outline, None, &TextFont::default()).is_some();
        println!("first text (the font loaded) in {:.1} ms: {first}", t.elapsed().as_secs_f64() * 1e3);
        let t = std::time::Instant::now();
        let _ = render_text("Ещё текст, длиннее", 40.0, COLOURS[1], TextStyle::Plate, None, &TextFont::default());
        println!("next text in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let at = |x: f32, y: f32| pos2(w * x, h * y);
        let frame = |x0, y0, x1, y1| Rect::from_min_max(at(x0, y0), at(x1, y1));
        let mut marks = vec![
            Mark { shape: Shape::Pixelate(frame(0.05, 0.55, 0.45, 0.95)), colour: COLOURS[0], size: 3.4, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Pixelate(frame(0.55, 0.55, 0.95, 0.95)), colour: COLOURS[0], size: 13.6, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Rect(frame(0.05, 0.05, 0.3, 0.3)), colour: COLOURS[0], size: 3.4, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Ellipse(frame(0.35, 0.05, 0.6, 0.3)), colour: COLOURS[1], size: 2.0, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Arrow(at(0.9, 0.05), at(0.65, 0.3)), colour: COLOURS[3], size: 3.5, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark { shape: Shape::Line(at(0.6, 0.45), at(0.95, 0.5)), colour: COLOURS[2], size: 6.0, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None },
            Mark {
                shape: Shape::Pen((0..200).map(|i| i as f32 / 200.0).map(|t| at(0.5 + 0.4 * t, 0.2 + 0.05 * (t * 25.0).sin())).collect()),
                colour: COLOURS[4],
                size: 1.5,
                text: TextStyle::Outline,
                font: TextFont::default(),
                head: ArrowHead::Normal,
                contrast: None,
            },
        ];
        for (i, &colour) in COLOURS.iter().enumerate() {
            let y = 0.35 + i as f32 * 0.03;
            marks.push(Mark { shape: Shape::Text(at(0.05, y).round(), format!("Текст {} — Text", i + 1)), colour, size: [6.6, 9.6, 14.1][i % 3], text: TextStyle::ALL[i % 3], font: TextFont::default(), head: ArrowHead::Normal, contrast: None });
        }
        marks.push(Mark { shape: Shape::Text(at(0.55, 0.35).round(), "Две строки\nTwo lines".into()), colour: COLOURS[0], size: 14.1, text: TextStyle::Plate, font: TextFont::default(), head: ArrowHead::Normal, contrast: None });
        for (k, (family, bold, italic)) in [("Times New Roman", false, true), ("Courier New", true, false), ("Comic Sans MS", false, false)].into_iter().enumerate() {
            let font = TextFont { family: family.into(), bold, italic };
            let shape = Shape::Text(at(0.05, 0.55 + k as f32 * 0.06).round(), format!("{family} — Шрифт"));
            marks.push(Mark { shape, colour: COLOURS[4], size: 9.6, text: TextStyle::ALL[k], font, head: ArrowHead::Normal, contrast: None });
        }
        let t = std::time::Instant::now();
        let out = draw(img, &marks).unwrap();
        println!("{} marks drawn on {w}x{h} in {:.1} ms", marks.len(), t.elapsed().as_secs_f64() * 1e3);
        out.save(std::env::var("QVIEW_MARKS_OUT").expect("QVIEW_MARKS_OUT")).unwrap();
    }

    #[test]
    fn text_is_drawn_below_and_right_of_its_point() {
        let (pixmap, [dx, dy]) = render_text("Ab\nЖ", 40.0, COLOURS[1], TextStyle::Outline, None, &TextFont::default()).expect("a font");
        assert!(dx > -6 && dx < 6, "{dx}");
        assert!(dy > -6 && dy < 12, "{dy}");
        // Two lines.
        assert!(pixmap.height() > 80, "{}", pixmap.height());
        assert!(pixmap.pixels().iter().any(|p| p.alpha() == 255));
        assert!(render_text(" \n ", 40.0, COLOURS[1], TextStyle::Plate, None, &TextFont::default()).is_none());
        let svg = text_svg("a<b & c", 10.0, COLOURS[0], &TextFont::default(), Some(Color32::WHITE));
        assert!(svg.contains(">a&lt;b &amp; c<") && svg.contains("stroke=\"#ffffff\""));
        assert!(!text_svg("a", 10.0, COLOURS[0], &TextFont::default(), None).contains("stroke="));
        let odd = TextFont { family: "A's \"B\", C".into(), bold: false, italic: true };
        let svg = text_svg("a", 10.0, COLOURS[0], &odd, None);
        assert!(svg.contains("font-family=\"'A\\'s &quot;B&quot;, C', 'Segoe UI'\" font-weight=\"normal\" font-style=\"italic\""), "{svg}");
        // A plate is larger than its glyphs and covers its corners' middle.
        let (plain, _) = render_text("Ab", 40.0, COLOURS[0], TextStyle::Plain, None, &TextFont::default()).unwrap();
        let (plate, [px, py]) = render_text("Ab", 40.0, COLOURS[0], TextStyle::Plate, None, &TextFont::default()).unwrap();
        assert!(plate.width() > plain.width() && plate.height() >= plain.height());
        assert!(px < 0 && py < 4);
        assert_eq!(plate.pixel(plate.width() / 2, 2).map(|c| c.alpha()), Some(255));
    }

    /// Other families draw other glyphs; one not installed draws Segoe UI.
    #[test]
    fn fonts_are_chosen() {
        assert!(families().iter().any(|f| f == "Segoe UI"), "{:?}", &families()[..families().len().min(10)]);
        let draw = |family: &str, bold| {
            let font = TextFont { family: family.into(), bold, italic: false };
            render_text("Hamburgefonstiv", 40.0, COLOURS[0], TextStyle::Plain, None, &font).unwrap().0.width()
        };
        let segoe = draw("Segoe UI", true);
        assert_ne!(draw("Segoe UI", false), segoe);
        assert_ne!(draw("Courier New", true), segoe);
        assert_eq!(draw("No Such Font", true), segoe);
    }

    #[test]
    fn presets_are_kept_as_lines() {
        let font = TextFont::default();
        let p = |tool, colour, size, text| Preset { tool, colour, size, text, font: font.clone(), head: ArrowHead::Normal, contrast: None };
        let presets = [
            p(Tool::Arrow, COLOURS[0], 3.0, TextStyle::Outline),
            p(Tool::Pen, COLOURS[1], 1.5, TextStyle::Outline),
            p(Tool::Text, COLOURS[0], 9.6, TextStyle::Plate),
            p(Tool::Pixelate, COLOURS[0], 13.6, TextStyle::Outline),
        ];
        for p in &presets {
            assert_eq!(Preset::parse(&p.to_line()), Some(p.clone()), "{}", p.to_line());
        }
        let p = Preset::parse("text #102030 999 plate").unwrap();
        assert_eq!((p.tool, p.colour, p.size, p.text), (Tool::Text, Color32::from_rgb(0x10, 0x20, 0x30), 150.0, TextStyle::Plate));
        assert_eq!(Preset::parse("arrow #ff0000 3").map(|p| p.text), Some(TextStyle::Outline));
        // A font of several words, and a preset of before fonts.
        let p = Preset { font: TextFont { family: "Times New Roman".into(), bold: false, italic: true }, ..presets[2].clone() };
        assert_eq!(p.to_line(), "text #e52b2b 9.6 plate -i Times New Roman");
        assert_eq!(Preset::parse(&p.to_line()), Some(p));
        assert_eq!(Preset::parse("text #e52b2b 9.6 plate").map(|p| p.font), Some(TextFont::default()));
        // An arrow's head, and an arrow of before heads.
        let p = Preset { head: ArrowHead::Sharp, ..presets[0].clone() };
        assert_eq!(p.to_line(), "arrow #e52b2b 3 outline sharp -- Segoe UI");
        assert_eq!(Preset::parse(&p.to_line()), Some(p));
        assert_eq!(Preset::parse("arrow #e52b2b 3 outline b- Segoe UI").map(|p| p.head), Some(ArrowHead::Normal));
        assert!(!presets[2].to_line().contains("normal"));
        // Text's second colour, kept only when chosen.
        let p = Preset { contrast: Some(COLOURS[3]), ..presets[2].clone() };
        assert_eq!(p.to_line(), "text #e52b2b 9.6 plate #1e7cf0 -- Segoe UI");
        assert_eq!(Preset::parse(&p.to_line()), Some(p));
        assert_eq!(Preset::parse("text #e52b2b 9.6 plate -- Segoe UI").map(|p| p.contrast), Some(None));
        assert!(Preset::parse("brush #ff0000 3").is_none());
        assert!(Preset::parse("").is_none());
    }
}
