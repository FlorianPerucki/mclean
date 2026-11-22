use super::{App, MenuItem, Modal, Row, Screen};
use crate::actions::Mode;
use crate::model::{Category, Finding, Fix, Risk, Status};
use crate::util::{self, fmt_size};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Clear, Gauge, HighlightSpacing, List, ListItem, Paragraph, Wrap,
};

const ACCENT: Color = Color::Cyan;
const SAFE: Color = Color::Green;
const REVIEW: Color = Color::Yellow;
const BAD: Color = Color::Red;
const DIM: Color = Color::DarkGray;
const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn block(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM))
        .title(title.into())
}

fn hl() -> Style {
    Style::new()
        .bg(Color::Rgb(40, 60, 80))
        .add_modifier(Modifier::BOLD)
}

fn risk_color(r: Risk) -> Color {
    match r {
        Risk::Safe => SAFE,
        Risk::Review => REVIEW,
    }
}

fn status_color(s: Status) -> Color {
    match s {
        Status::Ok => SAFE,
        Status::Info => ACCENT,
        Status::Warn => REVIEW,
        Status::Fail => BAD,
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn pad(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len > n {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        format!("{s}{}", " ".repeat(n - len))
    }
}

/// Tick box: a solid coloured block when ticked so it stands out at a glance.
fn checkbox(state: u8) -> Vec<Span<'static>> {
    let b = match state {
        2 => Span::styled(" ✔ ", Style::new().bg(SAFE).fg(Color::Black).bold()),
        1 => Span::styled(" ~ ", Style::new().bg(REVIEW).fg(Color::Black).bold()),
        _ => Span::styled(" · ", Style::new().fg(DIM)),
    };
    vec![b, Span::raw(" ")]
}

/// Text style of a row: unticked rows are greyed out.
fn row_style(state: u8) -> Style {
    if state == 0 {
        Style::new().fg(DIM)
    } else {
        Style::new()
    }
}

const CURSOR_BG: Color = Color::Rgb(40, 60, 80);
const CURSOR_FG: Color = Color::Rgb(240, 244, 248);
const CURSOR_DIM: Color = Color::Rgb(165, 180, 195);

/// Paints the cursor bar on every span except the tick box (spans[box_at]),
/// so the box keeps its colour on the current line. Text colours that would
/// be hard to read on the bar (grey, blue) are brightened. Pass
/// `usize::MAX` as `box_at` for rows without a tick box.
fn with_cursor(mut spans: Vec<Span<'static>>, cursor: bool, box_at: usize) -> Line<'static> {
    if cursor {
        for (k, sp) in spans.iter_mut().enumerate() {
            if k != box_at {
                sp.style = sp.style.bg(CURSOR_BG).add_modifier(Modifier::BOLD);
                sp.style = match sp.style.fg {
                    // Default text is black on a light terminal theme: always
                    // force light text on the dark bar (RGB, so themes can't remap it).
                    None
                    | Some(
                        Color::Reset | Color::Black | Color::White | Color::Blue | Color::LightBlue,
                    ) => sp.style.fg(CURSOR_FG),
                    Some(DIM | Color::Gray) => sp.style.fg(CURSOR_DIM),
                    _ => sp.style,
                };
            }
        }
    }
    Line::from(spans)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [header, body, toast, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    draw_header(f, app, header);
    match app.screen {
        Screen::Scanning => draw_scanning(f, app, body),
        Screen::Menu => draw_menu(f, app, body),
        Screen::Category(c) => draw_category(f, app, body, Some(c)),
        Screen::Smart => draw_category(f, app, body, None),
        Screen::Health => draw_health(f, app, body),
        Screen::Apps => draw_apps(f, app, body),
        Screen::Uninstall => draw_uninstall(f, app, body),
        Screen::Space => draw_space(f, app, body),
    }
    if let Some(t) = &app.toast {
        f.render_widget(
            Paragraph::new(Line::from(vec![" ▸ ".fg(ACCENT), t.clone().into()])),
            toast,
        );
    }
    let hints = match app.screen {
        Screen::Scanning => "Enter: go to the menu now · q quit",
        Screen::Menu => {
            "↑↓ · Enter open · s smart · m select manually · d delete selected · e disk · u uninstall · ? help · q quit"
        }
        Screen::Category(_) | Screen::Smart => {
            "↑↓ · Space tick · Tab expand/fold · → ← · a all · Enter clean · o Finder · p mode · Esc back"
        }
        Screen::Health if app.health_pick.is_some() => {
            "↑↓ · Space tick · a all · Enter run on ticked · o Finder · ← back to the list"
        }
        Screen::Health => "↑↓ · Enter fix · → / Space pick one by one · r check again · Esc back",
        Screen::Apps => "↑↓ · Enter uninstall · / filter · s size · n name · o Finder · Esc back",
        Screen::Uninstall => "↑↓ · Space tick · Enter uninstall · o Finder · Esc back",
        Screen::Space => {
            "↑↓ · Enter/→ open · ← up · Space mark · d delete marked (or current) · u unmark all · o Finder · Esc menu"
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(format!(" {hints}")).fg(DIM)),
        footer,
    );
    if let Some(m) = &app.modal {
        draw_modal(f, app, m);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" mclean ", Style::new().bg(ACCENT).fg(Color::Black).bold()),
        " ".into(),
    ];
    if let Some((total, avail)) = app.disk {
        let pct = avail as f64 * 100.0 / total.max(1) as f64;
        let c = if pct < 10.0 {
            BAD
        } else if pct < 20.0 {
            REVIEW
        } else {
            SAFE
        };
        spans.push(Span::raw("Disk: "));
        spans.push(Span::styled(
            format!("{} free", fmt_size(avail)),
            Style::new().fg(c).bold(),
        ));
        spans.push(Span::styled(
            format!(" / {}", fmt_size(total)),
            Style::new().fg(DIM),
        ));
    }
    if app.cleaned_session > 0 {
        spans.push(Span::styled(
            format!("   ✔ {} cleaned", fmt_size(app.cleaned_session)),
            Style::new().fg(SAFE),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let mode = match app.mode {
        Mode::Trash => Span::styled(" trash ", Style::new().fg(DIM)),
        Mode::Permanent => Span::styled(
            " ⚠ PERMANENT DELETION ",
            Style::new().bg(BAD).fg(Color::White).bold(),
        ),
    };
    f.render_widget(
        Paragraph::new(Line::from(mode)).alignment(Alignment::Right),
        area,
    );
}

fn draw_scanning(f: &mut Frame, app: &App, area: Rect) {
    let r = centered(area, 86, 16);
    let mut lines = vec![
        Line::from(""),
        Line::from("  Scanning your Mac…".bold()),
        Line::from(""),
    ];
    for (i, s) in app.steps.iter().enumerate() {
        let icon = if s.done {
            Span::styled("  ✔ ", Style::new().fg(SAFE))
        } else {
            Span::styled(
                format!("  {} ", SPIN[(app.tick as usize + i) % SPIN.len()]),
                Style::new().fg(ACCENT),
            )
        };
        lines.push(Line::from(vec![
            icon,
            Span::styled(crate::scan::STEPS[i], Style::new().bold()),
        ]));
        lines.push(Line::from(Span::styled(
            format!("      {}", s.text),
            Style::new().fg(DIM),
        )));
    }
    let found: u64 = app.findings.iter().map(|f| f.size).sum();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        "  Found so far: ".into(),
        Span::styled(fmt_size(found), Style::new().fg(ACCENT).bold()),
    ]));
    f.render_widget(Paragraph::new(lines).block(block(" mclean ")), r);
}

fn menu_label(app: &App, item: MenuItem) -> (String, String, Style) {
    match item {
        MenuItem::Manual => ("✋ Select manually".into(), String::new(), Style::new()),
        MenuItem::DeleteSelected => {
            let items = app.selected_items();
            let s: u64 = items.iter().map(|f| f.size).sum();
            if items.is_empty() {
                (
                    "❌ Delete selected".into(),
                    "—".into(),
                    Style::new().fg(DIM),
                )
            } else {
                (
                    "❌ Delete selected".into(),
                    format!("{} · {}", items.len(), fmt_size(s)),
                    Style::new().fg(BAD).bold(),
                )
            }
        }
        MenuItem::Smart => {
            let s: u64 = app.smart_items().iter().map(|f| f.size).sum();
            (
                "✨ Smart clean".into(),
                fmt_size(s),
                Style::new().fg(SAFE).bold(),
            )
        }
        MenuItem::Cat(c) => {
            let (t, n) = app.cat_total(c);
            let st = if n == 0 {
                Style::new().fg(DIM)
            } else {
                Style::new()
            };
            (
                format!("{} {}", c.icon(), c.label()),
                if n == 0 { "—".into() } else { fmt_size(t) },
                st,
            )
        }
        MenuItem::Health => {
            let (fail, warn) = app.health_issues();
            let (txt, st) = if fail > 0 {
                (format!("{fail} problem(s)"), Style::new().fg(BAD))
            } else if warn > 0 {
                (format!("{warn} warning(s)"), Style::new().fg(REVIEW))
            } else {
                ("OK".into(), Style::new().fg(SAFE))
            };
            ("🩺 Health & configuration".into(), txt, st)
        }
        MenuItem::Apps => ("📤 Uninstall an app".into(), String::new(), Style::new()),
        MenuItem::Space => ("📊 Explore disk space".into(), String::new(), Style::new()),
        MenuItem::Rescan => ("🔄 Scan again".into(), String::new(), Style::new().fg(DIM)),
        MenuItem::Quit => ("🚪 Quit".into(), String::new(), Style::new().fg(DIM)),
    }
}

fn draw_menu(f: &mut Frame, app: &mut App, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Length(56), Constraint::Min(30)]).areas(area);
    let items = app.menu_items();
    let sel = app.menu_state.selected();
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let (label, size, st) = menu_label(app, *it);
            let num = if i < 9 {
                format!("{} ", i + 1)
            } else {
                "  ".into()
            };
            let mut lines = vec![with_cursor(
                vec![
                    Span::styled(num, Style::new().fg(DIM)),
                    Span::styled(pad(&label, 33), st),
                    Span::styled(format!("{size:>14}"), st),
                ],
                sel == Some(i),
                usize::MAX,
            )];
            // Spacer after the last entry of a group (after, so the cursor never sits on it).
            if matches!(
                it,
                MenuItem::DeleteSelected | MenuItem::Cat(Category::Leftovers) | MenuItem::Space
            ) {
                lines.push(Line::from(""));
            }
            ListItem::new(lines)
        })
        .collect();
    let title = if app.scanning_done() {
        " Menu ".to_string()
    } else {
        format!(" Menu {} scanning… ", SPIN[app.tick as usize % SPIN.len()])
    };
    let list = List::new(list_items)
        .block(block(title))
        .highlight_style(hl())
        .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold()));
    f.render_stateful_widget(list, left, &mut app.menu_state);

    let sel = app
        .menu_state
        .selected()
        .map(|i| items[i])
        .unwrap_or(MenuItem::Smart);
    let mut lines: Vec<Line> = vec![Line::from("")];
    match sel {
        MenuItem::Manual => {
            lines.push(Line::from("  Select manually".bold()));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Unticks everything in every category, so nothing is cleaned unless you pick it.",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from("  1. Enter here (everything becomes unticked)"));
            lines.push(Line::from(
                "  2. Open the categories below, tick what you want with Space",
            ));
            lines.push(Line::from("     (Tab expands a line to pick single files)"));
            lines.push(Line::from(
                "  3. Come back and choose \"Delete selected\" to clean it all at once",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from("  Shortcut from the menu: m".fg(DIM)));
        }
        MenuItem::DeleteSelected => {
            let items = app.selected_items();
            let total: u64 = items.iter().map(|f| f.size).sum();
            lines.push(Line::from(vec![
                "  Delete selected: ".bold(),
                Span::styled(fmt_size(total), Style::new().fg(BAD).bold()),
                format!("  ({} items)", items.len()).fg(DIM),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Everything currently ticked, in every category (safe AND review items).".fg(DIM),
            ));
            lines.push(Line::from(""));
            for c in Category::ALL {
                let v: Vec<&crate::model::Finding> =
                    items.iter().filter(|f| f.category == c).collect();
                if !v.is_empty() {
                    let s: u64 = v.iter().map(|f| f.size).sum();
                    lines.push(Line::from(format!(
                        "    {} {:<28} {:>10}  ({})",
                        c.icon(),
                        c.label(),
                        fmt_size(s),
                        v.len()
                    )));
                }
            }
            if items.is_empty() {
                lines.push(Line::from("  Nothing ticked yet."));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Enter to review and confirm. Shortcut from the menu: d".fg(ACCENT),
            ));
        }
        MenuItem::Smart => {
            let items = app.smart_items();
            let total: u64 = items.iter().map(|f| f.size).sum();
            lines.push(Line::from(vec![
                "  Smart clean: ".bold(),
                Span::styled(fmt_size(total), Style::new().fg(SAFE).bold()),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  One click for everything marked \"safe\" across all categories:".fg(DIM),
            ));
            lines.push(Line::from(
                "  caches, logs and installers that are recreated or re-downloadable.".fg(DIM),
            ));
            lines.push(Line::from(
                "  None of your documents, projects or \"review\" items are included.".fg(DIM),
            ));
            lines.push(Line::from(
                "  Untick anything in a category first and it is left out here too.".fg(DIM),
            ));
            lines.push(Line::from(""));
            for c in Category::ALL {
                let s: u64 = items
                    .iter()
                    .filter(|f| f.category == c)
                    .map(|f| f.size)
                    .sum();
                if s > 0 {
                    lines.push(Line::from(format!(
                        "    {} {:<28} {:>10}",
                        c.icon(),
                        c.label(),
                        fmt_size(s)
                    )));
                }
            }
            let (fail, warn) = app.health_issues();
            if fail + warn > 0 {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  🩺 {} health item(s) to look at (Health menu).",
                        fail + warn
                    ),
                    Style::new().fg(REVIEW),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Enter: review the list (→ expands items, Space ticks), then clean.".fg(ACCENT),
            ));
        }
        MenuItem::Cat(c) => {
            let (t, n) = app.cat_total(c);
            lines.push(Line::from(vec![
                format!("  {} {}: ", c.icon(), c.label()).bold(),
                Span::styled(fmt_size(t), Style::new().fg(ACCENT).bold()),
                format!("  ({n} items)").fg(DIM),
            ]));
            lines.push(Line::from(format!(
                "  Selected: {}",
                fmt_size(app.cat_selected(c))
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("  {}", c.description())));
            lines.push(Line::from(""));
            for &i in app.cat_indices(c).iter().take(12) {
                let fd = &app.findings[i];
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {:>9}  ", fmt_size(fd.size)),
                        Style::new().fg(risk_color(fd.risk)),
                    ),
                    Span::raw(pad(&fd.title, 60)),
                ]));
            }
        }
        MenuItem::Health => {
            lines.push(Line::from("  Health & configuration".bold()));
            lines.push(Line::from(""));
            for h in app.health.iter().take(16) {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {} ", h.status.icon()),
                        Style::new().fg(status_color(h.status)),
                    ),
                    Span::raw(pad(&h.name, 34)),
                    Span::styled(pad(&h.message, 60), Style::new().fg(DIM)),
                ]));
            }
        }
        MenuItem::Apps => {
            lines.push(Line::from("  Full uninstall".bold()));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Removes the app AND its hidden files: preferences, caches,",
            ));
            lines.push(Line::from(
                "  containers, Application Support, launch agents…",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Dragging an app to the Trash often leaves hundreds of MB behind.".fg(DIM),
            ));
        }
        MenuItem::Space => {
            lines.push(Line::from("  Explore disk space".bold()));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Browse your home folder sorted by size, folder by folder,",
            ));
            lines.push(Line::from("  to find what takes up room."));
            if let Some((total, avail)) = app.disk {
                lines.push(Line::from(""));
                let used = total.saturating_sub(avail);
                let w = 40usize;
                let filled = ((used as f64 / total.max(1) as f64) * w as f64) as usize;
                lines.push(Line::from(vec![
                    "  ".into(),
                    Span::styled("█".repeat(filled.min(w)), Style::new().fg(ACCENT)),
                    Span::styled("░".repeat(w - filled.min(w)), Style::new().fg(DIM)),
                    format!("  {} used", fmt_size(used)).into(),
                ]));
            }
        }
        MenuItem::Rescan => lines.push(Line::from("  Runs every scan again.")),
        MenuItem::Quit => lines.push(Line::from("  See you 👋")),
    }
    if !app.errors.is_empty() {
        lines.push(Line::from(""));
        for e in &app.errors {
            lines.push(Line::from(Span::styled(
                format!("  ✖ {e}"),
                Style::new().fg(BAD),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block(" Details ")),
        right,
    );
}

fn finding_row(fd: &Finding, w: usize, show_cat: bool, cursor: bool) -> ListItem<'static> {
    let arrow = if fd.expanded { "▾ " } else { "▸ " };
    let cat = if show_cat {
        format!("{} ", fd.category.icon())
    } else {
        String::new()
    };
    let st = fd.selection_state();
    let text = row_style(st);
    let risk = if st == 0 {
        text
    } else {
        Style::new().fg(risk_color(fd.risk))
    };
    let mut spans = vec![Span::styled(arrow, Style::new().fg(DIM)), Span::raw(cat)];
    spans.extend(checkbox(st));
    spans.extend([
        Span::styled(format!("{:>9}  ", fmt_size(fd.size)), text.bold()),
        Span::styled(pad(fd.risk.label(), 8), risk),
    ]);
    // Expanded: title alone on the first line, full description wrapped below.
    let wrapped = fd.expanded && fd.about.is_some();
    spans.extend(title_spans(
        &fd.title,
        if wrapped { None } else { fd.about.as_deref() },
        w,
        text,
    ));
    let mut lines = vec![with_cursor(spans, cursor, 2)];
    if wrapped && let Some(about) = &fd.about {
        // Indent under the title column: arrow + icon + box + size + risk.
        let indent = 2 + if show_cat { 3 } else { 0 } + 4 + 11 + 8;
        for l in wrap(about, w.max(20)) {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(l, Style::new().fg(Color::Gray).italic()),
            ]));
        }
    }
    ListItem::new(Text::from(lines))
}

