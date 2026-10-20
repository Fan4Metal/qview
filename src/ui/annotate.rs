//! Marking on screen (A): the bar above the image with the tools, the
//! colours, the size, Undo, Save, Save As and Cancel, and the marks over
//! the image, drawn, chosen, moved and resized with the mouse (see
//! `annotate`). Text is typed into a field below its point.

use egui::{
    Color32, ColorImage, CursorIcon, Id, Key, Modifiers, Painter, PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    TextureOptions, Ui, Vec2, pos2, vec2,
};

use super::{TOOLBAR_BG, panel_frame};
use crate::annotate::{self, Annotating, COLOURS, Drag, Mark, Measure, Preset, Shape, TextFont, TextStyle, Tool, Typing};
use crate::app::App;
use crate::crop::{self, Grip};
use crate::input::Cmd;

/// How near a mark or a handle, in points, the pointer takes it.
const REACH: f32 = 8.0;
/// Side of the squares at a chosen frame's corners and edges.
const HANDLE: f32 = 7.0;
/// Marks drawn smaller than this, in points, are dropped.
const MIN_DRAWN: f32 = 4.0;
/// The text field's id: while it has the focus, the keys are its own.
pub const TEXT_ID: &str = "annotate_text";

impl App {
    pub(crate) fn annotate_bar(&mut self, root_ui: &mut Ui) {
        let saving = self.saving();
        let presets = &mut self.mark_presets;
        let Some(a) = &mut self.annotating else { return };
        let mut clicked = None;
        egui::Panel::top("annotate_bar")
            .frame(panel_frame(TOOLBAR_BG, egui::Margin::symmetric(8, 4)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                rows(ui, |ui, part| match part {
                    Part::Right => {
                    if ui.button(tr!("Cancel", "Отмена")).on_hover_text("Esc").clicked() {
                        clicked = Some(Cmd::Annotate);
                    }
                    let save_as = egui::Button::new(tr!("Save As…", "Сохранить как…"));
                    if ui.add_enabled(!saving, save_as).on_hover_text("Ctrl+Shift+S").clicked() {
                        clicked = Some(Cmd::SaveAs);
                    }
                    let save = egui::Button::new(tr!("Save", "Сохранить"));
                    if ui.add_enabled(!saving, save).on_hover_text("Enter, Ctrl+S").clicked() {
                        clicked = Some(Cmd::Save);
                    }
                    if saving {
                        ui.add(egui::Spinner::new());
                    }
                    }
                    Part::Left => {
                        for t in Tool::ALL {
                            if ui.selectable_label(a.tool == t, t.label()).on_hover_text(t.key()).clicked() {
                                clicked = Some(Cmd::MarkTool(t));
                            }
                        }
                        ui.add_space(12.0);
                        if ui.add_enabled(!a.undo.is_empty(), egui::Button::new(tr!("Undo", "Отменить"))).on_hover_text("Ctrl+Z").clicked() {
                            clicked = Some(Cmd::Undo);
                        }
                        let delete = egui::Button::new(tr!("Delete", "Удалить"));
                        if ui.add_enabled(a.selected.is_some(), delete).on_hover_text("Delete").clicked() {
                            clicked = Some(Cmd::Delete);
                        }
                    }
                    // The style: the colours, the size of what is drawn (or
                    // chosen), text's look, the presets.
                    Part::Below => {
                    for (i, &c) in COLOURS.iter().enumerate() {
                        if swatch(ui, c, c == a.colour).on_hover_text(format!("{}", i + 1)).clicked() {
                            clicked = Some(Cmd::MarkColour(i as u8));
                        }
                    }
                    let custom = !COLOURS.contains(&a.colour);
                    let (response, picked) = picker_button(ui, a.colour, custom);
                    response.on_hover_text(tr!("Any colour", "Любой цвет"));
                    if let Some(colour) = picked {
                        a.set_colour(colour);
                    }
                    ui.add_space(16.0);
                    let measure = a.measure();
                    let tip = match measure {
                        Measure::Line => tr!("Width of lines ([ and ])", "Толщина линий ([ и ])"),
                        Measure::Text => tr!("Height of text ([ and ])", "Высота текста ([ и ])"),
                        Measure::Block => tr!("Side of the pixelation's blocks ([ and ])", "Сторона блоков пикселизации ([ и ])"),
                    };
                    ui.label(measure.label()).on_hover_text(tip);
                    let mut size = a.size();
                    if size_slider(ui, &mut size, measure.range()).on_hover_text(tip).changed() {
                        a.set_size(size);
                    }
                    let px = measure.pixels(a.unit, a.size()).round();
                    ui.add_sized(vec2(48.0, 18.0), egui::Label::new(format!("{px} px")));
                    if measure == Measure::Text {
                        ui.add_space(8.0);
                        for style in TextStyle::ALL {
                            if ui.selectable_label(a.text_style == style, style.label()).clicked() {
                                a.set_text_style(style);
                            }
                        }
                        ui.add_space(8.0);
                        font_choice(ui, a);
                    }
                    ui.add_space(16.0);
                    presets_menu(ui, a, presets);
                    }
                });
            });
        self.clicked.extend(clicked);
    }
}

