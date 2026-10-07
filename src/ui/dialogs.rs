//! Modal dialogs: the delete confirmation, renaming, the list of shortcuts
//! and About.

use egui::{Align, Key, Layout, RichText, Ui};

use crate::app::{App, Dialog, file_name};
use crate::i18n::LangChoice;

/// Width of the language list in About, enough for its longest entry.
const LANG_WIDTH: f32 = 220.0;

/// The key bindings as the Shortcuts dialog lists them, in sections.
fn shortcuts() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (
            tr!("Browsing", "Просмотр"),
            vec![
                ("→  Page Down  Space  Ctrl+→", tr!("Next image", "Следующее изображение")),
                ("←  Page Up  Backspace  Ctrl+←", tr!("Previous image", "Предыдущее изображение")),
                (tr!("Wheel", "Колесо мыши"), tr!("Previous / next image", "Предыдущее / следующее")),
                ("Home  End", tr!("First / last image", "Первое / последнее")),
                (tr!("G  Enter  Double click", "G  Enter  Двойной щелчок"), tr!("Open the gallery", "Открыть галерею")),
            ],
        ),
        (
            tr!("Zoom", "Масштаб"),
            vec![
                ("+  =  -", tr!("Zoom in / out", "Увеличить / уменьшить")),
                (tr!("Ctrl+Wheel", "Ctrl+колесо"), tr!("Zoom at the pointer", "Масштаб у курсора")),
                ("1  Num /", tr!("Actual size (100%)", "Реальный размер (100%)")),
                ("2  Num *", tr!("Fit image to window", "Вписать в окно")),
                ("3", tr!("Fill the window (enlarge too)", "Заполнить окно (и с увеличением)")),
                ("4", tr!("Fill the entire window (crops)", "Заполнить окно целиком (с обрезкой)")),
                ("L", tr!("Keep zoom and position for the next images", "Сохранять масштаб и положение для следующих")),
                (
                    tr!("←  →  ↑  ↓  Drag", "←  →  ↑  ↓  Перетаскивание"),
                    tr!("Scroll a zoomed image", "Прокрутка увеличенного изображения"),
                ),
            ],
        ),
        (
            tr!("Rotating and cropping", "Поворот и обрезка"),
            vec![
                (
                    "[  ]  Ctrl+Alt+←  Ctrl+Alt+→",
                    tr!("Rotate left / right (the file changes on saving)", "Повернуть влево / вправо (файл меняется при сохранении)"),
                ),
                ("H  V", tr!("Flip horizontally / vertically", "Отразить по горизонтали / по вертикали")),
                ("C", tr!("Crop: drag the frame, its edges or corners", "Обрезка: перетаскивайте рамку, её края или углы")),
                ("Enter  Esc", tr!("While cropping: save / cancel", "При обрезке: сохранить / отменить")),
                ("Ctrl+S", tr!("Save the turned, flipped or cropped image", "Сохранить повёрнутое, отражённое или обрезанное")),
                ("Ctrl+Shift+S", tr!("Save as another file or format", "Сохранить в другой файл или формат")),
                ("Ctrl+E", tr!("Open in the editor chosen last", "Открыть в последнем выбранном редакторе")),
            ],
        ),
        (
            tr!("Animation", "Анимация"),
            vec![
                ("P", tr!("Pause / play", "Приостановить / продолжить")),
                (",  .", tr!("Previous / next frame", "Предыдущий / следующий кадр")),
            ],
        ),
        (
            tr!("Gallery", "Галерея"),
            vec![
                (
                    tr!("Click  arrows  Page Up/Down  Home  End", "Щелчок  стрелки  Page Up/Down  Home  End"),
                    tr!("Select an image", "Выбрать изображение"),
                ),
                (
                    tr!("G  Enter  Double click", "G  Enter  Двойной щелчок"),
                    tr!("Show the selected image", "Показать выбранное изображение"),
                ),
                ("F", tr!("Show the selected image in full screen", "Показать выбранное на полном экране")),
                (
                    tr!("Ctrl+click  Shift+click  Drag", "Ctrl+щелчок  Shift+щелчок  Перетаскивание"),
                    tr!("Choose several images", "Выбрать несколько изображений"),
                ),
                (
                    tr!("Shift+arrows/Page Up/Page Down/Home/End  Ctrl+A  Esc", "Shift+стрелки/Page Up/Page Down/Home/End  Ctrl+A  Esc"),
                    tr!("Choose on the way / all / none", "Выбрать по пути / все / снять выбор"),
                ),
                (tr!("+  -  Ctrl+Wheel", "+  -  Ctrl+колесо"), tr!("Thumbnail size", "Размер миниатюр")),
                (
                    tr!("Alt+←  Backspace  Alt+→  Mouse side buttons", "Alt+←  Backspace  Alt+→  Боковые кнопки мыши"),
                    tr!("Previous / next folder", "Предыдущая / следующая папка"),
                ),
                ("Alt+↑", tr!("Folder above", "Папка уровнем выше")),
                (
                    tr!("Enter  Double click", "Enter  Двойной щелчок"),
                    tr!("On a folder's cell: open the folder", "На ячейке папки: открыть папку"),
                ),
                (
                    "/  Ctrl+F  Esc",
                    tr!("Filter the images by name (*.png and the like too) / clear", "Фильтр изображений по имени (и маски вида *.png) / очистить"),
                ),
                ("Ctrl+Shift+F", tr!("Full screen for the gallery (with the bars)", "Галерея на полном экране (с панелями)")),
            ],
        ),
        (
            tr!("Files", "Файлы"),
            vec![
                ("S", tr!("Add to / remove from the favorites", "Добавить в избранное / убрать из него")),
                ("Delete", tr!("Move to the Recycle Bin", "Переместить в корзину")),
                ("F2", tr!("Rename the file", "Переименовать файл")),
                (
                    "Alt+1 … Alt+9",
                    tr!("Move the file into the pinned folder with that key", "Переместить файл в закреплённую папку с этой клавишей"),
                ),
                ("Shift+Alt+1 … 9", tr!("Copy it into that folder", "Копировать файл в эту папку")),
                ("Ctrl+Z", tr!("Undo the last rename, move, copy or save", "Отменить последнее переименование, перемещение, копирование или сохранение")),
                ("Ctrl+C", tr!("Copy the file", "Копировать файл")),
                ("Ctrl+Shift+C", tr!("Copy the image as shown", "Копировать картинку, как она показана")),
                (
                    tr!("Ctrl+V  Shift+Insert", "Ctrl+V  Shift+Insert"),
                    tr!("Open the clipboard's image, file or path", "Открыть изображение, файл или путь из буфера обмена"),
                ),
                ("Ctrl+P", tr!("Print the image as shown, or those chosen", "Печать изображения, как оно показано, или выбранных")),
                ("Ctrl+O", tr!("Open a file", "Открыть файл")),
                (
                    "F5",
                    tr!(
                        "Reload the image and the folder; in the gallery, the folder tree too",
                        "Перечитать изображение и папку; в галерее и дерево папок"
                    ),
                ),
            ],
        ),
        (
            tr!("Window", "Окно"),
            vec![
                (tr!("F  Ctrl+Shift+F  Middle click", "F  Ctrl+Shift+F  Средняя кнопка"), tr!("Full screen", "Полный экран")),
                ("Shift+F", tr!("Slideshow in full screen: start / stop", "Слайд-шоу на полном экране: начать / остановить")),
                (tr!("Space  Esc", "Space  Esc"), tr!("In a slideshow: pause / stop", "В слайд-шоу: пауза / стоп")),
                ("T  B", tr!("Show or hide the toolbar / status bar", "Панель инструментов / строка состояния")),
                ("I", tr!("Show or hide the information panel (EXIF)", "Панель сведений (EXIF)")),
                ("F1", tr!("This list", "Этот список")),
                (
                    "Esc",
                    tr!(
                        "Leave full screen; then the gallery, from it close",
                        "Выйти из полного экрана; затем галерея, из неё закрыть"
                    ),
                ),
                ("Ctrl+W  Alt+F4", tr!("Close", "Закрыть")),
            ],
        ),
    ]
}