/// Word-wraps `text` to `width` columns.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let need = line.chars().count() + word.chars().count() + usize::from(!line.is_empty());
        if need > width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// Title followed by a dim description, fitted into `w` columns.
fn title_spans(title: &str, about: Option<&str>, w: usize, style: Style) -> Vec<Span<'static>> {
    let tlen = title.chars().count();
    match about {
        Some(a) if tlen + 6 < w => {
            let rest = w - tlen - 3;
            vec![
                Span::styled(title.to_string(), style),
                Span::styled(format!(" — {}", pad(a, rest)), Style::new().fg(DIM)),
            ]
        }
        _ => vec![Span::styled(pad(title, w), style)],
    }
}

fn draw_category(f: &mut Frame, app: &mut App, area: Rect, scope: Option<Category>) {
    let [top, bottom] = Layout::vertical([Constraint::Min(5), Constraint::Length(10)]).areas(area);
    let rows = app.rows(scope);
    let w = top.width.saturating_sub(36) as usize;
    let cur = app.cat_state.selected();
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(k, r)| match *r {
            Row::Finding(i) => finding_row(&app.findings[i], w, scope.is_none(), cur == Some(k)),
            Row::Child(i, j) => {
                let fd = &app.findings[i];
                let ch = &fd.children.as_ref().unwrap()[j];
                let name = ch
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                // With several roots, show the parent folder for context.
                let label = if fd.paths.len() > 1 && !fd.contents_only {
                    util::pretty(&ch.path)
                } else {
                    name
                };
                let st = if ch.selected { 2 } else { 0 };
                let mut spans = vec![Span::raw("      ")];
                spans.extend(checkbox(st));
                spans.extend([Span::styled(
                    format!("{:>9}  ", fmt_size(ch.size)),
                    row_style(st),
                )]);
                let about = ch
                    .path
                    .file_name()
                    .and_then(|n| crate::describe::describe(&n.to_string_lossy()));
                // The sub-item under the cursor shows its whole description, wrapped.
                let here = cur == Some(k);
                let long = here
                    && about.as_ref().is_some_and(|a| {
                        label.chars().count() + a.chars().count() + 3 > w.saturating_sub(4)
                    });
                spans.extend(title_spans(
                    &label,
                    if long { None } else { about.as_deref() },
                    w.saturating_sub(4),
                    row_style(st),
                ));
                let mut lines = vec![with_cursor(spans, here, 1)];
                if long && let Some(a) = &about {
                    for l in wrap(a, w.saturating_sub(4).max(20)) {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(6 + 4 + 11)),
                            Span::styled(l, Style::new().fg(Color::Gray).italic()),
                        ]));
                    }
                }
                ListItem::new(Text::from(lines))
            }
        })
        .collect();
    let idx = app.scope_indices(scope);
    let ticked = idx
        .iter()
        .filter(|&&i| app.findings[i].selection_state() > 0)
        .count();
    let sel: u64 = idx.iter().map(|&i| app.findings[i].selected_size()).sum();
    let total: u64 = idx.iter().map(|&i| app.findings[i].size).sum();
    let name = match scope {
        Some(c) => format!("{} {}", c.icon(), c.label()),
        None => "✨ Smart clean — safe items from every category".into(),
    };
    let title = Line::from(vec![
        Span::raw(format!(" {name} — {} · ", fmt_size(total))),
        Span::styled(
            format!(" {ticked}/{} ticked · {} ", idx.len(), fmt_size(sel)),
            Style::new().bg(SAFE).fg(Color::Black).bold(),
        ),
        Span::raw(" "),
    ]);
    if items.is_empty() {
        f.render_widget(
            Paragraph::new("\n  Nothing to report here 🎉").block(block(title)),
            top,
        );
    } else {
        f.render_stateful_widget(
            List::new(items)
                .block(block(title))
                .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold())),
            top,
            &mut app.cat_state,
        );
    }
    let mut lines = vec![];
    match app.cat_state.selected().and_then(|i| rows.get(i).copied()) {
        Some(Row::Finding(i)) => {
            let fd = &app.findings[i];
            if let Some(a) = &fd.about {
                lines.push(Line::from(vec![
                    Span::styled("What is it: ", Style::new().fg(ACCENT).bold()),
                    Span::raw(a.clone()),
                ]));
            }
            lines.push(Line::from(fd.explanation.clone()));
            if let Some(lu) = fd.last_used {
                lines.push(Line::from(Span::styled(
                    format!("Modified / used: {}", util::fmt_age(lu)),
                    Style::new().fg(DIM),
                )));
            }
            if let Some(p) = &fd.process {
                lines.push(Line::from(Span::styled(
                    format!("\"{p}\" must be closed."),
                    Style::new().fg(REVIEW),
                )));
            }
            if fd.permanent {
                lines.push(Line::from(Span::styled(
                    "Permanent deletion (no Trash).",
                    Style::new().fg(BAD),
                )));
            }
            for p in fd.paths.iter().take(3) {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {}{}",
                        util::pretty(p),
                        if fd.contents_only { "/*" } else { "" }
                    ),
                    Style::new().fg(ACCENT),
                )));
            }
            if fd.paths.len() > 3 {
                lines.push(Line::from(Span::styled(
                    format!("  … and {} more paths", fd.paths.len() - 3),
                    Style::new().fg(DIM),
                )));
            }
            if !fd.expanded {
                lines.push(Line::from(Span::styled(
                    "Tab or → to pick individual files/folders inside.",
                    Style::new().fg(DIM),
                )));
            }
        }
        Some(Row::Child(i, j)) => {
            let ch = &app.findings[i].children.as_ref().unwrap()[j];
            if let Some(a) = ch
                .path
                .file_name()
                .and_then(|n| crate::describe::describe(&n.to_string_lossy()))
            {
                lines.push(Line::from(vec![
                    Span::styled("What is it: ", Style::new().fg(ACCENT).bold()),
                    Span::raw(a),
                ]));
            }
            lines.push(Line::from(Span::styled(
                util::pretty(&ch.path),
                Style::new().fg(ACCENT),
            )));
            lines.push(Line::from(format!("Size: {}", fmt_size(ch.size))));
            if let Some(m) = util::mtime(&ch.path) {
                lines.push(Line::from(Span::styled(
                    format!("Modified: {}", util::fmt_age(m)),
                    Style::new().fg(DIM),
                )));
            }
            lines.push(Line::from(Span::styled(
                "Tab or ← to fold.",
                Style::new().fg(DIM),
            )));
        }
        None => {}
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block(" Details ")),
        bottom,
    );
}