/// The parts of the bar (see `rows`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// At the right end of the first row.
    Right,
    /// The first row from the left, wrapping before it reaches `Right`.
    Left,
    /// The second row, wrapped.
    Below,
}

/// The bar's two rows, `part` drawing each. The first is a row of its own
/// (`horizontal`): a right-to-left layout laid straight in the panel
/// centred its buttons in the whole height left, and the bar took half the
/// window.
fn rows(ui: &mut Ui, mut part: impl FnMut(&mut Ui, Part)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            part(ui, Part::Right);
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| part(ui, Part::Left));
        });
    });
    ui.horizontal_wrapped(|ui| part(ui, Part::Below));
}

/// A colour to choose: a square, outlined when chosen.
fn swatch(ui: &mut Ui, colour: Color32, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect.shrink(2.0), 2.0, colour);
    painter.rect_stroke(rect.shrink(1.0), 3.0, swatch_outline(on, response.hovered()), StrokeKind::Inside);
    response
}

fn swatch_outline(on: bool, hovered: bool) -> Stroke {
    if on {
        Stroke::new(2.0, Color32::WHITE)
    } else if hovered {
        Stroke::new(1.0, Color32::from_gray(200))
    } else {
        Stroke::new(1.0, Color32::from_gray(90))
    }
}

/// Any colour: a strip of hues with `colour` in its middle, outlined when
/// `colour` is none of the six; a click opens egui's picker. The colour
/// picked, while it is being picked.
fn picker_button(ui: &mut Ui, colour: Color32, on: bool) -> (Response, Option<Color32>) {
    let (rect, response) = ui.allocate_exact_size(vec2(34.0, 18.0), Sense::click());
    let painter = ui.painter();
    let strip = rect.shrink(2.0);
    const HUES: usize = 12;
    for k in 0..HUES {
        let x0 = strip.left() + strip.width() * k as f32 / HUES as f32;
        let x1 = strip.left() + strip.width() * (k + 1) as f32 / HUES as f32;
        let hue: Color32 = egui::ecolor::Hsva::new(k as f32 / HUES as f32, 0.8, 0.95, 1.0).into();
        painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, strip.y_range()), 0.0, hue);
    }
    let middle = Rect::from_center_size(strip.center(), vec2(12.0, 10.0));
    painter.rect(middle, 2.0, colour, Stroke::new(1.0, Color32::from_black_alpha(180)), StrokeKind::Outside);
    painter.rect_stroke(rect.shrink(1.0), 3.0, swatch_outline(on, response.hovered()), StrokeKind::Inside);
    let mut picked = None;
    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut c = colour;
            if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
                picked = Some(c);
            }
        });
    (response, picked)
}

/// The size: a slider along a logarithmic scale, its rail light enough to
/// see on the bar, filled up to the handle.
fn size_slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(180.0, 20.0), Sense::click_and_drag());
    let (lo, hi) = (range.start().ln(), range.end().ln());
    let rail = Rect::from_min_max(pos2(rect.left() + 8.0, rect.center().y - 2.5), pos2(rect.right() - 8.0, rect.center().y + 2.5));
    if response.is_pointer_button_down_on()
        && let Some(p) = response.interact_pointer_pos()
    {
        let t = ((p.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
        let v = (lo + t * (hi - lo)).exp();
        if v != *value {
            *value = v;
            response.mark_changed();
        }
    }
    let t = ((value.max(*range.start()).ln() - lo) / (hi - lo)).clamp(0.0, 1.0);
    let x = rail.left() + t * rail.width();
    let painter = ui.painter();
    painter.rect_filled(rail, 2.5, Color32::from_gray(100));
    painter.rect_filled(Rect::from_min_max(rail.min, pos2(x, rail.max.y)), 2.5, ui.visuals().selection.bg_fill);
    let hot = response.hovered() || response.dragged();
    painter.circle(pos2(x, rect.center().y), if hot { 8.0 } else { 7.0 }, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(160)));
    response
}

