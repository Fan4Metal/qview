//! The toolbar: previous and next, rotate, crop, edit, zoom and delete buttons with
//! icons drawn as vector shapes, so they are crisp at any display scaling
//! and need no icon font. In the gallery: back, forward and up through the
//! folders instead, the gallery button and delete staying where they are.

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
    Gallery,
    Prev,
    Next,
    /// The folder above, in the gallery.
    Up,
    /// Mark as a favourite; filled when it is one.
    Star(bool),
    RotateLeft,
    RotateRight,
    Crop,
    /// Open in the editor (a pencil).
    Edit,
    ZoomIn,
    ZoomOut,
    Delete,
}

impl App {
    pub(crate) fn toolbar(&mut self, root_ui: &mut Ui) {
        let gallery = self.gallery_open;
        let has_image = self.shown.is_some();
        let prev = self.index.is_some_and(|i| i > 0);
        let next = self.index.is_some_and(|i| i + 1 < self.files.len());
        // Browsing and the file waits while an image is cropped.
        let cropping = self.crop.is_some();
        let (prev, next) = (prev && !cropping, next && !cropping);
        let file = self.current.is_some() && !cropping;
        // As the turns: while browsing fast the image on screen is often not
        // the current one yet, and the button would flicker.
        let crop = has_image;
        let edit_tip = match &self.editor {
            Some(e) => tr!(format!("Open in {} (Ctrl+E)", e.name), format!("Открыть в {} (Ctrl+E)", e.name)),
            None => tr!("Open in Editor (Ctrl+E)", "Открыть в редакторе (Ctrl+E)").into(),
        };
        let edit = (Icon::Edit, Cmd::Edit, edit_tip, file);
        let open = (Icon::Gallery, Cmd::Gallery, tr!("Gallery (G)", "Галерея (G)").into(), !cropping);
        let delete = (Icon::Delete, Cmd::Delete, tr!("Delete (Del)", "Удалить (Del)").into(), file);
        let favorite = self.current.as_deref().is_some_and(|c| self.favorites.contains(c));
        let star = if favorite {
            tr!("Remove from Favorites (S)", "Убрать из избранного (S)")
        } else {
            tr!("Add to Favorites (S)", "Добавить в избранное (S)")
        };
        let star = (Icon::Star(favorite), Cmd::Favorite, star.into(), file);
        egui::Panel::top("toolbar")
            .frame(panel_frame(TOOLBAR_BG, egui::Margin::symmetric(2, 2)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let buttons: Vec<Vec<(Icon, Cmd, String, bool)>> = if gallery {
                        vec![
                            vec![open],
                            vec![
                                (Icon::Prev, Cmd::Back, tr!("Back (Alt+←)", "Назад (Alt+←)").into(), self.history.can_go_back()),
                                (Icon::Next, Cmd::Forward, tr!("Forward (Alt+→)", "Вперёд (Alt+→)").into(), self.history.can_go_forward()),
                                (Icon::Up, Cmd::Up, tr!("Up (Alt+↑)", "Вверх (Alt+↑)").into(), self.parent_dir().is_some()),
                            ],
                            vec![edit, star, delete],
                        ]
                    } else {
                        vec![
                            vec![open],
                            vec![
                                (Icon::Prev, Cmd::Prev, tr!("Previous (Page Up)", "Предыдущее (Page Up)").into(), prev),
                                (Icon::Next, Cmd::Next, tr!("Next (Page Down)", "Следующее (Page Down)").into(), next),
                            ],
                            vec![
                                (Icon::RotateLeft, Cmd::RotateLeft, tr!("Rotate Left ([)", "Повернуть влево ([)").into(), has_image),
                                (Icon::RotateRight, Cmd::RotateRight, tr!("Rotate Right (])", "Повернуть вправо (])").into(), has_image),
                                (Icon::Crop, Cmd::Crop, tr!("Crop (C)", "Обрезать (C)").into(), crop),
                                edit,
                            ],
                            vec![
                                (Icon::ZoomIn, Cmd::ZoomIn, tr!("Zoom In (+)", "Увеличить (+)").into(), has_image),
                                (Icon::ZoomOut, Cmd::ZoomOut, tr!("Zoom Out (-)", "Уменьшить (-)").into(), has_image),
                            ],
                            vec![star, delete],
                        ]
                    };
                    for (g, group) in buttons.iter().enumerate() {
                        if g > 0 {
                            divider(ui);
                        }
                        for (icon, cmd, tip, enabled) in group.iter() {
                            let on = (matches!(icon, Icon::Gallery) && gallery) || (matches!(icon, Icon::Crop) && cropping);
                            if icon_button(ui, *icon, tip, *enabled, on) {
                                self.clicked.push(*cmd);
                            }
                        }
                    }
                    // A newer release, found by the update check, at the
                    // right end.
                    if let Some(tag) = self.updates.newer() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if update_button(ui, tag).clicked() {
                                let url = crate::update::release_url(tag);
                                if !crate::win::shell_open(&url) {
                                    log::warn!("could not open {url}");
                                }
                            }
                        });
                    }
                });
            });
    }
}

