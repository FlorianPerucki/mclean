//! Mac health and configuration checks.

use crate::model::{Fix, HealthItem, Status};
use crate::util::{self, run, run_full};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

type Check = fn() -> Vec<HealthItem>;

pub fn checks() -> Vec<(&'static str, Check)> {
    let mut v: Vec<(&'static str, Check)> = vec![
        ("Disk space", disk_space),
        ("Launch agents", launch_items),
        ("Corrupted preferences", broken_prefs),
        ("Broken symlinks", broken_symlinks),
        ("Uptime", uptime),
        ("Homebrew", homebrew),
        ("Docker", docker),
    ];
    if util::is_macos() {
        v.extend([
            ("Full Disk Access", full_disk_access as Check),
            ("FileVault", filevault),
            ("Firewall", firewall),
            ("Gatekeeper", gatekeeper),
            ("System Integrity Protection", sip),
            ("macOS updates", software_update),
            ("Time Machine", time_machine),
            ("Memory and swap", memory),
            ("Spotlight", spotlight),
            ("Battery", battery),
            ("Xcode simulators", simulators),
        ]);
    }
    v
}

pub fn scan(progress: &(dyn Fn(&str) + Sync)) -> Vec<HealthItem> {
    let list = checks();
    let mut out: Vec<HealthItem> = list
        .par_iter()
        .flat_map(|(name, f)| {
            let r = f();
            progress(&format!("{name} ✓"));
            r
        })
        .collect();
    out.sort_by(|a, b| a.status.cmp(&b.status).then(a.name.cmp(&b.name)));
    out
}

fn settings(pane: &str) -> String {
    format!("open 'x-apple.systempreferences:{pane}'")
}

// ───────────────────────────── Disk ─────────────────────────────

pub fn disk_free(path: &str) -> Option<(u64, u64)> {
    let out = run("df", &["-k", path])?;
    let line = out.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    let total: u64 = cols.get(1)?.parse().ok()?;
    let avail: u64 = cols.get(3)?.parse().ok()?;
    Some((total * 1024, avail * 1024))
}

fn disk_space() -> Vec<HealthItem> {
    let target = if util::is_macos() && Path::new("/System/Volumes/Data").exists() {
        "/System/Volumes/Data"
    } else {
        "/"
    };
    let Some((total, avail)) = disk_free(target) else {
        return vec![];
    };
    let pct = avail as f64 * 100.0 / total.max(1) as f64;
    let msg = format!(
        "{} free of {} ({:.0} %)",
        util::fmt_size(avail),
        util::fmt_size(total),
        pct
    );
    let st = if pct < 5.0 || avail < 10_000_000_000 {
        Status::Fail
    } else if pct < 15.0 || avail < 30_000_000_000 {
        Status::Warn
    } else {
        Status::Ok
    };
    vec![HealthItem::new("disk", "Disk space", st, msg).advice(
        "macOS needs at least 10–15 % free space for swap, updates and snapshots. Run the smart clean or explore disk space (key e in the menu).",
    )]
}

// ───────────────────────────── Security ─────────────────────────────

fn full_disk_access() -> Vec<HealthItem> {
    let probe = util::home().join("Library/Safari");
    let denied = match std::fs::read_dir(&probe) {
        Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
        Ok(_) => false,
    };
    if denied {
        vec![HealthItem::new("fda", "Full Disk Access", Status::Warn, "Your terminal lacks Full Disk Access: some caches and containers are invisible.")
            .advice("System Settings › Privacy & Security › Full Disk Access › enable your terminal app (Terminal, iTerm, Ghostty…), then restart mclean.")
            .fix(Fix::Shell { label: "Open the settings".into(), cmd: settings("com.apple.preference.security?Privacy_AllFiles") })]
    } else {
        vec![HealthItem::new(
            "fda",
            "Full Disk Access",
            Status::Ok,
            "Granted",
        )]
    }
}

fn filevault() -> Vec<HealthItem> {
    let Some(out) = run("fdesetup", &["status"]) else {
        return vec![];
    };
    if out.contains("On") {
        vec![HealthItem::new(
            "filevault",
            "FileVault",
            Status::Ok,
            "Disk is encrypted",
        )]
    } else {
        vec![HealthItem::new("filevault", "FileVault", Status::Warn, "Disk is not encrypted")
            .advice("If your Mac is stolen, all your data is readable. Turn on FileVault (no noticeable performance cost on recent Macs).")
            .fix(Fix::Shell { label: "Open FileVault settings".into(), cmd: settings("com.apple.settings.PrivacySecurity.extension?FileVault") })]
    }
}

