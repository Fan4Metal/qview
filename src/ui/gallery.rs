//! The gallery on screen: the folder tree, the bar with the folder and the
//! cell size slider, and the grid of thumbnails (the state is in
//! `gallery`).

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, Align2, Color32, FontId, Layout, Margin, Painter, Rect, Response, RichText, Sense, Ui, pos2, vec2};

use super::{TEXT, TEXT_WEAK, panel_frame};
use crate::app::{App, file_name};
use crate::gallery::{self, ASPECTS, HEADER, HOVER_DELAY, LABEL, MAX_SIZE, MIN_SIZE, PAD, Scroll};
use crate::input::Cmd;
use crate::thumbs::Request;

const TREE_BG: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const GRID_BG: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
const CELL_HOVER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const CELL_SELECTED: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
/// The frame of a thumbnail still being made.
const CELL_EMPTY: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
/// The bar above the grid: darker than the toolbar above it, with a line
/// between them.
const BAR_BG: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
const BAR_LINE: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const SLIDER_ICON: Color32 = Color32::from_rgb(0xb0, 0xb0, 0xb0);
/// The slider's rail, darker than the bar, and its part left of the handle.
const SLIDER_RAIL: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
const SLIDER_FILL: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
/// The line between groups of controls in the bar.
const BAR_SEPARATOR: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
/// The line of a folder's header in the grid.
const HEADER_LINE: Color32 = Color32::from_rgb(0x48, 0x48, 0x48);

impl App {
    /// The gallery in place of the image area.
    pub(crate) fn gallery_ui(&mut self, root_ui: &mut Ui) {
        let ctx = root_ui.ctx().clone();
        let Some(gallery) = self.gallery.as_mut() else { return };
        let selected = self.dir.clone();
        let tree = egui::Panel::left("gallery_tree")
            .resizable(true)
            .default_size(self.tree_width)
            .size_range(140.0..=640.0)
            .frame(panel_frame(TREE_BG, Margin::symmetric(0, 4)))
            .show(root_ui, |ui| gallery.tree.show(ui, selected.as_deref()));
        self.tree_width = tree.response.rect.width().round();
        // In the mode chosen above the grid.
        if let Some(dir) = tree.inner {
            self.open_folder(&ctx, dir, self.deep);
        }
        self.gallery_bar(root_ui);
        let mut open = None;
        egui::CentralPanel::no_frame().show(root_ui, |ui| open = self.grid(ui));
        // Double click: the image.
        if let Some(i) = open {
            self.go(i);
            self.leave_gallery();
        }
    }

