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
                    ui.menu_button(tr!("Help", "Справка"), |ui| self.help_menu(ui));
                });
            });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.item(ui, tr!("Open…", "Открыть…").into(), "Ctrl+O", Cmd::Open, true);
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, e.file);
        ui.separator();
        self.item(ui, tr!("Copy", "Копировать").into(), "Ctrl+C", Cmd::Copy, e.file);
        self.item(ui, tr!("Delete…", "Удалить…").into(), "Del", Cmd::Delete, e.file);
        ui.separator();
        self.item(ui, tr!("File Associations…", "Сопоставление файлов…").into(), "", Cmd::Associations, true);
        ui.separator();
        self.item(ui, tr!("Exit", "Выход").into(), "Esc", Cmd::Close, true);
    }

    fn view_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.check_item(ui, tr!("Gallery", "Галерея").into(), "G", Cmd::Gallery, self.gallery_open);
        ui.separator();
        self.item(ui, tr!("Previous", "Предыдущее").into(), "Page Up", Cmd::Prev, e.prev);
        self.item(ui, tr!("Next", "Следующее").into(), "Page Down", Cmd::Next, e.next);
        self.item(ui, tr!("First", "Первое").into(), "Home", Cmd::First, e.any);
        self.item(ui, tr!("Last", "Последнее").into(), "End", Cmd::Last, e.any);
        ui.separator();
        self.zoom_items(ui, &e);
        ui.separator();
        self.rotate_items(ui, &e);
        ui.separator();
        self.item(ui, tr!("Full Screen", "Полный экран").into(), "F", Cmd::FullScreen, true);
        self.check_item(ui, tr!("Toolbar", "Панель инструментов").into(), "T", Cmd::ToggleToolbar, self.show_toolbar);
        self.check_item(ui, tr!("Status Bar", "Строка состояния").into(), "B", Cmd::ToggleStatusBar, self.show_status_bar);
        ui.menu_button(tr!("Background", "Фон"), |ui| self.background_menu(ui));
        ui.separator();
        self.item(ui, tr!("Refresh", "Обновить").into(), "F5", Cmd::Refresh, e.file);
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
    }

    fn rotate_items(&mut self, ui: &mut Ui, e: &Enabled) {
        self.item(ui, tr!("Rotate Left", "Повернуть влево").into(), "[", Cmd::RotateLeft, e.image);
        self.item(ui, tr!("Rotate Right", "Повернуть вправо").into(), "]", Cmd::RotateRight, e.image);
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
        ui.label(tr!("Other colour:", "Другой цвет:"));
        egui::color_picker::color_picker_color32(ui, &mut self.background, egui::color_picker::Alpha::Opaque);
    }

    fn help_menu(&mut self, ui: &mut Ui) {
        self.item(ui, tr!("Keyboard Shortcuts", "Сочетания клавиш").into(), "F1", Cmd::Shortcuts, true);
        self.item(ui, tr!("About qview", "О программе").into(), "", Cmd::About, true);
    }

    /// Right click on the image.
    pub(crate) fn context_menu(&mut self, ui: &mut Ui) {
        let e = self.enabled();
        self.item(ui, tr!("Gallery", "Галерея").into(), "G", Cmd::Gallery, true);
        ui.separator();
        self.item(ui, tr!("Previous", "Предыдущее").into(), "Page Up", Cmd::Prev, e.prev);
        self.item(ui, tr!("Next", "Следующее").into(), "Page Down", Cmd::Next, e.next);
        ui.separator();
        self.zoom_items(ui, &e);
        ui.separator();
        self.rotate_items(ui, &e);
        ui.separator();
        self.item(ui, tr!("Full Screen", "Полный экран").into(), "F", Cmd::FullScreen, true);
        ui.separator();
        self.item(ui, tr!("Copy", "Копировать").into(), "Ctrl+C", Cmd::Copy, e.file);
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, e.file);
        ui.separator();
        self.item(ui, tr!("Delete…", "Удалить…").into(), "Del", Cmd::Delete, e.file);
    }
}
