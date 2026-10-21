//! Marking on screen (A): the bar above the image with the tools, the
//! colours, the size, Undo, Save, Save As and Cancel, the list of the
//! marks beside it, and the marks over the image, drawn, chosen, moved and
//! resized with the mouse (see `annotate`). Text is typed into a field
//! below its point.

use egui::{
    Color32, ColorImage, CursorIcon, FontId, Id, Key, Modifiers, Painter, PointerButton, Pos2, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, TextureOptions, Ui, Vec2, pos2, vec2,
};

use super::{TEXT, TEXT_WEAK, TOOLBAR_BG, panel_frame};
use crate::annotate::{self, Annotating, ArrowHead, COLOURS, Drag, Mark, Measure, Preset, Shape, TextFont, TextStyle, Tool, Typing};
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
        let list = self.mark_list;
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
                        // The marks wait while they are saved: those drawn
                        // meanwhile would be lost when it ends.
                        if saving {
                            ui.disable();
                        }
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
                        let clear = egui::Button::new(tr!("Clear", "Очистить"));
                        let tip = tr!("Delete every mark (Ctrl+Z brings them back)", "Удалить все пометки (Ctrl+Z возвращает их)");
                        if ui.add_enabled(!a.marks.is_empty(), clear).on_hover_text(tip).clicked() {
                            a.clear();
                        }
                        ui.add_space(12.0);
                        let tip = tr!("The marks in a list beside the image (O)", "Пометки списком рядом с изображением (O)");
                        if ui.selectable_label(list, tr!("List", "Список")).on_hover_text(tip).clicked() {
                            clicked = Some(Cmd::MarkList);
                        }
                    }
                    // The style: the colours, the size of what is drawn (or
                    // chosen), text's look or the arrow's head, the presets.
                    Part::Below => {
                    if saving {
                        ui.disable();
                    }
                    colours(ui, a, &mut clicked);
                    ui.add_space(16.0);
                    let measure = a.measure();
                    let tip = match measure {
                        Measure::Line => tr!("Width of lines ([ and ])", "Толщина линий ([ и ])"),
                        Measure::Text => tr!("Height of text ([ and ])", "Высота текста ([ и ])"),
                        Measure::Block => tr!("Side of the pixelation's blocks ([ and ])", "Сторона блоков пикселизации ([ и ])"),
                    };
                    ui.label(measure.label()).on_hover_text(tip);
                    let mut size = a.size();
                    let slider = size_slider(ui, &mut size, measure.range()).on_hover_text(tip);
                    if slider.changed() {
                        a.set_size(size);
                    }
                    if slider.drag_stopped() || slider.clicked() {
                        a.end_run();
                    }
                    let px = measure.pixels(a.unit, a.size()).round();
                    ui.add_sized(vec2(48.0, 18.0), egui::Label::new(format!("{px} px")));
                    match a.kind() {
                        Tool::Text => {
                            ui.add_space(8.0);
                            for style in TextStyle::ALL {
                                if ui.selectable_label(a.text_style == style, style.label()).clicked() {
                                    a.set_text_style(style);
                                }
                            }
                            if a.text_style != TextStyle::Plain {
                                ui.add_space(4.0);
                                contrast_choice(ui, a);
                            }
                            ui.add_space(8.0);
                            font_choice(ui, a);
                        }
                        Tool::Arrow => {
                            ui.add_space(8.0);
                            for head in ArrowHead::ALL {
                                if head_button(ui, head, a.head == head).on_hover_text(head.label()).clicked() {
                                    a.set_head(head);
                                }
                            }
                        }
                        _ => {}
                    }
                    ui.add_space(16.0);
                    presets_menu(ui, a, presets);
                    }
                });
            });
        self.clicked.extend(clicked);
    }
}

/// The width of the list of the marks.
const LIST_WIDTH: f32 = 240.0;
/// The height of a row in it.
const ROW: f32 = 26.0;