fn draw_health(f: &mut Frame, app: &mut App, area: Rect) {
    // Details grows to fit long advice and the file list, up to half the screen.
    let details_h = app
        .health_state
        .selected()
        .and_then(|i| app.health.get(i))
        .map_or(11, |h| {
            let files = h
                .fix
                .as_ref()
                .and_then(|x| x.pick_labels())
                .map_or(2, |p| p.len() + 3);
            ((h.advice.lines().count() + files + 5) as u16).clamp(11, (area.height / 2).max(11))
        });
    let [top, bottom] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(details_h)]).areas(area);
    let sel = app.health_state.selected();
    let items: Vec<ListItem> = app
        .health
        .iter()
        .enumerate()
        .map(|(k, h)| {
            ListItem::new(with_cursor(
                vec![
                    Span::styled(
                        format!(" {}  ", h.status.icon()),
                        Style::new().fg(status_color(h.status)).bold(),
                    ),
                    Span::styled(pad(&h.name, 46), Style::new().bold()),
                    Span::styled(h.message.clone(), Style::new().fg(DIM)),
                    if h.fix.is_some() {
                        Span::styled("  ⚙", Style::new().fg(ACCENT))
                    } else {
                        Span::raw("")
                    },
                ],
                sel == Some(k),
                usize::MAX,
            ))
        })
        .collect();
    f.render_stateful_widget(
        List::new(items)
            .block(block(" 🩺 Health & configuration "))
            .highlight_style(hl())
            .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold())),
        top,
        &mut app.health_state,
    );
    let Some(h) = app.health_state.selected().and_then(|i| app.health.get(i)) else {
        return;
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", h.status.icon()),
                Style::new().fg(status_color(h.status)),
            ),
            h.message.clone().bold(),
        ]),
        Line::from(""),
    ];
    for l in h.advice.lines() {
        lines.push(Line::from(l.to_string()));
    }
    let Some(files) = h.fix.as_ref().and_then(|x| x.pick_labels()) else {
        if let Some(fx) = &h.fix {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Enter: {}", fx.label()),
                Style::new().fg(ACCENT).bold(),
            )));
        }
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block(" Details ")),
            bottom,
        );
        return;
    };

    // Item with files to trash: they are listed and can be ticked one by one.
    let label = h.fix.as_ref().map(|x| x.label()).unwrap_or_default();
    let is_files = matches!(h.fix, Some(Fix::Trash { .. }));
    let picking = app.health_pick.is_some();
    let ticked: Vec<bool> = app
        .health_pick
        .as_ref()
        .map_or_else(|| vec![false; files.len()], |p| p.ticked.clone());
    let n_ticked = ticked.iter().filter(|t| **t).count();
    let hint = if picking {
        Line::from(vec![
            "Space tick · a all · ".fg(DIM),
            Span::styled(
                if is_files {
                    format!("Enter: move {n_ticked} ticked file(s) to the Trash")
                } else {
                    format!("Enter: {label} — {n_ticked} ticked")
                },
                Style::new().fg(ACCENT).bold(),
            ),
            if is_files {
                " · o Finder · ← back"
            } else {
                " · ← back"
            }
            .fg(DIM),
        ])
    } else {
        Line::from(vec![
            Span::styled("→ / Space: pick one by one", Style::new().fg(ACCENT).bold()),
            format!(" · Enter: {label} (all {})", files.len()).fg(DIM),
        ])
    };
    let text_h = lines.len() as u16;
    let rows: Vec<ListItem> = files
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let st = if ticked.get(k).copied().unwrap_or(false) {
                2
            } else {
                0
            };
            let mut spans = checkbox(st);
            spans.push(Span::raw(p.clone()));
            let cur =
                picking && app.health_pick.as_ref().and_then(|x| x.state.selected()) == Some(k);
            ListItem::new(with_cursor(spans, cur, 0))
        })
        .collect();
    let border = if picking {
        Style::new().fg(ACCENT)
    } else {
        Style::new().fg(DIM)
    };
    let blk = block(" Details ").border_style(border);
    let inner = blk.inner(bottom);
    f.render_widget(blk, bottom);
    let [t, l, hn] = Layout::vertical([
        Constraint::Length(text_h),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), t);
    let list = List::new(rows)
        .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold()))
        .highlight_spacing(HighlightSpacing::Always);
    match &mut app.health_pick {
        Some(p) => f.render_stateful_widget(list, l, &mut p.state),
        None => f.render_widget(list, l),
    }
    f.render_widget(Paragraph::new(hint), hn);
}

