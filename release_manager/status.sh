#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# status.sh — the release control centre: what is built here, what is live
# there, whether they agree, and every operation that changes either.
#
# ── HOW IT IS SAFE TO RUN MID-INCIDENT ──────────────────────────────────────
# Opening it changes nothing. The dashboard is read-only and one SSH round trip.
# Every action that touches the VPS asks first, and the ones that can interrupt
# trading say so before asking. --status stays purely read-only for scripts and
# for the moments when a menu is the wrong thing to be looking at.
#
# ── WHY THERE IS NO STACK PICKER ────────────────────────────────────────────
# One stack, one engine. The sibling repositories pick between dev, prod and
# monitor because they ship three; adding a prompt with one answer would be a
# keystroke that teaches the operator to stop reading prompts.
#
# ── WHY THE ACTIONS SHELL OUT ───────────────────────────────────────────────
# export.sh, deploy.sh and rollback.sh remain the entry points and keep working
# without this file. This is a front door, not a reimplementation: anything it
# can do can be done by running those scripts directly, which is what the
# printed command lines are for.
#
# Usage:
#   ./release_manager/status.sh              interactive control centre
#   ./release_manager/status.sh --status     print the dashboard and exit
#   ./release_manager/status.sh --diagnose   VPS readiness checks and exit
#   ./release_manager/status.sh --verify     run the offline test suites and exit
# ─────────────────────────────────────────────────────────────────────────────

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RM_DIR="$ROOT_DIR/release_manager"
BUILD_DIR="$RM_DIR/build"

# shellcheck source=lib/ui.sh
source "$RM_DIR/lib/ui.sh"
# shellcheck source=lib/version.sh
source "$RM_DIR/lib/version.sh"
# shellcheck source=lib/stacks.sh
source "$RM_DIR/lib/stacks.sh"
# shellcheck source=lib/paths.sh
source "$RM_DIR/lib/paths.sh"
# shellcheck source=lib/nginx_ship.sh
source "$RM_DIR/lib/nginx_ship.sh"

STACK=index_engine

for c in jq git ssh; do
    command -v "$c" >/dev/null || { err "$c is required"; exit 1; }
done

PATHS_FILE="$(stack_paths_file "$STACK")" || exit 1
REMOTE_DIR="$(paths_get "$PATHS_FILE" .vps.stack_dir)" || exit 1
REMOTE_ENV_FILE="$(paths_get "$PATHS_FILE" .vps.env_file)" || exit 1
VERSION_NAME="$(stack_attr "$STACK" version_file)"
PROJECT="$(paths_get "$PATHS_FILE" .vps.compose_project)" || exit 1
ROLLBACK_IMAGES="$(paths_get "$PATHS_FILE" .backup.rollback_images)" || exit 1
DEPLOY_LOG_DIR="$(paths_get "$PATHS_FILE" .backup.deploy_log)" || exit 1
NGINX_DIR="$(nginx_ship_remote_dir "$PATHS_FILE" || true)"
NGINX_VHOST="$(paths_get_opt "$PATHS_FILE" .nginx.vhost_file)"
NGINX_DOMAIN="$(paths_get_opt "$PATHS_FILE" .nginx.internal_domain)"

# ── remote state, fetched at most once per dashboard render ──────────────────
REMOTE_STATE=''
REMOTE_FETCHED=false
REMOTE_REACHABLE=false

fetch_remote_state() {
    [[ "$REMOTE_FETCHED" == true ]] && return 0
    REMOTE_FETCHED=true
    REMOTE_STATE=''
    REMOTE_REACHABLE=false
    bb_ssh true 2>/dev/null || return 0
    REMOTE_REACHABLE=true

    REMOTE_STATE="$(bb_ssh "bash -s -- '$REMOTE_DIR' '$VERSION_NAME' '$PROJECT' '$ROLLBACK_IMAGES' '$REMOTE_ENV_FILE' '$NGINX_DIR' '$NGINX_VHOST' '$NGINX_DOMAIN'" <<'REMOTE' || true
set -u
dir="$1"; version_name="$2"; project="$3"; rollback_images="$4"; env_file="$5"
nginx_dir="${6:-}"; nginx_vhost="${7:-}"; nginx_domain="${8:-}"
printf 'live_version=%s\n' "$(jq -r '.version // ""' "$dir/$version_name" 2>/dev/null || true)"
printf 'live_status=%s\n'  "$(jq -r '.status // ""'  "$dir/$version_name" 2>/dev/null || true)"
printf 'live_prev=%s\n'    "$(jq -r '.previous // ""' "$dir/$version_name" 2>/dev/null || true)"
printf 'env_present=%s\n'  "$([[ -e "$env_file" ]] && echo yes || echo no)"
printf 'containers=%s\n'   "$(docker ps --filter "label=com.docker.compose.project=$project" --format '{{.Names}} {{.Status}}' 2>/dev/null | paste -sd'|' -)"
printf 'rollbacks=%s\n'    "$(find "$rollback_images" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort | paste -sd',' -)"
printf 'disk=%s\n'         "$(df -h --output=avail "$dir" 2>/dev/null | tail -n1 | tr -d ' ')"
printf 'nginx_service=%s\n' "$(systemctl is-active nginx 2>/dev/null || echo unknown)"
printf 'nginx_staged=%s\n'  "$([[ -n "$nginx_dir" ]] && find "$nginx_dir" -maxdepth 1 -name '*.conf' -printf '%f\n' 2>/dev/null | sort | paste -sd',' -)"
printf 'nginx_enabled=%s\n' "$([[ -n "$nginx_vhost" && -e "$nginx_vhost" ]] && echo yes || echo no)"
printf 'nginx_hosts=%s\n'   "$([[ -n "$nginx_domain" ]] && grep -qs -- "$nginx_domain" /etc/hosts && echo yes || echo no)"
# Only meaningful once a vhost for this name exists. Without one, nginx answers
# from whatever the default server is, and a 200 from an unrelated site would read
# as the engine being reachable by hostname.
if [[ -n "$nginx_domain" && -n "$nginx_vhost" && -e "$nginx_vhost" ]]; then
    body="$(curl -sS -m 3 -H "Host: $nginx_domain" http://127.0.0.1/health 2>/dev/null || true)"
    code="$(curl -sS -o /dev/null -m 3 -w '%{http_code}' -H "Host: $nginx_domain" http://127.0.0.1/health 2>/dev/null || echo none)"
    printf 'nginx_reachable=%s\n' "$code"
    printf 'nginx_is_engine=%s\n' "$(printf '%s' "$body" | grep -q '"alive"' && echo yes || echo no)"
else
    printf 'nginx_reachable=%s\n' "no-vhost"
    printf 'nginx_is_engine=%s\n' "no"
fi
REMOTE
)"
}

