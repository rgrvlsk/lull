//! WP2.4 — browser connections (contract §5, §8, §9): the per-connection tab registry,
//! tab ↔ server binding, `tab` activity and covered browsers. Pure state; the daemon
//! sends the resulting `tab.*` commands.

use std::collections::{BTreeMap, BTreeSet};

use lull_protocol::rpc::BrowserTab;
use lull_protocol::server::{Activity, TabRef};

use crate::model::{ActivityEvent, CoveredBrowsers, ObservedServer, ServerId};
use crate::traits::Instant;

pub type ConnId = u64;

struct Conn {
    /// `hello.browser.name`, e.g. `helium`.
    name: String,
    /// The browser process: the nm-host's parent (§5 "Covered browser").
    browser_pid: Option<u32>,
    tabs: Vec<BrowserTab>,
    /// Higher = focused more recently (§8 `servers.open` routing).
    focused: u64,
}

/// A tab bound to a server id, or to the stopped-server record with that id (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    id: ServerId,
    /// The URL the tab showed when it was bound; a different URL is a new load.
    url: String,
}

#[derive(Default)]
pub struct Registry {
    conns: BTreeMap<ConnId, Conn>,
    bindings: BTreeMap<(ConnId, i64), Binding>,
    clock: u64,
}

/// Port of a loopback `http(s)` URL (§9 host permissions).
pub fn loopback_port(url: &str) -> Option<u16> {
    crate::ipc::loopback_url_port(url)
}

fn server_on(port: u16, servers: &[ObservedServer]) -> Option<&ObservedServer> {
    servers.iter().find(|s| s.ports.contains(&port))
}

impl Registry {
    /// A browser said `hello`.
    pub fn connect(&mut self, conn: ConnId, name: String, browser_pid: Option<u32>) {
        self.clock += 1;
        let conn_state = Conn {
            name,
            browser_pid,
            tabs: Vec::new(),
            focused: self.clock,
        };
        self.conns.insert(conn, conn_state);
    }

    pub fn is_browser(&self, conn: ConnId) -> bool {
        self.conns.contains_key(&conn)
    }

    pub fn has_browsers(&self) -> bool {
        !self.conns.is_empty()
    }

    pub fn name(&self, conn: ConnId) -> Option<&str> {
        self.conns.get(&conn).map(|c| c.name.as_str())
    }

    /// Everything the connection reported goes at once: tabs, bindings, coverage (§8).
    pub fn disconnect(&mut self, conn: ConnId) {
        self.conns.remove(&conn);
        self.bindings.retain(|(c, _), _| *c != conn);
    }

    /// `browser.tabs.sync`: a full snapshot of the connection's loopback tabs. A tab keeps
    /// its binding while it shows the URL it was bound with; a new URL is matched afresh
    /// against the live servers by port (§9 "Binding").
    pub fn sync(&mut self, conn: ConnId, tabs: Vec<BrowserTab>, servers: &[ObservedServer]) {
        let Some(c) = self.conns.get_mut(&conn) else {
            return;
        };
        if tabs.iter().any(|t| t.window_focused) {
            self.clock += 1;
            c.focused = self.clock;
        }
        let present: BTreeSet<i64> = tabs.iter().map(|t| t.tab_id).collect();
        self.bindings
            .retain(|(c, tab), _| *c != conn || present.contains(tab));
        for tab in &tabs {
            let key = (conn, tab.tab_id);
            if self.bindings.get(&key).is_some_and(|b| b.url == tab.url) {
                continue;
            }
            match loopback_port(&tab.url).and_then(|p| server_on(p, servers)) {
                Some(s) => {
                    let binding = Binding {
                        id: s.id.clone(),
                        url: tab.url.clone(),
                    };
                    self.bindings.insert(key, binding);
                }
                None => {
                    self.bindings.remove(&key);
                }
            }
        }
        c.tabs = tabs;
    }

    /// `browser.activity`: the tab is visible and the user is present (§5 `tab`). A tab
    /// still bound to a stopped-server record is matched afresh: its content script only
    /// runs on a page that loaded, so a server answered on that port again (§9).
    pub fn activity(
        &mut self,
        conn: ConnId,
        tab_id: i64,
        url: &str,
        servers: &[ObservedServer],
        now: &Instant,
    ) -> Option<ActivityEvent> {
        if !self.conns.contains_key(&conn) {
            return None;
        }
        let key = (conn, tab_id);
        let live = |id: &str| servers.iter().any(|s| s.id == id);
        let bound = self
            .bindings
            .get(&key)
            .filter(|b| b.url == url && live(&b.id))
            .map(|b| b.id.clone());
        let id = match bound {
            Some(id) => id,
            None => {
                let s = server_on(loopback_port(url)?, servers)?;
                let binding = Binding {
                    id: s.id.clone(),
                    url: url.to_owned(),
                };
                self.bindings.insert(key, binding);
                s.id.clone()
            }
        };
        Some(ActivityEvent {
            server_id: id,
            source: Activity::Tab,
            at: now.clone(),
        })
    }

    /// Tabs bound to a server or stopped-server record, for `Server.tabs` and history.
    pub fn tabs_of(&self, id: &str) -> Vec<TabRef> {
        self.bindings
            .iter()
            .filter(|(_, b)| b.id == id)
            .filter_map(|((conn, tab_id), _)| {
                let c = self.conns.get(conn)?;
                let tab = c.tabs.iter().find(|t| t.tab_id == *tab_id)?;
                Some(TabRef {
                    connection_id: format!("c{conn}"),
                    browser: c.name.clone(),
                    tab_id: *tab_id,
                    window_id: tab.window_id,
                })
            })
            .collect()
    }

    /// Tab ids bound to `id`, per connection (for `tab.*` commands).
    pub fn bound_tabs(&self, id: &str) -> BTreeMap<ConnId, Vec<i64>> {
        let mut out: BTreeMap<ConnId, Vec<i64>> = BTreeMap::new();
        for ((conn, tab), b) in &self.bindings {
            if b.id == id {
                out.entry(*conn).or_default().push(*tab);
            }
        }
        out
    }

    /// A restart completed: the record's tabs now belong to the new server (§7).
    pub fn rebind(&mut self, from: &str, to: &str) {
        for b in self.bindings.values_mut().filter(|b| b.id == from) {
            b.id = to.to_owned();
        }
    }

    /// A bound tab already showing `url`, if any (§8 `servers.open`: focus it).
    pub fn tab_showing(&self, url: &str) -> Option<(ConnId, i64)> {
        self.conns.iter().find_map(|(conn, c)| {
            let tab = c
                .tabs
                .iter()
                .find(|t| t.url == url && self.bindings.contains_key(&(*conn, t.tab_id)))?;
            Some((*conn, tab.tab_id))
        })
    }

    /// The most recently focused browser connection (§8 `servers.open`).
    pub fn open_target(&self) -> Option<ConnId> {
        self.conns
            .iter()
            .max_by_key(|(_, c)| c.focused)
            .map(|(id, _)| *id)
    }

    /// Browsers with a live extension connection (§5).
    pub fn covered(&self) -> CoveredBrowsers {
        CoveredBrowsers(self.conns.values().filter_map(|c| c.browser_pid).collect())
    }

    /// For `lull doctor`: `(connectionId, name, browser pid, tab count)`.
    pub fn summary(&self) -> Vec<(String, String, Option<u32>, usize)> {
        self.conns
            .iter()
            .map(|(id, c)| {
                (
                    format!("c{id}"),
                    c.name.clone(),
                    c.browser_pid,
                    c.tabs.len(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
