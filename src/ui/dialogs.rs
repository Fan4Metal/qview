//! Modal dialogs: the delete confirmation, the list of shortcuts and About.

use egui::{Align, Key, Layout, RichText, Ui};

use crate::app::{App, Dialog, file_name};

/// The key bindings as the Shortcuts dialog lists them.
fn shortcuts() -> Vec<(&'static str, &'static str)> {
    vec![
        ("→  Page Down  Space  Ctrl+→", tr!("Next image", "Следующее изображение")),
        ("←  Page Up  Backspace  Ctrl+←", tr!("Previous image", "Предыдущее изображение")),
        (tr!("Wheel", "Колесо мыши"), tr!("Previous / next image", "Предыдущее / следующее")),
        ("Home  End", tr!("First / last image", "Первое / последнее")),
        ("+  -", tr!("Zoom in / out", "Увеличить / уменьшить")),
        (tr!("Ctrl+Wheel", "Ctrl+колесо"), tr!("Zoom at the pointer", "Масштаб у курсора")),
        ("Num *", tr!("Fit image to window", "Вписать в окно")),
        ("Num /", tr!("Actual size (100%)", "Реальный размер (100%)")),
        ("←  →  ↑  ↓", tr!("Scroll a zoomed image", "Прокрутка увеличенного изображения")),
        (tr!("Drag", "Перетаскивание"), tr!("Scroll a zoomed image", "Прокрутка увеличенного изображения")),
        ("[  ]  Ctrl+Alt+←  Ctrl+Alt+→", tr!("Rotate left / right (view only)", "Повернуть влево / вправо (только просмотр)")),
        (
            tr!("F  Double click  Middle click", "F  Двойной щелчок  Средняя кнопка"),
            tr!("Full screen", "Полный экран"),
        ),
        ("T  B", tr!("Show or hide the toolbar / status bar", "Панель инструментов / строка состояния")),
        ("Delete", tr!("Move to the Recycle Bin", "Переместить в корзину")),
        ("Ctrl+C", tr!("Copy the file", "Копировать файл")),
        ("Ctrl+O", tr!("Open a file", "Открыть файл")),
        ("Shift+E", tr!("Open with the default program", "Открыть в программе по умолчанию")),
        ("F5", tr!("Reload the image and the folder", "Перечитать изображение и папку")),
        ("Esc  Ctrl+W", tr!("Leave full screen / close", "Выйти из полного экрана / закрыть")),
    ]
}

