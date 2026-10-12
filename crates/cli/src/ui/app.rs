//! Terminal UI state: what is on screen and what each key does. No I/O: `ui/mod.rs`
//! feeds in daemon messages and keys and sends the requests queued in `out`.

use std::collections::{HashMap, VecDeque};

use lull_protocol::rpc::{DaemonNotification, HelloResult, SubscribeResult};
use lull_protocol::{Config, OriginKind, Server, StoppedServer};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Servers,
    History,
    Config,
}

pub const VIEWS: [View; 3] = [View::Servers, View::History, View::Config];

/// What a response is for.
#[derive(Debug, Clone, PartialEq)]
pub enum Req {
    Subscribe,
    List,
    History,
    Config,
    SetConfig,
    /// An action; the text is shown when it succeeds.
    Act(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cmd {
    pub req: Req,
    pub method: &'static str,
    pub params: Value,
}

impl Cmd {
    pub fn new(req: Req, method: &'static str, params: Value) -> Cmd {
        Cmd {
            req,
            method,
            params,
        }
    }
}

pub enum Prompt {
    Confirm { text: String, cmd: Cmd },
    Edit { key: &'static str, value: String },
}

/// The scalar settings of contract §6, with the help shown beside them. Project rules
/// and agent markers are lists and stay in `config.toml`.
pub const FIELDS: [(&str, &str); 8] = [
    (
        "idle_minutes",
        "Minutes without activity before a server is stopped.",
    ),
    (
        "auto_stop",
        "Which servers lull stops on its own: agent, all or none.",
    ),
    (
        "warn_before_minutes",
        "How long a server shows as stopping soon before it is stopped.",
    ),
    (
        "wake_grace_minutes",
        "No stops for this long after the Mac wakes or the daemon starts.",
    ),
    (
        "tabs_on_stop",
        "What happens to a stopped server's tabs: discard, close or keep.",
    ),
    (
        "open_with",
        "App that opens URLs when no browser extension is connected. Empty means the system default.",
    ),
    ("history_size", "How many stopped servers to remember."),
    (
        "history_hours",
        "How many hours a stopped server is remembered.",
    ),
];

const FLASH_MS: u64 = 4000;
/// Load samples kept per server: 30 × the 5 s refresh = 2.5 minutes.
pub const SAMPLES: usize = 30;

pub struct App {
    pub view: View,
    pub now: u64,
    /// `None` while the daemon is unreachable.
    pub hello: Option<HelloResult>,
    /// Sorted by project, then port, so the list reads as groups.
    pub servers: Vec<Server>,
    pub history: Vec<StoppedServer>,
    pub config: Option<Config>,
    /// CPU % and RSS per server id, one sample per snapshot or refresh, newest last.
    pub load: HashMap<String, VecDeque<(f64, u64)>>,
    pub lists: [TableState; 3],
    pub prompt: Option<Prompt>,
    pub flash: Option<(String, bool, u64)>,
    pub out: Vec<Cmd>,
    pub quit: bool,
    /// Subscription sequence; `None` until the snapshot arrives (§8 delivery rules).
    seq: Option<u64>,
}

impl App {
    pub fn new(now: u64) -> App {
        App {
            view: View::Servers,
            now,
            hello: None,
            servers: Vec::new(),
            history: Vec::new(),
            config: None,
            load: HashMap::new(),
            lists: Default::default(),
            prompt: None,
            flash: None,
            out: Vec::new(),
            quit: false,
            seq: None,
        }
    }

    pub fn connected(&mut self, hello: HelloResult) {
        self.hello = Some(hello);
        self.seq = None;
        self.out.extend([
            Cmd::new(Req::Subscribe, "subscribe", json!({"topics": ["servers"]})),
            Cmd::new(Req::History, "servers.history", json!({})),
            Cmd::new(Req::Config, "config.get", json!({})),
        ]);
    }

    pub fn disconnected(&mut self) {
        self.hello = None;
        self.seq = None;
        self.prompt = None;
    }

    pub fn selected(&self) -> Option<usize> {
        self.lists[self.view as usize].selected()
    }

    pub fn selected_server(&self) -> Option<&Server> {
        self.servers
            .get(self.lists[View::Servers as usize].selected()?)
    }

    pub fn flash_text(&self) -> Option<(&str, bool)> {
        let (text, error, until) = self.flash.as_ref()?;
        (self.now < *until).then_some((text.as_str(), *error))
    }

    fn flash(&mut self, text: String, error: bool) {
        self.flash = Some((clean(&text), error, self.now + FLASH_MS));
    }

    // ───────────── daemon → app ─────────────

    pub fn on_note(&mut self, note: DaemonNotification) {
        let seq = match &note {
            DaemonNotification::ServerAdded(e) | DaemonNotification::ServerUpdated(e) => e.seq,
            DaemonNotification::ServerRemoved(r) => r.seq,
        };
        match self.seq {
            Some(s) if seq == s + 1 => self.seq = Some(seq),
            Some(s) if seq <= s => return,
            Some(_) => {
                // A gap: the snapshot is stale, so ask for a fresh one.
                self.seq = None;
                self.out.push(Cmd::new(
                    Req::Subscribe,
                    "subscribe",
                    json!({"topics": ["servers"]}),
                ));
                return;
            }
            None => return,
        }
        match note {
            DaemonNotification::ServerAdded(e) | DaemonNotification::ServerUpdated(e) => {
                let mut servers = std::mem::take(&mut self.servers);
                servers.retain(|s| s.id != e.server.id);
                servers.push(e.server);
                self.set_servers(servers);
            }
            DaemonNotification::ServerRemoved(r) => {
                if let Some(gone) = self.servers.iter().find(|s| s.id == r.id) {
                    let why = match r.reason {
                        lull_protocol::StopReason::Idle => "stopped: idle",
                        lull_protocol::StopReason::Manual => "stopped",
                        lull_protocol::StopReason::Exited => "exited",
                    };
                    self.flash(format!("{} {why}", gone.project), false);
                }
                let mut servers = std::mem::take(&mut self.servers);
                servers.retain(|s| s.id != r.id);
                self.set_servers(servers);
                self.out
                    .push(Cmd::new(Req::History, "servers.history", json!({})));
            }
        }
    }

    pub fn on_result(&mut self, req: Req, result: Result<Value, String>) {
        let value = match result {
            Ok(v) => v,
            Err(e) => return self.flash(e, true),
        };
        let decoded = match req {
            Req::Subscribe => serde_json::from_value(value).map(|r: SubscribeResult| {
                self.seq = Some(r.seq);
                self.set_servers(r.servers);
                self.sample();
            }),
            Req::List => serde_json::from_value(value).map(|s| {
                self.set_servers(s);
                self.sample();
            }),
            Req::History => serde_json::from_value(value).map(|h| self.set_history(h)),
            Req::Config | Req::SetConfig => serde_json::from_value(value).map(|mut c: Config| {
                c.open_with = clean(&c.open_with);
                self.config = Some(c);
            }),
            Req::Act(_) => Ok(()),
        };
        if let Err(e) = decoded {
            return self.flash(format!("unexpected reply from the daemon: {e}"), true);
        }
        match req {
            Req::Act(text) => {
                self.flash(text, false);
                self.out
                    .push(Cmd::new(Req::History, "servers.history", json!({})));
            }
            Req::SetConfig => self.flash("saved".into(), false),
            _ => {}
        }
    }

    /// Replace the servers, keeping the cursor on the same server when it survives.
    fn set_servers(&mut self, mut servers: Vec<Server>) {
        servers.iter_mut().for_each(clean_server);
        servers.sort_by(|a, b| {
            (&a.project, a.ports.first(), &a.id).cmp(&(&b.project, b.ports.first(), &b.id))
        });
        let list = &mut self.lists[View::Servers as usize];
        let id = list
            .selected()
            .and_then(|i| self.servers.get(i))
            .map(|s| s.id.clone());
        let at = id.and_then(|id| servers.iter().position(|s| s.id == id));
        let at = at.or(list.selected()).unwrap_or(0);
        self.servers = servers;
        let n = self.servers.len();
        list.select((n > 0).then(|| at.min(n - 1)));
    }

    /// Record each server's current load; forget servers that are gone.
    fn sample(&mut self) {
        let live: HashMap<_, _> = self.servers.iter().map(|s| (&s.id, s)).collect();
        self.load.retain(|id, _| live.contains_key(id));
        for (id, s) in live {
            let samples = self.load.entry(id.clone()).or_default();
            samples.push_back((s.cpu_percent, s.rss_bytes));
            if samples.len() > SAMPLES {
                samples.pop_front();
            }
        }
    }

    fn set_history(&mut self, mut history: Vec<StoppedServer>) {
        for h in &mut history {
            h.project = clean(&h.project);
            h.command = clean(&h.command);
            h.cwd = clean(&h.cwd);
            h.argv.iter_mut().for_each(|a| *a = clean(a));
            clean_origin(&mut h.origin);
            h.restart_env = h
                .restart_env
                .iter()
                .map(|(k, v)| (clean(k), clean(v)))
                .collect();
            h.tabs
                .iter_mut()
                .for_each(|t| t.browser = clean(&t.browser));
        }
        self.history = history;
        let n = self.history.len();
        let list = &mut self.lists[View::History as usize];
        list.select((n > 0).then(|| list.selected().unwrap_or(0).min(n - 1)));
    }

    // ───────────── keys ─────────────

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.prompt.take() {
            Some(Prompt::Confirm { cmd, .. }) => {
                if matches!(k.code, KeyCode::Char('y' | 'Y')) {
                    self.out.push(cmd);
                }
                return;
            }
            Some(Prompt::Edit { key, mut value }) => {
                match k.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => self.set_config(key, &value),
                    KeyCode::Backspace => {
                        value.pop();
                        self.prompt = Some(Prompt::Edit { key, value });
                    }
                    KeyCode::Char(c) if !c.is_control() => {
                        value.push(c);
                        self.prompt = Some(Prompt::Edit { key, value });
                    }
                    _ => self.prompt = Some(Prompt::Edit { key, value }),
                }
                return;
            }
            None => {}
        }
        let i = self.view as usize;
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Tab => self.view = VIEWS[(i + 1) % 3],
            KeyCode::BackTab => self.view = VIEWS[(i + 2) % 3],
            KeyCode::Char(c @ '1'..='3') => self.view = VIEWS[c as usize - '1' as usize],
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Home | KeyCode::Char('g') => self.step(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.step(isize::MAX),
            code => match self.view {
                View::Servers => self.server_action(code),
                View::History => self.history_action(code),
                View::Config => self.config_action(code),
            },
        }
    }

    fn step(&mut self, by: isize) {
        let len = match self.view {
            View::Servers => self.servers.len(),
            View::History => self.history.len(),
            View::Config => FIELDS.len(),
        };
        if len == 0 {
            return;
        }
        let list = &mut self.lists[self.view as usize];
        let at = list.selected().unwrap_or(0).saturating_add_signed(by);
        list.select(Some(at.min(len - 1)));
    }

    fn require_daemon(&mut self) -> bool {
        if self.hello.is_none() {
            self.flash("the daemon is not reachable".into(), true);
        }
        self.hello.is_some()
    }

    fn server_action(&mut self, code: KeyCode) {
        let Some(s) = self.selected_server() else {
            return;
        };
        let (id, name, pinned) = (s.id.clone(), s.project.clone(), s.pinned);
        let mine = (s.origin.kind == OriginKind::User).then(|| s.command.clone());
        let cmd = match code {
            KeyCode::Enter | KeyCode::Char('o') => Cmd::new(
                Req::Act(format!("opening {name}")),
                "servers.open",
                json!({"id": id}),
            ),
            KeyCode::Char('s') => {
                let cmd = Cmd::new(
                    Req::Act(format!("stopping {name}")),
                    "servers.stop",
                    json!({"id": id}),
                );
                // lull stops agent-started servers anyway; one the user started is theirs.
                if let Some(command) = mine.filter(|_| self.require_daemon()) {
                    let text = format!("Stop {name}? You started it: {}", tilde(&command));
                    self.prompt = Some(Prompt::Confirm { text, cmd });
                    return;
                }
                cmd
            }
            KeyCode::Char('p') => Cmd::new(
                Req::Act(format!(
                    "{} {name}",
                    if pinned { "unpinned" } else { "pinned" }
                )),
                "servers.pin",
                json!({"id": id, "pinned": !pinned}),
            ),
            KeyCode::Char('t') => Cmd::new(
                Req::Act(format!("kept {name} alive")),
                "servers.touch",
                json!({"id": id}),
            ),
            _ => return,
        };
        if self.require_daemon() {
            self.out.push(cmd);
        }
    }

    fn history_action(&mut self, code: KeyCode) {
        let Some(h) = self.selected().and_then(|i| self.history.get(i)) else {
            return;
        };
        let (id, name) = (h.id.clone(), h.project.clone());
        match code {
            KeyCode::Enter | KeyCode::Char('r') => {
                if h.argv.is_empty() {
                    return self.flash(format!("{name} has no launch command to restart"), true);
                }
                // Restart runs a recorded command, so show it before running it.
                let text = format!(
                    "Restart {name}? Runs `{}` in {}",
                    tilde(&h.command),
                    tilde(&h.cwd)
                );
                let cmd = Cmd::new(
                    Req::Act(format!("restarting {name}")),
                    "servers.restart",
                    json!({"id": id}),
                );
                if self.require_daemon() {
                    self.prompt = Some(Prompt::Confirm { text, cmd });
                }
            }
            KeyCode::Char('f') | KeyCode::Delete if self.require_daemon() => {
                self.out.push(Cmd::new(
                    Req::Act(format!("forgot {name}")),
                    "servers.forget",
                    json!({"id": id}),
                ));
            }
            _ => {}
        }
    }

    fn config_action(&mut self, code: KeyCode) {
        let (Some(i), Some(config)) = (self.selected(), &self.config) else {
            return;
        };
        if code == KeyCode::Enter && self.hello.is_some() {
            let key = FIELDS[i].0;
            self.prompt = Some(Prompt::Edit {
                key,
                value: config_value(config, key),
            });
        }
    }

    /// Checked here so a typo gets a plain answer, not the daemon's decoding error.
    fn set_config(&mut self, key: &str, value: &str) {
        let value = value.trim();
        let value = match accepts(key) {
            Accepts::Number => value.parse::<u32>().ok().map(|n| json!(n)),
            Accepts::OneOf(words) => words.contains(&value).then(|| json!(value)),
            Accepts::Text => Some(json!(value)),
        };
        match value {
            Some(v) => self.out.push(Cmd::new(
                Req::SetConfig,
                "config.set",
                json!({"patch": {key: v}}),
            )),
            None => self.flash(format!("{key} takes {}", accepts(key).describe()), true),
        }
    }
}

pub enum Accepts {
    Number,
    OneOf(&'static [&'static str]),
    Text,
}

