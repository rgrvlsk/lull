# A live terminal UI in the same binary, built on ratatui

`lull ls` is a snapshot: to watch a countdown or see a server appear you run it again.
The menu-bar app (M4) is the planned live view, but it waits on Developer ID signing
and the cask (M5). Most lull users already live in a terminal, so `lull ui` (and bare
`lull` when stdin and stdout are terminals) opens a full-screen terminal UI that stays
running: servers grouped by project with a detail pane, history with restart and
forget, and the config, with the actions the CLI has.

It is a pure client of the socket, like the app, and needs no protocol change:
`subscribe` streams server changes; `servers.list` is re-read every few seconds
because CPU and RSS changes alone are not notified; history and config are fetched
when shown or after a `server.removed`. It renders with `ratatui` on `crossterm`
(both MIT): a double-buffered diff renderer, resize handling, wide-character widths
and alternate-screen restore are exactly where a hand-rolled ANSI UI shows visual
glitches, and ratatui is the maintained standard for this in Rust.

## Considered Options

- **Hand-written ANSI, no dependency.** Smallest binary, but we would own diffing,
  resize, Unicode width and terminal restore, the parts that are hard to get right.
- **Wait for the menu-bar app.** Leaves v1 without a live view, and macOS-GUI only.
- **A `--watch` flag on `lull ls`.** Live, but no selection, so no actions.

## Consequences

- Two dependencies and roughly 0.5–1 MB of binary; the daemon and nm-host do not use them.
- Every string that comes from a process (argv, cwd, project, URL) is untrusted: an
  agent controls it. The UI replaces control characters before drawing, so a crafted
  command line cannot write escape sequences to the user's terminal.
- Colours come from the terminal's 16-colour ANSI palette (light and dark themes both
  work) and `NO_COLOR` is honoured. Keyboard only: the mouse is not captured, so text
  such as a URL stays selectable.
- Stopping a user-started server asks for confirmation; agent-started ones stop at once,
  since lull would stop them anyway. Restart shows the launch command and asks first,
  because it runs a command lull recorded rather than one the user typed.
- The UI owns no policy; config edits go through `config.set` and its validation. If the
  daemon goes away the UI says so and reconnects.
- The menu-bar app (M4) keeps its scope; the terminal UI ships first.