fn draw_apps(f: &mut Frame, app: &mut App, area: Rect) {
    let list = app.filtered_apps();
    let sel = app.apps_state.selected();
    let items: Vec<ListItem> = list
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let a = &app.apps[i];
            let size = if a.size == 0 {
                "…".to_string()
            } else {
                fmt_size(a.size)
            };
            ListItem::new(with_cursor(
                vec![
                    Span::styled(pad(&a.name, 36), Style::new().bold()),
                    Span::raw(format!("{size:>10}   ")),
                    Span::styled(a.bundle_id.clone(), Style::new().fg(DIM)),
                ],
                sel == Some(k),
                usize::MAX,
            ))
        })
        .collect();
    let title = if app.filtering || !app.apps_filter.is_empty() {
        format!(
            " 📤 Apps — filter: {}{} ",
            app.apps_filter,
            if app.filtering { "▏" } else { "" }
        )
    } else {
        format!(
            " 📤 Apps ({}) — Enter to see what will be removed ",
            list.len()
        )
    };
    f.render_stateful_widget(
        List::new(items)
            .block(block(title))
            .highlight_style(hl())
            .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold())),
        area,
        &mut app.apps_state,
    );
}

fn draw_uninstall(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(plan) = &mut app.uninstall else {
        return;
    };
    let total: u64 = plan.files.iter().filter(|x| x.2).map(|x| x.1).sum();
    let cur = plan.state.selected();
    let items: Vec<ListItem> = plan
        .files
        .iter()
        .enumerate()
        .map(|(k, (p, s, sel))| {
            let st = if *sel { 2 } else { 0 };
            let mut spans = checkbox(st);
            spans.push(Span::styled(
                format!("{:>9}  ", fmt_size(*s)),
                row_style(st),
            ));
            spans.push(Span::styled(util::pretty(p), row_style(st)));
            ListItem::new(with_cursor(spans, cur == Some(k), 0))
        })
        .collect();
    let title = format!(
        " Uninstall {} ({}) — {} in total ",
        plan.app.name,
        plan.app.bundle_id,
        fmt_size(total)
    );
    f.render_stateful_widget(
        List::new(items)
            .block(block(title))
            .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold())),
        area,
        &mut plan.state,
    );
}