fn firewall() -> Vec<HealthItem> {
    let Some(out) = run(
        "/usr/libexec/ApplicationFirewall/socketfilterfw",
        &["--getglobalstate"],
    ) else {
        return vec![];
    };
    if out.to_lowercase().contains("enabled") {
        vec![HealthItem::new("firewall", "Firewall", Status::Ok, "On")]
    } else {
        vec![HealthItem::new("firewall", "Firewall", Status::Warn, "Off")
            .advice("The application firewall blocks unsolicited incoming connections. Recommended on public networks.")
            .fix(Fix::Shell { label: "Turn on the firewall (sudo)".into(), cmd: "sudo /usr/libexec/ApplicationFirewall/socketfilterfw --setglobalstate on".into() })]
    }
}

fn gatekeeper() -> Vec<HealthItem> {
    let Some(out) = run_full("spctl", &["--status"], Duration::from_secs(5), true) else {
        return vec![];
    };
    if out.contains("enabled") {
        vec![HealthItem::new(
            "gatekeeper",
            "Gatekeeper",
            Status::Ok,
            "On",
        )]
    } else {
        vec![
            HealthItem::new(
                "gatekeeper",
                "Gatekeeper",
                Status::Fail,
                "Off: unsigned apps open without any check",
            )
            .advice("Turn it back on unless you have a specific reason.")
            .fix(Fix::Shell {
                label: "Turn Gatekeeper back on (sudo)".into(),
                cmd: "sudo spctl --global-enable".into(),
            }),
        ]
    }
}

fn sip() -> Vec<HealthItem> {
    let Some(out) = run("csrutil", &["status"]) else {
        return vec![];
    };
    if out.contains("enabled") {
        vec![HealthItem::new(
            "sip",
            "System Integrity Protection (SIP)",
            Status::Ok,
            "On",
        )]
    } else {
        vec![HealthItem::new("sip", "System Integrity Protection (SIP)", Status::Fail, "Off")
            .advice("SIP protects system files from malware. To turn it back on: boot into Recovery and run `csrutil enable`.")]
    }
}

fn software_update() -> Vec<HealthItem> {
    let Some(out) = run_full("softwareupdate", &["-l"], Duration::from_secs(40), true) else {
        return vec![HealthItem::new(
            "swupdate",
            "macOS updates",
            Status::Info,
            "Could not check (timed out or offline)",
        )];
    };
    let labels: Vec<String> = out
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("* Label:")
                .map(|s| s.trim().to_string())
        })
        .collect();
    if labels.is_empty() {
        vec![HealthItem::new(
            "swupdate",
            "macOS updates",
            Status::Ok,
            "System is up to date",
        )]
    } else {
        vec![
            HealthItem::new(
                "swupdate",
                "macOS updates",
                Status::Warn,
                format!(
                    "{} update(s) available: {}",
                    labels.len(),
                    labels.join(", ")
                ),
            )
            .advice("Updates fix security holes that are actively exploited.")
            .fix(Fix::Shell {
                label: "Open Software Update".into(),
                cmd: settings("com.apple.Software-Update-Settings.extension"),
            }),
        ]
    }
}

// ───────────────────────────── Backup ─────────────────────────────

fn time_machine() -> Vec<HealthItem> {
    let mut v = Vec::new();
    let dest = run_full(
        "tmutil",
        &["destinationinfo"],
        Duration::from_secs(10),
        true,
    )
    .unwrap_or_default();
    if dest.contains("No destinations configured") || dest.trim().is_empty() {
        v.push(
            HealthItem::new("tm", "Time Machine", Status::Warn, "No backup configured")
                .advice("An external drive is enough. Without a backup, a disk failure or an accidental deletion is final.")
                .fix(Fix::Shell { label: "Open Time Machine".into(), cmd: settings("com.apple.Time-Machine-Settings.extension") }),
        );
    } else if let Some(out) = run_full("tmutil", &["latestbackup"], Duration::from_secs(10), true) {
        // …/2024-05-03-101500.backup
        let date = out
            .split('/')
            .next_back()
            .and_then(|s| s.trim().get(..17))
            .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d-%H%M%S").ok());
        match date {
            Some(d) => {
                let ts = d.and_utc().timestamp();
                let st = if util::age_days(ts) > 14 {
                    Status::Warn
                } else {
                    Status::Ok
                };
                v.push(
                    HealthItem::new(
                        "tm",
                        "Time Machine",
                        st,
                        format!("Last backup {}", util::fmt_age(ts)),
                    )
                    .advice("Plug in your backup drive regularly."),
                );
            }
            None => v.push(HealthItem::new(
                "tm",
                "Time Machine",
                Status::Info,
                "Destination configured, last backup date unknown (drive unplugged?)",
            )),
        }
    }
    if let Some(out) = run("tmutil", &["listlocalsnapshots", "/"]) {
        let n = out.lines().filter(|l| l.contains("com.apple")).count();
        if n > 0 {
            v.push(
                HealthItem::new("snapshots", "Local APFS snapshots", Status::Info, format!("{n} local snapshot(s)"))
                    .advice("macOS removes them automatically when space runs low, but they can hide several GB. Deleting them frees the space right away.")
                    .fix(Fix::Shell { label: "Delete local snapshots".into(), cmd: "tmutil thinlocalsnapshots / 999999999999 4".into() }),
            );
        }
    }
    v
}

