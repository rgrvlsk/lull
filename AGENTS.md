# Agent instructions

lull is public, open-source software (`MIT OR Apache-2.0`) that people install through
Homebrew and run on their own Macs, where it stops processes. Write every commit, comment,
fixture, issue and pull request for a stranger who will read it and rely on it.

## Sources of truth

- `docs/contract.md`: behaviour. Read the sections your task touches before writing code.
  When code and contract disagree, stop and report; the contract changes only in its own
  reviewed pull request.
- `docs/plan.md`: work packages, the paths each one owns, toolchains, definition of done
  and commit style.
- `docs/adr/`: why past decisions were made; a new decision gets a new ADR.
- `GLOSSARY.md`: use its terms in code, docs and UI text; add new ones there.

## Workflow

- One work package or fix per branch (`wp1.2-discovery`, `fix/stop-race`), opened as a
  pull request against `main`. The maintainer (@rgrvlsk) reviews and merges.
- Keep a pull request inside the paths its work package owns. Its description says what
  changed, which contract sections it implements, and how you verified it (the commands
  you ran and their result).
- Everything in the repository is written in English, whatever language the conversation
  is in.

## Public by default

- Fixtures, tests, logs and examples use made-up data: `/Users/x`, `~/dev/example-app`,
  `host.local`, `example.com`. Anything recorded from a real machine goes through `lull
  debug snapshot`'s scrubbing and then a line-by-line read for what scrubbing misses:
  project and client directory names, hostnames, usernames, email addresses.
- Commits use the maintainer's GitHub noreply address, which this clone's git config
  already sets; leave `user.email` as it is.
- Secrets (signing keys, notary credentials, tokens) live only in GitHub Actions secrets.
  Refer to them by name; their values stay out of files, command output, logs and pull
  request text.
- A new dependency needs a licence compatible with MIT/Apache-2.0, active maintenance, and
  a line in the pull request saying why the standard library is not enough.
- lull makes no network requests (contract §14). Telemetry, update checks, crash reports
  or any other outbound request need an ADR first.

## Ask the maintainer first

These reach users or change shared state, so propose them and wait for a yes:

- pushing to `main`, force-pushing, or rewriting history that is already on GitHub
- version bumps, tags, releases, and any change to the `rgrvlsk/homebrew-tap` repository
- creating or deleting repositories, or changing repository settings or secrets
- publishing the extension (AMO signing, `updates.json`)
- merging pull requests; replying to, closing or labelling other people's issues and PRs
- editing `docs/contract.md` or an accepted ADR

## Ponytail debt

Deliberate shortcuts are marked with a `ponytail: <ceiling>, <upgrade trigger>` comment and
tracked in [`PONYTAIL-DEBT.md`](PONYTAIL-DEBT.md). Whenever the ponytail skill is active, or you
add, change, move or remove a `ponytail:` marker, update the ledger in the same commit:

- one row per marker, in the format the ledger uses, with the current `file:line`;
- every marker names an upgrade trigger; one without a trigger is a defect, not debt;
- when a trigger fires and the shortcut is upgraded, delete the marker and its row;
- re-check with `grep -rnE '(#|//) ?ponytail:' . --exclude-dir={node_modules,.git,target,dist,.build}`
  and keep the count line at the end of the ledger accurate.
