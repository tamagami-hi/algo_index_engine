#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# rollback.sh — invoke the VPS-native rollback from this computer.
#
# Runs no docker command itself; it is a thin, validated wrapper so the operator
# does not have to remember the remote path. Same division of labour as deploy.sh:
# the VPS-native script owns every container operation and holds the lock.
#
# Usage:
#   ./release_manager/rollback.sh --engine --list
#   ./release_manager/rollback.sh --engine --to 0.1.1-dev.14.gabc123def
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
REMOTE_ARGS=()

usage() {
    cat <<'USAGE'
Usage: ./release_manager/rollback.sh --engine [options]

Restores a previously archived release on the VPS.

Options:
  --list          show the current version and available targets
  --to VERSION    restore a specific release
  --yes, -y       skip the remote confirmation prompt
  --skip-checks   start the release but do not gate on health
  --help, -h      this message
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine|engine|algo_engine) STACK="$(resolve_stack "$1")" || exit 1; shift ;;
        --list)        REMOTE_ARGS+=(--list); shift ;;
        --to)          REMOTE_ARGS+=(--to "${2:-}"); shift 2 ;;
        --yes|-y)      REMOTE_ARGS+=(--yes); shift ;;
        --skip-checks) REMOTE_ARGS+=(--skip-checks); shift ;;
        --help|-h)     usage; exit 0 ;;
        *) err "unknown argument: $1"; usage >&2; exit 1 ;;
    esac
done

[[ -n "$STACK" ]] || { err "a stack is required: --engine"; usage >&2; exit 1; }
for c in ssh jq; do command -v "$c" >/dev/null || { err "$c is required"; exit 1; }; done

PATHS_FILE="$(stack_paths_file "$STACK")" || exit 1
paths_validate "$STACK" "$PATHS_FILE" || { err "path contract failed validation"; exit 1; }
REMOTE_DIR="$(paths_get "$PATHS_FILE" .vps.stack_dir)" || exit 1
assert_safe_remote_dir "$REMOTE_DIR" || exit 1
ROLLBACK_NAME="$(stack_attr "$STACK" rollback)"

banner "ROLLBACK · $STACK"
field "remote" "${BB_SSH_ALIAS}:${REMOTE_DIR}"

bb_ssh true 2>/dev/null || { err "cannot reach $BB_SSH_ALIAS over SSH"; exit 1; }
bb_ssh "test -x '$REMOTE_DIR/$ROLLBACK_NAME'" \
    || { err "no rollback script on the VPS — has a bundle ever been shipped?"; exit 1; }

bb_ssh_opts
ssh -t "${BB_SSH_OPTS[@]}" "$BB_SSH_ALIAS" \
    "cd '$REMOTE_DIR' && ./'$ROLLBACK_NAME' ${REMOTE_ARGS[*]:-}"
