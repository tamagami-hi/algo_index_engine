#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# rollback_pairing.sh — proves a rollback restores a coherent release.
#
# The failure this guards against: deploy.sh rsyncs the incoming compose file
# over the live one before the VPS-native deploy script runs, so anything that
# archives "the outgoing release" after that point pairs the OLD image with the
# NEW config. Rolling back then produces a combination that never ran.
#
# Runs entirely offline. docker is stubbed, so no daemon, no images, no SSH.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARED="$HERE/../stacks/_shared"

PASS=0
FAIL=0

check() {
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

STACK_DIR="$ROOT/srv/dev_stack/index_engine"
ROLLBACK_IMAGES="$ROOT/srv/backup/rollback/images"
mkdir -p "$STACK_DIR/images" "$ROLLBACK_IMAGES" "$ROOT/srv/backup/logs/deploy" \
         "$ROOT/srv/backup/logs/app" "$ROOT/bin"

cat > "$ROOT/bin/docker" <<'STUB'
#!/usr/bin/env bash
case "$1 ${2:-}" in
    "image inspect") exit 0 ;;
    "save")          printf 'FAKE IMAGE LAYER FOR %s\n' "${2:-unknown}" ;;
    "image load")    cat > /dev/null ;;
    *)               exit 0 ;;
esac
STUB
chmod +x "$ROOT/bin/docker"
export PATH="$ROOT/bin:$PATH"

PATHS_FILE="$STACK_DIR/paths.json"
cat > "$PATHS_FILE" <<JSON
{
  "schema": 1,
  "stack": "index_engine",
  "environment": "test",
  "short": "index",
  "vps": {
    "root": "$ROOT/srv/dev_stack",
    "stack_dir": "$STACK_DIR",
    "images_dir": "$STACK_DIR/images",
    "compose_file": "$STACK_DIR/compose.index_engine.yml",
    "compose_name": "compose.index_engine.yml",
    "env_file": "$STACK_DIR/.env",
    "env_example": "$STACK_DIR/.env.example",
    "version_file": "$STACK_DIR/index-engine-version.json",
    "version_name": "index-engine-version.json",
    "manifest_file": "$STACK_DIR/manifest.json",
    "checksums_file": "$STACK_DIR/checksums.sha256",
    "registry": "$ROOT/srv/dev_stack/manifest.json",
    "data_volume": "test_data",
    "docker": "docker",
    "container_prefix": "aie-engine",
    "compose_project": "algo_index_engine",
    "lock_file": "$ROOT/test.lock"
  },
  "backup": {
    "root": "$ROOT/srv/backup",
    "rollback_root": "$ROOT/srv/backup/rollback",
    "rollback_images": "$ROLLBACK_IMAGES",
    "logs_root": "$ROOT/srv/backup/logs",
    "deploy_log": "$ROOT/srv/backup/logs/deploy",
    "app_log": "$ROOT/srv/backup/logs/app"
  },
  "images": [{ "key": "engine", "archive": "engine.tar.gz" }],
  "has_database": false,
  "health": { "mode": "container", "settle_seconds": 0 },
  "nginx": { "enabled": false },
  "web": { "enabled": false },
  "retention": { "keep_releases": 3, "keep_instrument_masters": 5 }
}
JSON

declare -A P=()
# shellcheck source=/dev/null
source "$SHARED/_bb_lib.sh"
bb_load_paths "$PATHS_FILE"
P[paths_file]="$PATHS_FILE"

printf '\nrelease A goes live\n'

printf 'services:\n  engine:\n    image: algo-index-engine:1.0.0\n    environment:\n      RELEASE: A\n' \
    > "${P[compose_file]}"
printf 'DHAN_API_KEY=secret-for-A\n' > "${P[env_file]}"
printf '{"stack":"index_engine","version":"1.0.0","previous":"","status":"active"}\n' \
    > "${P[version_file]}"

bb_self_archive "1.0.0" >/dev/null 2>&1

check "A's compose is archived when A goes live" \
    "RELEASE: A" \
    "$(grep -o 'RELEASE: A' "$ROLLBACK_IMAGES/1.0.0/compose.index_engine.yml" || true)"
check "A's release manifest records A" \
    "1.0.0" \
    "$(jq -r .version "$ROLLBACK_IMAGES/1.0.0/release.json")"
