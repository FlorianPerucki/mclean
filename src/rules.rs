use crate::model::{Category, Finding, Risk};
use crate::util;
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT_RULES: &str = include_str!("../rules/default.toml");

#[derive(Debug, Deserialize, Clone)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(default = "default_risk")]
    pub risk: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub split: bool,
    #[serde(default)]
    pub contents: bool,
    #[serde(default)]
    pub min_age_days: i64,
    #[serde(default)]
    pub min_size_mb: u64,
    pub process: Option<String>,
    #[serde(default)]
    pub permanent: bool,
    #[serde(default)]
    pub explanation: String,
}

fn default_risk() -> String {
    "safe".into()
}

#[derive(Debug, Deserialize)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<Rule>,
}

pub fn user_rules_path() -> PathBuf {
    util::home().join(".config/mclean/rules.toml")
}

pub fn parse(text: &str) -> Result<Vec<Rule>> {
    let f: RuleFile = toml::from_str(text)?;
    for r in &f.rule {
        if Category::from_key(&r.category).is_none() {
            anyhow::bail!("rule {}: unknown category \"{}\"", r.id, r.category);
        }
    }
    Ok(f.rule)
}

/// User rules (take precedence) followed by the default rules.
pub fn load() -> Result<Vec<Rule>> {
    let mut rules = Vec::new();
    let up = user_rules_path();
    if up.exists() {
        let text = std::fs::read_to_string(&up)?;
        rules.extend(parse(&text).with_context(|| format!("reading {}", up.display()))?);
    }
    let mut defaults = parse(DEFAULT_RULES).context("default rules")?;
    defaults.retain(|d| !rules.iter().any(|u| u.id == d.id));
    rules.extend(defaults);
    Ok(rules)
}

fn excluded(rule: &Rule, p: &Path) -> bool {
    p.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        rule.exclude.iter().any(|pat| util::wildcard(pat, &name))
    })
}

/// Replaces a folder that contains already-claimed paths by its unclaimed
/// children, so nothing is ever counted or deleted twice.
fn carve(p: PathBuf, claimed: &[PathBuf], out: &mut Vec<PathBuf>) {
    if claimed.iter().any(|c| p.starts_with(c)) {
        return; // already covered by an earlier rule
    }
    if claimed.iter().any(|c| c.starts_with(&p)) {
        if let Ok(rd) = std::fs::read_dir(&p) {
            for e in rd.flatten() {
                carve(e.path(), claimed, out);
            }
        }
        return;
    }
    out.push(p);
}

/// Fixed part of a pattern, before its first wildcard (`~/Library/Caches/*` →
/// `~/Library/Caches`). Used to give split entries a readable, unique name.
fn fixed_prefix(pattern: &str) -> PathBuf {
    let full = util::expand(pattern);
    let mut out = PathBuf::new();
    for c in full.components() {
        let s = c.as_os_str().to_string_lossy();
        if s.contains('*') || s.contains('?') {
            break;
        }
        out.push(c);
    }
    out
}

/// Display name of a split entry: its path below the pattern's fixed prefix,
/// e.g. `Firefox/Profiles/abc.default/safebrowsing` instead of `safebrowsing`.
fn split_title(p: &Path, prefix: &Path) -> String {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // ~/Library/Containers/<id>/Data/Library/Caches → <id>
    if name == "Caches"
        && let Some(id) = p.ancestors().nth(3).and_then(|a| a.file_name())
        && p.ancestors()
            .nth(4)
            .is_some_and(|a| a.ends_with("Containers"))
    {
        return id.to_string_lossy().to_string();
    }
    match p.strip_prefix(prefix) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
        _ => name,
    }
}

struct Pending {
    rule_idx: usize,
    title: String,
    paths: Vec<PathBuf>,
}

