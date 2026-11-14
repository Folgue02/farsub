//! Sidebar file tree. Each project folder is a top-level node (like Sublime's FOLDERS);
//! listings are read lazily and cached, and only expanded folders are flattened into the
//! visible node list. Project exclude patterns hide matching entries.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::project::Folder;

pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    /// A project folder (top-level node).
    pub is_root: bool,
}

pub struct Tree {
    pub roots: Vec<Folder>,
    pub nodes: Vec<Node>,
    pub sel: usize,
    pub scroll: usize,
    pub expanded: HashSet<PathBuf>,
    /// Multi-selection for file operations.
    pub marked: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Vec<(String, bool)>>,
    show_hidden: bool,
    /// Folder open/close animations: folder → (start, opening).
    pub anims: HashMap<PathBuf, (Instant, bool)>,
    /// Animation length; zero disables animations.
    pub anim: Duration,
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

const ALWAYS_HIDDEN: &[&str] = &[".git", ".DS_Store", ".hg", ".svn"];

impl Tree {
    pub fn new() -> Self {
        Tree {
            roots: vec![],
            nodes: vec![],
            sel: 0,
            scroll: 0,
            expanded: HashSet::new(),
            marked: HashSet::new(),
            cache: HashMap::new(),
            show_hidden: false,
            anims: HashMap::new(),
            anim: Duration::from_millis(160),
        }
    }

    pub fn set_roots(&mut self, roots: Vec<Folder>, show_hidden: bool) {
        if roots == self.roots && show_hidden == self.show_hidden && !self.nodes.is_empty() {
            return;
        }
        // Newly added project folders start expanded.
        for r in &roots {
            if !self.roots.iter().any(|o| o.path == r.path) {
                self.expanded.insert(r.path.clone());
            }
        }
        if roots
            .iter()
            .all(|r| !self.roots.iter().any(|o| o.path == r.path))
        {
            self.sel = 0;
            self.scroll = 0;
        }
        self.roots = roots;
        self.show_hidden = show_hidden;
        self.cache.clear();
        self.rebuild();
    }

    pub fn is_root(&self, p: &Path) -> bool {
        self.roots.iter().any(|r| r.path == p)
    }

    /// The project folder containing a path (deepest, for nested folders).
    pub fn root_of(&self, p: &Path) -> Option<&Folder> {
        self.roots
            .iter()
            .filter(|r| p.starts_with(&r.path))
            .max_by_key(|r| r.path.components().count())
    }