// ───────────────────────────── Performance ─────────────────────────────

fn uptime() -> Vec<HealthItem> {
    let secs = if util::is_macos() {
        run("sysctl", &["-n", "kern.boottime"]).and_then(|s| {
            let sec = s
                .split("sec =")
                .nth(1)?
                .split(',')
                .next()?
                .trim()
                .parse::<i64>()
                .ok()?;
            Some(util::now() - sec)
        })
    } else {
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .map(|f| f as i64)
    };
    let Some(secs) = secs else { return vec![] };
    let days = secs / 86400;
    let st = if days >= 21 { Status::Warn } else { Status::Ok };
    vec![
        HealthItem::new("uptime", "Uptime", st, format!("Up for {days} day(s)")).advice(
            "An occasional restart clears compressed memory and swap, and applies pending updates.",
        ),
    ]
}

fn memory() -> Vec<HealthItem> {
    let mut v = Vec::new();
    if let Some(out) = run("sysctl", &["-n", "vm.swapusage"]) {
        // total = 2048.00M  used = 1093.50M  free = 954.50M
        let used = out
            .split("used =")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("0M");
        let mb: f64 = used
            .trim_end_matches(['M', 'G'])
            .replace(',', ".")
            .parse()
            .unwrap_or(0.0)
            * if used.ends_with('G') { 1024.0 } else { 1.0 };
        let st = if mb > 4096.0 {
            Status::Warn
        } else {
            Status::Ok
        };
        v.push(HealthItem::new("swap", "Swap", st, format!("{:.1} GB of swap in use", mb / 1024.0)).advice(
            "High swap means RAM is short: quit memory-hungry apps (browser with many tabs, Docker, VMs) or restart.",
        ));
    }
    if let Some(out) = run("memory_pressure", &["-Q"])
        && let Some(pct) = out
            .split(':')
            .nth(1)
            .and_then(|s| s.trim().trim_end_matches('%').parse::<u32>().ok())
    {
        let st = if pct < 10 { Status::Warn } else { Status::Ok };
        v.push(HealthItem::new(
            "mem",
            "Free memory",
            st,
            format!("{pct} % of memory available"),
        ));
    }
    v
}

fn spotlight() -> Vec<HealthItem> {
    let Some(out) = run_full("mdutil", &["-s", "/"], Duration::from_secs(5), true) else {
        return vec![];
    };
    if out.contains("disabled") {
        vec![
            HealthItem::new("spotlight", "Spotlight", Status::Warn, "Indexing is off")
                .advice("Without Spotlight, search and file \"last opened\" dates don't work.")
                .fix(Fix::Shell {
                    label: "Turn indexing back on (sudo)".into(),
                    cmd: "sudo mdutil -i on /".into(),
                }),
        ]
    } else {
        vec![HealthItem::new(
            "spotlight",
            "Spotlight",
            Status::Ok,
            "Indexing is on",
        )]
    }
}

