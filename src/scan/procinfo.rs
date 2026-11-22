//! Who is a process? Full executable path, owning app and a plain-English note,
//! so a list of PIDs (swap users, …) means something to the user.

use crate::util::{self, run};
use std::collections::HashMap;
use std::path::Path;

pub struct ProcInfo {
    /// Full path of the executable (or the short name when ps is unavailable).
    pub exe: String,
    /// Command line, used to tell helper processes apart.
    pub args: String,
}

impl ProcInfo {
    pub fn name(&self) -> &str {
        self.exe.rsplit('/').next().unwrap_or(&self.exe)
    }
}

/// Looks the given PIDs up with ps.
pub fn lookup(pids: &[u32]) -> HashMap<u32, ProcInfo> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let comm = run("ps", &["-o", "pid=,comm=", "-p", &list]).unwrap_or_default();
    let args = run("ps", &["-ww", "-o", "pid=,args=", "-p", &list]).unwrap_or_default();
    let args = parse_pid_lines(&args);
    parse_pid_lines(&comm)
        .into_iter()
        .map(|(pid, exe)| {
            let args = args.get(&pid).cloned().unwrap_or_default();
            (pid, ProcInfo { exe, args })
        })
        .collect()
}

/// "  812 /Applications/Foo.app/…" lines → pid → rest of the line.
fn parse_pid_lines(out: &str) -> HashMap<u32, String> {
    out.lines()
        .filter_map(|l| {
            let (pid, rest) = l.trim_start().split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, rest.trim().to_string()))
        })
        .collect()
}

/// Outermost app bundle of an executable: "/Applications/Google Chrome.app/…/Helper" → "Google Chrome".
pub fn app_of(exe: &str) -> Option<String> {
    let i = exe
        .find(".app/")
        .or_else(|| exe.strip_suffix(".app").map(str::len))?;
    let name = exe[..i].rsplit('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Apps that run virtual machines, found among the running executables.
pub fn running_vm_apps() -> Vec<&'static str> {
    let all = run("ps", &["-axo", "comm="]).unwrap_or_default();
    vm_apps_in(&all)
}

fn vm_apps_in(ps_comm: &str) -> Vec<&'static str> {
    const VM_APPS: &[(&str, &str)] = &[
        ("/Docker.app/", "Docker Desktop"),
        ("/OrbStack.app/", "OrbStack"),
        ("/UTM.app/", "UTM"),
        ("/Parallels Desktop.app/", "Parallels"),
        ("/VMware Fusion.app/", "VMware Fusion"),
        ("/Claude.app/", "Claude (desktop workspace)"),
        ("/limactl", "Lima / Colima"),
        ("/podman", "Podman"),
        ("/tart", "Tart"),
        ("/multipassd", "Multipass"),
    ];
    let mut found: Vec<&'static str> = VM_APPS
        .iter()
        .filter(|(pat, _)| ps_comm.lines().any(|l| l.contains(pat)))
        .map(|(_, name)| *name)
        .collect();
    found.dedup();
    found
}

