//! The menu bar and the image's context menu. Items only queue commands in
//! `App::clicked`; `App::run` carries them out after drawing.

use egui::{Button, Ui};

use super::{MENU_BG, panel_frame};
use crate::app::App;
use crate::input::Cmd;

/// What the menu items may act on this frame.
struct Enabled {
    file: bool,
    image: bool,
    /// The image can be turned, cropped and saved (see `App::editable`).
    edit: bool,
    /// Turned or being cropped: Save has something to save.
    edited: bool,
    prev: bool,
    next: bool,
    any: bool,
}

impl App {
    fn enabled(&self) -> Enabled {
        let n = self.files.len();
        Enabled {
            file: self.current.is_some(),
            image: self.shown.is_some(),
            edit: self.editable().is_some() && !self.saving(),
            edited: self.editable().is_some() && !self.saving() && (self.view.changed() || self.crop.is_some()),
            prev: self.index.is_some_and(|i| i > 0),
            next: self.index.is_some_and(|i| i + 1 < n),
            any: n > 0,
        }
    }

    pub(super) fn item(&mut self, ui: &mut Ui, text: String, shortcut: &str, cmd: Cmd, enabled: bool) {
        if ui.add_enabled(enabled, Button::new(text).shortcut_text(shortcut)).clicked() {
            self.clicked.push(cmd);
            ui.close();
        }
    }

    fn check_item(&mut self, ui: &mut Ui, text: String, shortcut: &str, cmd: Cmd, on: bool) {
        if ui.add(Button::new(text).shortcut_text(shortcut).selected(on)).clicked() {
            self.clicked.push(cmd);
            ui.close();
        }
    }

