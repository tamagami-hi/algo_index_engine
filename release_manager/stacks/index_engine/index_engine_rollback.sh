#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# index_engine_rollback.sh — VPS-native rollback entry point.
#
# Runs ON THE VPS, in the stack directory:
#
#     ssh beonedge 'cd /srv/dev_stack/ALGO_INDEX_ENGINE/index_engine && ./index_engine_rollback.sh --list'
#
# All logic is shared. This file declares identity only.
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=../_shared/_bb_lib.sh
source "$HERE/_bb_lib.sh"
# shellcheck source=../_shared/_bb_rollback.sh
source "$HERE/_bb_rollback.sh"

bb_rollback_main "$HERE/paths.json" "$@"
