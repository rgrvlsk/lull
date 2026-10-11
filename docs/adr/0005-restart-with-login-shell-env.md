# Manual restart uses the login-shell environment, not the original one

v1 lets the user restart a recently stopped server from its stopped-server record. The
daemon re-runs the recorded argv in the recorded cwd through `$SHELL -l -c`, as if the user
typed it in a terminal, keeping only `PORT`, `HOST` and `NODE_ENV` from the original
environment. Reusing the original environment would persist secrets (API keys, agent
messaging tokens) on disk and carry the agent markers over, so the restarted server would
be re-classified as agent-started and stopped again. Dev servers normally read their own
`.env`, so the login environment is enough in practice; variables passed inline
(`FOO=1 npm run dev`) are lost.

## Considered Options

- **Env diff in `state.json`.** Faithful, but leaves secrets in a persistent plain-text file.
- **Env diff in the Keychain.** Faithful and safer, but adds Keychain access from a
  background agent for a convenience feature.

Chosen compromise: an opt-in `faithful_restart` stores the env (minus agent markers) in a
`0600` file under the per-user `$TMPDIR`, which macOS empties at boot, so secrets never
outlive a reboot.