impl App {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        self.confirm_delete_dialog(ctx);
        self.confirm_clear_favorites_dialog(ctx);
        self.confirm_unpin_all_dialog(ctx);
        self.confirm_edit_dialog(ctx);
        self.rename_dialog(ctx);
        self.batch_rename_dialog(ctx);
        // The key that opened a dialog this frame must not close it.
        let fresh = std::mem::take(&mut self.dialog_fresh);
        let closed = match self.dialog {
            Some(Dialog::Shortcuts) => info_modal(ctx, "shortcuts", tr!("Keyboard Shortcuts", "Сочетания клавиш"), fresh, |ui| {
                // Longer than a small window: scrolled within all of it but
                // the title and the modal's margins.
                let height = (ctx.content_rect().height() - 120.0).max(200.0);
                // A modal's content gets the height of its previous frame,
                // so the list would never grow past its first size: when it
                // scrolls, it takes `height` all the same.
                egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).show(ui, |ui| {
                    // One grid, so that the columns line up across sections.
                    egui::Grid::new("shortcut_grid").num_columns(2).spacing([24.0, 5.0]).show(ui, |ui| {
                        for (i, (title, keys)) in shortcuts().into_iter().enumerate() {
                            if i > 0 {
                                ui.add_space(6.0);
                                ui.end_row();
                            }
                            ui.label(RichText::new(title).strong());
                            ui.end_row();
                            for (keys, action) in keys {
                                ui.label(RichText::new(keys).monospace());
                                ui.label(action);
                                ui.end_row();
                            }
                        }
                    });
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
    /// Cargo.toml when they are set, and where the settings are kept. True
    /// when closed.
    fn about(&mut self, ctx: &egui::Context, fresh: bool) -> bool {
        const ICON: f32 = 64.0;
        const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
        const LICENSE: &str = env!("CARGO_PKG_LICENSE");
        /// Cargo joins several authors with `:`.
        const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
        let icon = super::app_icon(ctx, ICON, &mut self.about_icon);
        let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            // The cross in the top right corner, over the centred content
            // (a child Ui takes no room in the layout).
            let corner = egui::Rect::from_min_size(
                egui::pos2(ui.max_rect().right() - 24.0, ui.cursor().top()),
                egui::vec2(24.0, 24.0),
            );
            let closed = close_cross(&mut ui.new_child(egui::UiBuilder::new().max_rect(corner)));
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
                    // Not `hyperlink_to`: eframe opens links only with its `links`
                    // feature (the webbrowser crate), which is off.
                    if ui.link(tr!("Homepage", "Сайт проекта")).on_hover_text(REPOSITORY).clicked()
                        && !crate::win::shell_open(REPOSITORY)
                    {
                        log::warn!("could not open {REPOSITORY}");
                    }
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
                ui.separator();
                ui.add_space(4.0);
                // The label over the list, both centred; a fixed width
                // keeps the list from jumping when the language changes.
                ui.weak(tr!("Interface language", "Язык интерфейса"));
                let mut choice = self.lang;
                // A combo box lays itself out left to right, ignoring the
                // centring: indented by hand (`width` is its outer width).
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - LANG_WIDTH) / 2.0).max(0.0));
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(choice.label())
                        .width(LANG_WIDTH)
                        .show_ui(ui, |ui| {
                            for c in LangChoice::ALL {
                                ui.selectable_value(&mut choice, c, c.label());
                            }
                        });
                });
                if choice != self.lang {
                    self.lang = choice;
                    crate::i18n::set_lang(choice.resolve());
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                update_section(ui, &mut self.updates);
                ui.add_space(4.0);
            });
            closed
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
        let defaults = assoc::defaults(exe.as_deref());
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
                    // RAW has dozens: the common ones and the count.
                    let shown: Vec<String> = t.extensions.iter().take(6).map(|e| format!(".{e}")).collect();
                    let more = t.extensions.len().saturating_sub(shown.len());
                    let text = shown.join(" ");
                    ui.label(if more > 0 {
                        tr!(format!("{text} and {more} more"), format!("{text} и ещё {more}"))
                    } else {
                        text
                    });
                    match defaults.get(i) {
                        Some(assoc::Opener::Qview) => ui.label("✔"),
                        Some(assoc::Opener::QviewElsewhere) => ui.label("✔").on_hover_text(tr!(
                            "qview opens these files, as picked in \"Open with\". Choosing qview for this type \
                             in Settings gives the files its icon and name.",
                            "Эти файлы открывает qview, выбранный через «Открыть с помощью». Если выбрать qview \
                             для этого типа в параметрах Windows, файлы получат его значок и название."
                        )),
                        _ => ui.label("—"),
                    };
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
        let Some(paths) = self.confirm_delete.clone() else { return };
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            let n = paths.len();
            if n == 1 {
                ui.heading(tr!("Delete File", "Удаление файла"));
                ui.add_space(6.0);
                let name = file_name(&paths[0]);
                ui.label(tr!(
                    format!("Move \"{name}\" to the Recycle Bin?"),
                    format!("Переместить «{name}» в корзину?")
                ));
            } else {
                ui.heading(tr!("Delete Files", "Удаление файлов"));
                ui.add_space(6.0);
                ui.label(tr!(format!("Move {n} files to the Recycle Bin?"), format!("Переместить файлы ({n}) в корзину?")));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                let delete = egui::Button::new(text(tr!("Delete", "Удалить")).color(egui::Color32::WHITE))
                    .fill(super::DANGER)
                    .min_size(egui::vec2(160.0, 34.0));
                if ui.add(delete).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
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
                self.delete(ctx, paths);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }
}

