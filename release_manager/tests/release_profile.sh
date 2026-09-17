#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# release_profile.sh — proves the shipped binary is an optimised build.
#
# The failure this guards against is quiet and expensive. A debug binary starts
# fine, answers /health, passes readiness, deploys, rolls back and reports the
# right version. Nothing in the pipeline inspects optimisation level. The only
# symptom is that the option chain hot path runs several times slower than it was
# ever measured at, on a strategy whose whole premise is reacting inside the
# window an opportunity exists.
#
# So the check is layered, and this suite covers every layer:
#   1. the Dockerfile compiles and installs the release profile
#   2. the binary derives its reported profile from the compiler, so it cannot lie
#   3. export.sh refuses an image whose own transcript says otherwise
#   4. status.sh's verify compiles that profile before an operator ships
#
# Runs entirely offline. No Docker, no cargo, no daemon.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RM_DIR="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$RM_DIR/.." && pwd)"

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

check_true() {
    local what="$1"; shift
    if "$@"; then
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n' "$what"
        FAIL=$((FAIL + 1))
    fi
}

check_false() {
    local what="$1"; shift
    if "$@"; then
        printf '  FAIL  %s\n' "$what"
        FAIL=$((FAIL + 1))
    else
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    fi
}

# shellcheck source=../lib/acceptance.sh
source "$RM_DIR/lib/acceptance.sh"

printf '\nThe image is compiled and assembled from the release profile\n'

check_true 'the Dockerfile compiles with cargo build --release' \
    grep -qE '^[[:space:]]*RUSTFLAGS=.*cargo build --release' "$REPO/Dockerfile"

check_true 'the Dockerfile installs the binary from target/release' \
    grep -q 'install -Dm755 target/release/algo_index_engine' "$REPO/Dockerfile"

check_false 'the Dockerfile never installs from target/debug' \
    grep -q 'target/debug' "$REPO/Dockerfile"

# A literal "release" would keep reporting release after someone dropped the
# flag, which is worse than no check at all: it would actively vouch for a debug
# build. The constant has to come from the compiler.
check_true 'the reported profile is derived from cfg!(debug_assertions)' \
    grep -q 'cfg!(debug_assertions)' "$REPO/src/server/state.rs"

check_true 'the startup log reports the profile so a container can be asked' \
    grep -q 'profile = server::state::PROFILE' "$REPO/src/main.rs"

check_true '/health carries the profile for a running deployment' \
    grep -q 'profile: crate::server::state::PROFILE' "$REPO/src/server/http.rs"

printf '\nexport.sh accepts a release image and refuses a debug one\n'

RELEASE_OUT='2026-09-17T08:02:14.669452Z  INFO algo_index_engine starting version="0.1.0" profile="release"
Error: Missing BLACKBOX_HTTP_ADDR; configure the backend address in .env'

DEBUG_OUT='2026-09-17T08:02:14.669452Z  INFO algo_index_engine starting version="0.1.0" profile="debug"
Error: Missing BLACKBOX_HTTP_ADDR; configure the backend address in .env'

LEGACY_OUT='Error: Missing BLACKBOX_HTTP_ADDR; configure the backend address in .env'

PANIC_OUT='thread '"'"'main'"'"' panicked at src/main.rs:1:1: boom'

check_true  'a release image is accepted'                 accept_release_profile "$RELEASE_OUT"
check_false 'a debug image is refused'                    accept_release_profile "$DEBUG_OUT"
check_false 'an image predating the check is refused'     accept_release_profile "$LEGACY_OUT"

check_true  'a debug image is distinguished from an old one' accept_profile_reported "$DEBUG_OUT"
check_false 'an image reporting no profile is recognised'    accept_profile_reported "$LEGACY_OUT"

check_true  'a release image still has to fail on configuration' \
    accept_starts_cleanly "$RELEASE_OUT"
check_false 'a panicking image is refused' accept_starts_cleanly "$PANIC_OUT"

# "release" appearing anywhere in the transcript must not satisfy the check; only
# the profile field may. A version label like 1.2.0-release would otherwise pass.
check_false 'the word release elsewhere in the log does not satisfy the check' \
    accept_release_profile 'INFO starting version="1.2.0-release" profile="debug"'

printf '\nexport.sh wires the acceptance library in\n'

check_true 'export.sh sources the acceptance library' \
    grep -q 'source "$RM_DIR/lib/acceptance.sh"' "$REPO/release_manager/export.sh"

check_true 'export.sh gates the bundle on the release profile' \
    grep -q 'accept_release_profile "$ACCEPT_OUT"' "$REPO/release_manager/export.sh"

check_true 'export.sh still gates on a clean configuration failure' \
    grep -q 'accept_starts_cleanly "$ACCEPT_OUT"' "$REPO/release_manager/export.sh"

# Every failure path must abort rather than warn: a bundle that was merely
# complained about still gets deployed. Counting err against exit keeps this
# honest as branches are added.
ACCEPT_SECTION="$(awk '/^section "RUNTIME ACCEPTANCE"/,/^# ── save/' "$REPO/release_manager/export.sh")"
check 'every acceptance failure aborts instead of warning' \
    "$(grep -c '^[[:space:]]*err ' <<< "$ACCEPT_SECTION")" \
    "$(grep -c '^[[:space:]]*exit 1$' <<< "$ACCEPT_SECTION")"
check 'the acceptance section never merely warns' '0' \
    "$(grep -c '^[[:space:]]*warn ' <<< "$ACCEPT_SECTION")"

printf '\nstatus.sh compiles the profile that ships before an operator ships it\n'

check_true 'verify builds the release profile' \
    grep -q 'cargo build --locked --release' "$REPO/release_manager/status.sh"

check_true 'verify treats a failed release build as a failure' \
    grep -q 'the release profile does not compile' "$REPO/release_manager/status.sh"

# CI is where the deployment target's -C target-cpu is exercised. If that moves
# out of CI, the flags the image is built with stop being built anywhere else.
check_true 'CI builds the release profile with the deployment target CPU' \
    grep -qE 'target-cpu=x86-64-v3" cargo build --locked --release' "$REPO/.github/workflows/ci.yml"

check_true 'CI runs this suite' \
    grep -q 'release_profile.sh' "$REPO/.github/workflows/ci.yml"

# The container step is the strongest guard of the four: it asks a running image
# what it is rather than reading what built it.
check_true 'CI asks the running container for its profile' \
    grep -q "profile == \"release\"" "$REPO/.github/workflows/ci.yml"

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
(( FAIL == 0 ))
