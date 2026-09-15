#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# paths.sh — the path contract reader.
#
# stacks/<stack>/paths.json is the SOLE authority for every path a deployment
# touches. Nothing derives a remote path in code; it is read from the contract
# and validated first. A path change is an edit to that JSON file.
#
# The contract is hand-written and tracked in git. It is never generated,
# regenerated or rewritten by any script here — export.sh copies it byte for
# byte, and deploy.sh refuses to ship a bundle whose copy has drifted.
# ─────────────────────────────────────────────────────────────────────────────

PATHS_SCHEMA_REQUIRED=1

# stack_paths_file <stack> — the tracked contract for a stack.
stack_paths_file() {
    local stack="$1" file
    file="${RM_DIR:?RM_DIR must be set}/stacks/$stack/paths.json"
    [[ -f "$file" ]] || { printf 'missing path contract: %s\n' "$file" >&2; return 1; }
    printf '%s\n' "$file"
}

# paths_get <file> <jq-query> — one value, failing on null or absent.
paths_get() {
    local file="$1" query="$2" value
    value="$(jq -r "$query // empty" "$file" 2>/dev/null)" || {
        printf 'cannot read %s from %s\n' "$query" "$file" >&2; return 1; }
    [[ -n "$value" ]] || { printf '%s is null or missing in %s\n' "$query" "$file" >&2; return 1; }
    printf '%s\n' "$value"
}

# paths_get_opt <file> <jq-query> — one value, empty string when absent.
paths_get_opt() {
    jq -r "${2} // empty" "$1" 2>/dev/null || true
}

# paths_validate <stack> <file> — prove the contract is usable before anything
# acts on it. Fails closed: a malformed contract must never reach the VPS,
# because by then a wrong path is a wrong rm -rf.
paths_validate() {
    local stack="$1" file="$2" schema declared value key

    [[ -f "$file" ]] || { printf 'no such contract: %s\n' "$file" >&2; return 1; }
    jq empty "$file" 2>/dev/null || { printf 'contract is not valid JSON: %s\n' "$file" >&2; return 1; }

    schema="$(jq -r '.schema // 0' "$file")"
    [[ "$schema" == "$PATHS_SCHEMA_REQUIRED" ]] || {
        printf 'contract schema %s, expected %s: %s\n' "$schema" "$PATHS_SCHEMA_REQUIRED" "$file" >&2
        return 1; }

    # Identity check. Binds a contract to one stack so a bundle built for
    # another stack cannot be deployed here even if every path happens to parse.
    declared="$(jq -r '.stack // empty' "$file")"
    [[ "$declared" == "$stack" ]] || {
        printf 'contract declares stack %s, expected %s\n' "${declared:-<none>}" "$stack" >&2
        return 1; }

    # Every path that will be created, written or removed must be absolute and
    # free of traversal. A relative path here would resolve against whatever
    # directory the remote script happened to be in.
    for key in \
        .vps.root .vps.stack_dir .vps.paths_file .vps.images_dir \
        .vps.compose_file .vps.env_file .vps.env_example .vps.version_file \
        .vps.manifest_file .vps.checksums_file .vps.deploy_script \
        .vps.rollback_script .vps.registry .vps.lock_file \
        .backup.root .backup.rollback_root .backup.rollback_images \
        .backup.logs_root .backup.deploy_log .backup.app_log
    do
        value="$(paths_get "$file" "$key")" || return 1
        [[ "$value" == /* ]]      || { printf '%s must be absolute: %s\n' "$key" "$value" >&2; return 1; }
        [[ "$value" != *".."* ]]  || { printf '%s must not contain ..: %s\n' "$key" "$value" >&2; return 1; }
    done

    for key in .vps.compose_name .vps.version_name .vps.container_prefix \
               .vps.compose_project .vps.docker .vps.data_volume \
               .environment .short
    do
        paths_get "$file" "$key" >/dev/null || return 1
    done

    # The stack dir must sit under the root, or archiving and pruning would
    # reach outside the tree this contract owns.
    local root stack_dir
    root="$(paths_get "$file" .vps.root)" || return 1
    stack_dir="$(paths_get "$file" .vps.stack_dir)" || return 1
    [[ "$stack_dir" == "$root"/* ]] || {
        printf 'stack_dir %s is not under root %s\n' "$stack_dir" "$root" >&2; return 1; }

    # At least one image, each with a key and an archive filename. The archive
    # is a bare filename: it is joined onto images_dir and onto the rollback
    # directory, so a path separator here would escape both.
    local count
    count="$(jq -r '.images | length' "$file" 2>/dev/null || echo 0)"
    (( count >= 1 )) || { printf 'contract declares no images\n' >&2; return 1; }
    while IFS=$'\t' read -r key value; do
        [[ -n "$key" ]]        || { printf 'image entry with no key\n' >&2; return 1; }
        [[ -n "$value" ]]      || { printf 'image %s has no archive\n' "$key" >&2; return 1; }
        [[ "$value" != */* ]]  || { printf 'image archive must be a bare filename: %s\n' "$value" >&2; return 1; }
    done < <(jq -r '.images[] | [.key, .archive] | @tsv' "$file")

    local mode
    mode="$(paths_get "$file" .health.mode)" || return 1
    case "$mode" in
        container|http) : ;;
        *) printf 'health.mode must be container or http, got %s\n' "$mode" >&2; return 1 ;;
    esac
    if [[ "$mode" == http ]]; then
        paths_get "$file" .health.http_url >/dev/null || return 1
    fi

    local keep
    keep="$(paths_get "$file" .retention.keep_releases)" || return 1
    [[ "$keep" =~ ^[0-9]+$ ]] && (( keep >= 1 )) || {
        printf 'retention.keep_releases must be a positive integer\n' >&2; return 1; }

    # The web edge is optional, but if declared it must be fully specified: the
    # root and Caddyfile are mounted into a container and a wrong path there
    # silently serves nothing.
    if [[ "$(paths_get_opt "$file" .web.enabled)" == "true" ]]; then
        for key in .web.domain .web.root .web.caddyfile .web.local_probe_url; do
            value="$(paths_get "$file" "$key")" || return 1
        done
        for key in .web.root .web.caddyfile; do
            value="$(paths_get "$file" "$key")" || return 1
            [[ "$value" == /* ]]     || { printf '%s must be absolute: %s\n' "$key" "$value" >&2; return 1; }
            [[ "$value" != *".."* ]] || { printf '%s must not contain ..: %s\n' "$key" "$value" >&2; return 1; }
            [[ "$value" == "$stack_dir"/* ]] || {
                printf '%s must live under the stack dir so it ships with the release: %s\n' \
                    "$key" "$value" >&2; return 1; }
        done
    fi
}