rget() { printf '%s\n' "$REMOTE_STATE" | sed -n "s/^$1=//p" | tail -n1; }

newest_bundle() { bundle_path_newest "$BUILD_DIR/$STACK"; }

# ── the dashboard ───────────────────────────────────────────────────────────

show_status() {
    banner "ALGO INDEX ENGINE · release control"

    section "LOCAL"
    local canonical git_sha git_branch dirty newest count
    canonical="$(cargo_version "$ROOT_DIR/Cargo.toml")" || return 1
    git_sha="$(git -C "$ROOT_DIR" rev-parse --short=9 HEAD 2>/dev/null || echo unknown)"
    git_branch="$(git -C "$ROOT_DIR" symbolic-ref --short -q HEAD 2>/dev/null || echo detached)"
    dirty=clean
    git_dirty "$ROOT_DIR" && dirty=dirty

    field "cargo version" "$canonical"
    field "branch"        "$git_branch"
    field "commit"        "$git_sha ($dirty)"

    if on_exact_release_tag "$ROOT_DIR" "$canonical"; then
        field "next label" "$canonical (stable)"
    else
        field "next label" "$(dev_version "$(bump_version "$canonical" patch)" "$ROOT_DIR") (dev)"
    fi

    newest="$(newest_bundle)"
    if [[ -n "$newest" ]]; then
        field "newest bundle" "$(basename "$newest")"
        field "bundle size"   "$(du -sh "$newest" | cut -f1)"
    else
        warn "no bundle staged — Build + Ship → Build a bundle"
    fi
    count="$(bundle_dirs_oldest_first "$BUILD_DIR/$STACK" | wc -l)"
    field "bundles kept" "$count"

    section "PATH CONTRACT"
    if paths_validate "$STACK" "$PATHS_FILE" 2>/dev/null; then
        ok "stacks/$STACK/paths.json validates (schema $PATHS_SCHEMA_REQUIRED)"
    else
        err "stacks/$STACK/paths.json FAILED validation"
        paths_validate "$STACK" "$PATHS_FILE" || true
    fi
    field "remote stack" "$REMOTE_DIR"
    field "health mode"  "$(paths_get "$PATHS_FILE" .health.mode)"

    if [[ -n "$newest" && -f "$newest/paths.json" ]]; then
        if cmp -s "$newest/paths.json" "$PATHS_FILE"; then
            ok "newest bundle's contract matches the tracked contract"
        else
            warn "newest bundle's contract has DRIFTED — deploy.sh will refuse it, re-export"
        fi
    fi

    section "LOCAL LEDGER"
    local ledger="$RM_DIR/state/versions.json"
    if [[ -s "$ledger" ]] && jq -e --arg s "$STACK" '.[$s]' "$ledger" >/dev/null 2>&1; then
        field "last shipped"  "$(jq -r --arg s "$STACK" '.[$s].built // "?"' "$ledger")"
        field "then deployed" "$(jq -r --arg s "$STACK" '.[$s].deployed // "?"' "$ledger")"
        field "status"        "$(jq -r --arg s "$STACK" '.[$s].status // "?"' "$ledger")"
        field "at"            "$(jq -r --arg s "$STACK" '.[$s].shipped_at // "?"' "$ledger")"
    else
        info "nothing shipped from this machine yet"
    fi

    section "REMOTE"
    fetch_remote_state
    if [[ "$REMOTE_REACHABLE" != true ]]; then
        warn "cannot reach $BB_SSH_ALIAS over SSH — remote state unknown"
        printf '\n'
        return 0
    fi
    ok "SSH ok ($BB_SSH_ALIAS)"

    field "live version" "$(rget live_version)"
    field "live status"  "$(rget live_status)"
    field "previous"     "$(rget live_prev)"
    field "env present"  "$(rget env_present)"
    field "disk free"    "$(rget disk)"
    field "rollbacks"    "$(rget rollbacks)"

    local containers
    containers="$(rget containers)"
    if [[ -n "$containers" ]]; then
        printf '\n'
        printf '%s\n' "${containers//|/$'\n'}" | while IFS= read -r line; do
            [[ -n "$line" ]] && info "$line"
        done
    else
        warn "no containers running for project $PROJECT"
    fi

    # Four separate things must be true before the hostname works, and each fails
    # quietly on its own: staged, installed, resolving, answering.
    section "EDGE" "${NGINX_DOMAIN:-<no domain configured>}"
    field "nginx"        "$(rget nginx_service)"
    field "staged"       "$(rget nginx_staged)"
    field "vhost"        "$(rget nginx_enabled)"
    field "hosts entry"  "$(rget nginx_hosts)"
    local code
    code="$(rget nginx_reachable)"
    field "health via edge" "$code"

    if [[ "$(rget nginx_staged)" == '' ]]; then
        warn "no configs staged on the VPS — Edge → Ship nginx configs"
    elif [[ "$(rget nginx_enabled)" != yes ]]; then
        warn "staged but not installed — Edge → Show install steps"
    elif [[ "$code" == 200 && "$(rget nginx_is_engine)" == yes ]]; then
        ok "the edge answers for ${NGINX_DOMAIN:-the configured domain}"
        [[ "$(rget nginx_hosts)" == yes ]] \
            || info "no hosts entry on the VPS itself; browsing devices need one too"
    elif [[ "$code" == 200 ]]; then
        warn "something answered on that hostname but it is not this engine"
        info "another vhost is probably claiming the name — check server_name collisions"
    elif [[ "$code" == 403 ]]; then
        warn "nginx returned 403 — the request did not come from the tailnet or loopback"
    elif [[ "$code" == 502 || "$code" == 504 ]]; then
        warn "nginx reached but the engine did not answer — check the proxy_pass port"
        info "the env file is the only source of truth for that port"
    elif [[ "$code" == 404 ]]; then
        warn "nginx has no server block for this hostname"
    fi

    section "AGREEMENT"
    local live built
    live="$(rget live_version)"
    built=''
    [[ -n "$newest" ]] && built="$(jq -r '.version // ""' "$newest/manifest.json" 2>/dev/null || true)"

    if [[ -z "$live" ]]; then
        warn "nothing deployed on the VPS yet"
    elif [[ -z "$built" ]]; then
        info "live: $live — no local bundle to compare"
    elif [[ "$live" == "$built" ]]; then
        ok "the newest local bundle is what is live: $live"
    else
        warn "newest local bundle ($built) is NOT live ($live)"
        info "Build + Ship → Ship and deploy"
    fi
    printf '\n'
}