    /// The folder and the cell size slider above the grid.
    fn gallery_bar(&mut self, root_ui: &mut Ui) {
        let before = (self.thumb_size, self.thumb_aspect, self.by_folder);
        let mut deep = self.deep;
        let shown = self.gallery.as_ref().map_or(1.0, |g| g.shown_aspect);
        egui::Panel::top("gallery_bar")
            .frame(panel_frame(BAR_BG, Margin::symmetric(8, 3)))
            .show_separator_line(false)
            .show(root_ui, |ui| {
                let top = ui.max_rect().top() - 3.0;
                let x = ui.max_rect().x_range().expand(8.0);
                ui.painter().hline(x, top + 0.5, egui::Stroke::new(1.0, BAR_LINE));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().slider_width = 150.0;
                    let visuals = ui.visuals_mut();
                    visuals.widgets.inactive.bg_fill = SLIDER_RAIL;
                    visuals.selection.bg_fill = SLIDER_FILL;
                    let tip = tr!("Thumbnail size (+ and -, Ctrl+Wheel)", "Размер миниатюр (+ и -, Ctrl+колесо)");
                    size_icon(ui, 12.0);
                    let slider = egui::Slider::new(&mut self.thumb_size, MIN_SIZE..=MAX_SIZE)
                        .show_value(false)
                        .trailing_fill(true)
                        .handle_shape(egui::style::HandleShape::Circle);
                    ui.add(slider).on_hover_text(tip);
                    size_icon(ui, 7.0);
                    ui.add_space(12.0);
                    ui.checkbox(&mut self.thumb_fill, tr!("Fill cells", "Заполнять ячейки")).on_hover_text(tr!(
                        "Thumbnails fill their cells, the edges cropped",
                        "Миниатюры заполняют ячейки, края обрезаются"
                    ));
                    ui.add_space(12.0);
                    let name = match self.thumb_aspect {
                        Some(a) => gallery::aspect_name(a).to_string(),
                        None => {
                            let shown = gallery::aspect_name(shown);
                            tr!(format!("Auto ({shown})"), format!("Авто ({shown})"))
                        }
                    };
                    egui::ComboBox::from_id_salt("cell_aspect")
                        .selected_text(name)
                        .width(96.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.thumb_aspect, None, tr!("Auto", "Авто"));
                            for (name, aspect) in ASPECTS {
                                ui.selectable_value(&mut self.thumb_aspect, Some(aspect), name);
                            }
                        })
                        .response
                        .on_hover_text(tr!(
                            "Proportions of the cells; Auto: those of most images of the folder",
                            "Пропорции ячеек; «Авто» — как у большинства изображений папки"
                        ));
                    // What is listed, set apart from how the cells look.
                    ui.add_space(10.0);
                    bar_separator(ui);
                    ui.add_space(10.0);
                    // Always there, so that nothing moves; only for the
                    // sub-folders.
                    let by_folder = egui::Checkbox::new(&mut self.by_folder, tr!("By folder", "По папкам"));
                    ui.add_enabled(self.deep, by_folder)
                        .on_hover_text(tr!(
                            "Each folder's images sorted and shown on their own, under a header",
                            "Изображения каждой папки сортируются и показываются отдельно, под её заголовком"
                        ))
                        .on_disabled_hover_text(tr!(
                            "With the sub-folders: each folder's images on their own",
                            "Для вложенных папок: изображения каждой папки отдельно"
                        ));
                    ui.add_space(8.0);
                    ui.checkbox(&mut deep, tr!("Sub-folders", "Вложенные папки")).on_hover_text(tr!(
                        "The images of all sub-folders too",
                        "Также изображения всех вложенных папок"
                    ));
                    ui.add_space(8.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if let Some(dir) = &self.dir {
                            let text = RichText::new(dir.display().to_string()).size(12.5).color(TEXT_WEAK);
                            ui.add(egui::Label::new(text).truncate());
                        }
                    });
                });
            });
        if (self.thumb_size, self.thumb_aspect, self.by_folder) != before {
            self.thumb_size_changed();
        }
        if deep != self.deep {
            self.set_deep(root_ui.ctx(), deep);
        } else if self.deep && self.by_folder != before.2 {
            // In the order of each folder, or in one through all.
            self.relist(root_ui.ctx());
        }
    }

    /// The cells of the visible rows, and the thumbnails to make: those of
    /// the visible cells first, then a screen ahead and half a screen
    /// back. A click selects a cell; returns the cell double-clicked.
    fn grid(&mut self, ui: &mut Ui) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, GRID_BG);
        // Ctrl+Wheel sizes the cells; the wheel alone scrolls.
        let modal_open = self.modal_open();
        if !modal_open && !egui::Popup::is_any_open(&ctx) {
            let (_, zoom) = self.wheel.read(&ctx);
            if zoom != 0 {
                for _ in 0..zoom.unsigned_abs() {
                    self.thumb_size = gallery::step_size(self.thumb_size, zoom > 0);
                }
                self.thumb_size_changed();
            }
        }
        let n = self.files.len();
        let listing = self.listing();
        let gallery = self.gallery.as_mut()?;
        if n == 0 {
            gallery.want(Vec::new());
            let text = if let Some(scan) = &self.scan {
                // The sub-folders of a large folder take a while.
                self.deep.then(|| {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    let found = scan.found();
                    tr!(
                        format!("Looking through the sub-folders… {found} images"),
                        format!("Просмотр вложенных папок… изображений: {found}")
                    )
                })
            } else if self.dir.is_none() {
                Some(tr!("Choose a folder on the left", "Выберите папку слева").to_string())
            } else if self.deep {
                Some(tr!("No images in this folder and its sub-folders", "В этой папке и вложенных папках нет изображений").into())
            } else {
                Some(tr!("No images in this folder", "В этой папке нет изображений").into())
            };
            if let Some(text) = text {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), TEXT_WEAK);
            }
            return None;
        }
        // Auto: the folder's proportions once found, the last ones until
        // then.
        let aspect = match (self.thumb_aspect, &listing) {
            (Some(a), _) => a,
            (None, Some(listing)) if self.scan.is_none() => {
                gallery.auto_aspect(listing, &self.files).unwrap_or(gallery.shown_aspect)
            }
            (None, _) => gallery.shown_aspect,
        };
        if aspect != gallery.shown_aspect {
            // The current image stays where it was on screen.
            if let Some(i) = self.index
                && !gallery.layout.sections.is_empty()
            {
                gallery.scroll = Some(Scroll::Keep(gallery.layout.cell_pos(i).y - gallery.top));
            }
            gallery.shown_aspect = aspect;
        }
        let frame = gallery::frame_size(self.thumb_size, aspect);
        // By folder: a section for each, under a header; not while the
        // listing in one order is still on screen.
        let sections = self.deep && self.by_folder && self.scan.is_none();
        let (starts, header) = if sections { (&self.starts[..], HEADER) } else { (&[][..], 0.0) };
        let layout = gallery::Layout::new(rect.width(), frame, n, starts, header);
        let cell = layout.cell;
        gallery.page_rows = ((rect.height() / cell.y).floor() as usize).max(1);

        // Follow the current image when it changes (keys, a deletion).
        if self.index.is_some() && self.current != gallery.scrolled_to {
            gallery.scroll.get_or_insert(Scroll::Visible);
        }
        let mut offset = None;
        if let (Some(scroll), Some(i)) = (gallery.scroll, self.index) {
            let y = layout.cell_pos(i).y;
            // The header above the first row of a folder comes into view
            // with it.
            let above = if layout.sections.iter().any(|s| s.first + layout.columns > i && s.first <= i) {
                layout.header
            } else {
                0.0
            };
            let (top, height) = (gallery.top, rect.height());
            offset = match scroll {
                Scroll::Centre => Some((y - (height - cell.y) / 2.0).max(0.0)),
                Scroll::Visible if y - above < top => Some(y - above),
                Scroll::Visible if y + cell.y > top + height => Some(y + cell.y - height),
                Scroll::Visible => None,
                Scroll::Keep(below) => Some((y - below.clamp(0.0, (height - cell.y).max(0.0))).max(0.0)),
            };
            gallery.scroll = None;
            gallery.scrolled_to = self.current.clone();
        }
        // The empty space between and after the cells: its right click
        // offers the order. Made before the cells, which are on top of it.
        let background = ui.interact(rect, ui.id().with("grid_background"), Sense::CLICK);
        // Each folder keeps its own scroll position.
        let mut area = egui::ScrollArea::vertical().id_salt(("grid", &listing)).auto_shrink([false, false]);
        if let Some(y) = offset {
            area = area.vertical_scroll_offset(y);
        }

        let fill = self.thumb_fill;
        let frame_px = frame * ppp;
        let mut requests = Vec::new();
        let mut cells: Vec<(usize, Response)> = Vec::new();
        let mut hovered = None;
        let files = &self.files;
        let index = self.index;
        let (dir, deep) = (self.dir.as_deref(), self.deep);
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.set_height(layout.height);
            let origin = ui.max_rect().min;
            let painter = ui.painter().clone();
            if layout.header > 0.0 {
                let first = layout.sections.partition_point(|s| s.y + layout.header <= viewport.min.y);
                for (k, s) in layout.sections.iter().enumerate().skip(first) {
                    if s.y >= viewport.max.y {
                        break;
                    }
                    let rect = Rect::from_min_size(origin + vec2(0.0, s.y), vec2(ui.max_rect().width(), layout.header));
                    folder_header(&painter, rect, &folder_name(dir, &files[s.first]), layout.len(k));
                }
            }
            let visible = layout.visible(viewport.min.y, viewport.max.y);
            for i in visible.clone() {
                let cell = Rect::from_min_size(origin + layout.cell_pos(i), cell);
                let response = ui.interact(cell, ui.id().with(("cell", i)), Sense::CLICK);
                let path = &files[i];
                // Which sub-folder it is in.
                let response = match dir.filter(|_| deep).and_then(|d| path.strip_prefix(d).ok()) {
                    Some(relative) => response.on_hover_text(relative.to_string_lossy()),
                    None => response,
                };
                if index == Some(i) {
                    painter.rect_filled(cell.shrink(1.0), 3.0, CELL_SELECTED);
                } else if response.hovered() {
                    painter.rect_filled(cell.shrink(1.0), 3.0, CELL_HOVER);
                }
                if response.hovered() {
                    hovered = Some(path.clone());
                }
                let square = Rect::from_min_size(pos2(cell.center().x - frame.x / 2.0, cell.top() + PAD), frame);
                let ratio = gallery.cache.get(path).and_then(|t| t.ratio());
                let side = gallery::side_needed(frame_px, fill, ratio);
                if gallery.needs(path, side) {
                    requests.push(Request { path: path.clone(), side });
                }
                match gallery.thumb(path) {
                    Some(t) => match &t.texture {
                        Some(texture) => {
                            let px = vec2(t.px[0] as f32, t.px[1] as f32);
                            let (rect, uv) = gallery::place_thumb(square, px, ppp, fill, t.shrunk());
                            painter.image(texture.id(), rect, uv, Color32::WHITE);
                        }
                        None => {
                            let ext = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                            painter.text(square.center(), Align2::CENTER_CENTER, ext, FontId::proportional(13.0), TEXT_WEAK);
                        }
                    },
                    None => {
                        painter.rect_filled(square.shrink(frame.min_elem() * 0.08), 2.0, CELL_EMPTY);
                    }
                }
                label(&painter, &file_name(path), cell, square.bottom() + 2.0);
                cells.push((i, response));
            }
            visible
        });
        gallery.top = out.state.offset.y;
        let columns = layout.columns;
        gallery.layout = layout;

        // A screen ahead, then half a screen back, as many as the budget
        // keeps besides the visible ones.
        let visible = out.inner;
        let page = visible.len().max(1);
        let room = gallery::cells_in_budget(gallery::side_needed(frame_px, fill, None)).saturating_sub(cells.len());
        let ahead = visible.end..(visible.end + page).min(n);
        let back = visible.start.saturating_sub(page / 2 + columns)..visible.start;
        for i in ahead.chain(back.rev()).take(room) {
            let ratio = gallery.cache.get(&self.files[i]).and_then(|t| t.ratio());
            let side = gallery::side_needed(frame_px, fill, ratio);
            if gallery.needs(&self.files[i], side) {
                requests.push(Request { path: self.files[i].clone(), side });
            }
        }
        gallery.want(requests);

        // The image under the pointer is decoded ahead (see `App::wanted`).
        match hovered {
            Some(p) if gallery.hovered.as_ref().is_none_or(|(h, _)| *h != p) => {
                gallery.hovered = Some((p, std::time::Instant::now()));
                ctx.request_repaint_after(HOVER_DELAY);
            }
            Some(_) => {}
            None => gallery.hovered = None,
        }

        let mut opened = None;
        for (i, response) in cells {
            if response.clicked() || response.secondary_clicked() {
                self.go(i);
            }
            if crate::input::double_clicked(&response) {
                opened = Some(i);
            }
            response.context_menu(|ui| self.cell_menu(ui));
        }
        background.context_menu(|ui| {
            ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
        });
        opened
    }

    /// Keep the current image in view when the cells change size.
    fn thumb_size_changed(&mut self) {
        if let Some(g) = &mut self.gallery {
            g.scroll = Some(Scroll::Visible);
        }
    }

    /// Right click on a cell, which is then the current image.
    fn cell_menu(&mut self, ui: &mut Ui) {
        self.item(ui, tr!("Open", "Открыть").into(), "Enter", Cmd::Gallery, true);
        ui.separator();
        self.item(ui, tr!("Copy", "Копировать").into(), "Ctrl+C", Cmd::Copy, true);
        self.item(ui, tr!("Rename…", "Переименовать…").into(), "F2", Cmd::Rename, true);
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, true);
        ui.separator();
        ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
        ui.separator();
        self.item(ui, tr!("Delete…", "Удалить…").into(), "Del", Cmd::Delete, true);
    }
}

