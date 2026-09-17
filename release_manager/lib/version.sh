#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# version.sh — version labelling.
#
# Cargo.toml is the single source of truth for the canonical version. A Rust
# project already declares its version there, and a second VERSION file would
# only be one more thing to forget to bump.
#
# Labels:
#   clean tree, on the exact vX.Y.Z tag  →  X.Y.Z              (releasable)
#   anything else                        →  <next>-dev.N.gSHA[.dirty]
#
# The patch is bumped for a dev label so it always sorts after the last release.
# ─────────────────────────────────────────────────────────────────────────────

# cargo_version <cargo.toml> — the [package] version.
cargo_version() {
    local file="$1"
    [[ -f "$file" ]] || { printf 'no such file: %s\n' "$file" >&2; return 1; }
    awk '
        /^\[/          { in_package = ($0 == "[package]") }
        in_package && /^[[:space:]]*version[[:space:]]*=/ {
            gsub(/^[^"]*"|"[^"]*$/, ""); print; exit
        }
    ' "$file"
}

assert_semver() {
    [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && return 0
    printf 'not a bare semver version: %s\n' "${1:-<empty>}" >&2
    return 1
}

# bump_version <x.y.z> <major|minor|patch>
bump_version() {
    local version="$1" part="${2:-patch}" major minor patch
    IFS=. read -r major minor patch <<<"$version"
    case "$part" in
        major) printf '%d.0.0\n' $((major + 1)) ;;
        minor) printf '%d.%d.0\n' "$major" $((minor + 1)) ;;
        patch) printf '%d.%d.%d\n' "$major" "$minor" $((patch + 1)) ;;
        *) printf 'unknown bump part: %s\n' "$part" >&2; return 1 ;;
    esac
}

git_dirty() {
    [[ -n "$(git -C "$1" status --porcelain 2>/dev/null)" ]]
}

# on_exact_release_tag <root> <version> — HEAD is exactly v<version>, clean.
on_exact_release_tag() {
    local root="$1" version="$2"
    git_dirty "$root" && return 1
    git -C "$root" describe --exact-match --tags HEAD 2>/dev/null | grep -qx "v$version"
}

# dev_version <base> <root> — <base>-dev.<commits>.g<sha>[.dirty]
dev_version() {
    local base="$1" root="$2" count sha suffix=''
    count="$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 0)"
    sha="$(git -C "$root" rev-parse --short=9 HEAD 2>/dev/null || echo unknown)"
    git_dirty "$root" && suffix='.dirty'
    printf '%s-dev.%s.g%s%s\n' "$base" "$count" "$sha" "$suffix"
}

# ── writing a version ───────────────────────────────────────────────────────
# Cargo.toml is the authority, but three other files quote the same number and
# two of them break the build if they disagree:
#
#   Cargo.lock                 records the package's own version. CI runs clippy,
#                              tests and the release build with --locked, which
#                              refuses to update the lock, so a bump that skips
#                              it fails every Rust job.
#   web/package.json           independent copy; nothing reads it, but npm ci
#   web/package-lock.json      errors if these two disagree with each other.
#
# So a bump is not an edit to one file, and doing it by hand is how they drift.