# ── guards ──────────────────────────────────────────────────────────────────

# One release action at a time. Two deploys interleaving would race on the
# rollback archive and the version file.
status_lock() {
    mkdir -p "$RM_DIR/state"
    exec 9>"$RM_DIR/state/.status.lock"
    flock -n 9 || { err "another status.sh release action is in progress"; return 1; }
}

require_remote() {
    fetch_remote_state
    [[ "$REMOTE_REACHABLE" == true ]] || { err "cannot reach $BB_SSH_ALIAS"; return 1; }
}

# Anything that recreates the container drops the market feed and every open
# browser stream. Worth one sentence before the prompt rather than a surprise.
warn_interrupts_trading() {
    warn "this restarts the engine: the market feed drops and reconnects, and any"
    warn "position the engine is managing is not being watched while it is down"
}

# ── actions ─────────────────────────────────────────────────────────────────

action_export() {
    local mode="${1:-build}"
    local -a extra=()
    case "$mode" in
        build) : ;;
        restage) extra+=(--skip-build) ;;
        *) err "unknown export mode: $mode"; return 1 ;;
    esac
    status_lock || return 1
    printf '\n'
    "$RM_DIR/export.sh" --engine "${extra[@]}"
}

action_deploy() {
    local mode="${1:-deploy}" rc=0
    local -a extra=()
    case "$mode" in
        deploy) : ;;
        ship-only) extra+=(--ship-only) ;;
        force) extra+=(--force) ;;
        *) err "unknown deployment mode: $mode"; return 1 ;;
    esac

    if [[ -z "$(newest_bundle)" ]]; then
        err "no bundle staged for $STACK"
        confirm "Build one now?" || return 1
        "$RM_DIR/export.sh" --engine || return 1
    fi

    if [[ "$mode" != ship-only ]]; then
        warn_interrupts_trading
        confirm "Continue?" || { warn "cancelled"; return 0; }
    fi

    status_lock || return 1
    printf '\n'
    "$RM_DIR/deploy.sh" --engine "${extra[@]}" || rc=$?
    REMOTE_FETCHED=false
    return "$rc"
}

# Recreate the containers with the CURRENT on-VPS .env and the already-deployed
# image. Nothing is shipped and the version does not change. This is the right
# tool after editing the env file on the VPS: compose re-reads it on `up -d`,
# where a plain `restart` would keep the old environment.
action_reload() {
    local rc=0
    require_remote || return 1

    section "RELOAD" "recreate containers with the current on-VPS env; nothing is shipped"
    field "stack dir" "$REMOTE_DIR"
    field "env file"  "$REMOTE_ENV_FILE"
    info "use this after changing BLACKBOX_HTTP_PORT or credentials on the VPS"

    if [[ "$UI_INTERACTIVE" == true ]]; then
        warn_interrupts_trading
        confirm "Reload on the VPS now?" || { warn "cancelled"; return 0; }
    fi

    local qd qv
    printf -v qd '%q' "$REMOTE_DIR"
    printf -v qv '%q' "$VERSION_NAME"
    printf '\n'
    bb_ssh "bash -s -- $qd $qv" <<'REMOTE' || rc=$?
set -euo pipefail
dir="$1"; version_name="$2"
cd "$dir"
version="$(jq -r '.version // empty' "$version_name" 2>/dev/null || true)"
[[ -n "$version" ]] || { echo "no deployed version recorded in $dir/$version_name — ship and deploy first" >&2; exit 65; }
echo "reloading version $version"
# _bb_lib.sh owns the compose invocation: it pins BB_VERSION, points Compose at
# the authoritative env file and unsets any shell port that would shadow it.
# shellcheck source=/dev/null
declare -A P=()
source ./_bb_lib.sh
bb_load_paths ./paths.json
P[paths_file]=./paths.json
BB_VERSION_FOR_COMPOSE="$version"
bb_assert_loopback_only
compose up -d --remove-orphans
compose ps
REMOTE
    REMOTE_FETCHED=false
    return "$rc"
}