fn battery() -> Vec<HealthItem> {
    let Some(out) = run_full(
        "system_profiler",
        &["SPPowerDataType"],
        Duration::from_secs(15),
        false,
    ) else {
        return vec![];
    };
    let get = |k: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(k).map(|s| s.trim().to_string()))
    };
    let Some(cycles) = get("Cycle Count:") else {
        return vec![];
    }; // no battery
    let condition = get("Condition:").unwrap_or_else(|| "?".into());
    let capacity = get("Maximum Capacity:").unwrap_or_else(|| "?".into());
    let cap_n: u32 = capacity.trim_end_matches('%').parse().unwrap_or(100);
    let st = if condition != "Normal" || cap_n < 80 {
        Status::Warn
    } else {
        Status::Ok
    };
    vec![HealthItem::new("battery", "Battery", st, format!("Condition: {condition} · max capacity: {capacity} · {cycles} cycles"))
        .advice("Below 80 % capacity or with a \"Service Recommended\" condition, a replacement is advised.")]
}

// ───────────────────────────── Configuration ─────────────────────────────

fn program_of(plist: &Path) -> Option<String> {
    let v = plist::Value::from_file(plist).ok()?;
    let d = v.as_dictionary()?;
    if let Some(p) = d.get("Program").and_then(|v| v.as_string()) {
        return Some(p.to_string());
    }
    d.get("ProgramArguments")?
        .as_array()?
        .first()?
        .as_string()
        .map(|s| s.to_string())
}

fn launch_items() -> Vec<HealthItem> {
    let home = util::home();
    let dirs: Vec<(PathBuf, bool)> = vec![
        (home.join("Library/LaunchAgents"), true),
        (PathBuf::from("/Library/LaunchAgents"), false),
        (PathBuf::from("/Library/LaunchDaemons"), false),
    ];
    let mut out = Vec::new();
    let mut total = 0;
    let mut unsigned = Vec::new();
    for (dir, user) in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "plist") {
                continue;
            }
            total += 1;
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            match program_of(&p) {
                None => {
                    out.push(
                        HealthItem::new(
                            &format!("launch:{name}"),
                            &format!("Unreadable agent: {name}"),
                            Status::Warn,
                            format!("{} is not a valid plist", util::pretty(&p)),
                        )
                        .advice("launchd ignores this file. It can be safely deleted."),
                    );
                }
                Some(prog) if prog.starts_with('/') && !Path::new(&prog).exists() => {
                    let item = HealthItem::new(&format!("launch:{name}"), &format!("Broken agent: {name}"), Status::Fail, format!("Launches {prog}, which no longer exists"))
                        .advice("Leftover of an uninstalled app. launchd keeps trying to start it (errors in the logs, slower startup).");
                    out.push(if user {
                        item.fix(Fix::Trash { label: "Move the agent to the Trash".into(), paths: vec![p.clone()] })
                    } else {
                        item.fix(Fix::Shell {
                            label: "Unload and remove (sudo)".into(),
                            cmd: format!("sudo launchctl bootout system '{0}' 2>/dev/null; sudo mv '{0}' ~/.Trash/", p.display()),
                        })
                    });
                }
                Some(prog) if util::is_macos() && prog.starts_with('/') => {
                    let is_script = std::fs::read(&prog)
                        .map(|b| b.starts_with(b"#!"))
                        .unwrap_or(true);
                    if !is_script && util::run("codesign", &["-v", &prog]).is_none() {
                        unsigned.push(format!("{name} → {prog}"));
                    }
                }
                _ => {}
            }
        }
    }
    if !unsigned.is_empty() {
        out.push(
            HealthItem::new("launch:unsigned", "Unsigned agents", Status::Warn, format!("{} auto-started program(s) without a valid signature", unsigned.len()))
                .advice(format!("Not necessarily malicious (open-source tools built locally), but worth checking:\n{}", unsigned.join("\n"))),
        );
    }
    out.push(
        HealthItem::new("launch:count", "Launch agents", Status::Info, format!("{total} third-party agent(s)/daemon(s) installed"))
            .advice("Programs started automatically in the background. System Settings › General › Login Items lets you disable them."),
    );
    out
}

fn broken_prefs() -> Vec<HealthItem> {
    let dir = util::home().join("Library/Preferences");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let bad: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "plist"))
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .filter(|p| plist::Value::from_file(p).is_err())
        .collect();
    if bad.is_empty() {
        return vec![HealthItem::new(
            "prefs",
            "Preference files",
            Status::Ok,
            "No corrupted file",
        )];
    }
    let names: Vec<String> = bad
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    vec![HealthItem::new("prefs", "Corrupted preferences", Status::Warn, format!("{} unreadable file(s)", bad.len()))
        .advice(format!("A corrupted preference file can crash its app. Once removed, the app starts over with default settings.\n{}", names.join("\n")))
        .fix(Fix::Trash { label: "Move to the Trash".into(), paths: bad })]
}

