use std::collections::HashSet;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Expands `~` to the home folder.
pub fn expand(p: &str) -> PathBuf {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// Displays a path with the home folder shortened to `~`.
pub fn pretty(p: &Path) -> String {
    let h = home();
    match p.strip_prefix(&h) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Matches a name against a pattern containing `*` and `?`.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Expands a path such as `~/Library/Caches/*/foo` (wildcards per component).
pub fn glob(pattern: &str) -> Vec<PathBuf> {
    let full = expand(pattern);
    let mut current: Vec<PathBuf> = vec![PathBuf::from("/")];
    for comp in full.components().skip(1) {
        let c = comp.as_os_str().to_string_lossy().to_string();
        let mut next = Vec::new();
        if c.contains('*') || c.contains('?') {
            for base in &current {
                if let Ok(rd) = std::fs::read_dir(base) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if wildcard(&c, &name) {
                            next.push(e.path());
                        }
                    }
                }
            }
        } else {
            for base in &current {
                let p = base.join(&c);
                if std::fs::symlink_metadata(&p).is_ok() {
                    next.push(p);
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current.sort();
    current
}

/// Real space used on disk (allocated blocks), without following symlinks and
/// counting hard links only once.
pub fn disk_usage(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.blocks() * 512;
    }
    let seen: Mutex<HashSet<(u64, u64)>> = Mutex::new(HashSet::new());
    let mut total = meta.blocks() * 512;
    // Serial walk: parallelism happens in the caller (rayon). jwalk on the global
    // rayon pool gives up when the pool is busy.
    for entry in jwalk::WalkDir::new(path)
        .parallelism(jwalk::Parallelism::Serial)
        .skip_hidden(false)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .flatten()
    {
        if let Ok(m) = entry.metadata() {
            if m.nlink() > 1 && !m.is_dir() {
                let mut s = seen.lock().unwrap();
                if !s.insert((m.dev(), m.ino())) {
                    continue;
                }
            }
            total += m.blocks() * 512;
        }
    }
    total
}

pub fn fmt_size(b: u64) -> String {
    // macOS uses decimal units (1 GB = 1000^3 bytes).
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", b, U[0])
    } else if v >= 100.0 {
        format!("{:.0} {}", v, U[i])
    } else {
        format!("{:.1} {}", v, U[i])
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn mtime(p: &Path) -> Option<i64> {
    std::fs::symlink_metadata(p).ok().map(|m| m.mtime())
}

pub fn age_days(ts: i64) -> i64 {
    (now() - ts).max(0) / 86400
}

pub fn fmt_age(ts: i64) -> String {
    let d = age_days(ts);
    if d < 1 {
        "today".into()
    } else if d < 31 {
        format!("{d} days ago")
    } else if d < 365 {
        format!("{} months ago", d / 30)
    } else {
        format!("{:.1} years ago", d as f64 / 365.0)
    }
}

/// Runs a command with a timeout. Returns stdout on exit code 0 (or always if
/// `any_status`).
pub fn run_full(cmd: &str, args: &[&str], timeout: Duration, any_status: bool) -> Option<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    }?;
    let mut out = out_t.join().unwrap_or_default();
    let err = err_t.join().unwrap_or_default();
    if status.success() || any_status {
        if out.trim().is_empty() {
            out = err;
        } else if any_status {
            out.push('\n');
            out.push_str(&err);
        }
        Some(out)
    } else {
        None
    }
}

pub fn run(cmd: &str, args: &[&str]) -> Option<String> {
    run_full(cmd, args, Duration::from_secs(10), false)
}

pub fn has_cmd(cmd: &str) -> bool {
    if cmd.contains('/') {
        return Path::new(cmd).exists();
    }
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

/// Is a process with this exact name running?
pub fn process_running(name: &str) -> bool {
    run("pgrep", &["-x", name])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_works() {
        assert!(wildcard("*", "abc"));
        assert!(wildcard("com.apple.*", "com.apple.Safari"));
        assert!(!wildcard("com.apple.*", "com.google.Chrome"));
        assert!(wildcard("*.dmg", "foo.dmg"));
        assert!(wildcard("a?c", "abc"));
        assert!(!wildcard("*.dmg", "foo.dmg.part"));
    }

    #[test]
    fn size_format() {
        assert_eq!(fmt_size(999), "999 B");
        assert_eq!(fmt_size(1_500_000), "1.5 MB");
        assert_eq!(fmt_size(250_000_000_000), "250 GB");
    }
}
