//! Snapshot-fixture tests (plan WP1.2 "Accept"). Fixtures are `serde_json` of [`Snapshot`]
//! under `crates/core/fixtures/snapshots/`; `$HOME` is `/Users/x`.

use std::collections::BTreeSet;

use lull_protocol::AgentMarker;
use lull_protocol::server::{Agent, Framework, KnownAgent, Origin, OriginKind};

use super::patterns::{PROTECTED, Protect, matches};
use super::*;

const HOME: &str = "/Users/x";
const T: u64 = 1_791_530_155_000;
/// The daemon runs inside the concurrently group of `agent-runtimes.json`.
const DAEMON_PID: u32 = 50906;
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/snapshots");

fn load(name: &str) -> Snapshot {
    let text = std::fs::read_to_string(format!("{FIXTURES}/{name}")).expect(name);
    serde_json::from_str(&text).expect(name)
}

fn git_root(cwd: &str) -> Option<String> {
    const ROOTS: &[&str] = &[
        "/Users/x/dev/acme",
        "/Users/x/dev/mono",
        "/Users/x/dev/turbo-mono",
        "/Users/x/dev/lull",
        "/Users/x/dev/example-landing",
    ];
    ROOTS
        .iter()
        .find(|r| cwd == **r || cwd.starts_with(&format!("{r}/")))
        .map(|r| r.to_string())
}

fn ctx_with(extra: &[AgentMarker]) -> Context<'_> {
    Context {
        home: HOME,
        uid: 501,
        daemon_pid: DAEMON_PID,
        extra_markers: extra,
        git_root: &git_root,
    }
}

fn run(snap: &Snapshot, prev: &[ObservedServer], resume: &[KnownGroup]) -> Discovery {
    discover(snap, &ctx_with(&[]), prev, resume)
}

fn fixture(name: &str) -> Discovery {
    run(&load(name), &[], &[])
}

fn server(d: &Discovery, pgid: u32) -> &ObservedServer {
    d.servers
        .iter()
        .find(|s| s.pgid == pgid)
        .unwrap_or_else(|| panic!("no server {pgid} in {d:#?}"))
}

fn pgids(d: &Discovery) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    (
        d.servers.iter().map(|s| s.pgid).collect(),
        d.unmanaged.iter().map(|u| u.pgid).collect(),
        d.protected.iter().copied().collect(),
    )
}

fn user() -> Origin {
    Origin {
        kind: OriginKind::User,
        agent: None,
        session_id: None,
    }
}

