//! Drawing. Colours are the terminal's own 16-colour palette, so light and dark themes
//! both work; every state also has a glyph or a word, so `NO_COLOR` loses nothing.

use lull_protocol::{OriginKind, RestartStatus, Server, ServerState, StoppedServer};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, HighlightSpacing, Padding, Paragraph, Row, Table, Wrap,
};

use super::app::{App, FIELDS, Prompt, View, config_value, tilde};
use crate::cli::{enum_str, origin, parse_rfc3339};

const MIN: (u16, u16) = (40, 8);

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if area.width < MIN.0 || area.height < MIN.1 {
        let msg = format!("lull needs {}×{}", MIN.0, MIN.1);
        f.render_widget(Paragraph::new(msg).centered().dim(), area);
        return;
    }
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    header(f, head, app);
    let (main, side) = split(body);
    match app.view {
        View::Servers => servers(f, main, side, app),
        View::History => history(f, main, side, app),
        View::Config => config(f, main, side, app),
    }
    footer(f, foot, app);
    if let Some(Prompt::Confirm { text, .. }) = &app.prompt {
        confirm(f, area, text);
    }
}

/// Detail pane beside the list when there is room, below it on tall narrow terminals,
/// and not at all on small ones.
fn split(area: Rect) -> (Rect, Option<Rect>) {
    if area.width >= 100 {
        let [a, b] = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(area);
        (a, Some(b))
    } else if area.height >= 24 {
        let [a, b] = Layout::vertical([Constraint::Fill(1), Constraint::Length(14)]).areas(area);
        (a, Some(b))
    } else {
        (area, None)
    }
}

fn panel(title: Line<'_>) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dim())
        .padding(Padding::horizontal(1))
        .title(title)
}

fn header(f: &mut Frame, area: Rect, app: &App) {
    let mut left = vec![" lull ".bold().cyan(), " ".into()];
    let counts = [Some(app.servers.len()), Some(app.history.len()), None];
    for (i, name) in ["Servers", "History", "Config"].into_iter().enumerate() {
        let tab = format!(" {name} ");
        left.push(if app.view as usize == i {
            tab.bold().underlined()
        } else {
            tab.dim()
        });
        if let Some(n) = counts[i].filter(|n| *n > 0) {
            left.push(format!("{n} ").dim());
        }
    }
    let status = match &app.hello {
        Some(h) => Line::from(vec![
            "● ".green(),
            format!(
                "daemon {} · {} ",
                h.daemon_version,
                enum_str(&h.activity_backend)
            )
            .dim(),
        ]),
        None => Line::from(vec!["○ ".red(), "daemon unreachable · retrying ".red()]),
    };
    let left = Line::from(left);
    let status = if left.width() + status.width() < area.width as usize {
        status
    } else {
        Line::from(status.spans[0].clone())
    };
    let [l, r] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(status.width() as u16),
    ])
    .areas(area);
    f.render_widget(left, l);
    f.render_widget(status, r);
}

fn table<'a>(rows: Vec<Row<'a>>, widths: Vec<Constraint>, block: Block<'a>) -> Table<'a> {
    Table::new(rows, widths)
        .block(block)
        .column_spacing(2)
        .highlight_symbol(Line::from("▌ ").cyan())
        .highlight_spacing(HighlightSpacing::Always)
        .row_highlight_style(Style::new().bold())
}

/// Width of the widest cell, for `Constraint::Length`.
fn widest<'a>(cells: impl Iterator<Item = &'a str>, cap: usize) -> Constraint {
    let w = cells.map(|c| Span::raw(c).width()).max().unwrap_or(0);
    Constraint::Length(w.min(cap) as u16)
}

fn empty(f: &mut Frame, area: Rect, block: Block, title: &str, hint: String) {
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [mid] = Layout::vertical([Constraint::Length(2)])
        .flex(ratatui::layout::Flex::Center)
        .areas(inner);
    f.render_widget(
        Paragraph::new(vec![Line::from(title.bold()), Line::from(hint.dim())]).centered(),
        mid,
    );
}

