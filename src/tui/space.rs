//! Disk space explorer (in the spirit of Space Lens / ncdu).

use crate::util;
use ratatui::widgets::ListState;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

pub struct SpaceView {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub state: ListState,
    cache: HashMap<PathBuf, u64>,
    rx: Option<Receiver<(PathBuf, u64)>>,
    history: Vec<(PathBuf, usize)>,
    /// The user moved the selection since the folder was loaded.
    touched: bool,
    /// Files and folders marked for deletion (Space), kept while browsing.
    /// Value: size when known (0 while a folder is still being measured).
    pub marked: BTreeMap<PathBuf, u64>,
}

impl SpaceView {
    pub fn new(path: PathBuf) -> SpaceView {
        let mut v = SpaceView {
            path,
            entries: vec![],
            state: ListState::default(),
            cache: HashMap::new(),
            rx: None,
            history: vec![],
            touched: false,
            marked: BTreeMap::new(),
        };
        v.load();
        v
    }

    pub fn pending(&self) -> usize {
        self.entries.iter().filter(|e| e.size.is_none()).count()
    }

    pub fn total(&self) -> u64 {
        self.entries.iter().filter_map(|e| e.size).sum()
    }

    fn load(&mut self) {
        self.touched = false;
        self.entries.clear();
        let mut todo = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.path) {
            for e in rd.flatten() {
                let Ok(m) = std::fs::symlink_metadata(e.path()) else {
                    continue;
                };
                if m.file_type().is_symlink() {
                    continue;
                }
                let p = e.path();
                let size = if m.is_dir() {
                    self.cache.get(&p).copied()
                } else {
                    Some(util::disk_usage(&p))
                };
                if size.is_none() {
                    todo.push(p.clone());
                }
                self.entries.push(Entry {
                    name: e.file_name().to_string_lossy().to_string(),
                    path: p,
                    is_dir: m.is_dir(),
                    size,
                });
            }
        }
        self.sort();
        self.state.select(if self.entries.is_empty() {
            None
        } else {
            Some(0)
        });
        if !todo.is_empty() {
            let (tx, rx) = channel();
            self.rx = Some(rx);
            std::thread::spawn(move || {
                use rayon::prelude::*;
                todo.par_iter().for_each(|p| {
                    let _ = tx.send((p.clone(), util::disk_usage(p)));
                });
            });
        }
    }

    fn sort(&mut self) {
        let sel = self.current_path();
        self.entries.sort_by(|a, b| {
            b.size
                .unwrap_or(0)
                .cmp(&a.size.unwrap_or(0))
                .then(a.name.cmp(&b.name))
        });
        if let Some(sel) = sel
            && let Some(i) = self.entries.iter().position(|e| e.path == sel)
        {
            self.state.select(Some(i));
        }
    }

    pub fn pump(&mut self) {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            while let Ok((p, s)) = rx.try_recv() {
                self.cache.insert(p.clone(), s);
                if let Some(m) = self.marked.get_mut(&p) {
                    *m = s;
                }
                if let Some(e) = self.entries.iter_mut().find(|e| e.path == p) {
                    e.size = Some(s);
                    changed = true;
                }
            }
        }
        if changed {
            self.sort();
            if !self.touched && !self.entries.is_empty() {
                self.state.select(Some(0));
            }
        }
    }

    pub fn current(&self) -> Option<(PathBuf, u64)> {
        let e = self.entries.get(self.state.selected()?)?;
        Some((e.path.clone(), e.size.unwrap_or(0)))
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.current().map(|c| c.0)
    }

    pub fn is_marked(&self, p: &Path) -> bool {
        self.marked.contains_key(p)
    }

    /// Marks or unmarks a path. Returns true when it is now marked.
    pub fn toggle_mark(&mut self, p: PathBuf, size: u64) -> bool {
        if self.marked.remove(&p).is_some() {
            false
        } else {
            self.marked.insert(p, size);
            true
        }
    }

    pub fn marked_total(&self) -> u64 {
        self.marked_roots().iter().map(|(_, s)| s).sum()
    }

    /// Marked paths, without those inside another marked folder
    /// (deleting the folder already takes them).
    pub fn marked_roots(&self) -> Vec<(PathBuf, u64)> {
        self.marked
            .iter()
            .filter(|(p, _)| !p.ancestors().skip(1).any(|a| self.marked.contains_key(a)))
            .map(|(p, s)| (p.clone(), *s))
            .collect()
    }

    /// Drops the marks of paths that no longer exist (after a deletion).
    pub fn prune_marks(&mut self) {
        self.marked
            .retain(|p, _| std::fs::symlink_metadata(p).is_ok());
    }

    pub fn move_sel(&mut self, d: i64) {
        self.touched = true;
        super::move_sel(&mut self.state, self.entries.len(), d);
    }

    pub fn enter(&mut self) {
        let Some(i) = self.state.selected() else {
            return;
        };
        let Some(e) = self.entries.get(i) else { return };
        if !e.is_dir {
            return;
        }
        self.history.push((self.path.clone(), i));
        self.path = e.path.clone();
        self.load();
    }

    pub fn up(&mut self) {
        let Some(parent) = self.path.parent().map(Path::to_path_buf) else {
            return;
        };
        let prev = self.path.clone();
        self.path = parent;
        self.history.pop();
        self.load();
        if let Some(i) = self.entries.iter().position(|e| e.path == prev) {
            self.state.select(Some(i));
            self.touched = true;
        }
    }

    /// After a deletion: invalidate the cache of the current folder and its parents.
    pub fn refresh(&mut self) {
        let keys: Vec<PathBuf> = self
            .cache
            .keys()
            .filter(|k| self.path.starts_with(k) || k.starts_with(&self.path))
            .cloned()
            .collect();
        for k in keys {
            self.cache.remove(&k);
        }
        let sel = self.state.selected();
        self.load();
        if let Some(s) = sel {
            super::move_sel(&mut self.state, self.entries.len(), 0);
            self.state
                .select(Some(s.min(self.entries.len().saturating_sub(1))));
        }
    }
}
