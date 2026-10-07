//! The information panel (I) at the right: the current image's histogram
//! in a section that folds (counted only while it is open, see
//! `App::update_histogram`), its file, its image, and what its metadata
//! says (see `info`), in sections; the values can be selected and copied.

use egui::{Color32, Margin, Mesh, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::{TEXT, TEXT_WEAK, panel_frame};
use crate::app::App;
use crate::format;
use crate::histogram::{BLUE, Channels, GREEN, Histogram, LUMA, RED};

const INFO_BG: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x2b, 0x2b);
const HEADING: egui::Color32 = egui::Color32::from_rgb(0xb8, 0xb8, 0xb8);
const GRAPH_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const GRAPH_HEIGHT: f32 = 96.0;
const LEVEL_LINE: Color32 = Color32::from_rgba_premultiplied(0x80, 0x80, 0x80, 0x80);
/// The colours of red, green, blue and luma: lines, and under them the
/// same at `FILL` opacity, so that where channels overlap their areas mix.
const COLOURS: [Color32; 4] = [
    Color32::from_rgb(0xf0, 0x40, 0x40),
    Color32::from_rgb(0x40, 0xe0, 0x40),
    Color32::from_rgb(0x50, 0x60, 0xff),
    Color32::from_rgb(0xc8, 0xc8, 0xc8),
];
const FILL: f32 = 0.35;

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
        let histogram = if self.show_histogram { self.current_histogram() } else { None };
        let mut open = self.show_histogram;
        let mut channels = self.histogram_channels;
        let panel = egui::Panel::right("info")
            .resizable(true)
            .default_size(self.info_width)
            .size_range(200.0..=600.0)
            .frame(panel_frame(INFO_BG, Margin::symmetric(10, 8)))
            .show(root_ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if sections.is_empty() {
                        ui.label(RichText::new(tr!("No image", "Нет изображения")).color(TEXT_WEAK));
                    } else {
                        histogram_section(ui, &mut open, &mut channels, histogram.as_ref().map(|h| h.as_deref()));
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
        self.histogram_channels = channels;
        if open != self.show_histogram {
            // Counting starts (or stops) in the next frame.
            self.show_histogram = open;
            root_ui.ctx().request_repaint();
        }
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
        if let Some(o) = x.orientation.filter(|&o| o != 1) {
            image.rows.push((tr!("Orientation", "Ориентация"), orientation(o)));
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
        if let Some(p) = x.program {
            shot.rows.push((tr!("Exposure program", "Режим съёмки"), program(p).into()));
        }
        if let Some(m) = x.metering {
            shot.rows.push((tr!("Metering", "Замер экспозиции"), metering(m).into()));
        }
        if let Some(w) = x.white_balance {
            let wb = if w == 0 { tr!("Auto", "Автоматический") } else { tr!("Manual", "Ручной") };
            shot.rows.push((tr!("White balance", "Баланс белого"), wb.into()));
        }
        if let Some(d) = x.distance {
            shot.rows.push((tr!("Subject distance", "Расстояние до объекта"), format::distance(d)));
        }
        if let Some(z) = x.zoom {
            shot.rows.push((tr!("Digital zoom", "Цифровой зум"), format::ratio(z)));
        }
        if let Some(software) = &x.software {
            shot.rows.push((tr!("Software", "Программа"), software.clone()));
        }
        sections.push(shot);

        let mut about = Section { title: tr!("Description", "Описание"), rows: Vec::new() };
        if let Some(r) = x.rating.filter(|&r| r > 0) {
            about.rows.push((tr!("Rating", "Оценка"), format::stars(r)));
        }
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
            if let Some((degrees, magnetic)) = gps.direction {
                place.rows.push((tr!("Direction", "Направление"), format::direction(degrees, magnetic)));
            }
            sections.push(place);
        }
        sections.retain(|s| !s.rows.is_empty());
        sections
    }
}

/// What the EXIF orientation `o` (2 to 8) does to the stored pixels to
/// show them.
fn orientation(o: u32) -> String {
    let what = match o {
        2 => tr!("mirrored", "отражение"),
        3 => tr!("turned 180°", "поворот на 180°"),
        4 => tr!("flipped upside down", "отражение сверху вниз"),
        5 => tr!("mirrored, turned 90° anticlockwise", "отражение и поворот на 90° против часовой"),
        6 => tr!("turned 90° clockwise", "поворот на 90° по часовой"),
        7 => tr!("mirrored, turned 90° clockwise", "отражение и поворот на 90° по часовой"),
        _ => tr!("turned 90° anticlockwise", "поворот на 90° против часовой"),
    };
    format!("{o}: {what}")
}