impl App {
    /// Open many files in the editor? Enter opens, Esc cancels.
    fn confirm_edit_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = &self.confirm_edit else { return };
        let n = request.files.len();
        let name = self.editor_name(request.editor.as_ref());
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_edit")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading(tr!("Open in Editor", "Открытие в редакторе"));
            ui.add_space(6.0);
            ui.label(match &name {
                Some(name) => tr!(format!("Open {n} files in {name}?"), format!("Открыть файлы ({n}) в {name}?")),
                None => tr!(format!("Open {n} files in the editor?"), format!("Открыть файлы ({n}) в редакторе?")),
            });
            ui.weak(tr!(
                "Programs that take one file at a time open a window for each.",
                "Программы, открывающие по одному файлу, откроют окно для каждого."
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                if ui.add(egui::Button::new(text(tr!("Open", "Открыть"))).min_size(egui::vec2(160.0, 34.0))).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                decision = Some(true);
            } else if modal.should_close() {
                decision = Some(false);
            }
        }
        if let Some(open) = decision
            && let Some(request) = self.confirm_edit.take()
            && open
        {
            self.edit_files(ctx, request.files, request.editor);
        }
    }

    /// Clear the favourites? Enter clears, Esc cancels.
    fn confirm_clear_favorites_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_clear_favorites {
            return;
        }
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_clear_favorites")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(tr!("Clear Favorites", "Очистка избранного"));
            ui.add_space(6.0);
            let n = self.favorites.len();
            ui.label(tr!(
                format!("Remove all {n} images from the favorites? The files stay where they are."),
                format!("Убрать из избранного все изображения ({n})? Сами файлы останутся на месте.")
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                let clear = egui::Button::new(text(tr!("Clear", "Очистить")).color(egui::Color32::WHITE))
                    .fill(super::DANGER)
                    .min_size(egui::vec2(160.0, 34.0));
                if ui.add(clear).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                decision = Some(true);
            } else if modal.should_close() {
                decision = Some(false);
            }
        }
        if let Some(clear) = decision {
            self.confirm_clear_favorites = false;
            if clear {
                self.clear_favorites(ctx);
            }
        }
    }

    /// Unpin every folder from Quick Access? Enter unpins, Esc cancels.
    fn confirm_unpin_all_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_unpin_all {
            return;
        }
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_unpin_all")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(tr!("Unpin All", "Открепление всех папок"));
            ui.add_space(6.0);
            let n = self.pinned.len();
            ui.label(tr!(
                format!("Unpin all {n} folders from Quick Access, with their keys? The folders stay where they are."),
                format!("Открепить от панели быстрого доступа все папки ({n}) вместе с их клавишами? Сами папки останутся на месте.")
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                let unpin = egui::Button::new(text(tr!("Unpin", "Открепить")).color(egui::Color32::WHITE))
                    .fill(super::DANGER)
                    .min_size(egui::vec2(160.0, 34.0));
                if ui.add(unpin).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                decision = Some(true);
            } else if modal.should_close() {
                decision = Some(false);
            }
        }
        if let Some(unpin) = decision {
            self.confirm_unpin_all = false;
            if unpin {
                self.unpin_all();
            }
        }
    }

    /// The new name of a file, the old one selected but its extension, as
    /// in Explorer; Enter renames, Esc cancels.
    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(rename) = self.rename.as_mut() else { return };
        let id = egui::Id::new("rename_name");
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("rename")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(tr!("Rename", "Переименование"));
            ui.add_space(8.0);
            let edit = ui.add(egui::TextEdit::singleline(&mut rename.name).id(id).desired_width(f32::INFINITY));
            if std::mem::take(&mut rename.focus) {
                edit.request_focus();
            }
            if std::mem::take(&mut rename.select) {
                select_stem(ctx, id, &rename.name);
            }
            if edit.changed() {
                rename.error = None;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                decision = Some(true);
            }
            if let Some(error) = &rename.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(0xff, 0x8a, 0x80), error);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                if ui.add(egui::Button::new(text(tr!("Rename", "Переименовать"))).min_size(egui::vec2(160.0, 34.0))).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() && modal.should_close() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                let (path, name) = (rename.path.clone(), rename.name.clone());
                match self.rename_to(ctx, &path, &name) {
                    Ok(()) => self.rename = None,
                    Err(e) => {
                        if let Some(rename) = &mut self.rename {
                            rename.error = Some(e);
                            rename.focus = true;
                        }
                    }
                }
            }
            Some(false) => self.rename = None,
            None => {}
        }
    }
}

