//! Three-pass duplicate detection: size → partial hash → full hash.

use crate::model::{Category, Finding, Risk};
use crate::scan::home::FileInfo;
use crate::util;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use xxhash_rust::xxh3::{Xxh3, xxh3_64};

const PARTIAL: usize = 64 * 1024;

fn partial_hash(p: &Path, size: u64) -> Option<u64> {
    let mut f = File::open(p).ok()?;
    let mut buf = vec![0u8; PARTIAL];
    let n = f.read(&mut buf).ok()?;
    let mut h = xxh3_64(&buf[..n]);
    if size > (PARTIAL * 2) as u64 {
        f.seek(SeekFrom::End(-(PARTIAL as i64))).ok()?;
        let n = f.read(&mut buf).ok()?;
        h ^= xxh3_64(&buf[..n]).rotate_left(1);
    }
    Some(h)
}

fn full_hash(p: &Path) -> Option<u128> {
    let mut f = File::open(p).ok()?;
    let mut h = Xxh3::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.digest128())
}

fn group_by<K: std::hash::Hash + Eq + Send, F>(files: Vec<FileInfo>, key: F) -> Vec<Vec<FileInfo>>
where
    F: Fn(&FileInfo) -> Option<K> + Sync,
{
    let keyed: Vec<(K, FileInfo)> = files
        .into_par_iter()
        .filter_map(|f| key(&f).map(|k| (k, f)))
        .collect();
    let mut map: HashMap<K, Vec<FileInfo>> = HashMap::new();
    for (k, f) in keyed {
        map.entry(k).or_default().push(f);
    }
    map.into_values().filter(|g| g.len() > 1).collect()
}

/// Picks the copy to keep: not in Downloads, shortest path, then oldest.
fn keeper_index(group: &[FileInfo], home: &Path) -> usize {
    let dl = home.join("Downloads");
    let mut idx: Vec<usize> = (0..group.len()).collect();
    idx.sort_by_key(|&i| {
        let f = &group[i];
        (f.path.starts_with(&dl), f.path.as_os_str().len(), f.mtime)
    });
    idx[0]
}

pub fn find(files: Vec<FileInfo>, home: &Path, progress: &(dyn Fn(&str) + Sync)) -> Vec<Finding> {
    // 1. By size, ignoring hard links (same inode = same file).
    let mut seen = HashSet::new();
    let files: Vec<FileInfo> = files
        .into_iter()
        .filter(|f| seen.insert((f.dev, f.ino)))
        .collect();
    let mut by_size: HashMap<u64, Vec<FileInfo>> = HashMap::new();
    for f in files {
        by_size.entry(f.size).or_default().push(f);
    }
    let same_size: Vec<FileInfo> = by_size
        .into_values()
        .filter(|g| g.len() > 1)
        .flatten()
        .collect();
    progress(&format!(
        "duplicates: {} files share a size",
        same_size.len()
    ));

    // 2. Partial hash (start + end of file).
    let partial_groups = group_by(same_size, |f| {
        partial_hash(&f.path, f.size).map(|h| (f.size, h))
    });
    let to_full: Vec<FileInfo> = partial_groups.into_iter().flatten().collect();
    progress(&format!(
        "duplicates: fully hashing {} files",
        to_full.len()
    ));

    // 3. Full hash.
    let groups = group_by(to_full, |f| full_hash(&f.path).map(|h| (f.size, h)));

    let mut out: Vec<Finding> = groups
        .into_iter()
        .map(|g| {
            let k = keeper_index(&g, home);
            let keep = &g[k];
            let name = keep.path.file_name().unwrap().to_string_lossy().to_string();
            let others: Vec<_> = g
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != k)
                .map(|(_, f)| f)
                .collect();
            let mut fi = Finding::new(
                Category::Duplicates,
                format!("dup:{}", keep.path.display()),
                format!("{} (×{})", name, g.len()),
            );
            fi.explanation = format!(
                "Kept copy: {}. The other {} copies are proposed for deletion. \
                 If they are APFS clones, the space actually freed may be smaller.",
                util::pretty(&keep.path),
                others.len()
            );
            fi.paths = others.iter().map(|f| f.path.clone()).collect();
            fi.size = others.iter().map(|f| f.blocks).sum();
            fi.risk = Risk::Review;
            fi.last_used = Some(keep.mtime);
            fi
        })
        .collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.size));
    out
}