impl App {
    /// The list of the marks beside the image (O, `App::mark_list`), the
    /// topmost first: a click chooses one, a double click edits text, and
    /// the row under the pointer or chosen has buttons that move the mark up
    /// or down the order and delete it; the mark under the pointer is
    /// outlined on the image.
    pub(crate) fn marks_panel(&mut self, root_ui: &mut Ui) {
        let saving = self.saving();
        let Some(a) = &mut self.annotating else { return };
        a.hovered = None;
        egui::Panel::right("annotate_marks")
            .resizable(false)
            .exact_size(LIST_WIDTH)
            .frame(panel_frame(TOOLBAR_BG, egui::Margin::symmetric(6, 8)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                if saving {
                    ui.disable();
                }
                marks_list(ui, a)
            });
    }
}

/// What a row of the list asks for.
enum RowAction {
    Choose(usize),
    EditText(usize),
    /// Later in the order (`true`: over the next one) or earlier.
    Reorder(usize, bool),
    Delete(usize),
}

fn marks_list(ui: &mut Ui, a: &mut Annotating) {
    let n = a.marks.len();
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(tr!("Marks", "Пометки")).strong().color(Color32::WHITE));
        if n > 0 {
            ui.label(RichText::new(n.to_string()).color(TEXT_WEAK));
        }
    });
    ui.add_space(4.0);
    if n == 0 {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("The marks drawn on the image are listed here", "Здесь появятся пометки, нарисованные на изображении")).color(TEXT_WEAK));
        });
        return;
    }
    // The chosen row is scrolled to when the choice changes on the image.
    let follow_id = Id::new("annotate_list_chosen");
    let follow = ui.data(|d| d.get_temp::<Option<usize>>(follow_id)) != Some(a.selected);
    ui.data_mut(|d| d.insert_temp(follow_id, a.selected));
    let mut action = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for i in (0..n).rev() {
            if let Some(asked) = mark_row(ui, a, i, follow) {
                action = Some(asked);
            }
        }
    });
    // Text emptied in its field goes when the field loses the focus,
    // before the rows' indices are taken.
    let Some(action) = action else { return };
    let removed = a.finish_typing();
    let at = |i| annotate::after_removal(removed, i);
    match action {
        RowAction::Choose(i) => at(i).into_iter().for_each(|i| a.select(i)),
        RowAction::EditText(i) => at(i).into_iter().for_each(|i| a.edit_text(i)),
        RowAction::Reorder(i, later) => at(i).into_iter().for_each(|i| a.reorder(i, later)),
        RowAction::Delete(i) => at(i).into_iter().for_each(|i| a.delete(i)),
    }
}

