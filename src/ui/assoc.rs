//! The File Associations dialog, in the look of MetalMPV's settings: one
//! rounded surface without inner frames, the title with a line under it,
//! the cross at the top right, and the content in cards of rows (the name
//! and a hint on the left, the button on the right): whether qview is
//! registered with Windows, the buttons that register, open Windows'
//! Default apps and remove the registration (`assoc.rs`), then the types
//! with their icons, extensions and whether qview opens them by default.

use egui::{Align, Color32, CornerRadius, FontId, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::app::App;
use crate::assoc::{self, Opener, Status};
use crate::filetypes::FILE_TYPES;
use crate::i18n::Lang;

/// The size of the window in points, less in a smaller window.
const SIZE: egui::Vec2 = vec2(640.0, 640.0);
/// The corner radius of the window.
const RADIUS: u8 = 10;
/// The side of a type's icon in points.
const ICON: f32 = 24.0;
/// The width of the registration's buttons.
const BUTTON: f32 = 170.0;
/// The extensions listed of a type; RAW has dozens.
const SHOWN_EXTENSIONS: usize = 6;

/// A card's ground, lighter than the window's (`visuals.window_fill`).
const CARD_BG: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
/// Lines between the rows of a card.
const LINE: Color32 = Color32::from_rgb(0x42, 0x42, 0x42);
const TEXT_BRIGHT: Color32 = Color32::WHITE;

enum Action {
    Register,
    Choose,
    Unregister,
}

impl App {
    /// Registration with Windows and which types qview opens by default.
    /// True when closed.
    pub(super) fn associations(&mut self, ctx: &egui::Context, fresh: bool) -> bool {
        let exe = std::env::current_exe().ok();
        // Read on every frame: cheap, and it follows choices made in
        // Settings while the dialog is open.
        let status = exe.as_deref().map_or(Status::NotRegistered, assoc::status);
        let defaults = assoc::defaults(exe.as_deref());
        let icons = self.type_icons(ctx);
        let ru = crate::i18n::lang() == Lang::Ru;

        let mut action = None;
        let mut closed = false;
        let modal = egui::Modal::new(egui::Id::new("associations"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .corner_radius(RADIUS)
                    .inner_margin(Margin::ZERO),
            )
            .show(ctx, |ui| {
                round_widgets(ui);
                let size = modal_size(ctx, SIZE, 12.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let cross = Rect::from_min_size(pos2(rect.right() - 14.0 - 24.0, rect.top() + 14.0), vec2(24.0, 24.0));
                closed = super::dialogs::close_cross(&mut child(ui, cross));
                // The scroll bar takes the right margin, next to the cards.
                let content = Rect::from_min_max(rect.left_top() + vec2(28.0, 18.0), rect.right_bottom() - vec2(10.0, 0.0));
                let mut ui = child(ui, content);
                ui.label(
                    RichText::new(tr!("File Associations", "Сопоставление файлов"))
                        .size(20.0)
                        .strong()
                        .color(TEXT_BRIGHT),
                );
                ui.add_space(2.0);
                ui.weak(tr!(
                    "Registering qview with Windows for image files.",
                    "Регистрация qview в Windows для файлов изображений."
                ));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin { left: 0, right: 18, top: 0, bottom: 18 })
                        .show(ui, |ui| {
                            action = self.registration(ui, &status);
                            types(ui, &icons, &defaults, ru);
                        });
                });
            });
        match action {
            Some(Action::Register) => {
                let result = exe
                    .ok_or_else(|| "the path of the program is unknown".to_string())
                    .and_then(|e| assoc::register(&e));
                self.assoc_message = Some(match result {
                    Ok(()) => tr!("qview is registered.".into(), "qview зарегистрирован.".into()),
                    Err(e) => tr!(format!("Registration failed: {e}"), format!("Не удалось зарегистрировать: {e}")),
                });
            }
            Some(Action::Choose) => {
                self.assoc_message = (!assoc::open_default_apps())
                    .then(|| tr!("Cannot open Settings.".into(), "Не удалось открыть параметры.".into()));
            }
            Some(Action::Unregister) => {
                self.assoc_message = Some(match assoc::unregister() {
                    Ok(()) => tr!("The registration is removed.".into(), "Регистрация удалена.".into()),
                    Err(e) => tr!(
                        format!("Cannot remove the registration: {e}"),
                        format!("Не удалось удалить регистрацию: {e}")
                    ),
                });
            }
            None => {}
        }
        if closed {
            return true;
        }
        !fresh && modal.should_close()
    }

    /// The Registration card and the note under it: a row per action, the
    /// buttons of one width in a column; the action clicked.
    fn registration(&self, ui: &mut Ui, status: &Status) -> Option<Action> {
        let mut action = None;
        let button = |ui: &mut Ui, enabled: bool, text: &str, disabled_tip: Option<&str>| {
            let size = vec2(BUTTON, ui.spacing().interact_size.y);
            let response = ui.add_enabled_ui(enabled, |ui| ui.add_sized(size, egui::Button::new(text))).inner;
            match disabled_tip {
                Some(tip) => response.on_disabled_hover_text(tip),
                None => response,
            }
            .clicked()
        };
        section(ui, tr!("Registration", "Регистрация"), |ui| {
            let state = match status {
                Status::NotRegistered => tr!("Not registered yet.".to_string(), "Ещё не зарегистрирован.".to_string()),
                Status::Registered => tr!(
                    "Registered for the types below.".to_string(),
                    "Зарегистрирован для типов файлов ниже.".to_string()
                ),
                Status::OtherCopy(path) => tr!(
                    format!("Another copy is registered: {path}"),
                    format!("Зарегистрирована другая копия: {path}")
                ),
            };
            let register = match status {
                Status::Registered => tr!("Update", "Обновить"),
                _ => tr!("Register", "Зарегистрировать"),
            };
            if row(ui, tr!("qview in Windows", "qview в Windows"), Some(&state), BUTTON, |ui| {
                button(ui, true, register, None)
            }) {
                action = Some(Action::Register);
            }
            divider(ui);
            if row(
                ui,
                tr!("Default program", "Программа по умолчанию"),
                Some(tr!(
                    "Opens Default apps in Windows' Settings: there, under \"Set defaults by app\", qview and the types it should open are chosen.",
                    "Открывает приложения по умолчанию в параметрах Windows: там в разделе «Задать значения по умолчанию по приложению» выбираются qview и нужные типы файлов."
                )),
                BUTTON,
                |ui| {
                    button(
                        ui,
                        *status == Status::Registered,
                        tr!("Choose…", "Выбрать…"),
                        Some(tr!("Windows offers qview once it is registered.", "Windows предложит qview после регистрации.")),
                    )
                },
            ) {
                action = Some(Action::Choose);
            }
            divider(ui);
            if row(
                ui,
                tr!("Remove the registration", "Удаление регистрации"),
                Some(tr!("Everything the registration wrote is removed.", "Удаляется всё, что записала регистрация.")),
                BUTTON,
                |ui| button(ui, *status != Status::NotRegistered, tr!("Unregister", "Удалить"), None),
            ) {
                action = Some(Action::Unregister);
            }
            if let Some(message) = &self.assoc_message {
                divider(ui);
                ui.add_space(8.0);
                ui.weak(message);
                ui.add_space(8.0);
            }
        });
        note(
            ui,
            tr!(
                "Windows lets only the user choose the default program. Windows also offers qview the next time an image is opened.",
                "Программу по умолчанию Windows разрешает выбирать только пользователю. Кроме того, Windows сама предложит qview при следующем открытии изображения."
            ),
        );
        action
    }

    /// The types' icons at the pixel size of [`ICON`] points, drawn once
    /// per size.
    fn type_icons(&mut self, ctx: &egui::Context) -> Vec<egui::TextureHandle> {
        let px = (ICON * ctx.pixels_per_point()).round() as u32;
        if self.type_icons.as_ref().is_none_or(|(n, _)| *n != px) {
            let icons = FILE_TYPES
                .iter()
                .map(|t| {
                    let image = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &crate::type_icon::rgba(t, px));
                    ctx.load_texture(format!("type_icon_{}_{px}", t.id), image, egui::TextureOptions::LINEAR)
                })
                .collect();
            self.type_icons = Some((px, icons));
        }
        self.type_icons.as_ref().map(|(_, i)| i.clone()).unwrap_or_default()
    }
}