action_rollback() {
    local rc=0
    require_remote || return 1

    printf '\n'
    "$RM_DIR/rollback.sh" --engine --list || return 1

    printf '\n   1) roll back to the newest archived release\n'
    printf '   2) roll back to a specific version\n'
    printf '   3) cancel\n'
    printf '\n%s   ➜ choice [1-3]: %s' "$c_bold" "$c_rst"
    local n; read -r n

    case "$n" in
        1) warn_interrupts_trading
           confirm "Roll back now?" || { warn "cancelled"; return 0; }
           status_lock || return 1
           "$RM_DIR/rollback.sh" --engine --yes || rc=$? ;;
        2) printf '%s   ➜ version: %s' "$c_bold" "$c_rst"
           local v; read -r v
           [[ -n "$v" ]] || { err "no version given"; return 1; }
           warn_interrupts_trading
           confirm "Roll back to $v now?" || { warn "cancelled"; return 0; }
           status_lock || return 1
           "$RM_DIR/rollback.sh" --engine --to "$v" --yes || rc=$? ;;
        3|'') warn "cancelled"; return 0 ;;
        *) err "unknown choice: $n"; return 1 ;;
    esac
    REMOTE_FETCHED=false
    return "$rc"
}

# ── the edge ────────────────────────────────────────────────────────────────
# Shipping the vhost is separate from a deploy on purpose. An nginx change is
# often the only change — a header, a timeout, a port that moved — and rebuilding
# an image and recreating the container to deliver a text file is both slower and
# riskier than the edit itself. This path ships from the working tree and touches
# no container.
#
# It still installs nothing. /etc/nginx needs root, and a bad file there takes
# down every site on this box at once, including ones this pipeline does not own.
action_nginx_ship() {
    local staged
    require_remote || return 1
    [[ -n "$NGINX_DIR" ]] || { err "cannot resolve the nginx staging directory"; return 1; }
    assert_safe_remote_dir "$NGINX_DIR" || { err "unsafe staging path: $NGINX_DIR"; return 1; }

    section "NGINX SHIP" "stage the vhost on the VPS; /etc/nginx stays untouched"
    field "source" "$(nginx_ship_source_dir)"
    field "remote" "$NGINX_DIR"
    field "domain" "${NGINX_DOMAIN:-<none>}"

    if [[ "$UI_INTERACTIVE" == true ]]; then
        confirm "Upload the nginx configs to $NGINX_DIR now?" || { warn "cancelled"; return 0; }
    fi

    staged="$(mktemp -d)" || { err "cannot create a staging directory"; return 1; }
    # shellcheck disable=SC2064 # expand now: the path must survive the return
    trap "rm -rf -- '$staged'" RETURN

    nginx_ship_stage "$staged" || { err "staging failed"; return 1; }
    compgen -G "$staged/nginx/*.conf" >/dev/null \
        || { err "nothing staged — check nginx_ship_map"; return 1; }

    bb_ssh_opts
    local rsync_ssh
    printf -v rsync_ssh '%q ' ssh "${BB_SSH_OPTS[@]}"

    step "uploading"
    bb_ssh "install -d -m 755 -- ${NGINX_DIR@Q}" || { err "cannot create $NGINX_DIR"; return 1; }
    rsync -az --checksum '--chmod=F644,D755' -e "$rsync_ssh" \
        "$staged/nginx/" "${BB_SSH_ALIAS}:${NGINX_DIR}/" \
        || { err "upload failed"; return 1; }
    nginx_ship_verify "$staged/nginx" "$NGINX_DIR" || return 1

    REMOTE_FETCHED=false
    nginx_ship_guide "$NGINX_DIR" "$PATHS_FILE"
}

action_nginx_guide() {
    require_remote || return 1
    [[ -n "$NGINX_DIR" ]] || { err "cannot resolve the nginx staging directory"; return 1; }
    nginx_ship_guide "$NGINX_DIR" "$PATHS_FILE"
}

action_nginx_test() {
    require_remote || return 1
    section "NGINX TEST" "validate the installed configuration without reloading"
    printf '\n'
    bb_ssh "sudo nginx -t" || { err "nginx -t failed — do NOT reload"; return 1; }
    ok "the installed configuration is valid"
    info "reload with: ssh $BB_SSH_ALIAS 'sudo systemctl reload nginx'"
    info "reload, never restart: a restart drops every in-flight SSE stream"
}

# With no edge installed the interface is still reachable, just not by name. This
# prints the exact tunnel rather than leaving the port to be worked out.
action_tunnel() {
    local port
    require_remote || return 1
    port="$(bb_ssh "cd ${REMOTE_DIR@Q} && sed -n 's/^BLACKBOX_HTTP_PORT=\([0-9][0-9]*\)\$/\1/p' ${REMOTE_ENV_FILE@Q} 2>/dev/null | tail -n1" || true)"

    section "SSH TUNNEL" "reach the interface without the nginx edge"
    if [[ -z "$port" ]]; then
        warn "could not read BLACKBOX_HTTP_PORT from $REMOTE_ENV_FILE"
        info "the env file is the only source of truth for the port; place it first"
        return 1
    fi
    field "published port" "$port"
    printf '\n     ssh -N -L %s:127.0.0.1:%s %s\n' "$port" "$port" "$BB_SSH_ALIAS"
    printf '     then open http://127.0.0.1:%s\n\n' "$port"
    info "leave it running; it is also how a browser reaches /dhan/callback for web login"
}