    pub(crate) fn menu_bar(&mut self, root_ui: &mut Ui) {
        egui::Panel::top("menu")
            .frame(panel_frame(MENU_BG, egui::Margin::symmetric(4, 2)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button(tr!("File", "Файл"), |ui| self.file_menu(ui));
                    ui.menu_button(tr!("View", "Вид"), |ui| self.view_menu(ui));
                    ui.menu_button(tr!("Favorites", "Избранное"), |ui| self.favorites_menu(ui));
                    ui.menu_button(tr!("Help", "Справка"), |ui| self.help_menu(ui));
                });
            });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.item(ui, tr!("Open…", "Открыть…").into(), "Ctrl+O", Cmd::Open, true);
        self.item(ui, tr!("Save", "Сохранить").into(), "Ctrl+S", Cmd::Save, e.edited);
        self.item(ui, tr!("Save As…", "Сохранить как…").into(), "Ctrl+Shift+S", Cmd::SaveAs, e.edit);
        self.convert_menu(ui);
        self.editor_items(ui, e.file && self.crop.is_none());
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, e.file);
        ui.separator();
        self.print_item(ui, e.file);
        self.wallpaper_item(ui, e.file);
        ui.separator();
        self.copy_item(ui, e.file);
        self.copy_image_item(ui, e.file);
        let paste = crate::clipboard::can_paste() && self.crop.is_none();
        self.item(ui, tr!("Paste", "Вставить").into(), "Ctrl+V", Cmd::Paste, paste);
        self.rename_item(ui, e.file);
        let undo = !self.undo.is_empty() && self.crop.is_none();
        self.item(ui, self.undo_label().into(), "Ctrl+Z", Cmd::Undo, undo);
        self.delete_item(ui, e.file);
        ui.separator();
        self.item(ui, tr!("File Associations…", "Сопоставление файлов…").into(), "", Cmd::Associations, true);
        ui.separator();
        self.item(ui, tr!("Exit", "Выход").into(), "Ctrl+W", Cmd::Close, true);
    }

    fn view_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.check_item(ui, tr!("Gallery", "Галерея").into(), "G", Cmd::Gallery, self.gallery_open);
        ui.separator();
        self.item(ui, tr!("Previous", "Предыдущее").into(), "Page Up", Cmd::Prev, e.prev);
        self.item(ui, tr!("Next", "Следующее").into(), "Page Down", Cmd::Next, e.next);
        self.item(ui, tr!("First", "Первое").into(), "Home", Cmd::First, e.any);
        self.item(ui, tr!("Last", "Последнее").into(), "End", Cmd::Last, e.any);
        ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
        ui.separator();
        self.zoom_items(ui, &e);
        ui.separator();
        self.rotate_items(ui, &e);
        ui.separator();
        self.animation_items(ui);
        ui.separator();
        self.item(ui, tr!("Full Screen", "Полный экран").into(), "F", Cmd::FullScreen, true);
        self.check_item(ui, tr!("Toolbar", "Панель инструментов").into(), "T", Cmd::ToggleToolbar, self.show_toolbar);
        self.check_item(ui, tr!("Status Bar", "Строка состояния").into(), "B", Cmd::ToggleStatusBar, self.show_status_bar);
        self.check_item(ui, tr!("Information", "Сведения").into(), "I", Cmd::Info, self.show_info);
        ui.menu_button(tr!("Background", "Фон"), |ui| self.background_menu(ui));
        ui.separator();
        self.item(ui, tr!("Refresh", "Обновить").into(), "F5", Cmd::Refresh, e.file);
    }

    /// Convert To: the formats the current image can be converted to, a
    /// copy beside it. Its own format is left out but for WebP, whose copy
    /// is lossless.
    pub(super) fn convert_menu(&mut self, ui: &mut Ui) {
        use crate::edit::Format;
        let own = self.current.as_deref().and_then(Format::of).filter(|&f| f != Format::WebP);
        ui.add_enabled_ui(self.can_convert(), |ui| {
            let text = match self.several() {
                Some(n) => tr!(format!("Convert {n} Files To"), format!("Конвертировать файлы ({n}) в")),
                None => tr!("Convert To", "Конвертировать в").into(),
            };
            ui.menu_button(text, |ui| {
                for format in Format::ALL {
                    self.item(ui, format.name().into(), "", Cmd::ConvertTo(format), own != Some(format));
                }
            })
        });
    }

    fn zoom_items(&mut self, ui: &mut Ui, e: &Enabled) {
        self.item(ui, tr!("Zoom In", "Увеличить").into(), "+", Cmd::ZoomIn, e.image);
        self.item(ui, tr!("Zoom Out", "Уменьшить").into(), "-", Cmd::ZoomOut, e.image);
        self.item(ui, tr!("Actual Size", "Реальный размер").into(), "1", Cmd::Actual, e.image);
        self.item(ui, tr!("Fit Image", "Вписать в окно").into(), "2", Cmd::Fit, e.image);
        self.item(ui, tr!("Fill Window", "Заполнить окно").into(), "3", Cmd::Fill, e.image);
        self.item(ui, tr!("Fill Entire Window", "Заполнить окно целиком").into(), "4", Cmd::Cover, e.image);
        let keep = self.view.keep;
        self.check_item(ui, tr!("Keep Zoom and Position", "Сохранять масштаб и положение").into(), "L", Cmd::KeepZoom, keep);
        ui.menu_button(tr!("Filtering", "Фильтрация"), |ui| {
            for filter in crate::view::Filter::ALL {
                if ui.add(Button::new(filter.label()).selected(self.filter == filter)).clicked() {
                    self.filter = filter;
                    ui.close();
                }
            }
        });
    }

    /// Pause and the frame steps, for the animation on screen.
    fn animation_items(&mut self, ui: &mut Ui) {
        let playing = self.player.is_some();
        let paused = self.player.as_ref().is_some_and(|p| p.paused());
        let text = tr!("Pause Animation", "Приостановить анимацию");
        if ui.add_enabled(playing, Button::new(text).shortcut_text("P").selected(paused)).clicked() {
            self.clicked.push(Cmd::Pause);
            ui.close();
        }
        self.item(ui, tr!("Next Frame", "Следующий кадр").into(), ".", Cmd::NextFrame, playing);
        self.item(ui, tr!("Previous Frame", "Предыдущий кадр").into(), ",", Cmd::PrevFrame, playing);
    }

    fn rotate_items(&mut self, ui: &mut Ui, e: &Enabled) {
        self.turn_items(ui, e);
        self.flip_items(ui, e);
        self.crop_item(ui, e);
    }

    fn turn_items(&mut self, ui: &mut Ui, e: &Enabled) {
        self.item(ui, tr!("Rotate Left", "Повернуть влево").into(), "[", Cmd::RotateLeft, e.image);
        self.item(ui, tr!("Rotate Right", "Повернуть вправо").into(), "]", Cmd::RotateRight, e.image);
    }

    fn flip_items(&mut self, ui: &mut Ui, e: &Enabled) {
        self.item(ui, tr!("Flip Horizontally", "Отразить по горизонтали").into(), "H", Cmd::FlipHorizontal, e.image);
        self.item(ui, tr!("Flip Vertically", "Отразить по вертикали").into(), "V", Cmd::FlipVertical, e.image);
    }

    fn crop_item(&mut self, ui: &mut Ui, e: &Enabled) {
        let cropping = self.crop.is_some();
        if ui.add_enabled(e.edit, Button::new(tr!("Crop", "Обрезать")).shortcut_text("C").selected(cropping)).clicked() {
            self.clicked.push(Cmd::Crop);
            ui.close();
        }
    }

    /// The order of the folder, or of the favourites, which have one of
    /// their own: the key, and the direction.
    pub(super) fn sort_menu(&mut self, ui: &mut Ui) {
        use crate::folder::SortKey;
        let order = self.order();
        let favorites = self.in_favorites();
        for key in SortKey::ALL {
            let name = match key {
                SortKey::Name => tr!("By Name", "По имени"),
                SortKey::Modified => tr!("By Date Modified", "По дате изменения"),
                SortKey::Taken => tr!("By Date Taken", "По дате съёмки"),
                SortKey::Size => tr!("By Size", "По размеру"),
                SortKey::Added if favorites => tr!("By Date Added", "По дате добавления"),
                SortKey::Added => continue,
            };
            self.check_item(ui, name.into(), "", Cmd::SortBy(key), order.key == key);
        }
        ui.separator();
        self.check_item(ui, tr!("Descending", "По убыванию").into(), "", Cmd::SortDescending, order.descending);
    }

    /// Presets, and a picker for any other colour; the picker is drawn in
    /// the menu itself (not as a popup of its own) so that the menu stays
    /// open while the colour is being chosen.
    fn background_menu(&mut self, ui: &mut Ui) {
        for (color, name) in crate::app::background_presets() {
            if ui.add(Button::new(name).selected(self.background == color)).clicked() {
                self.background = color;
                ui.close();
            }
        }
        ui.separator();
        let text = tr!("Checkerboard Behind Transparency", "Шахматка под прозрачными областями");
        if ui.add(Button::new(text).selected(self.checker)).clicked() {
            self.checker = !self.checker;
            ui.close();
        }
        ui.separator();
        ui.label(tr!("Other colour:", "Другой цвет:"));
        egui::color_picker::color_picker_color32(ui, &mut self.background, egui::color_picker::Alpha::Opaque);
    }

    fn favorites_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.favorite_item(ui, e.file);
        self.item(ui, tr!("Show Favorites", "Показать избранное").into(), "", Cmd::Favorites, true);
        self.go_to_folder_item(ui);
        ui.separator();
        self.favorites_items(ui);
        ui.separator();
        self.clear_favorites_item(ui);
    }

    /// Clear the favourites, after a confirmation.
    pub(super) fn clear_favorites_item(&mut self, ui: &mut Ui) {
        let any = self.favorites.len() > 0;
        self.item(ui, tr!("Clear Favorites…", "Очистить избранное…").into(), "", Cmd::ClearFavorites, any);
    }

    /// Add the current image, or those chosen, to the favourites, or
    /// remove them when they all are.
    pub(super) fn favorite_item(&mut self, ui: &mut Ui, enabled: bool) {
        let enabled = enabled && !self.in_archive();
        let targets = self.targets();
        let text = if !targets.is_empty() && targets.iter().all(|p| self.favorites.contains(p)) {
            tr!("Remove from Favorites", "Убрать из избранного")
        } else {
            tr!("Add to Favorites", "Добавить в избранное")
        };
        self.item(ui, text.into(), "S", Cmd::Favorite, enabled);
    }

    /// Open in the editor chosen last (Ctrl+E), and Edit With: the programs
    /// Windows offers for the type, the one in use ticked, and any other;
    /// the one picked becomes the editor.
    pub(super) fn editor_items(&mut self, ui: &mut Ui, enabled: bool) {
        let enabled = enabled && !self.in_archive();
        self.open_in_editor_item(ui, enabled);
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button(tr!("Edit With", "Редактировать в"), |ui| {
                // Read from Windows only while the menu is open.
                let choices = self.editor_choices();
                if choices.is_empty() {
                    ui.weak(tr!("Windows offers no programs", "Windows не предлагает программ"));
                }
                for (k, editor) in choices.iter().enumerate() {
                    let on = self.editor.as_ref().is_some_and(|e| e.id.eq_ignore_ascii_case(&editor.id));
                    if ui.add(Button::new(&editor.name).selected(on)).clicked() {
                        self.clicked.push(Cmd::EditWith(k));
                        ui.close();
                    }
                }
                self.menu_editors = choices;
                ui.separator();
                self.item(ui, tr!("Other Program…", "Другая программа…").into(), "", Cmd::EditWithOther, true);
            })
        });
    }

    /// Open in the editor chosen last (Ctrl+E).
    fn open_in_editor_item(&mut self, ui: &mut Ui, enabled: bool) {
        let enabled = enabled && !self.in_archive();
        let text = match &self.editor {
            Some(e) => tr!(format!("Open in {}", e.name), format!("Открыть в {}", e.name)),
            None => tr!("Open in Editor", "Открыть в редакторе").into(),
        };
        self.item(ui, text, "Ctrl+E", Cmd::Edit, enabled);
    }

    /// How many images the file commands act on, when more than one.
    fn several(&self) -> Option<usize> {
        Some(self.targets().len()).filter(|&n| n > 1)
    }

    /// An item with a tooltip saying what it does.
    fn item_explained(&mut self, ui: &mut Ui, text: String, shortcut: &str, cmd: Cmd, enabled: bool, tip: &str) {
        let button = ui.add_enabled(enabled, Button::new(text).shortcut_text(shortcut));
        if button.on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
            self.clicked.push(cmd);
            ui.close();
        }
    }

    /// The file itself, as Explorer copies it.
    pub(super) fn copy_item(&mut self, ui: &mut Ui, enabled: bool) {
        let text = match self.several() {
            Some(n) => tr!(format!("Copy {n} Files"), format!("Копировать файлы ({n})")),
            None => tr!("Copy File", "Копировать файл").into(),
        };
        let tip = tr!(
            "The file, to paste into Explorer, a messenger or an e-mail",
            "Файл — для вставки в Проводник, мессенджер или письмо"
        );
        self.item_explained(ui, text, "Ctrl+C", Cmd::Copy, enabled && !self.in_archive(), tip);
    }

    /// The pixels, as shown.
    pub(super) fn copy_image_item(&mut self, ui: &mut Ui, enabled: bool) {
        let tip = tr!(
            "The picture as shown (rotated, cropped), to paste into an editor or a document",
            "Картинка в показанном виде (с поворотом и обрезкой) — для вставки в редактор или документ"
        );
        self.item_explained(ui, tr!("Copy Image", "Копировать картинку").into(), "Ctrl+Shift+C", Cmd::CopyImage, enabled, tip);
    }

    pub(super) fn print_item(&mut self, ui: &mut Ui, enabled: bool) {
        let text = match self.several() {
            Some(n) => tr!(format!("Print {n} Images…"), format!("Печать изображений ({n})…")),
            None => tr!("Print…", "Печать…").into(),
        };
        self.item(ui, text, "Ctrl+P", Cmd::Print, enabled);
    }

    /// The current image only, even with several chosen.
    pub(super) fn wallpaper_item(&mut self, ui: &mut Ui, enabled: bool) {
        let text = tr!("Set as Desktop Background", "Сделать фоном рабочего стола");
        self.item(ui, text.into(), "", Cmd::Wallpaper, enabled && self.several().is_none());
    }

    pub(super) fn rename_item(&mut self, ui: &mut Ui, enabled: bool) {
        let text = match self.several() {
            Some(n) => tr!(format!("Rename {n} Files…"), format!("Переименовать файлы ({n})…")),
            None => tr!("Rename…", "Переименовать…").into(),
        };
        self.item(ui, text, "F2", Cmd::Rename, enabled && !self.in_archive());
    }

    pub(super) fn delete_item(&mut self, ui: &mut Ui, enabled: bool) {
        let text = match self.several() {
            Some(n) => tr!(format!("Delete {n} Files…"), format!("Удалить файлы ({n})…")),
            None => tr!("Delete…", "Удалить…").into(),
        };
        self.item(ui, text, "Delete", Cmd::Delete, enabled && !self.in_archive());
    }

    /// The current image's folder, from the favourites or the sub-folders.
    pub(super) fn go_to_folder_item(&mut self, ui: &mut Ui) {
        let enabled = self.can_go_to_folder();
        self.item(ui, tr!("Go to Folder", "Перейти к папке").into(), "", Cmd::GoToFolder, enabled);
    }

    /// What can be done with all the favourites.
    pub(super) fn favorites_items(&mut self, ui: &mut Ui) {
        let any = self.favorites.len() > 0;
        self.item(ui, tr!("Copy All", "Копировать все").into(), "", Cmd::CopyFavorites, any);
        self.item(ui, tr!("Copy All to Folder…", "Копировать все в папку…").into(), "", Cmd::CopyFavoritesTo, any);
    }

    fn help_menu(&mut self, ui: &mut Ui) {
        self.item(ui, tr!("Keyboard Shortcuts", "Сочетания клавиш").into(), "F1", Cmd::Shortcuts, true);
        self.item(ui, tr!("About qview", "О программе").into(), "", Cmd::About, true);
    }

    /// Right click on the image: what is done with the mouse; the zoom,
    /// the flips and the rarer commands in submenus, browsing left to the
    /// wheel, the keys and the View menu.
    pub(crate) fn context_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.item(ui, tr!("Gallery", "Галерея").into(), "G", Cmd::Gallery, true);
        self.item(ui, tr!("Full Screen", "Полный экран").into(), "F", Cmd::FullScreen, true);
        ui.separator();
        ui.menu_button(tr!("Zoom", "Масштаб"), |ui| self.zoom_items(ui, &e));
        self.turn_items(ui, &e);
        ui.menu_button(tr!("Flip", "Отразить"), |ui| self.flip_items(ui, &e));
        self.crop_item(ui, &e);
        if e.edited {
            self.item(ui, tr!("Save", "Сохранить").into(), "Ctrl+S", Cmd::Save, true);
        }
        if self.player.is_some() {
            ui.separator();
            self.animation_items(ui);
        }
        ui.separator();
        self.favorite_item(ui, e.file);
        if self.can_go_to_folder() {
            self.go_to_folder_item(ui);
        }
        self.file_items(ui, e.file);
        ui.separator();
        self.delete_item(ui, e.file);
    }

    /// What the context menus of the image and of a thumbnail do with the
    /// file, the rarer commands under More (the File menu has them all).
    pub(super) fn file_items(&mut self, ui: &mut Ui, enabled: bool) {
        self.copy_item(ui, enabled);
        self.copy_image_item(ui, enabled);
        self.rename_item(ui, enabled);
        self.open_in_editor_item(ui, enabled);
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, enabled);
        ui.menu_button(tr!("More", "Ещё"), |ui| {
            self.convert_menu(ui);
            self.print_item(ui, enabled);
            self.wallpaper_item(ui, enabled);
        });
    }
}
