//! The information panel (I) at the right: the current image's file, its
//! image, and what its metadata says (see `info`), in sections; the values
//! can be selected and copied.

use egui::{Margin, RichText, Ui};

use super::{TEXT, TEXT_WEAK, panel_frame};
use crate::app::App;
use crate::format;

const INFO_BG: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x2b, 0x2b);
const HEADING: egui::Color32 = egui::Color32::from_rgb(0xb8, 0xb8, 0xb8);

/// A titled group of (label, value) rows.
struct Section {
    title: &'static str,
    rows: Vec<(&'static str, String)>,
}

impl App {
    pub(crate) fn info_panel(&mut self, root_ui: &mut Ui) {
        let sections = self.info_sections();
        let reading = self.info.as_ref().is_some_and(|i| i.info.is_none());
        let gps = self.info.as_ref().and_then(|i| i.info.as_ref()).and_then(|i| i.exif.gps);
        let panel = egui::Panel::right("info")
            .resizable(true)
            .default_size(self.info_width)
            .size_range(200.0..=600.0)
            .frame(panel_frame(INFO_BG, Margin::symmetric(10, 8)))
            .show(root_ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if sections.is_empty() {
                        ui.label(RichText::new(tr!("No image", "Нет изображения")).color(TEXT_WEAK));
                    }
                    for section in &sections {
                        draw_section(ui, section);
                    }
                    if reading {
                        ui.label(RichText::new(tr!("Reading…", "Чтение…")).color(TEXT_WEAK));
                    }
                    if let Some(gps) = gps {
                        ui.add_space(4.0);
                        if ui.button(tr!("Show on Map", "Показать на карте")).clicked() {
                            let (lat, lon) = (gps.latitude, gps.longitude);
                            let url = format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=16/{lat:.6}/{lon:.6}");
                            crate::win::shell_open(url);
                        }
                    }
                });
            });
        self.info_width = panel.response.rect.width().round();
    }

    /// What the panel shows of the current image.
    fn info_sections(&self) -> Vec<Section> {
        let Some(path) = &self.current else { return Vec::new() };
        let info = self.info.as_ref().filter(|i| i.path == *path).and_then(|i| i.info.clone());
        // What the decoder found, when the image is on screen.
        let meta = self.shown.as_ref().filter(|(p, _)| p == path).map(|(_, picture)| &picture.meta);
        let mut file = Section { title: tr!("File", "Файл"), rows: Vec::new() };
        file.rows.push((tr!("Name", "Имя"), crate::app::file_name(path)));
        if let Some(parent) = path.parent() {
            file.rows.push((tr!("Folder", "Папка"), parent.display().to_string()));
        }
        let size = info.as_ref().map(|i| i.file_size).or(meta.map(|m| m.file_size)).filter(|&s| s > 0);
        if let Some(size) = size {
            file.rows.push((tr!("Size", "Размер"), format::exact_size(size)));
        }
        let date = |t: u64| crate::win::local_date_time(t).filter(|_| t != 0);
        if let Some(d) = info.as_ref().and_then(|i| date(i.modified)) {
            file.rows.push((tr!("Modified", "Изменён"), d));
        }
        if let Some(d) = info.as_ref().and_then(|i| date(i.created)) {
            file.rows.push((tr!("Created", "Создан"), d));
        }
        let mut sections = vec![file];
        let Some(info) = info else { return sections };
        let x = &info.exif;

        let mut image = Section { title: tr!("Image", "Изображение"), rows: Vec::new() };
        if let Some((w, h)) = meta.map(|m| (m.width, m.height)).or(info.size) {
            image.rows.push((tr!("Dimensions", "Размеры"), format::dimensions(w, h)));
        }
        if let Some(m) = meta {
            let animated = if m.animated { tr!(", animated", ", анимация") } else { "" };
            image.rows.push((tr!("Format", "Формат"), tr!(format!("{}, {}-bit{animated}", m.format, m.bits), format!("{}, {} бит{animated}", m.format, m.bits))));
        }
        let profile = info.profile.clone().or_else(|| (x.color_space == Some(1)).then(|| "sRGB".to_string()));
        if let Some(profile) = profile {
            image.rows.push((tr!("Colour profile", "Цветовой профиль"), profile));
        }
        sections.push(image);

        let mut shot = Section { title: tr!("Shooting", "Съёмка"), rows: Vec::new() };
        if let Some(camera) = format::camera(x.make.as_deref(), x.model.as_deref()) {
            shot.rows.push((tr!("Camera", "Камера"), camera));
        }
        if let Some(lens) = &x.lens {
            shot.rows.push((tr!("Lens", "Объектив"), lens.clone()));
        }
        if let Some(d) = x.taken.and_then(crate::win::date_time) {
            shot.rows.push((tr!("Date taken", "Дата съёмки"), d));
        }
        if let Some(t) = x.exposure.filter(|&t| t > 0.0) {
            shot.rows.push((tr!("Exposure", "Выдержка"), format::exposure(t)));
        }
        if let Some(f) = x.f_number.filter(|&f| f > 0.0) {
            shot.rows.push((tr!("Aperture", "Диафрагма"), format::aperture(f)));
        }
        if let Some(iso) = x.iso.filter(|&i| i > 0) {
            shot.rows.push(("ISO", iso.to_string()));
        }
        if let Some(focal) = format::focal(x.focal.filter(|&f| f > 0.0), x.focal_35) {
            shot.rows.push((tr!("Focal length", "Фокусное расстояние"), focal));
        }
        if let Some(ev) = x.bias.filter(|ev| ev.abs() >= 0.05) {
            shot.rows.push((tr!("Exposure bias", "Экспокоррекция"), format::bias(ev)));
        }
        // Bit 5: no flash at all.
        if let Some(flash) = x.flash.filter(|f| f & 0x20 == 0) {
            let fired = if flash & 1 != 0 { tr!("Fired", "Сработала") } else { tr!("Did not fire", "Не сработала") };
            shot.rows.push((tr!("Flash", "Вспышка"), fired.into()));
        }
        if let Some(software) = &x.software {
            shot.rows.push((tr!("Software", "Программа"), software.clone()));
        }
        sections.push(shot);

        let mut about = Section { title: tr!("Description", "Описание"), rows: Vec::new() };
        for (label, value) in [
            (tr!("Title", "Название"), &x.title),
            (tr!("Authors", "Авторы"), &x.artist),
            (tr!("Comment", "Комментарий"), &x.comment),
            (tr!("Tags", "Теги"), &x.keywords),
            (tr!("Copyright", "Авторские права"), &x.copyright),
        ] {
            if let Some(value) = value {
                about.rows.push((label, value.clone()));
            }
        }
        sections.push(about);

        if let Some(gps) = x.gps {
            let mut place = Section { title: tr!("Location", "Место съёмки"), rows: Vec::new() };
            // With a point, as maps take them, whatever the language.
            place.rows.push((tr!("Coordinates", "Координаты"), format!("{:.6}, {:.6}", gps.latitude, gps.longitude)));
            if let Some(alt) = gps.altitude {
                place.rows.push((tr!("Altitude", "Высота"), format::metres(alt)));
            }
            sections.push(place);
        }
        sections.retain(|s| !s.rows.is_empty());
        sections
    }
}

/// The section's title, then its rows: the label in a column of its own,
/// the value wrapped beside it.
fn draw_section(ui: &mut Ui, section: &Section) {
    ui.add_space(4.0);
    ui.label(RichText::new(section.title).strong().color(HEADING));
    ui.add_space(2.0);
    let label_width = (ui.available_width() * 0.4).clamp(70.0, 150.0);
    for (label, value) in &section.rows {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(label_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(label_width);
                ui.add(egui::Label::new(RichText::new(*label).color(TEXT_WEAK)).wrap());
            });
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(value).color(TEXT)).wrap().selectable(true));
            });
        });
    }
    ui.add_space(6.0);
}
