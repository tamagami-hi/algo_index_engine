#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# access_control.sh — proves the deploy refuses an open control surface.
#
# The engine's API arms and disarms trading strategies. Two things must hold on
# every deploy: the container is reachable only on loopback, and if an nginx edge
# is declared then that edge actually restricts. Both must FAIL the deploy when
# violated rather than warn, because the alternative is a silently public surface.
#
# Runs entirely offline. docker compose config is stubbed.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARED="$HERE/../stacks/_shared"

PASS=0
FAIL=0

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

check_not_contains() {
    local what="$1" needle="$2" haystack="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n        expected NOT to contain: %s\n        actual: %s\n' \
            "$what" "$needle" "$haystack"
        FAIL=$((FAIL + 1))
    fi
}

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

STACK_DIR="$ROOT/stack"
mkdir -p "$STACK_DIR" "$ROOT/srv/backup/rollback/images" "$ROOT/bin" "$ROOT/nginx"

write_paths() {
    local nginx_enabled="$1" vhost="$2"
    cat > "$STACK_DIR/paths.json" <<JSON
{
  "schema": 1,
  "stack": "index_engine",
  "environment": "test",
  "short": "index",
  "vps": {
    "root": "$ROOT", "stack_dir": "$STACK_DIR",
    "images_dir": "$STACK_DIR/images",
    "compose_file": "$STACK_DIR/compose.yml", "compose_name": "compose.yml",
    "env_file": "$STACK_DIR/.env", "env_example": "$STACK_DIR/.env.example",
    "version_file": "$STACK_DIR/version.json", "version_name": "version.json",
    "manifest_file": "$STACK_DIR/manifest.json",
    "checksums_file": "$STACK_DIR/checksums.sha256",
    "registry": "$ROOT/registry.json", "data_volume": "test_data",
    "docker": "docker", "container_prefix": "aie", "compose_project": "test",
    "lock_file": "$ROOT/test.lock"
  },
  "backup": {
    "root": "$ROOT/srv/backup", "rollback_root": "$ROOT/srv/backup/rollback",
    "rollback_images": "$ROOT/srv/backup/rollback/images",
    "logs_root": "$ROOT/srv/backup/logs",
    "deploy_log": "$ROOT/srv/backup/logs/deploy",
    "app_log": "$ROOT/srv/backup/logs/app"
  },
  "images": [{ "key": "engine", "archive": "engine.tar.gz" }],
  "has_database": false,
  "health": { "mode": "container", "settle_seconds": 0 },
  "nginx": { "enabled": $nginx_enabled, "vhost_file": "$vhost" },
  "web": { "enabled": false },
  "retention": { "keep_releases": 3 }
}
JSON
}

# compose config is stubbed by pointing the library's `compose` helper at a
# script that emits whatever port mapping the case under test needs.
stub_ports() {
    cat > "$ROOT/ports.json"
}

compose_stub() {
    if [[ "${1:-}" == "config" ]]; then
        cat "$ROOT/ports.json"
        return 0
    fi
    return 0
}

write_paths false ""
declare -A P=()
# shellcheck source=/dev/null
source "$SHARED/_bb_lib.sh"
bb_load_paths "$STACK_DIR/paths.json"
# shellcheck disable=SC2034 # Consumed by the sourced deployment library.
P[paths_file]="$STACK_DIR/paths.json"

# Defined after sourcing on purpose: the library declares its own compose helper,
# and whichever definition comes last is the one that runs.
compose() { compose_stub "$@"; }

printf '\nloopback binding\n'

stub_ports <<'JSON'
{"services":{"engine":{"ports":[{"host_ip":"127.0.0.1","published":"47601","target":8081}]}}}
JSON
LOOPBACK="$( ( bb_assert_loopback_only ) 2>&1 || true )"
check_contains "a loopback-only mapping is accepted" \
    "bound to loopback" "$LOOPBACK"

stub_ports <<'JSON'
{"services":{"engine":{"ports":[{"host_ip":"0.0.0.0","published":"47601","target":8081}]}}}
JSON
WIDE="$( ( bb_assert_loopback_only ) 2>&1 || true )"
check_contains "publishing on 0.0.0.0 is refused" \
    "would publish the engine beyond loopback" "$WIDE"
check_contains "and the offending mapping is named" "0.0.0.0:47601" "$WIDE"

stub_ports <<'JSON'
{"services":{"engine":{"ports":[{"published":"47601","target":8081}]}}}
JSON
DEFAULTED="$( ( bb_assert_loopback_only ) 2>&1 || true )"
check_contains "an omitted host_ip defaults to every interface and is refused" \
    "beyond loopback" "$DEFAULTED"

stub_ports <<'JSON'
{"services":{"engine":{"ports":[{"host_ip":"127.0.0.1","published":"47601","target":8081},
                                {"host_ip":"10.0.0.5","published":"9999","target":9999}]}}}
JSON
MIXED="$( ( bb_assert_loopback_only ) 2>&1 || true )"
check_contains "one bad mapping among good ones is still refused" "10.0.0.5:9999" "$MIXED"

printf '\nedge guard\n'

NO_EDGE="$( ( bb_assert_access_control ) 2>&1 || true )"
check_contains "with no edge declared the operator is told how to reach the UI" \
    "ssh -N -L 47601" "$NO_EDGE"
check_not_contains "and that is not treated as a failure" "✗" "$NO_EDGE"

VHOST="$ROOT/nginx/algo-index-engine"
write_paths true "$VHOST"
bb_load_paths "$STACK_DIR/paths.json"

MISSING="$( ( bb_assert_access_control ) 2>&1 || true )"
check_contains "an enabled edge whose vhost is absent is refused" \
    "is not installed" "$MISSING"
check_contains "and the refusal says why it matters" \
    "unprotected control surface" "$MISSING"

cat > "$VHOST" <<'CONF'
server {
    listen 80;
    server_name engine.boe.app.internal;
    location / { proxy_pass http://127.0.0.1:47601; }
}
CONF
UNGUARDED="$( ( bb_assert_access_control ) 2>&1 || true )"
check_contains "a vhost with no deny all is refused" "has no 'deny all'" "$UNGUARDED"

cat > "$VHOST" <<'CONF'
server {
    listen 80;
    server_name engine.boe.app.internal;
    allow 192.168.1.0/24;
    deny all;
    location / { proxy_pass http://127.0.0.1:47601; }
}
CONF
WRONG_RANGE="$( ( bb_assert_access_control ) 2>&1 || true )"
check_contains "a vhost that denies all but allows the wrong network is refused" \
    "does not restrict access to the tailnet" "$WRONG_RANGE"

cat > "$VHOST" <<'CONF'
server {
    listen 80;
    server_name engine.boe.app.internal;
    allow 100.64.0.0/10;
    allow 127.0.0.1;
    allow ::1;
    deny all;
    location / { proxy_pass http://127.0.0.1:47601; }
}
CONF
GUARDED="$( ( bb_assert_access_control ) 2>&1 || true )"
check_contains "a tailnet-guarded vhost is accepted" \
    "restricts the control surface to the tailnet" "$GUARDED"

printf '\nthe shipped configs satisfy their own check\n'

for shipped in "$HERE/../nginx/"*.conf; do
    cp "$shipped" "$VHOST"
    NAME="$(basename "$shipped")"
    SHIPPED_OUT="$( ( bb_assert_access_control ) 2>&1 || true )"
    check_contains "$NAME passes the guard assertion" \
        "restricts the control surface to the tailnet" "$SHIPPED_OUT"
done

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
