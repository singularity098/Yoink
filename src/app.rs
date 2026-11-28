//! The window and everything behind it: the list of apps and their status,
//! checking and updating in the background, the plan, the editor and the
//! options. Work that waits on the network or the disk runs on threads and
//! comes back to the UI thread through `post`.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::config::{self, tilde, Settings, DIR_INFO};
use crate::detect;
use crate::forge::{self, Offer, Remote, RepoInfo};
use crate::library::{
    App as LibApp, Channel, Deployment, IconSource, Installed, Kind, Library, Source,
};
use crate::pattern;
use crate::plan::{self, human_size, Pick, PlanContext, Prep, Task};
use crate::secret;
use crate::{
    Apps, AppRow, Cand, Confirm, DepRow, DirRow, EdDep, Editor, IconArt, MainWindow, PickOpt,
    Picker, PlanRow, PlanView, Prefs, Theme,
};

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
    static CURSOR: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((0.0, 0.0)) };
}

/// Runs `f` with the app on the UI thread (from any thread).
pub fn post(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(Box::new(f)));
}

fn with_app(f: Box<dyn FnOnce(&mut App)>) {
    let Some(app) = APP.with(|a| a.borrow().clone()) else {
        return;
    };
    let borrowed = app.try_borrow_mut();
    match borrowed {
        Ok(mut a) => f(&mut a),
        Err(_) => {
            drop(borrowed);
            slint::Timer::single_shot(Duration::from_millis(5), move || with_app(f));
        }
    }
}

/// Registers a callback that runs with the app borrowed (deferred a moment
/// if it is busy, as when a callback fires while another runs).
macro_rules! on {
    ($global:expr, $method:ident, |$a:ident $(, $arg:ident : $t:ty)*| $body:expr) => {{
        $global.$method(move |$($arg: $t),*| {
            with_app(Box::new(move |$a: &mut App| { $body; }));
        });
    }};
}

#[derive(Default, Clone)]
struct DepState {
    remote: Option<Remote>,
    error: Option<String>,
    checking: bool,
    /// What is being done to it now ("Downloading", "Installing").
    working: Option<String>,
    progress: Option<f32>,
}

struct EditorState {
    app: LibApp,
    editing: bool,
    info: Option<RepoInfo>,
    offers: HashMap<String, Result<Offer, String>>,
    selected: usize,
    /// A new icon to save with the app.
    icon: Option<(Vec<u8>, String, IconSource)>,
    fetch_serial: u64,
    icon_serial: u64,
}

struct Pending {
    tasks: Vec<Task>,
    /// Removing these apps whole once applied.
    remove_app: Option<String>,
}

struct PickState {
    queue: VecDeque<Pick>,
    current: Pick,
    checked: Vec<bool>,
    targets: Vec<(String, String)>,
    force: bool,
    ready: Vec<Task>,
}

/// The orders the list can be sorted in: the setting's value, the label.
const SORTS: &[(&str, &str)] = &[
    ("added", "Date added"),
    ("name", "Name"),
    ("updated", "Last updated"),
    ("updates", "Updates first"),
    ("repo", "Repository"),
];

pub struct App {
    ui: slint::Weak<MainWindow>,
    settings: Settings,
    library: Library,
    token: Option<String>,
    token_store: Option<secret::Store>,
    token_note: String,
    /// The token can do more than Yoink needs.
    token_warning: Option<secret::Warning>,
    status: HashMap<(String, String), DepState>,
    rows: Rc<VecModel<AppRow>>,
    /// The library index of each row, in the order the list is sorted.
    order: Vec<usize>,
    icons: HashMap<String, (PathBuf, Image)>,
    editor: Option<EditorState>,
    pending: Option<Pending>,
    picking: Option<PickState>,
    confirm: Option<String>,
    busy: Option<Arc<AtomicBool>>,
    checking: usize,
    /// Yoink itself was updated (from the list): the executable a restart runs.
    restart_into: Option<PathBuf>,
    toast_serial: u64,
    settings_dirty: bool,
}

pub fn run() -> anyhow::Result<()> {
    let settings = Settings::load();
    crate::desktop::select_backend(settings.native_wayland, &settings.dirs);
    let ui = MainWindow::new()?;
    crate::icons::install(&ui);
    plan::clear_staging();

    let (token, token_store) = match secret::load() {
        Some((t, s)) => (Some(t), Some(s)),
        None => (None, None),
    };
    let app = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        settings,
        library: Library::load(),
        token,
        token_store,
        token_note: String::new(),
        token_warning: None,
        status: HashMap::new(),
        rows: Rc::new(VecModel::default()),
        order: Vec::new(),
        icons: HashMap::new(),
        editor: None,
        pending: None,
        picking: None,
        confirm: None,
        busy: None,
        checking: 0,
        restart_into: None,
        toast_serial: 0,
        settings_dirty: false,
    }));
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));
    connect(&ui);
    {
        let mut a = app.borrow_mut();
        a.init_ui();
        a.retire_menu_entry();
        if let Some(t) = a.token.clone() {
            check_token(t);
        }
        if a.settings.check_on_startup {
            a.check_all();
        }
    }
    ui.show()?;
    {
        let a = app.borrow();
        if a.settings.maximized {
            ui.window().set_maximized(true);
            ui.set_window_maximized(true);
        } else {
            ui.window().set_size(slint::LogicalSize::new(
                a.settings.width.max(720.0),
                a.settings.height.max(480.0),
            ));
        }
    }
    slint::run_event_loop()?;
    if let Ok(mut a) = app.try_borrow_mut() {
        a.remember_window();
        a.save_settings();
    }
    APP.with(|a| *a.borrow_mut() = None);
    Ok(())
}

fn connect(ui: &MainWindow) {
    let apps = ui.global::<Apps>();
    on!(apps, on_add, |a| a.open_editor(None));
    on!(apps, on_check_all, |a| a.check_all());
    on!(apps, on_update_all, |a| a.update_all());
    // The list's callbacks give a row; the library is indexed by app.
    on!(apps, on_update, |a, i: i32| a.update_app(a.lib_index(i), false));
    on!(apps, on_force, |a, i: i32| a.update_app(a.lib_index(i), true));
    on!(apps, on_check, |a, i: i32| {
        if let Some(id) = a.library.apps.get(a.lib_index(i)).map(|x| x.id.clone()) {
            a.check_app(&id);
        }
    });
    on!(apps, on_edit, |a, i: i32| a.open_editor(Some(a.lib_index(i))));
    on!(apps, on_remove, |a, i: i32| a.ask_remove(a.lib_index(i)));
    on!(apps, on_set_sort, |a, i: i32| a.set_sort(i));
    on!(apps, on_set_sort_reversed, |a, v: bool| {
        a.settings.sort_reversed = v;
        a.save_settings();
        a.sync_sort();
        a.refresh_rows();
    });
    on!(apps, on_open_page, |a, i: i32| {
        if let Some(app) = a.library.apps.get(a.lib_index(i)) {
            open_url(&app.source.web_url());
        }
    });
    on!(apps, on_restart, |a| a.restart());
    on!(apps, on_cancel_busy, |a| {
        if let Some(c) = &a.busy {
            c.store(true, Ordering::Relaxed);
        }
    });

    let p = ui.global::<Prefs>();
    on!(p, on_set_dir, |a, id: SharedString, path: SharedString| a.set_dir(&id, &path));
    on!(p, on_choose_dir, |a, id: SharedString| a.choose_dir(id.to_string()));
    on!(p, on_save_token, |a, t: SharedString| a.save_token(&t));
    on!(p, on_clear_token, |a| {
        secret::clear();
        a.token = None;
        a.token_store = None;
        a.token_note.clear();
        a.token_warning = None;
        a.sync_prefs();
        a.toast("Token removed", false);
    });
    on!(p, on_dismiss_token_warning, |a| {
        if let Some(t) = &a.token {
            a.settings.token_warning_dismissed = secret::fingerprint(t);
            a.save_settings();
            a.sync_prefs();
        }
    });
    on!(p, on_set_theme, |a, i: i32| a.set_theme(i));
    on!(p, on_set_flag, |a, id: SharedString, v: bool| a.set_flag(&id, v));
    on!(p, on_add_self, |a| a.add_self());
    on!(p, on_set_plasma, |a, v: bool| a.set_plasma(v));

    let e = ui.global::<Editor>();
    on!(e, on_fetch, |a| a.editor_fetch());
    on!(e, on_select_dep, |a, i: i32| a.editor_select(i as usize));
    on!(e, on_add_dep, |a| a.editor_add_dep());
    on!(e, on_remove_dep, |a, i: i32| a.editor_remove_dep(i as usize));
    on!(e, on_load_offer, |a| a.editor_load_offer());
    on!(e, on_pick_cand, |a, i: i32| a.editor_pick(i as usize));
    on!(e, on_set_kind, |a, k: i32| a.editor_set_kind(k as usize));
    on!(e, on_forget_inner, |a| {
        a.editor_commit();
        if let Some(ed) = &mut a.editor {
            if let Some(d) = ed.app.deployments.get_mut(ed.selected) {
                d.inner.clear();
                d.vst2.clear();
            }
        }
        a.editor_show_dep();
    });
    on!(e, on_choose_icon, |a| a.editor_choose_icon());
    on!(e, on_repo_icon, |a| a.editor_repo_icon());
    on!(e, on_save, |a| a.editor_save());
    on!(e, on_cancel, |a| a.close_editor());

    let pv = ui.global::<PlanView>();
    on!(pv, on_apply, |a| a.apply_plan());
    on!(pv, on_cancel, |a| a.cancel_plan());

    let pk = ui.global::<Picker>();
    on!(pk, on_toggle, |a, i: i32| a.picker_toggle(i as usize));
    on!(pk, on_confirm, |a| a.picker_confirm());
    on!(pk, on_cancel, |a| a.picker_cancel());

    let c = ui.global::<Confirm>();
    on!(c, on_remove_files, |a| a.remove_files());
    on!(c, on_forget, |a| a.forget_app());
    on!(c, on_cancel, |a| {
        a.confirm = None;
        if let Some(ui) = a.ui.upgrade() {
            ui.global::<Confirm>().set_shown(false);
        }
    });

    // The window.
    on!(ui, on_start_drag, |a| {
        a.with_winit(|w| {
            let _ = w.drag_window();
        });
        a.release_pointer();
    });
    on!(ui, on_start_resize, |a, dir: SharedString| {
        use slint::winit_030::winit::window::ResizeDirection as D;
        let d = match dir.as_str() {
            "n" => D::North,
            "s" => D::South,
            "e" => D::East,
            "w" => D::West,
            "ne" => D::NorthEast,
            "nw" => D::NorthWest,
            "se" => D::SouthEast,
            _ => D::SouthWest,
        };
        a.with_winit(|w| {
            let _ = w.drag_resize_window(d);
        });
        a.release_pointer();
    });
    on!(ui, on_minimize_window, |a| {
        if let Some(ui) = a.ui.upgrade() {
            ui.window().set_minimized(true);
        }
    });
    on!(ui, on_toggle_maximize, |a| {
        if let Some(ui) = a.ui.upgrade() {
            let m = !ui.window().is_maximized();
            ui.window().set_maximized(m);
            ui.set_window_maximized(m);
        }
    });
    on!(ui, on_close_window, |a| {
        a.remember_window();
        a.save_settings();
        let _ = slint::quit_event_loop();
    });
    {
        use slint::winit_030::winit::event::WindowEvent;
        use slint::winit_030::{EventResult, WinitWindowAccessor};
        ui.window().on_winit_window_event(move |w, e| {
            if let WindowEvent::CursorMoved { position, .. } = e {
                let s = w.scale_factor();
                CURSOR.with(|c| c.set((position.x as f32 / s, position.y as f32 / s)));
            }
            EventResult::Propagate
        });
    }
    ui.window().on_close_requested(|| {
        with_app(Box::new(|a: &mut App| {
            a.remember_window();
            a.save_settings();
        }));
        slint::CloseRequestResponse::HideWindow
    });
    let _ = ui.global::<IconArt>();
}