impl Accepts {
    pub fn describe(&self) -> String {
        match self {
            Accepts::Number => "a whole number".into(),
            Accepts::OneOf(words) => words.join(", "),
            Accepts::Text => "an app name, or nothing for the default".into(),
        }
    }
}

/// What each §6 setting in `FIELDS` accepts.
pub fn accepts(key: &str) -> Accepts {
    match key {
        "auto_stop" => Accepts::OneOf(&["agent", "all", "none"]),
        "tabs_on_stop" => Accepts::OneOf(&["discard", "close", "keep"]),
        "open_with" => Accepts::Text,
        _ => Accepts::Number,
    }
}

/// A §6 setting as the user would type it (`30`, `agent`, an app name).
pub fn config_value(config: &Config, key: &str) -> String {
    match &serde_json::to_value(config).unwrap()[key] {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

/// Every `$HOME/` in a path or command line → `~/`, for display.
pub fn tilde(text: &str) -> String {
    let home = crate::paths::home();
    if home == "/" || home.is_empty() {
        return text.to_owned();
    }
    if text == home {
        return "~".into();
    }
    text.replace(&format!("{}/", home.trim_end_matches('/')), "~/")
}

/// Process-supplied text is untrusted (ADR-0009): an agent controls argv, cwd and the
/// environment. Control characters would reach the terminal as escape sequences and
/// bidi overrides can disguise a command, so both are replaced before anything is drawn.
pub fn clean(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            c if c.is_control() => '\u{fffd}',
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => '\u{fffd}',
            c => c,
        })
        .collect()
}

