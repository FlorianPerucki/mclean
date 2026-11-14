//! Single walk of the home folder: project artifacts, large files, duplicate
//! candidates and forgotten downloads.

use crate::model::{Category, Finding, Risk};
use crate::scan::dupes;
use crate::util;
use rayon::prelude::*;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const LARGE_FILE_BYTES: u64 = 500 * 1_000_000;
pub const DUPE_MIN_BYTES: u64 = 1_000_000;
pub const PROJECT_MIN_BYTES: u64 = 20 * 1_000_000;
const INSTALLER_EXT: [&str; 5] = ["dmg", "pkg", "mpkg", "xip", "iso"];
const ARCHIVE_EXT: [&str; 6] = ["zip", "tar", "gz", "tgz", "rar", "7z"];

/// Package folders we never descend into.
const OPAQUE_SUFFIX: [&str; 11] = [
    ".app",
    ".photoslibrary",
    ".musiclibrary",
    ".tvlibrary",
    ".imovielibrary",
    ".fcpbundle",
    ".logicx",
    ".band",
    ".xcarchive",
    ".sparsebundle",
    ".vmwarevm",
];

#[derive(Clone, Copy, Debug)]
pub struct ArtifactKind {
    pub ecosystem: &'static str,
    pub regenerate: &'static str,
}

/// A folder is an artifact only if the project's marker file sits next to it:
/// a `target/` without a Cargo.toml is never touched.
pub fn artifact_kind(name: &str, parent: &Path, dir: &Path) -> Option<ArtifactKind> {
    let has = |f: &str| parent.join(f).exists();
    let k = |_name: &str, ecosystem, regenerate| {
        Some(ArtifactKind {
            ecosystem,
            regenerate,
        })
    };
    match name {
        "node_modules" if has("package.json") => k(
            "node_modules",
            "Node.js",
            "npm install / pnpm install / yarn",
        ),
        ".next" if has("package.json") => k(".next", "Next.js", "next build"),
        ".nuxt" if has("package.json") => k(".nuxt", "Nuxt", "nuxt build"),
        ".svelte-kit" if has("package.json") => k(".svelte-kit", "SvelteKit", "vite build"),
        ".turbo" if has("package.json") => k(".turbo", "Turborepo", "turbo run build"),
        "target" if has("Cargo.toml") => k("target", "Rust", "cargo build"),
        ".venv" | "venv" if dir.join("pyvenv.cfg").exists() => {
            k("venv", "Python", "python -m venv + pip install")
        }
        ".tox" if has("tox.ini") || has("pyproject.toml") => k(".tox", "Python", "tox"),
        "Pods" if has("Podfile") => k("Pods", "CocoaPods", "pod install"),
        ".build" if has("Package.swift") => k(".build", "SwiftPM", "swift build"),
        ".gradle" | "build"
            if has("build.gradle")
                || has("build.gradle.kts")
                || has("settings.gradle")
                || has("settings.gradle.kts") =>
        {
            k("build", "Gradle", "./gradlew build")
        }
        ".dart_tool" if has("pubspec.yaml") => k(".dart_tool", "Dart/Flutter", "flutter pub get"),
        "vendor" if has("composer.json") => k("vendor", "PHP", "composer install"),
        ".terraform" if has("main.tf") || has("versions.tf") => {
            k(".terraform", "Terraform", "terraform init")
        }
        _ => None,
    }
}

const HIDDEN_ARTIFACTS: [&str; 10] = [
    ".venv",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".tox",
    ".build",
    ".gradle",
    ".dart_tool",
    ".terraform",
];

pub struct HomeScan {
    pub projects: Vec<Finding>,
    pub large: Vec<Finding>,
    pub duplicates: Vec<Finding>,
    pub downloads: Vec<Finding>,
    pub files_seen: u64,
}

#[derive(Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub blocks: u64,
    pub dev: u64,
    pub ino: u64,
    pub mtime: i64,
}

