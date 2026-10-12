# Agent instructions

## Ponytail debt

Deliberate shortcuts are marked with a `ponytail: <ceiling>, <upgrade trigger>` comment and
tracked in [`PONYTAIL-DEBT.md`](PONYTAIL-DEBT.md). Whenever the ponytail skill is active, or you
add, change, move or remove a `ponytail:` marker, update the ledger in the same commit:

- one row per marker, in the format the ledger uses, with the current `file:line`;
- every marker names an upgrade trigger; one without a trigger is a defect, not debt;
- when a trigger fires and the shortcut is upgraded, delete the marker and its row;
- re-check with `grep -rnE '(#|//) ?ponytail:' . --exclude-dir={node_modules,.git,target,dist,.build}`
  and keep the count line at the end of the ledger accurate.