pub fn scan(rules: &[Rule], progress: &dyn Fn(&str)) -> Vec<Finding> {
    let mut claimed: Vec<PathBuf> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();

    for (idx, rule) in rules.iter().enumerate() {
        let mut matched = Vec::new();
        let mut prefixes: Vec<(PathBuf, PathBuf)> = Vec::new();
        for pat in &rule.paths {
            let prefix = fixed_prefix(pat);
            for p in util::glob(pat) {
                if excluded(rule, &p) {
                    continue;
                }
                let Ok(meta) = std::fs::symlink_metadata(&p) else {
                    continue;
                };
                if meta.file_type().is_symlink() {
                    continue;
                }
                if rule.min_age_days > 0
                    && let Some(m) = util::mtime(&p)
                    && util::age_days(m) < rule.min_age_days
                {
                    continue;
                }
                let before = matched.len();
                carve(p, &claimed, &mut matched);
                for m in &matched[before..] {
                    prefixes.push((m.clone(), prefix.clone()));
                }
            }
        }
        matched.sort();
        matched.dedup();
        if matched.is_empty() {
            continue;
        }
        claimed.extend(matched.iter().cloned());
        if rule.split {
            for p in matched {
                let prefix = prefixes
                    .iter()
                    .find(|(m, _)| *m == p)
                    .map(|(_, pre)| pre.clone())
                    .unwrap_or_default();
                let title = split_title(&p, &prefix);
                pending.push(Pending {
                    rule_idx: idx,
                    title: format!("{} · {}", rule.name, title),
                    paths: vec![p],
                });
            }
        } else {
            pending.push(Pending {
                rule_idx: idx,
                title: rule.name.clone(),
                paths: matched,
            });
        }
    }

    progress(&format!("measuring {} items", pending.len()));

    let mut findings: Vec<Finding> = pending
        .into_par_iter()
        .filter_map(|p| {
            let rule = &rules[p.rule_idx];
            let mut size: u64 = 0;
            for path in &p.paths {
                let mut s = util::disk_usage(path);
                if rule.contents
                    && let Ok(m) = std::fs::symlink_metadata(path)
                {
                    use std::os::unix::fs::MetadataExt;
                    s = s.saturating_sub(m.blocks() * 512);
                }
                size += s;
            }
            let min = if rule.min_size_mb > 0 {
                rule.min_size_mb * 1_000_000
            } else {
                100_000
            };
            if size < min {
                return None;
            }
            let cat = Category::from_key(&rule.category)?;
            let mut f = Finding::new(cat, format!("rule:{}:{}", rule.id, p.title), p.title);
            f.explanation = rule.explanation.clone();
            f.paths = p.paths;
            f.size = size;
            f.risk = Risk::from_key(&rule.risk);
            f.selected = f.risk == Risk::Safe;
            f.process = rule.process.clone();
            f.contents_only = rule.contents;
            f.permanent = rule.permanent;
            f.last_used = f.paths.iter().filter_map(|p| util::mtime(p)).max();
            if rule.split {
                // Describe the entry by its own name (the app id for container caches).
                let key = f.title.rsplit(" · ").next().unwrap_or_default();
                let key = key.rsplit('/').next().unwrap_or(key);
                f.about = crate::describe::describe(key);
            }
            Some(f)
        })
        .collect();
    findings.sort_by_key(|x| std::cmp::Reverse(x.size));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules_parse() {
        let r = parse(DEFAULT_RULES).unwrap();
        assert!(r.len() > 30);
        let mut ids: Vec<_> = r.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, ids.len(), "duplicate rule ids");
    }

    #[test]
    fn split_titles_are_unique() {
        let pre = PathBuf::from("/h/Library/Caches");
        assert_eq!(
            split_title(Path::new("/h/Library/Caches/com.foo"), &pre),
            "com.foo"
        );
        assert_eq!(
            split_title(
                Path::new("/h/Library/Caches/Firefox/Profiles/a.default/safebrowsing"),
                &pre
            ),
            "Firefox/Profiles/a.default/safebrowsing"
        );
        let pre2 = PathBuf::from("/h/Library/Containers");
        assert_eq!(
            split_title(
                Path::new("/h/Library/Containers/com.x.y/Data/Library/Caches"),
                &pre2
            ),
            "com.x.y"
        );
        assert_eq!(fixed_prefix("/a/b/*/c"), PathBuf::from("/a/b"));
    }

    #[test]
    fn carve_splits_ancestors() {
        let d = std::env::temp_dir().join(format!("mclean-carve-{}", std::process::id()));
        std::fs::create_dir_all(d.join("Google/Chrome")).unwrap();
        std::fs::create_dir_all(d.join("Google/AndroidStudio")).unwrap();
        let claimed = vec![d.join("Google/Chrome")];
        let mut out = vec![];
        carve(d.join("Google"), &claimed, &mut out);
        assert_eq!(out, vec![d.join("Google/AndroidStudio")]);
        let mut out2 = vec![];
        carve(d.join("Google/Chrome"), &claimed, &mut out2);
        assert!(out2.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