impl App {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        self.confirm_delete_dialog(ctx);
        // The key that opened a dialog this frame must not close it.
        let fresh = std::mem::take(&mut self.dialog_fresh);
        let closed = match self.dialog {
            Some(Dialog::Shortcuts) => info_modal(ctx, "shortcuts", fresh, |ui| {
                ui.heading(tr!("Keyboard Shortcuts", "Сочетания клавиш"));
                ui.add_space(6.0);
                egui::Grid::new("shortcut_grid").num_columns(2).spacing([24.0, 5.0]).striped(true).show(ui, |ui| {
                    for (keys, action) in shortcuts() {
                        ui.label(RichText::new(keys).monospace());
                        ui.label(action);
                        ui.end_row();
                    }
                });
            }),
            Some(Dialog::About) => self.about(ctx, fresh),
            Some(Dialog::Associations) => self.associations(ctx, fresh),
            None => false,
        };
        if closed {
            self.dialog = None;
        }
    }

    /// Icon, name, version, a short description, the author, links from
    /// Cargo.toml when they are set, and where the settings are kept, as in
    /// disk_flashlight. True when closed.
    fn about(&mut self, ctx: &egui::Context, fresh: bool) -> bool {
        const ICON: f32 = 64.0;
        const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
        const LICENSE: &str = env!("CARGO_PKG_LICENSE");
        /// Cargo joins several authors with `:`.
        const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
        let icon = super::app_icon(ctx, ICON, &mut self.about_icon);
        let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.image((icon.id(), egui::vec2(ICON, ICON)));
                ui.add_space(6.0);
                ui.heading("qview");
                ui.label(tr!(format!("Version {}", crate::VERSION), format!("Версия {}", crate::VERSION)));
                ui.add_space(8.0);
                ui.label(tr!(
                    "A fast and simple image viewer for Windows.",
                    "Быстрый и простой просмотрщик изображений для Windows."
                ));
                ui.add_space(8.0);
                if !AUTHORS.is_empty() {
                    let authors = AUTHORS.replace(':', ", ");
                    ui.label(tr!(format!("Author: {authors}"), format!("Автор: {authors}")));
                }
                if !REPOSITORY.is_empty() {
                    ui.hyperlink_to(tr!("Homepage", "Сайт проекта"), REPOSITORY).on_hover_text(REPOSITORY);
                }
                if !LICENSE.is_empty() {
                    ui.weak(tr!(format!("{LICENSE} License"), format!("Лицензия {LICENSE}")));
                }
                let place = ui.weak(tr!("Settings: in the user profile", "Настройки: в профиле пользователя"));
                match eframe::storage_dir(crate::APP_ID) {
                    Some(dir) => place.on_hover_text(dir.join("app.ron").display().to_string()),
                    None => place.on_hover_text(tr!("Settings are not saved", "Настройки не сохраняются")),
                };
                ui.add_space(4.0);
                ui.weak(tr!("Keyboard shortcuts: F1", "Сочетания клавиш: F1"));
                ui.add_space(8.0);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.button(tr!("Close", "Закрыть")).clicked())
                .inner
        });
        if modal.inner {
            return true;
        }
        if fresh {
            return false;
        }
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) || modal.should_close()
    }

    /// Registration with Windows and which types qview opens by default.
    /// True when closed.
    fn associations(&mut self, ctx: &egui::Context, fresh: bool) -> bool {
        use crate::assoc::{self, Status};
        use crate::filetypes::FILE_TYPES;
        const ICON: f32 = 24.0;
        let exe = std::env::current_exe().ok();
        // Read on every frame: cheap, and it follows choices made in
        // Settings while the dialog is open.
        let status = exe.as_deref().map_or(Status::NotRegistered, assoc::status);
        let defaults = assoc::defaults();
        let px = (ICON * ctx.pixels_per_point()).round() as u32;
        if self.type_icons.as_ref().is_none_or(|(n, _)| *n != px) {
            let icons = FILE_TYPES
                .iter()
                .map(|t| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [px as usize, px as usize],
                        &crate::type_icon::rgba(t, px),
                    );
                    ctx.load_texture(format!("type_icon_{}_{px}", t.id), image, egui::TextureOptions::LINEAR)
                })
                .collect();
            self.type_icons = Some((px, icons));
        }
        let icons = self.type_icons.as_ref().map(|(_, i)| i.clone()).unwrap_or_default();
        let ru = crate::i18n::lang() == crate::i18n::Lang::Ru;

        enum Action {
            Register,
            Choose,
            Unregister,
        }
        let mut action = None;
        let modal = egui::Modal::new(egui::Id::new("associations")).show(ctx, |ui| {
            ui.set_width(520.0);
            // The buttons below fill the row in Russian, so Close is a
            // cross in the title row.
            let closed = ui
                .horizontal(|ui| {
                    ui.heading(tr!("File Associations", "Сопоставление файлов"));
                    ui.with_layout(Layout::right_to_left(Align::Center), close_cross).inner
                })
                .inner;
            ui.add_space(6.0);
            ui.label(match &status {
                Status::NotRegistered => tr!(
                    "qview is not registered with Windows yet.".to_string(),
                    "qview ещё не зарегистрирован в Windows.".to_string()
                ),
                Status::Registered => tr!(
                    "qview is registered for the types below.".to_string(),
                    "qview зарегистрирован для типов файлов ниже.".to_string()
                ),
                Status::OtherCopy(path) => tr!(
                    format!("Another copy of qview is registered: {path}"),
                    format!("Зарегистрирована другая копия qview: {path}")
                ),
            });
            ui.add_space(6.0);
            egui::Grid::new("types").num_columns(4).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label("");
                ui.weak(tr!("Type", "Тип"));
                ui.weak(tr!("Extensions", "Расширения"));
                ui.weak(tr!("Default", "По умолчанию"));
                ui.end_row();
                for (i, t) in FILE_TYPES.iter().enumerate() {
                    if let Some(icon) = icons.get(i) {
                        ui.image((icon.id(), egui::vec2(ICON, ICON)));
                    }
                    ui.label(if ru { t.name_ru } else { t.name_en });
                    ui.label(t.extensions.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(" "));
                    ui.label(if defaults.get(i) == Some(&true) { "✔" } else { "—" });
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.label(tr!(
                "Windows lets only the user choose the default program. After registering, \
                 \"Choose as Default\" opens Settings: there, under \"Set defaults by app\", \
                 pick qview and the types it should open. Windows also offers qview the next \
                 time an image is opened.",
                "Программу по умолчанию Windows разрешает выбирать только пользователю. После \
                 регистрации кнопка «Выбрать по умолчанию» открывает параметры Windows: там в \
                 разделе «Задать значения по умолчанию по приложению» выберите qview и нужные типы \
                 файлов. Кроме того, Windows сама предложит qview при следующем открытии изображения."
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let register = match status {
                    Status::Registered => tr!("Update Registration", "Обновить регистрацию"),
                    _ => tr!("Register", "Зарегистрировать"),
                };
                if ui.button(register).clicked() {
                    action = Some(Action::Register);
                }
                let choose = egui::Button::new(tr!("Choose as Default…", "Выбрать по умолчанию…"));
                if ui.add_enabled(status == Status::Registered, choose).clicked() {
                    action = Some(Action::Choose);
                }
                let remove = egui::Button::new(tr!("Unregister", "Удалить регистрацию"));
                if ui.add_enabled(status != Status::NotRegistered, remove).clicked() {
                    action = Some(Action::Unregister);
                }
            });
            closed
        });
        match action {
            Some(Action::Register) => {
                let result = exe
                    .ok_or_else(|| "the path of the program is unknown".to_string())
                    .and_then(|e| assoc::register(&e));
                self.notice(match result {
                    Ok(()) => tr!("qview is registered".into(), "qview зарегистрирован".into()),
                    Err(e) => tr!(format!("Registration failed: {e}"), format!("Не удалось зарегистрировать: {e}")),
                });
            }
            Some(Action::Choose) => {
                if !assoc::open_default_apps() {
                    self.notice(tr!("Cannot open Settings".into(), "Не удалось открыть параметры".into()));
                }
            }
            Some(Action::Unregister) => {
                self.notice(match assoc::unregister() {
                    Ok(()) => tr!("The registration is removed".into(), "Регистрация удалена".into()),
                    Err(e) => tr!(
                        format!("Cannot remove the registration: {e}"),
                        format!("Не удалось удалить регистрацию: {e}")
                    ),
                });
            }
            None => {}
        }
        if modal.inner {
            return true;
        }
        if fresh {
            return false;
        }
        modal.should_close()
    }

    fn confirm_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.confirm_delete.clone() else { return };
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(tr!("Delete File", "Удаление файла"));
            ui.add_space(6.0);
            let name = file_name(&path);
            ui.label(tr!(
                format!("Move \"{name}\" to the Recycle Bin?"),
                format!("Переместить «{name}» в корзину?")
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(tr!("Delete", "Удалить")).clicked() {
                    decision = Some(true);
                }
                if ui.button(tr!("Cancel", "Отмена")).clicked() {
                    decision = Some(false);
                }
                ui.weak(tr!("Enter: delete, Esc: cancel", "Enter — удалить, Esc — отмена"));
            });
        });
        if decision.is_none() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                decision = Some(true);
            } else if modal.should_close() {
                decision = Some(false);
            }
        }
        match decision {
            Some(true) => {
                self.confirm_delete = None;
                self.delete(ctx, path);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }
}

/// A cross button, as in a window caption; true when clicked.
fn close_cross(ui: &mut Ui) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.bg_fill);
    }
    let (c, r) = (rect.center(), 5.0);
    let stroke = egui::Stroke::new(1.5, visuals.fg_stroke.color);
    ui.painter().line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], stroke);
    ui.painter().line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], stroke);
    response.on_hover_text(tr!("Close", "Закрыть")).clicked()
}

/// A modal with `content` and a Close button; true when it is closed
/// (the button, Esc, Enter, F1 or a click outside). Keys are ignored in the
/// frame the modal opens (`fresh`).
fn info_modal(ctx: &egui::Context, id: &str, fresh: bool, content: impl FnOnce(&mut Ui)) -> bool {
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
        ui.set_max_width(560.0);
        content(ui);
        ui.add_space(10.0);
        if ui.button(tr!("Close", "Закрыть")).clicked() {
            close = true;
        }
    });
    if fresh {
        return close;
    }
    let key = ctx.input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, Key::Enter) || i.consume_key(egui::Modifiers::NONE, Key::F1)
    });
    close || key || modal.should_close()
}
