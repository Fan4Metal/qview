//! Cropping on screen: the frame over the image, dragged by its edges,
//! corners or inside, and the bar above the image with the proportions,
//! the size and Save, Save As and Cancel.

use egui::{Color32, CursorIcon, Painter, PointerButton, Pos2, Rect, Response, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::{TOOLBAR_BG, panel_frame};
use crate::app::App;
use crate::crop::{self, ASPECTS, Crop, Grip};
use crate::input::Cmd;

/// How near an edge, in points, a drag takes it.
const REACH: f32 = 8.0;
/// Side of the squares at the corners and the middles of the edges.
const HANDLE: f32 = 7.0;

impl App {
    pub(crate) fn crop_bar(&mut self, root_ui: &mut Ui) {
        let Some((_, picture)) = self.editable() else { return };
        let size = self.view.rotated(picture.size());
        let saving = self.saving();
        let Some(crop) = &mut self.crop else { return };
        let mut clicked = None;
        egui::Panel::top("crop_bar")
            .frame(panel_frame(TOOLBAR_BG, egui::Margin::symmetric(8, 4)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(tr!("Crop", "Обрезка"));
                    ui.add_space(8.0);
                    ui.label(tr!("Proportions:", "Пропорции:"));
                    let mut aspect = crop.aspect;
                    egui::ComboBox::from_id_salt("crop_aspect").selected_text(aspect.label()).show_ui(ui, |ui| {
                        for a in ASPECTS {
                            ui.selectable_value(&mut aspect, a, a.label());
                        }
                    });
                    if aspect != crop.aspect {
                        crop.set_aspect(aspect, size);
                    }
                    let [_, _, w, h] = crop::pixels(crop.rect, size);
                    ui.label(format!("{w} × {h}"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr!("Cancel", "Отмена")).on_hover_text("Esc").clicked() {
                            clicked = Some(Cmd::Crop);
                        }
                        let save_as = egui::Button::new(tr!("Save As…", "Сохранить как…"));
                        if ui.add_enabled(!saving, save_as).on_hover_text("Ctrl+Shift+S").clicked() {
                            clicked = Some(Cmd::SaveAs);
                        }
                        let save = egui::Button::new(tr!("Save", "Сохранить"));
                        if ui.add_enabled(!saving, save).on_hover_text(tr!("Enter, Ctrl+S", "Enter, Ctrl+S")).clicked() {
                            clicked = Some(Cmd::Save);
                        }
                        if saving {
                            ui.add(egui::Spinner::new());
                        }
                    });
                });
            });
        self.clicked.extend(clicked);
    }
}

/// The frame of `crop` over the image shown in `place` (points), `size`
/// its pixels as shown: dragged with the mouse on `response`, and painted
/// with the outside dimmed.
pub fn frame(crop: &mut Crop, response: &Response, painter: &Painter, place: Rect, size: Vec2) {
    // Points per image pixel.
    let k = place.width() / size.x;
    let to_image = |p: Pos2| ((p - place.min) / k).to_pos2();
    let on_screen = |r: Rect| Rect::from_min_max(place.min + r.min.to_vec2() * k, place.min + r.max.to_vec2() * k);
    let ctx = response.ctx.clone();

    if response.drag_started_by(PointerButton::Primary)
        && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
    {
        crop.drag = Some((crop::grip(on_screen(crop.rect), origin, REACH), crop.rect, to_image(origin)));
    }
    if let Some((grip, start, from)) = crop.drag {
        if response.dragged_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            crop.rect = crop::dragged(grip, start, from, to_image(p), size, crop.aspect.value(size));
        }
        if !response.dragged() {
            crop.drag = None;
        }
    }

    let held = crop.drag.map(|(g, ..)| g).or_else(|| response.hover_pos().map(|p| crop::grip(on_screen(crop.rect), p, REACH)));
    if let Some(grip) = held {
        ctx.set_cursor_icon(cursor(grip));
    }

    let f = on_screen(crop.rect);
    // The outside dimmed: above, below, left and right of the frame.
    let shade = Color32::from_black_alpha(150);
    for r in [
        Rect::from_min_max(place.min, pos2(place.max.x, f.min.y)),
        Rect::from_min_max(pos2(place.min.x, f.max.y), place.max),
        Rect::from_min_max(pos2(place.min.x, f.min.y), pos2(f.min.x, f.max.y)),
        Rect::from_min_max(pos2(f.max.x, f.min.y), pos2(place.max.x, f.max.y)),
    ] {
        painter.rect_filled(r, 0.0, shade);
    }
    // The rule of thirds.
    let thirds = Stroke::new(1.0, Color32::from_white_alpha(70));
    for i in 1..3 {
        let t = i as f32 / 3.0;
        painter.vline(f.min.x + f.width() * t, f.y_range(), thirds);
        painter.hline(f.x_range(), f.min.y + f.height() * t, thirds);
    }
    painter.rect_stroke(f, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
    let (c, x, y) = (f.center(), [f.min.x, f.center().x, f.max.x], [f.min.y, f.center().y, f.max.y]);
    for hx in x {
        for hy in y {
            let at = pos2(hx, hy);
            if at != c {
                let handle = Rect::from_center_size(at, vec2(HANDLE, HANDLE));
                painter.rect(handle, 0.0, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
            }
        }
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