impl App {
    /// Several files renamed to a name and a number each, in the order of
    /// the grid; the first and last new names shown. Enter renames, Esc
    /// cancels.
    fn batch_rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(batch) = self.batch_rename.as_mut() else { return };
        let id = egui::Id::new("batch_rename_name");
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("batch_rename")).show(ctx, |ui| {
            ui.set_width(460.0);
            let n = batch.paths.len();
            ui.heading(tr!(format!("Rename {n} Files"), format!("Переименование файлов ({n})")));
            ui.add_space(8.0);
            let mut changed = false;
            egui::Grid::new("batch_rename_fields").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr!("Name:", "Имя:"));
                let edit = ui.add(egui::TextEdit::singleline(&mut batch.base).id(id).desired_width(f32::INFINITY));
                if std::mem::take(&mut batch.focus) {
                    edit.request_focus();
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    decision = Some(true);
                }
                changed |= edit.changed();
                ui.end_row();
                ui.label(tr!("Start at:", "Начать с:"));
                changed |= ui.add(egui::DragValue::new(&mut batch.start).range(0..=999_999)).changed();
                ui.end_row();
            });
            if changed {
                batch.error = None;
            }
            ui.add_space(8.0);
            // The first names and the last: what the numbering looks like.
            let news = crate::rename::numbered(&batch.paths, &batch.base, batch.start);
            let shown: Vec<usize> = if n <= 4 { (0..n).collect() } else { vec![0, 1, 2, n - 1] };
            // The old and the new names share the width, the arrow between.
            let half = ((ui.available_width() - 30.0) / 2.0).max(60.0);
            let height = ui.spacing().interact_size.y;
            let name = |ui: &mut Ui, text: RichText| {
                ui.allocate_ui_with_layout(egui::vec2(half, height), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(half);
                    ui.add(egui::Label::new(text).truncate());
                });
            };
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (k, &i) in shown.iter().enumerate() {
                    if k == 3 && n > 4 {
                        ui.weak("…");
                    }
                    ui.horizontal(|ui| {
                        name(ui, RichText::new(file_name(&batch.paths[i])).weak());
                        ui.weak("→");
                        name(ui, RichText::new(file_name(&news[i])));
                    });
                }
            });
            if let Some(error) = &batch.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(0xff, 0x8a, 0x80), error);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                if ui.add(egui::Button::new(text(tr!("Rename", "Переименовать"))).min_size(egui::vec2(160.0, 34.0))).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() && modal.should_close() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                let (paths, base, start) = (batch.paths.clone(), batch.base.clone(), batch.start);
                match self.rename_batch(ctx, &paths, &base, start) {
                    Ok(()) => self.batch_rename = None,
                    Err(e) => {
                        if let Some(batch) = &mut self.batch_rename {
                            batch.error = Some(e);
                            batch.focus = true;
                        }
                    }
                }
            }
            Some(false) => self.batch_rename = None,
            None => {}
        }
    }
}

