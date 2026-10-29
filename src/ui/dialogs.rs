//! Modal dialogs: the delete confirmation, renaming, the list of shortcuts
//! and About (with the Licenses window, `licenses`).

use egui::{Align, Key, Layout, RichText, Ui};

use crate::app::{App, Dialog, file_name};
use crate::i18n::LangChoice;

/// Width of the language list in About, enough for its longest entry.
const LANG_WIDTH: f32 = 220.0;

/// The key bindings as the Shortcuts dialog lists them, in sections.
fn shortcuts(esc_closes: bool) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
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
                    "N",
                    tr!(
                        "The navigator over a zoomed image: where the window is on it; a click or a drag moves there",
                        "Навигатор над увеличенным изображением: где на нём окно; щелчок или перетаскивание переходит туда"
                    ),
                ),
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
                ("Ctrl+Alt+S", tr!("Save as another file, smaller or at another JPEG quality", "Сохранить в другой файл, уменьшив или с другим качеством JPEG")),
                ("Ctrl+E", tr!("Open in the editor chosen last", "Открыть в последнем выбранном редакторе")),
            ],
        ),
        (
            tr!("Marks", "Пометки"),
            vec![
                ("A", tr!("Mark up the image: frames, lines, arrows, drawing, pixelation, text", "Пометки на изображении: рамки, линии, стрелки, рисование, пикселизация, текст")),
                (
                    "D  L  A  R  E  P  T",
                    tr!("Pen / line / arrow / frame / ellipse / pixelate / text", "Карандаш / линия / стрелка / рамка / овал / пикселизация / текст"),
                ),
                ("1 – 6", tr!("Colour (any one from the picker in the bar)", "Цвет (любой — пикером на панели)")),
                ("[  ]", tr!("Smaller / larger: line, text or blocks (or the slider in the bar)", "Меньше / больше: линия, текст или блоки (или ползунок на панели)")),
                ("Shift", tr!("While drawing: a square, a circle, a line at 45°", "При рисовании: квадрат, круг, линия под 45°")),
                ("Delete  Ctrl+Z", tr!("Delete the chosen mark / undo the last change", "Удалить выбранную пометку / отменить последнее изменение")),
                ("O", tr!("The list of the marks beside the image", "Список пометок рядом с изображением")),
                ("Enter  Esc", tr!("Save / cancel (twice when there are marks)", "Сохранить / отменить (дважды, если есть пометки)")),
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
                ("Alt+N", tr!("Move the file into a new folder", "Переместить файл в новую папку")),
                ("Shift+Alt+N", tr!("Copy it into a new folder", "Копировать файл в новую папку")),
                ("Ctrl+Shift+N", tr!("Make a folder in the folder shown (gallery)", "Создать папку в показанной папке (галерея)")),
                ("Ctrl+Z", tr!("Undo the last rename, move, copy or save", "Отменить последнее переименование, перемещение, копирование или сохранение")),
                ("Ctrl+C", tr!("Copy the file", "Копировать файл")),
                ("Ctrl+Shift+C", tr!("Copy the image as shown", "Копировать картинку, как она показана")),
                (
                    tr!("Ctrl+V  Shift+Insert", "Ctrl+V  Shift+Insert"),
                    tr!("Open the clipboard's image, file or path", "Открыть изображение, файл или путь из буфера обмена"),
                ),
                ("Ctrl+P", tr!("Print the image as shown, or those chosen", "Печать изображения, как оно показано, или выбранных")),
                (
                    "Ctrl+Shift+P",
                    tr!("Save the images listed, or those chosen, as one PDF", "Сохранить показанные или выбранные изображения в один PDF"),
                ),
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
                ("M  T  B", tr!("Show or hide the menu bar / toolbar / status bar", "Строка меню / панель инструментов / строка состояния")),
                ("I", tr!("Show or hide the information panel (EXIF)", "Панель сведений (EXIF)")),
                ("K", tr!("This list", "Этот список")),
                ("F1", tr!("About the program", "О программе")),
                (
                    "Esc",
                    if esc_closes {
                        tr!("Leave full screen; then close", "Выйти из полного экрана; затем закрыть")
                    } else {
                        tr!(
                            "Leave full screen; then the gallery, from it close",
                            "Выйти из полного экрана; затем галерея, из неё закрыть"
                        )
                    },
                ),
                ("Ctrl+W  Alt+F4", tr!("Close", "Закрыть")),
            ],
        ),
    ]
}