# set_cargo_version <cargo.toml> <x.y.z>
#
# Rewrites only the version inside [package]. A dependency pinned as
# `version = "1.0"` sits in a different table and must never be touched, so the
# section is tracked rather than the first match taken.
set_cargo_version() {
    local file="$1" version="$2" tmp
    [[ -f "$file" ]] || { printf 'no such file: %s\n' "$file" >&2; return 1; }
    assert_semver "$version" || return 1

    tmp="$(mktemp "${file}.XXXXXX")" || { printf 'cannot stage %s\n' "$file" >&2; return 1; }
    if ! awk -v version="$version" '
        /^\[/ { in_package = ($0 == "[package]") }
        in_package && !written && /^[[:space:]]*version[[:space:]]*=/ {
            print "version = \"" version "\""; written = 1; next
        }
        { print }
        END { if (!written) exit 3 }
    ' "$file" > "$tmp"; then
        rm -f -- "$tmp"
        printf 'no [package] version line in %s\n' "$file" >&2
        return 1
    fi
    chmod --reference="$file" "$tmp" 2>/dev/null || true
    mv -- "$tmp" "$file"
}

# set_json_version <file> <x.y.z> — .version, and .packages[""].version when the
# file is an npm lock. Both must move together or npm ci refuses the tree.
set_json_version() {
    local file="$1" version="$2" tmp
    [[ -f "$file" ]] || return 0
    assert_semver "$version" || return 1

    tmp="$(mktemp "${file}.XXXXXX")" || { printf 'cannot stage %s\n' "$file" >&2; return 1; }
    if ! jq --arg v "$version" '
        .version = $v
        | if has("packages") and (.packages | has("")) then
              .packages[""].version = $v
          else . end
    ' "$file" > "$tmp"; then
        rm -f -- "$tmp"
        printf 'cannot rewrite %s\n' "$file" >&2
        return 1
    fi
    chmod --reference="$file" "$tmp" 2>/dev/null || true
    mv -- "$tmp" "$file"
}

# json_version <file> — .version, or empty when the file is absent.
json_version() {
    [[ -f "$1" ]] || return 0
    jq -r '.version // empty' "$1" 2>/dev/null || true
}

# lock_package_version <cargo.lock> <crate> — the version the lock records for
# this crate, which is what --locked compares against.
lock_package_version() {
    local file="$1" crate="$2"
    [[ -f "$file" ]] || return 0
    awk -v crate="$crate" '
        $0 == "name = \"" crate "\"" { found = 1; next }
        found && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
    ' "$file"
}

# assert_version_files_agree <root> <crate> <x.y.z> — every file that quotes the
# version quotes this one. Run after writing, so a partial bump cannot be
# committed and discovered by CI instead.
assert_version_files_agree() {
    local root="$1" crate="$2" want="$3" rc=0 found

    found="$(cargo_version "$root/Cargo.toml")"
    [[ "$found" == "$want" ]] || {
        printf 'Cargo.toml says %s, expected %s\n' "${found:-<none>}" "$want" >&2
        rc=1
    }

    found="$(lock_package_version "$root/Cargo.lock" "$crate")"
    [[ "$found" == "$want" ]] || {
        printf 'Cargo.lock says %s for %s, expected %s — regenerate the lock\n' \
            "${found:-<none>}" "$crate" "$want" >&2
        rc=1
    }

    local json
    for json in web/package.json web/package-lock.json; do
        [[ -f "$root/$json" ]] || continue
        found="$(json_version "$root/$json")"
        [[ "$found" == "$want" ]] || {
            printf '%s says %s, expected %s\n' "$json" "${found:-<none>}" "$want" >&2
            rc=1
        }
    done

    if [[ -f "$root/web/package-lock.json" ]]; then
        found="$(jq -r '.packages[""].version // empty' "$root/web/package-lock.json" 2>/dev/null)"
        [[ -z "$found" || "$found" == "$want" ]] || {
            printf 'web/package-lock.json packages[""] says %s, expected %s\n' "$found" "$want" >&2
            rc=1
        }
    fi

    return "$rc"
}

# version_files <root> — the files a bump touches, for staging and for restoring.
version_files() {
    local root="$1" f
    for f in Cargo.toml Cargo.lock web/package.json web/package-lock.json; do
        [[ -f "$root/$f" ]] && printf '%s\n' "$f"
    done
    return 0
}

# bundle_dirs_oldest_first <dir> — bundle basenames by build time, oldest first.
# Sorted on the UTC stamp in the directory name rather than mtime, so a bundle
# that was merely touched does not survive pruning ahead of a newer one.
bundle_dirs_oldest_first() {
    local dir="$1"
    [[ -d "$dir" ]] || return 0
    find "$dir" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null \
        | sed -E 's/^(.*)-([0-9]{8}T[0-9]{6}Z)$/\2\t\1-\2/' \
        | sort \
        | cut -f2
}

# bundle_path_newest <dir> — absolute path of the newest bundle, or empty.
bundle_path_newest() {
    local dir="$1" newest
    newest="$(bundle_dirs_oldest_first "$dir" | tail -n1)"
    [[ -n "$newest" ]] || return 0
    printf '%s/%s\n' "$dir" "$newest"
}