pub fn scan(root: &Path, progress: &(dyn Fn(&str) + Sync)) -> HomeScan {
    let home = root.to_path_buf();
    let downloads_dir = home.join("Downloads");
    let artifacts: Arc<Mutex<Vec<(PathBuf, ArtifactKind)>>> = Arc::new(Mutex::new(Vec::new()));
    let root_dev = std::fs::metadata(&home).map(|m| m.dev()).unwrap_or(0);

    let art = artifacts.clone();
    let home_c = home.clone();
    // Dedicated pool: the global rayon pool is busy with the other scans.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    let walker = jwalk::WalkDir::new(&home)
        .parallelism(jwalk::Parallelism::RayonNewPool(threads))
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(move |_depth, dir_path, _state, children| {
            let at_home = dir_path == home_c.as_path();
            children.retain(|r| {
                let Ok(e) = r else { return false };
                let name = e.file_name.to_string_lossy();
                if at_home && (name == "Library" || name == ".Trash") {
                    return false;
                }
                if e.file_type.is_dir()
                    && name.starts_with('.')
                    && !HIDDEN_ARTIFACTS.contains(&name.as_ref())
                {
                    return false;
                }
                true
            });
            for e in children.iter_mut().flatten() {
                if !e.file_type.is_dir() {
                    continue;
                }
                let name = e.file_name.to_string_lossy().to_string();
                let path = dir_path.join(&name);
                if let Some(kind) = artifact_kind(&name, dir_path, &path) {
                    art.lock().unwrap().push((path, kind));
                    e.read_children = None;
                } else if OPAQUE_SUFFIX.iter().any(|s| name.ends_with(s)) || name == ".git" {
                    e.read_children = None;
                }
            }
        });

    let files: Mutex<Vec<FileInfo>> = Mutex::new(Vec::new());
    let seen = AtomicU64::new(0);
    for entry in walker.into_iter().flatten() {
        let n = seen.fetch_add(1, Ordering::Relaxed);
        if n.is_multiple_of(20_000) && n > 0 {
            progress(&format!("{} files scanned", n));
        }
        if !entry.file_type.is_file() {
            continue;
        }
        let Ok(m) = entry.metadata() else { continue };
        if m.dev() != root_dev {
            continue; // another volume mounted inside the home folder
        }
        if m.len() < DUPE_MIN_BYTES {
            continue;
        }
        files.lock().unwrap().push(FileInfo {
            path: entry.path(),
            size: m.len(),
            blocks: m.blocks() * 512,
            dev: m.dev(),
            ino: m.ino(),
            mtime: m.mtime(),
        });
    }
    let files = files.into_inner().unwrap();
    let files_seen = seen.load(Ordering::Relaxed);

    // ── Forgotten downloads ──
    let mut downloads = Vec::new();
    let mut claimed: std::collections::HashSet<PathBuf> = Default::default();
    for f in &files {
        let Ok(rel) = f.path.strip_prefix(&downloads_dir) else {
            continue;
        };
        if rel.components().count() > 2 {
            continue;
        }
        let ext = f
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let age = util::age_days(f.mtime);
        let (risk, why) = if INSTALLER_EXT.contains(&ext.as_str()) && age >= 14 {
            (
                Risk::Safe,
                "Installer most likely already used. You can download it again if needed.",
            )
        } else if ARCHIVE_EXT.contains(&ext.as_str()) && age >= 30 {
            (Risk::Review, "Old archive: check that it was extracted.")
        } else if age >= 180 && f.size >= 50_000_000 {
            (
                Risk::Review,
                "Large file sitting in Downloads for more than 6 months.",
            )
        } else {
            continue;
        };
        let name = f.path.file_name().unwrap().to_string_lossy().to_string();
        let mut fi = Finding::new(
            Category::Downloads,
            format!("dl:{}", f.path.display()),
            name,
        );
        fi.explanation = why.into();
        fi.paths = vec![f.path.clone()];
        fi.size = f.blocks;
        fi.risk = risk;
        fi.selected = risk == Risk::Safe;
        fi.last_used = Some(f.mtime);
        claimed.insert(f.path.clone());
        downloads.push(fi);
    }

    // ── Large files ──
    progress("looking at large files");
    let mut large: Vec<Finding> = files
        .par_iter()
        .filter(|f| f.size >= LARGE_FILE_BYTES && !claimed.contains(&f.path))
        .map(|f| {
            let name = f.path.file_name().unwrap().to_string_lossy().to_string();
            let used = last_used(&f.path).unwrap_or(f.mtime);
            let mut fi = Finding::new(Category::Large, format!("large:{}", f.path.display()), name);
            fi.explanation = format!(
                "Last used: {}. Location: {}",
                util::fmt_age(used),
                util::pretty(f.path.parent().unwrap_or(Path::new("/")))
            );
            fi.paths = vec![f.path.clone()];
            fi.size = f.blocks.max(1);
            fi.risk = Risk::Review;
            fi.last_used = Some(used);
            fi
        })
        .collect();
    // "Dataless" files (iCloud, not downloaded) take no local space.
    large.retain(|f| f.size > 1);
    large.sort_by_key(|x| std::cmp::Reverse(x.size));

    // ── Duplicates ──
    progress("looking for duplicates");
    let candidates: Vec<FileInfo> = files
        .into_iter()
        .filter(|f| f.blocks > 0 && !claimed.contains(&f.path))
        .collect();
    let duplicates = dupes::find(candidates, &home, progress);

    // ── Project artifacts ──
    progress("measuring project artifacts");
    let arts = Arc::try_unwrap(artifacts)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or_default();
    let mut projects: Vec<Finding> = arts
        .into_par_iter()
        .filter_map(|(path, kind)| {
            let size = util::disk_usage(&path);
            if size < PROJECT_MIN_BYTES {
                return None;
            }
            let project = path.parent()?.to_path_buf();
            let touched = project_last_touch(&project, &path);
            let pname = project.file_name()?.to_string_lossy().to_string();
            let dname = path.file_name()?.to_string_lossy().to_string();
            let mut fi = Finding::new(
                Category::Projects,
                format!("proj:{}", path.display()),
                format!("{} · {}", pname, dname),
            );
            fi.explanation = format!(
                "{} — project last modified {}. To rebuild: {}. Folder: {}",
                kind.ecosystem,
                util::fmt_age(touched),
                kind.regenerate,
                util::pretty(&project)
            );
            fi.paths = vec![path];
            fi.size = size;
            fi.risk = Risk::Review;
            // Pre-select projects untouched for more than 90 days.
            fi.selected = util::age_days(touched) >= 90;
            fi.last_used = Some(touched);
            Some(fi)
        })
        .collect();
    projects.sort_by_key(|x| std::cmp::Reverse(x.size));

    downloads.sort_by_key(|x| std::cmp::Reverse(x.size));
    HomeScan {
        projects,
        large,
        duplicates,
        downloads,
        files_seen,
    }
}