action_logs() {
    require_remote || return 1
    section "DEPLOY LOGS" "$DEPLOY_LOG_DIR"
    local listing
    listing="$(bb_ssh "ls -t ${DEPLOY_LOG_DIR@Q} 2>/dev/null | head -20" || true)"
    [[ -n "$listing" ]] || { warn "no deploy logs on the VPS yet"; return 0; }
    printf '\n'
    printf '%s\n' "$listing" | nl -w6 -s'  '
    printf '\n%s   ➜ which log (number, or Enter for the newest): %s' "$c_bold" "$c_rst"
    local n file
    read -r n
    if [[ -z "$n" ]]; then
        file="$(printf '%s\n' "$listing" | head -1)"
    else
        [[ "$n" =~ ^[0-9]+$ ]] || { err "not a number: $n"; return 1; }
        file="$(printf '%s\n' "$listing" | sed -n "${n}p")"
    fi
    [[ -n "$file" ]] || { err "no such log"; return 1; }
    printf '\n'
    bb_ssh "tail -n 200 ${DEPLOY_LOG_DIR@Q}/${file@Q}"
}

action_containers() {
    require_remote || return 1
    section "CONTAINERS" "$PROJECT"
    printf '\n'
    bb_ssh "docker ps --filter 'label=com.docker.compose.project=$PROJECT' --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'" || true
    printf '\n'
    if confirm "Show the last 60 log lines from the engine?"; then
        printf '\n'
        bb_ssh "docker logs --tail 60 \$(docker ps -q --filter 'label=com.docker.compose.project=$PROJECT' | head -1) 2>&1" || true
    fi
}

action_engine_state() {
    local port
    require_remote || return 1
    port="$(bb_ssh "sed -n 's/^BLACKBOX_HTTP_PORT=\([0-9][0-9]*\)\$/\1/p' ${REMOTE_ENV_FILE@Q} 2>/dev/null | tail -n1" || true)"
    [[ -n "$port" ]] || { err "cannot read the port from $REMOTE_ENV_FILE"; return 1; }

    section "ENGINE STATE" "queried on the VPS over loopback"
    printf '\n'
    info "/health — process liveness"
    bb_ssh "curl -sS -m 5 http://127.0.0.1:$port/health | jq ." || warn "no answer"
    printf '\n'
    info "/ready — whether the engine can actually work, and why not"
    bb_ssh "curl -sS -m 5 http://127.0.0.1:$port/ready | jq ." || warn "no answer"
    printf '\n'
    info "order postbacks — what Dhan has sent and what reached disk"
    bb_ssh "curl -sS -m 5 http://127.0.0.1:$port/api/state | jq .postbacks" || warn "no answer"
}

action_diagnose() {
    section "DIAGNOSE" "$BB_SSH_ALIAS"

    if ! bb_ssh true 2>/dev/null; then
        err "SSH to $BB_SSH_ALIAS failed"
        info "check: ssh $BB_SSH_ALIAS"
        return 1
    fi
    ok "SSH reachable"

    printf '\n'
    bb_ssh "bash -s -- '$REMOTE_DIR' '$REMOTE_ENV_FILE' '$ROLLBACK_IMAGES' '$DEPLOY_LOG_DIR' '$NGINX_DIR'" <<'REMOTE' || true
set -u
dir="$1"; env_file="$2"; rollbacks="$3"; logs="$4"; nginx_dir="${5:-}"
say() { printf '     %-26s %s\n' "$1" "$2"; }

say 'docker'          "$(command -v docker >/dev/null && docker --version 2>/dev/null || echo MISSING)"
say 'docker compose'  "$(docker compose version --short 2>/dev/null || echo MISSING)"
say 'jq'              "$(command -v jq >/dev/null && echo present || echo MISSING)"
say 'rsync'           "$(command -v rsync >/dev/null && echo present || echo MISSING)"
say 'flock'           "$(command -v flock >/dev/null && echo present || echo MISSING)"
say 'nginx'           "$(systemctl is-active nginx 2>/dev/null || echo unknown)"
printf '\n'
say 'stack dir'       "$([[ -d "$dir" ]] && echo present || echo MISSING)"
say 'env file'        "$([[ -e "$env_file" ]] && echo present || echo 'MISSING — operator must place it')"
if [[ -e "$env_file" ]]; then
    say 'env mode'    "$(stat -c '%a %U:%G' "$env_file" 2>/dev/null || echo unknown)"
    say 'env port'    "$(sed -n 's/^BLACKBOX_HTTP_PORT=\(.*\)$/\1/p' "$env_file" 2>/dev/null | tail -n1)"
fi
say 'rollback tree'   "$([[ -d "$rollbacks" ]] && echo present || echo MISSING)"
say 'rollback writable' "$([[ -w "$rollbacks" ]] && echo yes || echo NO)"
say 'deploy log dir'  "$([[ -d "$logs" ]] && echo present || echo MISSING)"
say 'nginx staging'   "$([[ -n "$nginx_dir" && -d "$nginx_dir" ]] && echo present || echo MISSING)"
printf '\n'
say 'disk /srv'       "$(df -h --output=avail /srv 2>/dev/null | tail -n1 | tr -d ' ')"
say 'memory free'     "$(free -h 2>/dev/null | awk '/^Mem:/{print $7}')"
say 'load'            "$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null)"
REMOTE
    printf '\n'
    info "anything MISSING above is a blocker; provision with VPS → Provision the VPS"
}

action_provision() {
    require_remote || return 1
    section "PROVISION" "create the directory tree this pipeline expects"
    if [[ "$UI_INTERACTIVE" == true ]]; then
        confirm "Run provisioning on $BB_SSH_ALIAS?" || { warn "cancelled"; return 0; }
    fi
    printf '\n'
    "$RM_DIR/provision.sh" --engine
}

action_validate_contracts() {
    section "PATH CONTRACT" "the tracked paths.json is the sole path authority"
    printf '\n'
    if paths_validate "$STACK" "$PATHS_FILE"; then
        ok "stacks/$STACK/paths.json validates"
    else
        err "validation failed — fix stacks/$STACK/paths.json"
        return 1
    fi
    jq empty "$PATHS_FILE" && ok "valid JSON"
}

