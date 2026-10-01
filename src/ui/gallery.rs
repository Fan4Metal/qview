//! The gallery on screen: the folder tree, the bar with the folder and the
//! cell size slider, and the grid of thumbnails (the state is in
//! `gallery`).

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, Align2, Color32, FontId, Layout, Margin, Painter, Rect, Response, RichText, Sense, Ui, pos2, vec2};

use super::{TEXT, TEXT_WEAK, panel_frame};
use crate::app::{App, file_name};
use crate::gallery::{self, ASPECTS, HOVER_DELAY, LABEL, MAX_SIZE, MIN_SIZE, PAD, Scroll};
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
        if let Some(dir) = tree.inner {
            self.open_folder(&ctx, dir);
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
        let before = (self.thumb_size, self.thumb_aspect);
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
                    ui.add_space(8.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if let Some(dir) = &self.dir {
                            let text = RichText::new(dir.display().to_string()).size(12.5).color(TEXT_WEAK);
                            ui.add(egui::Label::new(text).truncate());
                        }
                    });
                });
            });
        if (self.thumb_size, self.thumb_aspect) != before {
            self.thumb_size_changed();
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
        let modal_open = self.confirm_delete.is_some() || self.dialog.is_some();
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
        let gallery = self.gallery.as_mut()?;
        if n == 0 {
            gallery.want(Vec::new());
            let text = if self.scan.is_some() {
                None
            } else if self.dir.is_none() {
                Some(tr!("Choose a folder on the left", "Выберите папку слева"))
            } else {
                Some(tr!("No images in this folder", "В этой папке нет изображений"))
            };
            if let Some(text) = text {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), TEXT_WEAK);
            }
            return None;
        }
        // Auto: the folder's proportions once found, the last ones until
        // then.
        let aspect = match (self.thumb_aspect, &self.dir) {
            (Some(a), _) => a,
            (None, Some(dir)) if self.scan.is_none() => {
                gallery.auto_aspect(dir, &self.files).unwrap_or(gallery.shown_aspect)
            }
            (None, _) => gallery.shown_aspect,
        };
        if aspect != gallery.shown_aspect {
            // The current image stays where it was on screen.
            if let Some(i) = self.index
                && gallery.row_height > 0.0
            {
                let y = (i / gallery.columns.max(1)) as f32 * gallery.row_height;
                gallery.scroll = Some(Scroll::Keep(y - gallery.top));
            }
            gallery.shown_aspect = aspect;
        }
        let frame = gallery::frame_size(self.thumb_size, aspect);
        let grid = gallery::grid(rect.width(), frame, n);
        gallery.columns = grid.columns;
        gallery.row_height = grid.cell.y;
        gallery.page_rows = ((rect.height() / grid.cell.y).floor() as usize).max(1);

        // Follow the current image when it changes (keys, a deletion).
        if self.index.is_some() && self.current != gallery.scrolled_to {
            gallery.scroll.get_or_insert(Scroll::Visible);
        }
        let mut offset = None;
        if let (Some(scroll), Some(i)) = (gallery.scroll, self.index) {
            let y = (i / grid.columns) as f32 * grid.cell.y;
            let (top, height) = (gallery.top, rect.height());
            offset = match scroll {
                Scroll::Centre => Some((y - (height - grid.cell.y) / 2.0).max(0.0)),
                Scroll::Visible if y < top => Some(y),
                Scroll::Visible if y + grid.cell.y > top + height => Some(y + grid.cell.y - height),
                Scroll::Visible => None,
                Scroll::Keep(below) => Some((y - below.clamp(0.0, (height - grid.cell.y).max(0.0))).max(0.0)),
            };
            gallery.scroll = None;
            gallery.scrolled_to = self.current.clone();
        }
        // The empty space between and after the cells: its right click
        // offers the order. Made before the cells, which are on top of it.
        let background = ui.interact(rect, ui.id().with("grid_background"), Sense::CLICK);
        // Each folder keeps its own scroll position.
        let mut area = egui::ScrollArea::vertical().id_salt(("grid", &self.dir)).auto_shrink([false, false]);
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
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.set_height(grid.rows as f32 * grid.cell.y);
            let origin = ui.max_rect().min;
            let first = (viewport.min.y / grid.cell.y).floor().max(0.0) as usize;
            let last = ((viewport.max.y / grid.cell.y).ceil().max(0.0) as usize).min(grid.rows);
            let painter = ui.painter().clone();
            for i in (first * grid.columns..last * grid.columns).take_while(|&i| i < n) {
                let (row, col) = (i / grid.columns, i % grid.columns);
                let cell = Rect::from_min_size(
                    origin + vec2(col as f32 * grid.cell.x, row as f32 * grid.cell.y),
                    grid.cell,
                );
                let response = ui.interact(cell, ui.id().with(("cell", i)), Sense::CLICK);
                let path = &files[i];
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
            (first, last)
        });
        gallery.top = out.state.offset.y;

        // A screen ahead, then half a screen back, as many as the budget
        // keeps besides the visible ones.
        let (first, last) = out.inner;
        let page = (last - first).max(1);
        let room = gallery::cells_in_budget(gallery::side_needed(frame_px, fill, None)).saturating_sub(cells.len());
        let ahead = last * grid.columns..((last + page) * grid.columns).min(n);
        let back = first.saturating_sub(page / 2 + 1) * grid.columns..first * grid.columns;
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
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, true);
        ui.separator();
        ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
        ui.separator();
        self.item(ui, tr!("Delete…", "Удалить…").into(), "Del", Cmd::Delete, true);
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

/// A square `side` points wide beside the slider: small thumbnails on its
/// left, large on its right.
fn size_icon(ui: &mut Ui, side: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let square = Rect::from_center_size(rect.center(), vec2(side, side));
    ui.painter().rect_stroke(square, 1.0, egui::Stroke::new(1.2, SLIDER_ICON), egui::StrokeKind::Inside);
}