fn agent(agent: Agent, session: Option<&str>) -> Origin {
    Origin {
        kind: OriginKind::Agent,
        agent: Some(agent),
        session_id: session.map(str::to_owned),
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

const CLAUDE_SESSION: &str = "d263f0c2-5b1e-4f7a-9c11-2e8f7d4b6a90";

#[test]
fn every_fixture_decodes() {
    let mut names: Vec<_> = std::fs::read_dir(FIXTURES)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert!(names.len() >= 10, "{names:?}");
    for name in names {
        let snap = load(&name);
        assert!(!snap.processes.is_empty(), "{name}");
    }
}

#[test]
fn author_machine_has_two_servers_among_thirteen_groups() {
    let d = fixture("author-machine.json");
    let (servers, unmanaged, protected) = pgids(&d);
    assert_eq!(servers, [5826, 22249]);
    // Herd nginx, redis, postgres, mariadb, rapportd, ControlCenter, Raycast, Canary Mail, Zed.
    assert_eq!(
        unmanaged,
        [612, 659, 733, 1824, 5397, 5411, 5414, 21961, 64567]
    );
    // Helium; T3 Code with its cloudflared tunnel and Claude Code sessions.
    assert_eq!(protected, [701, 93154]);

    let vite = server(&d, 22249);
    assert_eq!(vite.id, format!("22249-{}", T + 1000));
    assert_eq!(
        vite.members.iter().map(|w| w.pid).collect::<Vec<_>>(),
        [22250, 22276, 22293]
    );
    assert_eq!(vite.command, "npm run dev");
    assert_eq!(vite.launch_argv, strings(&["npm", "run", "dev"]));
    assert_eq!(vite.cwd, "/Users/x/proj/apps/crm");
    assert_eq!(vite.project, "crm");
    assert_eq!(vite.framework, Framework::Vite);
    assert_eq!(vite.ports, [5187, 56306, 61811]);
    // workerd's argv names 56306 (`--external-addr=loopback=…`), which must not win.
    assert_eq!(vite.primary_url.as_deref(), Some("http://127.0.0.1:5187/"));
    assert_eq!(
        vite.origin,
        agent(Agent::Known(KnownAgent::Claude), Some(CLAUDE_SESSION))
    );
    assert_eq!(vite.started_at_ms, T + 1000);
    assert_eq!(vite.rss_bytes, 20_000_000 + 120_000_000 + 42_452_224);
    assert!(vite.complete());

    let astro = server(&d, 5826);
    assert_eq!(astro.framework, Framework::Astro);
    assert_eq!(astro.project, "example-landing");
    assert_eq!(astro.origin, user());
    assert_eq!(astro.ports, [4328, 62430, 62431, 62432]);
    assert_eq!(astro.primary_url.as_deref(), Some("http://127.0.0.1:4328/"));
    assert_eq!(
        astro.command,
        "node ./node_modules/.bin/astro dev --host 127.0.0.1 --port 4328 --strictPort"
    );

    let zed = d.unmanaged.iter().find(|u| u.pgid == 64567).unwrap();
    assert_eq!(zed.ports, [44438]);
    assert_eq!(zed.cwd.as_deref(), Some("/"));
    assert_eq!(zed.command, "/Applications/Zed.app/Contents/MacOS/zed");
}

#[test]
fn claude_bash_tool_launch_command_skips_the_harness_shell() {
    let d = fixture("claude-bash-tool.json");
    let (servers, unmanaged, protected) = pgids(&d);
    assert_eq!(
        (servers, unmanaged, protected),
        (vec![31000], vec![], vec![30990])
    );
    let s = server(&d, 31000);
    assert_eq!(s.launch_argv, strings(&["npm", "run", "dev"]));
    assert_eq!(s.command, "npm run dev");
    assert_eq!(s.framework, Framework::Vite);
    assert_eq!(s.project, "acme");
    assert_eq!(s.cwd, "/Users/x/dev/acme/web");
    assert_eq!(
        s.origin,
        agent(Agent::Known(KnownAgent::Claude), Some(CLAUDE_SESSION))
    );
    assert_eq!(s.primary_url.as_deref(), Some("http://127.0.0.1:5173/"));
    assert_eq!(s.members.len(), 5, "the esbuild helper is a member");
    assert!(s.restart_env.is_empty());
    assert!(!s.env_pin);
}

#[test]
fn ide_terminal_with_claudecode_and_a_tty_is_user() {
    let d = fixture("ide-terminal-claude.json");
    let s = server(&d, 41000);
    assert_eq!(s.origin, user());
    assert_eq!(s.framework, Framework::Next);
    assert_eq!(s.command, "npm run dev");
    assert_eq!(s.project, "shop");
}

#[test]
fn homebrew_python_app_is_a_server() {
    let d = fixture("homebrew-python.json");
    let s = server(&d, 42000);
    assert_eq!(s.framework, Framework::PythonHttp);
    assert_eq!(s.cwd, "/private/tmp/preview");
    assert_eq!(s.project, "preview");
    assert_eq!(s.primary_url.as_deref(), Some("http://127.0.0.1:8000/"));
    assert_eq!(
        s.origin,
        agent(
            Agent::Known(KnownAgent::Codex),
            Some("0199c2a1-7e4b-7d10-a3f2-5b8e9c0d1e2f")
        )
    );
}

#[test]
fn agent_runtimes_and_the_daemon_are_protected() {
    let d = fixture("agent-runtimes.json");
    let (servers, unmanaged, protected) = pgids(&d);
    assert_eq!(
        protected,
        [50100, 50200, 50300, 50400, 50500, 50600, 50700, 50900]
    );
    assert!(unmanaged.is_empty());
    // `node ~/dev/claude/index.js` is only named like an agent.
    assert_eq!(servers, [51000]);
    let s = server(&d, 51000);
    assert_eq!(s.primary_url.as_deref(), Some("http://127.0.0.1:8080/"));
    assert_eq!(s.restart_env.get("PORT").map(String::as_str), Some("8080"));
}

#[test]
fn t3_code_and_its_direct_child_are_protected() {
    let d = fixture("t3-code.json");
    assert_eq!(pgids(&d), (vec![], vec![], vec![5460, 93154]));
}

#[test]
fn every_browser_bundle_is_protected_wherever_installed() {
    let d = fixture("browsers.json");
    assert!(d.servers.is_empty() && d.unmanaged.is_empty(), "{d:#?}");
    assert_eq!(d.protected.len(), BROWSER_BUNDLES.len());
}

#[test]
fn codex_in_a_terminal_is_user() {
    let d = fixture("codex-terminal.json");
    assert_eq!(d.protected, BTreeSet::from([69990]));
    let s = server(&d, 70000);
    assert_eq!(s.origin, user());
    assert_eq!(s.framework, Framework::Remix);
    assert_eq!(s.launch_argv, strings(&["pnpm", "dev"]));
}

#[test]
fn codex_under_t3_is_agent_and_t3_env_alone_is_not() {
    let d = fixture("codex-t3.json");
    assert_eq!(d.protected, BTreeSet::from([1257]));
    let s = server(&d, 71000);
    assert_eq!(
        s.origin,
        agent(
            Agent::Known(KnownAgent::Codex),
            Some("0199c2a1-7e4b-7d10-a3f2-5b8e9c0d1e2f")
        )
    );
    assert_eq!(s.framework, Framework::Next);
    assert_eq!(s.launch_argv, strings(&["pnpm", "dev"]));
    assert_eq!(s.restart_env, [("PORT".into(), "3001".into())].into());
    assert_eq!(s.primary_url.as_deref(), Some("http://127.0.0.1:3001/"));
    assert_eq!(s.project, "acme");

    let editor_started = server(&d, 72000);
    assert_eq!(editor_started.origin, user());
    assert_eq!(
        editor_started.primary_url.as_deref(),
        Some("http://127.0.0.1:5180/")
    );
}

#[test]
fn other_marker_table_agents_and_config_markers() {
    let extra = [AgentMarker {
        env: "AIDER_CHAT".into(),
        value: Some("1".into()),
        agent: "aider".into(),
    }];
    let d = discover(&load("other-agents.json"), &ctx_with(&extra), &[], &[]);

    let gemini = server(&d, 73000);
    assert_eq!(gemini.origin, agent(Agent::Known(KnownAgent::Gemini), None));
    assert_eq!(gemini.framework, Framework::Nuxt);
    assert_eq!(gemini.command, "npm run dev");
    assert_eq!(
        gemini.primary_url.as_deref(),
        Some("http://127.0.0.1:3000/")
    );

    let opencode = server(&d, 74000);
    assert_eq!(
        opencode.origin,
        agent(Agent::Known(KnownAgent::Opencode), None)
    );
    assert_eq!(opencode.framework, Framework::Artisan);
    assert_eq!(opencode.command, "php artisan serve --port=8001");
    assert_eq!(
        opencode.primary_url.as_deref(),
        Some("http://127.0.0.1:8001/")
    );

    let ai_agent_only = server(&d, 75000);
    assert_eq!(ai_agent_only.origin, user());
    assert_eq!(ai_agent_only.framework, Framework::Astro);

    let aider = server(&d, 76000);
    assert_eq!(aider.origin, agent(Agent::Other("aider".into()), None));
    assert_eq!(aider.framework, Framework::Rails);
    assert_eq!(aider.primary_url.as_deref(), Some("http://127.0.0.1:3002/"));

    let without_config = fixture("other-agents.json");
    assert_eq!(server(&without_config, 76000).origin, user());
}

#[test]
fn wrappers_concurrently_nodemon_turbo_wrangler() {
    let d = fixture("wrappers.json");
    assert_eq!(pgids(&d).0, [80000, 80100, 80200, 80300]);

    let concurrently = server(&d, 80000);
    assert_eq!(concurrently.command, "npm run dev");
    assert_eq!(concurrently.framework, Framework::Vite);
    assert_eq!(concurrently.members.len(), 10);
    assert_eq!(concurrently.project, "mono");
    assert_eq!(concurrently.origin, user());

    let nodemon = server(&d, 80100);
    assert_eq!(nodemon.command, "npm run dev:api");
    assert_eq!(nodemon.framework, Framework::Generic);
    assert_eq!(
        nodemon.primary_url.as_deref(),
        Some("http://127.0.0.1:10000/")
    );
    assert!(!nodemon.env_pin, "DEVSERVER_KEEP is not honoured");

    let turbo = server(&d, 80200);
    assert_eq!(turbo.launch_argv, strings(&["turbo", "run", "dev"]));
    assert_eq!(turbo.framework, Framework::Next);
    assert_eq!(turbo.cwd, "/Users/x/dev/turbo-mono/apps/web");
    assert_eq!(turbo.project, "turbo-mono");
    assert_eq!(turbo.ports, [3100, 5173]);
    assert_eq!(turbo.primary_url.as_deref(), Some("http://127.0.0.1:3100/"));
    assert_eq!(
        turbo.origin,
        agent(Agent::Known(KnownAgent::Claude), Some(CLAUDE_SESSION))
    );

    let wrangler = server(&d, 80300);
    assert_eq!(wrangler.command, "npm exec wrangler dev");
    assert_eq!(wrangler.framework, Framework::Wrangler);
    assert_eq!(wrangler.ports, [8787, 9229, 61234]);
    assert_eq!(
        wrangler.primary_url.as_deref(),
        Some("http://127.0.0.1:8787/")
    );
    assert!(wrangler.env_pin);
}

// ───────────────────────── identity and incomplete observation ─────────────────────────

/// `claude-bash-tool.json` after the harness shell (the group leader) exited.
fn leader_exited() -> Snapshot {
    let mut snap = load("claude-bash-tool.json");
    snap.processes.retain(|p| p.pid != 31000);
    for p in &mut snap.processes {
        if p.pid == 31002 {
            p.ppid = 1;
            p.parent_exe = Some("/sbin/launchd".into());
        }
    }
    snap
}

#[test]
fn leader_exit_keeps_id_and_launch_command() {
    let first = fixture("claude-bash-tool.json");
    let s1 = server(&first, 31000);
    assert_eq!(s1.id, format!("31000-{T}"));

    let second = run(&leader_exited(), &first.servers, &[]);
    let s2 = server(&second, 31000);
    assert_eq!(s2.id, s1.id);
    assert_eq!(s2.launch_argv, s1.launch_argv);
    assert_eq!(s2.started_at_ms, T);
    assert_eq!(s2.members.len(), 4);

    // Without `prev` the same group would get a different id.
    let fresh = run(&leader_exited(), &[], &[]);
    assert_eq!(server(&fresh, 31000).id, format!("31000-{}", T + 150));
}

#[test]
fn daemon_restart_rematches_via_witness() {
    let first = fixture("claude-bash-tool.json");
    let s1 = server(&first, 31000);
    let known = KnownGroup {
        id: s1.id.clone(),
        pgid: 31000,
        witnesses: s1.members.clone(),
    };
    let after_restart = run(&leader_exited(), &[], &[known]);
    assert_eq!(server(&after_restart, 31000).id, s1.id);
}

#[test]
fn pgid_reuse_without_a_shared_witness_gets_a_new_id() {
    let first = fixture("claude-bash-tool.json");
    let s1 = server(&first, 31000);
    let known = KnownGroup {
        id: s1.id.clone(),
        pgid: 31000,
        witnesses: s1.members.clone(),
    };
    let mut reused = load("claude-bash-tool.json");
    for p in reused.processes.iter_mut().filter(|p| p.pgid == 31000) {
        p.start_epoch_ms += 3_600_000;
    }
    let new_id = format!("31000-{}", T + 3_600_000);
    assert_eq!(server(&run(&reused, &first.servers, &[]), 31000).id, new_id);
    assert_eq!(server(&run(&reused, &[], &[known]), 31000).id, new_id);
}

#[test]
fn unreadable_tty_marks_incomplete_and_keeps_prev_origin() {
    let first = fixture("claude-bash-tool.json");
    let mut snap = load("claude-bash-tool.json");
    for p in snap.processes.iter_mut().filter(|p| p.pid == 31011) {
        p.has_tty = None;
        p.errors.push("tty: EPERM".into());
    }
    let kept = run(&snap, &first.servers, &[]);
    let s = server(&kept, 31000);
    assert_eq!(s.incomplete, ["pid 31011: tty unreadable"]);
    assert_eq!(s.origin, server(&first, 31000).origin);

    // Seen for the first time while incomplete: unknown is never treated as agent.
    let fresh = run(&snap, &[], &[]);
    assert!(!server(&fresh, 31000).complete());
    assert_eq!(server(&fresh, 31000).origin, user());
}

#[test]
fn lost_listener_keeps_server_with_no_ports() {
    let first = fixture("claude-bash-tool.json");
    let mut snap = load("claude-bash-tool.json");
    snap.listeners.retain(|l| l.pid != 31011);
    let d = run(&snap, &first.servers, &[]);
    let s = server(&d, 31000);
    assert!(s.ports.is_empty());
    assert_eq!(s.primary_url, None);
    assert_eq!(s.id, server(&first, 31000).id);
    // Never seen with a listener: not a server at all.
    assert!(run(&snap, &[], &[]).servers.is_empty());
}

#[test]
fn empty_group_is_absent_and_unreadable_group_is_kept_incomplete() {
    let first = fixture("claude-bash-tool.json");
    let mut snap = load("claude-bash-tool.json");
    snap.processes.retain(|p| p.pgid != 31000);
    snap.listeners.retain(|l| l.pid != 31011);
    assert!(run(&snap, &first.servers, &[]).servers.is_empty());

    snap.unreadable_groups.insert(31000);
    let d = run(&snap, &first.servers, &[]);
    let s = server(&d, 31000);
    assert_eq!(s.incomplete, ["member list unreadable"]);
    assert_eq!(s.origin, server(&first, 31000).origin);
}

#[test]
fn other_uid_member_makes_a_group_unmanaged() {
    let mut snap = load("claude-bash-tool.json");
    for p in snap.processes.iter_mut().filter(|p| p.pid == 31020) {
        p.uid = 0;
    }
    let d = run(&snap, &[], &[]);
    assert!(d.servers.is_empty());
    assert_eq!(d.unmanaged[0].pgid, 31000);
}

#[test]
fn is_protected_on_a_fresh_member_list() {
    let ctx = ctx_with(&[]);
    let snap = load("t3-code.json");
    let tunnel: Vec<_> = snap
        .processes
        .iter()
        .filter(|p| p.pgid == 5460)
        .cloned()
        .collect();
    assert!(is_protected(&tunnel, &ctx));
    let snap = load("claude-bash-tool.json");
    let dev: Vec<_> = snap
        .processes
        .iter()
        .filter(|p| p.pgid == 31000)
        .cloned()
        .collect();
    assert!(!is_protected(&dev, &ctx));
    // Unreadable fields match nothing: an exiting member never blocks the stop.
    let exiting: Vec<_> = dev
        .into_iter()
        .map(|p| ProcessInfo {
            exe: None,
            argv: None,
            parent_exe: None,
            ..p
        })
        .collect();
    assert!(!is_protected(&exiting, &ctx));
}

#[test]
fn every_protected_pattern_has_a_fixture() {
    let ctx = ctx_with(&[]);
    let names = [
        "author-machine.json",
        "claude-bash-tool.json",
        "agent-runtimes.json",
        "t3-code.json",
        "browsers.json",
        "codex-t3.json",
        "codex-terminal.json",
    ];
    let mut hits: Vec<(Protect, String)> = Vec::new();
    for name in names {
        let snap = load(name);
        let d = discover(&snap, &ctx, &[], &[]);
        for p in snap
            .processes
            .iter()
            .filter(|p| d.protected.contains(&p.pgid))
        {
            for rule in PROTECTED.iter().copied().filter(|r| matches(*r, p, &ctx)) {
                hits.push((rule, p.exe.clone().unwrap_or_default()));
            }
        }
    }
    for rule in PROTECTED {
        assert!(
            hits.iter().any(|(r, _)| r == rule),
            "no fixture for {rule:?}"
        );
    }
    for bundle in BROWSER_BUNDLES {
        let in_bundle = |exe: &String| exe.split('/').any(|c| c == *bundle);
        assert!(
            hits.iter()
                .any(|(r, exe)| *r == Protect::Browser && in_bundle(exe)),
            "no fixture for {bundle}"
        );
    }
}

#[test]
fn marker_keys_for_scrubbing() {
    for key in [
        "CLAUDECODE",
        "CODEX_THREAD_ID",
        "GEMINI_CLI",
        "CLAUDE_CODE_SESSION_ID",
        "LULL_KEEP",
    ] {
        assert!(is_marker_key(key), "{key}");
    }
    for key in [
        "T3CODE_CLI_PATH",
        "AI_AGENT",
        "CURSOR_TRACE_ID",
        "PATH",
        "PORT",
    ] {
        assert!(!is_marker_key(key), "{key}");
    }
}

#[test]
fn shell_quoting() {
    assert_eq!(
        labels::quote(&strings(&["npm", "run", "dev"])),
        "npm run dev"
    );
    assert_eq!(
        labels::quote(&strings(&["sh", "-c", "it's a $HOME", ""])),
        r"sh -c 'it'\''s a $HOME' ''"
    );
}

#[test]
fn git_root_walks_up_to_dot_git() {
    let here = env!("CARGO_MANIFEST_DIR");
    let root = find_git_root(here).expect("the repository has a .git");
    assert!(here.starts_with(&root));
}