/// The row of mark `i`: its kind, colour, what it is and its size, or the
/// buttons while it is under the pointer or chosen.
fn mark_row(ui: &mut Ui, a: &mut Annotating, i: usize, follow: bool) -> Option<RowAction> {
    let chosen = a.selected == Some(i);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if chosen && follow {
        response.scroll_to_me(None);
    }
    let under = ui.rect_contains_pointer(rect);
    if under {
        a.hovered = Some(i);
    }
    let m = &a.marks[i];
    let painter = ui.painter();
    if chosen {
        painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
    } else if under {
        painter.rect_filled(rect, 4.0, Color32::from_white_alpha(14));
    }
    let ink = if chosen { Color32::WHITE } else { TEXT };
    let weak = if chosen { Color32::from_white_alpha(170) } else { TEXT_WEAK };
    let tool = m.shape.tool();
    kind_icon(painter, Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 9.0), vec2(18.0, 18.0)), tool, m.head, ink);
    let mut x = rect.left() + 28.0;
    if tool != Tool::Pixelate {
        disc(painter, pos2(x + 5.0, rect.center().y), 5.0, m.colour);
        x += 16.0;
    }

    // The buttons, from the right; otherwise the size there.
    let mut asked = None;
    let mut right = rect.right() - 2.0;
    if under || chosen {
        let last = a.marks.len() - 1;
        let buttons = [
            (RowButton::Delete, true, tr!("Delete", "Удалить")),
            (RowButton::Down, i > 0, tr!("Down: under the mark below", "Ниже: под пометку под ней")),
            (RowButton::Up, i < last, tr!("Up: over the mark above", "Выше: поверх пометки над ней")),
        ];
        for (k, (button, enabled, tip)) in buttons.into_iter().enumerate() {
            let r = Rect::from_center_size(pos2(right - 10.0, rect.center().y), vec2(20.0, 20.0));
            right -= 22.0;
            let sense = if enabled { Sense::click() } else { Sense::hover() };
            let b = ui.interact(r, ui.id().with(("mark_row", i, k)), sense);
            let hovered = enabled && b.hovered();
            if hovered {
                ui.painter().rect_filled(r, 3.0, Color32::from_white_alpha(28));
            }
            let colour = match (enabled, button) {
                (false, _) => Color32::from_white_alpha(50),
                (true, RowButton::Delete) if hovered => Color32::from_rgb(0xff, 0x80, 0x80),
                _ => ink,
            };
            row_button(ui.painter(), r, button, colour);
            if b.on_hover_text(tip).clicked() && enabled {
                asked = Some(match button {
                    RowButton::Delete => RowAction::Delete(i),
                    RowButton::Down => RowAction::Reorder(i, false),
                    RowButton::Up => RowAction::Reorder(i, true),
                });
            }
        }
    } else {
        let px = Measure::of(&m.shape).pixels(a.unit, m.size).round();
        let size = ui.painter().text(pos2(right - 4.0, rect.center().y), egui::Align2::RIGHT_CENTER, format!("{px} px"), FontId::proportional(12.0), weak);
        right = size.left() - 4.0;
    }

    // What it is, cut to the room left.
    let (name, detail) = describe(m);
    let mut job = egui::text::LayoutJob::default();
    job.append(&name, 0.0, egui::TextFormat::simple(FontId::proportional(13.5), ink));
    if !detail.is_empty() {
        job.append(&detail, 6.0, egui::TextFormat::simple(FontId::proportional(12.0), weak));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x - 4.0).max(10.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);

    let text = matches!(m.shape, Shape::Text(..));
    let response = if text {
        let full = match &m.shape {
            Shape::Text(_, t) => t.clone(),
            _ => String::new(),
        };
        response.on_hover_text(format!("{full}\n\n{}", tr!("A double click edits it", "Двойной щелчок — правка")))
    } else {
        response
    };
    if asked.is_none() {
        if response.double_clicked() && text {
            asked = Some(RowAction::EditText(i));
        } else if response.clicked() {
            asked = Some(RowAction::Choose(i));
        }
    }
    asked
}

#[derive(Clone, Copy)]
enum RowButton {
    Up,
    Down,
    Delete,
}

/// A row button's sign: a chevron up or down, a cross.
fn row_button(painter: &Painter, r: Rect, button: RowButton, colour: Color32) {
    let c = r.center();
    let stroke = Stroke::new(1.5, colour);
    match button {
        RowButton::Up => {
            painter.line(vec![c + vec2(-4.5, 2.5), c + vec2(0.0, -2.5), c + vec2(4.5, 2.5)], stroke);
        }
        RowButton::Down => {
            painter.line(vec![c + vec2(-4.5, -2.5), c + vec2(0.0, 2.5), c + vec2(4.5, -2.5)], stroke);
        }
        RowButton::Delete => {
            painter.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], stroke);
            painter.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], stroke);
        }
    }
}

/// What a mark is, for its row: its kind or text, and a detail (the size
/// of a frame, the head of an arrow).
fn describe(m: &Mark) -> (String, String) {
    let tool = m.shape.tool();
    match &m.shape {
        Shape::Text(_, t) => {
            let mut lines = t.lines().map(str::trim).filter(|l| !l.is_empty());
            let first = lines.next().unwrap_or_default();
            let more = if lines.next().is_some() { " …" } else { "" };
            (tr!(format!("“{first}”{more}"), format!("«{first}»{more}")), String::new())
        }
        Shape::Rect(r) | Shape::Ellipse(r) | Shape::Pixelate(r) => (tool.label().into(), format!("{:.0} × {:.0}", r.width(), r.height())),
        Shape::Arrow(..) => (tool.label().into(), m.head.label().to_lowercase()),
        Shape::Line(a, b) => (tool.label().into(), format!("{:.0} px", a.distance(*b))),
        Shape::Pen(_) => (tool.label().into(), String::new()),
    }
}

