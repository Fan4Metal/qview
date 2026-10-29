//! Save With Options (Ctrl+Alt+S): the image as Save As would save it
//! (turned, cropped, marked), shrunk to a width and height, its proportions
//! kept or not, in a format, a JPEG at a quality; Save As's dialog then
//! asks for the name. The options are those of one save, not settings: only
//! the quality is kept for the next time.

use std::path::PathBuf;

use egui::{Key, RichText};

use crate::app::App;
use crate::edit::Format;

/// What Save With Options asks for.
pub struct SaveOptions {
    /// The image they were asked for: the dialog closes when another is
    /// current (a slideshow, a file opened from outside).
    pub path: PathBuf,
    /// The size of the image as it would be saved.
    pub full: [u32; 2],
    /// The size it is saved at: `full` or less.
    pub size: [u32; 2],
    /// The height follows the width, and the width the height.
    pub keep: bool,
    pub format: Format,
    /// Of a JPEG, 1 to 100.
    pub quality: u8,
    /// Without the metadata (see `edit::Job::strip`).
    pub strip: bool,
}

impl SaveOptions {
    pub fn new(path: PathBuf, full: [u32; 2], format: Format, quality: u8) -> Self {
        let full = full.map(|v| v.max(1));
        SaveOptions { path, full, size: full, keep: true, format, quality: quality.clamp(1, 100), strip: false }
    }

    /// The width `w` (within the image); the height follows if kept.
    pub fn set_width(&mut self, w: u32) {
        let w = w.clamp(1, self.full[0]);
        self.size[0] = w;
        if self.keep {
            self.size[1] = proportional(w, self.full[0], self.full[1]);
        }
    }

    /// The height `h` (within the image); the width follows if kept.
    pub fn set_height(&mut self, h: u32) {
        let h = h.clamp(1, self.full[1]);
        self.size[1] = h;
        if self.keep {
            self.size[0] = proportional(h, self.full[1], self.full[0]);
        }
    }

    /// Both sides at `percent` of the image's.
    pub fn set_percent(&mut self, percent: f64) {
        let percent = percent.clamp(0.0, 100.0);
        self.size = self.full.map(|v| ((f64::from(v) * percent / 100.0).round() as u32).clamp(1, v));
    }

    /// The width's share of the image's, in percent.
    pub fn percent(&self) -> f64 {
        f64::from(self.size[0]) * 100.0 / f64::from(self.full[0])
    }

    /// The proportions kept again: the height follows the width.
    pub fn set_keep(&mut self, keep: bool) {
        self.keep = keep;
        if keep {
            self.set_width(self.size[0]);
        }
    }

    /// The size to shrink to, if it is not the image's own.
    pub fn resize(&self) -> Option<[u32; 2]> {
        (self.size != self.full).then_some(self.size)
    }
}

/// `side` of an image whose other side is `of`, scaled to `other`'s
/// proportion: at least 1, at most `other`.
fn proportional(side: u32, of: u32, other: u32) -> u32 {
    ((f64::from(side) * f64::from(other) / f64::from(of.max(1))).round() as u32).clamp(1, other.max(1))
}

