//! The toolbar: previous and next, rotate, zoom and delete buttons with
//! icons drawn as vector shapes, so they are crisp at any display scaling
//! and need no icon font.

use std::f32::consts::{FRAC_PI_2, PI};

use egui::{Color32, Painter, Pos2, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::{TOOLBAR_BG, panel_frame};
use crate::app::App;
use crate::input::Cmd;

const ICON: Color32 = Color32::from_rgb(0xe0, 0xe0, 0xe0);
const ICON_DISABLED: Color32 = Color32::from_rgb(0x74, 0x74, 0x74);
const HOVER: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
const PRESSED: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
const DIVIDER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);

#[derive(Clone, Copy)]
enum Icon {
    Prev,
    Next,
    RotateLeft,
    RotateRight,
    ZoomIn,
    ZoomOut,
    Delete,
}

impl App {
    pub(crate) fn toolbar(&mut self, root_ui: &mut Ui) {
        let has_image = self.shown.is_some();
        let prev = self.index.is_some_and(|i| i > 0);
        let next = self.index.is_some_and(|i| i + 1 < self.files.len());
        let file = self.current.is_some();
        egui::Panel::top("toolbar")
            .frame(panel_frame(TOOLBAR_BG, egui::Margin::symmetric(2, 2)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let buttons: [&[(Icon, Cmd, String, bool)]; 4] = [
                        &[
                            (Icon::Prev, Cmd::Prev, tr!("Previous (Page Up)", "Предыдущее (Page Up)").into(), prev),
                            (Icon::Next, Cmd::Next, tr!("Next (Page Down)", "Следующее (Page Down)").into(), next),
                        ],
                        &[
                            (Icon::RotateLeft, Cmd::RotateLeft, tr!("Rotate Left ([)", "Повернуть влево ([)").into(), has_image),
                            (Icon::RotateRight, Cmd::RotateRight, tr!("Rotate Right (])", "Повернуть вправо (])").into(), has_image),
                        ],
                        &[
                            (Icon::ZoomIn, Cmd::ZoomIn, tr!("Zoom In (+)", "Увеличить (+)").into(), has_image),
                            (Icon::ZoomOut, Cmd::ZoomOut, tr!("Zoom Out (-)", "Уменьшить (-)").into(), has_image),
                        ],
                        &[(Icon::Delete, Cmd::Delete, tr!("Delete (Del)", "Удалить (Del)").into(), file)],
                    ];
                    for (g, group) in buttons.iter().enumerate() {
                        if g > 0 {
                            divider(ui);
                        }
                        for (icon, cmd, tip, enabled) in group.iter() {
                            if icon_button(ui, *icon, tip, *enabled) {
                                self.clicked.push(*cmd);
                            }
                        }
                    }
                });
            });
    }
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 26.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, DIVIDER));
}

/// A square button with `icon`; true when clicked.
fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, enabled: bool) -> bool {
    // CLICK without FOCUSABLE: Space and Enter belong to the viewer.
    let sense = if enabled { Sense::CLICK } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 28.0), sense);
    let painter = ui.painter();
    if enabled && response.is_pointer_button_down_on() {
        painter.rect_filled(rect, 3.0, PRESSED);
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, 3.0, HOVER);
    }
    paint_icon(painter, icon, rect.center(), if enabled { ICON } else { ICON_DISABLED });
    let clicked = response.clicked();
    response.on_hover_text(tip);
    enabled && clicked
}

/// Points of an arc around `c`: angles in radians, clockwise on screen from
/// the right.
fn arc(c: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    const STEPS: usize = 24;
    (0..=STEPS)
        .map(|i| {
            let a = from + (to - from) * i as f32 / STEPS as f32;
            c + r * vec2(a.cos(), a.sin())
        })
        .collect()
}

fn mirror(points: &mut [Pos2], x: f32) {
    for p in points {
        p.x = 2.0 * x - p.x;
    }
}

fn paint_icon(painter: &Painter, icon: Icon, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.8, color);
    match icon {
        Icon::Prev | Icon::Next => {
            let mut shaft = vec![pos2(c.x - 7.5, c.y), pos2(c.x + 7.0, c.y)];
            let mut head = vec![pos2(c.x + 1.5, c.y - 5.5), pos2(c.x + 7.5, c.y), pos2(c.x + 1.5, c.y + 5.5)];
            if let Icon::Prev = icon {
                mirror(&mut shaft, c.x);
                mirror(&mut head, c.x);
            }
            painter.add(Shape::line(shaft, stroke));
            painter.add(Shape::line(head, stroke));
        }
        Icon::RotateLeft | Icon::RotateRight => {
            // A clockwise arc ending at the top, the arrow pointing right.
            let r = 6.5;
            let end = -FRAC_PI_2 + 2.0 * PI;
            let mut line = arc(c, r, end - 1.62 * PI, end);
            let tip = c + vec2(0.0, -r);
            let mut head = vec![tip + vec2(-2.5, -4.0), tip + vec2(3.2, 0.0), tip + vec2(-2.5, 4.0)];
            if let Icon::RotateLeft = icon {
                mirror(&mut line, c.x);
                mirror(&mut head, c.x);
            }
            painter.add(Shape::line(line, stroke));
            painter.add(Shape::convex_polygon(head, color, Stroke::NONE));
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            let lens = c + vec2(-1.5, -1.5);
            let r = 5.5;
            painter.circle_stroke(lens, r, stroke);
            let d = std::f32::consts::FRAC_1_SQRT_2;
            painter.line_segment([lens + vec2(d, d) * r, c + vec2(7.0, 7.0)], Stroke::new(2.6, color));
            painter.line_segment([lens - vec2(3.0, 0.0), lens + vec2(3.0, 0.0)], stroke);
            if let Icon::ZoomIn = icon {
                painter.line_segment([lens - vec2(0.0, 3.0), lens + vec2(0.0, 3.0)], stroke);
            }
        }
        Icon::Delete => {
            let s = Stroke::new(2.0, color);
            painter.line_segment([c + vec2(-6.0, -6.0), c + vec2(6.0, 6.0)], s);
            painter.line_segment([c + vec2(6.0, -6.0), c + vec2(-6.0, 6.0)], s);
        }
    }
}
