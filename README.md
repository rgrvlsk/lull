# lull

Stops the dev servers you forgot.

AI coding agents start `vite`, `next dev`, `astro dev`, `php -S`… and leave them running for
days. lull finds them, keeps their tabs in one "Dev" tab group, notices when nobody is using
them, and stops idle agent-started servers after 30 minutes. Servers you started yourself are
listed but never stopped unless you opt in.

- a daemon + CLI (`lull ls`, `lull open <url>`, `lull restart <id>`), one Rust binary
- a Chromium extension (Helium, Chrome, Brave, Edge, Vivaldi) that reports which dev-server
  tabs are on screen
- later (v1.1): Firefox/Zen and a menu-bar app

macOS 14 or later. lull makes no network requests.

## Install

```sh
brew install rgrvlsk/tap/lull
brew services start lull      # the daemon, as a LaunchAgent
lull browsers install         # native-messaging manifests + the unpacked extension
```

`lull browsers install` copies the extension to
`~/Library/Application Support/lull/extension/chromium`. In each browser, open the extensions
page, enable **Developer mode**, choose **Load unpacked** and select that directory. After a
`brew upgrade`, restart the browser to pick up a new extension version.

Check everything with `lull doctor`.

## Use

```sh
lull ls                 # dev servers: project, URL, origin (agent/you), when they stop
lull ls --all           # plus other listeners lull leaves alone
lull open <url|id>      # open in the Dev group (in the background when an agent runs it)
lull stop <id>          # ids accept a unique prefix, e.g. the pgid
lull pin <id>           # never stop it automatically; `lull unpin <id>` undoes it
lull touch <id>         # count as activity now
lull history            # recently stopped servers
lull restart <id>       # start a stopped server again, as if you typed its command
lull config set idle_minutes=60
lull doctor
```

A server counts as in use while one of its tabs is visible on screen (with you at the
computer), while something outside it talks to it, and for its first `idle_minutes`. An agent
can keep a server by starting it with `LULL_KEEP=1`. Optional line for agent instruction files:

> Open dev-server previews with `lull open <url>`; idle servers are stopped after 30 min —
> restart them if needed.

Configuration lives in `~/Library/Application Support/lull/config.toml` (see
[`docs/contract.md`](docs/contract.md) §6); the log is `~/Library/Logs/lull.log`.

## Uninstall

```sh
lull browsers uninstall       # then remove the extension in each browser
brew services stop lull
brew uninstall lull
rm -rf ~/Library/Application\ Support/lull ~/Library/Logs/lull ~/Library/Logs/lull.log
```

## Develop

```sh
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
(cd extension && npm ci && npm run typecheck && npm test && npm run build)
(cd macos && swift test)
```

`lull daemon --dry-run` makes every decision without sending signals. The design is in
[`docs/contract.md`](docs/contract.md), decisions in [`docs/adr/`](docs/adr/), vocabulary in
[`GLOSSARY.md`](GLOSSARY.md), the work breakdown in [`docs/plan.md`](docs/plan.md), and the
release steps in [`packaging/RELEASE.md`](packaging/RELEASE.md).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.
