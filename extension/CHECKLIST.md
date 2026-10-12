# Manual Helium verification

Status: prepared, not executed by WP2.2. Use a disposable local project and browser
profile. Record Helium, daemon and extension versions, extension ID, date, server IDs
and results beside each step. Browser/runtime integration depends on WP2.1–WP2.4.

## Prepare and load

1. In this worktree's `extension/`, run `command npm ci`, `command npm run typecheck`,
   `command npm test`, and `command npm run build`.
2. In Helium open `chrome://extensions`, enable **Developer mode**, click **Load
   unpacked**, and choose this worktree's `extension/dist/chromium` directory. Confirm
   that the extension is named **lull**, has no load errors, and has a service worker.
3. Install the native host using `lull browsers install` from WP2.3. Its
   `allowed_origins` must include the ID shown for this unpacked build. WP2.2 has no
   manifest `key`; the release WP pins the ID. If host installation cannot target
   this development ID, arrange a development host manifest before continuing.
4. Start the daemon using the project's development LaunchAgent procedure. Pin the
   extension action in Helium. Its `off` badge must clear within 30 seconds. Open the
   service worker inspector and confirm no extension errors.
5. In a disposable project directory under `$HOME` or `/tmp`, start
   `python3 -m http.server 5187 --bind 127.0.0.1`. Run `lull ls` and record its server
   ID. This human-started server will not auto-stop; use explicit stop below.

## Group formation and drag-out

1. Open `http://127.0.0.1:5187/` and a second tab at the same URL. Within the debounce
   interval plus IPC time, both must enter one grey **Dev** group in that window.
2. In a second window, create a group named **Dev** and set its colour to blue.
   Open the same server URL there. The tab must join that existing group, which
   retains its colour. Leave an unrelated `https://example.com/` tab ungrouped.
3. Drag a server tab outside **Dev**. Activate other tabs, reload the dragged tab,
   and switch windows. It must remain outside **Dev** during this worker's lifetime.
4. Quit the daemon, confirm `off`, then start it again. After at most 30 seconds,
   the badge must clear, the tabs must resync, and the dragged tab must stay outside
   **Dev** if the service worker was not restarted. Reloading the extension resets
   the in-memory opt-out.

## Visible but unfocused on a second monitor

1. Put one Helium window on each monitor. Select a server tab in the second-monitor
   window and leave its document unobscured. Give focus to the first window.
2. Keep using the Mac. After at least 35 seconds run `lull ls`. The server's activity
   must report `tab`, and its last-activity time must advance despite the other
   window having focus. Keep the CLI window from covering the observed document.
3. Select an unrelated tab in the second-monitor window. Ensure no server tab is
   visible anywhere. After 35 seconds its activity time must no longer advance
   from `tab`. New HTTP traffic can independently produce `traffic` activity.
4. Restore the visible server tab, then provide no keyboard/mouse input for more
   than 95 seconds. Observe daemon logs/activity from another process without waking
   the Mac. Once `idle.queryState(60)` becomes idle, tab heartbeats must stop.
   Resume input; the next visible heartbeat within 30 seconds must count again.

## Discard on stop and reload after restart

1. Confirm daemon `tabs_on_stop` is `discard`. Keep a server tab in **Dev**, then
   select an unrelated tab in the same window so the server tab is inactive.
2. Run `lull stop <recorded-server-id>`. Check `chrome://discards` for the server
   tab: it must be discarded, retain its URL and group, and free its document.
   An active server tab cannot be discarded and must return an error ack without
   an extension crash. Test that separately with another disposable server.
3. Run `lull restart <recorded-server-id>`. When the restarted server has a listener,
   its retained tab must reload and show the server again. Confirm in the daemon's
   browser-command logs that `tab.reload` received an `{ok:true}` ack. Do not count
   a manual reload as proof of the daemon's reload command.
4. Navigate a retained server tab to an unrelated origin before a queued stop/reload
   command is delivered. Its close/discard/reload/focus command must return an error
   ack and leave the unrelated page unchanged. Use the daemon/mock command driver
   to control delivery; the production daemon must not target unrelated tabs.
