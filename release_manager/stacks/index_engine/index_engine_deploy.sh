#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# index_engine_deploy.sh — VPS-native deploy entry point.
#
# Runs ON THE VPS, in the stack directory. Invoked either by the operator
# machine (release_manager/deploy.sh --engine) or by hand:
#
#     ssh beonedge 'cd /srv/dev_stack/ALGO_INDEX_ENGINE/index_engine && ./index_engine_deploy.sh'
#
# All logic is shared. This file declares identity and policy only.
#
# Policy: confirmation required. This engine places real orders against a live
# Dhan account, so a deploy is never silent even though there is only one stack.
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=../_shared/_bb_lib.sh
source "$HERE/_bb_lib.sh"
# shellcheck source=../_shared/_bb_deploy.sh
source "$HERE/_bb_deploy.sh"

BB_REQUIRE_CONFIRM=true

bb_deploy_main "$HERE/paths.json" "$@"
