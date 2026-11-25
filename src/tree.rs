//! Sidebar file tree. Each project folder is a top-level node;
//! listings are read lazily and cached, and only expanded folders are flattened into the
//! visible node list. Project exclude patterns hide matching entries.
//!
//! Folders are listed in a background thread, so a slow (network) drive never freezes the
//! window: the tree waits a moment for each listing (a local folder is done by then, so it
//! shows at once) and otherwise marks the folder "loading…" and fills it in when it
//! arrives. Refreshing keeps showing the old listing until the new one is in.
//!
//! A file-system watcher (`FolderWatch`) reports changes made outside FarSub (a
//! terminal, git, a build): the listed folders they touch are listed again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::project::Folder;

pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    /// A project folder (top-level node).
    pub is_root: bool,
    /// Matched by a .gitignore (or inside an ignored folder): shown dimmed.
    pub ignored: bool,
}

pub struct Tree {
    pub roots: Vec<Folder>,
    pub nodes: Vec<Node>,
    pub sel: usize,
    pub scroll: usize,
    /// Set by `reveal`: the sidebar scrolls the selection into view even if its row
    /// number didn't change.
    pub follow: bool,
    pub expanded: HashSet<PathBuf>,
    /// Multi-selection for file operations.
    pub marked: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Entries>,
    /// Listings from before a refresh, shown until the new ones arrive.
    stale: HashMap<PathBuf, Entries>,
    /// Folders being listed in the background.
    loading: HashSet<PathBuf>,
    /// Bumped when listings are thrown away (refresh), so late ones are ignored.
    generation: u64,
    listed_tx: Sender<Listing>,
    listed_rx: Receiver<Listing>,
    /// A `reveal` waiting for a folder on the way to be listed.
    reveal_pending: Option<PathBuf>,
    /// Wakes the window when a background listing arrives.
    pub waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// How long a rebuild waits for listings (all of them together) before showing
    /// "loading…" for the rest.
    pub quick: Duration,
    /// The current rebuild's wait ends here.
    deadline: Option<Instant>,
    /// Test hook: every listing takes at least this long (a slow drive).
    pub list_delay: Duration,
    show_hidden: bool,
    /// Folder open/close animations: folder → (start, opening).
    pub anims: HashMap<PathBuf, (Instant, bool)>,
    /// Animation length; zero disables animations.
    pub anim: Duration,
    /// Folders changed while they were being listed: that listing may predate the
    /// change, so it's shown but listed once more.
    relist: HashSet<PathBuf>,
    /// The folders walked by the last rebuild (the ones whose listings are shown).
    shown: HashSet<PathBuf>,
    /// Reports folders changed on disk (None until the first rebuild, or if the
    /// platform has no watcher).
    watch: Option<FolderWatch>,
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

const ALWAYS_HIDDEN: &[&str] = &[".git", ".DS_Store", ".hg", ".svn"];

/// A folder's entries: (name, is_dir, ignored), folders first.
type Entries = Vec<(String, bool, bool)>;
/// A finished background listing: (generation, folder, entries; None if unreadable).
type Listing = (u64, PathBuf, Option<Entries>);

impl Tree {
    pub fn new() -> Self {
        let (listed_tx, listed_rx) = channel();
        Tree {
            stale: HashMap::new(),
            loading: HashSet::new(),
            generation: 0,
            listed_tx,
            listed_rx,
            reveal_pending: None,
            waker: None,
            // Tests wait longer, so a busy machine doesn't turn local folders "loading".
            quick: Duration::from_millis(if cfg!(test) { 3000 } else { 60 }),
            list_delay: Duration::ZERO,
            deadline: None,
            roots: vec![],
            nodes: vec![],
            sel: 0,
            scroll: 0,
            follow: false,
            expanded: HashSet::new(),
            marked: HashSet::new(),
            cache: HashMap::new(),
            show_hidden: false,
            anims: HashMap::new(),
            anim: Duration::from_millis(160),
            relist: HashSet::new(),
            shown: HashSet::new(),
            watch: None,
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
        self.forget_listings();
        self.stale.clear();
        self.reveal_pending = None;
        self.rebuild();
    }

    /// Drop every listing (and ignore the ones still running).
    fn forget_listings(&mut self) {
        self.generation += 1;
        self.loading.clear();
        self.cache.clear();
        self.relist.clear();
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

    /// A folder's entries: listed already, or listed now in the background (waiting up to
    /// `quick` for it); while it's still loading, the old listing if there is one.
    fn children(&mut self, dir: &Path) -> Entries {
        if let Some(v) = self.cache.get(dir) {
            return v.clone();
        }
        if self.loading.insert(dir.to_path_buf()) {
            let (tx, generation, d) = (self.listed_tx.clone(), self.generation, dir.to_path_buf());
            let (folder, show_hidden) = (self.root_of(dir).cloned(), self.show_hidden);
            let (wake, delay) = (self.waker.clone(), self.list_delay);
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let v = list_dir(&d, folder.as_ref(), show_hidden);
                let _ = tx.send((generation, d, v));
                if let Some(w) = wake {
                    w();
                }
            });
            let until = self.deadline.unwrap_or_else(|| Instant::now() + self.quick);
            while !self.cache.contains_key(dir) {
                let left = until.saturating_duration_since(Instant::now());
                match self.listed_rx.recv_timeout(left) {
                    Ok(r) => self.store(r),
                    Err(_) => break,
                }
            }
            if let Some(v) = self.cache.get(dir) {
                return v.clone();
            }
        }
        self.stale.get(dir).cloned().unwrap_or_default()
    }

