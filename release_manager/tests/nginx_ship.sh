#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# nginx_ship.sh — proves the vhost is shipped, routed and described correctly.
#
# The failure this guards against is silent drift: for months the nginx config was
# the one part of a release no script carried, so the box could sit on an old vhost
# with every other check passing. The sharpest case is the port, because the env
# file moves the engine and a stale proxy_pass leaves the interface dead while
# health, readiness and the deploy all report success.
#
# Runs entirely offline. No SSH, no nginx, no daemon.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RM_DIR="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$RM_DIR/.." && pwd)"

PASS=0
FAIL=0

check_equal() {
    local what="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n        expected: %s\n        actual:   %s\n' \
            "$what" "$expected" "$actual"
        FAIL=$((FAIL + 1))
    fi
}

check_contains() {
    local what="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n        expected to contain: %s\n        actual: %s\n' \
            "$what" "$needle" "$haystack"
        FAIL=$((FAIL + 1))
    fi
}

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

# shellcheck source=../lib/ui.sh
source "$RM_DIR/lib/ui.sh"
# shellcheck source=../lib/paths.sh
source "$RM_DIR/lib/paths.sh"
# shellcheck source=../lib/nginx_ship.sh
source "$RM_DIR/lib/nginx_ship.sh"

PATHS_FILE="$RM_DIR/stacks/index_engine/paths.json"

printf '\nthe routing table covers every shipped config\n'

UNMAPPED="$(nginx_ship_unmapped)"
check_equal "no config in the tree is unroutable" "" "$UNMAPPED"

MAPPED_COUNT="$(nginx_ship_map | grep -c .)"
TREE_COUNT="$(find "$RM_DIR/nginx" -maxdepth 1 -name '*.conf' | wc -l)"
check_equal "every config in the tree has a row" "$MAPPED_COUNT" "$TREE_COUNT"

while IFS='|' read -r file dest kind; do
    [[ -n "$file" ]] || continue
    check_equal "$file is present in the tree" \
        "yes" "$([[ -f "$RM_DIR/nginx/$file" ]] && echo yes || echo no)"
    check_contains "$file installs under sites-available" "sites-available/" "$dest"
    check_equal "$file is declared a site" "site" "$kind"
done < <(nginx_ship_map)

printf '\nthe staging destination comes from the contract\n'

REMOTE_DIR="$(nginx_ship_remote_dir "$PATHS_FILE")"
check_equal "the staging directory is the folder on the VPS" \
    "/srv/dev_stack/ALGO_INDEX_ENGINE/nginx" "$REMOTE_DIR"

DERIVED="$(jq 'del(.nginx.staging_dir)' "$PATHS_FILE" > "$ROOT/no-staging.json" \
    && nginx_ship_remote_dir "$ROOT/no-staging.json")"
check_equal "without an explicit setting it derives from vps.root" \
    "/srv/dev_stack/ALGO_INDEX_ENGINE/nginx" "$DERIVED"

source "$RM_DIR/lib/stacks.sh"
check_equal "the staging path passes the remote-path guard" \
    "ok" "$(assert_safe_remote_dir "$REMOTE_DIR" >/dev/null 2>&1 && echo ok || echo refused)"

printf '\nstaging copies the mapped configs into the bundle\n'

nginx_ship_stage "$ROOT/bundle" >/dev/null 2>&1
check_equal "the vhost lands in the bundle" \
    "yes" "$([[ -f "$ROOT/bundle/nginx/index.algo.boe.internal.conf" ]] && echo yes || echo no)"
check_equal "and is byte-identical to the tree" \
    "$(sha256sum < "$RM_DIR/nginx/index.algo.boe.internal.conf")" \
    "$(sha256sum < "$ROOT/bundle/nginx/index.algo.boe.internal.conf")"

printf '\nthe vhost answers to the configured domain\n'

DOMAIN="$(paths_get_opt "$PATHS_FILE" .nginx.internal_domain)"
check_equal "the contract names the domain" "index.algo.boe.internal" "$DOMAIN"
check_equal "the vhost server_name matches the contract" \
    "$DOMAIN" "$(nginx_ship_domains | paste -sd',' -)"

VHOST="$RM_DIR/nginx/index.algo.boe.internal.conf"
check_equal "the vhost carries the tailnet guard" "yes" \
    "$(grep -qE '^[[:space:]]*allow[[:space:]]+100\.64\.0\.0/10;' "$VHOST" && echo yes || echo no)"
check_equal "the vhost denies everything else" "yes" \
    "$(grep -qE '^[[:space:]]*deny[[:space:]]+all;' "$VHOST" && echo yes || echo no)"

printf '\nthe vhost is configured for a 20 Hz event stream\n'

for directive in "proxy_buffering off" "proxy_cache off" "gzip off"; do
    check_equal "$directive" "yes" \
        "$(grep -qE "^[[:space:]]*${directive};" "$VHOST" && echo yes || echo no)"
done
check_equal "the read timeout outlasts quiet keep-alive stretches" "yes" \
    "$(grep -qE '^[[:space:]]*proxy_read_timeout[[:space:]]+1h;' "$VHOST" && echo yes || echo no)"
check_equal "websocket upgrade headers are forwarded" "yes" \
    "$(grep -q 'connection_upgrade' "$VHOST" && echo yes || echo no)"

printf '\nthe vhost proxies to the port the env file publishes\n'

ENV_PORT="$(sed -n 's/^BLACKBOX_HTTP_PORT=\([0-9][0-9]*\)$/\1/p' \
    "$RM_DIR/stacks/index_engine/.env.example")"
VHOST_PORTS="$(grep -oE 'proxy_pass[[:space:]]+http://[^;]+' "$VHOST" \
    | grep -oE ':[0-9]+' | tr -d ':' | sort -u | paste -sd',' -)"
check_equal "the vhost and the stack env example agree on one port" \
    "$ENV_PORT" "$VHOST_PORTS"

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
