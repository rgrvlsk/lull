#!/bin/bash
# devserver-reaper — stops dev servers started by coding agents (Claude Code, Codex via T3 Code)
# after IDLE_MIN minutes without activity. Run every minute by launchd
# (~/Library/LaunchAgents/local.devserver-reaper.plist).
#
# A "server" is a process group that has a TCP listener and whose listener process
# inherited an agent env marker (CLAUDECODE=1 or T3CODE_CLI_PATH). Services started by
# brew/launchd/you in a terminal have no marker and are never touched. Groups that contain
# claude/codex/T3 itself (MCP servers, T3's cloudflared tunnel) are skipped.
# Opt-out for a single server: start it with DEVSERVER_KEEP=1 in its environment.
#
# Env overrides: IDLE_MIN (30), CPU_ACTIVE_SECS (1), DRY_RUN (0).

set -u
IDLE_MIN=${IDLE_MIN:-30}
CPU_ACTIVE_SECS=${CPU_ACTIVE_SECS:-1}
DRY_RUN=${DRY_RUN:-0}
STATE="$HOME/.local/state/devserver-reaper"
LOG="$HOME/Library/Logs/devserver-reaper.log"
LSOF=/usr/sbin/lsof
PS=/bin/ps

mkdir -p "$STATE"
now=$(date +%s)
log() { echo "$(date '+%F %T') $*" >> "$LOG"; }

# Activity policy for one sample (taken every ~60 s).
#   $1 cpu_delta  — CPU seconds the whole group used since the previous sample
#   $2 external   — 1 if some process outside the group (browser, curl, agent) holds an
#                   ESTABLISHED connection to one of the group's listening ports, else 0
# Return 0 (true) when the group counts as active right now.
is_active() {
  local cpu_delta=$1 external=$2
  [ "$external" = 1 ] && return 0
  [ "$cpu_delta" -ge "$CPU_ACTIVE_SECS" ] && return 0
  return 1
}

tmp=$(mktemp -d) || exit 1
trap 'rm -rf "$tmp"' EXIT
$PS -A -o pid=,pgid=,time=,comm= > "$tmp/procs"
$LSOF -nP -iTCP -sTCP:LISTEN -Fpn > "$tmp/listen" 2>/dev/null
$LSOF -nP -iTCP -sTCP:ESTABLISHED -Fpn > "$tmp/estab" 2>/dev/null

# Process groups that must never be signalled: anything sharing a group with the agents or T3.
protected=" $(awk '$4 ~ /(^|\/)(claude|codex)$|T3 Code/ {print $2}' "$tmp/procs" | sort -u | tr '\n' ' ') 1 $($PS -o pgid= -p $$ | tr -d ' ') "

candidates=" "
for pid in $(sed -n 's/^p//p' "$tmp/listen" | sort -u); do
  envline=$($PS eww -o command= -p "$pid" 2>/dev/null) || continue
  case "$envline" in *" CLAUDECODE=1"*|*" T3CODE_CLI_PATH="*) ;; *) continue ;; esac
  case "$envline" in *" DEVSERVER_KEEP=1"*) continue ;; esac
  pgid=$(awk -v p="$pid" '$1==p {print $2}' "$tmp/procs")
  [ -z "$pgid" ] && continue
  case "$protected" in *" $pgid "*) continue ;; esac
  case "$candidates" in *" $pgid "*) ;; *) candidates="$candidates$pgid " ;; esac
done

for pgid in $candidates; do
  # -> "<cpu_seconds> <external 0|1> <members,> <ports,>"
  read -r cpu external members ports < <(awk -v G="$pgid" '
    function tosec(t,   x, p, n, s, d, k) {
      d = 0; if (index(t, "-")) { split(t, x, "-"); d = x[1]; t = x[2] }
      n = split(t, p, ":"); s = 0; for (k = 1; k <= n; k++) s = s * 60 + p[k]
      return int(s + d * 86400)
    }
    FILENAME == ARGV[1] { if ($2 == G) { member[$1] = 1; cpu += tosec($3); mlist = mlist $1 "," } next }
    FILENAME == ARGV[2] {
      if (/^p/) pid = substr($0, 2)
      else if (/^n/ && (pid in member)) { pt = $0; sub(/.*:/, "", pt); if (!(pt in lport)) plist = plist pt ","; lport[pt] = 1 }
      next
    }
    FILENAME == ARGV[3] {
      if (/^p/) pid = substr($0, 2)
      else if (/^n/) {
        split(substr($0, 2), a, "->"); l = a[1]; sub(/.*:/, "", l)
        r = a[2]; rp = r; sub(/.*:/, "", rp); rh = r; sub(/:[0-9]+$/, "", rh)
        n++; L[n] = l; RP[n] = rp; RH[n] = rh; P[n] = pid; owner[l] = pid
      }
      next
    }
    END {
      ext = 0
      for (i = 1; i <= n; i++) {
        if (!(P[i] in member) || !(L[i] in lport)) continue
        if (RH[i] !~ /^(127\.|\[::1\]|localhost)/) ext = 1          # remote client
        else if (!(RP[i] in owner) || !(owner[RP[i]] in member)) ext = 1  # local client outside the group
      }
      printf "%d %d %s %s\n", cpu, ext, mlist, plist
    }' "$tmp/procs" "$tmp/listen" "$tmp/estab")

  sf="$STATE/$pgid"
  if [ -f "$sf" ]; then read -r s_members s_cpu s_active < "$sf"; else s_members=; fi
  if [ "$s_members" != "$members" ]; then
    echo "$members $cpu $now" > "$sf"   # new (or reshaped) group: start the idle clock now
    continue
  fi

  if is_active $((cpu - s_cpu)) "$external"; then s_active=$now; fi
  echo "$members $cpu $s_active" > "$sf"

  idle=$(( (now - s_active) / 60 ))
  [ "$idle" -lt "$IDLE_MIN" ] && continue

  leader=${members%%,*}
  what=$($PS -o command= -p "$leader" 2>/dev/null | cut -c1-120)
  cwd=$($LSOF -a -p "$leader" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')
  if [ "$DRY_RUN" = 1 ]; then
    log "DRY-RUN would stop pgid=$pgid ports=${ports%,} idle=${idle}m cwd=$cwd cmd=$what"
    continue
  fi
  log "stop pgid=$pgid ports=${ports%,} idle=${idle}m cwd=$cwd cmd=$what"
  kill -TERM -- "-$pgid" 2>/dev/null
  sleep 5
  kill -0 -- "-$pgid" 2>/dev/null && kill -KILL -- "-$pgid" 2>/dev/null
  rm -f "$sf"
done

# Forget groups that no longer exist.
for sf in "$STATE"/*; do
  [ -e "$sf" ] || continue
  case "$candidates" in *" $(basename "$sf") "*) ;; *) rm -f "$sf" ;; esac
done