// ───────────── servers ─────────────

fn servers(f: &mut Frame, main: Rect, side: Option<Rect>, app: &mut App) {
    let soon = app
        .servers
        .iter()
        .filter(|s| s.state == ServerState::StoppingSoon)
        .count();
    let mut title = vec![format!(" {} running ", app.servers.len()).into()];
    if soon > 0 {
        title.push(format!("· {soon} stopping soon ").yellow());
    }
    let block = panel(Line::from(title));
    if app.servers.is_empty() {
        let (title, hint) = match (&app.hello, &app.config) {
            (None, _) => (
                "Waiting for the lull daemon",
                "brew services start lull".into(),
            ),
            (_, Some(c)) => (
                "No dev servers running",
                format!("Idle agent-started servers stop after {}m", c.idle_minutes),
            ),
            _ => ("No dev servers running", String::new()),
        };
        return empty(f, main, block, title, hint);
    }
    let now = app.now;
    let rows: Vec<Row> = app
        .servers
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let first = i == 0 || app.servers[i - 1].project != s.project;
            Row::new([
                Cell::from(if first {
                    s.project.clone().bold()
                } else {
                    "".into()
                }),
                Cell::from(glyph(s)),
                Cell::from(enum_str(&s.framework)),
                Cell::from(s.ports.first().map_or("—".into(), |p| format!(":{p}"))),
                Cell::from(stops(s, now)),
                Cell::from(origin_span(&s.origin)),
            ])
        })
        .collect();
    let widths = vec![
        widest(app.servers.iter().map(|s| s.project.as_str()), 24),
        Constraint::Length(1),
        widest(
            app.servers
                .iter()
                .map(|s| enum_str(&s.framework))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str),
            12,
        ),
        Constraint::Length(6),
        Constraint::Length(11),
        Constraint::Fill(1),
    ];
    f.render_stateful_widget(table(rows, widths, block), main, &mut app.lists[0]);
    if let (Some(side), Some(s)) = (side, app.selected_server()) {
        let block = panel(Line::from(format!(" {} ", s.project).bold()));
        let inner = block.inner(side);
        f.render_widget(block, side);
        let text = server_detail(s, now, inner.width as usize);
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
    }
}

fn glyph(s: &Server) -> Span<'static> {
    match s.state {
        ServerState::Running => "●".green(),
        ServerState::StoppingSoon => "◐".yellow(),
        ServerState::Stopping => "○".red(),
        ServerState::NoListener => "◌".dim(),
    }
}

fn stops(s: &Server, now: u64) -> Span<'static> {
    if s.pinned {
        return "pinned".cyan();
    }
    match (s.state, s.stops_at.as_deref().and_then(parse_rfc3339)) {
        (ServerState::Stopping, _) => "stopping".red(),
        (ServerState::NoListener, _) => "no listener".dim(),
        (state, Some(at)) => {
            let text = if at > now {
                format!("in {}", dur(at - now))
            } else {
                "now".into()
            };
            if state == ServerState::StoppingSoon {
                text.yellow().bold()
            } else {
                text.into()
            }
        }
        (_, None) => "never".dim(),
    }
}

fn origin_span(o: &lull_protocol::Origin) -> Span<'static> {
    match o.kind {
        OriginKind::Agent => origin(o).magenta(),
        OriginKind::User => origin(o).into(),
    }
}

