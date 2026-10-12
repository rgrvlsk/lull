# Security

lull runs as your user and sends signals to process groups it classifies as idle dev
servers, so a bug that makes it stop the wrong process is a security issue.

## Reporting a vulnerability

Report it privately through GitHub: **Security → Report a vulnerability** on
<https://github.com/rgrvlsk/lull/security/advisories/new>. Please do not open a public
issue. Include the lull version (`lull --version`), your macOS version, and the steps or
`lull debug snapshot` output that reproduce it (read it first and remove anything
personal).

You can expect a first reply within a week. Fixes ship as a new release in the Homebrew
tap; the advisory is published once users can upgrade.

## Supported versions

Only the latest release receives fixes.