/// The EXIF ExposureProgram `p` (1 to 8).
fn program(p: u32) -> &'static str {
    match p {
        1 => tr!("Manual", "Ручной"),
        2 => tr!("Program", "Программный"),
        3 => tr!("Aperture priority", "Приоритет диафрагмы"),
        4 => tr!("Shutter priority", "Приоритет выдержки"),
        5 => tr!("Creative (depth of field)", "Творческий (глубина резкости)"),
        6 => tr!("Action (fast shutter)", "Спорт (короткая выдержка)"),
        7 => tr!("Portrait", "Портрет"),
        _ => tr!("Landscape", "Пейзаж"),
    }
}

/// The EXIF MeteringMode `m` (1 to 6).
fn metering(m: u32) -> &'static str {
    match m {
        1 => tr!("Average", "Средний по кадру"),
        2 => tr!("Centre-weighted", "Центровзвешенный"),
        3 => tr!("Spot", "Точечный"),
        4 => tr!("Multi-spot", "Многоточечный"),
        5 => tr!("Pattern (matrix)", "Оценочный (матричный)"),
        _ => tr!("Partial", "Частичный"),
    }
}

/// The section's title, then its rows.
fn draw_section(ui: &mut Ui, section: &Section) {
    ui.add_space(4.0);
    ui.label(RichText::new(section.title).strong().color(HEADING));
    ui.add_space(2.0);
    draw_rows(ui, &section.rows);
    ui.add_space(6.0);
}

/// The Histogram section: a heading with a triangle that opens and folds
/// it, and when open the graph of `histogram` (None: not available, Some(None):
/// being counted) of `channels`, the buttons that choose them, and its rows.
fn histogram_section(ui: &mut Ui, open: &mut bool, channels: &mut Channels, histogram: Option<Option<&Histogram>>) {
    ui.add_space(4.0);
    if fold_heading(ui, tr!("Histogram", "Гистограмма"), *open).clicked() {
        *open = !*open;
    }
    if !*open {
        ui.add_space(6.0);
        return;
    }
    ui.add_space(4.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), GRAPH_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, GRAPH_BG);
    let note = |text: &str| {
        let font = egui::TextStyle::Body.resolve(ui.style());
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, TEXT_WEAK);
    };
    let Some(histogram) = histogram else {
        note(tr!("Not available", "Недоступна"));
        ui.add_space(6.0);
        return;
    };
    let Some(h) = histogram else {
        note(tr!("Counting…", "Подсчёт…"));
        ui.add_space(6.0);
        return;
    };
    painter.extend(graph(h, rect, *channels));
    if let Some(pos) = response.hover_pos() {
        let width = rect.width() / 256.0;
        let level = (((pos.x - rect.left()) / width).max(0.0) as usize).min(255);
        painter.vline(rect.left() + (level as f32 + 0.5) * width, rect.y_range(), Stroke::new(1.0, LEVEL_LINE));
        let share = |channel: usize| format::percent(h.share(h.channels[channel][level] as u64));
        let rgb = format!("R {} · G {} · B {}", share(RED), share(GREEN), share(BLUE));
        let text = tr!(
            format!("Level {level}\n{rgb}\nLuma {}", share(LUMA)),
            format!("Уровень {level}\n{rgb}\nЯркость {}", share(LUMA))
        );
        response.on_hover_text_at_pointer(text);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for choice in Channels::ALL {
            let label = match choice {
                Channels::Luma => tr!("Brightness", "Яркость"),
                Channels::Rgb => "RGB",
                Channels::Red => "R",
                Channels::Green => "G",
                Channels::Blue => "B",
            };
            ui.selectable_value(channels, choice, label);
        }
    });
    ui.add_space(4.0);
    let mut rows = Vec::new();
    if let (Some(mean), Some(median)) = (h.mean(LUMA), h.median(LUMA)) {
        let mean = mean.round();
        rows.push((
            tr!("Brightness", "Яркость"),
            tr!(format!("mean {mean}, median {median}"), format!("средняя {mean}, медиана {median}")),
        ));
    }
    let clipped = |level: usize| {
        let shares = [RED, GREEN, BLUE].map(|c| h.channels[c][level]);
        if shares.iter().all(|&n| n == 0) {
            return tr!("none", "нет").to_string();
        }
        let [r, g, b] = shares.map(|n| format::percent(h.share(n as u64)));
        format!("R {r} · G {g} · B {b}")
    };
    rows.push((tr!("Clipped shadows (0)", "Провалы (0)"), clipped(0)));
    rows.push((tr!("Clipped highlights (255)", "Пересветы (255)"), clipped(255)));
    draw_rows(ui, &rows);
    ui.add_space(6.0);
}

