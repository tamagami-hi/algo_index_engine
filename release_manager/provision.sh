#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# provision.sh — create the directory tree this pipeline expects on the VPS.
#
# Idempotent and read-mostly: it creates directories and stages .env.example,
# and it will not overwrite an existing .env. Run it once per host, and again
# after any change to the path contract.
#
# It deliberately does NOT install .env. Credentials are placed by the operator
# so the only copy lives on the VPS; see the instructions it prints at the end.
#
# Usage:
#   ./release_manager/provision.sh --engine
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RM_DIR="$ROOT_DIR/release_manager"

# shellcheck source=lib/ui.sh
source "$RM_DIR/lib/ui.sh"
# shellcheck source=lib/stacks.sh
source "$RM_DIR/lib/stacks.sh"
# shellcheck source=lib/paths.sh
source "$RM_DIR/lib/paths.sh"

STACK=''
while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine|engine|index_engine) STACK="$(resolve_stack "$1")" || exit 1; shift ;;
        --help|-h) printf 'Usage: ./release_manager/provision.sh --engine\n'; exit 0 ;;
        *) err "unknown argument: $1"; exit 1 ;;
    esac
done
[[ -n "$STACK" ]] || { err "a stack is required: --engine"; exit 1; }

for c in ssh jq; do command -v "$c" >/dev/null || { err "$c is required"; exit 1; }; done

PATHS_FILE="$(stack_paths_file "$STACK")" || exit 1
paths_validate "$STACK" "$PATHS_FILE" || { err "path contract failed validation"; exit 1; }

banner "PROVISION · $STACK"

ROOT="$(paths_get "$PATHS_FILE" .vps.root)"
STACK_DIR="$(paths_get "$PATHS_FILE" .vps.stack_dir)"
assert_safe_remote_dir "$ROOT" || exit 1
assert_safe_remote_dir "$STACK_DIR" || exit 1

field "host"   "$BB_SSH_ALIAS"
field "root"   "$ROOT"
field "stack"  "$STACK_DIR"

step "checking SSH connectivity"
bb_ssh true 2>/dev/null || { err "cannot reach $BB_SSH_ALIAS over SSH"; exit 1; }
ok "SSH ok"

# Every directory comes from the contract, so this cannot drift from what the
# deploy scripts assert.
mapfile -t DIRS < <(jq -r '
    [ .vps.stack_dir, .vps.images_dir,
      .backup.root, .backup.rollback_root, .backup.rollback_images,
      .backup.logs_root, .backup.deploy_log, .backup.app_log ] | .[]' "$PATHS_FILE")

(( ${#DIRS[@]} > 0 )) || { err "the contract declares no directories"; exit 1; }

# Validated locally before they become part of a remote command line, so a
# hostile or truncated contract value cannot reach mkdir on the VPS.
for dir in "${DIRS[@]}"; do
    assert_safe_remote_dir "$dir" || exit 1
done

step "creating the directory tree"
# Arguments, not stdin: the heredoc already owns ssh's stdin, so piping the list
# would silently discard it and the remote loop would read the script itself.
REMOTE_ARGV="$(printf "'%s' " "${DIRS[@]}")"
bb_ssh "bash -s -- $REMOTE_ARGV" <<'REMOTE' || { err "failed to create the tree"; exit 1; }
set -euo pipefail
for dir in "$@"; do
    [[ -n "$dir" ]] || continue
    case "$dir" in
        /*) : ;;
        *) printf 'refusing relative path: %s\n' "$dir" >&2; exit 1 ;;
    esac
    mkdir -p -- "$dir"
    chmod 755 -- "$dir"
    printf '  %s\n' "$dir"
done
REMOTE
ok "directory tree present"

step "staging .env.example"
bb_ssh_opts
printf -v RSYNC_SSH '%q ' ssh "${BB_SSH_OPTS[@]}"
rsync -az --chmod=F644 -e "$RSYNC_SSH" \
    "$RM_DIR/stacks/$STACK/.env.example" "${BB_SSH_ALIAS}:${STACK_DIR}/" \
    || { err "failed to stage .env.example"; exit 1; }
ok ".env.example staged"

step "checking docker on the VPS"
bb_ssh 'docker info >/dev/null 2>&1' || { err "docker is not usable by the deploy user"; exit 1; }
bb_ssh 'docker compose version >/dev/null 2>&1' || { err "docker compose plugin missing"; exit 1; }
ok "docker and compose usable without sudo"

ENV_STATE="$(bb_ssh "test -e '$STACK_DIR/.env' && echo present || echo missing")"

banner "PROVISIONED"
if [[ "$ENV_STATE" == present ]]; then
    ok ".env is present — left completely untouched"
else
    warn ".env is not present; the deploy will refuse to run without it"
    printf '\n   Place it yourself at:\n\n     %s:%s/.env\n\n' "$BB_SSH_ALIAS" "$STACK_DIR"
    printf '   No script here reads, writes, or removes that file. .env.example\n'
    printf '   above is reference only. Use DHAN_AUTH_MODE=token_url: web mode\n'
    printf '   needs a browser this host does not have.\n\n'
fi