fn draw_space(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(sp) = &mut app.space else { return };
    let total = sp.total().max(1);
    let max = sp.entries.first().and_then(|e| e.size).unwrap_or(1).max(1);
    let barw = 24usize;
    let sel = sp.state.selected();
    // Inside a marked folder, everything shown goes with it.
    let inherited = sp.path.ancestors().any(|a| sp.is_marked(a));
    // Room left for the name: borders (2), cursor symbol (2), box (4),
    // size (10), bar, percentage (8).
    let name_w = (area.width as usize).saturating_sub(2 + 2 + 4 + 10 + barw + 8);
    let items: Vec<ListItem> = sp
        .entries
        .iter()
        .enumerate()
        .map(|(k, e)| {
            let (size, bar, pct) = match e.size {
                Some(s) => {
                    let n = (((s as f64 / max as f64) * barw as f64).round() as usize).min(barw);
                    (
                        fmt_size(s),
                        format!("{}{}", "█".repeat(n), "░".repeat(barw - n)),
                        format!("{:>3.0} %", s as f64 * 100.0 / total as f64),
                    )
                }
                None => ("…".into(), "░".repeat(barw), "    ".into()),
            };
            let name = if e.is_dir {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            };
            let name = pad(&name, name_w);
            let state = if sp.is_marked(&e.path) {
                2
            } else if inherited {
                1
            } else {
                0
            };
            let mut spans = checkbox(state);
            spans.extend([
                Span::styled(format!("{size:>9} "), Style::new().bold()),
                Span::styled(bar, Style::new().fg(ACCENT)),
                Span::styled(format!(" {pct:>5}  "), Style::new().fg(DIM)),
                if e.is_dir {
                    Span::styled(name, Style::new().fg(Color::LightBlue).bold())
                } else {
                    Span::raw(name)
                },
            ]);
            ListItem::new(with_cursor(spans, sel == Some(k), 0))
        })
        .collect();
    let pending = sp.pending();
    let marked = if sp.marked.is_empty() {
        String::new()
    } else {
        format!(
            "  ✔ {} marked, {} — d to delete ",
            sp.marked.len(),
            fmt_size(sp.marked_total())
        )
    };
    let title = format!(
        " 📊 {} — {}{}{} ",
        util::pretty(&sp.path),
        fmt_size(sp.total()),
        marked,
        if pending > 0 {
            format!(
                "  {} measuring {} folder(s)…",
                SPIN[app.tick as usize % SPIN.len()],
                pending
            )
        } else {
            String::new()
        }
    );
    f.render_stateful_widget(
        List::new(items)
            .block(block(title))
            .highlight_symbol(Span::styled("▶ ", Style::new().fg(ACCENT).bold())),
        area,
        &mut sp.state,
    );
}

