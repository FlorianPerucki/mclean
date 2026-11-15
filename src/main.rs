mod actions;
mod apps;
mod describe;
mod model;
mod rules;
mod scan;
mod tui;
mod util;

use clap::{Parser, Subcommand};
use model::{Category, Finding, HealthItem, Risk};
use scan::{Payload, ScanMsg};
use std::io::{IsTerminal, Write};

#[derive(Parser)]
#[command(
    name = "mclean",
    version,
    about = "Open-source macOS cleaner — run without arguments for the interactive UI"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan and print a report (nothing is deleted)
    Scan {
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    /// Clean the "safe" items without the UI (e.g. scheduled task)
    Clean {
        /// Show what would be cleaned without touching anything
        #[arg(long)]
        dry_run: bool,
        /// Don't ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
    /// List the active cleaning rules
    Rules,
}

pub struct ScanResult {
    pub findings: Vec<Finding>,
    pub health: Vec<HealthItem>,
    pub errors: Vec<String>,
}

fn scan_blocking(verbose: bool) -> ScanResult {
    let (tx, rx) = std::sync::mpsc::channel();
    scan::spawn_all(tx);
    let mut res = ScanResult {
        findings: vec![],
        health: vec![],
        errors: vec![],
    };
    let mut done = 0;
    while done < scan::STEPS.len() {
        match rx.recv() {
            Ok(ScanMsg::Progress(i, s)) => {
                if verbose {
                    eprint!("\r\x1b[2K  {} — {}", scan::STEPS[i], s);
                }
            }
            Ok(ScanMsg::Done(_, p)) => {
                done += 1;
                match p {
                    Payload::Findings(f) => res.findings.extend(f),
                    Payload::Health(h) => res.health.extend(h),
                    Payload::Error(e) => res.errors.push(e),
                }
            }
            Err(_) => break,
        }
    }
    if verbose {
        eprint!("\r\x1b[2K");
    }
    res
}

fn print_report(r: &ScanResult) {
    let mut grand = 0;
    for cat in Category::ALL {
        let items: Vec<&Finding> = r.findings.iter().filter(|f| f.category == cat).collect();
        if items.is_empty() {
            continue;
        }
        let total: u64 = items.iter().map(|f| f.size).sum();
        grand += total;
        println!(
            "\n{} {}  —  {}",
            cat.icon(),
            cat.label(),
            util::fmt_size(total)
        );
        for f in items.iter().take(8) {
            println!(
                "   {:>9}  [{}] {}",
                util::fmt_size(f.size),
                f.risk.label(),
                f.title
            );
        }
        if items.len() > 8 {
            println!("   … and {} more", items.len() - 8);
        }
    }
    let safe: u64 = r
        .findings
        .iter()
        .filter(|f| f.risk == Risk::Safe)
        .map(|f| f.size)
        .sum();
    println!(
        "\nTotal found: {}  ({} of it safe to clean)",
        util::fmt_size(grand),
        util::fmt_size(safe)
    );
    println!("\n🩺 Health and configuration");
    for h in &r.health {
        println!("   {} {} — {}", h.status.icon(), h.name, h.message);
    }
    for e in &r.errors {
        eprintln!("error: {e}");
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        None => {
            if !std::io::stdout().is_terminal() {
                anyhow::bail!("mclean must run in a terminal (or use `mclean scan`)");
            }
            tui::run()
        }
        Some(Cmd::Scan { json }) => {
            let r = scan_blocking(!json && std::io::stderr().is_terminal());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "findings": r.findings, "health": r.health, "errors": r.errors })
                    )?
                );
            } else {
                print_report(&r);
            }
            Ok(())
        }
        Some(Cmd::Clean { dry_run, yes }) => {
            let r = scan_blocking(std::io::stderr().is_terminal());
            let sel: Vec<Finding> = r
                .findings
                .iter()
                .filter(|f| f.risk == Risk::Safe)
                .filter_map(|f| f.to_job())
                .collect();
            let total: u64 = sel.iter().map(|f| f.size).sum();
            for f in &sel {
                println!("{:>9}  {}", util::fmt_size(f.size), f.title);
            }
            println!("\n{} items, {} in total.", sel.len(), util::fmt_size(total));
            if dry_run {
                println!("(dry run: nothing was deleted)");
                return Ok(());
            }
            if !yes {
                print!("Move these items to the Trash? [y/N] ");
                std::io::stdout().flush()?;
                let mut s = String::new();
                std::io::stdin().read_line(&mut s)?;
                if !matches!(s.trim().to_lowercase().as_str(), "y" | "yes") {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
            let rep = actions::clean(&sel, actions::Mode::Trash, false, &mut |_, t| {
                eprint!("\r\x1b[2K  {t}")
            });
            eprintln!();
            println!(
                "{} moved to the Trash, {} items processed.",
                util::fmt_size(rep.freed),
                rep.removed
            );
            for (p, e) in &rep.errors {
                println!("  ✖ {}: {}", util::pretty(p), e);
            }
            Ok(())
        }
        Some(Cmd::Rules) => {
            let rules = rules::load()?;
            for r in &rules {
                println!("{:<24} {:<9} {:<7} {}", r.id, r.category, r.risk, r.name);
            }
            println!(
                "\n{} rules. Your own rules: {}",
                rules.len(),
                rules::user_rules_path().display()
            );
            Ok(())
        }
    }
}