/// The Types card: icon, name, extensions and whether qview opens the type
/// by default.
fn types(ui: &mut Ui, icons: &[egui::TextureHandle], defaults: &[Opener], ru: bool) {
    section(ui, tr!("Types", "Типы"), |ui| {
        ui.add_space(6.0);
        egui::Grid::new("types").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label("");
            ui.weak(tr!("Type", "Тип"));
            ui.weak(tr!("Extensions", "Расширения"));
            ui.weak(tr!("Default", "По умолчанию"));
            ui.end_row();
            for (i, t) in FILE_TYPES.iter().enumerate() {
                if let Some(icon) = icons.get(i) {
                    ui.image((icon.id(), vec2(ICON, ICON)));
                }
                ui.label(if ru { t.name_ru } else { t.name_en });
                let all: Vec<String> = t.extensions.iter().map(|e| format!(".{e}")).collect();
                let more = all.len().saturating_sub(SHOWN_EXTENSIONS);
                if more > 0 {
                    let text = all[..SHOWN_EXTENSIONS].join(" ");
                    ui.label(tr!(format!("{text} and {more} more"), format!("{text} и ещё {more}")))
                        .on_hover_ui(|ui| {
                            ui.set_max_width(320.0);
                            ui.label(all.join(" "));
                        });
                } else {
                    ui.label(all.join(" "));
                }
                match defaults.get(i) {
                    Some(Opener::Qview) => ui.label("✔"),
                    Some(Opener::QviewElsewhere) => ui.label("✔").on_hover_text(tr!(
                        "qview opens these files, as picked in \"Open with\". Choosing qview for this type \
                         in Settings gives the files its icon and name.",
                        "Эти файлы открывает qview, выбранный через «Открыть с помощью». Если выбрать qview \
                         для этого типа в параметрах Windows, файлы получат его значок и название."
                    )),
                    _ => ui.weak("—"),
                };
                ui.end_row();
            }
        });
        ui.add_space(6.0);
    });
}