/// A section's heading that opens and folds it on a click, with a triangle
/// pointing right when folded and down when open, as the gallery's.
fn fold_heading(ui: &mut Ui, title: &str, open: bool) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(title.to_owned(), font, HEADING);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::click());
    let colour = if response.hovered() { TEXT } else { TEXT_WEAK };
    let (c, r) = (pos2(rect.left() + 4.0, rect.center().y), 4.0);
    let points = if open {
        vec![c + vec2(-r, -r * 0.6), c + vec2(r, -r * 0.6), c + vec2(0.0, r * 0.8)]
    } else {
        vec![c + vec2(-r * 0.6, -r), c + vec2(r * 0.8, 0.0), c + vec2(-r * 0.6, r)]
    };
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
    painter.galley(pos2(rect.left() + 14.0, rect.top()), galley, HEADING);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The counts of every level of `channels` over `rect`: for each a line
/// through the levels' counts and the area under it, translucent, so that
/// overlapping channels show through each other; a count at
/// `Histogram::scale` or above reaches the top, cut there.
fn graph(h: &Histogram, rect: Rect, channels: Channels) -> Vec<egui::Shape> {
    let indices = channels.indices();
    let scale = h.scale(indices);
    if scale == 0 {
        return Vec::new();
    }
    let width = rect.width() / 256.0;
    let mut shapes = Vec::new();
    for &c in indices {
        let points: Vec<Pos2> = (0..256)
            .map(|level| {
                let height = (h.channels[c][level] as f32 / scale as f32).min(1.0) * rect.height();
                pos2(rect.left() + (level as f32 + 0.5) * width, rect.bottom() - height)
            })
            .collect();
        let fill = COLOURS[c].gamma_multiply(FILL);
        let mut area = Mesh::default();
        for p in &points {
            area.colored_vertex(*p, fill);
            area.colored_vertex(pos2(p.x, rect.bottom()), fill);
        }
        for i in 0..points.len() as u32 - 1 {
            let (top, bottom) = (2 * i, 2 * i + 1);
            area.add_triangle(top, bottom, top + 2);
            area.add_triangle(bottom, bottom + 2, top + 2);
        }
        shapes.push(egui::Shape::mesh(area));
        shapes.push(egui::Shape::line(points, Stroke::new(1.0, COLOURS[c])));
    }
    shapes
}

/// Rows of a label in a column of its own and the value wrapped beside it.
fn draw_rows(ui: &mut Ui, rows: &[(&str, String)]) {
    let label_width = (ui.available_width() * 0.4).clamp(70.0, 150.0);
    for (label, value) in rows {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Building the graph's mesh for the histogram of `QVIEW_BENCH_FILE`
    /// (done in every frame the panel is painted with it open), printed
    /// with `--nocapture`.
    #[test]
    #[ignore]
    fn graph_timings() {
        let path = std::path::PathBuf::from(std::env::var("QVIEW_BENCH_FILE").expect("QVIEW_BENCH_FILE"));
        let (_, meta) = crate::loader::decode(&path, 16384, true).unwrap();
        let h = meta.histogram.unwrap();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(260.0, GRAPH_HEIGHT));
        for round in 0..3 {
            let t = std::time::Instant::now();
            let mut vertices = 0;
            for _ in 0..1000 {
                let shapes = std::hint::black_box(graph(&h, rect, Channels::Rgb));
                vertices += shapes.iter().map(|s| if let egui::Shape::Mesh(m) = s { m.vertices.len() } else { 0 }).sum::<usize>();
            }
            let us = t.elapsed().as_secs_f64() * 1e6 / 1000.0;
            println!("round {round}: {us:.1} us per graph, {} vertices in its areas", vertices / 1000);
        }
    }
}
