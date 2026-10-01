//! The folder tree of the gallery: the user's Pictures and Desktop and the
//! drives at the top, each folder listed on a thread the first time it is
//! expanded (a network or a sleeping drive can take seconds). Only the
//! visible rows are laid out, as in disk_flashlight's tree.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;

use egui::{Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

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
}

struct Node {
    path: PathBuf,
    /// As shown: the file name, or Explorer's name of a top-level folder.
    name: String,
    depth: u16,
    kind: Kind,
    /// Sub-folders, once listed.
    children: Option<Vec<usize>>,
    /// Whether it has sub-folders, while it is not listed; `None` when
    /// not known (the arrow shows).
    has_children: Option<bool>,
    expanded: bool,
    listing: bool,
}

impl Node {
    fn has_children(&self) -> bool {
        match &self.children {
            Some(c) => !c.is_empty(),
            None => self.has_children != Some(false),
        }
    }
}

enum Message {
    /// The sub-folders of a node, sorted.
    Listed(usize, Vec<String>),
    /// Whether each of them has sub-folders, in the same order.
    Peeked(usize, Vec<bool>),
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
}

impl Tree {
    /// The user's Pictures and Desktop, then the drives.
    pub fn new(ctx: egui::Context) -> Self {
        let folders = crate::win::known_folders().into_iter().map(|p| (p, Kind::Folder));
        let drives = crate::win::drives().into_iter().map(|p| (p, Kind::Drive));
        let tree = Self::with_roots(ctx, folders.chain(drives).collect());
        // Explorer's names ("Изображения", "Media (H:)"): a drive that is
        // asleep or gone can take a while to answer.
        let roots: Vec<(usize, PathBuf)> = tree.roots.iter().map(|&r| (r, tree.nodes[r].path.clone())).collect();
        let (tx, ctx) = (tree.tx.clone(), tree.ctx.clone());
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
        tree
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
        };
        for (path, kind) in roots {
            let name = match kind {
                Kind::Drive => path.to_string_lossy().trim_end_matches('\\').to_string(),
                Kind::Folder => path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into()),
            };
            let id = tree.add(path, name, 0, kind);
            tree.roots.push(id);
        }
        tree
    }

    fn add(&mut self, path: PathBuf, name: String, depth: u16, kind: Kind) -> usize {
        self.nodes.push(Node { path, name, depth, kind, children: None, has_children: None, expanded: false, listing: false });
        self.nodes.len() - 1
    }

    /// List the sub-folders of node `id` on a thread, then find out which
    /// of them have sub-folders.
    fn list(&mut self, id: usize) {
        let node = &mut self.nodes[id];
        if node.listing {
            return;
        }
        node.listing = true;
        let dir = node.path.clone();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::Builder::new()
            .name("folder tree".into())
            .spawn(move || {
                let names = subfolders(&dir);
                let peek: Vec<PathBuf> =
                    if names.len() <= PEEK_LIMIT { names.iter().map(|n| dir.join(n)).collect() } else { Vec::new() };
                if tx.send(Message::Listed(id, names)).is_err() {
                    return;
                }
                ctx.request_repaint();
                if !peek.is_empty() {
                    let flags = peek.iter().map(|p| has_subfolder(p)).collect();
                    let _ = tx.send(Message::Peeked(id, flags));
                    ctx.request_repaint();
                }
            })
            .expect("spawn folder tree thread");
    }

    fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Listed(id, names) => {
                    let (parent, depth) = (self.nodes[id].path.clone(), self.nodes[id].depth + 1);
                    let children = names.into_iter().map(|n| self.add(parent.join(&n), n, depth, Kind::Folder)).collect();
                    let node = &mut self.nodes[id];
                    node.children = Some(children);
                    node.listing = false;
                }
                Message::Peeked(id, flags) => {
                    let children = self.nodes[id].children.clone().unwrap_or_default();
                    if children.len() == flags.len() {
                        for (c, has) in children.into_iter().zip(flags) {
                            self.nodes[c].has_children.get_or_insert(has);
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
            self.list(id);
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
                self.list(id);
                return;
            };
            let name = lower(part);
            id = match children.iter().copied().find(|&c| self.nodes[c].name.to_lowercase() == name) {
                Some(c) => c,
                None => {
                    // Hidden, or made since the parent was listed.
                    let (path, depth) = (self.nodes[id].path.join(part), self.nodes[id].depth + 1);
                    let c = self.add(path, part.to_string_lossy().into(), depth, Kind::Folder);
                    self.nodes[id].children.as_mut().expect("listed").push(c);
                    c
                }
            };
        }
        self.reveal = None;
        self.scroll_to = Some(id);
        self.dirty = true;
    }

    /// List `dir` again, if it is listed.
    pub fn refresh(&mut self, dir: &Path) {
        if let Some(id) = self.nodes.iter().position(|n| n.children.is_some() && same_path(&n.path, dir)) {
            let node = &mut self.nodes[id];
            node.children = None;
            node.has_children = None;
            if node.expanded {
                self.list(id);
            }
            self.dirty = true;
        }
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

    /// Draw the tree with `selected` highlighted; returns the folder
    /// clicked.
    pub fn show(&mut self, ui: &mut Ui, selected: Option<&Path>) -> Option<PathBuf> {
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
                }
                painter.text(
                    pos2(x + ARROW_WIDTH + ICON_WIDTH + 2.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &node.name,
                    FontId::proportional(13.0),
                    TEXT,
                );
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
            }
        });
        self.viewport = (out.state.offset.y, out.inner_rect.height());
        if let Some(id) = toggled {
            self.toggle(id);
        }
        chosen
    }
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

/// The expand/collapse triangle, as in disk_flashlight.
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

        // A folder outside the roots gets a top-level row of its own.
        t.reveal(Path::new(r"\\server\share\photos"));
        assert_eq!(t.roots.len(), 2);
        assert_eq!(t.nodes[t.roots[1]].name, r"\\server\share");
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(hidden.as_ptr(), 0x80);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