fn clean_origin(o: &mut lull_protocol::Origin) {
    if let Some(lull_protocol::Agent::Other(name)) = &mut o.agent {
        *name = clean(name);
    }
    if let Some(id) = &mut o.session_id {
        *id = clean(id);
    }
}

fn clean_server(s: &mut Server) {
    s.project = clean(&s.project);
    s.command = clean(&s.command);
    s.cwd = clean(&s.cwd);
    s.argv.iter_mut().for_each(|a| *a = clean(a));
    s.primary_url = s.primary_url.as_deref().map(clean);
    clean_origin(&mut s.origin);
    s.tabs
        .iter_mut()
        .for_each(|t| t.browser = clean(&t.browser));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;

    pub fn fixture(name: &str, line: usize) -> Value {
        let path = format!(
            "{}/../../protocol/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(text.lines().nth(line).unwrap()).unwrap()
    }

    /// An app connected to a daemon that answered with the fixtures.
    pub fn app() -> App {
        let mut app = App::new(crate::cli::parse_rfc3339("2026-10-11T20:38:00Z").unwrap());
        app.connected(
            serde_json::from_value(fixture("hello.ndjson", 1)["result"].clone()).unwrap(),
        );
        app.out.clear();
        app.on_result(
            Req::List,
            Ok(fixture("servers-list.ndjson", 1)["result"].clone()),
        );
        app.on_result(
            Req::History,
            Ok(fixture("servers-history.ndjson", 1)["result"].clone()),
        );
        app.on_result(
            Req::Config,
            Ok(fixture("config-get.ndjson", 1)["result"].clone()),
        );
        app
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(c))
    }

    fn select(app: &mut App, project: &str) {
        let i = app
            .servers
            .iter()
            .position(|s| s.project == project)
            .unwrap();
        app.lists[0].select(Some(i));
    }

    #[test]
    fn stopping_a_user_started_server_asks_first() {
        let mut app = app();
        select(&mut app, "site");
        app.on_key(key('s'));
        assert!(app.out.is_empty() && matches!(app.prompt, Some(Prompt::Confirm { .. })));
        app.on_key(key('n'));
        assert!(app.out.is_empty() && app.prompt.is_none());
        app.on_key(key('s'));
        app.on_key(key('y'));
        assert_eq!(app.out[0].method, "servers.stop");
    }

    #[test]
    fn stopping_an_agent_started_server_does_not_ask() {
        let mut app = app();
        select(&mut app, "crm");
        app.on_key(key('s'));
        assert!(app.prompt.is_none());
        assert_eq!(app.out[0].method, "servers.stop");
    }

    #[test]
    fn a_sequence_gap_resubscribes() {
        let mut app = app();
        app.on_result(
            Req::Subscribe,
            Ok(fixture("subscribe.ndjson", 1)["result"].clone()),
        );
        let note = |seq: u64| {
            let mut n = fixture("subscribe.ndjson", 2);
            n["params"]["seq"] = json!(seq);
            serde_json::from_value::<DaemonNotification>(
                json!({"method": n["method"], "params": n["params"]}),
            )
            .unwrap()
        };
        app.on_note(note(42));
        assert!(app.out.is_empty());
        app.on_note(note(44));
        assert_eq!(app.out[0].method, "subscribe");
    }

    #[test]
    fn the_cursor_follows_its_server_across_updates() {
        let mut app = app();
        select(&mut app, "site");
        let mut servers = app.servers.clone();
        let mut early = servers[0].clone();
        early.id = "1-1".into();
        early.project = "aaa".into();
        servers.push(early);
        app.on_result(Req::List, Ok(serde_json::to_value(servers).unwrap()));
        assert_eq!(app.selected_server().unwrap().project, "site");
    }

    #[test]
    fn config_edits_are_checked_before_they_are_sent() {
        let mut app = app();
        for (key, bad) in [
            ("auto_stop", "bogus"),
            ("idle_minutes", "soon"),
            ("idle_minutes", "-1"),
        ] {
            app.set_config(key, bad);
            assert!(
                app.out.is_empty() && app.flash_text().unwrap().1,
                "{key}={bad}"
            );
        }
        app.set_config("idle_minutes", " 45 ");
        app.set_config("open_with", "30");
        assert_eq!(app.out[0].params, json!({"patch": {"idle_minutes": 45}}));
        assert_eq!(app.out[1].params, json!({"patch": {"open_with": "30"}}));
    }

    #[test]
    fn load_keeps_the_last_samples_of_live_servers() {
        let mut app = app();
        let list = fixture("servers-list.ndjson", 1)["result"].clone();
        for _ in 0..SAMPLES + 5 {
            app.on_result(Req::List, Ok(list.clone()));
        }
        assert!(app.load.values().all(|v| v.len() == SAMPLES));
        let one = json!([list[0].clone()]);
        app.on_result(Req::List, Ok(one));
        assert_eq!(app.load.len(), 1);
    }

    #[test]
    fn home_paths_shorten_everywhere_in_a_command() {
        let home = crate::paths::home();
        assert_eq!(
            tilde(&format!("{home}/n/node {home}/app/x.mjs")),
            "~/n/node ~/app/x.mjs"
        );
        assert_eq!(tilde(&format!("{home}x")), format!("{home}x"));
    }

    #[test]
    fn untrusted_text_loses_control_and_bidi_characters() {
        assert_eq!(
            clean("vite\u{1b}[2J\u{202e}x\u{9b}"),
            "vite\u{fffd}[2J\u{fffd}x\u{fffd}"
        );
    }
}
