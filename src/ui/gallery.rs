//! The gallery on screen: the folder tree, the bar with the folder and the
//! cell size slider, and the grid of thumbnails (the state is in
//! `gallery`).

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, Align2, Color32, FontId, Layout, Margin, Painter, PointerButton, Pos2, Rect, Response, RichText, Sense, Ui, pos2, vec2};

use super::{TEXT, TEXT_WEAK, panel_frame};
use crate::app::{App, file_name};
use crate::gallery::{self, ASPECTS, Cell, HEADER, HOVER_DELAY, LABEL, MAX_SIZE, MIN_SIZE, PAD, Scroll};
use crate::input::Cmd;
use crate::thumbs::Request;
use crate::tree::Action;

const TREE_BG: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const GRID_BG: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
const CELL_HOVER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const CELL_SELECTED: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
/// The outline of the current image while images are chosen.
const CELL_FOCUS: Color32 = Color32::WHITE;
/// The colour of choosing: the outline and the tick of a chosen image, the
/// frame dragged over the grid.
const ACCENT: Color32 = Color32::from_rgb(0x4a, 0x90, 0xe2);
/// A chosen cell: the accent at 30% over the grid.
const CELL_CHOSEN: Color32 = Color32::from_rgb(0x2e, 0x42, 0x5d);
/// How fast the grid scrolls while the frame is dragged past its edge: a
/// part of the distance, in points per frame.
const BAND_SCROLL: f32 = 0.3;
/// The frame of a thumbnail still being made.
const CELL_EMPTY: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
/// A sub-folder's cell: a dim amber ground (lighter under the pointer and
/// with the cursor), and the places of its pictures with none to show.
const FOLDER_CELL: Color32 = Color32::from_rgb(0x3a, 0x32, 0x20);
const FOLDER_HOVER: Color32 = Color32::from_rgb(0x48, 0x3e, 0x27);
const FOLDER_SELECTED: Color32 = Color32::from_rgb(0x62, 0x53, 0x2e);
const FOLDER_SLOT: Color32 = Color32::from_rgb(0x2c, 0x27, 0x1b);
/// The outline of a folder with no images.
const FOLDER_LINE: Color32 = Color32::from_rgb(0x8a, 0x76, 0x48);
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

/// The filter's text field (Ctrl+F); while it has the focus, the keys are
/// its own (`App::ui`).
pub const FILTER_ID: &str = "gallery_filter";
/// Its width.
const FILTER_WIDTH: f32 = 170.0;
/// Width of the list of what is listed.
const LISTING_WIDTH: f32 = 190.0;

/// What the grid lists, as the list above it offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listing {
    /// The folder alone.
    Alone,
    /// With its sub-folders, all in one order.
    Deep,
    /// With its sub-folders, folder by folder under headers.
    DeepByFolder,
    /// The favourites or an archive, from several folders already: in one
    /// order, or folder by folder.
    Flat,
    ByFolder,
}

impl Listing {
    fn offered(from_folders: bool) -> &'static [Listing] {
        if from_folders { &[Listing::Flat, Listing::ByFolder] } else { &[Listing::Alone, Listing::Deep, Listing::DeepByFolder] }
    }

    fn current(from_folders: bool, deep: bool, by_folder: bool) -> Listing {
        match (from_folders, deep, by_folder) {
            (true, _, false) => Listing::Flat,
            (true, _, true) => Listing::ByFolder,
            (false, false, _) => Listing::Alone,
            (false, true, false) => Listing::Deep,
            (false, true, true) => Listing::DeepByFolder,
        }
    }

    /// `(deep, by_folder)` for this choice: the folder alone keeps the
    /// grouping for the favourites and the next sub-folders.
    fn apply(self, deep: bool, by_folder: bool) -> (bool, bool) {
        match self {
            Listing::Alone => (false, by_folder),
            Listing::Deep => (true, false),
            Listing::DeepByFolder => (true, true),
            Listing::Flat => (deep, false),
            Listing::ByFolder => (deep, true),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Listing::Alone => tr!("This folder only", "Только эта папка"),
            Listing::Deep => tr!("With sub-folders", "С вложенными папками"),
            Listing::DeepByFolder => tr!("With sub-folders, by folder", "С вложенными, по папкам"),
            Listing::Flat => tr!("One list", "Общим списком"),
            Listing::ByFolder => tr!("By folder", "По папкам"),
        }
    }
}