/// A modal's content size: `wanted`, or less in a window too small for
/// it, leaving `margin` points free around it, so its cross stays on the
/// screen.
fn modal_size(ctx: &egui::Context, wanted: egui::Vec2, margin: f32) -> egui::Vec2 {
    let room = ctx.content_rect().size() - vec2(2.0 * margin, 2.0 * margin);
    wanted.min(room).max(vec2(200.0, 160.0))
}

/// Rounder buttons in the window.
fn round_widgets(ui: &mut Ui) {
    let w = &mut ui.style_mut().visuals.widgets;
    for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        v.corner_radius = CornerRadius::same(5);
    }
    ui.spacing_mut().button_padding = vec2(10.0, 4.0);
}

/// A part of the window in `rect`, laid out from the top.
fn child(ui: &mut Ui, rect: Rect) -> Ui {
    ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)))
}

/// A titled group: the title over a card, a lighter rounded ground
/// without a frame.
fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(16.0);
    ui.label(RichText::new(title).strong().color(TEXT_BRIGHT));
    ui.add_space(4.0);
    egui::Frame::new().fill(CARD_BG).corner_radius(8.0).inner_margin(Margin::symmetric(16, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// A remark under a card.
fn note(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(12.5).color(super::TEXT_WEAK));
}

/// A line between the rows of a card.
fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, LINE));
}

/// A row of a card: the name with an optional hint under it on the left,
/// what `control` adds right-aligned in `control_width` points on the
/// right, centred vertically.
fn row<R>(ui: &mut Ui, title: &str, hint: Option<&str>, control_width: f32, control: impl FnOnce(&mut Ui) -> R) -> R {
    const PAD: f32 = 8.0;
    let width = ui.available_width();
    let text_width = (width - control_width - 24.0).max(80.0);
    let title = ui.painter().layout(title.to_owned(), FontId::proportional(14.0), super::TEXT, text_width);
    let hint = hint.map(|h| ui.painter().layout(h.to_owned(), FontId::proportional(12.5), super::TEXT_WEAK, text_width));
    let text_height = title.size().y + hint.as_ref().map_or(0.0, |h| 2.0 + h.size().y);
    let height = text_height.max(ui.spacing().interact_size.y) + 2.0 * PAD;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut y = rect.center().y - text_height / 2.0;
    let title_height = title.size().y;
    ui.painter().galley(pos2(rect.left(), y), title, super::TEXT);
    y += title_height + 2.0;
    if let Some(hint) = hint {
        ui.painter().galley(pos2(rect.left(), y), hint, super::TEXT_WEAK);
    }
    let control_rect = Rect::from_min_max(pos2(rect.right() - control_width, rect.top()), rect.max);
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(control_rect).layout(Layout::right_to_left(Align::Center)));
    control(&mut ui)
}