fn server_detail(s: &Server, now: u64, width: usize) -> Text<'static> {
    let at = |t: &str| parse_rfc3339(t).unwrap_or(now);
    let last = at(&s.last_activity_at);
    let mut lines = vec![
        Line::from(match &s.primary_url {
            Some(url) => url.clone().cyan().underlined(),
            None => "no listener".dim(),
        }),
        Line::from(tilde(&s.cwd).dim()),
        Line::default(),
    ];
    match s.stops_at.as_deref().map(at) {
        Some(stop) if !s.pinned => {
            let total = stop.saturating_sub(last).max(1);
            let idle = now.saturating_sub(last).min(total);
            let style = if s.state == ServerState::StoppingSoon {
                Style::new().yellow()
            } else {
                Style::new().cyan()
            };
            lines.push(bar(idle as f64 / total as f64, width, style));
            let left = if stop > now {
                format!("stops in {}", dur(stop - now))
            } else {
                "stopping".into()
            };
            lines.push(Line::from(
                format!("idle {} of {} · {left}", dur(idle), dur(total)).dim(),
            ));
        }
        _ if s.pinned => lines.push(Line::from("pinned: lull never stops it".cyan())),
        _ => lines.push(Line::from("auto-stop off: lull never stops it".dim())),
    }
    lines.push(Line::default());
    let session = s
        .origin
        .session_id
        .as_deref()
        .map(|id| format!(" · {}", id.chars().take(8).collect::<String>()));
    let tabs = match s.tabs.len() {
        0 => "none".to_owned(),
        n => format!(
            "{n} in {}",
            s.tabs
                .iter()
                .map(|t| t.browser.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let join = |v: &[_]| {
        v.iter()
            .map(|x: &u32| x.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    for (k, v) in [
        (
            "origin",
            format!("{}{}", origin(&s.origin), session.unwrap_or_default()),
        ),
        ("state", enum_str(&s.state)),
        (
            "ports",
            s.ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        ("pids", join(&s.pids)),
        (
            "load",
            format!("{:.1}% cpu · {}", s.cpu_percent, bytes(s.rss_bytes)),
        ),
        (
            "started",
            format!("{} ago", dur(now.saturating_sub(at(&s.started_at)))),
        ),
        (
            "activity",
            format!(
                "{} · {} ago",
                enum_str(&s.last_activity_source),
                dur(now.saturating_sub(last))
            ),
        ),
        ("tabs", tabs),
        (
            "policy",
            format!(
                "idle {}m · auto-stop {}",
                s.idle_minutes,
                if s.auto_stop { "on" } else { "off" }
            ),
        ),
    ] {
        lines.push(kv(k, v));
    }
    // Last: it can be long (absolute paths to node and friends) and wraps.
    lines.extend([Line::default(), Line::from(tilde(&s.command).bold())]);
    Text::from(lines)
}

/// A thin progress bar the width of the pane.
fn bar(ratio: f64, width: usize, style: Style) -> Line<'static> {
    let filled = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;
    Line::from(vec![
        Span::styled("━".repeat(filled), style),
        "─".repeat(width - filled).dim(),
    ])
}

fn kv(k: &'static str, v: String) -> Line<'static> {
    Line::from(vec![format!("{k:<10}").dim(), v.into()])
}

// ───────────── history ─────────────

fn history(f: &mut Frame, main: Rect, side: Option<Rect>, app: &mut App) {
    let block = panel(Line::from(" recently stopped "));
    if app.history.is_empty() {
        return empty(f, main, block, "Nothing stopped recently", String::new());
    }
    let now = app.now;
    let rows: Vec<Row> = app
        .history
        .iter()
        .map(|h| {
            Row::new([
                Cell::from(h.project.clone().bold()),
                Cell::from(enum_str(&h.framework)),
                Cell::from(ago(&h.stopped_at, now)),
                Cell::from(enum_str(&h.reason)),
                Cell::from(status(h)),
            ])
        })
        .collect();
    let widths = vec![
        widest(app.history.iter().map(|h| h.project.as_str()), 24),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Fill(1),
    ];
    f.render_stateful_widget(table(rows, widths, block), main, &mut app.lists[1]);
    let Some(h) = app.selected().and_then(|i| app.history.get(i)) else {
        return;
    };
    if let Some(side) = side {
        let block = panel(Line::from(format!(" {} ", h.project).bold()));
        let mut lines = vec![
            Line::from(tilde(&h.command).bold()),
            Line::from(tilde(&h.cwd).dim()),
            Line::default(),
            kv(
                "stopped",
                format!("{} · {}", ago(&h.stopped_at, now), enum_str(&h.reason)),
            ),
            kv("origin", origin(&h.origin)),
            kv(
                "ports",
                h.ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            kv(
                "env",
                h.restart_env
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            kv("tabs", h.tabs.len().to_string()),
            Line::default(),
        ];
        lines.push(if h.argv.is_empty() {
            Line::from("no launch command: cannot restart".dim())
        } else {
            Line::from(vec![
                "r".bold(),
                " restart  ".dim(),
                "f".bold(),
                " forget".dim(),
            ])
        });
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block),
            side,
        );
    }
}

fn status(h: &StoppedServer) -> Span<'static> {
    match h.status {
        RestartStatus::Stopped => "stopped".dim(),
        RestartStatus::Restarting => "restarting".yellow(),
        RestartStatus::RestartFailed => "restart failed".red(),
    }
}

fn ago(t: &str, now: u64) -> String {
    parse_rfc3339(t).map_or(t.to_owned(), |t| {
        format!("{} ago", dur(now.saturating_sub(t)))
    })
}

// ───────────── config ─────────────

fn config(f: &mut Frame, main: Rect, side: Option<Rect>, app: &mut App) {
    let block = panel(Line::from(" settings "));
    let Some(c) = &app.config else {
        return empty(f, main, block, "Loading settings", String::new());
    };
    let rows: Vec<Row> = FIELDS
        .iter()
        .map(|(key, _)| {
            let v = config_value(c, key);
            let v = if v.is_empty() {
                "system default".dim()
            } else {
                v.into()
            };
            Row::new([Cell::from(*key), Cell::from(v)])
        })
        .collect();
    let widths = vec![Constraint::Length(20), Constraint::Fill(1)];
    let table = table(rows, widths, block);
    if app.lists[2].selected().is_none() {
        app.lists[2].select(Some(0));
    }
    f.render_stateful_widget(table, main, &mut app.lists[2]);
    if let (Some(side), Some(i)) = (side, app.lists[2].selected()) {
        let path = tilde(
            &crate::paths::Paths::from_env()
                .config()
                .display()
                .to_string(),
        );
        let lines = vec![
            Line::from(FIELDS[i].1),
            Line::default(),
            Line::from(
                format!(
                    "project rules: {} · agent markers: {}",
                    c.projects.len(),
                    c.agent_markers.len()
                )
                .dim(),
            ),
            Line::from(format!("Lists are edited in {path}").dim()),
        ];
        let block = panel(Line::from(format!(" {} ", FIELDS[i].0).bold()));
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block),
            side,
        );
    }
}

// ───────────── footer and prompts ─────────────

fn footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(Prompt::Edit { key, value }) = &app.prompt {
        let line = Line::from(vec![format!(" {key} = ").bold(), value.clone().into()]);
        let x = area.x + line.width() as u16;
        f.render_widget(line, area);
        let hint = Line::from(" enter save · esc cancel ".dim()).right_aligned();
        f.render_widget(hint, area);
        f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
        return;
    }
    if let Some((text, error)) = app.flash_text() {
        let text = format!(" {text}");
        f.render_widget(if error { text.red() } else { text.green() }, area);
        return;
    }
    let keys: &[(&str, &str)] = match app.view {
        View::Servers => &[
            ("↑↓", "move"),
            ("⏎", "open"),
            ("s", "stop"),
            ("p", "pin"),
            ("t", "keep alive"),
        ],
        View::History => &[("↑↓", "move"), ("r", "restart"), ("f", "forget")],
        View::Config => &[("↑↓", "move"), ("⏎", "edit")],
    };
    let mut spans = vec![" ".into()];
    for (k, label) in keys.iter().chain(&[("tab", "view"), ("q", "quit")]) {
        spans.extend([k.bold(), format!(" {label}   ").dim()]);
    }
    f.render_widget(Line::from(spans), area);
}

fn confirm(f: &mut Frame, area: Rect, text: &str) {
    let width = area.width.saturating_sub(4).min(64);
    let inner = width.saturating_sub(4).max(1) as usize;
    let height = (Span::raw(text).width().div_ceil(inner) as u16 + 4).min(area.height);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    let [rect] = Layout::horizontal([Constraint::Length(width)])
        .flex(ratatui::layout::Flex::Center)
        .areas(row);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().yellow())
        .padding(Padding::horizontal(1));
    let lines = vec![
        Line::from(text.to_owned()),
        Line::default(),
        Line::from(vec![
            "y".bold(),
            " confirm   ".dim(),
            "any other key".bold(),
            " cancel".dim(),
        ]),
    ];
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

// ───────────── formatting ─────────────

/// `42s`, `4m12s`, `18m`, `2h05m`, `3d4h`: seconds only while they matter.
pub fn dur(ms: u64) -> String {
    let s = ms / 1000;
    let (m, h, d) = (s / 60, s / 3600, s / 86_400);
    match () {
        _ if s < 60 => format!("{s}s"),
        _ if m < 10 => format!("{m}m{:02}s", s % 60),
        _ if h < 1 => format!("{m}m"),
        _ if d < 1 => format!("{h}h{:02}m", m % 60),
        _ => format!("{d}d{}h", h % 24),
    }
}

fn bytes(b: u64) -> String {
    match b {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b => format!("{} MB", b >> 20),
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::tests::app;
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        let buf = t.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    }

    #[test]
    fn every_view_draws_at_every_size() {
        let mut app = app();
        let mut many = app.servers[0].clone();
        many.command = "node ".to_owned() + &"very/long/path/".repeat(20);
        many.cwd = "/Users/x/".to_owned() + &"deeply/nested/".repeat(20);
        let thirty: Vec<_> = (0..30)
            .map(|i| {
                let mut s = many.clone();
                s.id = format!("{i}-1");
                s.project = format!("project-with-a-rather-long-name-{}", i / 3);
                s
            })
            .collect();
        for servers in [app.servers.clone(), thirty, Vec::new()] {
            app.on_result(
                super::super::app::Req::List,
                Ok(serde_json::to_value(&servers).unwrap()),
            );
            for view in super::super::app::VIEWS {
                app.view = view;
                for (w, h) in [(30, 6), (40, 10), (80, 24), (120, 40), (200, 60)] {
                    render(&mut app, w, h);
                }
            }
        }
    }

    #[test]
    fn a_crafted_command_draws_no_escape() {
        let mut app = app();
        let mut s = app.servers[0].clone();
        s.command = "vite \u{1b}[2J\u{1b}]52;c;aGk=\u{7}".into();
        s.project = "\u{1b}[31mred".into();
        app.on_result(
            super::super::app::Req::List,
            Ok(serde_json::to_value([s]).unwrap()),
        );
        let screen = render(&mut app, 120, 30);
        assert!(!screen.contains('\u{1b}') && !screen.contains('\u{7}'));
        assert!(screen.contains("vite \u{fffd}[2J"));
    }

    #[test]
    fn durations_read_naturally() {
        let s = 1000;
        assert_eq!(
            [42 * s, 252 * s, 18 * 60 * s, 125 * 60 * s, 76 * 3600 * s].map(dur),
            ["42s", "4m12s", "18m", "2h05m", "3d4h"]
        );
    }

    /// `cargo test -p lull ui::view::tests::show -- --ignored --nocapture` prints the
    /// screens, for eyeballing a layout change.
    #[test]
    #[ignore]
    fn show() {
        let mut app = app();
        for view in super::super::app::VIEWS {
            app.view = view;
            print!("{}", render(&mut app, 120, 24));
            print!("{}", render(&mut app, 80, 30));
        }
    }
}
