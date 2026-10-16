# Third-party logos

The panel draws these marks to name what a server is built with (`tech-*`) and which
coding agent started it (`agent-*`). They are unmodified copies; the app tints the
single-colour `tech-*` marks with each brand's colour and draws the `agent-*` marks as
they are. `scripts/build-app.sh` copies them into the app bundle.

| Files | Source | Licence of the artwork |
|---|---|---|
| `tech-*.svg` | [Simple Icons](https://simpleicons.org) 16.35.0 (`vite`, `nextdotjs`, `astro`, `nuxt`, `remix`, `cloudflareworkers`, `laravel`, `rubyonrails`, `django`, `modelcontextprotocol`, `nodedotjs`, `bun`, `deno`, `python`, `go`, `elixir`) | CC0-1.0; `deno` MIT |
| `agent-*.svg` | [LobeHub Icons](https://github.com/lobehub/lobe-icons) 1.95.1 (`claudecode-color`, `codex-color`, `geminicli-color`, `cursor`, `copilot-color`, `opencode`, `antigravity-color`) | MIT, © 2023 LobeHub |

PHP, Ruby and Rust are left out: Simple Icons lists their marks under CC BY-SA, which
does not fit the MIT/Apache-2.0 licence. The app shows their initials instead.

Every mark is a trademark of its owner. lull uses them only to say which technology or
agent a server belongs to, which implies no endorsement; the licences above cover the
drawings, not the trademarks.
