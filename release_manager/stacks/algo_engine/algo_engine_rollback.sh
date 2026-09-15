#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# algo_engine_rollback.sh — VPS-native rollback entry point.
#
# Runs ON THE VPS, in the stack directory:
#
#     ssh algo_engine 'cd /home/ubuntu/blackbox_trage/algo_engine && ./algo_engine_rollback.sh --list'
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
