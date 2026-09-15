#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# status.sh — what is built here, what is live there, and do they agree.
#
# Read-only. Changes nothing locally or remotely, so it is safe to run at any
# point including mid-incident.
#
# Usage:
#   ./release_manager/status.sh --engine
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

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

STACK=''
while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine|engine|algo_engine) STACK="$(resolve_stack "$1")" || exit 1; shift ;;
        --help|-h) printf 'Usage: ./release_manager/status.sh --engine\n'; exit 0 ;;
        *) err "unknown argument: $1"; exit 1 ;;
    esac
done
[[ -n "$STACK" ]] || STACK=algo_engine

for c in jq git; do command -v "$c" >/dev/null || { err "$c is required"; exit 1; }; done

banner "STATUS · $STACK"

# ── local ───────────────────────────────────────────────────────────────────
section "LOCAL"

CANONICAL="$(cargo_version "$ROOT_DIR/Cargo.toml")" || exit 1
GIT_SHA="$(git -C "$ROOT_DIR" rev-parse --short=9 HEAD 2>/dev/null || echo unknown)"
GIT_BRANCH="$(git -C "$ROOT_DIR" symbolic-ref --short -q HEAD 2>/dev/null || echo detached)"
DIRTY=clean
git_dirty "$ROOT_DIR" && DIRTY=dirty

field "cargo version" "$CANONICAL"
field "branch"        "$GIT_BRANCH"
field "commit"        "$GIT_SHA ($DIRTY)"

if on_exact_release_tag "$ROOT_DIR" "$CANONICAL"; then
    field "next label" "$CANONICAL (stable)"
else
    field "next label" "$(dev_version "$(bump_version "$CANONICAL" patch)" "$ROOT_DIR") (dev)"
fi

NEWEST="$(bundle_path_newest "$BUILD_DIR/$STACK")"
if [[ -n "$NEWEST" ]]; then
    field "newest bundle" "$(basename "$NEWEST")"
    field "bundle size"   "$(du -sh "$NEWEST" | cut -f1)"
else
    warn "no bundle staged — run: ./release_manager/export.sh --engine"
fi

COUNT="$(bundle_dirs_oldest_first "$BUILD_DIR/$STACK" | wc -l)"
field "bundles kept" "$COUNT"

# ── contract ────────────────────────────────────────────────────────────────
section "PATH CONTRACT"
PATHS_FILE="$(stack_paths_file "$STACK")" || exit 1
if paths_validate "$STACK" "$PATHS_FILE" 2>/dev/null; then
    ok "stacks/$STACK/paths.json validates (schema $PATHS_SCHEMA_REQUIRED)"
else
    err "stacks/$STACK/paths.json FAILED validation"
    paths_validate "$STACK" "$PATHS_FILE" || true
fi
REMOTE_DIR="$(paths_get "$PATHS_FILE" .vps.stack_dir)" || exit 1
field "remote stack" "$REMOTE_DIR"
field "health mode"  "$(paths_get "$PATHS_FILE" .health.mode)"

if [[ -n "$NEWEST" && -f "$NEWEST/paths.json" ]]; then
    if cmp -s "$NEWEST/paths.json" "$PATHS_FILE"; then
        ok "newest bundle's contract matches the tracked contract"
    else
        warn "newest bundle's contract has DRIFTED — deploy.sh will refuse it, re-export"
    fi
fi

# ── ledger ──────────────────────────────────────────────────────────────────
section "LOCAL LEDGER"
LEDGER="$RM_DIR/state/versions.json"
if [[ -s "$LEDGER" ]] && jq -e --arg s "$STACK" '.[$s]' "$LEDGER" >/dev/null 2>&1; then
    field "last shipped" "$(jq -r --arg s "$STACK" '.[$s].built // "?"' "$LEDGER")"
    field "then deployed" "$(jq -r --arg s "$STACK" '.[$s].deployed // "?"' "$LEDGER")"
    field "status"       "$(jq -r --arg s "$STACK" '.[$s].status // "?"' "$LEDGER")"
    field "at"           "$(jq -r --arg s "$STACK" '.[$s].shipped_at // "?"' "$LEDGER")"
else
    info "nothing shipped from this machine yet"
fi

# ── remote ──────────────────────────────────────────────────────────────────
section "REMOTE"
if ! bb_ssh true 2>/dev/null; then
    warn "cannot reach $BB_SSH_ALIAS over SSH — remote state unknown"
    printf '\n'
    exit 0
fi
ok "SSH ok ($BB_SSH_ALIAS)"

VERSION_NAME="$(stack_attr "$STACK" version_file)"
PROJECT="$(paths_get "$PATHS_FILE" .vps.compose_project)"
ROLLBACK_IMAGES="$(paths_get "$PATHS_FILE" .backup.rollback_images)"

REMOTE_STATE="$(bb_ssh "bash -s -- '$REMOTE_DIR' '$VERSION_NAME' '$PROJECT' '$ROLLBACK_IMAGES'" <<'REMOTE' || true
set -u
dir="$1"; version_name="$2"; project="$3"; rollback_images="$4"
printf 'live_version=%s\n' "$(jq -r '.version // ""' "$dir/$version_name" 2>/dev/null || true)"
printf 'live_status=%s\n'  "$(jq -r '.status // ""'  "$dir/$version_name" 2>/dev/null || true)"
printf 'live_prev=%s\n'    "$(jq -r '.previous // ""' "$dir/$version_name" 2>/dev/null || true)"
printf 'env_present=%s\n'  "$([[ -e "$dir/.env" ]] && echo yes || echo no)"
printf 'containers=%s\n'   "$(docker ps --filter "label=com.docker.compose.project=$project" --format '{{.Names}} {{.Status}}' 2>/dev/null | paste -sd'|' -)"
printf 'rollbacks=%s\n'    "$(find "$rollback_images" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort | paste -sd',' -)"
printf 'disk=%s\n'         "$(df -h --output=avail "$dir" 2>/dev/null | tail -n1 | tr -d ' ')"
REMOTE
)"

rget() { printf '%s\n' "$REMOTE_STATE" | sed -n "s/^$1=//p" | tail -n1; }

field "live version" "$(rget live_version)"
field "live status"  "$(rget live_status)"
field "previous"     "$(rget live_prev)"
field "env present"  "$(rget env_present)"
field "disk free"    "$(rget disk)"
field "rollbacks"    "$(rget rollbacks)"

CONTAINERS="$(rget containers)"
if [[ -n "$CONTAINERS" ]]; then
    printf '\n'
    printf '%s\n' "${CONTAINERS//|/$'\n'}" | while IFS= read -r line; do
        [[ -n "$line" ]] && info "$line"
    done
else
    warn "no containers running for project $PROJECT"
fi

# ── agreement ───────────────────────────────────────────────────────────────
section "AGREEMENT"
LIVE="$(rget live_version)"
BUILT=''
[[ -n "$NEWEST" ]] && BUILT="$(jq -r '.version // ""' "$NEWEST/manifest.json" 2>/dev/null || true)"

if [[ -z "$LIVE" ]]; then
    warn "nothing deployed on the VPS yet"
elif [[ -z "$BUILT" ]]; then
    info "live: $LIVE — no local bundle to compare"
elif [[ "$LIVE" == "$BUILT" ]]; then
    ok "the newest local bundle is what is live: $LIVE"
else
    warn "newest local bundle ($BUILT) is NOT live ($LIVE)"
    info "ship it with: ./release_manager/deploy.sh --engine"
fi
printf '\n'
