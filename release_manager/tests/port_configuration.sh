#!/usr/bin/env bash
# Offline port contract regression tests; Compose renders config without a daemon.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
PASS=0
FAIL=0
unset BLACKBOX_HTTP_PORT
export BB_VERSION=port-test

for command in docker jq; do
    command -v "$command" >/dev/null || { printf '%s is required\n' "$command" >&2; exit 1; }
done
docker compose version >/dev/null || exit 1

check_equal() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        printf '  ok    %s\n' "$name"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n        expected: %s\n        actual: %s\n' "$name" "$expected" "$actual"
        FAIL=$((FAIL + 1))
    fi
}

check_rejected() {
    local name="$1"
    shift
    if ("${@:-bb_health_http_url}") >"$ROOT/result" 2>&1; then
        printf '  FAIL  %s was accepted\n' "$name"
        FAIL=$((FAIL + 1))
    else
        printf '  ok    %s\n' "$name"
        PASS=$((PASS + 1))
    fi
}

render_port() {
    local file="$1" env_file="$2"
    docker compose --project-name port-test --env-file "$env_file" \
        --file "$file" config --format json | jq -r \
        '.services.engine | [.ports[0].host_ip, .ports[0].published,
         (.ports[0].target | tostring), .environment.BLACKBOX_HTTP_ADDR,
         .environment.DHAN_REDIRECT_URL] | join("|")'
}

printf '\nCompose ports, container listener, and callback share the env port\n'
mkdir -p "$ROOT/local" "$ROOT/vps"
cp "$REPO/compose.yaml" "$ROOT/local/compose.yaml"
cp "$REPO/release_manager/stacks/index_engine/compose.index_engine.yml" "$ROOT/vps/compose.yaml"
for variant in local vps; do
    example="$REPO/.env.example"
    if [[ "$variant" == vps ]]; then
        example="$REPO/release_manager/stacks/index_engine/.env.example"
    fi
    example_port="$(sed -n 's/^BLACKBOX_HTTP_PORT=\([0-9][0-9]*\)$/\1/p' "$example")"
    cp "$example" "$ROOT/$variant/.env"
    check_equal "$variant example port" \
        "127.0.0.1|$example_port|$example_port|0.0.0.0:$example_port|http://127.0.0.1:$example_port/dhan/callback" \
        "$(render_port "$ROOT/$variant/compose.yaml" "$ROOT/$variant/.env")"
    sed 's/^BLACKBOX_HTTP_PORT=.*/BLACKBOX_HTTP_PORT=49123/' "$example" > "$ROOT/$variant/.env"
    check_equal "$variant env override updates listener and callback" \
        '127.0.0.1|49123|49123|0.0.0.0:49123|http://127.0.0.1:49123/dhan/callback' \
        "$(render_port "$ROOT/$variant/compose.yaml" "$ROOT/$variant/.env")"
    : > "$ROOT/$variant/.env"
    check_rejected "$variant missing port" render_port "$ROOT/$variant/compose.yaml" "$ROOT/$variant/.env"
    printf 'BLACKBOX_HTTP_PORT=\n' > "$ROOT/$variant/.env"
    check_rejected "$variant empty port" render_port "$ROOT/$variant/compose.yaml" "$ROOT/$variant/.env"
done

printf '\nNative deployment uses the authoritative env path\n'
# shellcheck source=../stacks/_shared/_bb_lib.sh
source "$HERE/../stacks/_shared/_bb_lib.sh"
jq --arg root "$ROOT" '.vps.stack_dir = ($root + "/vps")
    | .backup.rollback_images = ($root + "/backup/images")
    | .vps.compose_file = ($root + "/vps/compose.yaml")
    | .vps.env_file = ($root + "/selected.env")' \
    "$REPO/release_manager/stacks/index_engine/paths.json" > "$ROOT/paths.json"
bb_load_paths "$ROOT/paths.json"
BB_VERSION_FOR_COMPOSE=port-test
cat > "$ROOT/selected.env" <<'ENV'
BLACKBOX_HTTP_PORT=49234
DHAN_REDIRECT_URL=http://127.0.0.1:${BLACKBOX_HTTP_PORT}/dhan/callback
ENV
export BLACKBOX_HTTP_PORT=49999
check_equal 'native compose uses paths.json env_file for ports and container config' \
    '49234|http://127.0.0.1:49234/dhan/callback' \
    "$(compose config --format json | jq -r '[.services.engine.ports[0].published, .services.engine.environment.DHAN_REDIRECT_URL] | join("|")')"
unset BLACKBOX_HTTP_PORT
check_equal 'health follows the selected host port' 'http://127.0.0.1:49234/health' \
    "$(bb_health_http_url)"

if ! declare -F bb_health_http_url >/dev/null; then
    printf '  FAIL  bb_health_http_url is missing\n'
    FAIL=$((FAIL + 1))
else
    P[health_http_url]='http://127.0.0.1:47601/custom/ready?probe=1'
    check_equal 'health preserves its configured path and query' \
        'http://127.0.0.1:49234/custom/ready?probe=1' "$(bb_health_http_url)"
    P[health_compose_service]=''
    check_equal 'legacy contracts retain their explicit health URL' \
        'http://127.0.0.1:47601/custom/ready?probe=1' "$(bb_health_http_url)"
    P[health_compose_service]=engine

    compose() { cat "$ROOT/config.json"; }
    printf '%s\n' '{"services":{"engine":{"ports":[{"host_ip":"::1","published":"49234","target":8081,"protocol":"tcp"}]}}}' > "$ROOT/config.json"
    check_equal 'IPv6 loopback uses a bracketed URL authority' \
        'http://[::1]:49234/custom/ready?probe=1' "$(bb_health_http_url)"
    for mapping in \
        '[]' \
        '[{"host_ip":"0.0.0.0","published":"49234","target":8081}]' \
        '[{"host_ip":"127.0.0.1","published":"49234","target":8081,"protocol":"udp"}]' \
        '[{"host_ip":"127.0.0.1","published":"49234","target":8081},{"host_ip":"127.0.0.1","published":"49235","target":8082}]' \
        '[{"host_ip":"127.0.0.1","published":"0","target":8081}]' \
        '[{"host_ip":"127.0.0.1","published":"65536","target":8081}]' \
        '[{"host_ip":"127.0.0.1","published":"49234-49236","target":8081}]'; do
        jq -n --argjson ports "$mapping" '{services:{engine:{ports:$ports}}}' > "$ROOT/config.json"
        check_rejected "invalid health mapping $mapping"
    done
    printf '{"services":{}}\n' > "$ROOT/config.json"
    check_rejected 'missing health service'
    printf 'invalid json\n' > "$ROOT/config.json"
    check_rejected 'invalid Compose JSON'
    compose() { return 1; }
    check_rejected 'Compose configuration failure'
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
