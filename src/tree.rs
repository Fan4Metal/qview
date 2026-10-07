//! The folder tree of the gallery: Quick Access with the pinned folders
//! under it, the favourites, the user's Pictures and Desktop and the drives
//! at the top, each folder listed on a thread the first time it is expanded
//! (a network or a sleeping drive can take seconds). Only the visible rows
//! are laid out. A folder's context menu pins it to Quick Access, or unpins
//! it, and gives a pinned folder its key (Alt+1 to Alt+9).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;

use egui::{Button, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

use crate::folder::same_path;
use crate::ui::{TEXT, TEXT_WEAK};

const ROW_HEIGHT: f32 = 22.0;
const INDENT: f32 = 14.0;
const ARROW_WIDTH: f32 = 16.0;
const ICON_WIDTH: f32 = 22.0;
/// Folders with more sub-folders than this are not looked into to find
/// out which of those have sub-folders (their arrows show until expanded).
const PEEK_LIMIT: usize = 500;

const SELECTED: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
const HOVER: Color32 = Color32::from_rgb(0x36, 0x36, 0x36);
const FOLDER: Color32 = Color32::from_rgb(0xdc, 0xb4, 0x5a);
const FOLDER_BACK: Color32 = Color32::from_rgb(0xb8, 0x92, 0x40);
const DRIVE: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xa8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Folder,
    Drive,
    /// The favourite images, listed in place of a folder (`favorites::DIR`).
    Favorites,
    /// Quick Access: the pinned folders are its children, and its grid
    /// (`favorites::PINNED_DIR`) shows them as cells.
    Pinned,
}

impl Kind {
    /// A list of qview's, not a folder: never listed, no context menu.
    fn is_virtual(self) -> bool {
        matches!(self, Kind::Favorites | Kind::Pinned)
    }
}

struct Node {
    path: PathBuf,
    /// As shown: the file name, or Explorer's name of a top-level folder.
    name: String,
    depth: u16,
    kind: Kind,
    /// A pinned folder's key (Alt+1 to Alt+9), under Quick Access.
    key: Option<u8>,
    /// Sub-folders, once listed.
    children: Option<Vec<usize>>,
    /// Whether it has sub-folders, while it is not listed; `None` when
    /// not known (the arrow shows).
    has_children: Option<bool>,
    expanded: bool,
    listing: bool,
    /// Added to take the tree down to a folder (`Tree::reveal`) that the
    /// listing does not show (hidden, or outside the top-level folders):
    /// kept when the tree is read again.
    revealed: bool,
}

impl Node {
    fn has_children(&self) -> bool {
        match &self.children {
            Some(c) => !c.is_empty(),
            None => self.has_children != Some(false),
        }
    }
}

/// What a folder's context menu in the tree asks for, taken by the
/// gallery with [`Tree::take_action`].
pub enum Action {
    Pin(PathBuf),
    Unpin(PathBuf),
    /// Give a pinned folder this key, or none.
    SetKey(PathBuf, Option<u8>),
    /// Open a folder shown under Quick Access where it is in the tree.
    ShowInTree(PathBuf),
    ShowInExplorer(PathBuf),
}

enum Message {
    /// The sub-folders of a node, sorted, and which of its children
    /// revealed outside the listing (hidden ones) are still there.
    Listed(usize, Vec<String>, Vec<String>),
    /// Whether each of them has sub-folders, by name.
    Peeked(usize, Vec<(String, bool)>),
    /// Explorer's name of a top-level node.
    Named(usize, String),
}

pub struct Tree {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    /// The visible rows, in order.
    rows: Vec<usize>,
    dirty: bool,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    ctx: egui::Context,
    /// Expand the tree down to this folder as its parents are listed.
    reveal: Option<PathBuf>,
    /// Scroll the row of this node into view on the next frame.
    scroll_to: Option<usize>,
    /// Scroll offset and height of the list in the last frame.
    viewport: (f32, f32),
    /// The pinned folders with their keys, shown under Quick Access.
    pinned: Vec<(PathBuf, Option<u8>)>,
    /// Asked for in a context menu in the last frame.
    action: Option<Action>,
}

