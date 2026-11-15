//! Cleaning: safety checks, Trash, permanent deletion, journal.

use crate::model::Finding;
use crate::util;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Trash,
    Permanent,
}

#[derive(Debug, Default)]
pub struct Report {
    pub freed: u64,
    pub removed: usize,
    pub errors: Vec<(PathBuf, String)>,
}

/// Central safety net: refuses any path outside the allowed area.
pub fn check_safe(p: &Path) -> Result<(), String> {
    let home = util::home();
    if !p.is_absolute() {
        return Err("relative path".into());
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("path contains ..".into());
    }
    let protected_exact = [
        "",
        "Library",
        "Documents",
        "Desktop",
        "Downloads",
        "Pictures",
        "Movies",
        "Music",
        "Public",
        "Applications",
        ".Trash",
        ".config",
        ".local",
        ".local/share",
        ".cache",
        "Library/Application Support",
        "Library/Caches",
        "Library/Preferences",
        "Library/Containers",
        "Library/Group Containers",
        "Library/Logs",
        "Library/Developer",
        "Library/LaunchAgents",
        "Library/Saved Application State",
        "Library/Mail",
    ];
    for e in protected_exact {
        let q = if e.is_empty() {
            home.clone()
        } else {
            home.join(e)
        };
        if p == q {
            return Err(format!("{} is protected", util::pretty(p)));
        }
    }
    let protected_prefix = [
        home.join("Library/Keychains"),
        home.join(".ssh"),
        home.join(".gnupg"),
        home.join("Library/Mobile Documents"),
        home.join("Library/CloudStorage"),
        home.join("Library/Mail"),
        home.join("Library/Messages"),
        home.join("Library/Photos"),
        PathBuf::from("/System"),
        PathBuf::from("/usr"),
        PathBuf::from("/bin"),
        PathBuf::from("/sbin"),
        PathBuf::from("/private"),
        PathBuf::from("/Library"),
    ];
    for pre in &protected_prefix {
        if p.starts_with(pre) {
            return Err(format!("{} is in a protected area", util::pretty(p)));
        }
    }
    if p.components()
        .any(|c| c.as_os_str().to_string_lossy().ends_with(".photoslibrary"))
    {
        return Err("photo library".into());
    }
    let is_app = p.extension().is_some_and(|x| x == "app");
    if p.starts_with(&home) {
        return Ok(());
    }
    if is_app && p.starts_with("/Applications") {
        return Ok(());
    }
    Err(format!("{} is outside your home folder", p.display()))
}

fn trash_one(p: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(p).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(p).map_err(|e| e.to_string())
    }
}

fn delete_one(p: &Path) -> Result<(), String> {
    let m = std::fs::symlink_metadata(p).map_err(|e| e.to_string())?;
    if m.is_dir() {
        // remove_dir_all does not follow symlinks.
        std::fs::remove_dir_all(p).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(p).map_err(|e| e.to_string())
    }
}

pub fn journal_path() -> PathBuf {
    if util::is_macos() {
        util::home().join("Library/Logs/mclean/journal.tsv")
    } else {
        util::home().join(".local/state/mclean/journal.tsv")
    }
}

fn journal(lines: &[String]) {
    let p = journal_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        for l in lines {
            let _ = writeln!(f, "{l}");
        }
    }
}