    /// Keep a finished listing (unless it's from before a refresh).
    fn store(&mut self, (generation, dir, v): Listing) {
        if generation != self.generation {
            return;
        }
        self.loading.remove(&dir);
        if self.relist.remove(&dir) {
            // Started before the folder changed: show it, but list it again.
            if let Some(v) = v {
                self.stale.insert(dir, v);
            }
            return;
        }
        self.stale.remove(&dir);
        if v.is_none() && !self.is_root(&dir) {
            // Gone (or unreadable): don't keep it open.
            self.expanded.remove(&dir);
        }
        self.cache.insert(dir, v.unwrap_or_default());
    }

    /// Take in listings that finished in the background. True if the tree changed.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        while let Ok(r) = self.listed_rx.try_recv() {
            got |= r.0 == self.generation;
            self.store(r);
        }
        let changed: Vec<PathBuf> = self.watch.as_ref().map_or(vec![], |w| w.changed());
        if !changed.is_empty() && self.disk_changed(&changed) {
            got = true;
        }
        if got {
            self.rebuild();
            if let Some(p) = self.reveal_pending.take() {
                self.reveal(&p);
            }
        }
        got
    }

    /// The folder is open but its listing hasn't arrived yet.
    pub fn is_loading(&self, dir: &Path) -> bool {
        self.loading.contains(dir) && !self.cache.contains_key(dir)
    }

    /// Files listed so far under a project folder, relative to it (Goto Anything for a
    /// folder with `index = false`).
    pub fn known_files(&self, root: &Path) -> Vec<String> {
        let mut out: Vec<String> = (self.cache.iter().chain(&self.stale))
            .filter(|(d, _)| d.starts_with(root))
            .flat_map(|(d, v)| {
                v.iter().filter(|e| !e.1).filter_map(move |e| {
                    let rel = d.join(&e.0);
                    let rel = rel.strip_prefix(root).ok()?;
                    Some(rel.to_string_lossy().replace('\\', "/"))
                })
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn rebuild(&mut self) {
        // Many open folders on a slow drive wait once, not once each.
        let outer = self.deadline.is_none();
        if outer {
            self.deadline = Some(Instant::now() + self.quick);
        }
        self.rebuild_nodes();
        if outer {
            self.deadline = None;
        }
    }

    /// Paths changed on disk: list the shown folders they're in (or are) again, keeping
    /// the old entries on screen meanwhile. True if any of them is shown.
    fn disk_changed(&mut self, paths: &[PathBuf]) -> bool {
        let mut dirs = HashSet::new();
        for p in paths {
            let parent = p.parent().map(Path::to_path_buf);
            if p.file_name().is_some_and(|n| n == ".gitignore")
                && parent.as_ref().is_some_and(|d| self.shown.contains(d))
            {
                // What's dimmed can change anywhere below it.
                self.refresh();
                return true;
            }
            for d in parent.into_iter().chain([p.clone()]) {
                if self.shown.contains(&d) {
                    dirs.insert(d);
                }
            }
        }
        if dirs.is_empty() {
            return false;
        }
        for d in dirs {
            if self.loading.contains(&d) {
                self.relist.insert(d);
            } else if let Some(v) = self.cache.remove(&d) {
                self.stale.insert(d, v);
            }
        }
        self.rebuild();
        true
    }

    /// Point the watcher at what's shown: the project folders (recursively, where that's
    /// one cheap watch) or each open folder.
    fn sync_watch(&mut self) {
        if self.watch.is_none() {
            self.watch = FolderWatch::start(self.waker.clone());
        }
        let Some(w) = &self.watch else { return };
        let want: HashSet<PathBuf> = if FolderWatch::RECURSIVE {
            self.roots.iter().map(|r| r.path.clone()).collect()
        } else {
            self.shown.clone()
        };
        w.set(want);
    }

    fn rebuild_nodes(&mut self) {
        let keep = self.selected_path().map(Path::to_path_buf);
        self.shown.clear();
        let mut out = vec![];
        for r in self.roots.clone() {
            out.push(Node {
                name: r.name.clone(),
                path: r.path.clone(),
                is_dir: true,
                depth: 0,
                is_root: true,
                ignored: false,
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
        self.sync_watch();
    }

    fn walk(&mut self, dir: &Path, depth: usize, out: &mut Vec<Node>) {
        let parent_ignored = out
            .iter()
            .rev()
            .find(|n| n.path == dir)
            .is_some_and(|n| n.ignored);
        self.shown.insert(dir.to_path_buf());
        let kids = self.children(dir);
        for (name, is_dir, ignored) in kids {
            let path = dir.join(&name);
            let open = is_dir && self.expanded.contains(&path);
            out.push(Node {
                name,
                path: path.clone(),
                is_dir,
                depth,
                is_root: false,
                ignored: ignored || parent_ignored,
            });
            if open {
                self.walk(&path, depth + 1, out);
            }
        }
    }

    /// Re-read directories from disk (after saves and file operations).
    /// Folders that turn out to be gone close when their listing fails (no check per
    /// open folder here, which would wait on a slow drive).
    pub fn refresh(&mut self) {
        self.anims.clear();
        let old = std::mem::take(&mut self.cache);
        self.forget_listings();
        self.stale.extend(old);
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
        self.reveal_pending = None;
        if let Some(i) = self.nodes.iter().position(|n| n.path == path) {
            self.sel = i;
            self.follow = true;
        } else if path
            .ancestors()
            .skip(1)
            .take_while(|a| a.starts_with(&root))
            .any(|a| self.is_loading(a))
        {
            // A folder on the way is still being listed: finish when it arrives.
            self.reveal_pending = Some(path.to_path_buf());
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

    /// A slow drive: folders show "loading…" instead of freezing, fill in when listed, keep
    /// their old entries while refreshing, and a reveal waits for the folders on the way.
    #[test]
    fn slow_listings_load_in_the_background() {
        let d = tmp("slow");
        std::fs::create_dir_all(d.join("src/deep")).unwrap();
        std::fs::write(d.join("a.txt"), "").unwrap();
        std::fs::write(d.join("src/deep/x.rs"), "").unwrap();
        let mut t = Tree::new();
        t.anim = Duration::ZERO;
        t.list_delay = Duration::from_millis(300);
        t.quick = Duration::from_millis(20);
        let poll_until = |t: &mut Tree, f: &dyn Fn(&Tree) -> bool| {
            let end = Instant::now() + Duration::from_secs(5);
            while !f(t) && Instant::now() < end {
                t.poll();
                std::thread::sleep(Duration::from_millis(10));
            }
            f(t)
        };
        let started = Instant::now();
        t.set_roots(vec![Folder::plain(d.clone())], false);
        assert!(started.elapsed() < Duration::from_millis(200), "waited for the drive");
        assert_eq!(t.nodes.len(), 1, "only the root while it lists");
        assert!(t.is_loading(&d));
        assert!(poll_until(&mut t, &|t| t.nodes.len() == 3), "src and a.txt arrive");
        assert!(!t.is_loading(&d));
        // Reveal through two folders that aren't listed yet.
        let x = d.join("src/deep/x.rs");
        t.reveal(&x);
        assert!(poll_until(&mut t, &|t| t.selected_path() == Some(x.as_path())));
        // Refreshing keeps the rows while the new listings load.
        let rows = t.nodes.len();
        t.refresh();
        assert_eq!(t.nodes.len(), rows);
        assert!(t.is_loading(&d));
        assert!(poll_until(&mut t, &|t| !t.is_loading(&d)));
        assert_eq!(t.nodes.len(), rows);
        // What's listed is known to Goto Anything for unindexed folders.
        assert_eq!(t.known_files(&d), ["a.txt", "src/deep/x.rs"]);
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

    /// Files created, renamed and deleted by another program show up without a refresh,
    /// in open folders; a closed folder isn't listed.
    #[test]
    fn changes_on_disk_show_up() {
        let d = tmp("watch");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("closed")).unwrap();
        let mut t = Tree::new();
        t.anim = Duration::ZERO;
        t.set_roots(vec![Folder::plain(d.clone())], false);
        t.sel = t.nodes.iter().position(|n| n.name == "src").unwrap();
        t.expand();
        let wait_for = |t: &mut Tree, what: &str, ok: &dyn Fn(&[String]) -> bool| {
            let until = Instant::now() + Duration::from_secs(10);
            while !ok(&names(t)) {
                assert!(Instant::now() < until, "{what}: {:?}", names(t));
                std::thread::sleep(Duration::from_millis(20));
                t.poll();
            }
        };
        // Watches start on their own thread; give them a moment before changing things.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(d.join("new.txt"), "").unwrap();
        std::fs::write(d.join("src/lib.rs"), "").unwrap();
        wait_for(&mut t, "created", &|n| n.iter().any(|n| n == "  new.txt") && n.iter().any(|n| n == "    lib.rs"));
        std::fs::rename(d.join("new.txt"), d.join("renamed.txt")).unwrap();
        wait_for(&mut t, "renamed", &|n| n.iter().any(|n| n == "  renamed.txt") && !n.iter().any(|n| n == "  new.txt"));
        std::fs::remove_file(d.join("src/lib.rs")).unwrap();
        wait_for(&mut t, "deleted", &|n| !n.iter().any(|n| n == "    lib.rs"));
        assert!(!t.shown.contains(&d.join("closed")), "a closed folder isn't listed");
    }

    /// A change to a folder that's still being listed lists it once more: the listing
    /// that was running may be from before the change.
    #[test]
    fn a_change_while_listing_lists_again() {
        let d = tmp("relist");
        let mut t = Tree::new();
        t.anim = Duration::ZERO;
        t.watch = None;
        t.set_roots(vec![Folder::plain(d.clone())], false);
        t.quick = Duration::ZERO;
        t.list_delay = Duration::from_millis(200);
        t.refresh(); // listing in the background now
        assert!(t.is_loading(&d));
        std::fs::write(d.join("late.txt"), "").unwrap();
        assert!(t.disk_changed(&[d.join("late.txt")]));
        t.list_delay = Duration::ZERO;
        let until = Instant::now() + Duration::from_secs(5);
        while !names(&t).iter().any(|n| n == "  late.txt") {
            assert!(Instant::now() < until, "{:?}", names(&t));
            std::thread::sleep(Duration::from_millis(20));
            t.poll();
        }
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
                index: None,
                lsp_disabled: vec![],
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

/// List one folder (in a background thread). None if it can't be read.
fn list_dir(dir: &Path, folder: Option<&Folder>, show_hidden: bool) -> Option<Entries> {
    let rd = std::fs::read_dir(dir).ok()?;
    let ignores = Ignores::for_dir(dir);
    let mut v: Entries = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if ALWAYS_HIDDEN.contains(&name.as_str())
                || name.ends_with(".farsub-workspace")
                || (!show_hidden && name.starts_with('.'))
            {
                return None;
            }
            // The listing itself says what each entry is: no extra request per entry,
            // which is what makes network drives slow. Only symlinks are followed, so
            // linked folders expand.
            let is_dir = match e.file_type() {
                Ok(t) if t.is_symlink() => std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()),
                Ok(t) => t.is_dir(),
                Err(_) => false,
            };
            if folder.is_some_and(|f| f.excluded(&name, is_dir)) {
                return None;
            }
            let ignored = ignores.ignored(&e.path(), is_dir);
            Some((name, is_dir, ignored))
        })
        .collect();
    v.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    Some(v)
}

/// The .gitignore files (and .git/info/exclude) that apply to a folder's entries: in it
/// and its parents, git repository or not. Hidden files are judged by them only, not by
/// being hidden.
struct Ignores(Vec<ignore::gitignore::Gitignore>);

impl Ignores {
    fn for_dir(dir: &Path) -> Ignores {
        let mut v = vec![];
        // Deepest first: the nearest file decides.
        for a in dir.ancestors() {
            let mut b = ignore::gitignore::GitignoreBuilder::new(a);
            let mut any = false;
            for f in [a.join(".gitignore"), a.join(".git").join("info").join("exclude")] {
                if f.is_file() {
                    any = true;
                    let _ = b.add(&f);
                }
            }
            if any && let Ok(g) = b.build() {
                v.push(g);
            }
        }
        Ignores(v)
    }

    fn ignored(&self, path: &Path, is_dir: bool) -> bool {
        use ignore::Match;
        for g in &self.0 {
            match g.matched(path, is_dir) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }
}

/// Watches folders for changes made outside FarSub and reports the changed paths in
/// batches. Watching (which can wait on a slow drive) happens on a thread of its own, and
/// changes are gathered for a moment so a build or a checkout lists each folder once.
/// Folders that can't be watched (network drives, the system's watch limit) are simply
/// not watched: Sidebar: Refresh and regaining focus still catch their changes.
struct FolderWatch {
    want_tx: Sender<HashSet<PathBuf>>,
    sent: std::cell::RefCell<HashSet<PathBuf>>,
    changed_rx: Receiver<Vec<PathBuf>>,
}

impl FolderWatch {
    /// One recursive watch per project folder is cheap with macOS' FSEvents (which also
    /// allows only a few watches per process) and Windows; inotify would add a watch per
    /// subfolder (`target/`, `node_modules/`), so elsewhere each open folder is watched.
    const RECURSIVE: bool = cfg!(any(target_os = "macos", windows));
    /// How long changes are gathered before the sidebar hears of them.
    const GATHER: Duration = Duration::from_millis(150);

    fn start(wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Option<FolderWatch> {
        use notify::{EventKind, RecursiveMode, Watcher, event::ModifyKind};
        let (event_tx, event_rx) = channel::<PathBuf>();
        let mut watcher = notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
            let Ok(e) = r else { return };
            // A file's contents (or reading it) don't change a listing; a .gitignore's do.
            let content = matches!(e.kind, EventKind::Access(_) | EventKind::Modify(ModifyKind::Data(_)));
            for p in e.paths {
                if !content || p.file_name().is_some_and(|n| n == ".gitignore") {
                    let _ = event_tx.send(p);
                }
            }
        })
        .ok()?;
        let (want_tx, want_rx) = channel::<HashSet<PathBuf>>();
        let (changed_tx, changed_rx) = channel();
        std::thread::spawn(move || {
            let mode = if Self::RECURSIVE { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            let mut watched: HashSet<PathBuf> = HashSet::new();
            // Ends when the tree (and its sender) is gone.
            while let Ok(mut want) = want_rx.recv() {
                while let Ok(w) = want_rx.try_recv() {
                    want = w;
                }
                for p in watched.difference(&want) {
                    let _ = watcher.unwatch(p);
                }
                watched.retain(|p| want.contains(p));
                for p in want {
                    if !watched.contains(&p) && watcher.watch(&p, mode).is_ok() {
                        watched.insert(p);
                    }
                }
            }
        });
        std::thread::spawn(move || {
            // Ends when the watcher (and its event sender) is dropped.
            while let Ok(first) = event_rx.recv() {
                let mut batch = vec![first];
                let until = Instant::now() + Self::GATHER;
                while let Ok(p) = event_rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                    batch.push(p);
                }
                batch.sort();
                batch.dedup();
                if changed_tx.send(batch).is_err() {
                    break;
                }
                if let Some(w) = &wake {
                    w();
                }
            }
        });
        Some(FolderWatch { want_tx, sent: Default::default(), changed_rx })
    }

    /// Watch these folders (and stop watching the rest).
    fn set(&self, want: HashSet<PathBuf>) {
        if *self.sent.borrow() != want {
            let _ = self.want_tx.send(want.clone());
            *self.sent.borrow_mut() = want;
        }
    }

    /// Paths changed since the last call.
    fn changed(&self) -> Vec<PathBuf> {
        self.changed_rx.try_iter().flatten().collect()
    }
}