# The offline suites, so a change can be checked before it is shipped rather than
# after. None of these touch the VPS or need a docker daemon beyond compose
# config rendering.
action_verify() {
    local rc=0
    section "VERIFY" "offline suites; nothing is shipped and nothing is deployed"

    step "shell syntax"
    local script
    while IFS= read -r script; do
        bash -n "$script" || { err "syntax error: $script"; rc=1; }
    done < <(find "$RM_DIR" -name '*.sh')
    (( rc == 0 )) && ok "every release_manager script parses"

    if command -v shellcheck >/dev/null; then
        step "shellcheck"
        find "$RM_DIR" -name '*.sh' -print0 \
            | xargs -0 shellcheck -x -P SCRIPTDIR -S warning \
            && ok "shellcheck clean" || rc=1
    else
        info "shellcheck not installed locally; CI runs it"
    fi

    local suite
    for suite in rollback_pairing access_control port_configuration nginx_ship release_profile version_bump; do
        [[ -f "$RM_DIR/tests/$suite.sh" ]] || continue
        step "$suite"
        bash "$RM_DIR/tests/$suite.sh" >/dev/null 2>&1 \
            && ok "$suite passed" \
            || { err "$suite FAILED — run: bash release_manager/tests/$suite.sh"; rc=1; }
    done

    if confirm "Also run the Rust and frontend suites? (slower)"; then
        step "cargo test"
        ( cd "$ROOT_DIR" && cargo test --quiet ) && ok "Rust tests passed" || rc=1
        if [[ -d "$ROOT_DIR/web/node_modules" ]]; then
            step "frontend tests"
            ( cd "$ROOT_DIR/web" && npm run test --silent ) \
                && ok "frontend tests passed" || rc=1
        else
            info "web/node_modules absent; run npm ci in web/ first"
        fi

        # cargo test builds the debug profile. The image ships the release
        # profile, so a release-only compile failure would sit here unnoticed
        # until export.sh runs the Docker build, after the version has been
        # advanced. Building it here is the same work, done before it costs
        # anything. The image also applies -C target-cpu=x86-64-v3; that exact
        # combination is built in CI, and pinning it here would invalidate the
        # local cache on every hand-run cargo build --release.
        step "cargo build --release (the profile the image ships)"
        ( cd "$ROOT_DIR" && cargo build --locked --release --quiet ) \
            && ok "the release profile compiles" \
            || { err "the release profile does not compile — export.sh would fail on the same code"; rc=1; }
    fi

    printf '\n'
    (( rc == 0 )) && ok "verification complete" || err "verification found problems"
    return "$rc"
}

action_engine_guide() {
    local guide="$RM_DIR/stacks/$STACK/$(stack_attr "$STACK" guide)"
    [[ -f "$guide" ]] || { err "guide not found: $guide"; return 1; }
    if command -v less >/dev/null; then less "$guide"; else cat "$guide"; fi
}

# ── cutting a release ───────────────────────────────────────────────────────
# This is the ONLY place the version advances, and the only place a tag is made.
# export.sh reads the tag to decide between a stable and a dev label; it never
# writes one, so nothing else in the pipeline can move the version by accident.

CRATE_NAME='algo_index_engine'

# A release tag has to name a commit that exists somewhere other than this
# laptop. Tagging an unpushed commit produces a version nobody else can check
# out, and the tag is what export.sh trusts to call a bundle stable.
require_release_git() {
    local branch dirty ahead behind

    branch="$(git -C "$ROOT_DIR" symbolic-ref --short -q HEAD || true)"
    [[ "$branch" == main ]] || {
        err "releases are cut from main, not ${branch:-a detached HEAD}"
        return 1
    }

    dirty="$(git -C "$ROOT_DIR" status --porcelain | wc -l)"
    (( dirty == 0 )) || {
        err "the tree has $dirty uncommitted change(s); commit or stash before cutting a release"
        info "the bump commit must contain the version files and nothing else"
        return 1
    }

    step "fetching origin"
    git -C "$ROOT_DIR" fetch --quiet --tags origin || {
        err "cannot reach origin; a release tag must be pushable"
        return 1
    }

    git -C "$ROOT_DIR" rev-parse --verify --quiet refs/remotes/origin/main >/dev/null || {
        err "origin/main does not exist; push main before cutting a release"
        return 1
    }

    behind="$(git -C "$ROOT_DIR" rev-list --count HEAD..refs/remotes/origin/main)"
    (( behind == 0 )) || {
        err "main is $behind commit(s) behind origin/main; integrate before releasing"
        return 1
    }

    ahead="$(git -C "$ROOT_DIR" rev-list --count refs/remotes/origin/main..HEAD)"
    if (( ahead > 0 )); then
        warn "main is $ahead commit(s) ahead of origin/main"
        info "the release push below sends them together with the tag, atomically"
    fi
    ok "on main, clean, and in step with origin"
}

