mod space;
mod ui;

use crate::actions::{self, Mode, Report};
use crate::apps::{self, App as MacApp};
use crate::model::{Category, Finding, Fix, HealthItem, Risk, SubItem};
use crate::scan::{self, Payload, ScanMsg, health};
use crate::util;
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

pub use space::SpaceView;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Scanning,
    Menu,
    Category(Category),
    /// Smart clean: every "safe" finding, from all categories.
    Smart,
    Health,
    Apps,
    Uninstall,
    Space,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuItem {
    Smart,
    /// Untick everything, so the user picks only what they want.
    Manual,
    /// Clean everything ticked, in every category, at once.
    DeleteSelected,
    Cat(Category),
    Health,
    Apps,
    Space,
    Rescan,
    Quit,
}

/// A visible line in a category screen: a finding, or one of its sub-items.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Finding(usize),
    Child(usize, usize),
}

pub enum Modal {
    /// Cleaning confirmation.
    Confirm {
        title: String,
        items: Vec<Finding>,
        from_findings: bool,
    },
    /// Shell command confirmation.
    ConfirmShell {
        label: String,
        cmd: String,
    },
    /// Cleaning in progress.
    Running {
        title: String,
        current: String,
        done: usize,
        total: usize,
    },
    Message {
        title: String,
        lines: Vec<String>,
    },
    Help,
}

pub enum JobMsg {
    Item(usize, String),
    Done(Report, Vec<(String, u64)>),
}

pub struct StepState {
    pub text: String,
    pub done: bool,
}

pub struct UninstallPlan {
    pub app: MacApp,
    pub files: Vec<(PathBuf, u64, bool)>,
    pub state: ListState,
}

/// Files of a health item being picked one by one (focus in the Details pane).
pub struct HealthPick {
    pub ticked: Vec<bool>,
    pub state: ListState,
}

pub struct App {
    pub screen: Screen,
    pub steps: Vec<StepState>,
    pub findings: Vec<Finding>,
    pub health: Vec<HealthItem>,
    pub errors: Vec<String>,
    tx: Sender<ScanMsg>,
    rx: Receiver<ScanMsg>,
    pub menu_state: ListState,
    pub cat_state: ListState,
    pub health_state: ListState,
    pub apps: Vec<MacApp>,
    pub apps_state: ListState,
    pub apps_filter: String,
    pub filtering: bool,
    apps_rx: Option<Receiver<(PathBuf, u64)>>,
    pub uninstall: Option<UninstallPlan>,
    pub space: Option<SpaceView>,
    pub health_pick: Option<HealthPick>,
    pub modal: Option<Modal>,
    pub mode: Mode,
    pub toast: Option<String>,
    pub disk: Option<(u64, u64)>,
    pub tick: u64,
    pub cleaned_session: u64,
    job_rx: Option<Receiver<JobMsg>>,
    job_from_findings: bool,
    pending_shell: Option<String>,
    pub quit: bool,
}

fn disk() -> Option<(u64, u64)> {
    let t = if util::is_macos() && std::path::Path::new("/System/Volumes/Data").exists() {
        "/System/Volumes/Data"
    } else {
        "/"
    };
    health::disk_free(t)
}

impl App {
    fn new() -> App {
        let (tx, rx) = channel();
        scan::spawn_all(tx.clone());
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        App {
            screen: Screen::Scanning,
            steps: scan::STEPS
                .iter()
                .map(|_| StepState {
                    text: "waiting…".into(),
                    done: false,
                })
                .collect(),
            findings: vec![],
            health: vec![],
            errors: vec![],
            tx,
            rx,
            menu_state,
            cat_state: ListState::default(),
            health_state: ListState::default(),
            apps: vec![],
            apps_state: ListState::default(),
            apps_filter: String::new(),
            filtering: false,
            apps_rx: None,
            uninstall: None,
            space: None,
            health_pick: None,
            modal: None,
            mode: Mode::Trash,
            toast: None,
            disk: disk(),
            tick: 0,
            cleaned_session: 0,
            job_rx: None,
            job_from_findings: false,
            pending_shell: None,
            quit: false,
        }
    }

    // ───────────────────────── Derived data ─────────────────────────

    pub fn menu_items(&self) -> Vec<MenuItem> {
        let mut v = vec![MenuItem::Smart, MenuItem::Manual, MenuItem::DeleteSelected];
        v.extend(Category::ALL.iter().map(|c| MenuItem::Cat(*c)));
        v.extend([
            MenuItem::Health,
            MenuItem::Apps,
            MenuItem::Space,
            MenuItem::Rescan,
            MenuItem::Quit,
        ]);
        v
    }