impl Tree {
    /// The user's Pictures and Desktop, then the drives.
    pub fn new(ctx: egui::Context) -> Self {
        let mut tree = Self::with_roots(ctx, Vec::new());
        let added = tree.set_roots(system_roots());
        tree.name_roots(added);
        tree
    }

    /// Explorer's names of top-level nodes ("Изображения", "Media (H:)"),
    /// on a thread: a drive that is asleep or gone can take a while to
    /// answer.
    fn name_roots(&self, ids: Vec<usize>) {
        if ids.is_empty() {
            return;
        }
        let roots: Vec<(usize, PathBuf)> = ids
            .into_iter()
            .filter(|&r| !self.nodes[r].kind.is_virtual())
            .map(|r| (r, self.nodes[r].path.clone()))
            .collect();
        if roots.is_empty() {
            return;
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::Builder::new()
            .name("drive names".into())
            .spawn(move || {
                let _com = crate::win::com_init();
                for (id, path) in roots {
                    if let Some(name) = crate::win::display_name(&path) {
                        let _ = tx.send(Message::Named(id, name));
                        ctx.request_repaint();
                    }
                }
            })
            .expect("spawn drive names thread");
    }

    fn with_roots(ctx: egui::Context, roots: Vec<(PathBuf, Kind)>) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut tree = Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            rows: Vec::new(),
            dirty: true,
            tx,
            rx,
            ctx,
            reveal: None,
            scroll_to: None,
            viewport: (0.0, 0.0),
            pinned: Vec::new(),
            action: None,
        };
        tree.set_roots(roots);
        tree
    }

    /// Make `roots` the top-level nodes, those already there kept as they
    /// are (expanded, listed), and the ones revealed outside them too (a
    /// network share). Returns the nodes added.
    fn set_roots(&mut self, roots: Vec<(PathBuf, Kind)>) -> Vec<usize> {
        let mut ids = Vec::with_capacity(roots.len());
        let mut added = Vec::new();
        for (path, kind) in roots {
            match self.roots.iter().copied().find(|&r| same_path(&self.nodes[r].path, &path)) {
                Some(r) => ids.push(r),
                None => {
                    let name = match kind {
                        Kind::Drive => path.to_string_lossy().trim_end_matches('\\').to_string(),
                        Kind::Folder => {
                            path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into())
                        }
                        // Named when drawn, in the language of the moment.
                        Kind::Favorites | Kind::Pinned => String::new(),
                    };
                    let id = self.add(path, name, 0, kind);
                    if kind.is_virtual() {
                        self.nodes[id].has_children = Some(false);
                    }
                    ids.push(id);
                    added.push(id);
                }
            }
        }
        let revealed: Vec<usize> =
            self.roots.iter().copied().filter(|&r| self.nodes[r].revealed && !ids.contains(&r)).collect();
        ids.extend(revealed);
        self.roots = ids;
        self.dirty = true;
        added
    }

    /// Show `pinned`, each with its key, under Quick Access in this order;
    /// the folders pinned before stay as they are (expanded, listed). Quick
    /// Access is expanded when the first is pinned.
    pub fn set_pinned(&mut self, pinned: &[(PathBuf, Option<u8>)]) {
        self.pinned = pinned.to_vec();
        let Some(root) = self.roots.iter().copied().find(|&r| self.nodes[r].kind == Kind::Pinned) else { return };
        let old = self.nodes[root].children.take().unwrap_or_default();
        let mut children = Vec::with_capacity(pinned.len());
        for (path, key) in pinned {
            let id = match old.iter().copied().find(|&c| same_path(&self.nodes[c].path, path)) {
                Some(c) => c,
                None => {
                    // A drive's root has no name of its own.
                    let (name, kind) = match path.file_name() {
                        Some(n) => (n.to_string_lossy().into_owned(), Kind::Folder),
                        None => (path.to_string_lossy().trim_end_matches('\\').to_string(), Kind::Drive),
                    };
                    self.add(path.clone(), name, 1, kind)
                }
            };
            self.nodes[id].key = *key;
            children.push(id);
        }
        if old.is_empty() && !children.is_empty() {
            self.nodes[root].expanded = true;
        }
        self.nodes[root].children = Some(children);
        self.dirty = true;
    }

    /// What a context menu asked for in the last frame.
    pub fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    fn add(&mut self, path: PathBuf, name: String, depth: u16, kind: Kind) -> usize {
        self.nodes.push(Node {
            path,
            name,
            depth,
            kind,
            key: None,
            children: None,
            has_children: None,
            expanded: false,
            listing: false,
            revealed: false,
        });
        self.nodes.len() - 1
    }

    /// List the sub-folders of nodes `ids` on a thread, one after another,
    /// then find out which of those have sub-folders.
    fn list(&mut self, ids: Vec<usize>) {
        let mut jobs: Vec<(usize, PathBuf, Vec<String>)> = Vec::new();
        for id in ids {
            if self.nodes[id].listing {
                continue;
            }
            // Revealed children are kept only while they are there.
            let revealed: Vec<String> = self.nodes[id]
                .children
                .iter()
                .flatten()
                .filter(|&&c| self.nodes[c].revealed)
                .map(|&c| self.nodes[c].name.clone())
                .collect();
            let node = &mut self.nodes[id];
            node.listing = true;
            jobs.push((id, node.path.clone(), revealed));
        }
        if jobs.is_empty() {
            return;
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::Builder::new()
            .name("folder tree".into())
            .spawn(move || {
                let mut peek = Vec::new();
                for (id, dir, revealed) in jobs {
                    let names = subfolders(&dir);
                    let still: Vec<String> = revealed.into_iter().filter(|n| dir.join(n).is_dir()).collect();
                    if names.len() <= PEEK_LIMIT {
                        peek.push((id, dir, names.clone()));
                    }
                    if tx.send(Message::Listed(id, names, still)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
                for (id, dir, names) in peek {
                    if names.is_empty() {
                        continue;
                    }
                    let flags = names
                        .into_iter()
                        .map(|n| {
                            let has = has_subfolder(&dir.join(&n));
                            (n, has)
                        })
                        .collect();
                    if tx.send(Message::Peeked(id, flags)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn folder tree thread");
    }

    /// Read the tree again (F5), keeping what is expanded: the folders
    /// listed so far (folders made or removed since), the top-level folders
    /// and the drives (one plugged in or removed).
    pub fn refresh(&mut self) {
        let added = self.set_roots(system_roots());
        self.name_roots(added);
        self.relist();
    }

    /// `dir` has been listed elsewhere (the gallery) with the sub-folders
    /// `names`: the nodes of `dir` the tree has listed, if they show other
    /// sub-folders (one deleted, renamed or made since), are listed again.
    pub fn sync(&mut self, dir: &Path, names: &[String]) {
        let wanted: std::collections::HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let stale: Vec<usize> = (0..self.nodes.len())
            .filter(|&id| {
                let node = &self.nodes[id];
                if node.kind.is_virtual() || node.listing || !same_path(&node.path, dir) {
                    return false;
                }
                let Some(children) = &node.children else { return false };
                let shown: std::collections::HashSet<String> =
                    children.iter().map(|&c| self.nodes[c].name.to_lowercase()).collect();
                shown != wanted
            })
            .collect();
        if !stale.is_empty() {
            self.list(stale);
        }
    }

    /// List again the folders listed so far, the visible ones first.
    fn relist(&mut self) {
        let mut ids = Vec::new();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if let Some(children) = &self.nodes[id].children {
                // Quick Access's children are the pinned folders, not a
                // listing.
                if !self.nodes[id].kind.is_virtual() {
                    ids.push(id);
                }
                stack.extend(children.iter().rev());
            }
        }
        let expanded = |id: &usize| self.nodes[*id].expanded;
        let (visible, rest): (Vec<usize>, Vec<usize>) = ids.into_iter().partition(expanded);
        self.list(visible.into_iter().chain(rest).collect());
    }

    fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Listed(id, names, still) => {
                    let (parent, depth) = (self.nodes[id].path.clone(), self.nodes[id].depth + 1);
                    // Listed again: the folders still there are kept as
                    // they are (expanded, listed).
                    let old = self.nodes[id].children.take().unwrap_or_default();
                    let by_name: HashMap<String, usize> =
                        old.iter().map(|&c| (self.nodes[c].name.to_lowercase(), c)).collect();
                    let mut children: Vec<usize> = names
                        .into_iter()
                        .map(|n| match by_name.get(&n.to_lowercase()) {
                            Some(&c) => c,
                            None => self.add(parent.join(&n), n, depth, Kind::Folder),
                        })
                        .collect();
                    // A hidden folder the tree was taken down to stays,
                    // while it is there.
                    let there = |name: &str| still.iter().any(|n| n.to_lowercase() == name.to_lowercase());
                    let kept: Vec<usize> = old
                        .into_iter()
                        .filter(|&c| self.nodes[c].revealed && !children.contains(&c) && there(&self.nodes[c].name))
                        .collect();
                    children.extend(kept);
                    let node = &mut self.nodes[id];
                    node.children = Some(children);
                    node.listing = false;
                }
                Message::Peeked(id, flags) => {
                    let children = self.nodes[id].children.clone().unwrap_or_default();
                    // In the order listed, unless listed again since.
                    for (c, (name, has)) in children.into_iter().zip(flags) {
                        let node = &mut self.nodes[c];
                        if node.name == name && node.children.is_none() {
                            node.has_children = Some(has);
                        }
                    }
                }
                Message::Named(id, name) => self.nodes[id].name = name,
            }
            self.dirty = true;
        }
        self.step_reveal();
    }

    fn toggle(&mut self, id: usize) {
        let node = &mut self.nodes[id];
        node.expanded = !node.expanded;
        if node.expanded && node.children.is_none() {
            self.list(vec![id]);
        }
        self.dirty = true;
    }

    /// Expand the tree down to `dir` and scroll it into view; the parents
    /// that are not listed yet are listed first.
    pub fn reveal(&mut self, dir: &Path) {
        self.reveal = Some(dir.to_path_buf());
        self.step_reveal();
    }

    /// Go down towards `reveal` as far as the listed folders allow.
    fn step_reveal(&mut self) {
        let Some(target) = self.reveal.clone() else { return };
        let parts: Vec<&OsStr> = target.components().map(Component::as_os_str).collect();
        let lower = |s: &OsStr| s.to_string_lossy().to_lowercase();
        let matches = |root: &Path| {
            let r: Vec<&OsStr> = root.components().map(Component::as_os_str).collect();
            (r.len() <= parts.len() && r.iter().zip(&parts).all(|(a, b)| lower(a) == lower(b))).then_some(r.len())
        };
        // The deepest top-level folder that holds it: Pictures before C:.
        let found = self.roots.iter().filter_map(|&r| matches(&self.nodes[r].path).map(|n| (r, n))).max_by_key(|&(_, n)| n);
        let (root, skip) = match found {
            Some(f) => f,
            None => {
                // Another drive or a network share: a top-level row of its own.
                let prefix: PathBuf = target
                    .components()
                    .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
                    .collect();
                if prefix.as_os_str().is_empty() {
                    self.reveal = None;
                    return;
                }
                let n = prefix.components().count();
                let id = self.add(prefix.clone(), prefix.to_string_lossy().trim_end_matches('\\').into(), 0, Kind::Drive);
                self.nodes[id].revealed = true;
                self.roots.push(id);
                (id, n)
            }
        };
        let mut id = root;
        for part in &parts[skip..] {
            let node = &mut self.nodes[id];
            if !node.expanded {
                node.expanded = true;
                self.dirty = true;
            }
            let Some(children) = node.children.clone() else {
                self.list(vec![id]);
                return;
            };
            let name = lower(part);
            id = match children.iter().copied().find(|&c| self.nodes[c].name.to_lowercase() == name) {
                Some(c) => c,
                None => {
                    // Hidden, or made since the parent was listed.
                    let (path, depth) = (self.nodes[id].path.join(part), self.nodes[id].depth + 1);
                    let c = self.add(path, part.to_string_lossy().into(), depth, Kind::Folder);
                    self.nodes[c].revealed = true;
                    self.nodes[id].children.as_mut().expect("listed").push(c);
                    c
                }
            };
        }
        self.reveal = None;
        self.scroll_to = Some(id);
        self.dirty = true;
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            self.rows.push(id);
            let node = &self.nodes[id];
            if node.expanded
                && let Some(children) = &node.children
            {
                stack.extend(children.iter().rev());
            }
        }
        self.dirty = false;
    }

    /// Draw the tree with `selected` highlighted, and how many favourites
    /// there are; returns the folder clicked (`favorites::DIR` for them).
    pub fn show(&mut self, ui: &mut Ui, selected: Option<&Path>, favorites: usize) -> Option<PathBuf> {
        self.poll();
        if self.dirty {
            self.rebuild();
        }
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let (top, height) = self.viewport;
        let offset = self.scroll_to.take().and_then(|id| self.rows.iter().position(|&r| r == id)).and_then(|row| {
            let y = row as f32 * ROW_HEIGHT;
            // Out of view: a third of the way down.
            (y < top || y + ROW_HEIGHT > top + height).then(|| (y - height / 3.0).max(0.0))
        });
        let mut area = egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]);
        if let Some(y) = offset {
            area = area.vertical_scroll_offset(y);
        }
        let mut chosen = None;
        let mut toggled = None;
        let mut action = None;
        let pinned = &self.pinned;
        // The rows under Quick Access: up to the next top-level node.
        let quick = self.rows.iter().position(|&r| self.nodes[r].kind == Kind::Pinned).map(|start| {
            let end = self.rows[start + 1..].iter().position(|&r| self.nodes[r].depth == 0).map_or(self.rows.len(), |k| start + 1 + k);
            start + 1..end
        });
        let out = area.show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
            for row in range {
                let id = self.rows[row];
                let node = &self.nodes[id];
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::CLICK);
                let painter = ui.painter_at(rect);
                let is_selected = selected.is_some_and(|s| same_path(s, &node.path));
                if is_selected {
                    painter.rect_filled(rect, 0.0, SELECTED);
                } else if response.hovered() {
                    painter.rect_filled(rect, 0.0, HOVER);
                }
                let x = rect.left() + 4.0 + node.depth as f32 * INDENT;
                let arrow = Rect::from_min_size(pos2(x, rect.top()), vec2(ARROW_WIDTH, ROW_HEIGHT));
                let pointer = response.hover_pos();
                let on_arrow = pointer.is_some_and(|p| arrow.contains(p));
                let has_children = node.has_children();
                if has_children {
                    paint_arrow(&painter, arrow.center(), node.expanded, if on_arrow { TEXT } else { TEXT_WEAK });
                }
                let icon = pos2(x + ARROW_WIDTH + ICON_WIDTH / 2.0 - 2.0, rect.center().y);
                match node.kind {
                    Kind::Folder => paint_folder(&painter, icon),
                    Kind::Drive => paint_drive(&painter, icon),
                    Kind::Favorites => crate::ui::paint_star(&painter, icon, 7.5, Some(crate::ui::STAR), crate::ui::STAR),
                    Kind::Pinned => paint_pin(&painter, icon),
                }
                let name = match node.kind {
                    Kind::Favorites => tr!("Favorites", "Избранное"),
                    Kind::Pinned => tr!("Quick Access", "Быстрый доступ"),
                    _ => &node.name,
                };
                let text = pos2(x + ARROW_WIDTH + ICON_WIDTH + 2.0, rect.center().y);
                let label = painter.text(text, egui::Align2::LEFT_CENTER, name, FontId::proportional(13.0), TEXT);
                if node.kind == Kind::Favorites && favorites > 0 {
                    let at = pos2(label.right() + 6.0, rect.center().y);
                    painter.text(at, egui::Align2::LEFT_CENTER, favorites.to_string(), FontId::proportional(12.0), TEXT_WEAK);
                }
                // A pinned folder's key, at the right.
                if let Some(k) = node.key {
                    let at = pos2(rect.right() - 6.0, rect.center().y);
                    painter.text(at, egui::Align2::RIGHT_CENTER, format!("Alt+{k}"), FontId::proportional(11.0), TEXT_WEAK);
                }
                // The second click of a double click is a click too; the
                // first one has chosen the folder already.
                if crate::input::double_clicked(&response) && has_children && !on_arrow {
                    toggled = Some(id);
                } else if response.clicked() {
                    if has_children && on_arrow {
                        toggled = Some(id);
                    } else {
                        chosen = Some(node.path.clone());
                    }
                }
                if !node.kind.is_virtual() {
                    let path = node.path.clone();
                    let pinned_as = pinned.iter().find(|(p, _)| same_path(p, &path)).map(|(_, k)| *k);
                    let in_quick = quick.as_ref().is_some_and(|q| q.contains(&row));
                    response.context_menu(|ui| {
                        if in_quick && ui.button(tr!("Show in Tree", "Отобразить в дереве")).clicked() {
                            action = Some(Action::ShowInTree(path.clone()));
                            ui.close();
                        }
                        let (text, asked) = if pinned_as.is_some() {
                            (tr!("Unpin from Quick Access", "Открепить от панели быстрого доступа"), Action::Unpin(path.clone()))
                        } else {
                            (tr!("Pin to Quick Access", "Закрепить на панели быстрого доступа"), Action::Pin(path.clone()))
                        };
                        if ui.button(text).clicked() {
                            action = Some(asked);
                            ui.close();
                        }
                        if let Some(key) = pinned_as
                            && let Some(k) = key_menu(ui, pinned, &path, key)
                        {
                            action = Some(Action::SetKey(path.clone(), k));
                        }
                        if ui.button(tr!("Show in Explorer", "Показать в Проводнике")).clicked() {
                            action = Some(Action::ShowInExplorer(path.clone()));
                            ui.close();
                        }
                    });
                }
            }
        });
        self.action = action;
        self.viewport = (out.state.offset.y, out.inner_rect.height());
        if let Some(id) = toggled {
            self.toggle(id);
        }
        chosen
    }
}