restore_version_files() {
    local files
    mapfile -t files < <(version_files "$ROOT_DIR")
    (( ${#files[@]} > 0 )) || return 0
    ( cd "$ROOT_DIR" && git checkout --quiet -- "${files[@]}" ) \
        && info "restored the version files" \
        || warn "could not restore: check git status"
}

# write_release_version <x.y.z> — every file that quotes the version, then proof.
write_release_version() {
    local next="$1"

    step "Cargo.toml → $next"
    set_cargo_version "$ROOT_DIR/Cargo.toml" "$next" || return 1

    # cargo owns the lock. Regenerating it here rather than leaving it to the
    # next build is what makes the bump commit self-consistent: --locked, which
    # CI and the release build both pass, refuses to update it later.
    step "Cargo.lock (cargo update --workspace --offline)"
    if ! ( cd "$ROOT_DIR" && cargo update --workspace --offline --quiet 2>/dev/null ); then
        ( cd "$ROOT_DIR" && cargo update --workspace --quiet ) || {
            err "cargo could not update the lock file"
            return 1
        }
    fi

    local json
    for json in web/package.json web/package-lock.json; do
        [[ -f "$ROOT_DIR/$json" ]] || continue
        step "$json → $next"
        set_json_version "$ROOT_DIR/$json" "$next" || return 1
    done

    step "every version file agrees"
    assert_version_files_agree "$ROOT_DIR" "$CRATE_NAME" "$next" || return 1
    ok "Cargo.toml, Cargo.lock and the frontend all read $next"

    # The lock is only proven consistent by the flag that will reject it.
    step "cargo check --locked"
    ( cd "$ROOT_DIR" && cargo check --locked --quiet ) || {
        err "the bumped tree does not satisfy --locked, which CI requires"
        return 1
    }
    ok "the lock file matches the manifest"
}

action_cut_release() {
    status_lock || return 1

    local canonical next bump choice remote
    canonical="$(cargo_version "$ROOT_DIR/Cargo.toml")" || return 1
    assert_semver "$canonical" || return 1

    section "CUT RELEASE" "Cargo.toml currently reads $canonical"

    if on_exact_release_tag "$ROOT_DIR" "$canonical"; then
        info "HEAD is already tag v$canonical — export would produce a stable bundle now"
    fi

    require_release_git || return 1

    printf '\n'
    printf '   1) patch → %s   %s\n' "$(bump_version "$canonical" patch)" \
        "${c_dim}a fix or an internal change${c_rst}"
    printf '   2) minor → %s   %s\n' "$(bump_version "$canonical" minor)" \
        "${c_dim}new capability, backwards compatible${c_rst}"
    printf '   3) major → %s   %s\n' "$(bump_version "$canonical" major)" \
        "${c_dim}a break in behaviour or contract${c_rst}"
    printf '   4) release %s as it stands   %s\n' "$canonical" \
        "${c_dim}tag the current version without bumping${c_rst}"
    printf '   5) cancel\n\n'
    printf '%s   ➜ choice [1-5]: %s' "$c_bold" "$c_rst"
    read -r choice || return 0
    case "$choice" in
        1) bump=patch ;;
        2) bump=minor ;;
        3) bump=major ;;
        4) bump=none ;;
        *) warn "cancelled"; return 0 ;;
    esac

    if [[ "$bump" == none ]]; then
        next="$canonical"
    else
        next="$(bump_version "$canonical" "$bump")" || return 1
    fi
    assert_semver "$next" || return 1

    if git -C "$ROOT_DIR" rev-parse --verify --quiet "refs/tags/v$next" >/dev/null; then
        err "tag v$next already exists locally; a released version is never re-cut"
        return 1
    fi
    remote=0
    git -C "$ROOT_DIR" ls-remote --exit-code --tags origin "refs/tags/v$next" \
        >/dev/null 2>&1 || remote=$?
    case "$remote" in
        0) err "tag v$next already exists on origin"; return 1 ;;
        2) : ;;
        *) err "could not check whether v$next exists on origin"; return 1 ;;
    esac

    printf '\n'
    field "from"    "$canonical"
    field "to"      "$next"
    field "tag"     "v$next"
    field "commit"  "chore(release): v$next"
    field "pushes"  "main and the tag, in one atomic push"
    field "then"    "export.sh labels the bundle $next instead of a dev version"

    warn "cutting a release rewrites Cargo.toml, Cargo.lock and the frontend versions"
    confirm "Run the local suites first? (recommended)" && { action_verify || {
        err "verification failed; nothing was changed"
        return 1
    }; }

    confirm "Cut release v$next?" || { warn "cancelled"; return 0; }

    if [[ "$bump" != none ]]; then
        printf '\n'
        if ! write_release_version "$next"; then
            err "the version was not advanced"
            restore_version_files
            return 1
        fi

        local files
        mapfile -t files < <(version_files "$ROOT_DIR")
        ( cd "$ROOT_DIR" && git add -- "${files[@]}" ) || {
            err "could not stage the version files"
            restore_version_files
            return 1
        }
        git -C "$ROOT_DIR" commit --quiet -m "chore(release): v$next" || {
            err "the release commit failed; no tag or push was attempted"
            return 1
        }
        ok "committed the bump"
    fi

    git -C "$ROOT_DIR" tag -a "v$next" -m "Release v$next" || {
        err "tagging failed; nothing was pushed"
        return 1
    }
    ok "tagged v$next"

    # Atomic: main and the tag land together or neither does. A tag on a commit
    # origin does not have is a version nobody else can build.
    step "pushing main and v$next atomically"
    if ! git -C "$ROOT_DIR" push --atomic origin \
        refs/heads/main:refs/heads/main "refs/tags/v$next:refs/tags/v$next"; then
        err "the atomic push failed; origin was not partially updated"
        warn "the commit and tag v$next are retained locally — retry the push after checking origin"
        warn "export.sh will keep producing dev labels until the tag is on origin"
        return 1
    fi
    ok "pushed main and v$next"

    printf '\n'
    field "version" "$next"
    field "next"    "./release_manager/export.sh --engine"
    info "the bundle and image will be labelled $next, with kind=stable"
    printf '\n'
}

# ── menus ───────────────────────────────────────────────────────────────────

pause_after_action() {
    printf '\n%s   ➜ press Enter to continue %s' "$c_dim" "$c_rst"
    read -r _
}

menu_build_ship() {
    local choice
    while true; do
        cat <<MENU

${c_bold}━━ Build + Ship${c_rst}
   1) Cut a release           bump the version, commit, tag and push
   2) Build a bundle          build the image and stage a versioned bundle
   3) Re-stage a bundle       reuse the existing image, restage the artifacts
   4) Ship and deploy         upload the newest bundle and deploy it
   5) Ship only               upload for inspection; do not deploy
   6) Force redeploy          deploy again even if that version is already live
   b) Back

