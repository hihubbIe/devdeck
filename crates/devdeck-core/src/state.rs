//! Machine-local state (R10.1): what devdeck remembers between runs, as opposed to the
//! configuration a user writes. Lives in `DEVDECK_STATE_DIR`, else
//! `$XDG_STATE_HOME/devdeck`, else `~/.local/state/devdeck`.
//!
//! Layout: `app.json` (active workspace), `workspaces/<id>/layout.json` and
//! `workspaces/<id>/sidebar.json` (activity, env, pins). Every write
//! is atomic (temporary file + rename). Reads never fail the app: a missing file means
//! "nothing saved", a broken one is reported and ignored.

use crate::layout::Layout;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The state folder (`devdeck_sys::dirs::state`).
pub fn state_dir(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    devdeck_sys::dirs::state(env)
}

pub fn default_state_dir() -> PathBuf {
    state_dir(|k| std::env::var(k).ok())
}

/// Pinned keys per list id (`apps`, `deployments`, `ports`). Keys missing from the
/// catalog are kept, just not shown.
pub type Pins = BTreeMap<String, BTreeSet<String>>;

/// The deploys a workspace saw (`deploys.json`): by env, then service, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Deploys {
    pub version: u32,
    pub envs: BTreeMap<String, BTreeMap<String, Vec<crate::cluster::DeployRecord>>>,
}

/// What the sidebar of one workspace remembers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarState {
    pub activity: String,
    pub visible: bool,
    pub env: Option<String>,
    pub pins: Pins,
    /// Open tree rows (folders, apps holding apps) per list id, by tree path.
    pub open: BTreeMap<String, BTreeSet<String>>,
    /// The Cluster list watches every pod of its env (M3, off by default).
    pub watch_pods: bool,
    /// Folded sections (`pinned`, `all`) per list id (R4.7).
    pub collapsed: BTreeMap<String, BTreeSet<String>>,
    /// The groups the user made per list id (R4.8); their open state is in `open`.
    pub groups: BTreeMap<String, crate::list::Groups>,
    /// The env the Cluster list shows when it isn't the workspace's (user, 2026-11-09);
    /// `None`: it follows the workspace's env (synced).
    pub cluster_env: Option<String>,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self { activity: "apps".into(), visible: true, env: None, pins: Pins::new(), open: BTreeMap::new(), watch_pods: false, collapsed: BTreeMap::new(), groups: BTreeMap::new(), cluster_env: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub active_workspace: Option<String>,
    /// Main window size in logical pixels.
    pub main_size: Option<(f32, f32)>,
    /// Whether the main window was maximized when last seen (R9.8); `main_size` stays
    /// its windowed size.
    pub main_maximized: bool,
    /// The sidebar's width in logical pixels at UI scale 1 (R4.9); `None`: the theme's.
    pub sidebar_w: Option<f32>,
}

/// Views' own states by view ref (each view decides what it keeps).
pub type ViewStates = std::collections::BTreeMap<String, serde_json::Value>;

/// Outcome of reading a saved file.
#[derive(Debug, PartialEq)]
pub enum Loaded<T> {
    Missing,
    Ok(T),
    Broken(String),
}

/// Writes `text` to `path` so a crash never leaves a half-written file: a unique
/// temporary file is written and synced, renamed over the target, and the folder synced.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = path.parent().ok_or("path has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("{}: {e}", path.display()))
}

/// Moves a file that couldn't be read aside (`<name>.bad`) so the next save doesn't
/// destroy it.
fn set_aside(path: &Path) {
    let bad = path.with_extension(format!("{}.bad", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    let _ = std::fs::rename(path, bad);
}

fn read(path: &Path) -> Loaded<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Loaded::Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::Missing,
        Err(e) => Loaded::Broken(format!("{}: {e}", path.display())),
    }
}

/// A workspace id usable as a folder name (letters, digits, `-`, `_`).
pub fn safe_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[derive(Clone, Debug)]
pub struct StateStore {
    pub dir: PathBuf,
}