/// A small picture of a mark's kind in `r`, in `colour`.
fn kind_icon(painter: &Painter, r: Rect, tool: Tool, head: ArrowHead, colour: Color32) {
    let stroke = Stroke::new(1.5, colour);
    let r = r.shrink(2.0);
    match tool {
        Tool::Rect => {
            painter.rect_stroke(r.shrink(1.0), 0.0, stroke, StrokeKind::Inside);
        }
        Tool::Ellipse => {
            painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0 - vec2(1.0, 2.5), stroke));
        }
        Tool::Line => {
            painter.line_segment([r.left_bottom() + vec2(1.0, -1.0), r.right_top() + vec2(-1.0, 1.0)], stroke);
        }
        Tool::Arrow => paint_arrow(painter, r.left_bottom() + vec2(1.0, -1.0), r.right_top() + vec2(-1.0, 1.0), 1.5, head, colour),
        Tool::Pen => {
            let points = (0..=12).map(|k| k as f32 / 12.0).map(|t| pos2(r.left() + t * r.width(), r.center().y - (t * 6.0).sin() * r.height() * 0.3)).collect();
            painter.line(points, stroke);
        }
        Tool::Pixelate => {
            let side = r.width() / 3.0;
            for y in 0..3 {
                for x in 0..3 {
                    let cell = Rect::from_min_size(r.min + vec2(x as f32, y as f32) * side, vec2(side, side)).shrink(0.5);
                    painter.rect_filled(cell, 0.0, if (x + y) % 2 == 0 { colour } else { colour.gamma_multiply(0.35) });
                }
            }
        }
        Tool::Text => {
            painter.text(r.center(), egui::Align2::CENTER_CENTER, "T", FontId::proportional(15.0), colour);
        }
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

/// The side of a colour's swatch.
const SWATCH: f32 = 22.0;
/// The width of the picker's square and hue bar.
const PICKER: f32 = 180.0;

/// The six colours (1 to 6) and any other, as round swatches like
/// MetalMPV's accent colours, the chosen one ringed.
fn colours(ui: &mut Ui, a: &mut Annotating, clicked: &mut Option<Cmd>) {
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 4.0;
    for (i, &c) in COLOURS.iter().enumerate() {
        let response = swatch(ui, a.colour() == c, |p, centre, r| disc(p, centre, r, c));
        if response.on_hover_text(format!("{}", i + 1)).clicked() {
            *clicked = Some(Cmd::MarkColour(i as u8));
        }
    }
    let custom = !COLOURS.contains(&a.colour());
    if let Some(colour) = own_colour(ui, a.colour(), custom) {
        a.set_colour(colour);
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

/// A round swatch, ringed when chosen, a little larger under the pointer;
/// `fill` paints the inside at a centre and a radius.
fn swatch(ui: &mut Ui, on: bool, fill: impl FnOnce(&Painter, Pos2, f32)) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    let painter = ui.painter();
    let (centre, radius) = (rect.center(), SWATCH / 2.0);
    if on {
        painter.circle_stroke(centre, radius - 1.0, Stroke::new(2.0, Color32::WHITE));
        fill(painter, centre, radius - 4.5);
    } else {
        fill(painter, centre, radius - 3.0 + if response.hovered() { 1.0 } else { 0.0 });
    }
    response
}

/// A disc of `colour`, edged when it is too dark to see on the bar.
fn disc(painter: &Painter, centre: Pos2, radius: f32, colour: Color32) {
    painter.circle_filled(centre, radius, colour);
    let [r, g, b, _] = colour.to_array();
    if 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b) < 60.0 {
        painter.circle_stroke(centre, radius - 0.5, Stroke::new(1.0, Color32::from_gray(120)));
    }
}

/// Any other colour: a colour wheel, or the colour when it is none of the
/// six; a click opens a picker (MetalMPV's: a square, a hue bar, the code
/// as `#rrggbb` and red, green and blue). The colour picked.
fn own_colour(ui: &mut Ui, colour: Color32, custom: bool) -> Option<Color32> {
    let response = swatch(ui, custom, |p, centre, r| if custom { disc(p, centre, r, colour) } else { colour_wheel(p, centre, r) });
    let response = response.on_hover_text(tr!("Any colour…", "Любой цвет…"));
    let mut picked = None;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| picked = picker_fields(ui, colour));
    picked
}

