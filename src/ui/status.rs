//! The status bar:
//! `3/16 | name.jpg | 337.3 KB | 800x532x24b JPEG | Modified Date: … | 100%`.

use egui::{RichText, Stroke, Ui};

use super::{STATUS_BG, TEXT, TEXT_WEAK, panel_frame};
use crate::app::{App, Slot};
use crate::format;

const SEPARATOR: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x4a, 0x4a);

impl App {
    /// The status bar's fields, left to right.
    fn status_fields(&self, ppp: f32) -> Vec<String> {
        let mut fields = Vec::new();
        fields.push(match self.index {
            Some(i) if self.scan.is_none() => format!("{}/{}", i + 1, self.files.len()),
            _ if self.current.is_none() && self.scan.is_none() => return fields,
            _ => "?/?".into(),
        });
        let Some(current) = &self.current else { return fields };
        fields.push(self.display_name(current));
        let modified = |fields: &mut Vec<String>, modified: u64| {
            if let Some(date) = crate::win::local_date_time(modified).filter(|_| modified != 0) {
                fields.push(tr!(format!("Modified Date: {date}"), format!("Дата изменения: {date}")));
            }
        };
        // In the gallery, what the thumbnail found out until the image is
        // decoded.
        let thumb = self.gallery.as_ref().filter(|_| self.gallery_open).and_then(|g| g.cache.get(current));
        match (&self.shown, self.cache.get(current), thumb) {
            (Some((path, picture)), _, _) if path == current => {
                let m = &picture.meta;
                fields.push(format::file_size(m.file_size));
                let animated = if m.animated { tr!(", animated", ", анимация") } else { "" };
                fields.push(format!("{}x{}x{}b {}{animated}", m.width, m.height, m.bits, m.format));
                modified(&mut fields, m.modified);
                if !self.gallery_open {
                    let zoom = format::zoom(self.view.scale(picture.size(), self.viewport, ppp));
                    fields.push(if self.view.keep {
                        tr!(format!("{zoom} (kept)"), format!("{zoom} (сохраняется)"))
                    } else {
                        zoom
                    });
                }
            }
            (_, Some(Slot::Failed(e)), _) => fields.push(e.clone()),
            (_, _, Some(t)) if t.file_size > 0 => {
                fields.push(format::file_size(t.file_size));
                if t.width > 0 {
                    fields.push(format!("{}x{}", t.width, t.height));
                }
                modified(&mut fields, t.modified);
            }
            _ if self.gallery_open => {}
            _ => fields.push(tr!("Loading…", "Загрузка…").into()),
        }
        fields
    }

    pub(crate) fn status_bar(&mut self, root_ui: &mut Ui) {
        let ppp = root_ui.ctx().pixels_per_point();
        let fields = self.status_fields(ppp);
        let notice = self.current_notice().map(str::to_owned);
        egui::Panel::bottom("status")
            .frame(panel_frame(STATUS_BG, egui::Margin::symmetric(6, 3)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    for field in &fields {
                        ui.label(RichText::new(field).size(12.5).color(TEXT));
                        separator(ui);
                    }
                    if let Some(text) = notice {
                        ui.label(RichText::new(text).size(12.5).color(TEXT_WEAK));
                    }
                    // Keeps the bar's height when it is empty.
                    ui.label(RichText::new(" ").size(12.5));
                });
            });
    }
}

fn separator(ui: &mut Ui) {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, SEPARATOR));
}