/// Select `name` in the text field `id` up to its extension.
fn select_stem(ctx: &egui::Context, id: egui::Id, name: &str) {
    use egui::text::{CCursor, CCursorRange};
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        let stem = name.rfind('.').filter(|&i| i > 0).unwrap_or(name.len());
        let end = name[..stem].chars().count();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
        state.store(ctx, id);
    }
}

/// A cross button, as in a window caption; true when clicked.
/// The update check: the start-up option, a button to check now and the
/// outcome.
fn update_section(ui: &mut Ui, updates: &mut crate::update::Updates) {
    use crate::update::{Status, release_url, version_of};
    let before = updates.enabled;
    ui.checkbox(
        &mut updates.enabled,
        tr!("Check for updates at start-up (once a day)", "Проверять обновления при запуске (раз в сутки)"),
    )
    .on_hover_text(tr!(
        "Asks api.github.com for the latest release; nothing else is sent, and nothing is downloaded",
        "Запрашивает у api.github.com последний выпуск; больше ничего не отправляется и не скачивается"
    ));
    if updates.enabled && !before {
        updates.start_if_due(ui.ctx());
    }
    let checking = updates.status == Status::Checking;
    if ui.add_enabled(!checking, egui::Button::new(tr!("Check now", "Проверить сейчас"))).clicked() {
        updates.start(ui.ctx());
    }
    match &updates.status {
        Status::Unknown => {}
        Status::Checking => {
            ui.horizontal(|ui| {
                // Centred by hand, as the language list above.
                ui.add_space(((ui.available_width() - 110.0) / 2.0).max(0.0));
                ui.add(egui::Spinner::new());
                ui.weak(tr!("Checking…", "Проверка…"));
            });
        }
        Status::UpToDate => {
            ui.weak(tr!("This is the latest version", "Установлена последняя версия"));
        }
        Status::Newer(tag) => {
            let version = version_of(tag);
            let url = release_url(tag);
            // Not `hyperlink_to`: eframe's `links` feature is off.
            if ui
                .link(tr!(format!("Version {version} is available"), format!("Доступна версия {version}")))
                .on_hover_text(&url)
                .clicked()
                && !crate::win::shell_open(&url)
            {
                log::warn!("could not open {url}");
            }
        }
        Status::Failed(e) => {
            ui.weak(tr!("Could not check for updates", "Не удалось проверить обновления")).on_hover_text(e);
        }
    }
}

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

