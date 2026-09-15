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