fn broken_symlinks() -> Vec<HealthItem> {
    let mut bad = Vec::new();
    for dir in ["/usr/local/bin", "/opt/homebrew/bin"] {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if std::fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
                && std::fs::metadata(&p).is_err()
            {
                bad.push(p);
            }
        }
    }
    if bad.is_empty() {
        return vec![];
    }
    let list: Vec<String> = bad
        .iter()
        .take(20)
        .map(|p| p.display().to_string())
        .collect();
    let fix = if util::has_cmd("brew") {
        Fix::Shell {
            label: "brew cleanup --prune-prefix".into(),
            cmd: "brew cleanup --prune-prefix".into(),
        }
    } else {
        Fix::Shell {
            label: "Show the list".into(),
            cmd: format!(
                "ls -l {}",
                bad.iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        }
    };
    vec![
        HealthItem::new(
            "symlinks",
            "Broken links in PATH",
            Status::Warn,
            format!("{} command(s) point to missing files", bad.len()),
        )
        .advice(format!(
            "Leftovers of uninstalled tools:\n{}",
            list.join("\n")
        ))
        .fix(fix),
    ]
}

fn homebrew() -> Vec<HealthItem> {
    if !util::has_cmd("brew") {
        return vec![];
    }
    let mut v = Vec::new();
    if let Some(out) = run_full(
        "brew",
        &["outdated", "--quiet"],
        Duration::from_secs(30),
        false,
    ) {
        let n = out.lines().filter(|l| !l.trim().is_empty()).count();
        if n > 0 {
            v.push(
                HealthItem::new(
                    "brew",
                    "Homebrew",
                    Status::Info,
                    format!("{n} outdated package(s)"),
                )
                .advice(out.lines().take(30).collect::<Vec<_>>().join(", "))
                .fix(Fix::Shell {
                    label: "brew upgrade".into(),
                    cmd: "brew upgrade".into(),
                }),
            );
        } else {
            v.push(HealthItem::new(
                "brew",
                "Homebrew",
                Status::Ok,
                "Packages up to date",
            ));
        }
    }
    if let Some(out) = run_full(
        "brew",
        &["autoremove", "--dry-run"],
        Duration::from_secs(30),
        false,
    ) {
        let n = out
            .lines()
            .filter(|l| !l.starts_with("==>") && !l.trim().is_empty())
            .count();
        if n > 0 {
            v.push(
                HealthItem::new(
                    "brew-orphans",
                    "Homebrew – orphaned dependencies",
                    Status::Info,
                    format!("{n} dependency(ies) no longer needed"),
                )
                .fix(Fix::Shell {
                    label: "brew autoremove".into(),
                    cmd: "brew autoremove".into(),
                }),
            );
        }
    }
    v
}

fn docker() -> Vec<HealthItem> {
    if !util::has_cmd("docker") {
        return vec![];
    }
    let Some(out) = run_full(
        "docker",
        &["system", "df", "--format", "{{.Type}}\t{{.Reclaimable}}"],
        Duration::from_secs(8),
        false,
    ) else {
        return vec![];
    };
    let lines: Vec<String> = out
        .lines()
        .map(|l| l.replace('\t', ": reclaimable "))
        .collect();
    vec![HealthItem::new("docker", "Docker", Status::Info, lines.join(" · "))
        .advice("Unused images, stopped containers, volumes and build cache. `docker system prune` leaves running containers alone; add `-a --volumes` to go further.")
        .fix(Fix::Shell { label: "docker system prune".into(), cmd: "docker system prune".into() })]
}

fn simulators() -> Vec<HealthItem> {
    if !Path::new("/Applications/Xcode.app").exists() {
        return vec![];
    }
    let Some(out) = run_full(
        "xcrun",
        &["simctl", "list", "devices", "unavailable"],
        Duration::from_secs(15),
        false,
    ) else {
        return vec![];
    };
    let n = out.lines().filter(|l| l.contains("(unavailable")).count();
    if n == 0 {
        return vec![];
    }
    vec![
        HealthItem::new(
            "simctl",
            "Obsolete Xcode simulators",
            Status::Info,
            format!("{n} simulator(s) for removed runtimes"),
        )
        .advice("Each simulator can take several GB in ~/Library/Developer/CoreSimulator/Devices.")
        .fix(Fix::Shell {
            label: "xcrun simctl delete unavailable".into(),
            cmd: "xcrun simctl delete unavailable".into(),
        }),
    ]
}