check "A's manifest records the env digest, not the env file" \
    "$(sha256sum "${P[env_file]}" | cut -d' ' -f1)" \
    "$(jq -r .env_sha256 "$ROLLBACK_IMAGES/1.0.0/release.json")"
check "the env file itself is never copied into the backup tree" \
    "" \
    "$(find "$ROOT/srv/backup" -name '.env' -o -name '*.env' | tr -d '\n')"
check_contains "no credential value reaches the backup tree" \
    "" \
    "$(grep -rl 'secret-for-A' "$ROOT/srv/backup" 2>/dev/null | tr -d '\n')"

printf '\ndeploy B: the operator machine overwrites the live compose first\n'

printf 'services:\n  engine:\n    image: algo-index-engine:2.0.0\n    environment:\n      RELEASE: B\n' \
    > "${P[compose_file]}"
printf '{"version":"2.0.0"}\n' > "${P[manifest_file]}"

RB_DIR="$ROLLBACK_IMAGES/1.0.0"
ARCHIVE_OUT="$(bb_archive_current_images "$RB_DIR" "1.0.0" 2>&1)"

check_contains "archiving the outgoing release succeeds when its config was preserved" \
    "already archived" \
    "$ARCHIVE_OUT"
check "the archive still holds A's compose, not B's" \
    "RELEASE: A" \
    "$(grep -o 'RELEASE: [AB]' "$RB_DIR/compose.index_engine.yml" || true)"
check "A's image was saved into the archive" \
    "yes" \
    "$([[ -s "$RB_DIR/engine.tar.gz" ]] && echo yes || echo no)"

printf '{"stack":"index_engine","version":"2.0.0","previous":"1.0.0","status":"active"}\n' \
    > "${P[version_file]}"
bb_self_archive "2.0.0" >/dev/null 2>&1

check "B self-archives its own compose" \
    "RELEASE: B" \
    "$(grep -o 'RELEASE: B' "$ROLLBACK_IMAGES/2.0.0/compose.index_engine.yml" || true)"

printf '\nrollback to A\n'

DESCRIBE="$(bb_rollback_describe "$RB_DIR" "1.0.0" 2>&1)"
check_contains "the rollback names the release it is restoring" "restoring release 1.0.0" "$DESCRIBE"
check_contains "the archived compose is checked against its manifest digest" \
    "compose matches the manifest" "$DESCRIBE"
check_contains "an unchanged env file is confirmed" "env file unchanged" "$DESCRIBE"

cp "$RB_DIR/compose.index_engine.yml" "${P[compose_file]}"
check "rolling back restores A's configuration, not A's image under B's config" \
    "RELEASE: A" \
    "$(grep -o 'RELEASE: [AB]' "${P[compose_file]}" || true)"
check "and the restored compose pins A's image" \
    "algo-index-engine:1.0.0" \
    "$(grep -o 'algo-index-engine:[0-9.]*' "${P[compose_file]}" || true)"

printf '\nconfig drift is reported rather than silently undone\n'

printf 'DHAN_API_KEY=rotated-after-A\n' > "${P[env_file]}"
DRIFT="$(bb_rollback_describe "$RB_DIR" "1.0.0" 2>&1)"
check_contains "a changed env file is called out" "env file has changed" "$DRIFT"
check_contains "and the operator is told rollback cannot fix it" \
    "NOT credentials" "$DRIFT"

printf '\nrefusing to build a mismatched bundle\n'

UNPRESERVED="$ROLLBACK_IMAGES/3.0.0"
mkdir -p "$UNPRESERVED"
REFUSAL="$( ( bb_archive_current_images "$UNPRESERVED" "3.0.0" ) 2>&1 || true )"
check_contains "archiving without a preserved compose is refused" \
    "refusing to build a mismatched rollback bundle" "$REFUSAL"

MISMATCH="$ROLLBACK_IMAGES/4.0.0"
mkdir -p "$MISMATCH"
cp "$RB_DIR/release.json" "$MISMATCH/release.json"
cp "$RB_DIR/compose.index_engine.yml" "$MISMATCH/compose.index_engine.yml"
CROSSED="$( ( bb_rollback_describe "$MISMATCH" "4.0.0" ) 2>&1 || true )"
check_contains "a manifest naming a different release is refused" \
    "refusing to restore a mismatched bundle" "$CROSSED"

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
