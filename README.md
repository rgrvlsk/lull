# lull

Puts forgotten dev servers to sleep.

AI coding agents start `vite`, `next dev`, `astro dev`, `php -S`… and leave them running for days.
lull finds them, keeps their tabs in one "Dev" tab group, notices when you stop looking,
and stops idle servers after 30 minutes.

- Rust daemon + CLI (`lull ls`, `lull open <url>`)
- Browser extension for Chromium (Helium, Chrome, Brave, Edge) and Firefox/Zen
- macOS menu-bar app

**Status:** design phase. Start with [`docs/contract.md`](docs/contract.md).
