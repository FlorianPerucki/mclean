//! Installed apps, their related files (full uninstall) and leftovers of
//! removed apps.

use crate::model::{Category, Finding, Risk};
use crate::util;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub bundle_id: String,
    pub path: PathBuf,
    pub executable: Option<String>,
    pub size: u64,
}

fn read_info(app: &Path) -> Option<App> {
    let info = app.join("Contents/Info.plist");
    let v = plist::Value::from_file(&info).ok()?;
    let d = v.as_dictionary()?;
    let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(|s| s.to_string());
    let bundle_id = s("CFBundleIdentifier")?;
    let name = app
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .or_else(|| s("CFBundleName"))?;
    Some(App {
        name,
        bundle_id,
        path: app.to_path_buf(),
        executable: s("CFBundleExecutable"),
        size: 0,
    })
}

fn app_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        util::home().join("Applications"),
        PathBuf::from("/System/Applications"),
        util::home().join("Library/Application Support/Setapp/Applications"),
    ]
}

/// All apps (system ones included), to know which bundle IDs are still alive.
pub fn installed(include_system: bool) -> Vec<App> {
    let mut out = Vec::new();
    for dir in app_dirs() {
        if !include_system && dir.starts_with("/System") {
            continue;
        }
        collect(&dir, 0, &mut out);
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<App>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        if p.extension().is_some_and(|x| x == "app") {
            if let Some(a) = read_info(&p) {
                out.push(a);
            }
        } else if depth < 2 {
            collect(&p, depth + 1, out);
        }
    }
}

/// Places where apps leave files behind.
const LIB_LOCATIONS: [&str; 14] = [
    "Application Support",
    "Caches",
    "Containers",
    "Group Containers",
    "Preferences",
    "Preferences/ByHost",
    "Saved Application State",
    "Logs",
    "HTTPStorages",
    "WebKit",
    "LaunchAgents",
    "Application Scripts",
    "Cookies",
    "Caches/com.apple.helpd/Generated",
];

fn strip_known_ext(name: &str) -> &str {
    for ext in [
        ".plist",
        ".savedState",
        ".binarycookies",
        ".binarycookies_tmp",
    ] {
        if let Some(s) = name.strip_suffix(ext) {
            return s;
        }
    }
    name
}

/// Files belonging to an installed app (for a full uninstall).
pub fn related_files(app: &App) -> Vec<PathBuf> {
    let lib = util::home().join("Library");
    let id = app.bundle_id.to_lowercase();
    let name = app.name.to_lowercase();
    let mut out = Vec::new();
    for loc in LIB_LOCATIONS {
        let Ok(rd) = std::fs::read_dir(lib.join(loc)) else {
            continue;
        };
        for e in rd.flatten() {
            let raw = e.file_name().to_string_lossy().to_lowercase();
            let n = strip_known_ext(&raw);
            let by_id = n == id
                || n.starts_with(&format!("{id}."))
                || (loc == "Group Containers" && n.ends_with(&format!(".{id}")))
                || (loc == "LaunchAgents" && n.starts_with(&id));
            // Match by name only if it is distinctive enough.
            let by_name = name.len() >= 4
                && n == name
                && matches!(loc, "Application Support" | "Caches" | "Logs");
            if by_id || by_name {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Does this look like a bundle id (com.vendor.app)?
fn looks_like_bundle_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        && matches!(
            parts[0],
            "com"
                | "org"
                | "net"
                | "io"
                | "de"
                | "fr"
                | "co"
                | "app"
                | "dev"
                | "ru"
                | "jp"
                | "uk"
                | "me"
                | "info"
                | "cc"
                | "tv"
                | "us"
                | "ch"
                | "nl"
                | "se"
                | "at"
                | "be"
                | "it"
                | "es"
                | "pl"
                | "cn"
                | "ai"
        )
}

fn vendor(id: &str) -> String {
    id.split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".")
        .to_lowercase()
}

/// Leftovers of uninstalled apps: bundle ids found in ~/Library whose vendor is
/// shared by NO installed app (conservative approach).
pub fn leftovers(progress: &(dyn Fn(&str) + Sync)) -> Vec<Finding> {
    let apps = installed(true);
    progress(&format!("{} installed apps", apps.len()));
    let mut vendors: HashSet<String> = apps.iter().map(|a| vendor(&a.bundle_id)).collect();
    // Services started by launchd (agents without an .app): leave their files alone.
    if let Some(out) = util::run("launchctl", &["list"]) {
        for line in out.lines().skip(1) {
            if let Some(label) = line.split_whitespace().nth(2)
                && looks_like_bundle_id(label)
            {
                vendors.insert(vendor(label));
            }
        }
    }
    for v in [
        "com.apple",
        "com.microsoft",
        "com.google",
        "org.mozilla",
        "com.adobe",
    ] {
        // vendors whose shared components outlive the removal of one app
        vendors.insert(v.into());
    }

    let lib = util::home().join("Library");
    let mut by_id: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    // Caches and Logs are already covered by the cleaning rules.
    for loc in [
        "Application Support",
        "Containers",
        "Preferences",
        "Saved Application State",
        "HTTPStorages",
        "WebKit",
        "Application Scripts",
        "Cookies",
    ] {
        let Ok(rd) = std::fs::read_dir(lib.join(loc)) else {
            continue;
        };
        for e in rd.flatten() {
            let raw = e.file_name().to_string_lossy().to_string();
            let id = strip_known_ext(&raw).to_string();
            if !looks_like_bundle_id(&id) || vendors.contains(&vendor(&id)) {
                continue;
            }
            by_id.entry(id).or_default().push(e.path());
        }
    }
    let mut out: Vec<Finding> = by_id
        .into_par_iter()
        .map(|(id, paths)| {
            let size: u64 = paths.iter().map(|p| util::disk_usage(p)).sum();
            let mut f = Finding::new(Category::Leftovers, format!("left:{id}"), id.clone());
            f.explanation = format!(
                "No app from vendor \"{}\" is installed. {} item(s) found in ~/Library. \
                 Make sure this is not a command-line tool you still use.",
                vendor(&id),
                paths.len()
            );
            f.last_used = paths.iter().filter_map(|p| util::mtime(p)).max();
            f.about = crate::describe::describe(&id);
            f.paths = paths;
            f.size = size;
            f.risk = Risk::Review;
            f
        })
        .filter(|f| f.size >= 50_000)
        .collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.size));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_ids() {
        assert!(looks_like_bundle_id("com.spotify.client"));
        assert!(looks_like_bundle_id("org.videolan.vlc"));
        assert!(!looks_like_bundle_id("Google"));
        assert!(!looks_like_bundle_id("com.foo"));
        assert!(!looks_like_bundle_id("x.y.z"));
        assert_eq!(vendor("com.Spotify.client"), "com.spotify");
        assert_eq!(strip_known_ext("com.foo.bar.plist"), "com.foo.bar");
    }
}