impl App {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.save_options_dialog(ctx, frame);
        self.confirm_delete_dialog(ctx);
        self.confirm_clear_favorites_dialog(ctx);
        self.confirm_unpin_all_dialog(ctx);
        self.confirm_edit_dialog(ctx);
        self.rename_dialog(ctx);
        self.new_folder_dialog(ctx);
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
                        for (i, (title, keys)) in shortcuts(self.esc_closes).into_iter().enumerate() {
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
    /// Cargo.toml when they are set, the licences, and where the settings
    /// are kept. True when closed.
    fn about(&mut self, ctx: &egui::Context, fresh: bool) -> bool {
        if let Some(view) = &mut self.licenses {
            if view.show(ctx) {
                self.licenses = None;
            }
            return false;
        }
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
                // The texts of all the licences, embedded (`licenses`).
                if ui
                    .link(tr!("Licenses…", "Лицензии…"))
                    .on_hover_text(tr!(
                        "The licences of qview, of libheif and libde265, of UnRAR and of the libraries and fonts compiled in",
                        "Лицензии qview, libheif и libde265, UnRAR и встроенных библиотек и шрифтов"
                    ))
                    .clicked()
                {
                    self.licenses = Some(Default::default());
                }
                let place = if crate::portable_dir().is_some() {
                    ui.weak(tr!("Settings: beside the program (portable)", "Настройки: рядом с программой (портативная версия)"))
                } else {
                    ui.weak(tr!("Settings: in the user profile", "Настройки: в профиле пользователя"))
                };
                match crate::settings_dir() {
                    Some(dir) => place.on_hover_text(dir.join(crate::SETTINGS_FILE).display().to_string()),
                    None => place.on_hover_text(tr!("Settings are not saved", "Настройки не сохраняются")),
                };
                ui.add_space(4.0);
                ui.weak(tr!("Keyboard shortcuts: K", "Сочетания клавиш: K"));
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
        key_closes(ctx, &[Key::Enter, Key::F1]) || modal.should_close()
    }

    fn confirm_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.confirm_delete.clone() else { return };
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            let n = paths.len();
            let folders = self.delete_folders.min(n);
            if folders > 1 && folders == n {
                ui.heading(tr!("Delete Folders", "Удаление папок"));
                ui.add_space(6.0);
                ui.label(tr!(
                    format!("Move {n} folders with everything in them to the Recycle Bin?"),
                    format!("Переместить папки ({n}) со всем содержимым в корзину?")
                ));
            } else if folders > 0 && folders < n {
                let files = n - folders;
                ui.heading(tr!("Delete Folders and Files", "Удаление папок и файлов"));
                ui.add_space(6.0);
                ui.label(tr!(
                    format!("Move {folders} folders with everything in them and {files} files to the Recycle Bin?"),
                    format!("Переместить в корзину папки ({folders}) со всем содержимым и файлы ({files})?")
                ));
            } else if n == 1 && paths[0].is_dir() {
                ui.heading(tr!("Delete Folder", "Удаление папки"));
                ui.add_space(6.0);
                let name = file_name(&paths[0]);
                ui.label(tr!(
                    format!("Move the folder \"{name}\" with everything in it to the Recycle Bin?"),
                    format!("Переместить папку «{name}» со всем содержимым в корзину?")
                ));
            } else if n == 1 {
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
            // Files only: folders are always asked about.
            if folders == 0 {
                ui.add_space(6.0);
                let tip = tr!("File → Confirm Deletion asks again", "«Файл» → «Подтверждать удаление» включает вопрос снова");
                ui.checkbox(&mut self.dont_ask_delete, tr!("Don't ask again", "Больше не спрашивать")).on_hover_text(tip);
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
                if std::mem::take(&mut self.dont_ask_delete) {
                    self.confirm_deletion = false;
                }
                self.delete(ctx, paths);
            }
            Some(false) => {
                self.confirm_delete = None;
                self.dont_ask_delete = false;
            }
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
                // A folder's name is selected whole.
                select_stem(ctx, id, &rename.name, rename.path.is_dir());
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
    /// The name of a folder to make: in the folder shown, or beside the
    /// image to move it, or the chosen images, into (Alt+N), or copy them
    /// into (Shift+Alt+N); Enter does it, Esc cancels.
    fn new_folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(new) = self.new_folder.as_mut() else { return };
        let id = egui::Id::new("new_folder_name");
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("new_folder")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(tr!("New Folder", "Новая папка"));
            ui.add_space(4.0);
            ui.label(tr!(format!("In {}", new.parent.display()), format!("В папке {}", new.parent.display())));
            ui.add_space(8.0);
            let edit = ui.add(egui::TextEdit::singleline(&mut new.name).id(id).desired_width(f32::INFINITY));
            if std::mem::take(&mut new.focus) {
                edit.request_focus();
            }
            if std::mem::take(&mut new.select) {
                select_stem(ctx, id, &new.name, true);
            }
            if edit.changed() {
                new.error = None;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                decision = Some(true);
            }
            if let Some(error) = &new.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(0xff, 0x8a, 0x80), error);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                let go = match new.transfer {
                    None => tr!("Create", "Создать"),
                    Some(false) => tr!("Move", "Переместить"),
                    Some(true) => tr!("Copy", "Копировать"),
                };
                if ui.add(egui::Button::new(text(go)).min_size(egui::vec2(160.0, 34.0))).clicked() {
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
                let (parent, name, transfer) = (new.parent.clone(), new.name.clone(), new.transfer);
                match self.make_folder(ctx, &parent, &name, transfer) {
                    Ok(()) => self.new_folder = None,
                    Err(e) => {
                        if let Some(new) = &mut self.new_folder {
                            new.error = Some(e);
                            new.focus = true;
                        }
                    }
                }
            }
            Some(false) => self.new_folder = None,
            None => {}
        }
    }

    /// Several files renamed to a name and a number each, in the order of
    /// the grid; the first and last new names shown. Enter renames, Esc
    /// cancels.
    fn batch_rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(batch) = self.batch_rename.as_mut() else { return };
        batch.poll_dates();
        if batch.dates.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let id = egui::Id::new("batch_rename_name");
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("batch_rename")).show(ctx, |ui| {
            ui.set_width(580.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            let n = batch.paths.len();
            ui.heading(match batch.dirs {
                0 => tr!(format!("Rename {n} Files"), format!("Переименование файлов ({n})")),
                d if d == n => tr!(format!("Rename {n} Folders"), format!("Переименование папок ({n})")),
                _ => tr!(format!("Rename {n} Folders and Files"), format!("Переименование папок и файлов ({n})")),
            });
            ui.add_space(8.0);
            let mut changed = false;
            // The template places the number itself: nothing to add.
            let placed = crate::rename::places_number(&batch.base);
            // Its dates are not read yet: the names are not known.
            let waiting = crate::rename::uses_dates(&batch.base) && batch.dates.is_none();
            // Nothing would be left of the names.
            let blank = crate::rename::blank(&batch.base, batch.number || placed);
            let ready = !waiting && !blank;
            egui::Grid::new("batch_rename_fields").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr!("Name:", "Имя:"));
                ui.horizontal(|ui| {
                    // The field, then an "i" button with the placeholders.
                    let side = ui.spacing().interact_size.y;
                    let width = ui.available_width() - side - ui.spacing().item_spacing.x;
                    let edit = ui.add(egui::TextEdit::singleline(&mut batch.base).id(id).desired_width(width));
                    if std::mem::take(&mut batch.focus) {
                        edit.request_focus();
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && ready {
                        decision = Some(true);
                    }
                    changed |= edit.changed();
                    let info = egui::Button::new(RichText::new("i").strong()).min_size(egui::vec2(side, side));
                    ui.add(info).on_hover_text(tr!(
                        "{n} — the number ({n:4} — with four digits)\n{name} — the old name\n{taken} — the date taken, 2024-05-16 10-30-15 (the date modified when the file has none)\n{modified} — the date modified\n{taken:YYYY-MM-DD} — a pattern of YYYY, MM, DD, HH, mm, ss\n\nWithout {n} the number is added after _, or, with \"Add a number\" off, equal names get (2), (3)…",
                        "{n} — номер ({n:4} — четырьмя цифрами)\n{name} — старое имя\n{taken} — дата съёмки, 2024-05-16 10-30-15 (дата изменения, если в файле её нет)\n{modified} — дата изменения\n{taken:YYYY-MM-DD} — образец из YYYY, MM, DD, HH, mm, ss\n\nБез {n} номер добавляется после _, а со снятым флажком «Добавлять номер» совпавшие имена получают (2), (3)…"
                    ));
                });
                ui.end_row();
                ui.label(tr!("Start at:", "Начать с:"));
                ui.horizontal(|ui| {
                    let mut number = batch.number || placed;
                    changed |= ui.add_enabled(number, egui::DragValue::new(&mut batch.start).range(0..=999_999)).changed();
                    ui.add_space(10.0);
                    changed |= ui.add_enabled(!placed, egui::Checkbox::new(&mut number, tr!("Add a number", "Добавлять номер"))).changed();
                    if !placed {
                        batch.number = number;
                    }
                });
                ui.end_row();
            });
            if changed {
                batch.error = None;
            }
            ui.add_space(8.0);
            // The first names and the last: what the numbering looks like.
            let news = crate::rename::templated(&batch.paths, batch.dirs, &batch.base, batch.start, batch.number, batch.dates.as_deref().unwrap_or(&[]));
            // Every old and new name, the two sharing the width with the
            // arrow between, a grip at the right; a row dragged (egui's
            // drag and drop: the row follows the pointer) goes where the
            // line shows, a folder among the folders, a file among the
            // files.
            let height = ui.spacing().interact_size.y;
            let grip = height;
            let half = ((ui.available_width() - 30.0 - grip) / 2.0).max(60.0);
            let name = |ui: &mut Ui, text: RichText| {
                ui.allocate_ui_with_layout(egui::vec2(half, height), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(half);
                    ui.add(egui::Label::new(text).truncate());
                });
            };
            let dragged = egui::DragAndDrop::payload::<usize>(ctx).map(|i| *i);
            let pointer = ctx.pointer_interact_pos();
            // The rows drawn: (index, rect), for where the drag would land.
            let mut rows_drawn: Vec<(usize, egui::Rect)> = Vec::new();
            egui::ScrollArea::vertical()
                .max_height(8.0 * (height + ui.spacing().item_spacing.y))
                .auto_shrink([false, true])
                .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..Default::default() })
                .show_rows(ui, height, n, |ui, rows| {
                    for i in rows {
                        let id = egui::Id::new("batch_rename_row").with(i);
                        let row = ui.dnd_drag_source(id, i, |ui| {
                            ui.horizontal(|ui| {
                                name(ui, RichText::new(file_name(&batch.paths[i])).weak());
                                ui.weak("→");
                                name(ui, RichText::new(file_name(&news[i])));
                                // A grip of six dots (no glyph for one in egui's fonts).
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(grip, height), egui::Sense::hover());
                                let colour = ui.visuals().weak_text_color();
                                let c = rect.center();
                                for dx in [-2.5, 2.5] {
                                    for dy in [-5.0, 0.0, 5.0] {
                                        ui.painter().circle_filled(c + egui::vec2(dx, dy), 1.2, colour);
                                    }
                                }
                            });
                        });
                        rows_drawn.push((i, row.response.rect));
                    }
                    // Where the dragged row would go: before the first row
                    // whose middle is below the pointer, else after the last.
                    if let (Some(from), Some(p)) = (dragged, pointer)
                        && let Some(first) = rows_drawn.first()
                        && let Some(last) = rows_drawn.last()
                        && p.y >= first.1.top() - height
                        && p.y <= last.1.bottom() + height
                    {
                        let (to, y) = rows_drawn
                            .iter()
                            .find(|(_, r)| r.center().y > p.y)
                            .map_or((last.0 + 1, last.1.bottom()), |(i, r)| (*i, r.top()));
                        let x = first.1.x_range();
                        ui.painter().hline(x, y, ui.visuals().selection.stroke);
                        if ui.input(|i| i.pointer.any_released()) {
                            egui::DragAndDrop::clear_payload(ctx);
                            batch.move_item(from, to);
                        }
                    }
                });
            if let Some(error) = &batch.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(0xff, 0x8a, 0x80), error);
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                let rename = egui::Button::new(text(tr!("Rename", "Переименовать"))).min_size(egui::vec2(160.0, 34.0));
                if ui.add_enabled(ready, rename).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
                if blank {
                    ui.weak(tr!("Type a name or add a number", "Введите имя или добавьте номер"));
                } else if waiting {
                    ui.weak(tr!("Reading the dates…", "Чтение дат…"));
                }
            });
        });
        if decision.is_none() && modal.should_close() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                let Some(mut batch) = self.batch_rename.take() else { return };
                match self.rename_batch(ctx, &batch) {
                    Ok(()) => {}
                    Err(e) => {
                        batch.error = Some(e);
                        batch.focus = true;
                        self.batch_rename = Some(batch);
                    }
                }
            }
            Some(false) => self.batch_rename = None,
            None => {}
        }
    }
}