/// The folder of `file` as its header shows it: its path from `dir`, the
/// folder shown, or the name of `dir` itself.
fn folder_name(dir: Option<&std::path::Path>, file: &std::path::Path) -> String {
    let parent = file.parent().unwrap_or(file);
    match dir.and_then(|d| parent.strip_prefix(d).ok()) {
        Some(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().into_owned(),
        _ => file_name(dir.unwrap_or(parent)),
    }
}

/// The header of a folder's images in the grid: its name, how many images
/// it has, and a line to the right edge.
fn folder_header(painter: &Painter, rect: Rect, name: &str, count: usize) {
    let y = rect.center().y + 2.0;
    let left = rect.left() + PAD + 4.0;
    let mut job = LayoutJob::simple_singleline(name.to_owned(), FontId::proportional(13.0), TEXT);
    job.wrap = TextWrapping::truncate_at_width(rect.width() * 0.7);
    let name = painter.layout_job(job);
    let count = painter.layout_no_wrap(count.to_string(), FontId::proportional(12.0), TEXT_WEAK);
    painter.galley(pos2(left, y - name.size().y / 2.0), name.clone(), TEXT);
    let x = left + name.size().x + 8.0;
    painter.galley(pos2(x, y - count.size().y / 2.0), count.clone(), TEXT_WEAK);
    let x = x + count.size().x + 10.0;
    let right = rect.right() - PAD - 4.0;
    if x < right {
        painter.hline(x..=right, y, egui::Stroke::new(1.0, HEADER_LINE));
    }
}

/// The file name under a thumbnail, cut short with an ellipsis.
fn label(painter: &Painter, name: &str, cell: Rect, top: f32) {
    let mut job = LayoutJob::simple_singleline(name.to_owned(), FontId::proportional(12.0), TEXT);
    job.wrap = TextWrapping::truncate_at_width(cell.width() - 2.0 * PAD);
    let galley = painter.layout_job(job);
    let pos = pos2(cell.center().x - galley.size().x / 2.0, top + (LABEL - galley.size().y) / 2.0);
    painter.galley(pos, galley, TEXT);
}

/// A vertical line between groups of controls in the bar.
fn bar_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, BAR_SEPARATOR));
}

/// A square `side` points wide beside the slider: small thumbnails on its
/// left, large on its right.
fn size_icon(ui: &mut Ui, side: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let square = Rect::from_center_size(rect.center(), vec2(side, side));
    ui.painter().rect_stroke(square, 1.0, egui::Stroke::new(1.2, SLIDER_ICON), egui::StrokeKind::Inside);
}