/// Text's second colour, while it is outlined or on a plate: of the
/// outline, or of the letters; automatic (black or white, whichever stands
/// out, a disc half of each) unless chosen in a popup like `own_colour`'s.
fn contrast_choice(ui: &mut Ui, a: &mut Annotating) {
    let (label, tip) = match a.text_style {
        TextStyle::Plate => (tr!("Letters", "Буквы"), tr!("Colour of the letters on the plate", "Цвет букв на плашке")),
        _ => (tr!("Outline", "Обводка"), tr!("Colour of the outline", "Цвет обводки")),
    };
    ui.label(RichText::new(label).color(TEXT_WEAK)).on_hover_text(tip);
    let shown = a.contrast.unwrap_or_else(|| annotate::outline(a.colour()));
    let auto = a.contrast.is_none();
    let response = swatch(ui, false, |p, centre, r| {
        if auto {
            p.circle_filled(centre, r, Color32::WHITE);
            let half: Vec<Pos2> = (0..=16).map(|k| centre + Vec2::angled(std::f32::consts::FRAC_PI_4 + k as f32 / 16.0 * std::f32::consts::PI) * r).collect();
            p.add(egui::Shape::convex_polygon(half, Color32::BLACK, Stroke::NONE));
            p.circle_stroke(centre, r - 0.5, Stroke::new(1.0, Color32::from_gray(120)));
        } else {
            disc(p, centre, r, shown);
        }
    });
    let tip = if auto {
        tr!(format!("{tip}: automatic, black or white"), format!("{tip}: авто, чёрный или белый"))
    } else {
        tip.to_string()
    };
    let response = response.on_hover_text(tip);
    let mut chosen = None;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.horizontal(|ui| {
            let tip = tr!("Black or white, whichever stands out", "Чёрный или белый — смотря что контрастнее");
            if ui.selectable_label(auto, tr!("Automatic", "Авто")).on_hover_text(tip).clicked() {
                chosen = Some(None);
            }
            for c in [Color32::BLACK, Color32::WHITE] {
                if swatch(ui, a.contrast == Some(c), |p, centre, r| disc(p, centre, r, c)).clicked() {
                    chosen = Some(Some(c));
                }
            }
        });
        ui.add_space(6.0);
        if let Some(c) = picker_fields(ui, shown) {
            chosen = Some(Some(c));
        }
    });
    if let Some(contrast) = chosen {
        a.set_contrast(contrast);
    }
}

/// The picker's square and hue bar, the code as `#rrggbb` and red, green and
/// blue, starting at `colour`. The colour picked.
fn picker_fields(ui: &mut Ui, colour: Color32) -> Option<Color32> {
    let mut picked = None;
    let mut c = colour;
    if colour_picker(ui, &mut c) {
        picked = Some(c);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = ui.label("HEX");
        // Kept as typed while the field has the focus.
        let id = ui.id().with("mark_hex");
        let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| hex(colour));
        let width = PICKER - label.rect.width() - ui.spacing().item_spacing.x;
        let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width));
        if field.changed()
            && let Some(typed) = typed_colour(&text)
        {
            picked = Some(typed);
        }
        if field.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let size = vec2((PICKER - 2.0 * spacing) / 3.0, ui.spacing().interact_size.y);
        let [r, g, b, _] = colour.to_array();
        let mut rgb = [r, g, b];
        let mut edited = false;
        for (value, name) in rgb.iter_mut().zip(["R ", "G ", "B "]) {
            edited |= ui.add_sized(size, egui::DragValue::new(value).speed(0.5).range(0..=255).prefix(name)).changed();
        }
        if edited {
            picked = Some(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        }
    });
    picked
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// A colour typed as `#rrggbb` or `rrggbb`.
fn typed_colour(text: &str) -> Option<Color32> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let [_, r, g, b] = u32::from_str_radix(digits, 16).ok()?.to_be_bytes();
    Some(Color32::from_rgb(r, g, b))
}