/// A newer release: its version with a download arrow in the link colour;
/// a click opens its release page.
fn update_button(ui: &mut Ui, tag: &str) -> egui::Response {
    let version = crate::update::version_of(tag);
    let icon = ui.id().with("update_icon");
    let resp = egui::Button::new((egui::Atom::custom(icon, vec2(12.0, 14.0)), version)).atom_ui(ui);
    let response = resp.response.clone().on_hover_text(tr!(
        format!("Version {version} is available: open its release page"),
        format!("Доступна версия {version}: открыть страницу выпуска")
    ));
    if let Some(rect) = resp.rect(icon) {
        let stroke = Stroke::new(1.6, ui.visuals().hyperlink_color);
        let painter = ui.painter();
        let (cx, top, bottom) = (rect.center().x, rect.top() + 1.5, rect.bottom() - 1.5);
        let tip = bottom - 3.0;
        // Arrow down onto a tray.
        painter.line_segment([pos2(cx, top), pos2(cx, tip)], stroke);
        painter.line_segment([pos2(cx - 4.0, tip - 4.0), pos2(cx, tip)], stroke);
        painter.line_segment([pos2(cx + 4.0, tip - 4.0), pos2(cx, tip)], stroke);
        painter.line_segment([pos2(rect.left() + 0.5, bottom), pos2(rect.right() - 0.5, bottom)], stroke);
    }
    response
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 26.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, DIVIDER));
}

/// A square button with `icon`, shown pressed while `on`; true when
/// clicked.
fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, enabled: bool, on: bool) -> bool {
    // CLICK without FOCUSABLE: Space and Enter belong to the viewer.
    let sense = if enabled { Sense::CLICK } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 28.0), sense);
    let painter = ui.painter();
    if enabled && (on || response.is_pointer_button_down_on()) {
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
        Icon::Gallery => {
            // Four thumbnails.
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let centre = c + vec2(dx, dy) * 4.0;
                painter.rect_filled(egui::Rect::from_center_size(centre, vec2(6.0, 6.0)), 1.0, color);
            }
        }
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
        Icon::Star(on) => crate::ui::paint_star(painter, c, 8.5, on.then_some(crate::ui::STAR), if on { crate::ui::STAR } else { color }),
        Icon::Up => {
            painter.line_segment([pos2(c.x, c.y + 7.0), pos2(c.x, c.y - 7.5)], stroke);
            painter.add(Shape::line(vec![pos2(c.x - 5.5, c.y - 1.5), pos2(c.x, c.y - 7.5), pos2(c.x + 5.5, c.y - 1.5)], stroke));
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
        Icon::Crop => {
            // Two right angles crossing, as on a cropping tool.
            let p = |x: f32, y: f32| c + vec2(x, y);
            painter.add(Shape::line(vec![p(-4.5, -8.0), p(-4.5, 4.5), p(8.0, 4.5)], stroke));
            painter.add(Shape::line(vec![p(-8.0, -4.5), p(4.5, -4.5), p(4.5, 8.0)], stroke));
        }
        Icon::Edit => {
            // A pencil from the bottom left, its point down: along `d`, as
            // wide as `n` across.
            let (d, n) = (vec2(1.0, -1.0).normalized(), vec2(1.0, 1.0).normalized());
            let tip = c + vec2(-7.0, 7.0);
            let side = |along: f32, across: f32| tip + d * along + n * across;
            painter.add(Shape::closed_line(vec![tip, side(5.0, 2.8), side(17.5, 2.8), side(17.5, -2.8), side(5.0, -2.8)], stroke));
            // Where the point is sharpened, and the band by the end.
            painter.line_segment([side(5.0, 2.8), side(5.0, -2.8)], Stroke::new(1.2, color));
            painter.line_segment([side(14.5, 2.8), side(14.5, -2.8)], Stroke::new(1.2, color));
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
            // A bin: the handle, the lid, the body narrowing downwards and
            // two ribs.
            let p = |x: f32, y: f32| c + vec2(x, y);
            painter.add(Shape::line(vec![p(-2.5, -5.5), p(-2.5, -7.5), p(2.5, -7.5), p(2.5, -5.5)], stroke));
            painter.line_segment([p(-7.0, -5.5), p(7.0, -5.5)], stroke);
            painter.add(Shape::line(vec![p(-5.0, -3.0), p(-4.0, 7.0), p(4.0, 7.0), p(5.0, -3.0)], stroke));
            let rib = Stroke::new(1.4, color);
            painter.line_segment([p(-1.5, -0.5), p(-1.3, 4.5)], rib);
            painter.line_segment([p(1.5, -0.5), p(1.3, 4.5)], rib);
        }
    }
}