/// The font of text: a list of the families installed, narrowed by what is
/// typed at its top, and bold and italic.
fn font_choice(ui: &mut Ui, a: &mut Annotating) {
    let mut font = a.font.clone();
    egui::ComboBox::from_id_salt("annotate_font")
        .selected_text(font.family.as_str())
        .width(170.0)
        .height(420.0)
        .show_ui(ui, |ui| {
            let filter = ui.add(egui::TextEdit::singleline(&mut a.font_filter).hint_text(tr!("Find a font", "Найти шрифт")));
            if !filter.has_focus() && a.font_filter.is_empty() {
                filter.request_focus();
            }
            let words = a.font_filter.to_lowercase();
            for family in annotate::families().iter().filter(|f| f.to_lowercase().contains(words.trim())) {
                if ui.selectable_label(*family == font.family, family).clicked() {
                    font.family = family.clone();
                    ui.close();
                }
            }
        });
    let bold = ui.selectable_label(font.bold, egui::RichText::new(tr!("B", "Ж")).strong()).on_hover_text(tr!("Bold", "Жирный"));
    if bold.clicked() {
        font.bold = !font.bold;
    }
    let italic = ui.selectable_label(font.italic, egui::RichText::new(tr!("I", "К")).italics()).on_hover_text(tr!("Italic", "Курсив"));
    if italic.clicked() {
        font.italic = !font.italic;
    }
    if font != a.font {
        a.set_font(font);
    }
}

/// The user's presets (removed with ×), and the tool's style kept as a
/// new one.
fn presets_menu(ui: &mut Ui, a: &mut Annotating, presets: &mut Vec<Preset>) {
    ui.menu_button(tr!("Presets", "Пресеты"), |ui| {
        let current = a.preset();
        let mut chosen = None;
        let mut remove = None;
        if presets.is_empty() {
            ui.label(tr!("None yet: set a tool up and save its style", "Пока нет: настройте инструмент и сохраните его стиль"));
        }
        for (k, p) in presets.iter().enumerate() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect_filled(rect.shrink(1.0), 2.0, p.colour);
                if ui.selectable_label(*p == current, p.label(a.unit)).clicked() {
                    chosen = Some(p.clone());
                }
                if ui.small_button("×").on_hover_text(tr!("Remove", "Убрать")).clicked() {
                    remove = Some(k);
                }
            });
        }
        if let Some(i) = remove {
            presets.remove(i);
        }
        if let Some(p) = chosen {
            a.apply(&p);
            ui.close();
        }
        ui.separator();
        let known = presets.contains(&current);
        let save = egui::Button::new(tr!("Save Current Style", "Сохранить текущий стиль"));
        let tip = tr!("The tool with its colour, size and text style", "Инструмент с его цветом, размером и стилем текста");
        if ui.add_enabled(!known, save).on_hover_text(tip).clicked() {
            presets.push(current);
            ui.close();
        }
    });
}

/// The text of mark `i` as shown: what is typed while it is being typed.
fn shown_text(a: &Annotating, i: usize) -> Option<&str> {
    match &a.marks[i].shape {
        Shape::Text(_, t) => Some(match &a.typing {
            Some(Typing { index: Some(j), text, .. }) if *j == i => text.as_str(),
            _ => t.as_str(),
        }),
        _ => None,
    }
}

/// The texture of text drawn for the screen and its box on the image (in
/// its pixels) for text at `at` of size `px`, a colour and a style; made
/// when not yet made.
fn text_texture(
    a: &mut Annotating,
    ctx: &egui::Context,
    text: &str,
    at: Pos2,
    (px, colour, style, font): (f32, Color32, TextStyle, &TextFont),
    used: &mut Vec<annotate::TextKey>,
) -> Option<(egui::TextureId, Rect)> {
    let key = (text.to_string(), px.to_bits(), colour, style, font.clone());
    if !a.texts.contains_key(&key) {
        let (pixmap, offset) = annotate::render_text(text, px, colour, style, font)?;
        let size = [pixmap.width(), pixmap.height()];
        let image = ColorImage::from_rgba_premultiplied([size[0] as usize, size[1] as usize], pixmap.data());
        let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
        let texture = ctx.load_texture("annotate_text", image, options);
        a.texts.insert(key.clone(), (texture, offset, size));
    }
    let (texture, [dx, dy], [w, h]) = a.texts.get(&key)?;
    let min = at.round() + vec2(*dx as f32, *dy as f32);
    used.push(key);
    Some((texture.id(), Rect::from_min_size(min, vec2(*w as f32, *h as f32))))
}