fn open_url(url: &str) {
    if let Err(e) = std::process::Command::new("xdg-open").arg(url).spawn() {
        log::warn!("could not open {url}: {e}");
    }
}

fn channel_key(c: &Channel) -> String {
    match c {
        Channel::Releases { prerelease } => format!("r:{prerelease}"),
        Channel::Actions { workflow, branch } => format!("a:{workflow}:{branch}"),
    }
}

/// Checks the token in the background, and shows what GitHub said and any
/// warning in Options.
fn check_token(token: String) {
    std::thread::spawn(move || {
        let c = secret::check(&token);
        post(move |a| {
            // Only if it is still the token saved.
            if a.token.as_deref() == Some(token.as_str()) {
                a.token_note = c.note;
                a.token_warning = c.warning;
                a.sync_prefs();
            }
        });
    });
}

/// Starts `exe` on its own. Yoink's files are its own, so a missing
/// executable bit is put back first (a plain copy can lose it); a folder
/// mounted `noexec` still refuses, and says so.
fn spawn_exe(exe: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(m) = std::fs::metadata(exe) {
        let mode = m.permissions().mode();
        if mode & 0o100 == 0 {
            let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(mode | 0o755));
        }
    }
    let mut cmd = std::process::Command::new(exe);
    // Not the old AppImage's mount: the new one sets these itself.
    for var in ["APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
        cmd.env_remove(var);
    }
    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(std::io::Error::new(
            e.kind(),
            format!("{e} (is its folder mounted noexec?)"),
        )),
        Err(e) => Err(e),
    }
}

/// The first line of an error, for a status line.
fn short(e: &anyhow::Error) -> String {
    let s = e.to_string();
    s.lines().next().unwrap_or("").to_string()
}