impl App {
    /// The options' dialog: Save As… (or Enter) goes on to the name, Esc or
    /// Cancel gives up.
    pub(super) fn save_options_dialog(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.save_options.as_ref().is_some_and(|o| self.current.as_ref() != Some(&o.path)) {
            self.save_options = None;
        }
        let Some(o) = self.save_options.as_mut() else { return };
        // Enter saves unless it ends typing in a field.
        let typing = ctx.memory(|m| m.focused().is_some());
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("save_options")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(tr!("Save With Options", "Сохранение с параметрами"));
            ui.add_space(4.0);
            let [fw, fh] = o.full;
            ui.label(RichText::new(tr!(format!("The image: {fw} × {fh} px"), format!("Изображение: {fw} × {fh} px"))).weak());
            ui.add_space(8.0);
            egui::Grid::new("save_options_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                let speed = |side: u32| (f64::from(side) / 400.0).max(1.0);
                ui.label(tr!("Width", "Ширина"));
                let mut w = o.size[0];
                if ui.add(egui::DragValue::new(&mut w).range(1..=fw).speed(speed(fw)).suffix(" px")).changed() {
                    o.set_width(w);
                }
                ui.end_row();
                ui.label(tr!("Height", "Высота"));
                let mut h = o.size[1];
                if ui.add(egui::DragValue::new(&mut h).range(1..=fh).speed(speed(fh)).suffix(" px")).changed() {
                    o.set_height(h);
                }
                ui.end_row();
                ui.label(tr!("Scale", "Масштаб"));
                ui.horizontal(|ui| {
                    let mut percent = o.percent();
                    // Taken when typing ends: the percent read back from the
                    // width rounded to pixels would replace the text typed.
                    let field = egui::DragValue::new(&mut percent)
                        .range(0.1..=100.0)
                        .speed(0.5)
                        .max_decimals(1)
                        .suffix(" %")
                        .update_while_editing(false);
                    if ui.add(field).changed() {
                        o.set_percent(percent);
                    }
                    for p in [25, 50, 75, 100] {
                        let on = o.size == o.full.map(|v| ((f64::from(v) * f64::from(p) / 100.0).round() as u32).clamp(1, v));
                        if ui.selectable_label(on, format!("{p} %")).clicked() {
                            o.set_percent(f64::from(p));
                        }
                    }
                });
                ui.end_row();
                ui.label("");
                let mut keep = o.keep;
                if ui.checkbox(&mut keep, tr!("Keep the proportions", "Сохранять пропорции")).changed() {
                    o.set_keep(keep);
                }
                ui.end_row();
                ui.label(tr!("Format", "Формат"));
                egui::ComboBox::from_id_salt("save_options_format").selected_text(o.format.name()).width(180.0).show_ui(ui, |ui| {
                    for f in Format::ALL {
                        ui.selectable_value(&mut o.format, f, f.name());
                    }
                });
                ui.end_row();
                if o.format == Format::Jpeg {
                    ui.label(tr!("Quality", "Качество"));
                    let tip = tr!(
                        "Lower: a smaller file, more visible loss; 92 is the usual",
                        "Ниже — файл меньше, потери заметнее; обычно 92"
                    );
                    ui.add(egui::Slider::new(&mut o.quality, 1..=100)).on_hover_text(tip);
                    ui.end_row();
                }
                ui.label("");
                let tip = tr!(
                    "No EXIF (camera, date, GPS position), XMP, IPTC or comment; the ICC colour profile is kept",
                    "Без EXIF (камера, дата, координаты GPS), XMP, IPTC и комментария; цветовой профиль ICC сохраняется"
                );
                ui.checkbox(&mut o.strip, tr!("Without metadata", "Без метаданных")).on_hover_text(tip);
                ui.end_row();
            });
            if o.format == Format::Jpeg && Format::of(&o.path) == Some(Format::Jpeg) && o.resize().is_none() {
                ui.add_space(4.0);
                ui.label(RichText::new(tr!(
                    "The JPEG is encoded anew at this quality",
                    "JPEG будет закодирован заново с этим качеством"
                )).weak());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| RichText::new(s).size(16.0);
                if ui.add(egui::Button::new(text(tr!("Save As…", "Сохранить как…"))).min_size(egui::vec2(160.0, 34.0))).clicked() {
                    decision = Some(true);
                }
                if ui.add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() && !typing && ctx.input(|i| i.key_pressed(Key::Enter)) {
            decision = Some(true);
        }
        if decision.is_none() && modal.should_close() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                let options = self.save_options.take().expect("shown");
                self.save_quality = options.quality;
                self.save_with(ctx, frame, true, Some(options), false);
            }
            Some(false) => self.save_options = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_keep_their_proportions_within_the_image() {
        let mut o = SaveOptions::new(PathBuf::new(), [6000, 4000], Format::Jpeg, 92);
        assert_eq!(o.resize(), None);
        o.set_width(3000);
        assert_eq!(o.size, [3000, 2000]);
        o.set_height(1000);
        assert_eq!(o.size, [1500, 1000]);
        // Never larger than the image.
        o.set_width(9000);
        assert_eq!(o.size, [6000, 4000]);
        o.set_percent(25.0);
        assert_eq!((o.size, o.percent()), ([1500, 1000], 25.0));
        // Apart, then together again from the width.
        o.set_keep(false);
        o.set_height(3000);
        assert_eq!(o.size, [1500, 3000]);
        o.set_keep(true);
        assert_eq!(o.size, [1500, 1000]);
        // Tiny sides stay at least a pixel.
        o.set_width(1);
        assert_eq!(o.size, [1, 1]);
        o.set_percent(0.0);
        assert_eq!(o.size, [1, 1]);
        assert_eq!(o.resize(), Some([1, 1]));
        assert_eq!(SaveOptions::new(PathBuf::new(), [3, 1000], Format::Png, 0).quality, 1);
    }
}