/// The top-level nodes: Quick Access, the favourites, the user's Pictures
/// and Desktop, then the drives.
fn system_roots() -> Vec<(PathBuf, Kind)> {
    let pinned = std::iter::once((PathBuf::from(crate::favorites::PINNED_DIR), Kind::Pinned));
    let favorites = std::iter::once((PathBuf::from(crate::favorites::DIR), Kind::Favorites));
    let folders = crate::win::known_folders().into_iter().map(|p| (p, Kind::Folder));
    let drives = crate::win::drives().into_iter().map(|p| (p, Kind::Drive));
    pinned.chain(favorites).chain(folders).chain(drives).collect()
}

/// The submenu of a pinned folder's key (Alt+1 to Alt+9, or none): the
/// key chosen, `Some(None)` for none, None while nothing is chosen. A key
/// another pinned folder has shows that folder's name; chosen, it is taken
/// from it.
pub fn key_menu(ui: &mut Ui, pinned: &[(PathBuf, Option<u8>)], path: &Path, key: Option<u8>) -> Option<Option<u8>> {
    let mut chosen = None;
    ui.menu_button(tr!("Key", "Клавиша"), |ui| {
        for n in crate::favorites::KEYS {
            let other = pinned.iter().find(|(p, k)| *k == Some(n) && !same_path(p, path)).map(|(p, _)| crate::app::folder_label(p));
            let button = Button::new(format!("Alt+{n}")).shortcut_text(other.unwrap_or_default()).selected(key == Some(n));
            if ui.add(button).clicked() {
                chosen = Some(Some(n));
                ui.close();
            }
        }
        ui.separator();
        if ui.add(Button::new(tr!("None", "Нет")).selected(key.is_none())).clicked() {
            chosen = Some(None);
            ui.close();
        }
    });
    chosen
}