/// The topmost mark under `p` (image pixels), `boxes` the text marks':
/// pixelated areas lie under the rest, whatever their order.
fn hit(a: &Annotating, p: Pos2, reach: f32, boxes: &[Option<Rect>]) -> Option<usize> {
    let under = |i: &usize| matches!(a.marks[*i].shape, Shape::Pixelate(_));
    let order = (0..a.marks.len()).rev().filter(|i| !under(i)).chain((0..a.marks.len()).rev().filter(under));
    order.into_iter().find(|&i| {
        let m = &a.marks[i];
        let width = annotate::line_width(a.unit, m.size) / 2.0;
        match &m.shape {
            Shape::Rect(r) => annotate::near_frame(*r, p, reach + width),
            Shape::Ellipse(r) => annotate::near_ellipse(*r, p, reach + width),
            Shape::Line(s, e) | Shape::Arrow(s, e) => annotate::segment_distance(p, *s, *e) <= reach + width,
            Shape::Pen(points) => points.windows(2).any(|w| annotate::segment_distance(p, w[0], w[1]) <= reach + width),
            Shape::Pixelate(r) => r.contains(p),
            Shape::Text(..) => boxes.get(i).copied().flatten().is_some_and(|b| b.expand(reach / 2.0).contains(p)),
        }
    })
}

/// What a drag starting at `p` takes: a handle of the mark chosen, a mark
/// to move, or a new mark of the tool.
fn start(a: &mut Annotating, p: Pos2, reach: f32, boxes: &[Option<Rect>], size: Vec2) -> Option<Drag> {
    if let Some(i) = a.selected.filter(|&i| i < a.marks.len()) {
        if let Some((s, e)) = a.marks[i].ends() {
            for (end, head) in [(e, true), (s, false)] {
                if p.distance(end) <= reach * 1.5 {
                    a.remember();
                    return Some(Drag::End { index: i, head });
                }
            }
        } else if let Some(frame) = a.marks[i].frame()
            && let grip @ Grip::Edges { .. } = crop::grip(frame, p, reach)
        {
            a.remember();
            return Some(Drag::Edges { index: i, grip, start: frame, from: p });
        }
    }
    if let Some(i) = hit(a, p, reach, boxes) {
        a.select(i);
        a.remember();
        return Some(Drag::Move { index: i, start: a.marks[i].clone(), from: p });
    }
    a.selected = None;
    let p = Rect::from_min_size(Pos2::ZERO, size).clamp(p);
    let shape = match a.tool {
        Tool::Rect => Shape::Rect(Rect::from_min_max(p, p)),
        Tool::Ellipse => Shape::Ellipse(Rect::from_min_max(p, p)),
        Tool::Pixelate => Shape::Pixelate(Rect::from_min_max(p, p)),
        Tool::Line => Shape::Line(p, p),
        Tool::Arrow => Shape::Arrow(p, p),
        Tool::Pen => Shape::Pen(vec![p]),
        // Text is placed by a click.
        Tool::Text => return None,
    };
    a.remember();
    a.marks.push(a.new_mark(shape));
    let index = a.marks.len() - 1;
    a.selected = Some(index);
    Some(Drag::New { index, from: p })
}

/// `to` from `from` turned to the nearest eighth of a turn (Shift).
fn snapped(from: Pos2, to: Pos2) -> Pos2 {
    let d = to - from;
    let step = std::f32::consts::FRAC_PI_4;
    let angle = (d.angle() / step).round() * step;
    from + Vec2::angled(angle) * d.length()
}