MENU
        printf '%s   ➜ Build + Ship choice: %s' "$c_bold" "$c_rst"
        read -r choice || return 0
        case "$choice" in
            1) action_cut_release      || warn "no release was cut" ;;
            2) action_export build     || warn "export did not complete" ;;
            3) action_export restage   || warn "re-stage did not complete" ;;
            4) action_deploy deploy    || warn "deploy did not complete" ;;
            5) action_deploy ship-only || warn "shipping did not complete" ;;
            6) action_deploy force     || warn "forced redeploy did not complete" ;;
            b|B) return 0 ;;
            *) warn "unknown choice: $choice"; continue ;;
        esac
        pause_after_action || return 0
    done
}

menu_vps() {
    local choice
    while true; do
        cat <<MENU

${c_bold}━━ VPS${c_rst}
   1) Reload the stack        recreate containers with the current on-VPS env
   2) Roll back               restore an archived release with its own config
   3) Engine health/ready     ask the running engine what state it is in
   4) Inspect containers      what is running, and recent engine logs
   5) View deploy logs        tail a deployment log from the VPS
   6) Diagnose the VPS        tooling, paths, permissions and free space
   7) Provision the VPS       create the directory tree this pipeline expects
   8) SSH tunnel              reach the interface without the nginx edge
   b) Back

MENU
        printf '%s   ➜ VPS choice: %s' "$c_bold" "$c_rst"
        read -r choice || return 0
        case "$choice" in
            1) action_reload      || warn "reload did not complete" ;;
            2) action_rollback    || warn "rollback did not complete" ;;
            3) action_engine_state || true ;;
            4) action_containers  || true ;;
            5) action_logs        || true ;;
            6) action_diagnose    || true ;;
            7) action_provision   || warn "provisioning did not complete" ;;
            8) action_tunnel      || true ;;
            b|B) return 0 ;;
            *) warn "unknown choice: $choice"; continue ;;
        esac
        pause_after_action || return 0
    done
}

menu_edge() {
    local choice
    while true; do
        cat <<MENU

${c_bold}━━ Edge${c_rst}  ${c_dim}${NGINX_DOMAIN:-no domain configured}${c_rst}
   1) Ship nginx configs      stage the vhost on the VPS, then show what to install
   2) Show install steps      compare staged against /etc/nginx and print commands
   3) Test installed config   run nginx -t on the VPS without reloading
   b) Back

MENU
        printf '%s   ➜ Edge choice: %s' "$c_bold" "$c_rst"
        read -r choice || return 0
        case "$choice" in
            1) action_nginx_ship  || warn "shipping did not complete" ;;
            2) action_nginx_guide || true ;;
            3) action_nginx_test  || true ;;
            b|B) return 0 ;;
            *) warn "unknown choice: $choice"; continue ;;
        esac
        pause_after_action || return 0
    done
}

menu_checks() {
    local choice
    while true; do
        cat <<MENU

${c_bold}━━ Checks${c_rst}
   1) Verify locally          shell, deployment and optionally Rust/frontend suites
   2) Validate path contract  the tracked paths.json is the sole path authority
   3) Engine guide            the operator document for this stack
   b) Back
MENU
        printf '%s   ➜ Checks choice: %s' "$c_bold" "$c_rst"
        read -r choice || return 0
        case "$choice" in
            1) action_verify             || true ;;
            2) action_validate_contracts || true ;;
            3) action_engine_guide       || true ;;
            b|B) return 0 ;;
            *) warn "unknown choice: $choice"; continue ;;
        esac
        pause_after_action || return 0
    done
}

menu_main() {
    local choice
    while true; do
        show_status
        cat <<MENU
${c_bold}━━ workflows${c_rst}
   1) Build + Ship            build, stage, upload and deploy
   2) VPS                     operate, inspect and recover the deployed engine
   3) Edge                    the nginx vhost and the hostname
   4) Checks                  verify locally before shipping
   r) Refresh
   q) Quit

MENU
        printf '%s   ➜ workflow: %s' "$c_bold" "$c_rst"
        read -r choice || return 0
        case "$choice" in
            1) menu_build_ship ;;
            2) menu_vps ;;
            3) menu_edge ;;
            4) menu_checks ;;
            r|R) REMOTE_FETCHED=false ;;
            q|Q) printf '\n'; return 0 ;;
            *) warn "unknown workflow: $choice" ;;
        esac
    done
}

# ── entry ───────────────────────────────────────────────────────────────────

status_main() {
    case "${1:-}" in
        --status)   show_status; return 0 ;;
        --diagnose) action_diagnose; return $? ;;
        --verify)   action_verify; return $? ;;
        --reload)   action_reload; return $? ;;
        --cut-release)
            [[ "$UI_INTERACTIVE" == true ]] \
                || { err "--cut-release needs a terminal: it asks which part to bump and confirms before pushing"; return 1; }
            action_cut_release; return $? ;;
        --engine|engine|index_engine) shift; status_main "${1:-}"; return $? ;;
        --help|-h)
            cat <<'USAGE'
Usage: ./release_manager/status.sh [--status | --diagnose | --verify | --reload | --cut-release]

With no arguments, opens the interactive release control centre.

  --status        print the state dashboard and exit (read-only)
  --diagnose      check the VPS for tooling, paths and permissions, then exit
  --verify        run the offline test suites and exit
  --reload        recreate the deployed containers with the current on-VPS env
  --cut-release   bump the version, commit, tag and push; the only place the
                  version advances. export.sh reads the tag, never writes one.
USAGE
            return 0 ;;
        "") : ;;
        *) err "unknown argument: $1"; return 1 ;;
    esac

    [[ "$UI_INTERACTIVE" == true ]] \
        || { err "not a terminal — use --status, --diagnose, --verify, --reload or --cut-release"; return 1; }
    menu_main
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    status_main "$@"
fi
