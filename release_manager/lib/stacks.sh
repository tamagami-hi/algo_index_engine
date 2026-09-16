#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# stacks.sh — the stack registry.
#
# AUTHORITY BOUNDARY
# This file owns stack IDENTITY only: ids, selector resolution, image naming,
# and non-path metadata (compose filename, container prefix, retention). It owns
# NO paths. Every deployment, backup, log and image path lives in the stack's
# tracked paths.json contract and is read through lib/paths.sh. A path change is
# a paths.json edit, never an edit here.
#
# One algo per repository, so there is exactly one stack. The registry shape is
# kept anyway: this folder is copied verbatim into the next algo's repo, and a
# lookup table is cheaper to retarget than scattered literals.
# ─────────────────────────────────────────────────────────────────────────────

BB_SSH_ALIAS="${BB_SSH_ALIAS:-beonedge}"

BB_STACKS=(index_engine)

# stack_attr <stack> <attr> — echo one non-path attribute, or return 1.
#
#   env            environment name
#   short          short id used in filenames and image tags
#   compose        compose filename inside the stack dir
#   version_file   per-stack version filename
#   deploy         native deploy script filename
#   rollback       native rollback script filename
#   guide          guide filename
#   prefix         container name prefix
#   project        compose project name
#   keep           rollback releases retained
stack_attr() {
    local stack="$1" attr="$2"
    case "$stack" in
        index_engine)
            case "$attr" in
                env)          printf 'production\n' ;;
                short)        printf 'index\n' ;;
                compose)      printf 'compose.index_engine.yml\n' ;;
                version_file) printf 'index-engine-version.json\n' ;;
                deploy)       printf 'index_engine_deploy.sh\n' ;;
                rollback)     printf 'index_engine_rollback.sh\n' ;;
                guide)        printf 'ENGINE_GUIDE.md\n' ;;
                prefix)       printf 'aie-engine\n' ;;
                project)      printf 'algo_index_engine\n' ;;
                keep)         printf '3\n' ;;
                *) return 1 ;;
            esac ;;
        *) return 1 ;;
    esac
}

is_stack() {
    local s
    for s in "${BB_STACKS[@]}"; do [[ "$s" == "$1" ]] && return 0; done
    return 1
}

resolve_stack() {
    case "${1:-}" in
        --engine|engine|index_engine) printf 'index_engine\n' ;;
        *) printf 'Unknown stack selector: %s\n' "${1:-<empty>}" >&2
           printf 'Expected: --engine\n' >&2
           return 1 ;;
    esac
}

# stack_images <stack> — one "key:archive" pair per line.
#
# A single image: the engine is one static Rust binary in a slim runtime. There
# is no separate frontend or database image to build.
stack_images() {
    case "$1" in
        index_engine) printf 'engine:engine.tar.gz\n' ;;
        *) return 1 ;;
    esac
}

# stack_image_tag <stack> <key> <version> — the fully qualified local image tag.
stack_image_tag() {
    local stack="$1" key="$2" version="$3"
    stack_attr "$stack" short >/dev/null || return 1
    printf 'algo-index-%s:%s\n' "$key" "$version"
}

# ── ssh plumbing ────────────────────────────────────────────────────────────
# BB_SSH_ALIAS and BB_SSH_KEY are operator-controlled and become ssh argv, so
# they are charset-validated first: an alias like "-oProxyCommand=..." would
# otherwise smuggle an extra option.
bb_ssh_opts() {
    if [[ ! "$BB_SSH_ALIAS" =~ ^[A-Za-z0-9._-]+$ || "$BB_SSH_ALIAS" == -* ]]; then
        printf 'bb_ssh_opts: unsafe BB_SSH_ALIAS: %s\n' \
            "$(printf '%s' "$BB_SSH_ALIAS" | LC_ALL=C tr -d '\000-\010\013-\037\177')" >&2
        return 1
    fi
    BB_SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=15)
    if [[ -n "${BB_SSH_KEY:-}" ]]; then
        if [[ ! "$BB_SSH_KEY" =~ ^[A-Za-z0-9._/~+-]+$ || "$BB_SSH_KEY" == -* ]]; then
            printf 'bb_ssh_opts: unsafe BB_SSH_KEY path\n' >&2
            return 1
        fi
        BB_SSH_OPTS+=(-i "$BB_SSH_KEY" -o IdentitiesOnly=yes)
    fi
}

bb_ssh() {
    bb_ssh_opts || return 1
    ssh "${BB_SSH_OPTS[@]}" "$BB_SSH_ALIAS" "$@"
}

# assert_safe_remote_dir <path> — refuse a path that could be the filesystem
# root or escape by traversal, so a later rm -rf inside it cannot become
# catastrophic. Depth >= 3 means a truncated variable cannot resolve to a
# top-level directory.
assert_safe_remote_dir() {
    local dir="${1:-}" shown depth
    shown="$(printf '%s' "$dir" | LC_ALL=C tr -d '\000-\010\013-\037\177')"
    [[ -n "$dir" ]]                     || { printf 'Refusing empty remote dir\n' >&2; return 1; }
    [[ "$dir" != *".."* ]]              || { printf 'Refusing remote dir with ..: %s\n' "$shown" >&2; return 1; }
    [[ "$dir" =~ ^/[A-Za-z0-9_./-]+$ ]] || { printf 'Refusing unsafe remote dir: %s\n' "$shown" >&2; return 1; }
    depth="$(printf '%s' "${dir//[!\/]/}" | wc -c)"
    (( depth >= 3 )) || { printf 'Refusing shallow remote dir: %s\n' "$shown" >&2; return 1; }
}