/// The drag `drag` with the pointer at `to`; a freehand line takes a
/// point `step` image pixels from the last.
fn update(a: &mut Annotating, drag: &Drag, to: Pos2, size: Vec2, shift: bool, step: f32) {
    let image = Rect::from_min_size(Pos2::ZERO, size);
    match drag {
        Drag::New { index, from } => {
            let Some(mark) = a.marks.get_mut(*index) else { return };
            if let Shape::Line(s, e) | Shape::Arrow(s, e) = &mut mark.shape {
                *s = *from;
                *e = image.clamp(if shift { snapped(*from, to) } else { to });
            } else if let Shape::Pen(points) = &mut mark.shape {
                let to = image.clamp(to);
                if points.last().is_none_or(|last| last.distance(to) >= step) {
                    points.push(to);
                }
            } else {
                // Shift: a square, a circle.
                let frame = crop::dragged(Grip::New, Rect::NOTHING, *from, to, size, shift.then_some(1.0));
                mark.set_frame(frame);
            }
        }
        Drag::Move { index, start, from } => {
            if let Some(mark) = a.marks.get_mut(*index) {
                *mark = start.moved(to - *from, size);
            }
        }
        Drag::Edges { index, grip, start, from } => {
            if let Some(mark) = a.marks.get_mut(*index) {
                mark.set_frame(crop::dragged(*grip, *start, *from, to, size, None));
            }
        }
        Drag::End { index, head } => {
            if let Some(Mark { shape: Shape::Line(s, e) | Shape::Arrow(s, e), .. }) = a.marks.get_mut(*index) {
                let (end, other) = if *head { (e, *s) } else { (s, *e) };
                *end = image.clamp(if shift { snapped(other, to) } else { to });
            }
        }
    }
}

/// The drag is over: a mark drawn too small is dropped, and a drag that
/// changed nothing is not kept for Ctrl+Z.
fn end(a: &mut Annotating, drag: &Drag, min: f32) {
    if let Drag::New { index, .. } = drag
        && a.marks.get(*index).is_some_and(|m| m.is_tiny(min))
    {
        a.marks.remove(*index);
        a.selected = None;
        a.undo.pop();
        return;
    }
    if a.undo.last() == Some(&a.marks) {
        a.undo.pop();
    }
}

fn cursor(grip: Grip) -> CursorIcon {
    match grip {
        Grip::Move => CursorIcon::Move,
        Grip::New => CursorIcon::Crosshair,
        Grip::Edges { left, right, top, bottom } => match (left || right, top || bottom) {
            (true, false) => CursorIcon::ResizeHorizontal,
            (false, true) => CursorIcon::ResizeVertical,
            _ if (left && top) || (right && bottom) => CursorIcon::ResizeNwSe,
            _ => CursorIcon::ResizeNeSw,
        },
    }
}

/// Each mark's text drawn for the screen (made as needed) and its box on
/// the image, None for the other marks: the textures, and the boxes alone.
#[allow(clippy::type_complexity)]
fn text_boxes(
    a: &mut Annotating,
    ctx: &egui::Context,
    unit: f32,
    used: &mut Vec<annotate::TextKey>,
) -> (Vec<Option<(egui::TextureId, Rect)>>, Vec<Option<Rect>>) {
    let texts: Vec<_> = (0..a.marks.len())
        .map(|i| {
            let text = shown_text(a, i)?.to_string();
            let Shape::Text(at, _) = a.marks[i].shape else { return None };
            let m = &a.marks[i];
            let font = m.font.clone();
            let look = (annotate::font_size(unit, m.size), m.colour, m.text, &font);
            text_texture(a, ctx, &text, at, look, used)
        })
        .collect();
    let boxes = texts.iter().map(|t| t.map(|(_, b)| b)).collect();
    (texts, boxes)
}

