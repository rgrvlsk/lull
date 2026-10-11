# Agent origin is decided by agent env markers, vetoed by a terminal or an installed app

A server is **agent-started** when any member of its process group carries a strong agent
marker in its initial environment (`CLAUDECODE=1`, `CODEX_THREAD_ID`, or an entry from the
built-in marker table derived from `vercel/detect-agent`, extensible in config), **unless**
any member has a controlling terminal or its executable lives under `/Applications`,
`~/Applications` or `/System`. Otherwise it is user-started. Only agent-started servers are
auto-stopped by default, so a false positive kills something the user cares about; the
rule is biased towards `user`.

## Considered Options

- **`T3CODE_CLI_PATH` as a marker** (what the prototype did). Rejected: T3 Code sets it for
  its whole backend, so it also reaches editors opened from T3 (a running Zed had it plus a
  TCP listener and was a kill candidate) and project scripts the user starts from the T3 UI.
  Agents that T3 runs carry their own marker, so nothing is lost.
- **Markers alone.** Rejected: Claude Code also sets `CLAUDECODE=1` in IDE integrated
  terminals, and markers are inherited by anything launched from an agent shell. Agent
  tool shells have no controlling terminal (verified for Claude Code's Bash tool); a
  human's terminal does.
- **Veto any `*.app/Contents/` executable.** Rejected: Homebrew Python runs from
  `Python.framework/…/Python.app/Contents/MacOS/Python`, so `python -m http.server` would
  never be classified. The veto is limited to installed-app locations.
- **Generic `AI_AGENT`.** Not used as a sole marker: only some agents set it.
