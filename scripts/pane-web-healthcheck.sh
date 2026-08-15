#!/usr/bin/env bash
#
# Daily liveness check for the public docs site, with self-repair.
#
# Background. The site went down for three weeks and nobody noticed. Two
# independent faults had to line up, and fixing either one alone would have
# left it at 502:
#
#   1. The `pane-web` container was stopped explicitly. `restart:
#      unless-stopped` honours an explicit stop forever, so it never came
#      back.
#   2. The reverse SSH tunnel was half-open: the local `ssh` process was
#      alive with the right `-R` in its argv, yet the VPS had no listener
#      on the port at all. `autossh -M 0` has no monitoring channel of its
#      own and only notices a process that exits — which never happened.
#
# One `launchctl kickstart -k` is not always enough to clear the half-open
# state (seen 2026-08-15: a flapping network kept re-landing the fresh
# session in the same half-bound state). The tunnel repair below retries a
# few times, rechecking the public URL between attempts, before giving up.
#
# So a check that only pings the public URL would tell us "down" without
# saying which link broke, and a check that only looks at local processes
# would have reported everything healthy through the whole outage. This
# script walks the chain end to end and repairs what it finds:
#
#   public URL -> (Caddy on the VPS) -> reverse tunnel -> local port -> container
#
# Exit codes: 0 site is up (possibly after repair), 1 still down.
#
# Everything is overridable by env so the script carries no host specifics;
# the launchd plist supplies them. Absolute paths matter here: launchd hands
# an agent a nearly empty PATH.

set -uo pipefail

# launchd hands an agent a PATH of roughly `/usr/bin:/bin:/usr/sbin:/sbin`.
# Binaries this script calls directly are resolved absolutely below, but the
# notifier cannot be: `openclaw` is a wrapper that runs node via `env node`,
# so without Homebrew's bin on PATH it dies with "env: node: No such file or
# directory" — and the alert channel is silently dead exactly when it is
# needed. Caught only by running the repair path through launchd; by hand
# from a terminal it worked every time.
export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"

#
# Deployment note, learned the hard way: this script must be installed
# OUTSIDE ~/Documents (see `make healthcheck-install`, which copies it to
# ~/.local/bin). macOS TCC denies a launchd agent any access to ~/Documents,
# ~/Desktop and ~/Downloads, so running it from the repo checkout fails with
# "Operation not permitted" (exit 126) — and only under launchd. Run by hand
# from a terminal it works fine, because the terminal has been granted
# access, which is exactly how such a check ends up silently dead.
#
# For the same reason the container is repaired with `docker start <name>`
# rather than `docker compose -f <file>`: the compose file lives in the repo,
# i.e. under ~/Documents, and reading it would hit the same wall. Compose is
# used only if PANE_HC_COMPOSE is set and readable — worth having for the
# rarer case where the container no longer exists at all.