/// Cleans a list of findings. `on_item` is called before each finding.
pub fn clean(
    findings: &[Finding],
    mode: Mode,
    dry_run: bool,
    on_item: &mut dyn FnMut(usize, &str),
) -> Report {
    let mut rep = Report::default();
    let mut log = Vec::new();
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for (i, f) in findings.iter().enumerate() {
        on_item(i, &f.title);
        if let Some(proc_name) = &f.process
            && util::process_running(proc_name)
        {
            rep.errors.push((
                f.paths.first().cloned().unwrap_or_default(),
                format!("{proc_name} is running: quit it and try again"),
            ));
            continue;
        }
        let mode = if f.permanent { Mode::Permanent } else { mode };
        let mut failed_size = 0;
        for t in f.item_paths() {
            if let Err(e) = check_safe(&t) {
                rep.errors.push((t.clone(), format!("refused: {e}")));
                failed_size += util::disk_usage(&t);
                continue;
            }
            if dry_run {
                rep.removed += 1;
                continue;
            }
            let r = match mode {
                Mode::Trash => trash_one(&t),
                Mode::Permanent => delete_one(&t),
            };
            match r {
                Ok(()) => {
                    rep.removed += 1;
                    let how = if mode == Mode::Trash {
                        "trashed"
                    } else {
                        "deleted"
                    };
                    log.push(format!("{stamp}\t{how}\t{}", t.display()));
                }
                Err(e) => {
                    failed_size += util::disk_usage(&t);
                    rep.errors.push((t, e));
                }
            }
        }
        rep.freed += f.size.saturating_sub(failed_size);
    }
    if !dry_run {
        journal(&log);
    }
    rep
}

/// Runs a command in the terminal (inherited stdin/stdout, sudo allowed).
pub fn run_shell(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety() {
        let h = util::home();
        assert!(check_safe(&h).is_err());
        assert!(check_safe(&h.join("Library")).is_err());
        assert!(check_safe(&h.join("Library/Caches")).is_err());
        assert!(check_safe(&h.join("Documents")).is_err());
        assert!(check_safe(&h.join(".ssh/id_rsa")).is_err());
        assert!(check_safe(&h.join("Library/Mobile Documents/x")).is_err());
        assert!(
            check_safe(&h.join("Pictures/Photos Library.photoslibrary/originals/a.jpg")).is_err()
        );
        assert!(check_safe(Path::new("/System/Library")).is_err());
        assert!(check_safe(Path::new("/etc/hosts")).is_err());
        assert!(check_safe(Path::new("/Applications/Safari.app/../../etc")).is_err());
        assert!(check_safe(&h.join("Library/Caches/com.foo")).is_ok());
        assert!(check_safe(&h.join("Library/Mail Downloads/x.pdf")).is_ok());
        assert!(check_safe(&h.join("dev/proj/node_modules")).is_ok());
        assert!(check_safe(Path::new("/Applications/Foo.app")).is_ok());
        assert!(check_safe(Path::new("/Applications/Utilities")).is_err());
    }

    #[test]
    fn clean_contents_permanently() {
        let d = util::home().join(format!(".mclean-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join("cache/sub")).unwrap();
        std::fs::write(d.join("cache/a"), vec![0u8; 10_000]).unwrap();
        std::fs::write(d.join("cache/sub/b"), vec![0u8; 10_000]).unwrap();
        let mut f = Finding::new(crate::model::Category::System, "t", "t");
        f.paths = vec![d.join("cache")];
        f.contents_only = true;
        f.size = 20_000;
        f.selected = true;
        let r = clean(&[f], Mode::Permanent, false, &mut |_, _| {});
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.removed, 2);
        assert!(d.join("cache").exists());
        assert_eq!(std::fs::read_dir(d.join("cache")).unwrap().count(), 0);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn clean_only_selected_children() {
        use crate::model::SubItem;
        let d = util::home().join(format!(".mclean-test2-{}", std::process::id()));
        std::fs::create_dir_all(d.join("logs")).unwrap();
        std::fs::write(d.join("logs/keep.log"), b"k").unwrap();
        std::fs::write(d.join("logs/drop.log"), b"d").unwrap();
        let mut f = Finding::new(crate::model::Category::System, "t", "t");
        f.paths = vec![d.join("logs")];
        f.contents_only = true;
        f.children = Some(vec![
            SubItem {
                path: d.join("logs/keep.log"),
                size: 1,
                selected: false,
            },
            SubItem {
                path: d.join("logs/drop.log"),
                size: 1,
                selected: true,
            },
        ]);
        let job = f.to_job().unwrap();
        let r = clean(&[job], Mode::Permanent, false, &mut |_, _| {});
        assert!(r.errors.is_empty());
        assert!(d.join("logs/keep.log").exists());
        assert!(!d.join("logs/drop.log").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