impl App {
    /// The gallery in place of the image area.
    pub(crate) fn gallery_ui(&mut self, root_ui: &mut Ui) {
        let ctx = root_ui.ctx().clone();
        let favorites = self.favorites.len();
        let Some(gallery) = self.gallery.as_mut() else { return };
        let selected = self.dir.clone();
        let tree = egui::Panel::left("gallery_tree")
            .resizable(true)
            .default_size(self.tree_width)
            .size_range(140.0..=640.0)
            .frame(panel_frame(TREE_BG, Margin::symmetric(0, 4)))
            .show(root_ui, |ui| gallery.tree.show(ui, selected.as_deref(), favorites));
        self.tree_width = tree.response.rect.width().round();
        // In the mode chosen above the grid.
        if let Some(dir) = tree.inner {
            self.open_folder(&ctx, dir, self.deep);
        }
        match self.gallery.as_mut().and_then(|g| g.tree.take_action()) {
            Some(Action::Pin(dir) | Action::Unpin(dir)) => self.toggle_pin(dir),
            Some(Action::SetKey(dir, key)) => self.set_pin_key(dir, key),
            // Opened where it is, the tree taken down to it.
            Some(Action::ShowInTree(dir)) => {
                self.open_folder(&ctx, dir.clone(), self.deep);
                if let Some(gallery) = &mut self.gallery {
                    gallery.tree.reveal(&dir);
                }
            }
            Some(Action::ShowInExplorer(dir)) => crate::win::show_in_explorer(&dir),
            Some(Action::Rename(dir)) => self.ask_rename_folder(dir),
            Some(Action::Delete(dir)) => self.ask_delete_folder(dir),
            None => {}
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

    /// The folder, the filter, what is listed and how the cells look,
    /// above the grid.
    fn gallery_bar(&mut self, root_ui: &mut Ui) {
        let before = (self.thumb_size, self.thumb_aspect);
        let filter_before = self.name_filter.clone();
        let (mut deep, mut by_folder) = (self.deep, self.by_folder);
        let (in_favorites, mixed) = (self.in_favorites(), self.mixed());
        // The favourites and an archive come from several folders already.
        let from_folders = in_favorites || self.in_archive();
        // Quick Access has no images to list one way or another.
        let quick_access = self.in_quick_access();
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
                    let tip = tr!(
                        "Thumbnail size (+ and -); Ctrl+Wheel over the grid steps between the sizes at which the cells fill the width exactly",
                        "Размер миниатюр (+ и -); Ctrl+колесо над сеткой ходит по размерам, при которых ячейки заполняют ширину точно"
                    );
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
                    // What is listed, as one list of whole choices, so that
                    // it reads what includes what. The choice stays for the
                    // folders chosen next.
                    let choice = Listing::current(from_folders, deep, by_folder);
                    let tip = if from_folders {
                        tr!(
                            "The images in one order, or folder by folder under headers",
                            "Изображения общим списком или по папкам под заголовками"
                        )
                    } else {
                        tr!(
                            "This folder's images, or those of all its sub-folders too: in one order, or folder by folder under headers",
                            "Изображения этой папки или также всех вложенных: общим списком или по папкам под заголовками"
                        )
                    };
                    ui.add_enabled_ui(!quick_access, |ui| {
                        egui::ComboBox::from_id_salt("listing")
                            .selected_text(choice.name())
                            .width(LISTING_WIDTH)
                            .show_ui(ui, |ui| {
                                let mut chosen = choice;
                                for c in Listing::offered(from_folders) {
                                    ui.selectable_value(&mut chosen, *c, c.name());
                                }
                                if chosen != choice {
                                    (deep, by_folder) = chosen.apply(deep, by_folder);
                                }
                            })
                            .response
                            .on_hover_text(tip);
                    });
                    ui.add_space(12.0);
                    self.filter_field(ui);
                    ui.add_space(8.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if in_favorites {
                            let n = self.favorites.len();
                            ui.label(RichText::new(tr!("Favorites", "Избранное")).size(12.5).color(TEXT));
                            ui.label(RichText::new(n.to_string()).size(12.5).color(TEXT_WEAK));
                        } else if quick_access {
                            let n = self.pinned.len();
                            ui.label(RichText::new(tr!("Quick Access", "Быстрый доступ")).size(12.5).color(TEXT));
                            ui.label(RichText::new(n.to_string()).size(12.5).color(TEXT_WEAK));
                        } else if let Some(dir) = &self.dir {
                            let text = RichText::new(dir.display().to_string()).size(12.5).color(TEXT_WEAK);
                            ui.add(egui::Label::new(text).truncate());
                        }
                    });
                });
            });
        if (self.thumb_size, self.thumb_aspect) != before {
            self.thumb_size_changed();
        }
        if self.name_filter != filter_before {
            self.filter_changed();
        }
        if deep != self.deep {
            // The grouping goes with the new listing.
            self.by_folder = by_folder;
            self.set_deep(root_ui.ctx(), deep);
        } else if by_folder != self.by_folder {
            self.by_folder = by_folder;
            self.thumb_size_changed();
            if mixed {
                // In the order of each folder, or in one through all.
                self.relist(root_ui.ctx());
            }
        }
    }

    /// The filter: a magnifier until it is wanted (`/`, Ctrl+F, a click on
    /// it), then a field with a cross to clear it on its right (the bar is
    /// laid out from the right). The field stays while it has the focus or
    /// a filter, so that a filter in effect is always in sight; empty and
    /// left, it is a magnifier again. Esc clears it, Enter gives the keys
    /// back to the grid.
    fn filter_field(&mut self, ui: &mut Ui) {
        let id = egui::Id::new(FILTER_ID);
        let open = std::mem::take(&mut self.filter_open);
        if self.name_filter.is_empty() && !open && !ui.memory(|m| m.has_focus(id)) {
            let tip = tr!("Filter the images by name ( / )", "Фильтр изображений по имени ( / )");
            if magnifier(ui).on_hover_text(tip).clicked() {
                ui.memory_mut(|m| m.request_focus(id));
                self.filter_open = true;
            }
            return;
        }
        if !self.name_filter.is_empty() {
            let clear = ui.add(egui::Button::new(RichText::new("×").color(TEXT_WEAK)).frame(false));
            if clear.on_hover_text(tr!("Clear the filter", "Очистить фильтр")).clicked() {
                self.name_filter.clear();
            }
        }
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let hint = RichText::new(tr!("Filter by name", "Фильтр по имени")).color(TEXT_WEAK);
        let edit = egui::TextEdit::singleline(&mut self.name_filter).id(id).hint_text(hint).desired_width(FILTER_WIDTH);
        let response = ui.add(edit).on_hover_text(tr!(
            "Only the images whose names hold every word; *.png and other masks with * and ?. Esc clears it",
            "Только изображения, в именах которых есть все слова; маски вида *.png с * и ?. Esc очищает"
        ));
        // egui takes the focus away on Esc (before the frame: see
        // `App::filter_focused`).
        if response.lost_focus() && escape {
            self.name_filter.clear();
        }
        self.filter_focused = response.has_focus();
    }

    /// The cells of the visible rows, and the thumbnails to make: those of
    /// the visible cells first, then a screen ahead and half a screen
    /// back. A click selects a cell; returns the cell double-clicked.
    fn grid(&mut self, ui: &mut Ui) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, GRID_BG);
        // Ctrl+Wheel sizes the cells, one column more or fewer each notch,
        // the cells filling the width (`gallery::fit_sizes`); the wheel
        // alone scrolls.
        let modal_open = self.modal_open();
        if !modal_open && !egui::Popup::is_any_open(&ctx) {
            let (_, zoom) = self.wheel.read(&ctx);
            if zoom != 0 {
                let aspect = self.thumb_aspect.or_else(|| self.gallery.as_ref().map(|g| g.shown_aspect)).unwrap_or(1.0);
                let fits = gallery::fit_sizes(rect.width(), aspect);
                for _ in 0..zoom.unsigned_abs() {
                    self.thumb_size = gallery::step_fit(self.thumb_size, &fits, zoom > 0);
                }
                self.thumb_size_changed();
            }
        }
        let n = self.files.len();
        let listing = self.listing();
        let order = self.order();
        let (in_favorites, mixed) = (self.in_favorites(), self.mixed());
        let gallery = self.gallery.as_mut()?;
        if n == 0 && self.folders.is_empty() {
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
            } else if self.in_archive() {
                Some(tr!("No images in this archive", "В этом архиве нет изображений").into())
            } else if !self.name_filter.trim().is_empty() && self.listed_count() > 0 {
                Some(tr!("No images match the filter", "Нет изображений, подходящих под фильтр").into())
            } else if in_favorites {
                Some(tr!(
                    "No favorites yet: S marks the selected image",
                    "Избранного пока нет: клавиша S отмечает выбранное изображение"
                )
                .into())
            } else if self.in_quick_access() {
                Some(tr!(
                    "No pinned folders yet: Pin to Quick Access is in the context menu of a folder",
                    "Закреплённых папок пока нет: пункт «Закрепить на панели быстрого доступа» есть в контекстном меню папки"
                )
                .into())
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
        // No images (sub-folders only): nothing to find out, and 1:1
        // must not be remembered for the folder.
        let aspect = match (self.thumb_aspect, &listing) {
            (Some(a), _) => a,
            (None, Some(listing)) if self.scan.is_none() && n > 0 => {
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
        let sections = mixed && self.by_folder && self.scan.is_none();
        // The sub-folders' section under its header, folded to the header
        // alone; the images then have a header of their own.
        let folded = self.folders_folded;
        let folders_header = if self.folders.is_empty() { 0.0 } else { HEADER };
        let folders_shown = if folded { 0 } else { self.folders.len() };
        let (starts, header) = if sections { (&self.starts[..], HEADER) } else { (&[][..], folders_header) };
        let layout = gallery::Layout::new(rect.width(), frame, folders_shown, n, starts, header, folders_header);
        let cell = layout.cell;
        gallery.page_rows = ((rect.height() / cell.y).floor() as usize).max(1);

        // Follow the current image when it changes (keys, a deletion).
        if self.index.is_some() && self.current != gallery.scrolled_to {
            gallery.scroll.get_or_insert(Scroll::Visible);
        }
        let mut offset = None;
        // The cell with the cursor: a sub-folder's, or the current image's.
        let target = self.folder_focus.filter(|&k| k < self.folders.len() && !folded).map(Cell::Folder).or(self.index.map(Cell::Image));
        if let (Some(scroll), Some(c)) = (gallery.scroll, target) {
            let y = layout.pos(c).y;
            // The header above the first row of a folder comes into view
            // with it.
            let above = match c {
                Cell::Image(i) if layout.sections.iter().any(|s| s.first + layout.columns > i && s.first <= i) => layout.header,
                _ => 0.0,
            };
            let (top, height) = (gallery.top, rect.height());
            let cell_height = match c {
                Cell::Folder(_) => layout.folder_cell.y,
                Cell::Image(_) => cell.y,
            };
            offset = match scroll {
                Scroll::Centre => Some((y - (height - cell_height) / 2.0).max(0.0)),
                Scroll::Visible if y - above < top => Some(y - above),
                Scroll::Visible if y + cell_height > top + height => Some(y + cell_height - height),
                Scroll::Visible => None,
                Scroll::Keep(below) => Some((y - below.clamp(0.0, (height - cell_height).max(0.0))).max(0.0)),
            };
            gallery.scroll = None;
            gallery.scrolled_to = self.current.clone();
        }
        // A frame dragged past the top or bottom edge scrolls the grid.
        if self.selection.band.is_some()
            && let Some(p) = ctx.pointer_latest_pos()
        {
            let beyond = if p.y < rect.top() { p.y - rect.top() } else { (p.y - rect.bottom()).max(0.0) };
            if beyond != 0.0 {
                let bottom = (layout.height - rect.height()).max(0.0);
                offset = Some((offset.unwrap_or(gallery.top) + beyond * BAND_SCROLL).clamp(0.0, bottom));
                ctx.request_repaint();
            }
        }
        // The empty space between and after the cells: its right click
        // offers the order. Made before the cells, which are on top of it;
        // drags anywhere on the grid are its own, for the frame that
        // chooses images (the cells take only clicks).
        let background = ui.interact(rect, ui.id().with("grid_background"), Sense::CLICK | Sense::DRAG);
        // Each folder keeps its own scroll position. Dragging chooses
        // images, it does not scroll, even on a touch screen.
        let source = egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..Default::default() };
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("grid", &listing))
            .auto_shrink([false, false])
            .scroll_source(source);
        if let Some(y) = offset {
            area = area.vertical_scroll_offset(y);
        }

        let fill = self.thumb_fill;
        let frame_px = frame * ppp;
        let mut requests = Vec::new();
        // Each visible cell, and while images are chosen the centre of its
        // circle, which a click alone ticks or unticks.
        let mut cells: Vec<(usize, Response, Option<Pos2>)> = Vec::new();
        let mut folder_cells: Vec<(usize, Response)> = Vec::new();
        // Sub-folders to list for their cells.
        let mut unlisted = Vec::new();
        let mut hovered = None;
        let files = &self.files;
        let folders = &self.folders;
        let folder_focus = self.folder_focus;
        // With a sub-folder's cell under the cursor, the current image is
        // not marked.
        let index = self.index.filter(|_| folder_focus.is_none());
        let (dir, deep) = (self.dir.as_deref(), self.deep);
        // An archive's folders are not opened on their own.
        let archive = self.archive;
        let favorites = &self.favorites;
        let pinned = &self.pinned;
        let selection = &self.selection;
        let band = selection.band.as_ref().map(|b| b.start).zip(ctx.pointer_latest_pos());
        // A folder whose header was double-clicked.
        let mut folder = None;
        // The folders' header was clicked: folded, or unfolded.
        let mut fold_clicked = false;
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.set_height(layout.height);
            let origin = ui.max_rect().min;
            let painter = ui.painter().clone();
            if layout.folders_header > 0.0 && viewport.min.y < layout.folders_header {
                let rect = Rect::from_min_size(origin, vec2(ui.max_rect().width(), layout.folders_header));
                folder_header(&painter, rect, tr!("Folders", "Папки"), folders.len(), Some(folded));
                let tip = if folded { tr!("Show the folders", "Показать папки") } else { tr!("Hide the folders", "Скрыть папки") };
                if ui.interact(rect, ui.id().with("folders_header"), Sense::CLICK).on_hover_text(tip).clicked() {
                    fold_clicked = true;
                }
            }
            if layout.header > 0.0 {
                let first = layout.sections.partition_point(|s| s.y + layout.header <= viewport.min.y);
                for (k, s) in layout.sections.iter().enumerate().skip(first) {
                    if s.y >= viewport.max.y {
                        break;
                    }
                    let rect = Rect::from_min_size(origin + vec2(0.0, s.y), vec2(ui.max_rect().width(), layout.header));
                    let name = match files[s.first].parent() {
                        // The images after the folders' section.
                        _ if !sections => tr!("Images", "Изображения").to_string(),
                        // From anywhere: the whole path.
                        Some(parent) if in_favorites => parent.display().to_string(),
                        _ => folder_name(dir, &files[s.first]),
                    };
                    folder_header(&painter, rect, &name, layout.len(k), None);
                    // A sub-folder's header opens it; the folder shown has
                    // nothing to open.
                    let parent = files[s.first].parent().filter(|_| sections);
                    if let Some(parent) = parent.filter(|p| !archive && dir.is_none_or(|d| !crate::folder::same_path(p, d))) {
                        let response = ui
                            .interact(rect, ui.id().with(("header", k)), Sense::CLICK)
                            .on_hover_text(tr!("Double click: open the folder", "Двойной щелчок: открыть папку"));
                        if crate::input::double_clicked(&response) {
                            folder = Some(parent.to_path_buf());
                        }
                    }
                }
            }
            let folder_frame = gallery::folder_frame(frame);
            for k in layout.visible_folders(viewport.min.y, viewport.max.y) {
                let cell = Rect::from_min_size(origin + layout.folder_pos(k), layout.folder_cell);
                let response = ui.interact(cell, ui.id().with(("folder", k)), Sense::CLICK);
                let ground = if folder_focus == Some(k) {
                    FOLDER_SELECTED
                } else if response.hovered() {
                    FOLDER_HOVER
                } else {
                    FOLDER_CELL
                };
                painter.rect_filled(cell.shrink(1.0), 3.0, ground);
                let path = &folders[k];
                let square = Rect::from_min_size(pos2(cell.center().x - folder_frame.x / 2.0, cell.top() + PAD), folder_frame);
                let preview = gallery.preview(path).cloned();
                if preview.is_none() {
                    unlisted.push(path.clone());
                }
                // No images: a folder's outline instead of empty places.
                if preview.as_ref().is_some_and(|p| p.count == 0) {
                    folder_outline(&painter, square);
                    let name = file_name(path);
                    label(&painter, &name, cell, square.bottom() + 2.0);
                    key_badge(&painter, cell, pinned.key(path));
                    folder_cells.push((k, response));
                    continue;
                }
                let slots = mosaic(&painter, square, ppp);
                // Its first images, cropped to their places.
                let side = gallery::side_needed(slots[0].size() * ppp, true, None);
                for (slot, image) in slots.iter().zip(preview.iter().flat_map(|p| &p.images)) {
                    if gallery.needs(image, side) {
                        requests.push(Request { path: image.clone(), side });
                    }
                    if let Some(t) = gallery.thumb(image)
                        && let Some(texture) = &t.texture
                    {
                        let px = vec2(t.px[0] as f32, t.px[1] as f32);
                        let (rect, uv) = gallery::place_thumb(*slot, px, ppp, true, true);
                        painter.image(texture.id(), rect, uv, Color32::WHITE);
                    }
                }
                let name = file_name(path);
                let name = match &preview {
                    Some(p) if p.count > 0 => format!("{name} · {}", p.count),
                    _ => name,
                };
                label(&painter, &name, cell, square.bottom() + 2.0);
                key_badge(&painter, cell, pinned.key(path));
                folder_cells.push((k, response));
            }
            let visible = layout.visible(viewport.min.y, viewport.max.y);
            for i in visible.clone() {
                let cell = Rect::from_min_size(origin + layout.cell_pos(i), cell);
                let response = ui.interact(cell, ui.id().with(("cell", i)), Sense::CLICK);
                let path = &files[i];
                // Which sub-folder it is in; for a favourite, where it is.
                let response = match dir.filter(|_| deep).and_then(|d| path.strip_prefix(d).ok()) {
                    _ if in_favorites => response.on_hover_text(path.display().to_string()),
                    Some(relative) => response.on_hover_text(relative.to_string_lossy()),
                    None => response,
                };
                // Chosen images in the accent; with none chosen, the current
                // image in grey, as it always was.
                let choosing = !selection.is_empty();
                let chosen = selection.contains(path);
                if chosen {
                    let outline = egui::Stroke::new(2.0, ACCENT);
                    painter.rect(cell.shrink(1.0), 3.0, CELL_CHOSEN, outline, egui::StrokeKind::Inside);
                } else if !choosing && index == Some(i) {
                    painter.rect_filled(cell.shrink(1.0), 3.0, CELL_SELECTED);
                } else if response.hovered() {
                    painter.rect_filled(cell.shrink(1.0), 3.0, CELL_HOVER);
                }
                // Where Shift goes from: inside the accent when chosen.
                if choosing && index == Some(i) {
                    let inset = if chosen { 3.5 } else { 1.0 };
                    painter.rect_stroke(cell.shrink(inset), 2.0, egui::Stroke::new(1.0, CELL_FOCUS), egui::StrokeKind::Inside);
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
                // The image's corners: the star of a favourite goes in the
                // right one, the tick of a chosen image in the left.
                let mut corner = square.right_top();
                let mut left = square.left_top();
                match gallery.thumb(path) {
                    Some(t) => match &t.texture {
                        Some(texture) => {
                            let px = vec2(t.px[0] as f32, t.px[1] as f32);
                            let (rect, uv) = gallery::place_thumb(square, px, ppp, fill, t.shrunk());
                            painter.image(texture.id(), rect, uv, Color32::WHITE);
                            corner = rect.right_top();
                            left = rect.left_top();
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
                if favorites.contains(path) {
                    star(&painter, corner);
                }
                let circle = choosing.then(|| left + vec2(TICK_INSET, TICK_INSET));
                if let Some(c) = circle {
                    tick(&painter, c, chosen);
                    if response.hover_pos().is_some_and(|p| p.distance(c) <= TICK_REACH) {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
                label(&painter, &file_name(path), cell, square.bottom() + 2.0);
                cells.push((i, response, circle));
            }
            if let Some((start, pointer)) = band {
                let r = Rect::from_two_pos(origin + start.to_vec2(), pointer);
                painter.rect(r, 0.0, ACCENT.gamma_multiply(0.15), egui::Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
            }
            visible
        });
        gallery.top = out.state.offset.y;
        gallery.want_previews(unlisted, order);
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
        let modifiers = ctx.input(|i| i.modifiers);
        // A click puts the cursor on a sub-folder, a double click opens it.
        for (k, response) in folder_cells {
            if response.clicked() || response.secondary_clicked() {
                self.folder_focus = Some(k);
                self.selection.clear();
            }
            if crate::input::double_clicked(&response) {
                folder = self.folders.get(k).cloned();
            }
            response.context_menu(|ui| self.folder_menu(ui));
        }
        for (i, response, circle) in cells {
            if response.clicked() || response.secondary_clicked() {
                self.folder_focus = None;
            }
            // The circle of a cell ticks or unticks it, as Ctrl+click does
            // anywhere on the cell.
            let on_circle = circle.zip(response.interact_pointer_pos()).is_some_and(|(c, p)| p.distance(c) <= TICK_REACH);
            if response.clicked() {
                self.click_cell(i, if on_circle { egui::Modifiers::CTRL } else { modifiers });
            }
            if response.secondary_clicked() {
                self.right_click_cell(i);
            }
            // Ctrl and Shift choose, as does the circle; a double click with
            // them opens nothing.
            if crate::input::double_clicked(&response) && !modifiers.ctrl && !modifiers.shift && !on_circle {
                opened = Some(i);
            }
            response.context_menu(|ui| self.cell_menu(ui));
        }
        // A click on the empty space chooses nothing.
        if background.clicked() && !modifiers.ctrl {
            self.selection.clear();
        }
        self.drag_band(&ctx, &background, rect);
        if fold_clicked {
            self.folders_folded = !self.folders_folded;
            self.folder_focus = None;
            if let Some(gallery) = &mut self.gallery {
                gallery.scroll = Some(Scroll::Visible);
            }
        }
        background.context_menu(|ui| {
            ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
            if let Some(dir) = self.pinnable_dir() {
                ui.separator();
                self.pin_item(ui, dir, true);
            }
            if self.in_favorites() {
                ui.separator();
                self.favorites_items(ui);
                ui.separator();
                self.clear_favorites_item(ui);
            }
            if self.in_quick_access() {
                ui.separator();
                self.unpin_all_item(ui);
            }
        });
        // As if chosen in the tree, with its sub-folders still.
        if let Some(dir) = folder {
            self.open_subfolder(&ctx, dir);
        }
        opened
    }

    /// Right click on a sub-folder's cell, which then has the cursor.
    fn folder_menu(&mut self, ui: &mut Ui) {
        self.item(ui, tr!("Open", "Открыть").into(), "Enter", Cmd::OpenFolder, true);
        self.item(ui, tr!("Show in Explorer", "Показать в Проводнике").into(), "", Cmd::ShowInExplorer, true);
        ui.separator();
        self.item(ui, tr!("Rename…", "Переименовать…").into(), "F2", Cmd::Rename, true);
        self.item(ui, tr!("Delete…", "Удалить…").into(), "Delete", Cmd::Delete, true);
        ui.separator();
        ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
        if let Some(dir) = self.focused_folder() {
            ui.separator();
            self.pin_item(ui, dir.clone(), false);
            // A pinned folder's key (Alt+1 to Alt+9).
            if self.pinned.contains(&dir)
                && let Some(key) = crate::tree::key_menu(ui, &self.pinned.entries(), &dir, self.pinned.key(&dir))
            {
                self.set_pin_key(dir, key);
            }
        }
    }

    /// The frame dragged over the grid (from anywhere on it): the images it
    /// touches are chosen, with Ctrl besides those chosen before.
    fn drag_band(&mut self, ctx: &egui::Context, background: &Response, rect: Rect) {
        let Some(top) = self.gallery.as_ref().map(|g| g.top) else { return };
        let to_grid = |p: Pos2| pos2(p.x - rect.min.x, p.y - rect.min.y + top);
        if background.drag_started_by(PointerButton::Primary)
            && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
        {
            let before = if ctx.input(|i| i.modifiers.ctrl) { self.chosen_or_current() } else { Default::default() };
            self.selection.band = Some(crate::selection::Band { start: to_grid(origin), before });
            // The images chosen are the file commands' again.
            self.folder_focus = None;
        }
        let Some(band) = &self.selection.band else { return };
        if !background.dragged() {
            self.selection.band = None;
            return;
        }
        let Some(p) = background.interact_pointer_pos() else { return };
        let Some(gallery) = &self.gallery else { return };
        let cells = gallery.layout.cells_in(Rect::from_two_pos(band.start, to_grid(p)));
        let before = band.before.clone();
        let chosen: Vec<_> = cells.into_iter().filter_map(|i| self.files.get(i).cloned()).collect();
        self.selection.set(&before, chosen);
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
        self.favorite_item(ui, true);
        if self.can_go_to_folder() {
            self.go_to_folder_item(ui);
        }
        if self.in_favorites() {
            self.favorites_items(ui);
        }
        ui.separator();
        self.file_items(ui, true);
        ui.separator();
        ui.menu_button(tr!("Sort", "Сортировка"), |ui| self.sort_menu(ui));
        ui.separator();
        self.delete_item(ui, true);
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
/// A header over a section of the grid: its name, its count and a line;
/// with `fold`, a triangle for the folders' section, pointing right while
/// it is folded.
fn folder_header(painter: &Painter, rect: Rect, name: &str, count: usize, fold: Option<bool>) {
    let y = rect.center().y + 2.0;
    let mut left = rect.left() + PAD + 4.0;
    if let Some(folded) = fold {
        let (c, r) = (pos2(left + 4.0, y), 4.0);
        let points = if folded {
            vec![c + vec2(-r * 0.6, -r), c + vec2(r * 0.8, 0.0), c + vec2(-r * 0.6, r)]
        } else {
            vec![c + vec2(-r, -r * 0.6), c + vec2(r, -r * 0.6), c + vec2(0.0, r * 0.8)]
        };
        painter.add(egui::Shape::convex_polygon(points, TEXT_WEAK, egui::Stroke::NONE));
        left += 16.0;
    }
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

/// The star of a favourite in the top right corner of its thumbnail, on a
/// dark disc so that it shows on any image.
/// A folder's outline in the middle of `frame`, small.
fn folder_outline(painter: &Painter, frame: Rect) {
    let side = (frame.size().min_elem() * 0.35).clamp(16.0, 64.0);
    let stroke = egui::Stroke::new((side / 24.0).max(1.0), FOLDER_LINE);
    let body = Rect::from_center_size(frame.center() + vec2(0.0, side * 0.06), vec2(side, side * 0.72));
    let tab = [
        body.left_top(),
        body.left_top() + vec2(0.0, -side * 0.12),
        body.left_top() + vec2(side * 0.36, -side * 0.12),
        body.left_top() + vec2(side * 0.44, 0.0),
    ];
    painter.add(egui::Shape::line(tab.to_vec(), stroke));
    painter.rect_stroke(body, side * 0.06, stroke, egui::StrokeKind::Middle);
}

/// The places of four pictures in two rows filling `frame`, the frame of
/// an image's thumbnail, drawn empty and returned (on whole pixels).
fn mosaic(painter: &Painter, frame: Rect, ppp: f32) -> [Rect; gallery::PREVIEW_IMAGES] {
    let snap = |v: f32| (v * ppp).round() / ppp;
    let gap = snap(2.0);
    let inner = Rect::from_min_max(pos2(snap(frame.min.x), snap(frame.min.y)), pos2(snap(frame.max.x), snap(frame.max.y)));
    let (w, h) = (snap((inner.width() - gap) / 2.0), snap((inner.height() - gap) / 2.0));
    let slot = |col: f32, row: f32| Rect::from_min_size(inner.min + vec2(col * (w + gap), row * (h + gap)), vec2(w, h));
    let slots = [slot(0.0, 0.0), slot(1.0, 0.0), slot(0.0, 1.0), slot(1.0, 1.0)];
    for s in &slots {
        painter.rect_filled(*s, 1.5, FOLDER_SLOT);
    }
    slots
}

fn star(painter: &Painter, corner: egui::Pos2) {
    let c = corner + vec2(-10.0, 10.0);
    painter.circle_filled(c, 9.0, Color32::from_black_alpha(150));
    super::paint_star(painter, c, 6.5, Some(super::STAR), super::STAR);
}

/// The circle's centre from the image's top left corner, and how near it a
/// click ticks it (a little beyond the circle, as it is small).
const TICK_INSET: f32 = 10.0;
const TICK_REACH: f32 = 11.0;

/// The mark of choosing at `c`, in the top left corner of a thumbnail: a
/// tick on the accent when chosen, otherwise an empty circle; a click on
/// either ticks or unticks the image.
fn tick(painter: &Painter, c: Pos2, chosen: bool) {
    if chosen {
        painter.circle(c, 8.0, ACCENT, egui::Stroke::new(1.5, Color32::WHITE));
        let points = vec![c + vec2(-3.6, 0.2), c + vec2(-1.0, 2.8), c + vec2(3.8, -2.6)];
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.8, Color32::WHITE)));
    } else {
        painter.circle(c, 8.0, Color32::from_black_alpha(90), egui::Stroke::new(1.5, Color32::from_white_alpha(200)));
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

/// A pinned folder's key ("Alt+3") in the top left corner of its cell, if
/// it has one.
fn key_badge(painter: &Painter, cell: Rect, key: Option<u8>) {
    let Some(k) = key else { return };
    let galley = painter.layout_no_wrap(format!("Alt+{k}"), FontId::proportional(11.0), TEXT);
    let size = galley.size() + vec2(8.0, 4.0);
    let badge = Rect::from_min_size(cell.min + vec2(PAD + 1.0, PAD + 1.0), size);
    painter.rect_filled(badge, 3.0, Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 0xc0));
    painter.galley(badge.min + vec2(4.0, 2.0), galley, TEXT);
}

/// A vertical line between groups of controls in the bar.
fn bar_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, BAR_SEPARATOR));
}

/// A square `side` points wide beside the slider: small thumbnails on its
/// left, large on its right.
/// The filter's button: a magnifying glass, drawn like the bar's other
/// icons.
fn magnifier(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::CLICK);
    let colour = if response.hovered() { TEXT } else { SLIDER_ICON };
    let stroke = egui::Stroke::new(1.4, colour);
    let centre = rect.center() + vec2(-1.5, -1.5);
    ui.painter().circle_stroke(centre, 4.5, stroke);
    let from = centre + vec2(3.2, 3.2);
    ui.painter().line_segment([from, from + vec2(3.5, 3.5)], egui::Stroke::new(1.8, colour));
    response
}

fn size_icon(ui: &mut Ui, side: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let square = Rect::from_center_size(rect.center(), vec2(side, side));
    ui.painter().rect_stroke(square, 1.0, egui::Stroke::new(1.2, SLIDER_ICON), egui::StrokeKind::Inside);
}