/// A modal with `title`, a cross that closes it (as in the associations
/// dialog) and `content`; true when it is closed (the cross, Esc, Enter,
/// F1 or a click outside). Keys are ignored in the frame the modal opens
/// (`fresh`).
fn info_modal(ctx: &egui::Context, id: &str, title: &str, fresh: bool, content: impl FnOnce(&mut Ui)) -> bool {
    // As wide as the content, measured in the previous frame (egui does not
    // show a new window's first frame), so that the cross sits at the right
    // edge of the content.
    let width_id = egui::Id::new(id).with("width");
    let width = ctx.data(|d| d.get_temp::<f32>(width_id)).unwrap_or(400.0);
    let modal = egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
        ui.set_width(width);
        let close = ui
            .horizontal(|ui| {
                ui.heading(title);
                ui.with_layout(Layout::right_to_left(Align::Center), close_cross).inner
            })
            .inner;
        ui.add_space(6.0);
        let used = ui.scope(|ui| content(ui)).response.rect.width();
        ctx.data_mut(|d| d.insert_temp(width_id, used));
        close
    });
    let close = modal.inner;
    if fresh {
        return close;
    }
    let key = ctx.input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, Key::Enter) || i.consume_key(egui::Modifiers::NONE, Key::F1)
    });
    close || key || modal.should_close()
}