/// Select `name` in the text field `id` up to its extension.
fn select_stem(ctx: &egui::Context, id: egui::Id, name: &str, whole: bool) {
    use egui::text::{CCursor, CCursorRange};
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        let stem = name.rfind('.').filter(|&i| i > 0 && !whole).unwrap_or(name.len());
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

/// A cross that closes a dialog; true when clicked.
pub(super) fn close_cross(ui: &mut Ui) -> bool {
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

/// A modal with `title`, a cross that closes it (as in About) and `content`; true when it is closed (the cross, Esc, Enter,
/// K, which opens the only one, the Shortcuts list, or a click outside).
/// Keys are ignored in the frame the modal opens (`fresh`).
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
    let key = key_closes(ctx, &[Key::Enter, Key::K]);
    close || key || modal.should_close()
}

/// A press of one of `keys` without modifiers, taken out of the frame's
/// events with its repeats: a repeat does not count, or holding the key
/// that opened a window closed it half a second later.
pub(super) fn key_closes(ctx: &egui::Context, keys: &[Key]) -> bool {
    ctx.input_mut(|i| {
        let mut pressed = false;
        i.events.retain(|e| match e {
            egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if keys.contains(key) && modifiers.is_none() => {
                pressed |= !repeat;
                false
            }
            _ => true,
        });
        pressed
    })
}
