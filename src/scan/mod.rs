pub mod dupes;
pub mod health;
pub mod home;
pub mod procinfo;

use crate::model::{Finding, HealthItem};
use crate::{apps, rules, util};
use std::sync::mpsc::Sender;

pub const STEPS: [&str; 4] = [
    "Caches, logs and developer junk",
    "Home folder: projects, large files, duplicates",
    "Leftovers of uninstalled apps",
    "Health and configuration",
];

pub enum Payload {
    Findings(Vec<Finding>),
    Health(Vec<HealthItem>),
    Error(String),
}

pub enum ScanMsg {
    Progress(usize, String),
    Done(usize, Payload),
}

/// Starts the 4 scans in parallel; each one sends its progress then its result.
pub fn spawn_all(tx: Sender<ScanMsg>) {
    let t = tx.clone();
    std::thread::spawn(move || {
        let p = |s: &str| {
            let _ = t.send(ScanMsg::Progress(0, s.to_string()));
        };
        let payload = match rules::load() {
            Ok(r) => {
                p(&format!("{} rules loaded", r.len()));
                Payload::Findings(rules::scan(&r, &p))
            }
            Err(e) => Payload::Error(format!("{e:#}")),
        };
        let _ = t.send(ScanMsg::Done(0, payload));
    });

    let t = tx.clone();
    std::thread::spawn(move || {
        let tc = std::sync::Mutex::new(t.clone());
        let p = |s: &str| {
            let _ = tc.lock().unwrap().send(ScanMsg::Progress(1, s.to_string()));
        };
        let r = home::scan(&util::home(), &p);
        let mut all = r.projects;
        all.extend(r.large);
        all.extend(r.duplicates);
        all.extend(r.downloads);
        p(&format!("{} files scanned", r.files_seen));
        let _ = t.send(ScanMsg::Done(1, Payload::Findings(all)));
    });

    let t = tx.clone();
    std::thread::spawn(move || {
        let tc = std::sync::Mutex::new(t.clone());
        let p = |s: &str| {
            let _ = tc.lock().unwrap().send(ScanMsg::Progress(2, s.to_string()));
        };
        let r = if util::is_macos() || util::home().join("Library").exists() {
            apps::leftovers(&p)
        } else {
            vec![]
        };
        let _ = t.send(ScanMsg::Done(2, Payload::Findings(r)));
    });

    std::thread::spawn(move || {
        let tc = std::sync::Mutex::new(tx.clone());
        let p = |s: &str| {
            let _ = tc.lock().unwrap().send(ScanMsg::Progress(3, s.to_string()));
        };
        let r = health::scan(&p);
        let _ = tx.send(ScanMsg::Done(3, Payload::Health(r)));
    });
}