impl StateStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn layout_path(&self, workspace: &str) -> PathBuf {
        self.dir.join("workspaces").join(safe_id(workspace)).join("layout.json")
    }

    pub fn load_layout(&self, workspace: &str) -> Loaded<Layout> {
        let path = self.layout_path(workspace);
        match read(&path) {
            Loaded::Ok(t) => match Layout::from_json(&t) {
                Ok(l) => Loaded::Ok(l),
                Err(e) => {
                    set_aside(&path);
                    Loaded::Broken(format!("{}: {e} (kept as layout.json.bad)", path.display()))
                }
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_layout(&self, workspace: &str, layout: &Layout) -> Result<(), String> {
        write_atomic(&self.layout_path(workspace), &layout.to_json())
    }

    fn sidebar_path(&self, workspace: &str) -> PathBuf {
        self.dir.join("workspaces").join(safe_id(workspace)).join("sidebar.json")
    }

    pub fn load_sidebar(&self, workspace: &str) -> Loaded<SidebarState> {
        let path = self.sidebar_path(workspace);
        match read(&path) {
            Loaded::Ok(t) => match serde_json::from_str(&t) {
                Ok(p) => Loaded::Ok(p),
                Err(e) => {
                    set_aside(&path);
                    Loaded::Broken(format!("{}: {e} (kept as sidebar.json.bad)", path.display()))
                }
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_sidebar(&self, workspace: &str, state: &SidebarState) -> Result<(), String> {
        write_atomic(&self.sidebar_path(workspace), &serde_json::to_string_pretty(state).map_err(|e| e.to_string())?)
    }

    fn book_path(&self, workspace: &str) -> PathBuf {
        self.dir.join("workspaces").join(safe_id(workspace)).join("layouts.json")
    }

    pub fn load_book(&self, workspace: &str) -> Loaded<crate::layout_book::LayoutBook> {
        let path = self.book_path(workspace);
        match read(&path) {
            Loaded::Ok(t) => match crate::layout_book::LayoutBook::from_json(&t) {
                Ok(b) if b.layouts.values().filter_map(|n| n.layout.as_ref()).all(|l| l.validate().is_ok()) => {
                    // Converted (ids given to a book saved by name) or repaired: written
                    // back at once, so the ids it got are the ones pins and groups keep
                    // (a failed write shows on the next save of the book).
                    let json = b.to_json();
                    if json != t {
                        let _ = write_atomic(&path, &json);
                    }
                    Loaded::Ok(b)
                }
                Ok(_) => {
                    set_aside(&path);
                    Loaded::Broken(format!("{}: a saved layout is invalid (kept as layouts.json.bad)", path.display()))
                }
                Err(e) => {
                    set_aside(&path);
                    Loaded::Broken(format!("{}: {e} (kept as layouts.json.bad)", path.display()))
                }
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_book(&self, workspace: &str, book: &crate::layout_book::LayoutBook) -> Result<(), String> {
        write_atomic(&self.book_path(workspace), &book.to_json())
    }

    fn deploys_path(&self, workspace: &str) -> PathBuf {
        self.dir.join("workspaces").join(safe_id(workspace)).join("deploys.json")
    }

    /// The deploys devdeck saw in a workspace (R7.2), the last few per service.
    pub fn load_deploys(&self, workspace: &str) -> Loaded<Deploys> {
        let path = self.deploys_path(workspace);
        match read(&path) {
            Loaded::Ok(t) => match serde_json::from_str(&t) {
                Ok(d) => Loaded::Ok(d),
                Err(e) => {
                    set_aside(&path);
                    Loaded::Broken(format!("{}: {e} (kept as deploys.json.bad)", path.display()))
                }
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_deploys(&self, workspace: &str, deploys: &Deploys) -> Result<(), String> {
        write_atomic(&self.deploys_path(workspace), &serde_json::to_string_pretty(deploys).map_err(|e| e.to_string())?)
    }

    fn views_path(&self, workspace: &str) -> PathBuf {
        self.dir.join("workspaces").join(safe_id(workspace)).join("views.json")
    }

    /// What each view kept of itself (scroll, filters, a run that was running), by view
    /// ref: views come back as they were when the workspace reopens.
    pub fn load_views(&self, workspace: &str) -> Loaded<ViewStates> {
        let path = self.views_path(workspace);
        match read(&path) {
            Loaded::Ok(t) => match serde_json::from_str(&t) {
                Ok(v) => Loaded::Ok(v),
                Err(e) => {
                    set_aside(&path);
                    Loaded::Broken(format!("{}: {e} (kept as views.json.bad)", path.display()))
                }
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_views(&self, workspace: &str, views: &ViewStates) -> Result<(), String> {
        write_atomic(&self.views_path(workspace), &serde_json::to_string_pretty(views).map_err(|e| e.to_string())?)
    }

    pub fn load_app(&self) -> Loaded<AppState> {
        let path = self.dir.join("app.json");
        match read(&path) {
            Loaded::Ok(t) => match serde_json::from_str(&t) {
                Ok(s) => Loaded::Ok(s),
                Err(e) => Loaded::Broken(format!("{}: {e}", path.display())),
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_app(&self, state: &AppState) -> Result<(), String> {
        write_atomic(&self.dir.join("app.json"), &serde_json::to_string_pretty(state).map_err(|e| e.to_string())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ViewRef;

    /// A store in a folder removed with it.
    struct Store {
        store: StateStore,
        _dir: crate::testing::TempDir,
    }

    impl std::ops::Deref for Store {
        type Target = StateStore;
        fn deref(&self) -> &StateStore {
            &self.store
        }
    }

    fn store(tag: &str) -> Store {
        let d = crate::testing::TempDir::new(tag);
        Store { store: StateStore::new(d.to_path_buf()), _dir: d }
    }

    #[test]
    fn layout_round_trip_missing_and_broken() {
        let s = store("layout");
        assert_eq!(s.load_layout("work"), Loaded::Missing);
        let mut l = Layout::new();
        let p = l.main().focused;
        l.open(ViewRef::new("a"), p).unwrap();
        s.save_layout("work", &l).unwrap();
        assert_eq!(s.load_layout("work"), Loaded::Ok(l));
        std::fs::write(s.dir.join("workspaces/work/layout.json"), "{oops").unwrap();
        assert!(matches!(s.load_layout("work"), Loaded::Broken(_)));
        assert!(s.dir.join("workspaces/work/layout.json.bad").exists(), "broken file kept aside");
        assert_eq!(s.load_layout("work"), Loaded::Missing, "and no longer read");
        let leftovers = std::fs::read_dir(s.dir.join("workspaces/work")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).count();
        assert_eq!(leftovers, 0, "no temp file left behind");
    }

    #[test]
    fn deploys_round_trip_and_broken() {
        let s = store("deploys");
        assert_eq!(s.load_deploys("work"), Loaded::Missing);
        let r = crate::cluster::DeployRecord { at_s: 5, from: "main".into(), to: "lan-1".into(), by: "devdeck".into(), outcome: "done".into(), ..Default::default() };
        let d = Deploys { version: 1, envs: BTreeMap::from([("dev33".to_string(), BTreeMap::from([("graphservice".to_string(), vec![r])]))]) };
        s.save_deploys("work", &d).unwrap();
        assert_eq!(s.load_deploys("work"), Loaded::Ok(d));
        std::fs::write(s.dir.join("workspaces/work/deploys.json"), "{").unwrap();
        assert!(matches!(s.load_deploys("work"), Loaded::Broken(_)));
    }

    #[test]
    fn an_old_deploys_file_still_reads() {
        let s = store("deploys-old");
        let old = r#"{"version":1,"envs":{"dev33":{"api":[{"at_s":5,"from":"a","to":"b","by":"seen","outcome":"done"}]}}}"#;
        std::fs::create_dir_all(s.dir.join("workspaces/work")).unwrap();
        std::fs::write(s.dir.join("workspaces/work/deploys.json"), old).unwrap();
        let Loaded::Ok(d) = s.load_deploys("work") else { panic!("read") };
        let r = &d.envs["dev33"]["api"][0];
        assert_eq!((r.revision, r.restarted_at.as_deref()), (None, None), "fields added later default");
    }

    #[test]
    fn sidebar_round_trip_and_broken() {
        let s = store("sidebar");
        assert_eq!(s.load_sidebar("work"), Loaded::Missing);
        let pins: Pins = BTreeMap::from([("apps".to_string(), BTreeSet::from(["entityservice".to_string()]))]);
        let open = BTreeMap::from([("apps".to_string(), BTreeSet::from(["front-end".to_string()]))]);
        let state = SidebarState { activity: "cluster".into(), visible: false, env: Some("dev35".into()), pins, open, watch_pods: true, ..Default::default() };
        s.save_sidebar("work", &state).unwrap();
        assert_eq!(s.load_sidebar("work"), Loaded::Ok(state));
        std::fs::write(s.dir.join("workspaces/work/sidebar.json"), "[").unwrap();
        assert!(matches!(s.load_sidebar("work"), Loaded::Broken(_)));
        assert!(s.dir.join("workspaces/work/sidebar.json.bad").exists());
    }

    #[test]
    fn layout_book_round_trip_and_broken() {
        let s = store("book");
        assert_eq!(s.load_book("work"), Loaded::Missing);
        let mut book = crate::layout_book::LayoutBook::default();
        book.save_as("triage", &Layout::new()).unwrap();
        s.save_book("work", &book).unwrap();
        assert_eq!(s.load_book("work"), Loaded::Ok(book));
        std::fs::write(s.dir.join("workspaces/work/layouts.json"), "{oops").unwrap();
        assert!(matches!(s.load_book("work"), Loaded::Broken(_)));
        assert!(s.dir.join("workspaces/work/layouts.json.bad").exists());
    }

    #[test]
    fn a_book_saved_by_name_keeps_the_ids_it_got_on_its_first_load() {
        let s = store("converted");
        let layout = serde_json::to_value(Layout::new()).unwrap();
        let old = serde_json::json!({ "current": "Triage", "layouts": { "Triage": layout, "default": layout }, "keep": [] });
        std::fs::create_dir_all(s.dir.join("workspaces/work")).unwrap();
        std::fs::write(s.dir.join("workspaces/work/layouts.json"), old.to_string()).unwrap();
        let Loaded::Ok(first) = s.load_book("work") else { panic!("loads") };
        let Loaded::Ok(second) = s.load_book("work") else { panic!("loads again") };
        assert_eq!(first, second, "written back: the same ids every time");
        assert_eq!(first.current_name(), "Triage");
    }

    #[test]
    fn app_state_and_safe_ids() {
        let s = store("app");
        assert_eq!(s.load_app(), Loaded::Missing);
        let state = AppState { active_workspace: Some("personal".into()), main_size: Some((1600., 1000.)), main_maximized: true, sidebar_w: Some(360.) };
        s.save_app(&state).unwrap();
        assert_eq!(s.load_app(), Loaded::Ok(state));
        // Saved before the window's maximized state was kept: windowed.
        std::fs::write(s.dir.join("app.json"), r#"{"active_workspace":"personal","main_size":[1600.0,1000.0]}"#).unwrap();
        assert!(matches!(s.load_app(), Loaded::Ok(AppState { main_maximized: false, .. })));
        assert_eq!(safe_id("../evil name"), "___evil_name");
    }

    #[test]
    fn view_states_round_trip_and_broken() {
        let s = store("views");
        assert_eq!(s.load_views("work"), Loaded::Missing);
        let views: ViewStates = [("run/web/pnpm%3Adev".to_string(), serde_json::json!({"follow": false, "running": true}))].into();
        s.save_views("work", &views).unwrap();
        assert_eq!(s.load_views("work"), Loaded::Ok(views));
        std::fs::write(s.views_path("work"), "{ broken").unwrap();
        assert!(matches!(s.load_views("work"), Loaded::Broken(_)));
        assert_eq!(s.load_views("work"), Loaded::Missing, "set aside");
    }
}