fn chain(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

impl App {
    fn ui(&self) -> MainWindow {
        self.ui.upgrade().expect("the window")
    }

    fn init_ui(&mut self) {
        let ui = self.ui();
        ui.global::<Apps>().set_rows(ModelRc::from(self.rows.clone()));
        let names: Vec<SharedString> = ui.global::<Theme>().get_names().iter().collect();
        let theme = names
            .iter()
            .position(|n| n.as_str() == self.settings.theme)
            .or_else(|| names.iter().position(|n| n.as_str() == "Iris"))
            .unwrap_or(0);
        ui.global::<Theme>().set_index(theme as i32);
        ui.set_hover_opens(self.settings.hover_opens);
        ui.global::<Apps>().set_sort_names(ModelRc::new(VecModel::from(
            SORTS.iter().map(|s| SharedString::from(s.1)).collect::<Vec<_>>(),
        )));
        self.sync_sort();
        // The synced settings may name another desktop entries folder than
        // this computer's Plasma script adds.
        if let Err(e) = crate::plasma::follow(&self.settings.dirs) {
            log::warn!("could not update the Plasma script: {e:#}");
        }
        self.refresh_rows();
        self.sync_prefs();
    }

    // ------------------------------------------------------------ the list

    fn icon_for(&mut self, app: &LibApp) -> Option<Image> {
        let path = app.icon_path()?;
        if let Some((p, img)) = self.icons.get(&app.id) {
            if *p == path {
                return Some(img.clone());
            }
        }
        let img = crate::icons::smooth(Image::load_from_path(&path).ok()?, self.icon_px());
        self.icons.insert(app.id.clone(), (path, img.clone()));
        Some(img)
    }

    /// The size app icons are drawn at, in physical pixels (the largest
    /// shown: 56 px tile in the editor, the image 84% of it).
    fn icon_px(&self) -> u32 {
        let scale = self.ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0);
        (48.0 * scale.max(1.0)).ceil() as u32
    }

    fn dep_row(&self, app: &LibApp, d: &Deployment) -> DepRow {
        let st = self
            .status
            .get(&(app.id.clone(), d.id.clone()))
            .cloned()
            .unwrap_or_default();
        let installed = d.installed.as_ref().map(|i| i.version.clone()).unwrap_or_default();
        let latest = st.remote.as_ref().map(|r| r.version.clone()).unwrap_or_default();
        let (tone, status) = if let Some(w) = &st.working {
            (3, w.clone())
        } else if st.checking {
            (0, "checking…".to_string())
        } else if let Some(e) = &st.error {
            (4, e.clone())
        } else if let Some(r) = &st.remote {
            match &d.installed {
                None => (5, format!("not installed · {}", human_size(r.size))),
                Some(i) if i.key == r.key => (1, "up to date".into()),
                Some(_) => (2, format!("update · {}", human_size(r.size))),
            }
        } else if d.installed.is_none() {
            (5, "not installed".into())
        } else {
            (0, String::new())
        };
        DepRow {
            label: d.label().into(),
            installed: installed.into(),
            latest: latest.into(),
            status: status.into(),
            tone,
            progress: st.progress.unwrap_or(-1.0),
            note: st.remote.as_ref().map(|r| r.note.clone()).unwrap_or_default().into(),
        }
    }

    fn app_row(&mut self, i: usize) -> AppRow {
        let app = self.library.apps[i].clone();
        let deps: Vec<DepRow> = app.deployments.iter().map(|d| self.dep_row(&app, d)).collect();
        let busy = deps.iter().any(|d| d.tone == 3);
        // The same rule as Update all: something newer upstream, or not
        // installed yet (an error since does not hide it, so it can be
        // retried).
        let updatable = self.is_updatable(&app);
        let icon = self.icon_for(&app);
        AppRow {
            id: app.id.clone().into(),
            name: app.name.clone().into(),
            repo: app.source.label().into(),
            has_icon: icon.is_some(),
            icon: icon.unwrap_or_default(),
            letter: app
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
                .into(),
            deps: ModelRc::new(VecModel::from(deps)),
            busy,
            updatable,
        }
    }

    /// The same rule as Update all: something newer upstream, or not
    /// installed yet.
    fn is_updatable(&self, app: &crate::library::App) -> bool {
        app.deployments.iter().any(|d| {
            d.installed.is_none()
                || self
                    .status
                    .get(&(app.id.clone(), d.id.clone()))
                    .and_then(|s| s.remote.as_ref())
                    .is_some_and(|r| d.installed.as_ref().is_some_and(|i| i.key != r.key))
        })
    }

    /// The library indexes in the order the list shows them.
    fn sorted_order(&self) -> Vec<usize> {
        use std::cmp::Ordering as O;
        let apps = &self.library.apps;
        let name = |i: usize| apps[i].name.to_lowercase();
        let when = |i: usize| {
            apps[i]
                .deployments
                .iter()
                .filter_map(|d| d.installed.as_ref().map(|x| x.when))
                .max()
                .unwrap_or(0)
        };
        let mut order: Vec<usize> = (0..apps.len()).collect();
        let by_name = |a: usize, b: usize| name(a).cmp(&name(b)).then(a.cmp(&b));
        let cmp: Box<dyn Fn(usize, usize) -> O> = match self.settings.sort.as_str() {
            "name" => Box::new(by_name),
            // Newest first.
            "updated" => Box::new(move |a, b| when(b).cmp(&when(a)).then_with(|| by_name(a, b))),
            "updates" => Box::new(move |a, b| {
                self.is_updatable(&apps[b])
                    .cmp(&self.is_updatable(&apps[a]))
                    .then_with(|| by_name(a, b))
            }),
            "repo" => Box::new(move |a, b| {
                apps[a]
                    .source
                    .label()
                    .to_lowercase()
                    .cmp(&apps[b].source.label().to_lowercase())
                    .then_with(|| by_name(a, b))
            }),
            // The order they were added in.
            _ => Box::new(|a: usize, b: usize| a.cmp(&b)),
        };
        order.sort_by(|&a, &b| cmp(a, b));
        if self.settings.sort_reversed {
            order.reverse();
        }
        order
    }

    /// The library index of a row of the list.
    fn lib_index(&self, row: i32) -> usize {
        self.order.get(row as usize).copied().unwrap_or(usize::MAX)
    }

    fn sync_sort(&self) {
        let ui = self.ui();
        let apps = ui.global::<Apps>();
        let i = SORTS.iter().position(|s| s.0 == self.settings.sort).unwrap_or(0);
        apps.set_sort(i as i32);
        apps.set_sort_reversed(self.settings.sort_reversed);
    }

    fn set_sort(&mut self, i: i32) {
        let Some(s) = SORTS.get(i as usize) else { return };
        self.settings.sort = s.0.to_string();
        self.save_settings();
        self.sync_sort();
        self.refresh_rows();
    }

    fn refresh_rows(&mut self) {
        self.order = self.sorted_order();
        let rows: Vec<AppRow> = self.order.clone().into_iter().map(|i| self.app_row(i)).collect();
        self.rows.set_vec(rows);
        self.refresh_summary();
        self.sync_self();
    }

    fn refresh_app(&mut self, id: &str) {
        let Some(i) = self.library.apps.iter().position(|a| a.id == id) else {
            return;
        };
        // A change that moves the app (its update came, a check found
        // one) redraws the list in the new order.
        if self.sorted_order() != self.order {
            self.refresh_rows();
            return;
        }
        let row = self.app_row(i);
        match self.order.iter().position(|&x| x == i) {
            Some(r) if r < self.rows.row_count() => self.rows.set_row_data(r, row),
            _ => self.refresh_rows(),
        }
        self.refresh_summary();
    }

    fn updatable_targets(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for app in &self.library.apps {
            for d in &app.deployments {
                let st = self.status.get(&(app.id.clone(), d.id.clone()));
                let remote = st.and_then(|s| s.remote.as_ref());
                let update = match (&d.installed, remote) {
                    (None, Some(_)) => true,
                    (Some(i), Some(r)) => i.key != r.key,
                    _ => false,
                };
                if update {
                    out.push((app.id.clone(), d.id.clone()));
                }
            }
        }
        out
    }

    fn refresh_summary(&mut self) {
        let ui = self.ui();
        let n = self.updatable_targets().len();
        let apps = ui.global::<Apps>();
        apps.set_updates(n as i32);
        apps.set_checking(self.checking > 0);
        apps.set_busy(self.busy.is_some());
        let count = self.library.apps.len();
        let sub = if count == 0 {
            String::new()
        } else if self.checking > 0 {
            format!("{count} app{} · checking…", if count == 1 { "" } else { "s" })
        } else if n > 0 {
            format!(
                "{count} app{} · {n} update{}",
                if count == 1 { "" } else { "s" },
                if n == 1 { "" } else { "s" }
            )
        } else {
            format!("{count} app{}", if count == 1 { "" } else { "s" })
        };
        ui.set_subtitle(sub.into());
        let (summary, tone) = self.overall_status();
        apps.set_summary(summary.into());
        apps.set_summary_tone(tone);
    }

    /// One status for the whole list, toned like a deployment's (see
    /// DepRow): what is under way first, then updates, errors, not
    /// installed, not checked, and else "Everything up to date".
    fn overall_status(&self) -> (String, i32) {
        let plural = |n: usize, one: &str, many: &str| {
            format!("{n} {}", if n == 1 { one } else { many })
        };
        if self.library.apps.is_empty() {
            return (String::new(), 0);
        }
        let mut counts = [0usize; 6];
        for app in &self.library.apps {
            for d in &app.deployments {
                let tone = self.dep_row(app, d).tone;
                counts[tone.clamp(0, 5) as usize] += 1;
            }
        }
        let [unknown, _ok, updates, working, errors, missing] = counts;
        if working > 0 {
            return (format!("Working on {}…", plural(working, "deployment", "deployments")), 3);
        }
        if self.checking > 0 {
            return ("Checking for updates…".into(), 3);
        }
        let mut parts = Vec::new();
        if updates > 0 {
            parts.push(format!("{} available", plural(updates, "update", "updates")));
        }
        if errors > 0 {
            parts.push(plural(errors, "error", "errors"));
        }
        if missing > 0 {
            parts.push(format!("{missing} not installed"));
        }
        if unknown > 0 && parts.is_empty() {
            parts.push(format!("{unknown} not checked yet"));
        }
        let tone = if updates > 0 {
            2
        } else if errors > 0 {
            4
        } else if missing > 0 {
            5
        } else if unknown > 0 {
            0
        } else {
            return ("Everything up to date".into(), 1);
        };
        (parts.join(" · "), tone)
    }

    fn set_busy_text(&self, text: &str) {
        self.ui().global::<Apps>().set_busy_text(text.into());
    }

    fn toast(&mut self, text: &str, error: bool) {
        let ui = self.ui();
        ui.set_toast(text.into());
        ui.set_toast_error(error);
        self.toast_serial += 1;
        let serial = self.toast_serial;
        let secs = if error { 12 } else { 6 };
        slint::Timer::single_shot(Duration::from_secs(secs), move || {
            with_app(Box::new(move |a: &mut App| {
                if a.toast_serial == serial {
                    a.ui().set_toast("".into());
                }
            }));
        });
    }

    fn save_library(&mut self) {
        if let Err(e) = self.library.save() {
            self.toast(&format!("Could not save the library: {e:#}"), true);
        }
    }

    // ------------------------------------------------------------ checking

    fn check_all(&mut self) {
        let ids: Vec<String> = self.library.apps.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            self.check_app(&id);
        }
    }

    fn check_app(&mut self, id: &str) {
        let Some(app) = self.library.app(id).cloned() else {
            return;
        };
        if app.deployments.is_empty() {
            return;
        }
        for d in &app.deployments {
            let st = self.status.entry((app.id.clone(), d.id.clone())).or_default();
            st.checking = true;
            st.error = None;
        }
        self.checking += 1;
        self.refresh_app(id);
        let token = self.token.clone();
        let allow_failed = self.settings.allow_failed_runs;
        std::thread::spawn(move || {
            let forge = forge::forge(&app.source, token);
            let mut results = Vec::new();
            for d in &app.deployments {
                let r = forge
                    .resolve(&d.channel, &d.pattern, allow_failed)
                    .map_err(|e| short(&e));
                results.push((d.id.clone(), r));
            }
            let icon = if app.icon_auto && app.icon_source != IconSource::Custom {
                forge
                    .icon()
                    .map_err(|e| log::info!("{}: icon not fetched: {e:#}", app.name))
                    .ok()
                    .flatten()
            } else {
                None
            };
            post(move |a| {
                a.checking = a.checking.saturating_sub(1);
                if let Some(icon) = icon {
                    a.follow_icon(&app.id, icon);
                }
                for (dep, r) in results {
                    let st = a.status.entry((app.id.clone(), dep)).or_default();
                    st.checking = false;
                    match r {
                        Ok(remote) => {
                            st.remote = Some(remote);
                            st.error = None;
                        }
                        Err(e) => {
                            st.remote = None;
                            st.error = Some(e);
                        }
                    }
                }
                a.refresh_app(&app.id);
            });
        });
    }

    /// Puts a freshly fetched repository icon in place for an app that
    /// follows the repository's, when it differs from the one it has.
    fn follow_icon(&mut self, id: &str, icon: forge::Icon) {
        let Some(app) = self.library.app_mut(id) else { return };
        if !app.icon_auto || app.icon_source == IconSource::Custom {
            return;
        }
        // An avatar (the repository has no icon) never replaces the
        // package's own icon.
        if icon.source == IconSource::Avatar && app.icon_source == IconSource::AppImage {
            return;
        }
        let same = app
            .icon_path()
            .and_then(|p| std::fs::read(p).ok())
            .is_some_and(|old| old == icon.bytes);
        if same && app.icon_source == icon.source {
            return;
        }
        match save_icon(&app.id, app.icon.as_deref(), &icon.bytes, icon.ext) {
            Ok(file) => {
                app.icon = Some(file);
                app.icon_source = icon.source;
                self.icons.remove(id);
                self.save_library();
            }
            Err(e) => log::warn!("could not save the icon: {e:#}"),
        }
    }

    // ------------------------------------------------------------ updating

    fn update_all(&mut self) {
        let targets = self.updatable_targets();
        if targets.is_empty() {
            self.toast("Nothing to update: check for updates first", false);
            return;
        }
        self.prepare(targets, false);
    }

    fn update_app(&mut self, i: usize, force: bool) {
        let Some(app) = self.library.apps.get(i) else {
            return;
        };
        let all = self.updatable_targets();
        let targets: Vec<(String, String)> = if force {
            app.deployments.iter().map(|d| (app.id.clone(), d.id.clone())).collect()
        } else {
            let mine: Vec<_> = all.into_iter().filter(|(a, _)| *a == app.id).collect();
            if mine.is_empty() {
                // Not checked yet: whatever is not installed.
                app.deployments
                    .iter()
                    .filter(|d| d.installed.is_none())
                    .map(|d| (app.id.clone(), d.id.clone()))
                    .collect()
            } else {
                mine
            }
        };
        if targets.is_empty() {
            self.toast(&format!("{} is up to date", app.name), false);
            return;
        }
        self.prepare(targets, force);
    }

    /// Downloads, checks and unpacks the builds for `targets`, then shows the
    /// plan (or asks what only the user can decide first).
    fn prepare(&mut self, targets: Vec<(String, String)>, force: bool) {
        if self.busy.is_some() {
            self.toast("Already working on something: wait for it to finish", false);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.busy = Some(cancel.clone());
        self.set_busy_text("Preparing…");
        let mut jobs = Vec::new();
        for (app_id, dep_id) in &targets {
            let Some(app) = self.library.app(app_id) else { continue };
            let Some(dep) = app.deployments.iter().find(|d| d.id == *dep_id) else {
                continue;
            };
            let st = self.status.entry((app_id.clone(), dep_id.clone())).or_default();
            st.working = Some("waiting…".into());
            st.error = None;
            jobs.push((app.clone(), dep.clone(), st.remote.clone()));
        }
        for (app_id, _) in &targets {
            self.refresh_app(app_id);
        }
        self.refresh_summary();
        let token = self.token.clone();
        let allow_failed = self.settings.allow_failed_runs;
        let dirs = self.settings.dirs.clone();
        let library = self.library.clone();
        std::thread::spawn(move || {
            let mut ready = Vec::new();
            let mut picks = Vec::new();
            let mut failed = Vec::new();
            for (app, dep, remote) in jobs {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let (app_id, dep_id) = (app.id.clone(), dep.id.clone());
                let set_working = |text: String, progress: Option<f32>| {
                    let (a_id, d_id) = (app_id.clone(), dep_id.clone());
                    post(move |a| {
                        let st = a.status.entry((a_id.clone(), d_id)).or_default();
                        st.working = Some(text.clone());
                        st.progress = progress;
                        a.set_busy_text(&text);
                        a.refresh_app(&a_id);
                    });
                };
                set_working(format!("{}: checking…", app.name), None);
                let forge = forge::forge(&app.source, token.clone());
                let result = (|| -> anyhow::Result<Option<Prep>> {
                    let remote = match remote {
                        Some(r) => r,
                        None => forge.resolve(&dep.channel, &dep.pattern, allow_failed)?,
                    };
                    if !force && dep.installed.as_ref().is_some_and(|i| i.key == remote.key) {
                        return Ok(Some(Prep::UpToDate));
                    }
                    let mut last = Instant::now() - Duration::from_secs(1);
                    let name = app.name.clone();
                    let staged = plan::stage(forge.as_ref(), &remote, &cancel, &mut |done, total| {
                        if last.elapsed() < Duration::from_millis(120) && Some(done) != total {
                            return;
                        }
                        last = Instant::now();
                        let total = total.unwrap_or(remote.size).max(1);
                        let frac = done as f32 / total as f32;
                        set_working(
                            format!("{name}: downloading {}%", (frac * 100.0).round().min(100.0)),
                            Some(frac),
                        );
                    })?;
                    set_working(format!("{}: looking inside…", app.name), None);
                    let ctx = PlanContext {
                        dirs: &dirs,
                        library: &library,
                    };
                    Ok(Some(plan::build(&ctx, &app, &dep, &remote, &staged)?))
                })();
                match result {
                    Ok(Some(Prep::Ready(t))) => ready.push(*t),
                    Ok(Some(Prep::NeedPick(p))) => picks.push(p),
                    Ok(Some(Prep::UpToDate)) | Ok(None) => {}
                    Err(e) => failed.push((app_id.clone(), dep_id.clone(), chain(&e))),
                }
            }
            let cancelled = cancel.load(Ordering::Relaxed);
            post(move |a| a.prepared(targets, force, ready, picks, failed, cancelled));
        });
    }

    fn prepared(
        &mut self,
        targets: Vec<(String, String)>,
        force: bool,
        ready: Vec<Task>,
        picks: Vec<Pick>,
        failed: Vec<(String, String, String)>,
        cancelled: bool,
    ) {
        self.busy = None;
        for (a, d) in &targets {
            if let Some(st) = self.status.get_mut(&(a.clone(), d.clone())) {
                st.working = None;
                st.progress = None;
            }
        }
        for (a, d, e) in &failed {
            let st = self.status.entry((a.clone(), d.clone())).or_default();
            st.error = Some(e.lines().next().unwrap_or("").to_string());
        }
        self.refresh_rows();
        if cancelled {
            for t in &ready {
                plan::discard_staging(t);
            }
            self.toast("Stopped", false);
            return;
        }
        if !failed.is_empty() {
            let names: Vec<String> = failed
                .iter()
                .map(|(a, _, e)| {
                    let name = self.library.app(a).map(|x| x.name.clone()).unwrap_or_default();
                    format!("{name}: {e}")
                })
                .collect();
            self.toast(&names.join("\n"), true);
        }
        let mut queue: VecDeque<Pick> = picks.into();
        if let Some(current) = queue.pop_front() {
            self.show_picker(PickState {
                checked: vec![false; current.options.len()],
                current,
                queue,
                targets,
                force,
                ready,
            });
            return;
        }
        if ready.is_empty() {
            if failed.is_empty() {
                self.toast("Everything is up to date", false);
            }
            return;
        }
        self.show_plan(Pending {
            tasks: ready,
            remove_app: None,
        });
    }

    fn show_plan(&mut self, pending: Pending) {
        let ui = self.ui();
        let pv = ui.global::<PlanView>();
        let mut rows = Vec::new();
        let (mut downloads, mut bytes, mut copies, mut deletes, mut warnings) = (0, 0u64, 0, 0, 0);
        for t in &pending.tasks {
            for r in t.rows() {
                match r.tone {
                    1 => downloads += 1,
                    2 | 3 | 5 | 6 => copies += 1,
                    4 => deletes += 1,
                    8 => warnings += 1,
                    _ => {}
                }
                rows.push(PlanRow {
                    tone: r.tone,
                    verb: r.verb.into(),
                    text: r.text.into(),
                    detail: r.detail.into(),
                });
            }
            for a in &t.actions {
                if let plan::Action::Downloaded { size, .. } = a {
                    bytes += size;
                }
            }
        }
        let removal = pending.remove_app.is_some() || pending.tasks.iter().all(|t| t.removal);
        let mut parts = Vec::new();
        if downloads > 0 {
            parts.push(format!(
                "{downloads} download{} ({}, already fetched to the cache)",
                if downloads == 1 { "" } else { "s" },
                human_size(bytes)
            ));
        }
        if copies > 0 {
            parts.push(format!("{copies} file{} to put in place", if copies == 1 { "" } else { "s" }));
        }
        if deletes > 0 {
            parts.push(format!("{deletes} to delete"));
        }
        if warnings > 0 {
            parts.push(format!("{warnings} warning{}", if warnings == 1 { "" } else { "s" }));
        }
        let summary = format!(
            "{}. Nothing changes until you {}.",
            if parts.is_empty() { "Nothing to do".to_string() } else { parts.join(", ") },
            if removal { "remove" } else { "apply" }
        );
        pv.set_title(
            if removal {
                format!("Remove {}", pending.tasks.first().map(|t| t.app_name.as_str()).unwrap_or(""))
            } else if pending.tasks.len() == 1 {
                format!("Update {}", pending.tasks[0].app_name)
            } else {
                format!("Update {} deployments", pending.tasks.len())
            }
            .into(),
        );
        pv.set_summary(summary.into());
        pv.set_rows(ModelRc::new(VecModel::from(rows)));
        pv.set_danger(removal);
        pv.set_apply_text(if removal { "Remove" } else { "Apply" }.into());
        pv.set_can_apply(!(removal && deletes == 0 && pending.remove_app.is_none()));
        pv.set_applying(false);
        pv.set_shown(true);
        self.pending = Some(pending);
    }

    fn cancel_plan(&mut self) {
        if let Some(p) = self.pending.take() {
            for t in &p.tasks {
                plan::discard_staging(t);
            }
        }
        self.ui().global::<PlanView>().set_shown(false);
    }

    fn stop_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = DIR_INFO
            .iter()
            .filter_map(|d| self.settings.dirs.path(d.0).ok())
            .collect();
        // Never prune above the home folder.
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h));
        }
        v
    }

    fn apply_plan(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        let mut tasks = pending.tasks.clone();
        // Yoink itself last, once everything else is in place.
        tasks.sort_by_key(|t| self.library.app(&t.app_id).is_some_and(|a| a.is_yoink()));
        let cancel = Arc::new(AtomicBool::new(false));
        self.busy = Some(cancel);
        self.ui().global::<PlanView>().set_applying(true);
        self.set_busy_text("Installing…");
        self.refresh_summary();
        let stops = self.stop_dirs();
        for t in &tasks {
            let st = self.status.entry((t.app_id.clone(), t.dep_id.clone())).or_default();
            st.working = Some(if t.removal { "removing…" } else { "installing…" }.into());
        }
        self.refresh_rows();
        std::thread::spawn(move || {
            let mut results = Vec::new();
            for t in tasks {
                let (placed, r) = plan::apply(&t, &stops);
                if r.is_ok() {
                    plan::discard_staging(&t);
                }
                results.push((t, placed, r.map_err(|e| chain(&e))));
            }
            post(move |a| a.applied(results));
        });
    }

    fn applied(&mut self, results: Vec<(Task, Vec<PathBuf>, Result<(), String>)>) {
        self.busy = None;
        let pending = self.pending.take();
        let ui = self.ui();
        ui.global::<PlanView>().set_shown(false);
        ui.global::<PlanView>().set_applying(false);
        let mut ok = 0;
        let mut errors = Vec::new();
        let mut yoink_updated = None;
        for (t, placed, r) in results {
            let key = (t.app_id.clone(), t.dep_id.clone());
            if let Some(st) = self.status.get_mut(&key) {
                st.working = None;
                st.progress = None;
            }
            let Some(app) = self.library.app_mut(&t.app_id) else { continue };
            if t.removal {
                match &r {
                    Ok(()) => {
                        for d in &mut app.deployments {
                            if t.dep_id.is_empty() || d.id == t.dep_id {
                                d.installed = None;
                            }
                        }
                        ok += 1;
                    }
                    Err(e) => errors.push(format!("{}: {e}", app.name)),
                }
                continue;
            }
            let is_yoink = app.is_yoink();
            let Some(dep) = app.deployments.iter_mut().find(|d| d.id == t.dep_id) else {
                continue;
            };
            match r {
                Ok(()) => {
                    if is_yoink && matches!(dep.kind, Kind::AppImage | Kind::Binary) {
                        if let Some(exe) = &t.exe {
                            self.restart_into = Some(exe.clone());
                            yoink_updated = Some(t.to.clone());
                        }
                    }
                    let remote = t.remote.clone();
                    dep.installed = Some(Installed {
                        key: remote.as_ref().map(|r| r.key.clone()).unwrap_or_default(),
                        version: t.to.clone(),
                        files: t.new_files.clone(),
                        when: crate::now(),
                    });
                    ok += 1;
                    // The package's own icon is better than an avatar, even
                    // for an app that follows the repository's icon (only an
                    // icon file found in the repository is kept then).
                    if let Some((bytes, ext, source)) = &t.icon {
                        let keep = match app.icon_source {
                            IconSource::Custom => true,
                            IconSource::Repo => app.icon_auto,
                            _ => false,
                        };
                        if !keep {
                            if let Ok(file) = save_icon(&app.id, app.icon.as_deref(), bytes, ext) {
                                app.icon = Some(file);
                                app.icon_source = *source;
                                self.icons.remove(&t.app_id);
                            }
                        }
                    }
                    if let Some(st) = self.status.get_mut(&key) {
                        st.error = None;
                    }
                }
                Err(e) => {
                    // Keep track of what was put in place, so it is cleaned
                    // up by the next update or a removal.
                    if let Some(inst) = &mut dep.installed {
                        for p in placed {
                            if !inst.files.contains(&p) {
                                inst.files.push(p);
                            }
                        }
                    } else if !placed.is_empty() {
                        dep.installed = Some(Installed {
                            key: String::new(),
                            version: "incomplete".into(),
                            files: placed,
                            when: crate::now(),
                        });
                    }
                    let name = app.name.clone();
                    if let Some(st) = self.status.get_mut(&key) {
                        st.error = Some(e.lines().next().unwrap_or("").to_string());
                    }
                    errors.push(format!("{name}: {e}"));
                }
            }
        }
        let mut removed = None;
        if let Some(id) = pending.and_then(|p| p.remove_app) {
            if errors.is_empty() {
                removed = self.library.app(&id).map(|a| a.name.clone());
                self.drop_app(&id);
            }
        }
        self.save_library();
        self.refresh_rows();
        self.sync_prefs();
        if let Some(name) = removed {
            self.toast(&format!("Removed {name} and its files"), false);
        } else if let (Some(v), true) = (&yoink_updated, errors.is_empty()) {
            self.toast(&format!("Yoink {v} is installed: restart Yoink to use it"), false);
        } else if errors.is_empty() {
            self.toast(&format!("Done: {ok} item{}", if ok == 1 { "" } else { "s" }), false);
        } else {
            self.toast(&errors.join("\n"), true);
        }
    }

    // ------------------------------------------------------------- picking

    fn show_picker(&mut self, state: PickState) {
        let ui = self.ui();
        let pk = ui.global::<Picker>();
        let app = self.library.app(&state.current.app_id).map(|a| a.name.clone()).unwrap_or_default();
        pk.set_title(state.current.title.clone().into());
        pk.set_detail(format!("{}  ({app}, {})", state.current.detail, state.current.version).into());
        pk.set_multi(state.current.multi);
        self.picking = Some(state);
        self.sync_picker();
        pk.set_shown(true);
    }

    fn sync_picker(&self) {
        let Some(s) = &self.picking else { return };
        let opts: Vec<PickOpt> = s
            .current
            .options
            .iter()
            .zip(&s.checked)
            .map(|(o, c)| PickOpt {
                text: o.clone().into(),
                checked: *c,
            })
            .collect();
        self.ui().global::<Picker>().set_options(ModelRc::new(VecModel::from(opts)));
    }

    fn picker_toggle(&mut self, i: usize) {
        let Some(s) = &mut self.picking else { return };
        if i >= s.checked.len() {
            return;
        }
        if s.current.multi {
            s.checked[i] = !s.checked[i];
        } else {
            for (j, c) in s.checked.iter_mut().enumerate() {
                *c = j == i;
            }
        }
        self.sync_picker();
    }

    fn picker_confirm(&mut self) {
        let Some(s) = self.picking.take() else { return };
        let chosen: Vec<&String> = s
            .current
            .options
            .iter()
            .zip(&s.checked)
            .filter(|(_, c)| **c)
            .map(|(o, _)| o)
            .collect();
        if chosen.is_empty() && !s.current.multi {
            self.picking = Some(s);
            self.toast("Pick one first", false);
            return;
        }
        let version = s.current.version.clone();
        let globs: Vec<String> = chosen
            .iter()
            .map(|o| pattern::derive(o, Some(&version)))
            .collect();
        if let Some(app) = self.library.app_mut(&s.current.app_id) {
            if let Some(d) = app.deployments.iter_mut().find(|d| d.id == s.current.dep_id) {
                if s.current.vst2 {
                    if globs.is_empty() {
                        d.formats.vst2 = false;
                    } else {
                        d.vst2 = globs;
                    }
                } else {
                    d.inner = globs.into_iter().next().unwrap_or_default();
                }
            }
        }
        self.save_library();
        self.ui().global::<Picker>().set_shown(false);
        for t in &s.ready {
            plan::discard_staging(t);
        }
        // Prepare again: the downloads are kept in the cache.
        self.prepare(s.targets, s.force);
    }

    fn picker_cancel(&mut self) {
        let Some(s) = self.picking.take() else { return };
        self.ui().global::<Picker>().set_shown(false);
        let mut s = s;
        if let Some(next) = s.queue.pop_front() {
            s.checked = vec![false; next.options.len()];
            s.current = next;
            self.show_picker(s);
            return;
        }
        if !s.ready.is_empty() {
            self.show_plan(Pending {
                tasks: s.ready,
                remove_app: None,
            });
        }
    }

    // ------------------------------------------------------------ removing

    fn ask_remove(&mut self, i: usize) {
        let Some(app) = self.library.apps.get(i) else { return };
        let files: usize = app
            .deployments
            .iter()
            .filter_map(|d| d.installed.as_ref())
            .map(|inst| inst.files.len())
            .sum();
        let ui = self.ui();
        let c = ui.global::<Confirm>();
        c.set_title(format!("Remove {}?", app.name).into());
        c.set_text(
            if files > 0 {
                format!(
                    "Yoink put {files} file{} in place for it. Delete them (you see the list first), or stop managing it and leave them where they are.",
                    if files == 1 { "" } else { "s" }
                )
            } else {
                "Nothing of it is installed.".to_string()
            }
            .into(),
        );
        c.set_has_files(files > 0);
        c.set_shown(true);
        self.confirm = Some(app.id.clone());
    }

    fn remove_files(&mut self) {
        let Some(id) = self.confirm.take() else { return };
        self.ui().global::<Confirm>().set_shown(false);
        let Some(app) = self.library.app(&id) else { return };
        let task = plan::removal(app, None);
        self.show_plan(Pending {
            tasks: vec![task],
            remove_app: Some(id),
        });
    }

    fn forget_app(&mut self) {
        let Some(id) = self.confirm.take() else { return };
        self.ui().global::<Confirm>().set_shown(false);
        let name = self.library.app(&id).map(|a| a.name.clone()).unwrap_or_default();
        self.drop_app(&id);
        self.save_library();
        self.refresh_rows();
        self.toast(&format!("{name} is no longer managed; its files stay"), false);
    }

    fn drop_app(&mut self, id: &str) {
        if let Some(app) = self.library.app(id) {
            if let Some(p) = app.icon_path() {
                let _ = std::fs::remove_file(p);
            }
        }
        self.library.apps.retain(|a| a.id != id);
        self.status.retain(|(a, _), _| a != id);
        self.icons.remove(id);
    }

    // -------------------------------------------------------------- editor

    fn open_editor(&mut self, index: Option<usize>) {
        if self.busy.is_some() && index.is_some() {
            self.toast("Wait for the current work to finish before editing", false);
            return;
        }
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let state = match index.and_then(|i| self.library.apps.get(i)) {
            Some(app) => EditorState {
                app: app.clone(),
                editing: true,
                info: None,
                offers: HashMap::new(),
                selected: 0,
                icon: None,
                fetch_serial: 0,
                icon_serial: 0,
            },
            None => EditorState {
                app: LibApp {
                    id: String::new(),
                    name: String::new(),
                    source: Source::Github {
                        owner: String::new(),
                        repo: String::new(),
                    },
                    icon: None,
                    icon_source: IconSource::None,
                    // Follow the repository's icon unless one is picked by hand.
                    icon_auto: true,
                    description: String::new(),
                    deployments: Vec::new(),
                },
                editing: false,
                info: None,
                offers: HashMap::new(),
                selected: 0,
                icon: None,
                fetch_serial: 0,
                icon_serial: 0,
            },
        };
        e.set_editing(state.editing);
        e.set_repo(if state.editing { state.app.source.label().into() } else { "".into() });
        e.set_status("".into());
        e.set_status_error(false);
        e.set_busy(false);
        e.set_fetched(state.editing);
        e.set_name(state.app.name.clone().into());
        e.set_icon_auto(state.app.icon_auto);
        e.set_icon_loading(false);
        self.editor = Some(state);
        self.editor_show_icon();
        self.editor_show_deps();
        self.editor_show_dep();
        e.set_shown(true);
        if index.is_some() {
            // Workflows, default branch, offers.
            self.editor_fetch();
        }
    }

    fn close_editor(&mut self) {
        self.editor = None;
        let ui = self.ui();
        ui.global::<Editor>().set_shown(false);
        ui.invoke_focus_main();
    }

    fn editor_show_icon(&mut self) {
        let Some(ed) = &self.editor else { return };
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let img = match &ed.icon {
            Some((bytes, ext, _)) => {
                if ext == "svg" {
                    Image::load_from_svg_data(bytes).ok()
                } else {
                    let dir = config::cache_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    let p = dir.join(format!("editor-icon.{ext}"));
                    std::fs::write(&p, bytes).ok().and_then(|_| Image::load_from_path(&p).ok())
                }
            }
            None => ed.app.icon_path().and_then(|p| Image::load_from_path(&p).ok()),
        };
        let img = img.map(|i| crate::icons::smooth(i, self.icon_px()));
        let source = ed.icon.as_ref().map(|(_, _, s)| *s).unwrap_or(ed.app.icon_source);
        e.set_icon_from(
            match (img.is_some(), source) {
                (false, _) | (true, IconSource::None) => "No icon yet",
                (true, IconSource::Avatar) => "The owner's avatar (the repository has no icon Yoink recognised)",
                (true, IconSource::Repo) => "From the repository",
                (true, IconSource::AppImage) => "From inside the AppImage",
                (true, IconSource::Custom) => "Your own image",
            }
            .into(),
        );
        e.set_has_icon(img.is_some());
        e.set_icon(img.unwrap_or_default());
        let name = e.get_name();
        e.set_letter(
            name.chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".into())
                .into(),
        );
    }

    fn editor_fetch(&mut self) {
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let Some(ed) = &mut self.editor else { return };
        let source = if ed.editing {
            ed.app.source.clone()
        } else {
            match Source::parse(&e.get_repo()) {
                Some(s) => s,
                None => {
                    e.set_status("That is not a GitHub repository: type owner/name or paste its URL".into());
                    e.set_status_error(true);
                    return;
                }
            }
        };
        if !ed.editing {
            if let Some(existing) = self.library.apps.iter().find(|a| a.source == source) {
                e.set_status(
                    format!(
                        "{} is already in the list as {}: add another deployment to it by editing it, or carry on to add it twice.",
                        source.label(),
                        existing.name
                    )
                    .into(),
                );
            } else {
                e.set_status(format!("Looking up {}…", source.label()).into());
            }
            e.set_status_error(false);
        }
        e.set_busy(true);
        ed.fetch_serial += 1;
        let serial = ed.fetch_serial;
        let editing = ed.editing;
        let token = self.token.clone();
        std::thread::spawn(move || {
            let f = forge::forge(&source, token);
            let info = f.info();
            let icon = if editing { Ok(None) } else { info.as_ref().ok().map(|_| f.icon()).unwrap_or(Ok(None)) };
            let mut offers = Vec::new();
            if info.is_ok() && !editing {
                let rel = Channel::Releases { prerelease: false };
                let r = f.offer(&rel);
                let empty = r.as_ref().map(|o| o.files.is_empty()).unwrap_or(true);
                offers.push((rel, r.map_err(|e| short(&e))));
                if empty {
                    // No releases: maybe builds from Actions.
                    let act = Channel::Actions {
                        workflow: String::new(),
                        branch: String::new(),
                    };
                    let r = f.offer(&act).map_err(|e| short(&e));
                    offers.push((act, r));
                }
            }
            let info = info.map_err(|e| short(&e));
            let icon = icon.ok().flatten();
            post(move |a| a.editor_fetched(serial, source, info, icon, offers));
        });
    }

    fn editor_fetched(
        &mut self,
        serial: u64,
        source: Source,
        info: Result<RepoInfo, String>,
        icon: Option<forge::Icon>,
        offers: Vec<(Channel, Result<Offer, String>)>,
    ) {
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let Some(ed) = &mut self.editor else { return };
        if ed.fetch_serial != serial {
            return;
        }
        e.set_busy(false);
        let info = match info {
            Ok(i) => i,
            Err(err) => {
                e.set_status(format!("Could not look up {}: {err}", source.label()).into());
                e.set_status_error(true);
                return;
            }
        };
        for (c, o) in &offers {
            ed.offers.insert(channel_key(c), o.clone());
        }
        if !ed.editing {
            ed.app.source = source;
            ed.app.name = info.name.clone();
            ed.app.description = info.description.clone();
            e.set_name(info.name.clone().into());
            if let Some(icon) = icon {
                ed.icon = Some((icon.bytes, icon.ext.to_string(), icon.source));
            }
            if ed.app.deployments.is_empty() {
                // The first deployment: Releases if it has files, else Actions.
                let channel = offers
                    .iter()
                    .find(|(_, o)| o.as_ref().map(|o| !o.files.is_empty()).unwrap_or(false))
                    .map(|(c, _)| c.clone())
                    .unwrap_or(Channel::Releases { prerelease: false });
                ed.app.deployments.push(Deployment::new(
                    "1".into(),
                    channel,
                    Kind::Binary,
                    String::new(),
                ));
                ed.selected = 0;
            }
            let none = offers.iter().all(|(_, o)| o.as_ref().map(|o| o.files.is_empty()).unwrap_or(true));
            e.set_status(
                if none {
                    format!(
                        "Found {}{}. It offers no files in Releases or recent Actions runs yet.",
                        info.name,
                        if info.description.is_empty() { String::new() } else { format!(": {}", info.description) }
                    )
                } else {
                    format!(
                        "Found {}{}",
                        info.name,
                        if info.description.is_empty() { String::new() } else { format!(": {}", info.description) }
                    )
                }
                .into(),
            );
            e.set_status_error(false);
        }
        e.set_default_branch(info.default_branch.clone().into());
        let wf: Vec<SharedString> = info.workflows.iter().map(|w| w.into()).collect();
        e.set_workflows(ModelRc::new(VecModel::from(wf)));
        ed.info = Some(info);
        e.set_fetched(true);
        self.editor_show_icon();
        self.editor_show_deps();
        self.editor_show_dep();
        self.editor_load_offer();
    }

    fn editor_show_deps(&self) {
        let Some(ed) = &self.editor else { return };
        let rows: Vec<EdDep> = ed
            .app
            .deployments
            .iter()
            .map(|d| EdDep {
                label: d.label().into(),
                detail: if d.pattern.is_empty() { "no file picked".to_string() } else { d.pattern.clone() }.into(),
            })
            .collect();
        let ui = self.ui();
        let e = ui.global::<Editor>();
        e.set_deps(ModelRc::new(VecModel::from(rows)));
        e.set_selected(ed.selected as i32);
    }

    /// The selected deployment into the form.
    fn editor_show_dep(&self) {
        let Some(ed) = &self.editor else { return };
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let Some(d) = ed.app.deployments.get(ed.selected) else {
            e.set_cands(ModelRc::default());
            return;
        };
        match &d.channel {
            Channel::Releases { prerelease } => {
                e.set_channel(0);
                e.set_prerelease(*prerelease);
            }
            Channel::Actions { workflow, branch } => {
                e.set_channel(1);
                let idx = ed
                    .info
                    .as_ref()
                    .and_then(|i| i.workflows.iter().position(|w| w == workflow))
                    .map(|i| i as i32)
                    .unwrap_or(-1);
                e.set_workflow(idx);
                e.set_branch(branch.clone().into());
            }
        }
        e.set_pattern(d.pattern.clone().into());
        e.set_kind(d.kind.index() as i32);
        e.set_desktop(d.desktop);
        e.set_install_name(d.install_name.clone().into());
        e.set_vst2(d.formats.vst2);
        e.set_vst3(d.formats.vst3);
        e.set_lv2(d.formats.lv2);
        e.set_clap(d.formats.clap);
        let mut inner = Vec::new();
        if !d.inner.is_empty() {
            inner.push(d.inner.clone());
        }
        if !d.vst2.is_empty() {
            inner.push(format!("VST2: {}", d.vst2.join(", ")));
        }
        e.set_inner(inner.join("; ").into());
        self.editor_show_offer();
    }

    fn editor_show_offer(&self) {
        let Some(ed) = &self.editor else { return };
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let Some(d) = ed.app.deployments.get(ed.selected) else { return };
        match ed.offers.get(&channel_key(&d.channel)) {
            None => {
                e.set_offer_loading(true);
                e.set_cands(ModelRc::default());
                e.set_offer_version("".into());
                e.set_offer_note("".into());
                e.set_cand(-1);
            }
            Some(Err(err)) => {
                e.set_offer_loading(false);
                e.set_cands(ModelRc::default());
                e.set_offer_version("".into());
                e.set_offer_note(err.clone().into());
                e.set_cand(-1);
            }
            Some(Ok(o)) => {
                e.set_offer_loading(false);
                let cands: Vec<Cand> = o
                    .files
                    .iter()
                    .map(|f| Cand {
                        name: f.name.clone().into(),
                        size: human_size(f.size).into(),
                        kind: detect::guess_kind(&f.name).label().into(),
                        unlikely: detect::suitability(&f.name).is_none_or(|s| s < 0),
                    })
                    .collect();
                e.set_cands(ModelRc::new(VecModel::from(cands)));
                e.set_offer_version(o.version.clone().into());
                e.set_offer_note(o.note.clone().into());
                let sel = o
                    .files
                    .iter()
                    .position(|f| !d.pattern.is_empty() && pattern::glob_match(&d.pattern, &f.name))
                    .map(|i| i as i32)
                    .unwrap_or(-1);
                e.set_cand(sel);
            }
        }
    }

    /// The form into the selected deployment.
    fn editor_commit(&mut self) {
        let ui = self.ui();
        let e = ui.global::<Editor>();
        let Some(ed) = &mut self.editor else { return };
        ed.app.name = e.get_name().trim().to_string();
        let workflows = ed.info.as_ref().map(|i| i.workflows.clone()).unwrap_or_default();
        let Some(d) = ed.app.deployments.get_mut(ed.selected) else { return };
        d.channel = if e.get_channel() == 1 {
            let w = e.get_workflow();
            Channel::Actions {
                workflow: if w >= 0 { workflows.get(w as usize).cloned().unwrap_or_default() } else { String::new() },
                branch: e.get_branch().trim().to_string(),
            }
        } else {
            Channel::Releases {
                prerelease: e.get_prerelease(),
            }
        };
        d.pattern = e.get_pattern().trim().to_string();
        d.kind = Kind::ALL[(e.get_kind().max(0) as usize).min(3)];
        d.desktop = e.get_desktop();
        d.install_name = e.get_install_name().trim().to_string();
        d.formats.vst2 = e.get_vst2();
        d.formats.vst3 = e.get_vst3();
        d.formats.lv2 = e.get_lv2();
        d.formats.clap = e.get_clap();
    }

    fn editor_load_offer(&mut self) {
        self.editor_commit();
        let Some(ed) = &mut self.editor else { return };
        let Some(d) = ed.app.deployments.get(ed.selected) else { return };
        let channel = d.channel.clone();
        let key = channel_key(&channel);
        if ed.offers.contains_key(&key) {
            self.editor_auto_pick();
            self.editor_show_deps();
            self.editor_show_dep();
            return;
        }
        let source = ed.app.source.clone();
        let token = self.token.clone();
        self.editor_show_deps();
        self.editor_show_dep();
        std::thread::spawn(move || {
            let r = forge::forge(&source, token).offer(&channel).map_err(|e| short(&e));
            post(move |a| {
                if let Some(ed) = &mut a.editor {
                    ed.offers.insert(key, r);
                    a.editor_auto_pick();
                    a.editor_show_deps();
                    a.editor_show_dep();
                }
            });
        });
    }

    /// A fresh deployment (no file yet) takes the most likely file offered,
    /// one no other deployment of the app uses.
    fn editor_auto_pick(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        let Some(d) = ed.app.deployments.get(ed.selected) else { return };
        if !d.pattern.is_empty() {
            return;
        }
        let Some(Ok(o)) = ed.offers.get(&channel_key(&d.channel)) else { return };
        let taken: Vec<String> = ed
            .app
            .deployments
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != ed.selected)
            .map(|(_, d)| d.pattern.clone())
            .collect();
        let free: Vec<(usize, &str)> = o
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !taken.iter().any(|p| !p.is_empty() && pattern::glob_match(p, &f.name)))
            .map(|(i, f)| (i, f.name.as_str()))
            .collect();
        let Some(best) = detect::best_candidate(free.iter().map(|(_, n)| *n)) else { return };
        let i = free[best].0;
        self.editor_pick_inner(i);
    }

    fn editor_pick(&mut self, i: usize) {
        self.editor_commit();
        self.editor_pick_inner(i);
        self.editor_show_deps();
        self.editor_show_dep();
    }

    fn editor_pick_inner(&mut self, i: usize) {
        let Some(ed) = &mut self.editor else { return };
        let Some(d) = ed.app.deployments.get(ed.selected) else { return };
        let Some(Ok(o)) = ed.offers.get(&channel_key(&d.channel)) else { return };
        let Some(f) = o.files.get(i) else { return };
        let tag = match d.channel {
            Channel::Releases { .. } => Some(o.version.clone()),
            Channel::Actions { .. } => None,
        };
        let pat = pattern::derive(&f.name, tag.as_deref());
        let kind = detect::guess_kind(&f.name);
        let d = &mut ed.app.deployments[ed.selected];
        if d.kind != kind {
            d.desktop = kind == Kind::AppImage;
        }
        d.kind = kind;
        d.pattern = pat;
        d.inner.clear();
        d.vst2.clear();
    }

    fn editor_set_kind(&mut self, k: usize) {
        self.editor_commit();
        let Some(ed) = &mut self.editor else { return };
        let Some(d) = ed.app.deployments.get_mut(ed.selected) else { return };
        let kind = Kind::ALL[k.min(3)];
        if d.kind != kind {
            d.desktop = kind == Kind::AppImage;
            d.inner.clear();
        }
        d.kind = kind;
        self.editor_show_deps();
        self.editor_show_dep();
    }

    fn editor_select(&mut self, i: usize) {
        self.editor_commit();
        if let Some(ed) = &mut self.editor {
            if i < ed.app.deployments.len() {
                ed.selected = i;
            }
        }
        self.editor_show_deps();
        self.editor_show_dep();
        self.editor_load_offer();
    }

    fn editor_add_dep(&mut self) {
        self.editor_commit();
        let Some(ed) = &mut self.editor else { return };
        let channel = ed
            .app
            .deployments
            .get(ed.selected)
            .map(|d| d.channel.clone())
            .unwrap_or(Channel::Releases { prerelease: false });
        let id = ed.app.next_deployment_id();
        ed.app
            .deployments
            .push(Deployment::new(id, channel, Kind::Binary, String::new()));
        ed.selected = ed.app.deployments.len() - 1;
        self.editor_load_offer();
    }

    fn editor_remove_dep(&mut self, i: usize) {
        self.editor_commit();
        let Some(ed) = &mut self.editor else { return };
        if ed.app.deployments.len() <= 1 || i >= ed.app.deployments.len() {
            return;
        }
        let d = ed.app.deployments.remove(i);
        if d.installed.is_some() {
            let n = d.installed.as_ref().map(|x| x.files.len()).unwrap_or(0);
            let msg = format!(
                "Its {n} installed file{} stay where they are once saved",
                if n == 1 { "" } else { "s" }
            );
            ed.selected = ed.selected.min(ed.app.deployments.len() - 1);
            self.toast(&msg, false);
        } else {
            ed.selected = ed.selected.min(ed.app.deployments.len() - 1);
        }
        self.editor_show_deps();
        self.editor_show_dep();
        self.editor_load_offer();
    }

    fn editor_choose_icon(&mut self) {
        std::thread::spawn(|| {
            let file = rfd::FileDialog::new()
                .set_title("Pick an icon")
                .add_filter("Images", &["png", "svg", "jpg", "jpeg"])
                .pick_file();
            let Some(file) = file else { return };
            let bytes = std::fs::read(&file).ok();
            post(move |a| {
                let Some(bytes) = bytes else { return };
                let Some(ext) = forge::image_ext(&bytes) else {
                    a.toast("That is not a PNG, JPEG or SVG image", true);
                    return;
                };
                if let Some(ed) = &mut a.editor {
                    ed.icon = Some((bytes, ext.to_string(), IconSource::Custom));
                    // Never replaced: following the repository makes no sense.
                    ed.icon_serial += 1;
                }
                let ui = a.ui();
                let e = ui.global::<Editor>();
                e.set_icon_auto(false);
                e.set_icon_loading(false);
                a.editor_show_icon();
            });
        });
    }

    /// Fetches the repository's icon (or the owner's avatar) again for the
    /// app being edited.
    fn editor_repo_icon(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        let source = ed.app.source.clone();
        ed.icon_serial += 1;
        let serial = ed.icon_serial;
        self.ui().global::<Editor>().set_icon_loading(true);
        let token = self.token.clone();
        std::thread::spawn(move || {
            let icon = forge::forge(&source, token).icon().map_err(|e| short(&e));
            post(move |a| {
                let Some(ed) = &mut a.editor else { return };
                if ed.icon_serial != serial {
                    return;
                }
                a.ui().global::<Editor>().set_icon_loading(false);
                match icon {
                    Ok(Some(icon)) => {
                        if let Some(ed) = &mut a.editor {
                            ed.icon = Some((icon.bytes, icon.ext.to_string(), icon.source));
                        }
                        a.editor_show_icon();
                    }
                    Ok(None) => a.toast("The repository has no icon, and its owner no avatar", false),
                    Err(e) => a.toast(&format!("Could not fetch the icon: {e}"), true),
                }
            });
        });
    }

    fn editor_save(&mut self) {
        self.editor_commit();
        let Some(ed) = &self.editor else { return };
        let mut app = ed.app.clone();
        if app.name.trim().is_empty() {
            app.name = match &app.source {
                Source::Github { repo, .. } => repo.clone(),
            };
        }
        if let Some(i) = app.deployments.iter().position(|d| d.pattern.is_empty()) {
            if let Some(ed) = &mut self.editor {
                ed.selected = i;
            }
            self.editor_show_deps();
            self.editor_show_dep();
            let ui = self.ui();
            let e = ui.global::<Editor>();
            e.set_status("Pick a file for every deployment (or type a pattern)".into());
            e.set_status_error(true);
            return;
        }
        let editing = ed.editing;
        let icon = ed.icon.clone();
        app.icon_auto = self.ui().global::<Editor>().get_icon_auto();
        if !editing {
            app.id = self.library.new_id(&app.source);
        }
        if let Some((bytes, ext, source)) = icon {
            match save_icon(&app.id, app.icon.as_deref(), &bytes, &ext) {
                Ok(file) => {
                    app.icon = Some(file);
                    app.icon_source = source;
                    self.icons.remove(&app.id);
                }
                Err(e) => log::warn!("could not save the icon: {e:#}"),
            }
        }
        let id = app.id.clone();
        if editing {
            if let Some(slot) = self.library.app_mut(&id) {
                *slot = app;
            }
            // A changed deployment may resolve to another build.
            self.status.retain(|(a, _), _| *a != id);
        } else {
            self.library.apps.push(app);
        }
        self.save_library();
        self.close_editor();
        self.refresh_rows();
        self.check_app(&id);
    }

    // ------------------------------------------------------------- options

    fn sync_prefs(&self) {
        let ui = self.ui();
        let p = ui.global::<Prefs>();
        let dirs: Vec<DirRow> = DIR_INFO
            .iter()
            .map(|(id, label, detail)| DirRow {
                id: (*id).into(),
                label: (*label).into(),
                path: self.settings.dirs.get(id).into(),
                detail: (*detail).into(),
            })
            .collect();
        p.set_dirs(ModelRc::new(VecModel::from(dirs)));
        p.set_has_token(self.token.is_some());
        let warning = self.token_warning.as_ref().filter(|w| {
            !(w.dismissible
                && self.token.as_deref().is_some_and(|t| {
                    !self.settings.token_warning_dismissed.is_empty()
                        && secret::fingerprint(t) == self.settings.token_warning_dismissed
                }))
        });
        p.set_token_warning(warning.map(|w| w.text.clone()).unwrap_or_default().into());
        p.set_token_warning_dismissible(warning.is_some_and(|w| w.dismissible));
        p.set_token_status(
            match (&self.token, self.token_store) {
                (Some(_), Some(s)) => {
                    if self.token_note.is_empty() {
                        format!("A token is saved in {}.", s.label())
                    } else {
                        format!("A token is saved in {}. {}", s.label(), self.token_note)
                    }
                }
                _ => "No token: public repositories and releases only, 60 requests an hour.".into(),
            }
            .into(),
        );
        let names: Vec<SharedString> = ui.global::<Theme>().get_names().iter().collect();
        p.set_theme(
            names
                .iter()
                .position(|n| n.as_str() == self.settings.theme)
                .or_else(|| names.iter().position(|n| n.as_str() == "Iris"))
                .unwrap_or(0) as i32,
        );
        p.set_check_on_startup(self.settings.check_on_startup);
        p.set_allow_failed(self.settings.allow_failed_runs);
        p.set_hover_opens(self.settings.hover_opens);
        p.set_native_wayland(self.settings.native_wayland);
        p.set_app_subdirs(self.settings.dirs.app_subdirs);
        p.set_version(version_label().into());
        self.sync_self();
        let plasma = crate::plasma::state(&self.settings.dirs);
        p.set_plasma_on(plasma.present());
        p.set_plasma_enabled(plasma.present() || matches!(plasma, crate::plasma::State::Missing(_)));
        p.set_plasma_status(plasma.describe().into());
        p.set_config_path(tilde(&config::config_dir().join("settings.toml")).into());
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            log::warn!("could not save the settings: {e:#}");
        }
        self.settings_dirty = false;
    }

    fn set_dir(&mut self, id: &str, path: &str) {
        let path = path.trim().to_string();
        if self.settings.dirs.get(id) == path {
            return;
        }
        self.settings.dirs.set(id, path);
        self.save_settings();
        if id == "desktop" {
            match crate::plasma::follow(&self.settings.dirs) {
                Ok(Some(_)) => self.toast("Plasma will look in the new folder from the next login", false),
                Ok(None) => {}
                Err(e) => self.toast(&format!("Could not update the Plasma script: {e:#}"), true),
            }
        }
        self.sync_prefs();
    }

    fn choose_dir(&mut self, id: String) {
        let start = self.settings.dirs.path(&id).ok();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new().set_title("Pick a folder");
            if let Some(s) = start.filter(|s| s.is_dir()) {
                d = d.set_directory(s);
            }
            if let Some(dir) = d.pick_folder() {
                let text = tilde(&dir);
                post(move |a| a.set_dir(&id, &text));
            }
        });
    }

    fn save_token(&mut self, t: &str) {
        let t = t.trim().to_string();
        if t.is_empty() {
            return;
        }
        match secret::save(&t) {
            Ok(store) => {
                self.token = Some(t.clone());
                self.token_store = Some(store);
                self.token_note = "Checking it…".into();
                self.token_warning = None;
                self.sync_prefs();
                check_token(t);
            }
            Err(e) => self.toast(&format!("Could not save the token: {e:#}"), true),
        }
    }

    fn set_theme(&mut self, i: i32) {
        let ui = self.ui();
        let names: Vec<SharedString> = ui.global::<Theme>().get_names().iter().collect();
        if let Some(n) = names.get(i as usize) {
            self.settings.theme = n.to_string();
            ui.global::<Theme>().set_index(i);
            self.save_settings();
            self.sync_prefs();
        }
    }

    fn set_flag(&mut self, id: &str, v: bool) {
        match id {
            "check-on-startup" => self.settings.check_on_startup = v,
            "allow-failed" => {
                self.settings.allow_failed_runs = v;
                self.status.clear();
                self.refresh_rows();
            }
            "hover-opens" => {
                self.settings.hover_opens = v;
                self.ui().set_hover_opens(v);
            }
            "native-wayland" => self.settings.native_wayland = v,
            "app-subdirs" => self.settings.dirs.app_subdirs = v,
            _ => return,
        }
        self.save_settings();
        self.sync_prefs();
    }

    fn set_plasma(&mut self, v: bool) {
        let result = if v {
            crate::plasma::install(&self.settings.dirs).map(drop)
        } else {
            crate::plasma::remove()
        };
        match result {
            Ok(()) if v => self.toast("Plasma will show these entries from the next login", false),
            Ok(()) => self.toast("Plasma script removed: takes effect at the next login", false),
            Err(e) => self.toast(&format!("Could not change the Plasma script: {e:#}"), true),
        }
        self.sync_prefs();
    }

    // --------------------------------------------------------- self update

    /// Yoink keeps itself up to date as an app in the list, like any other
    /// (as on Android): Options says whether it is there, and offers to add it.
    fn sync_self(&self) {
        let ui = self.ui();
        let p = ui.global::<Prefs>();
        let listed = self.library.apps.iter().any(|a| a.is_yoink());
        p.set_self_listed(listed);
        let status = if self.restart_into.is_some() {
            "A new version is installed: restart Yoink to use it."
        } else if listed {
            "Yoink is in your apps: it is checked and updated with them, and gets a desktop entry if its deployment asks for one, like any app."
        } else {
            "To keep Yoink up to date, add it to your apps: it is then checked and updated with them."
        };
        p.set_self_status(status.into());
        ui.global::<Apps>().set_restart_ready(self.restart_into.is_some());
    }

    /// Yoink's place in the applications menu follows its own deployment's
    /// desktop setting, like any app's: an entry it wrote for itself (an
    /// older version's option) goes, unless that deployment wants one.
    fn retire_menu_entry(&self) {
        let wanted = self.library.apps.iter().filter(|a| a.is_yoink()).any(|a| {
            a.deployments
                .iter()
                .any(|d| d.desktop && matches!(d.kind, Kind::AppImage | Kind::Binary))
        });
        crate::desktop::retire_menu_entry(&self.settings.dirs, wanted);
    }

    /// Opens the editor on Yoink's own repository, ready to pick its build.
    fn add_self(&mut self) {
        if let Some(i) = self.library.apps.iter().position(|a| a.is_yoink()) {
            self.open_editor(Some(i));
            return;
        }
        self.open_editor(None);
        let repo = format!("{}/{}", crate::library::YOINK_OWNER, crate::library::YOINK_REPO);
        self.ui().global::<Editor>().set_repo(repo.into());
        self.editor_fetch();
    }

    fn restart(&mut self) {
        // The new version first, else what runs now (as it was started).
        let mut candidates: Vec<PathBuf> = Vec::new();
        for p in self.restart_into.iter().cloned().chain(plan::running_exe()) {
            if p.is_file() && !candidates.contains(&p) {
                candidates.push(p);
            }
        }
        if candidates.is_empty() {
            self.toast("Could not restart: cannot tell where Yoink is", true);
            return;
        }
        self.remember_window();
        self.save_settings();
        let mut errors = Vec::new();
        for exe in candidates {
            match spawn_exe(&exe) {
                Ok(()) => {
                    let _ = slint::quit_event_loop();
                    return;
                }
                Err(e) => {
                    log::warn!("could not start {}: {e}", exe.display());
                    errors.push(format!("{}: {e}", tilde(&exe)));
                }
            }
        }
        self.toast(&format!("Could not restart: {}", errors.join("; ")), true);
    }

    // -------------------------------------------------------------- window

    fn remember_window(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let w = ui.window();
        self.settings.maximized = w.is_maximized();
        if !w.is_maximized() {
            let size = w.size().to_logical(w.scale_factor());
            if size.width > 100.0 && size.height > 100.0 {
                self.settings.width = size.width;
                self.settings.height = size.height;
            }
        }
    }

    /// Once the window manager has taken over a move or a resize, the
    /// button's release goes to it: without one, the element pressed would
    /// keep the pointer (as in Chippy).
    fn release_pointer(&self) {
        let (x, y) = CURSOR.with(|c| c.get());
        let position = slint::LogicalPosition::new(x, y);
        let weak = self.ui.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(ui) = weak.upgrade() else { return };
            let w = ui.window();
            w.dispatch_event(slint::platform::WindowEvent::PointerReleased {
                position,
                button: slint::platform::PointerEventButton::Left,
            });
            w.dispatch_event(slint::platform::WindowEvent::PointerExited);
        });
    }

    fn with_winit(&self, f: impl FnOnce(&slint::winit_030::winit::window::Window)) {
        use slint::winit_030::WinitWindowAccessor;
        if let Some(ui) = self.ui.upgrade() {
            ui.window().with_winit_window(f);
        }
    }
}

fn version_label() -> String {
    let commit = env!("YOINK_COMMIT");
    if commit.len() >= 7 {
        format!("{} ({})", crate::VERSION, &commit[..7])
    } else {
        crate::VERSION.to_string()
    }
}

/// Saves an app's icon as `<id>.<ext>` in Yoink's icons folder (removing
/// one of another type), returning its file name.
fn save_icon(id: &str, old: Option<&str>, bytes: &[u8], ext: &str) -> anyhow::Result<String> {
    let dir = crate::library::icons_dir();
    std::fs::create_dir_all(&dir)?;
    let file = format!("{}.{ext}", crate::library::slug(id));
    if let Some(old) = old {
        if old != file {
            let _ = std::fs::remove_file(dir.join(old));
        }
    }
    config::write_atomic(&dir.join(&file), bytes)?;
    Ok(file)
}
