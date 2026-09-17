#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# version_bump.sh — proves a version bump moves every file that quotes it.
#
# Cargo.toml is the authority, but three other files repeat the number and two of
# them break the build when they disagree:
#
#   Cargo.lock              CI runs clippy, tests and the release build with
#                           --locked, which refuses to update the lock. A bump
#                           that skips it fails every Rust job.
#   web/package.json        npm ci errors when these two disagree with each
#   web/package-lock.json   other, and the lock repeats it in two places.
#
# The sharpest trap is Cargo.toml itself: dependencies are also declared with a
# version key, so a naive edit can pin a dependency to the release number.
#
# Runs entirely offline against temporary files. No cargo, no npm, no git.
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

check_rejected() {
    local what="$1"; shift
    if "$@" 2>/dev/null; then
        printf '  FAIL  %s (was accepted)\n' "$what"
        FAIL=$((FAIL + 1))
    else
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    fi
}

check_accepted() {
    local what="$1"; shift
    if "$@" 2>/dev/null; then
        printf '  ok    %s\n' "$what"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s (was rejected)\n' "$what"
        FAIL=$((FAIL + 1))
    fi
}

ROOT="$(mktemp -d)" || exit 1
trap 'rm -rf -- "$ROOT"' EXIT

# shellcheck source=../lib/ui.sh
source "$RM_DIR/lib/ui.sh"
# shellcheck source=../lib/version.sh
source "$RM_DIR/lib/version.sh"

fixture() {
    mkdir -p "$ROOT/web"
    cat > "$ROOT/Cargo.toml" <<'TOML'
[package]
name = "algo_index_engine"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1.0.104"
serde = { version = "1.0.229", features = ["derive"] }

[profile.release]
version = "not-a-real-key"
TOML
    cat > "$ROOT/Cargo.lock" <<'LOCK'
version = 4

[[package]]
name = "anyhow"
version = "1.0.104"

[[package]]
name = "algo_index_engine"
version = "0.1.0"
dependencies = [
 "anyhow",
]
LOCK
    printf '{\n  "name": "algo-index-engine-web",\n  "version": "0.1.0"\n}\n' \
        > "$ROOT/web/package.json"
    jq -n '{name:"algo-index-engine-web", lockfileVersion:3, version:"0.1.0",
            packages:{"":{name:"algo-index-engine-web", version:"0.1.0"},
                      "node_modules/react":{version:"19.0.0"}}}' \
        > "$ROOT/web/package-lock.json"
}

printf '\nThe canonical version is read from [package] and nowhere else\n'

fixture
check 'cargo_version reads the package version' '0.1.0' "$(cargo_version "$ROOT/Cargo.toml")"
check 'lock_package_version finds this crate, not the first package' '0.1.0' \
    "$(lock_package_version "$ROOT/Cargo.lock" algo_index_engine)"
check 'lock_package_version reads a named dependency too' '1.0.104' \
    "$(lock_package_version "$ROOT/Cargo.lock" anyhow)"

printf '\nWriting Cargo.toml touches the package version only\n'

check_accepted 'a valid version is written' set_cargo_version "$ROOT/Cargo.toml" 0.2.0
check 'the package version moved' '0.2.0' "$(cargo_version "$ROOT/Cargo.toml")"
check 'a dependency pinned with a bare version is untouched' '1' \
    "$(grep -c '^anyhow = "1.0.104"$' "$ROOT/Cargo.toml")"
check 'a dependency pinned inside a table is untouched' '1' \
    "$(grep -c 'version = "1.0.229"' "$ROOT/Cargo.toml")"
check 'a version key in a later section is untouched' '1' \
    "$(grep -c 'version = "not-a-real-key"' "$ROOT/Cargo.toml")"
check 'exactly one line carries the new version' '1' \
    "$(grep -c '^version = "0.2.0"$' "$ROOT/Cargo.toml")"

printf '\nA malformed version or manifest is refused rather than written\n'

fixture
check_rejected 'a non-semver version is refused' set_cargo_version "$ROOT/Cargo.toml" 0.2
check_rejected 'a prerelease label is refused'   set_cargo_version "$ROOT/Cargo.toml" 1.0.0-rc1
check_rejected 'an empty version is refused'     set_cargo_version "$ROOT/Cargo.toml" ''
check 'the manifest is unchanged after a refusal' '0.1.0' "$(cargo_version "$ROOT/Cargo.toml")"

printf 'name = "x"\n' > "$ROOT/no-package.toml"
check_rejected 'a manifest with no [package] version is refused' \
    set_cargo_version "$ROOT/no-package.toml" 1.0.0
check_rejected 'a missing manifest is refused' \
    set_cargo_version "$ROOT/absent.toml" 1.0.0

printf '\nThe npm files move together, including the lock self-entry\n'