/// Last modification time of a project, ignoring its artifacts.
fn project_last_touch(project: &Path, artifact: &Path) -> i64 {
    let mut latest = 0;
    if let Ok(rd) = std::fs::read_dir(project) {
        for e in rd.flatten() {
            let p = e.path();
            if p == artifact {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if artifact_kind(&name, project, &p).is_some() {
                continue;
            }
            if let Some(m) = util::mtime(&p) {
                latest = latest.max(m);
            }
        }
    }
    if latest == 0 {
        util::mtime(project).unwrap_or(0)
    } else {
        latest
    }
}

/// Last opened date according to Spotlight (macOS only).
pub fn last_used(p: &Path) -> Option<i64> {
    if !util::is_macos() {
        return None;
    }
    let out = util::run(
        "mdls",
        &["-raw", "-name", "kMDItemLastUsedDate", &p.to_string_lossy()],
    )?;
    let s = out.trim();
    chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z")
        .ok()
        .map(|d| d.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_needs_marker() {
        let d = std::env::temp_dir().join(format!("mclean-art-{}", std::process::id()));
        std::fs::create_dir_all(d.join("p/target")).unwrap();
        assert!(artifact_kind("target", &d.join("p"), &d.join("p/target")).is_none());
        std::fs::write(d.join("p/Cargo.toml"), "").unwrap();
        assert!(artifact_kind("target", &d.join("p"), &d.join("p/target")).is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