/// The marks of `a` over the image shown in `place` (points), `size` its
/// pixels as shown: drawn with the mouse on `response`, and painted.
pub fn overlay(a: &mut Annotating, response: &Response, painter: &Painter, place: Rect, size: Vec2) {
    let ctx = response.ctx.clone();
    // Points per image pixel.
    let k = place.width() / size.x;
    let to_image = |p: Pos2| ((p - place.min) / k).to_pos2();
    let to_screen = |p: Pos2| place.min + p.to_vec2() * k;
    let on_screen = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));
    let unit = annotate::unit(size);
    let reach = REACH / k;

    // The mosaics, once read.
    if let Some(rx) = &a.mosaics_rx
        && let Ok(result) = rx.try_recv()
    {
        a.mosaics_rx = None;
        match result {
            Ok(mosaics) => {
                a.mosaics = mosaics
                    .into_iter()
                    .map(|(block, data, w, h)| {
                        let image = ColorImage::from_rgba_premultiplied([w as usize, h as usize], &data);
                        (block, ctx.load_texture("annotate_mosaic", image, TextureOptions::NEAREST), [w, h])
                    })
                    .collect();
            }
            Err(e) => log::warn!("mosaics not read: {e}"),
        }
    }

    // The text marks drawn for the screen, as needed, and their boxes: by
    // the marks' indices, so measured again whenever the marks change (text
    // finished by a click is a new mark).
    let mut used = Vec::new();
    let mut boxes = text_boxes(a, &ctx, unit, &mut used).1;

    // The mouse.
    let shift = ctx.input(|i| i.modifiers.shift);
    if response.drag_started_by(PointerButton::Primary)
        && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
    {
        a.finish_typing();
        boxes = text_boxes(a, &ctx, unit, &mut used).1;
        a.drag = start(a, to_image(origin), reach, &boxes, size);
    }
    if let Some(drag) = a.drag.clone() {
        if response.dragged_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            update(a, &drag, to_image(p), size, shift, 1.5 / k);
        }
        if !response.dragged() {
            end(a, &drag, MIN_DRAWN / k);
            a.drag = None;
        }
    }
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos().map(to_image)
    {
        let was_typing = a.typing.is_some();
        a.finish_typing();
        boxes = text_boxes(a, &ctx, unit, &mut used).1;
        let found = hit(a, p, reach, &boxes);
        let text_at = |i: usize| match &a.marks[i].shape {
            Shape::Text(at, t) => Some((*at, t.clone())),
            _ => None,
        };
        match found {
            // A click on text with the text tool edits it.
            Some(i) if a.tool == Tool::Text && text_at(i).is_some() => {
                let (at, text) = text_at(i).expect("checked");
                a.select(i);
                a.typing = Some(Typing { index: Some(i), at, text, focus: true });
            }
            Some(i) => a.select(i),
            None if a.tool == Tool::Text && !was_typing && Rect::from_min_size(Pos2::ZERO, size).contains(p) => {
                a.selected = None;
                a.typing = Some(Typing { index: None, at: p.round(), text: String::new(), focus: true });
            }
            None => a.selected = None,
        }
    }
    if response.double_clicked()
        && let Some(p) = response.interact_pointer_pos().map(to_image)
        && let Some(i) = hit(a, p, reach, &boxes)
        && let Shape::Text(at, text) = &a.marks[i].shape
    {
        let (at, text) = (*at, text.clone());
        a.select(i);
        a.typing = Some(Typing { index: Some(i), at, text, focus: true });
    }

    // The marks as they are now, for the pointer and the painting.
    let (texts, boxes) = text_boxes(a, &ctx, unit, &mut used);

    // The pointer's shape.
    if let Some(p) = response.hover_pos().map(to_image) {
        let icon = match &a.drag {
            Some(Drag::Edges { grip, .. }) => cursor(*grip),
            Some(Drag::New { .. }) => CursorIcon::Crosshair,
            Some(_) => CursorIcon::Grabbing,
            None => {
                let handle = a.selected.filter(|&i| i < a.marks.len()).and_then(|i| match a.marks[i].ends() {
                    Some((s, e)) => (p.distance(s) <= reach * 1.5 || p.distance(e) <= reach * 1.5).then_some(CursorIcon::Move),
                    None => match a.marks[i].frame().map(|f| crop::grip(f, p, reach)) {
                        Some(g @ Grip::Edges { .. }) => Some(cursor(g)),
                        _ => None,
                    },
                });
                match handle {
                    Some(icon) => icon,
                    None if hit(a, p, reach, &boxes).is_some() => CursorIcon::Grab,
                    None if a.tool == Tool::Text => CursorIcon::Text,
                    None => CursorIcon::Crosshair,
                }
            }
        };
        ctx.set_cursor_icon(icon);
    }

    // The marks: pixelated areas under the rest.
    let clip = place.intersect(painter.clip_rect());
    let painter = painter.with_clip_rect(clip);
    for m in &a.marks {
        let Shape::Pixelate(r) = m.shape else { continue };
        // The whole pixels saving fills.
        let [x, y, w, h] = crop::pixels(r, size).map(|v| v as f32);
        let area = on_screen(Rect::from_min_size(pos2(x, y), vec2(w, h)));
        let block = annotate::block(unit, m.size);
        match a.mosaics.iter().find(|(b, ..)| *b == block) {
            Some((_, texture, [cw, ch])) => {
                let block = block as f32;
                let whole = Rect::from_min_size(place.min, vec2(*cw as f32, *ch as f32) * block * k);
                let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                painter.with_clip_rect(area.intersect(clip)).image(texture.id(), whole, uv, Color32::WHITE);
            }
            // Not read yet.
            None => {
                painter.rect_filled(area, 0.0, Color32::from_gray(128));
            }
        }
    }
    for (i, m) in a.marks.iter().enumerate() {
        let width = annotate::line_width(unit, m.size) * k;
        let stroke = Stroke::new(width, m.colour);
        match &m.shape {
            Shape::Pixelate(_) => {}
            Shape::Rect(r) => {
                painter.rect_stroke(on_screen(*r), 0.0, stroke, StrokeKind::Middle);
            }
            Shape::Ellipse(r) => {
                let r = on_screen(*r);
                painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, stroke));
            }
            Shape::Line(s, e) => {
                painter.line_segment([to_screen(*s), to_screen(*e)], stroke);
            }
            Shape::Pen(points) => {
                // Round ends, as saved.
                painter.add(egui::Shape::line(points.iter().map(|p| to_screen(*p)).collect(), stroke));
                for p in [points.first(), points.last()].into_iter().flatten() {
                    painter.circle_filled(to_screen(*p), width / 2.0, m.colour);
                }
            }
            Shape::Arrow(s, e) => {
                let (shaft, head) = annotate::arrow(*s, *e, annotate::line_width(unit, m.size));
                painter.line_segment([to_screen(*s), to_screen(shaft)], stroke);
                painter.add(egui::Shape::convex_polygon(head.map(to_screen).to_vec(), m.colour, Stroke::NONE));
            }
            Shape::Text(..) => {
                if let Some((texture, b)) = texts.get(i).copied().flatten() {
                    painter.image(texture, on_screen(b), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
    }
    // New text being typed.
    let mut field_at = None;
    if let Some(Typing { index: None, at, text, .. }) = &a.typing {
        let (at, text) = (*at, text.clone());
        let px = annotate::font_size(unit, a.size_of(Measure::Text));
        let font = a.font.clone();
        let look = (px, a.colour, a.text_style, &font);
        let shown = text_texture(a, &ctx, &text, at, look, &mut used);
        if let Some((texture, b)) = shown {
            painter.image(texture, on_screen(b), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        // The point, so that it shows where the text goes.
        let p = to_screen(at);
        painter.line_segment([p, p + vec2(0.0, px * k)], Stroke::new(1.5, a.colour));
        field_at = Some(to_screen(pos2(at.x, shown.map_or(at.y + px * 1.25, |(_, b)| b.max.y))));
    }
    if let Some(Typing { index: Some(i), at, .. }) = &a.typing {
        let px = annotate::font_size(unit, a.marks.get(*i).map_or(a.size_of(Measure::Text), |m| m.size));
        let bottom = boxes.get(*i).copied().flatten().map_or(at.y + px * 1.25, |b| b.max.y);
        field_at = Some(to_screen(pos2(at.x, bottom)));
    }

    // The mark chosen: its handles.
    if let Some(i) = a.selected.filter(|&i| i < a.marks.len()) {
        let handle = |p: Pos2| {
            let r = Rect::from_center_size(p, vec2(HANDLE, HANDLE));
            painter.rect(r, 0.0, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
        };
        match &a.marks[i].shape {
            Shape::Pen(points) => {
                let r = on_screen(Rect::from_points(points)).expand(3.0);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
            }
            Shape::Line(s, e) | Shape::Arrow(s, e) => {
                for p in [*s, *e] {
                    painter.circle(to_screen(p), HANDLE / 2.0 + 1.0, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(160)));
                }
            }
            Shape::Text(..) => {
                if let Some(b) = boxes.get(i).copied().flatten() {
                    let r = on_screen(b).expand(3.0);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
                }
            }
            _ => {
                if let Some(f) = a.marks[i].frame().map(on_screen) {
                    painter.rect_stroke(f, 0.0, Stroke::new(1.0, Color32::from_white_alpha(160)), StrokeKind::Outside);
                    let (c, x, y) = (f.center(), [f.min.x, f.center().x, f.max.x], [f.min.y, f.center().y, f.max.y]);
                    for hx in x {
                        for hy in y {
                            if pos2(hx, hy) != c {
                                handle(pos2(hx, hy));
                            }
                        }
                    }
                }
            }
        }
    }

    // Text drawn for the screen and no longer shown is let go.
    a.texts.retain(|key, _| used.contains(key));
    if let Some(at) = field_at {
        text_field(a, &ctx, at + vec2(0.0, 6.0));
    }
}

/// The field the text is typed into, at `at` (points): Enter makes the
/// mark, Shift+Enter starts a new line, Esc gives the text up, a click
/// elsewhere keeps it.
fn text_field(a: &mut Annotating, ctx: &egui::Context, at: Pos2) {
    let Some(typing) = &mut a.typing else { return };
    let (mut commit, mut cancel) = (false, false);
    egui::Area::new(Id::new("annotate_text_area")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                commit = true;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            let hint = tr!("Text; Shift+Enter for a new line", "Текст; Shift+Enter — новая строка");
            let edit = egui::TextEdit::multiline(&mut typing.text).id(Id::new(TEXT_ID)).desired_rows(1).desired_width(260.0).hint_text(hint);
            let response = ui.add(edit);
            if std::mem::take(&mut typing.focus) {
                response.request_focus();
            } else if !response.has_focus() {
                // A click elsewhere: the text stays.
                commit = true;
            }
        });
    });
    if cancel {
        a.typing = None;
    } else if commit {
        a.finish_typing();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bar in a top panel of a window `width` wide, 800 high: its
    /// height and where the right buttons, the left items and the second
    /// row's items are.
    fn bar_at(width: f32) -> (f32, Vec<Rect>, Vec<Rect>, Vec<Rect>) {
        let ctx = egui::Context::default();
        let mut out = (0.0, Vec::new(), Vec::new(), Vec::new());
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0))), ..Default::default() };
            let _ = ctx.run_ui(input, |root| {
                let (mut right, mut left, mut below) = (Vec::new(), Vec::new(), Vec::new());
                let panel = egui::Panel::top("bar").show(root, |ui| {
                    rows(ui, |ui, part| match part {
                        Part::Right => right.extend(["Отмена", "Сохранить как…", "Сохранить"].map(|t| ui.button(t).rect)),
                        Part::Left => left.extend(Tool::ALL.map(|t| ui.selectable_label(false, t.label()).rect)),
                        Part::Below => below.extend((0..12).map(|i| ui.button(format!("Цвет {i}")).rect)),
                    });
                });
                out = (panel.response.rect.height(), right, left, below);
            });
        }
        out
    }

    #[test]
    fn the_bar_is_two_rows_that_do_not_overlap() {
        let (height, right, left, below) = bar_at(1600.0);
        assert!(height < 80.0, "{height}");
        let row = |r: &[Rect]| r.iter().map(|r| r.center().y.round() as i32).collect::<std::collections::BTreeSet<_>>().len();
        assert_eq!((row(&right), row(&left), row(&below)), (1, 1, 1));
        assert!(below[0].min.y >= left[0].max.y, "{below:?} {left:?}");
        // Narrow: the tools wrap, never under the buttons.
        let (height, right, left, below) = bar_at(600.0);
        assert!(height < 200.0, "{height}");
        assert!(row(&left) > 1);
        assert!(left.iter().all(|l| right.iter().all(|r| !l.intersects(*r))), "{left:?} {right:?}");
        assert!(below.iter().all(|b| left.iter().chain(&right).all(|l| !b.intersects(*l))));
    }

    /// Text finished by a click is a new mark that the boxes measured
    /// before it do not have: no box, no hit, no panic.
    #[test]
    fn a_mark_without_a_box_is_not_hit() {
        let size = vec2(400.0, 300.0);
        let mut a = Annotating::new(Default::default(), size, crate::view::Zoom::Fit, Vec2::ZERO);
        a.typing = Some(Typing { index: None, at: pos2(10.0, 10.0), text: "abc".into(), focus: false });
        a.finish_typing();
        assert_eq!(a.marks.len(), 1);
        assert_eq!(hit(&a, pos2(12.0, 12.0), 4.0, &[]), None);
        let frame = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 200.0));
        a.marks.push(Mark { shape: Shape::Rect(frame), colour: COLOURS[0], size: 2.0, text: TextStyle::Outline, font: TextFont::default() });
        assert_eq!(hit(&a, pos2(100.0, 150.0), 4.0, &[None]), Some(1));
        assert_eq!(hit(&a, pos2(12.0, 12.0), 4.0, &[Some(Rect::from_min_max(pos2(10.0, 10.0), pos2(40.0, 30.0)))]), Some(0));
    }
}