URL=${PANE_HC_URL:-https://pane.thothlab.tech/docs/}
LOCAL_URL=${PANE_HC_LOCAL_URL:-http://127.0.0.1:8744/docs/}
CONTAINER=${PANE_HC_CONTAINER:-pane-web}
COMPOSE_FILE=${PANE_HC_COMPOSE:-}
TUNNEL_LABEL=${PANE_HC_TUNNEL_LABEL:-com.pane.tunnel}
TUNNEL_RETRIES=${PANE_HC_TUNNEL_RETRIES:-5}
TG_TARGET=${PANE_HC_TG_TARGET:-}
OPENCLAW=${PANE_HC_OPENCLAW:-$HOME/.local/bin/openclaw}
LOG=${PANE_HC_LOG:-$HOME/Library/Logs/pane-web-healthcheck.log}

# How long to wait for a repaired link to actually come up before re-testing.
SETTLE_SECONDS=${PANE_HC_SETTLE:-10}

log() { printf '%s %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$*" >>"$LOG"; }

# Resolve a binary that launchd will not have on PATH.
find_bin() {
  local name=$1 p
  for p in "/opt/homebrew/bin/$name" "/usr/local/bin/$name" "/usr/bin/$name"; do
    [ -x "$p" ] && { printf '%s' "$p"; return 0; }
  done
  command -v "$name" 2>/dev/null
}

DOCKER=${PANE_HC_DOCKER:-}
if [ -z "$DOCKER" ]; then
  for p in /Applications/Docker.app/Contents/Resources/bin/docker \
           /opt/homebrew/bin/docker /usr/local/bin/docker; do
    [ -x "$p" ] && { DOCKER=$p; break; }
  done
fi
CURL=$(find_bin curl)

# HTTP status, or 000 when the request could not be made at all. Never fails
# the script — an unreachable host is a result, not an error.
http_code() {
  # curl already prints `000` on a connection failure *and* exits non-zero,
  # so a `|| printf 000` fallback would concatenate into `000000`. Substitute
  # only when curl printed nothing at all.
  local out
  out=$("$CURL" -sS -o /dev/null -w '%{http_code}' --max-time 20 "$1" 2>/dev/null)
  [ -n "$out" ] && printf '%s' "$out" || printf '000'
}

notify() {
  [ -n "$TG_TARGET" ] || { log "notify skipped: PANE_HC_TG_TARGET unset"; return; }
  [ -x "$OPENCLAW" ] || { log "notify skipped: $OPENCLAW not executable"; return; }
  "$OPENCLAW" message send --channel telegram --target "$TG_TARGET" -m "$1" \
    >>"$LOG" 2>&1 || log "notify failed"
}

actions=()

code=$(http_code "$URL")
if [ "$code" = "200" ]; then
  log "ok $URL -> 200"
  exit 0
fi
log "DOWN $URL -> $code, walking the chain"

# Link 1: the container behind the local port. If this is down, the tunnel
# has nothing to forward to and repairing it alone would change nothing.
local_code=$(http_code "$LOCAL_URL")
if [ "$local_code" != "200" ]; then
  log "local upstream $LOCAL_URL -> $local_code, starting container"
  if [ -z "$DOCKER" ]; then
    actions+=("docker не найден")
    log "docker binary not found — cannot repair"
  else
    if [ -n "$COMPOSE_FILE" ] && [ -r "$COMPOSE_FILE" ]; then
      "$DOCKER" compose -f "$COMPOSE_FILE" up -d >>"$LOG" 2>&1 \
        && actions+=("контейнер поднят (compose)") \
        || { actions+=("НЕ УДАЛОСЬ поднять контейнер"); log "docker compose up failed"; }
    else
      "$DOCKER" start "$CONTAINER" >>"$LOG" 2>&1 \
        && actions+=("контейнер запущен") \
        || { actions+=("НЕ УДАЛОСЬ запустить контейнер"); log "docker start $CONTAINER failed"; }
    fi
    sleep "$SETTLE_SECONDS"
    local_code=$(http_code "$LOCAL_URL")
    log "local upstream after repair -> $local_code"
  fi
fi

# Link 2: the tunnel. Only worth touching once the upstream answers locally,
# otherwise we would be restarting a tunnel that correctly forwards to a dead
# port and blaming the wrong thing.
if [ "$local_code" = "200" ]; then
  code=$(http_code "$URL")
  attempt=0
  while [ "$code" != "200" ] && [ "$attempt" -lt "$TUNNEL_RETRIES" ]; do
    attempt=$((attempt + 1))
    log "local ok but public -> $code, restarting tunnel $TUNNEL_LABEL (попытка $attempt/$TUNNEL_RETRIES)"
    if launchctl kickstart -k "gui/$(id -u)/$TUNNEL_LABEL" >>"$LOG" 2>&1; then
      actions+=("туннель перезапущен (попытка $attempt)")
    else
      actions+=("НЕ УДАЛОСЬ перезапустить туннель (попытка $attempt)")
      log "launchctl kickstart failed"
    fi
    sleep "$SETTLE_SECONDS"
    code=$(http_code "$URL")
  done
fi

code=$(http_code "$URL")
done_list=$(IFS='; '; printf '%s' "${actions[*]:-ничего не предпринято}")

if [ "$code" = "200" ]; then
  log "RECOVERED $URL -> 200 after: $done_list"
  notify "✅ pane.thothlab.tech восстановлен автоматически.
Было: $URL не отвечал.
Сделано: $done_list."
  exit 0
fi

log "STILL DOWN $URL -> $code after: $done_list (local=$local_code)"
notify "🔴 pane.thothlab.tech лежит и не поднялся сам.
Публично: $code. Локальный апстрим: $local_code.
Попытки: $done_list.
Лог: $LOG"
exit 1
