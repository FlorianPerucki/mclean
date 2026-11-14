use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    System,
    Browsers,
    Dev,
    Projects,
    Large,
    Duplicates,
    Downloads,
    Leftovers,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::System,
        Category::Browsers,
        Category::Dev,
        Category::Projects,
        Category::Large,
        Category::Duplicates,
        Category::Downloads,
        Category::Leftovers,
    ];

    pub fn from_key(s: &str) -> Option<Category> {
        Some(match s {
            "system" => Category::System,
            "browsers" => Category::Browsers,
            "dev" => Category::Dev,
            "projects" => Category::Projects,
            "large" => Category::Large,
            "duplicates" => Category::Duplicates,
            "downloads" => Category::Downloads,
            "leftovers" => Category::Leftovers,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::System => "System caches & logs",
            Category::Browsers => "Browser caches",
            Category::Dev => "Developer junk",
            Category::Projects => "Project build artifacts",
            Category::Large => "Large & old files",
            Category::Duplicates => "Duplicates",
            Category::Downloads => "Forgotten downloads",
            Category::Leftovers => "App leftovers",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::System => "🧹",
            Category::Browsers => "🌐",
            Category::Dev => "🔧",
            Category::Projects => "📦",
            Category::Large => "🐘",
            Category::Duplicates => "👯",
            Category::Downloads => "📥",
            Category::Leftovers => "🧩",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Category::System => {
                "App caches, logs, crash reports, Trash. Everything here is recreated automatically by macOS or by the apps."
            }
            Category::Browsers => {
                "Web caches of Safari, Chrome, Firefox, Arc, Brave, Edge… History, passwords and cookies are NOT touched."
            }
            Category::Dev => {
                "Xcode (DerivedData, simulators, DeviceSupport), npm, pnpm, yarn, pip, Homebrew, Gradle, Cargo, Go… Everything can be downloaded again."
            }
            Category::Projects => {
                "node_modules, target/, .venv… of projects you haven't touched in a while. Rebuildable with npm install / cargo build, but it takes time."
            }
            Category::Large => {
                "Files over 500 MB in your home folder. Review them one by one: these are YOUR files."
            }
            Category::Duplicates => "Byte-identical files (xxh3 hash). One copy is always kept.",
            Category::Downloads => {
                "Installers (.dmg, .pkg…) and archives left behind in Downloads."
            }
            Category::Leftovers => {
                "Preferences, caches and containers of apps that are no longer installed."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Recreated automatically; selected by default.
    Safe,
    /// Needs a human look; not selected by default.
    Review,
}

impl Risk {
    pub fn from_key(s: &str) -> Risk {
        match s {
            "safe" => Risk::Safe,
            _ => Risk::Review,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Risk::Safe => "safe",
            Risk::Review => "review",
        }
    }
}

/// One file or folder inside a finding, selectable on its own.
#[derive(Debug, Clone)]
pub struct SubItem {
    pub path: PathBuf,
    pub size: u64,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: String,
    pub category: Category,
    pub title: String,
    pub explanation: String,
    pub paths: Vec<PathBuf>,
    pub size: u64,
    pub risk: Risk,
    #[serde(skip)]
    pub selected: bool,
    /// Process that must be closed before cleaning.
    pub process: Option<String>,
    /// Delete the contents of the folders, not the folders themselves.
    pub contents_only: bool,
    /// Permanent deletion (e.g. emptying the Trash).
    pub permanent: bool,
    /// Last use / modification time (epoch seconds), if known.
    pub last_used: Option<i64>,
    /// Plain-English description of what this is.
    pub about: Option<String>,
    /// Individual items, filled in when the user expands the finding.
    #[serde(skip)]
    pub children: Option<Vec<SubItem>>,
    #[serde(skip)]
    pub expanded: bool,
}

impl Finding {
    pub fn new(category: Category, id: impl Into<String>, title: impl Into<String>) -> Self {
        Finding {
            id: id.into(),
            category,
            title: title.into(),
            explanation: String::new(),
            paths: vec![],
            size: 0,
            risk: Risk::Review,
            selected: false,
            process: None,
            contents_only: false,
            permanent: false,
            last_used: None,
            about: None,
            children: None,
            expanded: false,
        }
    }

    /// The paths that cleaning this finding would remove, one level deep.
    pub fn item_paths(&self) -> Vec<PathBuf> {
        if !self.contents_only {
            return self.paths.clone();
        }
        let mut out = Vec::new();
        for p in &self.paths {
            if let Ok(rd) = std::fs::read_dir(p) {
                out.extend(rd.flatten().map(|e| e.path()));
            }
        }
        out.sort();
        out
    }

    /// What expanding the finding shows: its items, or the contents of its
    /// single folder (cleaning a subset of them then leaves the folder in place).
    pub fn expand_paths(&self) -> Vec<PathBuf> {
        if !self.contents_only
            && self.paths.len() == 1
            && std::fs::symlink_metadata(&self.paths[0]).is_ok_and(|m| m.is_dir())
        {
            let mut out: Vec<PathBuf> = std::fs::read_dir(&self.paths[0])
                .map(|rd| rd.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            out.sort();
            return out;
        }
        self.item_paths()
    }

    /// Size of what is currently selected in this finding.
    pub fn selected_size(&self) -> u64 {
        match &self.children {
            Some(ch) => ch.iter().filter(|c| c.selected).map(|c| c.size).sum(),
            None if self.selected => self.size,
            None => 0,
        }
    }

    /// Selection state: 0 = none, 1 = partial, 2 = all.
    pub fn selection_state(&self) -> u8 {
        match &self.children {
            Some(ch) => {
                let n = ch.iter().filter(|c| c.selected).count();
                if n == 0 {
                    0
                } else if n == ch.len() {
                    2
                } else {
                    1
                }
            }
            None => {
                if self.selected {
                    2
                } else {
                    0
                }
            }
        }
    }

    pub fn set_selected(&mut self, on: bool) {
        self.selected = on;
        if let Some(ch) = &mut self.children {
            for c in ch {
                c.selected = on;
            }
        }
    }

    /// Recompute `selected` after a child was toggled.
    pub fn sync_selected(&mut self) {
        if let Some(ch) = &self.children {
            self.selected = ch.iter().any(|c| c.selected);
        }
    }

    /// What will actually be cleaned: only the selected sub-items if the
    /// finding was expanded. Returns None if nothing is selected.
    pub fn to_job(&self) -> Option<Finding> {
        match &self.children {
            None => self.selected.then(|| self.clone()),
            Some(ch) => {
                let sel: Vec<&SubItem> = ch.iter().filter(|c| c.selected).collect();
                if sel.is_empty() {
                    return None;
                }
                let mut f = self.clone();
                f.paths = sel.iter().map(|c| c.path.clone()).collect();
                f.size = sel.iter().map(|c| c.size).sum();
                f.contents_only = false;
                f.children = None;
                Some(f)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Fail,
    Warn,
    Info,
    Ok,
}

impl Status {
    pub fn icon(self) -> &'static str {
        match self {
            Status::Ok => "✔",
            Status::Info => "ℹ",
            Status::Warn => "⚠",
            Status::Fail => "✖",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum Fix {
    /// Command run in the terminal (the TUI is suspended, sudo allowed).
    Shell { label: String, cmd: String },
    /// Files to move to the Trash.
    Trash { label: String, paths: Vec<PathBuf> },
}

impl Fix {
    pub fn label(&self) -> &str {
        match self {
            Fix::Shell { label, .. } | Fix::Trash { label, .. } => label,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthItem {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub message: String,
    pub advice: String,
    pub fix: Option<Fix>,
}

impl HealthItem {
    pub fn new(id: &str, name: &str, status: Status, message: impl Into<String>) -> Self {
        HealthItem {
            id: id.into(),
            name: name.into(),
            status,
            message: message.into(),
            advice: String::new(),
            fix: None,
        }
    }
    pub fn advice(mut self, a: impl Into<String>) -> Self {
        self.advice = a.into();
        self
    }
    pub fn fix(mut self, f: Fix) -> Self {
        self.fix = Some(f);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_selection_builds_job() {
        let mut f = Finding::new(Category::System, "x", "x");
        f.paths = vec!["/a".into()];
        f.contents_only = true;
        f.size = 30;
        f.children = Some(vec![
            SubItem {
                path: "/a/1".into(),
                size: 10,
                selected: true,
            },
            SubItem {
                path: "/a/2".into(),
                size: 20,
                selected: false,
            },
        ]);
        f.sync_selected();
        assert!(f.selected);
        assert_eq!(f.selection_state(), 1);
        assert_eq!(f.selected_size(), 10);
        let j = f.to_job().unwrap();
        assert_eq!(j.paths, vec![PathBuf::from("/a/1")]);
        assert!(!j.contents_only);
        assert_eq!(j.size, 10);
        f.set_selected(false);
        assert!(f.to_job().is_none());
    }
}