    pub fn cat_indices(&self, c: Category) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.findings.len())
            .filter(|&i| self.findings[i].category == c)
            .collect();
        idx.sort_by(|&a, &b| self.findings[b].size.cmp(&self.findings[a].size));
        idx
    }

    /// Findings shown on a list screen: one category, or (None) the smart clean.
    pub fn scope_indices(&self, scope: Option<Category>) -> Vec<usize> {
        match scope {
            Some(c) => self.cat_indices(c),
            None => {
                let mut idx: Vec<usize> = (0..self.findings.len())
                    .filter(|&i| self.findings[i].risk == Risk::Safe)
                    .collect();
                idx.sort_by(|&a, &b| self.findings[b].size.cmp(&self.findings[a].size));
                idx
            }
        }
    }

    pub fn rows(&self, scope: Option<Category>) -> Vec<Row> {
        let mut rows = Vec::new();
        for i in self.scope_indices(scope) {
            rows.push(Row::Finding(i));
            let f = &self.findings[i];
            if f.expanded
                && let Some(ch) = &f.children
            {
                rows.extend((0..ch.len()).map(|j| Row::Child(i, j)));
            }
        }
        rows
    }

    pub fn cat_total(&self, c: Category) -> (u64, usize) {
        self.findings
            .iter()
            .filter(|f| f.category == c)
            .fold((0, 0), |(s, n), f| (s + f.size, n + 1))
    }

    pub fn cat_selected(&self, c: Category) -> u64 {
        self.findings
            .iter()
            .filter(|f| f.category == c)
            .map(|f| f.selected_size())
            .sum()
    }

    /// Smart clean = every "safe" finding, honouring sub-item selections.
    pub fn smart_items(&self) -> Vec<Finding> {
        let mut v: Vec<Finding> = self
            .findings
            .iter()
            .filter(|f| f.risk == Risk::Safe)
            .filter_map(|f| f.to_job())
            .collect();
        v.sort_by_key(|f| std::cmp::Reverse(f.size));
        v
    }

    /// Everything ticked, in every category.
    pub fn selected_items(&self) -> Vec<Finding> {
        let mut v: Vec<Finding> = self.findings.iter().filter_map(|f| f.to_job()).collect();
        v.sort_by_key(|f| std::cmp::Reverse(f.size));
        v
    }

    pub fn health_issues(&self) -> (usize, usize) {
        use crate::model::Status;
        let fail = self
            .health
            .iter()
            .filter(|h| h.status == Status::Fail)
            .count();
        let warn = self
            .health
            .iter()
            .filter(|h| h.status == Status::Warn)
            .count();
        (fail, warn)
    }

    pub fn filtered_apps(&self) -> Vec<usize> {
        let f = self.apps_filter.to_lowercase();
        (0..self.apps.len())
            .filter(|&i| {
                f.is_empty()
                    || self.apps[i].name.to_lowercase().contains(&f)
                    || self.apps[i].bundle_id.to_lowercase().contains(&f)
            })
            .collect()
    }

    pub fn scanning_done(&self) -> bool {
        self.steps.iter().all(|s| s.done)
    }

    // ───────────────────────── Thread messages ─────────────────────────

    fn pump(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        while let Ok(m) = self.rx.try_recv() {
            match m {
                ScanMsg::Progress(i, s) => self.steps[i].text = s,
                ScanMsg::Done(i, p) => {
                    self.steps[i].done = true;
                    match p {
                        Payload::Findings(f) => {
                            self.steps[i].text = format!(
                                "{} item(s), {}",
                                f.len(),
                                util::fmt_size(f.iter().map(|x| x.size).sum())
                            );
                            self.findings.extend(f);
                        }
                        Payload::Health(h) => {
                            use crate::model::Status;
                            let fail = h.iter().filter(|x| x.status == Status::Fail).count();
                            let warn = h.iter().filter(|x| x.status == Status::Warn).count();
                            self.steps[i].text = format!(
                                "{} check(s), {} problem(s), {} warning(s)",
                                h.len(),
                                fail,
                                warn
                            );
                            self.health = h;
                            self.health_pick = None;
                        }
                        Payload::Error(e) => {
                            self.steps[i].text = format!("error: {e}");
                            self.errors.push(e);
                        }
                    }
                    if self.screen == Screen::Scanning && self.scanning_done() {
                        self.screen = Screen::Menu;
                    }
                }
            }
        }
        if let Some(rx) = &self.apps_rx {
            while let Ok((p, s)) = rx.try_recv() {
                if let Some(a) = self.apps.iter_mut().find(|a| a.path == p) {
                    a.size = s;
                }
            }
        }
        if let Some(sp) = &mut self.space {
            sp.pump();
        }
        let mut finished = None;
        if let Some(rx) = &self.job_rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    JobMsg::Item(i, t) => {
                        if let Some(Modal::Running { current, done, .. }) = &mut self.modal {
                            *current = t;
                            *done = i;
                        }
                    }
                    JobMsg::Done(rep, sizes) => finished = Some((rep, sizes)),
                }
            }
        }
        if let Some((rep, sizes)) = finished {
            self.job_rx = None;
            self.finish_job(rep, sizes);
        }
    }

    fn finish_job(&mut self, rep: Report, sizes: Vec<(String, u64)>) {
        if self.job_from_findings {
            for (id, s) in sizes {
                let Some(f) = self.findings.iter_mut().find(|f| f.id == id) else {
                    continue;
                };
                match &mut f.children {
                    // Expanded finding: drop the sub-items that are gone, keep the rest.
                    Some(ch) => {
                        ch.retain(|c| std::fs::symlink_metadata(&c.path).is_ok());
                        for c in ch.iter_mut() {
                            c.selected = false;
                        }
                        f.size = ch.iter().map(|c| c.size).sum();
                    }
                    None => f.size = s,
                }
                f.selected = false;
            }
            self.findings.retain(|f| {
                f.size >= 100_000 || f.children.as_ref().is_some_and(|c| !c.is_empty())
            });
        }
        if self.screen == Screen::Uninstall
            && let Some(plan) = &self.uninstall
        {
            let p = plan.app.path.clone();
            if !p.exists() {
                self.apps.retain(|a| a.path != p);
                self.uninstall = None;
                self.screen = Screen::Apps;
            } else {
                // reload the list (some files may have failed)
                self.open_uninstall_for(plan.app.clone());
            }
        }
        if self.screen == Screen::Health {
            self.health_pick = None;
            self.rescan_health();
        }
        if self.screen == Screen::Space
            && let Some(sp) = &mut self.space
        {
            sp.refresh();
            sp.prune_marks();
        }
        self.cleaned_session += rep.freed;
        self.disk = disk();
        let mut lines = vec![format!("{} item(s) processed", rep.removed)];
        if self.mode == Mode::Trash {
            lines.push(format!("Moved to the Trash: {}", util::fmt_size(rep.freed)));
            lines.push("The space is freed once the Trash is emptied (\"Trash\" entry after a rescan, or Finder).".into());
        } else {
            lines.push(format!("Space freed: {}", util::fmt_size(rep.freed)));
        }
        if !rep.errors.is_empty() {
            lines.push(String::new());
            lines.push(format!("{} error(s):", rep.errors.len()));
            for (p, e) in rep.errors.iter().take(12) {
                lines.push(format!("  ✖ {} — {}", util::pretty(p), e));
            }
        }
        lines.push(String::new());
        lines.push(format!(
            "Journal: {}",
            util::pretty(&actions::journal_path())
        ));
        self.modal = Some(Modal::Message {
            title: "Cleaning done".into(),
            lines,
        });
    }

    fn start_job(&mut self, title: String, items: Vec<Finding>, from_findings: bool) {
        let (tx, rx) = channel();
        let mode = self.mode;
        let total = items.len();
        self.job_from_findings = from_findings;
        self.job_rx = Some(rx);
        self.modal = Some(Modal::Running {
            title,
            current: String::new(),
            done: 0,
            total,
        });
        std::thread::spawn(move || {
            let rep = actions::clean(&items, mode, false, &mut |i, t| {
                let _ = tx.send(JobMsg::Item(i, t.to_string()));
            });
            // New size of each finding after cleaning.
            let sizes = items
                .iter()
                .map(|f| {
                    let s: u64 = f
                        .paths
                        .iter()
                        .map(|p| {
                            let s = util::disk_usage(p);
                            if f.contents_only {
                                s.saturating_sub(4096)
                            } else {
                                s
                            }
                        })
                        .sum();
                    (f.id.clone(), s)
                })
                .collect();
            let _ = tx.send(JobMsg::Done(rep, sizes));
        });
    }

    fn rescan(&mut self) {
        let (tx, rx) = channel();
        self.tx = tx.clone();
        self.rx = rx;
        self.findings.clear();
        self.health.clear();
        self.errors.clear();
        for s in &mut self.steps {
            s.done = false;
            s.text = "waiting…".into();
        }
        self.screen = Screen::Scanning;
        scan::spawn_all(tx);
    }

    fn rescan_health(&mut self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = health::scan(&|_| {});
            let _ = tx.send(ScanMsg::Done(3, Payload::Health(r)));
        });
    }

    /// Lists the sub-items of a finding (once) and shows them.
    fn expand(&mut self, i: usize) {
        let f = &mut self.findings[i];
        if f.children.is_none() {
            use rayon::prelude::*;
            let sel = f.selected;
            let mut ch: Vec<SubItem> = f
                .expand_paths()
                .into_par_iter()
                .map(|p| SubItem {
                    size: util::disk_usage(&p),
                    path: p,
                    selected: sel,
                })
                .collect();
            ch.sort_by_key(|c| std::cmp::Reverse(c.size));
            f.children = Some(ch);
        }
        let n = f.children.as_ref().map_or(0, |c| c.len());
        if n == 0 {
            self.toast = Some("Nothing inside.".into());
            return;
        }
        f.expanded = true;
    }

    // ───────────────────────── Navigation ─────────────────────────

    fn open_menu_item(&mut self, item: MenuItem) {
        match item {
            MenuItem::Smart => {
                if self.findings.iter().any(|f| f.risk == Risk::Safe) {
                    self.screen = Screen::Smart;
                    self.cat_state.select(Some(0));
                } else {
                    self.toast = Some("Nothing safe to clean right now.".into());
                }
            }
            MenuItem::Manual => {
                for f in &mut self.findings {
                    f.set_selected(false);
                }
                // Put the cursor on the first category so the user can start ticking.
                let first = self
                    .menu_items()
                    .iter()
                    .position(|m| matches!(m, MenuItem::Cat(_)));
                self.menu_state.select(first);
                self.toast = Some("Everything unticked. Open the categories, tick what you want (Space), then \"Delete selected\".".into());
            }
            MenuItem::DeleteSelected => {
                let items = self.selected_items();
                if items.is_empty() {
                    self.toast = Some(
                        "Nothing ticked yet: open a category and tick items with Space.".into(),
                    );
                } else {
                    self.modal = Some(Modal::Confirm {
                        title: "Delete selected".into(),
                        items,
                        from_findings: true,
                    });
                }
            }
            MenuItem::Cat(c) => {
                self.screen = Screen::Category(c);
                self.cat_state.select(Some(0));
            }
            MenuItem::Health => {
                self.screen = Screen::Health;
                self.health_state.select(Some(0));
            }
            MenuItem::Apps => self.open_apps(),
            MenuItem::Space => {
                if self.space.is_none() {
                    self.space = Some(SpaceView::new(util::home()));
                }
                self.screen = Screen::Space;
            }
            MenuItem::Rescan => self.rescan(),
            MenuItem::Quit => self.quit = true,
        }
    }

    fn open_apps(&mut self) {
        self.screen = Screen::Apps;
        if self.apps.is_empty() {
            self.apps = apps::installed(false);
            let paths: Vec<PathBuf> = self.apps.iter().map(|a| a.path.clone()).collect();
            let (tx, rx) = channel();
            self.apps_rx = Some(rx);
            std::thread::spawn(move || {
                use rayon::prelude::*;
                paths.par_iter().for_each(|p| {
                    let _ = tx.send((p.clone(), util::disk_usage(p)));
                });
            });
        }
        self.apps_state.select(Some(0));
    }

    fn open_uninstall_for(&mut self, app: MacApp) {
        let mut files: Vec<(PathBuf, u64, bool)> = vec![(
            app.path.clone(),
            app.size.max(util::disk_usage(&app.path)),
            true,
        )];
        for p in apps::related_files(&app) {
            let s = util::disk_usage(&p);
            files.push((p, s, true));
        }
        files.retain(|(p, _, _)| p.exists());
        let mut state = ListState::default();
        state.select(Some(0));
        self.uninstall = Some(UninstallPlan { app, files, state });
        self.screen = Screen::Uninstall;
    }

    fn back(&mut self) {
        self.screen = match self.screen {
            Screen::Uninstall => Screen::Apps,
            Screen::Scanning if !self.scanning_done() => Screen::Scanning,
            _ => Screen::Menu,
        };
    }

    fn reveal(&mut self, p: &std::path::Path) {
        if util::is_macos() {
            let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
            self.toast = Some(format!("Shown in Finder: {}", util::pretty(p)));
        } else {
            self.toast = Some(util::pretty(p));
        }
    }

    // ───────────────────────── Keyboard ─────────────────────────

    fn on_key(&mut self, k: KeyEvent) {
        self.toast = None;
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        if self.modal.is_some() {
            self.on_modal_key(k);
            return;
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.apps_filter.clear();
                }
                KeyCode::Enter | KeyCode::Down => self.filtering = false,
                KeyCode::Backspace => {
                    self.apps_filter.pop();
                }
                KeyCode::Char(c) => {
                    self.apps_filter.push(c);
                    self.apps_state.select(Some(0));
                }
                _ => {}
            }
            return;
        }
        // Outside the menu, q goes one step back, exactly like ←.
        let k = if k.code == KeyCode::Char('q')
            && !matches!(self.screen, Screen::Menu | Screen::Scanning)
        {
            KeyEvent::new(KeyCode::Left, k.modifiers)
        } else {
            k
        };
        let in_cat = matches!(self.screen, Screen::Category(_) | Screen::Smart);
        // ← / Esc leave the file list of a health item before leaving the screen.
        let picking = self.screen == Screen::Health && self.health_pick.is_some();
        match k.code {
            KeyCode::Char('?') => {
                self.modal = Some(Modal::Help);
                return;
            }
            KeyCode::Char('q')
                if self.screen == Screen::Menu || self.screen == Screen::Scanning =>
            {
                self.quit = true;
                return;
            }
            KeyCode::Char('p') => {
                self.mode = if self.mode == Mode::Trash {
                    Mode::Permanent
                } else {
                    Mode::Trash
                };
                self.toast = Some(match self.mode {
                    Mode::Trash => "Mode: move to Trash (recoverable)".into(),
                    Mode::Permanent => "Mode: PERMANENT deletion".into(),
                });
                return;
            }
            KeyCode::Esc => {
                if self.screen != Screen::Space && !picking {
                    self.back();
                    return;
                }
            }
            // In a category, ← / h collapses first (handled below).
            KeyCode::Left | KeyCode::Char('h')
                if self.screen != Screen::Space && !in_cat && !picking =>
            {
                self.back();
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Scanning => {
                if matches!(k.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    self.screen = Screen::Menu; // you can browse while the scan continues
                }
            }
            Screen::Menu => {
                let items = self.menu_items();
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        move_sel(&mut self.menu_state, items.len(), -1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        move_sel(&mut self.menu_state, items.len(), 1)
                    }
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(i) = self.menu_state.selected() {
                            self.open_menu_item(items[i]);
                        }
                    }
                    KeyCode::Char(c @ '1'..='9') => {
                        let i = c as usize - '1' as usize;
                        if i < items.len() {
                            self.menu_state.select(Some(i));
                            self.open_menu_item(items[i]);
                        }
                    }
                    KeyCode::Char('s') => self.open_menu_item(MenuItem::Smart),
                    KeyCode::Char('m') => self.open_menu_item(MenuItem::Manual),
                    KeyCode::Char('d') => self.open_menu_item(MenuItem::DeleteSelected),
                    KeyCode::Char('e') => self.open_menu_item(MenuItem::Space),
                    KeyCode::Char('u') => self.open_menu_item(MenuItem::Apps),
                    KeyCode::Char('r') => self.rescan(),
                    _ => {}
                }
            }
            Screen::Category(c) => self.on_category_key(Some(c), k),
            Screen::Smart => self.on_category_key(None, k),
            Screen::Health if picking => self.on_health_pick_key(k),
            Screen::Health => {
                let n = self.health.len();
                match k.code {
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab | KeyCode::Char(' ') => {
                        match self.health_picks() {
                            Some(picks) if !picks.is_empty() => {
                                let mut state = ListState::default();
                                state.select(Some(0));
                                self.health_pick = Some(HealthPick {
                                    ticked: vec![false; picks.len()],
                                    state,
                                });
                            }
                            _ => self.toast = Some("Nothing to pick for this item.".into()),
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.health_state, n, -1),
                    KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.health_state, n, 1),
                    KeyCode::Char('r') => {
                        self.rescan_health();
                        self.toast = Some("Checking again…".into());
                    }
                    KeyCode::Enter => {
                        let Some(h) = self
                            .health_state
                            .selected()
                            .and_then(|i| self.health.get(i))
                        else {
                            return;
                        };
                        match &h.fix {
                            None => self.toast = Some("No automatic action for this item.".into()),
                            Some(Fix::Shell { label, cmd } | Fix::ShellEach { label, cmd, .. }) => {
                                self.modal = Some(Modal::ConfirmShell {
                                    label: label.clone(),
                                    cmd: cmd.clone(),
                                })
                            }
                            Some(Fix::Trash { label, paths }) => {
                                let mut f = Finding::new(
                                    Category::System,
                                    format!("fix:{}", h.id),
                                    label.clone(),
                                );
                                f.paths = paths.clone();
                                f.size = paths.iter().map(|p| util::disk_usage(p)).sum();
                                f.explanation = h.advice.clone();
                                self.modal = Some(Modal::Confirm {
                                    title: h.name.clone(),
                                    items: vec![f],
                                    from_findings: false,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Screen::Apps => {
                let list = self.filtered_apps();
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        move_sel(&mut self.apps_state, list.len(), -1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        move_sel(&mut self.apps_state, list.len(), 1)
                    }
                    KeyCode::PageDown => move_sel(&mut self.apps_state, list.len(), 15),
                    KeyCode::PageUp => move_sel(&mut self.apps_state, list.len(), -15),
                    KeyCode::Char('/') => {
                        self.filtering = true;
                        self.apps_filter.clear();
                    }
                    KeyCode::Char('s') => {
                        self.apps.sort_by_key(|x| std::cmp::Reverse(x.size));
                        self.toast = Some("Sorted by size".into());
                    }
                    KeyCode::Char('n') => {
                        self.apps.sort_by_key(|a| a.name.to_lowercase());
                        self.toast = Some("Sorted by name".into());
                    }
                    KeyCode::Char('o') => {
                        if let Some(&i) = self.apps_state.selected().and_then(|s| list.get(s)) {
                            let p = self.apps[i].path.clone();
                            self.reveal(&p);
                        }
                    }
                    KeyCode::Enter | KeyCode::Right => {
                        if let Some(&i) = self.apps_state.selected().and_then(|s| list.get(s)) {
                            let a = self.apps[i].clone();
                            self.open_uninstall_for(a);
                        }
                    }
                    _ => {}
                }
            }
            Screen::Uninstall => {
                let Some(plan) = &mut self.uninstall else {
                    return;
                };
                let n = plan.files.len();
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => move_sel(&mut plan.state, n, -1),
                    KeyCode::Down | KeyCode::Char('j') => move_sel(&mut plan.state, n, 1),
                    KeyCode::Char(' ') => {
                        if let Some(i) = plan.state.selected() {
                            plan.files[i].2 = !plan.files[i].2;
                            move_sel(&mut plan.state, n, 1);
                        }
                    }
                    KeyCode::Char('o') => {
                        if let Some(i) = plan.state.selected() {
                            let p = plan.files[i].0.clone();
                            self.reveal(&p);
                        }
                    }
                    KeyCode::Enter | KeyCode::Char('c') => {
                        let app = plan.app.clone();
                        if let Some(exe) = &app.executable
                            && util::process_running(exe)
                        {
                            self.toast = Some(format!("{} is running: quit it first.", app.name));
                            return;
                        }
                        let items: Vec<Finding> = plan
                            .files
                            .iter()
                            .filter(|(_, _, sel)| *sel)
                            .map(|(p, s, _)| {
                                let mut f = Finding::new(
                                    Category::Leftovers,
                                    format!("uninst:{}", p.display()),
                                    util::pretty(p),
                                );
                                f.paths = vec![p.clone()];
                                f.size = *s;
                                f
                            })
                            .collect();
                        if items.is_empty() {
                            self.toast = Some("Nothing selected.".into());
                        } else {
                            self.modal = Some(Modal::Confirm {
                                title: format!("Uninstall {}", app.name),
                                items,
                                from_findings: false,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Screen::Space => {
                let Some(sp) = &mut self.space else { return };
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => sp.move_sel(-1),
                    KeyCode::Down | KeyCode::Char('j') => sp.move_sel(1),
                    KeyCode::PageDown => sp.move_sel(15),
                    KeyCode::PageUp => sp.move_sel(-15),
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => sp.enter(),
                    KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                        if sp.path == util::home() {
                            self.screen = Screen::Menu; // already at the top: back to the menu
                        } else {
                            sp.up();
                        }
                    }
                    KeyCode::Esc => self.screen = Screen::Menu,
                    KeyCode::Char('o') => {
                        if let Some(p) = sp.current_path() {
                            self.reveal(&p);
                        }
                    }
                    KeyCode::Char(' ') => {
                        if let Some((p, s)) = sp.current() {
                            if sp.is_marked(&p) {
                                sp.toggle_mark(p, s);
                                sp.move_sel(1);
                            } else {
                                match actions::check_safe(&p) {
                                    Err(e) => self.toast = Some(format!("Not allowed: {e}")),
                                    Ok(()) => {
                                        sp.toggle_mark(p, s);
                                        sp.move_sel(1);
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('u') => {
                        let n = sp.marked.len();
                        sp.marked.clear();
                        if n > 0 {
                            self.toast = Some(format!("{n} mark(s) cleared"));
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Delete if !sp.marked.is_empty() => {
                        let items: Vec<Finding> = sp
                            .marked_roots()
                            .into_iter()
                            .filter(|(p, _)| actions::check_safe(p).is_ok())
                            .map(|(p, s)| {
                                let mut f = Finding::new(
                                    Category::Large,
                                    format!("space:{}", p.display()),
                                    util::pretty(&p),
                                );
                                f.paths = vec![p];
                                f.size = s;
                                f.selected = true;
                                f
                            })
                            .collect();
                        self.modal = Some(Modal::Confirm {
                            title: format!("Delete {} marked item(s)", items.len()),
                            items,
                            from_findings: false,
                        });
                    }
                    KeyCode::Char('d') | KeyCode::Delete => {
                        if let Some((p, s)) = sp.current() {
                            match actions::check_safe(&p) {
                                Err(e) => self.toast = Some(format!("Not allowed: {e}")),
                                Ok(()) => {
                                    let mut f = Finding::new(
                                        Category::Large,
                                        format!("space:{}", p.display()),
                                        util::pretty(&p),
                                    );
                                    f.paths = vec![p];
                                    f.size = s;
                                    f.selected = true;
                                    self.modal = Some(Modal::Confirm {
                                        title: "Delete".into(),
                                        items: vec![f],
                                        from_findings: false,
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn health_fix(&self) -> Option<&Fix> {
        self.health.get(self.health_state.selected()?)?.fix.as_ref()
    }

    /// Lines of the selected health item that can be ticked one by one.
    pub fn health_picks(&self) -> Option<Vec<String>> {
        self.health_fix()?.pick_labels()
    }

    fn on_health_pick_key(&mut self, k: KeyEvent) {
        let fix = self.health_fix().cloned();
        let n = self.health_picks().map_or(0, |p| p.len());
        let paths = match &fix {
            Some(Fix::Trash { paths, .. }) => paths.clone(),
            _ => vec![],
        };
        let Some(pick) = &mut self.health_pick else {
            return;
        };
        let cur = pick.state.selected().filter(|&i| i < n);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut pick.state, n, -1),
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut pick.state, n, 1),
            KeyCode::PageDown => move_sel(&mut pick.state, n, 15),
            KeyCode::PageUp => move_sel(&mut pick.state, n, -15),
            KeyCode::Char(' ') => {
                if let Some(i) = cur {
                    pick.ticked[i] = !pick.ticked[i];
                    move_sel(&mut pick.state, n, 1);
                }
            }
            KeyCode::Char('a') => {
                let all = pick.ticked.iter().all(|t| *t);
                pick.ticked.iter_mut().for_each(|t| *t = !all);
            }
            KeyCode::Char('o') => {
                if let Some(p) = cur.and_then(|i| paths.get(i)) {
                    let p = p.clone();
                    self.reveal(&p);
                }
            }
            KeyCode::Enter | KeyCode::Char('d') | KeyCode::Delete
                if matches!(fix, Some(Fix::ShellEach { .. })) =>
            {
                let Some(Fix::ShellEach { label, items, .. }) = fix else {
                    return;
                };
                let cmds: Vec<&str> = items
                    .iter()
                    .zip(&pick.ticked)
                    .filter(|(_, t)| **t)
                    .map(|(i, _)| i.1.as_str())
                    .collect();
                if cmds.is_empty() {
                    self.toast =
                        Some("Nothing ticked: tick entries with Space (a for all).".into());
                } else {
                    self.modal = Some(Modal::ConfirmShell {
                        label: format!("{label} — {} selected", cmds.len()),
                        cmd: cmds.join("; "),
                    });
                }
            }
            KeyCode::Enter | KeyCode::Char('d') | KeyCode::Delete => {
                let items: Vec<Finding> = paths
                    .iter()
                    .zip(&pick.ticked)
                    .filter(|(_, t)| **t)
                    .map(|(p, _)| {
                        let mut f = Finding::new(
                            Category::System,
                            format!("fix-file:{}", p.display()),
                            util::pretty(p),
                        );
                        f.paths = vec![p.clone()];
                        f.size = util::disk_usage(p);
                        f.selected = true;
                        f
                    })
                    .collect();
                if items.is_empty() {
                    self.toast = Some("Nothing ticked: tick files with Space (a for all).".into());
                } else {
                    let title = self
                        .health_state
                        .selected()
                        .and_then(|i| self.health.get(i))
                        .map_or("Delete".into(), |h| h.name.clone());
                    self.modal = Some(Modal::Confirm {
                        title,
                        items,
                        from_findings: false,
                    });
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc | KeyCode::Tab => {
                self.health_pick = None
            }
            _ => {}
        }
    }

    fn on_category_key(&mut self, scope: Option<Category>, k: KeyEvent) {
        let rows = self.rows(scope);
        let n = rows.len();
        let cur = self.cat_state.selected().and_then(|i| rows.get(i).copied());
        let row_of = |rows: &[Row], r: Row| rows.iter().position(|x| *x == r);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.cat_state, n, -1),
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.cat_state, n, 1),
            KeyCode::PageDown => move_sel(&mut self.cat_state, n, 15),
            KeyCode::PageUp => move_sel(&mut self.cat_state, n, -15),
            // Tab toggles: expands a line, or folds it back (also from one of its sub-items).
            KeyCode::Tab => match cur {
                Some(Row::Finding(i)) if self.findings[i].expanded => {
                    self.findings[i].expanded = false
                }
                Some(Row::Finding(i)) => self.expand(i),
                Some(Row::Child(i, _)) => {
                    self.findings[i].expanded = false;
                    let rows = self.rows(scope);
                    self.cat_state.select(row_of(&rows, Row::Finding(i)));
                }
                None => {}
            },
            KeyCode::Right | KeyCode::Char('l') => match cur {
                Some(Row::Finding(i)) => {
                    if self.findings[i].expanded {
                        move_sel(&mut self.cat_state, n, 1); // step into the first sub-item
                    } else {
                        self.expand(i);
                    }
                }
                Some(Row::Child(..)) | None => {}
            },
            KeyCode::Left | KeyCode::Char('h') => match cur {
                Some(Row::Child(i, _)) => {
                    self.findings[i].expanded = false;
                    let rows = self.rows(scope);
                    self.cat_state.select(row_of(&rows, Row::Finding(i)));
                }
                Some(Row::Finding(i)) if self.findings[i].expanded => {
                    self.findings[i].expanded = false
                }
                _ => self.back(),
            },
            KeyCode::Char(' ') => {
                match cur {
                    Some(Row::Finding(i)) => {
                        let f = &mut self.findings[i];
                        let on = f.selection_state() != 2;
                        f.set_selected(on);
                    }
                    Some(Row::Child(i, j)) => {
                        let f = &mut self.findings[i];
                        if let Some(ch) = &mut f.children {
                            ch[j].selected = !ch[j].selected;
                        }
                        f.sync_selected();
                    }
                    None => {}
                }
                move_sel(&mut self.cat_state, n, 1);
            }
            KeyCode::Char('a') => {
                let idx = self.scope_indices(scope);
                let all = idx.iter().all(|&i| self.findings[i].selection_state() == 2);
                for i in idx {
                    self.findings[i].set_selected(!all);
                }
            }
            KeyCode::Char('o') => {
                let p = match cur {
                    Some(Row::Finding(i)) => self.findings[i].paths.first().cloned(),
                    Some(Row::Child(i, j)) => self.findings[i]
                        .children
                        .as_ref()
                        .map(|c| c[j].path.clone()),
                    None => None,
                };
                if let Some(p) = p {
                    self.reveal(&p);
                }
            }
            KeyCode::Enter | KeyCode::Char('c') => {
                let items: Vec<Finding> = self
                    .scope_indices(scope)
                    .iter()
                    .filter_map(|&i| self.findings[i].to_job())
                    .collect();
                if items.is_empty() {
                    self.toast =
                        Some("Nothing selected: tick items with Space (or a for all).".into());
                } else {
                    self.modal = Some(Modal::Confirm {
                        title: scope.map_or("Smart clean", |c| c.label()).into(),
                        items,
                        from_findings: true,
                    });
                }
            }
            _ => {}
        }
    }

    fn on_modal_key(&mut self, k: KeyEvent) {
        match self.modal.take() {
            Some(Modal::Confirm {
                title,
                items,
                from_findings,
            }) => match k.code {
                KeyCode::Char('y') | KeyCode::Enter => self.start_job(title, items, from_findings),
                KeyCode::Char('p') => {
                    self.mode = if self.mode == Mode::Trash {
                        Mode::Permanent
                    } else {
                        Mode::Trash
                    };
                    self.modal = Some(Modal::Confirm {
                        title,
                        items,
                        from_findings,
                    });
                }
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => {
                    self.modal = Some(Modal::Confirm {
                        title,
                        items,
                        from_findings,
                    })
                }
            },
            Some(Modal::ConfirmShell { label, cmd }) => match k.code {
                KeyCode::Char('y') | KeyCode::Enter => self.pending_shell = Some(cmd),
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.modal = Some(Modal::ConfirmShell { label, cmd }),
            },
            Some(m @ Modal::Running { .. }) => self.modal = Some(m), // cannot be interrupted
            Some(Modal::Message { .. }) | Some(Modal::Help) | None => {}
        }
    }

    fn after_shell(&mut self) {
        self.disk = disk();
        if self.screen == Screen::Health {
            self.rescan_health();
            self.toast = Some("Command finished — checking again.".into());
        }
    }
}

pub fn move_sel(state: &mut ListState, len: usize, delta: i64) {
    if len == 0 {
        state.select(None);
        return;
    }
    let cur = state.selected().unwrap_or(0) as i64;
    let next = (cur + delta).clamp(0, len as i64 - 1);
    state.select(Some(next as usize));
}

pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new();
    let result = (|| -> Result<()> {
        loop {
            app.pump();
            terminal.draw(|f| ui::draw(f, &mut app))?;
            if event::poll(Duration::from_millis(80))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                app.on_key(k);
            }
            if let Some(cmd) = app.pending_shell.take() {
                ratatui::restore();
                println!("\n\x1b[1;36m▶ {cmd}\x1b[0m\n");
                match actions::run_shell(&cmd) {
                    Ok(st) if st.success() => println!("\n\x1b[32m✔ Done.\x1b[0m"),
                    Ok(st) => println!("\n\x1b[31m✖ The command failed ({st}).\x1b[0m"),
                    Err(e) => println!("\n\x1b[31m✖ Could not run the command: {e}\x1b[0m"),
                }
                println!("Press Enter to go back to mclean…");
                let mut s = String::new();
                let _ = std::io::stdin().read_line(&mut s);
                terminal = ratatui::init();
                app.after_shell();
            }
            if app.quit {
                return Ok(());
            }
        }
    })();
    ratatui::restore();
    if app.cleaned_session > 0 {
        println!(
            "mclean: {} cleaned this session.",
            util::fmt_size(app.cleaned_session)
        );
    }
    result
}