/// Saturation across and brightness up a square, the hue along a bar under
/// it (egui's picker has a row of numbers over them, wider than they are and
/// with tooltips in English only). True when `colour` was changed.
fn colour_picker(ui: &mut Ui, colour: &mut Color32) -> bool {
    use egui::ecolor::HsvaGamma;
    // The hue kept while it gives the colour shown: a grey has none.
    let id = ui.id().with("mark_hsv");
    let mut hsv = ui
        .data(|d| d.get_temp::<(Color32, HsvaGamma)>(id))
        .filter(|(c, _)| c == colour)
        .map_or_else(|| HsvaGamma::from(*colour), |(_, hsv)| hsv);
    let before = hsv;
    let marker = |p: &Painter, at: Pos2, r: f32| {
        p.circle_stroke(at, r + 1.0, Stroke::new(1.0, Color32::BLACK));
        p.circle_stroke(at, r, Stroke::new(1.5, Color32::WHITE));
    };

    let (square, response) = ui.allocate_exact_size(vec2(PICKER, PICKER), Sense::click_and_drag());
    if let Some(at) = response.interact_pointer_pos() {
        hsv.s = egui::remap_clamp(at.x, square.x_range(), 0.0..=1.0);
        hsv.v = egui::remap_clamp(at.y, square.bottom()..=square.top(), 0.0..=1.0);
    }
    const STEPS: u32 = 24;
    let mut mesh = egui::Mesh::default();
    for row in 0..=STEPS {
        for column in 0..=STEPS {
            let (s, v) = (column as f32 / STEPS as f32, row as f32 / STEPS as f32);
            mesh.colored_vertex(square.lerp_inside(vec2(s, 1.0 - v)), HsvaGamma { s, v, a: 1.0, ..hsv }.into());
            if row < STEPS && column < STEPS {
                let i = row * (STEPS + 1) + column;
                mesh.add_triangle(i, i + 1, i + STEPS + 1);
                mesh.add_triangle(i + 1, i + STEPS + 2, i + STEPS + 1);
            }
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    marker(painter, square.lerp_inside(vec2(hsv.s, 1.0 - hsv.v)), 5.0);

    ui.add_space(6.0);
    let (bar, response) = ui.allocate_exact_size(vec2(PICKER, 14.0), Sense::click_and_drag());
    if let Some(at) = response.interact_pointer_pos() {
        hsv.h = egui::remap_clamp(at.x, bar.x_range(), 0.0..=1.0);
    }
    const HUES: u32 = 36;
    let mut mesh = egui::Mesh::default();
    for i in 0..=HUES {
        let h = i as f32 / HUES as f32;
        let x = egui::lerp(bar.x_range(), h);
        let hue: Color32 = HsvaGamma { h, s: 1.0, v: 1.0, a: 1.0 }.into();
        mesh.colored_vertex(pos2(x, bar.top()), hue);
        mesh.colored_vertex(pos2(x, bar.bottom()), hue);
        if i < HUES {
            let j = 2 * i;
            mesh.add_triangle(j, j + 1, j + 2);
            mesh.add_triangle(j + 1, j + 3, j + 2);
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    marker(painter, pos2(egui::lerp(bar.x_range(), hsv.h), bar.center().y), 5.0);

    let changed = hsv != before;
    if changed {
        *colour = hsv.into();
    }
    ui.data_mut(|d| d.insert_temp(id, (*colour, hsv)));
    changed
}

/// A disc of all the hues round its centre.
fn colour_wheel(painter: &Painter, centre: Pos2, radius: f32) {
    const SLICES: usize = 36;
    let point = |i: usize| {
        let angle = i as f32 / SLICES as f32 * std::f32::consts::TAU;
        centre + radius * vec2(angle.cos(), angle.sin())
    };
    for i in 0..SLICES {
        let colour = Color32::from(egui::ecolor::Hsva::new((i as f32 + 0.5) / SLICES as f32, 0.75, 0.95, 1.0));
        painter.add(egui::Shape::convex_polygon(vec![centre, point(i), point(i + 1)], colour, Stroke::new(0.6, colour)));
    }
}

/// An arrow's head to choose: an arrow with that head, framed when chosen
/// as a selectable label is.
fn head_button(ui: &mut Ui, head: ArrowHead, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    let visuals = ui.style().interact_selectable(&response, on);
    let painter = ui.painter();
    if on || response.hovered() {
        painter.rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
    }
    let y = rect.center().y;
    paint_arrow(painter, pos2(rect.left() + 5.0, y), pos2(rect.right() - 5.0, y), 2.0, head, visuals.text_color());
    response
}

/// An arrow from `a` to `b`, as the marks are drawn on screen.
fn paint_arrow(painter: &Painter, a: Pos2, b: Pos2, width: f32, head: ArrowHead, colour: Color32) {
    let (shaft, tip) = annotate::arrow(a, b, width, head);
    painter.line_segment([a, shaft], Stroke::new(width, colour));
    painter.add(egui::Shape::convex_polygon(tip.to_vec(), colour, Stroke::NONE));
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
    (px, colour, style, contrast, font): (f32, Color32, TextStyle, Option<Color32>, &TextFont),
    used: &mut Vec<annotate::TextKey>,
) -> Option<(egui::TextureId, Rect)> {
    let key = (text.to_string(), px.to_bits(), colour, style, contrast, font.clone());
    if !a.texts.contains_key(&key) {
        // Larger text is drawn smaller and shown enlarged, in its own box.
        let limit = ctx.input(|i| i.max_texture_side).min(annotate::TEXT_LIMIT as usize) as f32;
        let scale = annotate::text_scale(text, px, font, limit);
        let (pixmap, [dx, dy]) = annotate::render_text(text, px * scale, colour, style, contrast, font)?;
        let size = [pixmap.width(), pixmap.height()];
        let image = ColorImage::from_rgba_premultiplied([size[0] as usize, size[1] as usize], pixmap.data());
        let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
        let texture = ctx.load_texture("annotate_text", image, options);
        let offset = [dx, dy].map(|d| (d as f32 / scale).round() as i32);
        let size = size.map(|s| (s as f32 / scale).round() as u32);
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
            // Inside only when chosen or pixelating: over the rest of it
            // other marks are drawn, text clicked in (it was always hit).
            Shape::Pixelate(r) if a.selected == Some(i) || a.tool == Tool::Pixelate => r.contains(p),
            Shape::Pixelate(r) => annotate::near_frame(*r, p, reach),
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
        return Some(Drag::Move { index: i, start: a.marks[i].clone(), from: p, text_box: boxes.get(i).copied().flatten() });
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
        Drag::Move { index, start, from, text_box } => {
            if let Some(mark) = a.marks.get_mut(*index) {
                *mark = start.moved(to - *from, size, *text_box);
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
            let look = (annotate::font_size(unit, m.size), m.colour, m.text, m.contrast, &font);
            text_texture(a, ctx, &text, at, look, used)
        })
        .collect();
    let boxes = texts.iter().map(|t| t.map(|(_, b)| b)).collect();
    (texts, boxes)
}

/// The marks of `a` over the image shown in `place` (points), `size` its
/// pixels as shown: drawn with the mouse on `response`, and painted.
/// `still`: the marks are being saved, and wait.
pub fn overlay(a: &mut Annotating, response: &Response, painter: &Painter, place: Rect, size: Vec2, still: bool) {
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
    if !still
        && response.drag_started_by(PointerButton::Primary)
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
    if !still
        && response.clicked()
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
            Some(i) if a.tool == Tool::Text && text_at(i).is_some() => a.edit_text(i),
            Some(i) => a.select(i),
            None if a.tool == Tool::Text && !was_typing && Rect::from_min_size(Pos2::ZERO, size).contains(p) => {
                a.selected = None;
                a.typing = Some(Typing { index: None, at: p.round(), text: String::new(), focus: true });
            }
            None => a.selected = None,
        }
    }
    if !still
        && response.double_clicked()
        && let Some(p) = response.interact_pointer_pos().map(to_image)
        && let Some(i) = hit(a, p, reach, &boxes)
    {
        a.edit_text(i);
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
                let (shaft, head) = annotate::arrow(*s, *e, annotate::line_width(unit, m.size), m.head);
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
        let look = (px, a.colour_of(Tool::Text), a.text_style, a.contrast, &font);
        let shown = text_texture(a, &ctx, &text, at, look, &mut used);
        if let Some((texture, b)) = shown {
            painter.image(texture, on_screen(b), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        // The point, so that it shows where the text goes.
        let p = to_screen(at);
        painter.line_segment([p, p + vec2(0.0, px * k)], Stroke::new(1.5, a.colour_of(Tool::Text)));
        field_at = Some(to_screen(pos2(at.x, shown.map_or(at.y + px * 1.25, |(_, b)| b.max.y))));
    }
    if let Some(Typing { index: Some(i), at, .. }) = &a.typing {
        let px = annotate::font_size(unit, a.marks.get(*i).map_or(a.size_of(Measure::Text), |m| m.size));
        let bottom = boxes.get(*i).copied().flatten().map_or(at.y + px * 1.25, |b| b.max.y);
        field_at = Some(to_screen(pos2(at.x, bottom)));
    }

    // The mark whose row in the list is under the pointer.
    if let Some(i) = a.hovered.take().filter(|&i| i < a.marks.len() && a.selected != Some(i)) {
        let r = on_screen(a.marks[i].bounds(boxes.get(i).copied().flatten())).expand(4.0);
        painter.rect_stroke(r, 2.0, Stroke::new(4.0, Color32::from_black_alpha(110)), StrokeKind::Outside);
        painter.rect_stroke(r, 2.0, Stroke::new(2.0, ctx.global_style().visuals.selection.bg_fill), StrokeKind::Outside);
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
            // Enter alone: `consume_key` takes Shift+Enter for Enter too.
            if ui.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter)) {
                commit = true;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            let hint = tr!("Text; Shift+Enter for a new line", "Текст; Shift+Enter — новая строка");
            let edit = egui::TextEdit::multiline(&mut typing.text)
                .id(Id::new(TEXT_ID))
                .return_key(egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                .desired_rows(1)
                .desired_width(260.0)
                .hint_text(hint);
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
            let mut output = ctx.run_ui(input, |root| {
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
            // No painter takes the font atlas here; dropped unhandled, it fails a debug assertion.
            output.textures_delta.clear();
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

    #[test]
    fn colours_are_typed_as_hex() {
        assert_eq!(typed_colour(" #1e7CF0 "), Some(Color32::from_rgb(0x1e, 0x7c, 0xf0)));
        assert_eq!(typed_colour("ffc400"), Some(Color32::from_rgb(0xff, 0xc4, 0x00)));
        assert_eq!(typed_colour("+12345"), None);
        assert_eq!(typed_colour("#12345"), None);
        assert_eq!(hex(Color32::from_rgb(0xe5, 0x2b, 0x2b)), "#e52b2b");
    }

    /// A row's action after the text being edited was emptied, which takes
    /// that mark out first: it acts on the mark of the row, not the next.
    #[test]
    fn rows_act_on_their_marks_after_text_goes() {
        let size = vec2(400.0, 300.0);
        let mut a = Annotating::new(Default::default(), size, crate::view::Zoom::Fit, Vec2::ZERO);
        let frame = |x: f32| Mark { shape: Shape::Rect(Rect::from_min_size(pos2(x, 10.0), vec2(20.0, 20.0))), colour: COLOURS[0], size: 2.0, text: TextStyle::Plain, font: TextFont::default(), head: ArrowHead::Normal, contrast: None };
        a.marks = vec![frame(0.0), frame(100.0), frame(200.0)];
        a.marks.insert(0, Mark { shape: Shape::Text(pos2(5.0, 5.0), "abc".into()), ..frame(0.0) });
        a.edit_text(0);
        a.typing.as_mut().unwrap().text.clear();
        a.delete(2);
        let left: Vec<f32> = a.marks.iter().filter_map(|m| m.frame()).map(|r| r.min.x).collect();
        assert_eq!(left, [0.0, 200.0]);
        assert_eq!(annotate::after_removal(Some(1), 1), None);
        assert_eq!(annotate::after_removal(Some(1), 3), Some(2));
        assert_eq!(annotate::after_removal(None, 3), Some(3));
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
        a.marks.push(Mark { shape: Shape::Rect(frame), colour: COLOURS[0], size: 2.0, text: TextStyle::Outline, font: TextFont::default(), head: ArrowHead::Normal, contrast: None });
        assert_eq!(hit(&a, pos2(100.0, 150.0), 4.0, &[None]), Some(1));
        assert_eq!(hit(&a, pos2(12.0, 12.0), 4.0, &[Some(Rect::from_min_max(pos2(10.0, 10.0), pos2(40.0, 30.0)))]), Some(0));
    }
}