/// One line saying what the process is.
pub fn note(p: &ProcInfo, vm_apps: &[&str]) -> String {
    let name = p.name();
    let app = app_of(&p.exe);
    let of_app = |what: &str| match &app {
        Some(a) => format!("{a}: {what}"),
        None => what.to_string(),
    };
    let arg = |flag: &str| p.args.split_whitespace().any(|a| a.starts_with(flag));
    if name.starts_with("com.apple.Virtualization.VirtualMachine") {
        let who = if vm_apps.is_empty() {
            "started by an app such as Docker, OrbStack, UTM or Claude".to_string()
        } else {
            format!("VM apps running now: {}", vm_apps.join(", "))
        };
        return format!(
            "a virtual machine (Apple Virtualization framework); its RAM counts here — {who}. Stop the VM in its app to free it."
        );
    }
    if name == "plugin-container" {
        let kind = match p.args.split_whitespace().last().unwrap_or("") {
            "tab" => "web page content (one or more tabs)",
            "gpu" => "graphics (GPU) process",
            "rdd" => "media decoding",
            "socket" => "network process",
            "utility" => "utility process (audio, extensions…)",
            "extension" => "extensions",
            _ => "helper process",
        };
        let app = app.as_deref().unwrap_or("Firefox");
        return format!("{app}: {kind}. Closing tabs frees it.");
    }
    if arg("--type=renderer") {
        return if arg("--extension-process") {
            of_app("browser extension")
        } else {
            of_app(
                "page / window renderer (one or more tabs or windows). Closing tabs or windows frees it.",
            )
        };
    }
    if arg("--type=gpu-process") || name.contains("GPU") {
        return of_app("graphics (GPU) process");
    }
    if arg("--type=utility") {
        return if p.args.contains("NetworkService") {
            of_app("network process")
        } else if p.args.contains("audio") {
            of_app("audio process")
        } else {
            of_app("utility process")
        };
    }
    const KNOWN: &[(&str, &str)] = &[
        ("kernel_task", "the macOS kernel"),
        (
            "WindowServer",
            "macOS: draws everything on screen (grows with displays and open windows)",
        ),
        ("mds_stores", "Spotlight: search index"),
        ("mds", "Spotlight: indexing server"),
        ("mdworker_shared", "Spotlight: file indexer"),
        ("photoanalysisd", "Photos: library analysis (faces, scenes)"),
        (
            "mediaanalysisd",
            "Photos: media analysis (Live Text, objects)",
        ),
        ("bird", "iCloud Drive sync"),
        ("cloudd", "iCloud sync"),
        ("Finder", "Finder"),
        ("Dock", "Dock and Mission Control"),
        ("com.docker.backend", "Docker Desktop backend"),
        (
            "com.docker.virtualization",
            "Docker Desktop virtual machine",
        ),
        ("qemu-system-aarch64", "a QEMU virtual machine"),
        ("qemu-system-x86_64", "a QEMU virtual machine"),
        ("java", "a Java program (IDE, Gradle, Minecraft…)"),
        ("node", "a Node.js program"),
        ("python3", "a Python program"),
        ("Python", "a Python program"),
        ("rust-analyzer", "Rust language server (editor)"),
        ("tsserver", "TypeScript language server (editor)"),
    ];
    if let Some((_, d)) = KNOWN.iter().find(|(n, _)| *n == name) {
        return d.to_string();
    }
    match app {
        Some(a) if a == name => format!("{a}: main process"),
        Some(a) => format!("part of {a}"),
        None if p.exe.starts_with("/System/") || p.exe.starts_with("/usr/") => {
            "macOS system process".into()
        }
        None => String::new(),
    }
}

/// Path shown to the user: ~ for the home folder.
pub fn pretty_exe(exe: &str) -> String {
    if exe.starts_with('/') {
        util::pretty(Path::new(exe))
    } else {
        exe.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pi(exe: &str, args: &str) -> ProcInfo {
        ProcInfo {
            exe: exe.into(),
            args: args.into(),
        }
    }

    #[test]
    fn finds_outer_app() {
        assert_eq!(
            app_of("/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)").as_deref(),
            Some("Google Chrome")
        );
        assert_eq!(app_of("/usr/sbin/cfprefsd"), None);
    }

    #[test]
    fn parses_ps() {
        let m = parse_pid_lines(
            "  812 /Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)\n    1 /sbin/launchd\n",
        );
        assert_eq!(m[&1], "/sbin/launchd");
        assert!(m[&812].ends_with("Claude Helper (Renderer)"));
    }

    #[test]
    fn notes() {
        let ff = pi(
            "/Applications/Firefox.app/Contents/MacOS/plugin-container.app/Contents/MacOS/plugin-container",
            "/Applications/Firefox.app/Contents/MacOS/plugin-container.app/Contents/MacOS/plugin-container -isForBrowser 12 tab",
        );
        assert!(note(&ff, &[]).starts_with("Firefox: web page content"));
        let r = pi(
            "/Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)",
            "/Applications/Claude.app/… --type=renderer --lang=fr",
        );
        assert!(note(&r, &[]).starts_with("Claude: page / window renderer"));
        let vm = pi(
            "/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine",
            "",
        );
        assert!(note(&vm, &["Claude (desktop workspace)"]).contains("VM apps running now: Claude"));
        assert_eq!(
            vm_apps_in(
                "/Applications/Docker.app/Contents/MacOS/com.docker.backend\n/sbin/launchd\n"
            ),
            vec!["Docker Desktop"]
        );
    }
}