/// Clears the whole rows covered by a popup: a wide emoji half-covered by the
/// popup border would otherwise smear across it.
fn clear_band(f: &mut Frame, r: Rect) {
    let a = f.area();
    f.render_widget(
        Clear,
        Rect {
            x: a.x,
            y: r.y,
            width: a.width,
            height: r.height,
        },
    );
}

fn draw_modal(f: &mut Frame, app: &App, m: &Modal) {
    let area = f.area();
    match m {
        Modal::Confirm { title, items, .. } => {
            let total: u64 = items.iter().map(|f| f.size).sum();
            let any_perm = items.iter().any(|f| f.permanent);
            let r = centered(area, 84, (items.len().min(10) + 11) as u16);
            clear_band(f, r);
            let mut lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::raw(format!("  {} item(s) · ", items.len())),
                    Span::styled(fmt_size(total), Style::new().fg(ACCENT).bold()),
                ]),
                Line::from(""),
            ];
            for it in items.iter().take(10) {
                let extra = if it.paths.len() > 1 {
                    format!(" ({} entries)", it.paths.len())
                } else {
                    String::new()
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("   {:>9}  ", fmt_size(it.size)),
                        Style::new().fg(DIM),
                    ),
                    Span::raw(pad(&format!("{}{}", it.title, extra), 64)),
                ]));
            }
            if items.len() > 10 {
                lines.push(Line::from(Span::styled(
                    format!("   … and {} more", items.len() - 10),
                    Style::new().fg(DIM),
                )));
            }
            lines.push(Line::from(""));
            lines.push(match app.mode {
                Mode::Trash => Line::from(vec![
                    "  Mode: ".into(),
                    Span::styled("move to Trash (recoverable)", Style::new().fg(SAFE).bold()),
                ]),
                Mode::Permanent => Line::from(vec![
                    "  Mode: ".into(),
                    Span::styled("PERMANENT DELETION", Style::new().fg(BAD).bold()),
                ]),
            });
            if any_perm {
                lines.push(Line::from(Span::styled(
                    "  The Trash itself will be emptied permanently.",
                    Style::new().fg(REVIEW),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  y / Enter", Style::new().fg(ACCENT).bold()),
                " confirm   ".into(),
                Span::styled("p", Style::new().fg(ACCENT).bold()),
                " switch mode   ".into(),
                Span::styled("Esc", Style::new().fg(ACCENT).bold()),
                " cancel".into(),
            ]));
            f.render_widget(
                Paragraph::new(lines)
                    .block(block(format!(" {title} ")).border_style(Style::new().fg(ACCENT))),
                r,
            );
        }
        Modal::ConfirmShell { label, cmd } => {
            let r = centered(area, 84, 11);
            clear_band(f, r);
            let lines = vec![
                Line::from(""),
                Line::from(format!("  {label}").bold()),
                Line::from(""),
                Line::from(vec![
                    "  Command: ".into(),
                    Span::styled(cmd.clone(), Style::new().fg(ACCENT)),
                ]),
                Line::from(""),
                Line::from("  The UI pauses while the command runs (sudo may ask for your".fg(DIM)),
                Line::from(
                    "  password; you type it in the terminal, mclean never sees it).".fg(DIM),
                ),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  y / Enter", Style::new().fg(ACCENT).bold()),
                    " run   ".into(),
                    Span::styled("Esc", Style::new().fg(ACCENT).bold()),
                    " cancel".into(),
                ]),
            ];
            f.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .block(block(" Run a command ").border_style(Style::new().fg(ACCENT))),
                r,
            );
        }
        Modal::Running {
            title,
            current,
            done,
            total,
        } => {
            let r = centered(area, 72, 7);
            clear_band(f, r);
            let b = block(format!(" {title}… ")).border_style(Style::new().fg(ACCENT));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let [l1, l2] =
                Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(inner);
            f.render_widget(Paragraph::new(format!("\n {}", pad(current, 66))), l1);
            let ratio = if *total == 0 {
                1.0
            } else {
                (*done as f64 / *total as f64).min(1.0)
            };
            f.render_widget(
                Gauge::default()
                    .gauge_style(Style::new().fg(ACCENT))
                    .ratio(ratio)
                    .label(format!("{done}/{total}")),
                l2,
            );
        }
        Modal::Message { title, lines } => {
            let r = centered(area, 90, (lines.len() + 5) as u16);
            clear_band(f, r);
            let mut t: Vec<Line> = vec![Line::from("")];
            t.extend(lines.iter().map(|l| Line::from(format!("  {l}"))));
            t.push(Line::from(""));
            t.push(Line::from("  Press any key to continue".fg(DIM)));
            f.render_widget(
                Paragraph::new(Text::from(t))
                    .wrap(Wrap { trim: false })
                    .block(block(format!(" {title} ")).border_style(Style::new().fg(SAFE))),
                r,
            );
        }
        Modal::Help => {
            let r = centered(area, 82, 28);
            clear_band(f, r);
            let rows = [
                ("↑ ↓ / j k", "move"),
                ("Enter", "open, confirm, clean"),
                ("Tab", "expand / fold an item (pick its files one by one)"),
                ("→ / l", "expand an item"),
                ("← / h", "collapse, or go back"),
                ("Esc / q", "go back one step (q quits from the menu)"),
                ("Space", "tick / untick"),
                ("a", "tick / untick everything"),
                ("o", "show in Finder"),
                ("p", "switch Trash ⇄ permanent deletion"),
                ("s", "smart clean (menu)"),
                ("m", "select manually: untick everything (menu)"),
                ("d", "delete everything ticked, all categories (menu)"),
                ("e", "explore disk space (menu)"),
                ("u", "uninstall an app (menu)"),
                ("r", "scan again"),
                ("1-9", "jump to a menu entry"),
                ("q / Ctrl-C", "quit"),
            ];
            let mut lines = vec![Line::from("")];
            for (k, d) in rows {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {k:<14}"), Style::new().fg(ACCENT).bold()),
                    Span::raw(d),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "  Everything goes to the Trash by default. Deletion journal:".fg(DIM),
            ));
            lines.push(Line::from(
                format!("  {}", util::pretty(&crate::actions::journal_path())).fg(DIM),
            ));
            lines.push(Line::from(
                format!(
                    "  Your own rules: {}",
                    util::pretty(&crate::rules::user_rules_path())
                )
                .fg(DIM),
            ));
            f.render_widget(
                Paragraph::new(lines).block(block(" Help ").border_style(Style::new().fg(ACCENT))),
                r,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wraps_words() {
        let l = super::wrap("one two three four five", 9);
        assert_eq!(l, vec!["one two", "three", "four five"]);
        assert!(l.iter().all(|x| x.chars().count() <= 9));
    }
}