/// The sub-folders of `dir` that Explorer shows, in its order.
fn subfolders(dir: &Path) -> Vec<String> {
    use std::os::windows::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<(Vec<u16>, String)> = entries
        .flatten()
        // On Windows the attributes come with the listing: no extra call.
        .filter(|e| e.metadata().is_ok_and(|m| crate::win::is_visible_folder(m.file_attributes())))
        .map(|e| {
            let name = e.file_name();
            (crate::win::wide(&name), name.to_string_lossy().into_owned())
        })
        .collect();
    names.sort_by(|(a, _), (b, _)| crate::win::logical_cmp(a, b));
    names.into_iter().map(|(_, n)| n).collect()
}

fn has_subfolder(dir: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::read_dir(dir)
        .is_ok_and(|mut it| it.any(|e| e.is_ok_and(|e| e.metadata().is_ok_and(|m| crate::win::is_visible_folder(m.file_attributes())))))
}

/// The expand/collapse triangle.
fn paint_arrow(painter: &egui::Painter, c: Pos2, expanded: bool, color: Color32) {
    let s = 4.0;
    let points = if expanded {
        vec![c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6), c + vec2(0.0, s * 0.8)]
    } else {
        vec![c + vec2(-s * 0.6, -s), c + vec2(s * 0.8, 0.0), c + vec2(-s * 0.6, s)]
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A folder 16 points wide centred on `c`.
fn paint_folder(painter: &egui::Painter, c: Pos2) {
    let tab = Rect::from_min_size(c + vec2(-8.0, -6.0), vec2(7.0, 3.0));
    painter.rect_filled(tab, 1.0, FOLDER_BACK);
    let body = Rect::from_min_size(c + vec2(-8.0, -4.0), vec2(16.0, 11.0));
    painter.rect_filled(body, 1.5, FOLDER);
}

/// A pushpin 16 points high centred on `c`, as Explorer marks Quick Access.
fn paint_pin(painter: &egui::Painter, c: Pos2) {
    let color = Color32::from_rgb(0x6c, 0xa8, 0xe8);
    // The head, the collar and the needle.
    let head = Rect::from_center_size(c + vec2(0.0, -4.0), vec2(7.0, 6.0));
    painter.rect_filled(head, 1.5, color);
    let collar = Rect::from_center_size(c + vec2(0.0, 0.0), vec2(12.0, 3.0));
    painter.rect_filled(collar, 1.0, color);
    painter.line_segment([c + vec2(0.0, 1.5), c + vec2(0.0, 8.0)], Stroke::new(1.5, color));
}

/// A drive 16 points wide centred on `c`.
fn paint_drive(painter: &egui::Painter, c: Pos2) {
    let body = Rect::from_center_size(c + vec2(0.0, 1.0), vec2(16.0, 8.0));
    painter.rect_filled(body, 1.5, DRIVE);
    painter.circle_filled(pos2(body.right() - 3.5, body.center().y), 1.3, Color32::from_rgb(0x50, 0xc8, 0x50));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn names(t: &Tree) -> Vec<String> {
        t.rows.iter().map(|&id| t.nodes[id].name.clone()).collect()
    }

    /// Poll until `done` or ten seconds.
    fn pump(t: &mut Tree, done: impl Fn(&Tree) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(t) && Instant::now() < deadline {
            t.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        t.poll();
        t.rebuild();
    }

    #[test]
    fn reveals_lists_and_hides() {
        let root = std::env::temp_dir().join(format!("qview_tree_{}", std::process::id()));
        for d in [r"a\b\c", r"a\b10", r"a\b2", "z", "hidden"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        // A file is no folder.
        std::fs::write(root.join("a").join("f.jpg"), b"x").unwrap();
        let hidden = crate::win::wide(root.join("hidden"));
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(hidden.as_ptr(), 0x2);
        }
        let mut t = Tree::with_roots(egui::Context::default(), vec![(root.clone(), Kind::Folder)]);
        let root_name = t.nodes[0].name.clone();

        t.reveal(&root.join("A").join("b").join("c"));
        pump(&mut t, |t| t.reveal.is_none());
        assert_eq!(names(&t), [&root_name, "a", "b", "c", "b2", "b10", "z"]);
        assert_eq!(t.scroll_to.map(|id| t.nodes[id].name.as_str()), Some("c"));
        // Sub-folders are found out: b has one, z none.
        pump(&mut t, |t| ["b", "z"].iter().all(|n| t.nodes.iter().any(|x| x.name == *n && x.has_children.is_some())));
        let node = |name: &str| t.nodes.iter().find(|n| n.name == name).unwrap();
        assert!(node("b").has_children());
        assert!(!node("z").has_children());

        // Collapsing and expanding again keeps the listing.
        let a = t.rows[1];
        t.toggle(a);
        t.rebuild();
        assert_eq!(names(&t), [&root_name, "a", "z"]);
        t.toggle(a);
        t.rebuild();
        assert_eq!(names(&t).len(), 7);

        // A hidden folder is not listed, but can still be revealed.
        t.reveal(&root.join("hidden"));
        pump(&mut t, |t| t.reveal.is_none());
        assert!(names(&t).contains(&"hidden".to_string()));

        // Read again: a folder made and one removed since are found, what
        // is expanded stays so, and so does the hidden folder revealed.
        std::fs::create_dir_all(root.join("a").join("b3")).unwrap();
        std::fs::remove_dir(root.join("z")).unwrap();
        t.relist();
        pump(&mut t, |t| t.nodes.iter().all(|n| !n.listing));
        assert_eq!(names(&t), [&root_name, "a", "b", "c", "b2", "b3", "b10", "hidden"]);
        // Listed elsewhere with other sub-folders: read again; with the
        // same ones, not.
        std::fs::create_dir_all(root.join("a").join("b4")).unwrap();
        let a = t.nodes.iter().position(|n| n.name == "a").unwrap();
        let listed = |t: &Tree| t.nodes[a].children.iter().flatten().map(|&c| t.nodes[c].name.clone()).collect::<Vec<_>>();
        t.sync(&root.join("A"), &listed(&t));
        assert!(!t.nodes[a].listing);
        t.sync(&root.join("a"), &["b".into(), "b2".into(), "b3".into(), "b4".into(), "b10".into()]);
        assert!(t.nodes[a].listing);
        pump(&mut t, |t| t.nodes.iter().all(|n| !n.listing));
        assert_eq!(names(&t), [&root_name, "a", "b", "c", "b2", "b3", "b4", "b10", "hidden"]);
        std::fs::remove_dir(root.join("a").join("b4")).unwrap();
        // Removed (or renamed) since: read again, it goes.
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(hidden.as_ptr(), 0x80);
        }
        std::fs::remove_dir(root.join("hidden")).unwrap();
        t.relist();
        pump(&mut t, |t| t.nodes.iter().all(|n| !n.listing));
        assert_eq!(names(&t), [&root_name, "a", "b", "c", "b2", "b3", "b10"]);
        // The top-level nodes that are still there are kept.
        let before = t.roots.clone();
        assert!(t.set_roots(vec![(root.clone(), Kind::Folder)]).is_empty());
        assert_eq!(t.roots, before);

        // A folder outside the roots gets a top-level row of its own.
        t.reveal(Path::new(r"\\server\share\photos"));
        assert_eq!(t.roots.len(), 2);
        assert_eq!(t.nodes[t.roots[1]].name, r"\\server\share");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pinned_folders_under_quick_access() {
        let root = std::env::temp_dir().join(format!("qview_tree_pinned_{}", std::process::id()));
        for d in [r"p1\sub", "p2"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let pinned = (PathBuf::from(crate::favorites::PINNED_DIR), Kind::Pinned);
        let favorites = (PathBuf::from(crate::favorites::DIR), Kind::Favorites);
        let mut t = Tree::with_roots(egui::Context::default(), vec![pinned, favorites, (root.clone(), Kind::Folder)]);
        let root_name = t.nodes[2].name.clone();
        t.rebuild();
        // None pinned: Quick Access has no arrow, nor do the favourites.
        assert!(!t.nodes[0].has_children() && !t.nodes[1].has_children());
        // Pinned, in the order given, with their keys: Quick Access
        // expanded to show them.
        t.set_pinned(&[(root.join("p2"), Some(3)), (root.join("p1"), None)]);
        t.rebuild();
        assert_eq!(names(&t), ["", "p2", "p1", "", &root_name]);
        assert_eq!(t.nodes[t.rows[1]].key, Some(3));
        // A pinned folder expands like any other.
        let p1 = t.nodes.iter().position(|n| n.name == "p1").unwrap();
        t.toggle(p1);
        pump(&mut t, |t| t.nodes[p1].children.is_some());
        assert_eq!(names(&t), ["", "p2", "p1", "sub", "", &root_name]);
        // Read again: the pinned folders are no listing of Quick Access.
        t.relist();
        pump(&mut t, |t| t.nodes.iter().all(|n| !n.listing));
        assert_eq!(names(&t), ["", "p2", "p1", "sub", "", &root_name]);
        // One unpinned, the other's key changed: it stays as it was,
        // expanded.
        t.set_pinned(&[(root.join("p1"), Some(1))]);
        t.rebuild();
        assert_eq!(names(&t), ["", "p1", "sub", "", &root_name]);
        assert_eq!(t.nodes.iter().position(|n| n.name == "p1"), Some(p1));
        assert_eq!(t.nodes[p1].key, Some(1));
        t.set_pinned(&[]);
        t.rebuild();
        assert_eq!(names(&t), ["", "", &root_name]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