fixture
check_accepted 'package.json is written' set_json_version "$ROOT/web/package.json" 0.2.0
check_accepted 'package-lock.json is written' set_json_version "$ROOT/web/package-lock.json" 0.2.0
check 'package.json version moved' '0.2.0' "$(json_version "$ROOT/web/package.json")"
check 'the lock root version moved' '0.2.0' "$(json_version "$ROOT/web/package-lock.json")"
check 'the lock self-entry moved, which npm ci compares' '0.2.0' \
    "$(jq -r '.packages[""].version' "$ROOT/web/package-lock.json")"
check 'a dependency in the lock is untouched' '19.0.0' \
    "$(jq -r '.packages["node_modules/react"].version' "$ROOT/web/package-lock.json")"
check 'the lock stays valid json' '3' \
    "$(jq -r '.lockfileVersion' "$ROOT/web/package-lock.json")"
check_accepted 'an absent optional file is not an error' \
    set_json_version "$ROOT/web/absent.json" 0.2.0

printf '\nA partial bump is caught before it can be committed\n'

fixture
set_cargo_version "$ROOT/Cargo.toml" 0.2.0
check_rejected 'a stale Cargo.lock is caught' \
    assert_version_files_agree "$ROOT" algo_index_engine 0.2.0

sed -i 's/^version = "0.1.0"$/version = "0.2.0"/' "$ROOT/Cargo.lock"
check_rejected 'a stale package.json is caught' \
    assert_version_files_agree "$ROOT" algo_index_engine 0.2.0

set_json_version "$ROOT/web/package.json" 0.2.0
check_rejected 'a stale package-lock.json is caught' \
    assert_version_files_agree "$ROOT" algo_index_engine 0.2.0

set_json_version "$ROOT/web/package-lock.json" 0.2.0
check_accepted 'a complete bump is accepted' \
    assert_version_files_agree "$ROOT" algo_index_engine 0.2.0

# The lock self-entry is the one npm ci compares and the easiest to miss, so it
# has to fail the check on its own.
jq '.packages[""].version = "0.1.0"' "$ROOT/web/package-lock.json" > "$ROOT/tmp.json" \
    && mv "$ROOT/tmp.json" "$ROOT/web/package-lock.json"
check_rejected 'a stale lock self-entry alone is caught' \
    assert_version_files_agree "$ROOT" algo_index_engine 0.2.0

printf '\nBumping arithmetic and the files a bump owns\n'

check 'patch'                '0.1.1' "$(bump_version 0.1.0 patch)"
check 'minor resets patch'   '0.2.0' "$(bump_version 0.1.9 minor)"
check 'major resets both'    '1.0.0' "$(bump_version 0.9.9 major)"
check 'patch across a minor' '1.2.4' "$(bump_version 1.2.3 patch)"
check_rejected 'an unknown bump part is refused' bump_version 1.2.3 sideways

fixture
check 'a bump owns exactly the four version files' \
    'Cargo.toml Cargo.lock web/package.json web/package-lock.json' \
    "$(version_files "$ROOT" | tr '\n' ' ' | sed 's/ $//')"
rm -f "$ROOT/web/package-lock.json"
check 'an absent file drops out of the list rather than failing' \
    'Cargo.toml Cargo.lock web/package.json' \
    "$(version_files "$ROOT" | tr '\n' ' ' | sed 's/ $//')"

printf '\nThe real repository is consistent right now\n'

check 'this repository has no partial bump' '0' \
    "$(assert_version_files_agree "$REPO" algo_index_engine \
        "$(cargo_version "$REPO/Cargo.toml")" >/dev/null 2>&1; echo $?)"

printf '\nstatus.sh is the only place the version advances\n'

check 'status.sh exposes cutting a release' '1' \
    "$(grep -c '^action_cut_release()' "$REPO/release_manager/status.sh")"
check 'export.sh never writes a tag' '0' \
    "$(grep -cE '^[[:space:]]*git .*(tag|commit|push)' "$REPO/release_manager/export.sh")"
check 'deploy.sh never writes a tag' '0' \
    "$(grep -cE '^[[:space:]]*git .*(tag|commit|push)' "$REPO/release_manager/deploy.sh")"

check_accepted 'the release push is atomic so a tag cannot outrun its commit' \
    grep -q 'push --atomic origin' "$REPO/release_manager/status.sh"
check_accepted 'cutting a release refuses a dirty tree' \
    grep -q 'uncommitted change(s); commit or stash' "$REPO/release_manager/status.sh"
check_accepted 'cutting a release refuses an existing tag' \
    grep -q 'a released version is never re-cut' "$REPO/release_manager/status.sh"
check_accepted 'the bump proves the lock with --locked' \
    grep -q 'cargo check --locked' "$REPO/release_manager/status.sh"

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
(( FAIL == 0 ))