    fn children(&mut self, dir: &Path) -> &[(String, bool)] {
        let show_hidden = self.show_hidden;
        let folder = self.root_of(dir).cloned();
        self.cache.entry(dir.to_path_buf()).or_insert_with(|| {
            let mut v: Vec<(String, bool)> = std::fs::read_dir(dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter_map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            if ALWAYS_HIDDEN.contains(&name.as_str())
                                || name.ends_with(".farsub-workspace")
                                || (!show_hidden && name.starts_with('.'))
                            {
                                return None;
                            }
                            // Follow symlinks so linked folders expand.
                            let is_dir = std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir());
                            if folder.as_ref().is_some_and(|f| f.excluded(&name, is_dir)) {
                                return None;
                            }
                            Some((name, is_dir))
                        })
                        .collect()
                })
                .unwrap_or_default();
            v.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });
            v
        })
    }

    pub fn rebuild(&mut self) {
        let keep = self.selected_path().map(Path::to_path_buf);
        let mut out = vec![];
        for r in self.roots.clone() {
            out.push(Node {
                name: r.name.clone(),
                path: r.path.clone(),
                is_dir: true,
                depth: 0,
                is_root: true,
            });
            if self.expanded.contains(&r.path) {
                self.walk(&r.path, 1, &mut out);
            }
        }
        self.nodes = out;
        if let Some(k) = keep {
            if let Some(i) = self.nodes.iter().position(|n| n.path == k) {
                self.sel = i;
            }
        }
        self.sel = self.sel.min(self.nodes.len().saturating_sub(1));
    }

    fn walk(&mut self, dir: &Path, depth: usize, out: &mut Vec<Node>) {
        let kids = self.children(dir).to_vec();
        for (name, is_dir) in kids {
            let path = dir.join(&name);
            let open = is_dir && self.expanded.contains(&path);
            out.push(Node {
                name,
                path: path.clone(),
                is_dir,
                depth,
                is_root: false,
            });
            if open {
                self.walk(&path, depth + 1, out);
            }
        }
    }

    /// Re-read directories from disk (after saves and file operations).
    pub fn refresh(&mut self) {
        self.anims.clear();
        self.cache.clear();
        self.expanded.retain(|p| p.is_dir());
        self.marked.retain(|p| p.exists());
        self.rebuild();
    }

    pub fn selected(&self) -> Option<&Node> {
        self.nodes.get(self.sel)
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected().map(|n| n.path.as_path())
    }

    /// Marked entries, or the selected one.
    pub fn targets(&self) -> Vec<PathBuf> {
        if !self.marked.is_empty() {
            let mut v: Vec<PathBuf> = self.marked.iter().cloned().collect();
            v.sort();
            return v;
        }
        self.selected_path()
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_default()
    }

    /// Folder that new files / pasted files go into: the selected folder, or the selected
    /// file's folder, or the root.
    pub fn target_dir(&self) -> PathBuf {
        match self.selected() {
            Some(n) if n.is_dir => n.path.clone(),
            Some(n) => n
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.first_root()),
            None => self.first_root(),
        }
    }

    pub fn first_root(&self) -> PathBuf {
        self.roots
            .first()
            .map(|r| r.path.clone())
            .unwrap_or_default()
    }

    pub fn toggle_mark(&mut self) {
        if let Some(p) = self.selected_path().map(Path::to_path_buf) {
            if !self.marked.remove(&p) {
                self.marked.insert(p);
            }
        }
    }

    pub fn move_sel(&mut self, d: isize) {
        if self.nodes.is_empty() {
            return;
        }
        let n = self.nodes.len() as isize;
        self.sel = (self.sel as isize + d).clamp(0, n - 1) as usize;
    }

    fn is_open(&self, p: &Path) -> bool {
        self.expanded.contains(p) && !self.anims.get(p).is_some_and(|a| !a.1)
    }

    fn open_dir(&mut self, p: PathBuf) {
        self.expanded.insert(p.clone());
        self.rebuild();
        if !self.anim.is_zero() {
            self.anims.insert(p, (Instant::now(), true));
        }
    }

    fn close_dir(&mut self, p: PathBuf) {
        // Keep the selection out of the folder that's closing.
        if let Some(sel) = self.selected_path().map(Path::to_path_buf)
            && sel != p
            && sel.starts_with(&p)
            && let Some(i) = self.nodes.iter().position(|n| n.path == p)
        {
            self.sel = i;
        }
        if self.anim.is_zero() {
            self.expanded.remove(&p);
            self.rebuild();
        } else {
            // Children stay in the list until the animation finishes (see `tick`).
            self.anims.insert(p, (Instant::now(), false));
        }
    }

    pub fn toggle(&mut self) {
        let Some(n) = self.selected() else { return };
        if !n.is_dir {
            return;
        }
        let p = n.path.clone();
        if self.is_open(&p) {
            self.close_dir(p)
        } else {
            self.open_dir(p)
        }
    }

    pub fn expand(&mut self) {
        let Some(n) = self.selected() else { return };
        if !n.is_dir {
            return;
        }
        let p = n.path.clone();
        if self.is_open(&p) {
            // Already open: step into the first child.
            if self
                .nodes
                .get(self.sel + 1)
                .is_some_and(|c| c.path.parent() == Some(&p))
            {
                self.sel += 1;
            }
        } else {
            self.open_dir(p);
        }
    }

    pub fn collapse(&mut self) {
        let Some(n) = self.selected() else { return };
        let p = n.path.clone();
        if n.is_dir && self.is_open(&p) {
            self.close_dir(p);
            return;
        }
        // Otherwise jump to the parent folder.
        if let Some(parent) = p.parent()
            && let Some(i) = self.nodes.iter().position(|n| n.path == parent)
        {
            self.sel = i;
        }
    }

    /// Advance animations; returns true while any are running.
    pub fn tick(&mut self) -> bool {
        let done: Vec<(PathBuf, bool)> = self
            .anims
            .iter()
            .filter(|(_, (t, _))| t.elapsed() >= self.anim)
            .map(|(p, (_, o))| (p.clone(), *o))
            .collect();
        let mut rebuild = false;
        for (p, opening) in done {
            self.anims.remove(&p);
            if !opening {
                self.expanded.remove(&p);
                rebuild = true;
            }
        }
        if rebuild {
            self.rebuild();
        }
        !self.anims.is_empty()
    }

    fn progress(&self, p: &Path) -> Option<f32> {
        let (t, opening) = self.anims.get(p)?;
        let e = ease(t.elapsed().as_secs_f32() / self.anim.as_secs_f32().max(1e-3));
        Some(if *opening { e } else { 1.0 - e })
    }

    /// Visible height fraction of a row (children of an animating folder grow/shrink).
    pub fn row_factor(&self, i: usize) -> f32 {
        let Some(n) = self.nodes.get(i) else {
            return 1.0;
        };
        let mut f = 1.0;
        if n.is_root {
            return 1.0;
        }
        for a in n.path.ancestors().skip(1) {
            if let Some(pr) = self.progress(a) {
                f *= pr;
            }
            if self.is_root(a) {
                break;
            }
        }
        f
    }

    /// Disclosure arrow rotation: 0 = closed (▸), 1 = open (▾).
    pub fn arrow(&self, p: &Path) -> f32 {
        self.progress(p)
            .unwrap_or(if self.expanded.contains(p) { 1.0 } else { 0.0 })
    }

    /// Expand a folder (and its ancestors) without moving the selection.
    pub fn expand_path(&mut self, dir: &Path) {
        let Some(root) = self.root_of(dir).map(|r| r.path.clone()) else {
            return;
        };
        for a in dir.ancestors() {
            self.expanded.insert(a.to_path_buf());
            if a == root {
                break;
            }
        }
    }

    /// Expand every folder between the root and `path`, then select it.
    pub fn reveal(&mut self, path: &Path) {
        let Some(root) = self.root_of(path).map(|r| r.path.clone()) else {
            return;
        };
        let Ok(rel) = path.strip_prefix(&root) else {
            return;
        };
        self.expanded.insert(root.clone());
        let mut cur = root.clone();
        let comps: Vec<_> = rel.components().collect();
        for c in comps.iter().take(comps.len().saturating_sub(1)) {
            cur.push(c);
            self.expanded.insert(cur.clone());
        }
        self.rebuild();
        if let Some(i) = self.nodes.iter().position(|n| n.path == path) {
            self.sel = i;
        }
    }

    /// Type-to-jump: next visible entry whose name starts with `c`.
    #[allow(dead_code)]
    pub fn jump_char(&mut self, c: char) {
        let n = self.nodes.len();
        let c = c.to_lowercase().next().unwrap_or(c);
        for k in 1..=n {
            let i = (self.sel + k) % n;
            if self.nodes[i].name.to_lowercase().starts_with(c) {
                self.sel = i;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::FolderDef;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farsub-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn names(t: &Tree) -> Vec<String> {
        t.nodes
            .iter()
            .map(|n| format!("{}{}", "  ".repeat(n.depth), n.name))
            .collect()
    }

    #[test]
    fn expand_collapse_reveal() {
        let d = tmp("basic");
        std::fs::create_dir_all(d.join("src/deep")).unwrap();
        std::fs::create_dir_all(d.join(".git")).unwrap();
        std::fs::write(d.join("src/deep/x.rs"), "").unwrap();
        std::fs::write(d.join("b.txt"), "").unwrap();
        let mut t = Tree::new();
        t.anim = Duration::ZERO;
        t.set_roots(vec![Folder::plain(d.clone())], false);
        let root = d.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            names(&t),
            [root.clone(), "  src".into(), "  b.txt".into()],
            ".git hidden, dirs first"
        );
        assert!(t.nodes[0].is_root);
        t.sel = 1;
        t.expand();
        assert_eq!(t.nodes.len(), 4);
        t.collapse();
        assert_eq!(t.nodes.len(), 3);
        t.reveal(&d.join("src/deep/x.rs"));
        assert_eq!(t.selected().unwrap().name, "x.rs");
        assert_eq!(t.selected().unwrap().depth, 3);
        t.collapse(); // file -> parent folder
        assert_eq!(t.selected().unwrap().name, "deep");
        t.jump_char('b');
        assert_eq!(t.selected().unwrap().name, "b.txt");
        // Collapsing the root folder hides everything under it.
        t.sel = 0;
        t.collapse();
        assert_eq!(t.nodes.len(), 1);
    }

    #[test]
    fn multiple_folders_and_excludes() {
        let a = tmp("multi-a");
        let b = tmp("multi-b");
        std::fs::create_dir_all(a.join("target")).unwrap();
        std::fs::create_dir_all(a.join("src")).unwrap();
        std::fs::write(a.join("app.min.js"), "").unwrap();
        std::fs::write(a.join("app.js"), "").unwrap();
        std::fs::write(b.join("lib.rs"), "").unwrap();
        let fa = Folder::new(
            a.clone(),
            FolderDef {
                path: ".".into(),
                name: Some("App".into()),
                folder_exclude_patterns: vec!["target".into()],
                file_exclude_patterns: vec!["*.min.js".into()],
            },
        );
        let mut t = Tree::new();
        t.anim = Duration::ZERO;
        t.set_roots(vec![fa, Folder::plain(b.clone())], false);
        let bname = b.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            names(&t),
            [
                "App".to_string(),
                "  src".into(),
                "  app.js".into(),
                bname,
                "  lib.rs".into()
            ]
        );
        assert_eq!(t.root_of(&b.join("lib.rs")).unwrap().path, b);
        t.reveal(&b.join("lib.rs"));
        assert_eq!(t.selected().unwrap().name, "lib.rs");
        assert_eq!(t.target_dir(), b);
    }

    #[test]
    fn folder_animation_grows_and_shrinks() {
        let d = tmp("anim");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/a.rs"), "").unwrap();
        let mut t = Tree::new();
        t.anim = Duration::from_millis(80);
        t.set_roots(vec![Folder::plain(d.clone())], false);
        t.tick();
        t.sel = 1;
        t.expand();
        assert_eq!(t.nodes.len(), 3, "children appear immediately");
        assert!(t.row_factor(2) < 0.6, "but start small");
        assert!(t.arrow(&d.join("src")) < 0.6);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!t.tick(), "animation finished");
        assert_eq!(t.row_factor(2), 1.0);
        assert_eq!(t.arrow(&d.join("src")), 1.0);
        t.sel = 2;
        t.collapse(); // on a file: jump to parent
        t.collapse(); // close the folder
        assert_eq!(t.nodes.len(), 3, "children stay while shrinking");
        assert!(t.tick());
        std::thread::sleep(Duration::from_millis(100));
        t.tick();
        assert_eq!(t.nodes.len(), 2, "removed once closed");
        assert_eq!(t.arrow(&d.join("src")), 0.0);
    }
}
