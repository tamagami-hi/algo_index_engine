#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# _bb_lib.sh — shared VPS-native runtime library.
#
# Runs ON THE VPS. Sourced by the native deploy and rollback scripts. Everything
# it does is driven by paths.json, so one implementation serves every stack and
# they cannot drift apart.
#
# `docker`, never `sudo docker`: the deploy user is in the docker group.
# ─────────────────────────────────────────────────────────────────────────────

if [[ -t 1 ]]; then
    _c_rst=$'\033[0m'; _c_bold=$'\033[1m'; _c_dim=$'\033[2m'
    _c_red=$'\033[31m'; _c_grn=$'\033[32m'; _c_ylw=$'\033[33m'; _c_cyn=$'\033[36m'
else
    _c_rst=''; _c_bold=''; _c_dim=''; _c_red=''; _c_grn=''; _c_ylw=''; _c_cyn=''
fi

declare -A P=()
BB_LOG_FILE=''

step() { printf '\n%s→%s %s\n' "$_c_cyn" "$_c_rst" "$1"; log "STEP $1"; }
ok()   { printf '   %s✓%s %s\n' "$_c_grn" "$_c_rst" "$1"; log "OK $1"; }
warn() { printf '   %s!%s %s\n' "$_c_ylw" "$_c_rst" "$1" >&2; log "WARN $1"; }
info() { printf '     %s%s%s\n' "$_c_dim" "$1" "$_c_rst"; log "INFO $1"; }
die()  { printf '   %s✗%s %s\n' "$_c_red" "$_c_rst" "$1" >&2; log "FAIL $1"; exit 1; }

log() {
    [[ -n "$BB_LOG_FILE" ]] || return 0
    printf '%s %s\n' "$(date -Is)" "$1" >> "$BB_LOG_FILE" 2>/dev/null || true
}

require_cmds() {
    local c
    for c in "$@"; do command -v "$c" >/dev/null || die "$c is required on this host"; done
}

# ── contract ────────────────────────────────────────────────────────────────
# bb_load_paths <paths.json> — read the contract into P[].
#
# Read once into an associative array so a later step cannot re-read a file that
# changed underneath it mid-deploy.
bb_load_paths() {
    local file="$1" key value
    [[ -f "$file" ]] || die "path contract missing: $file"
    require_cmds jq
    jq empty "$file" 2>/dev/null || die "path contract is not valid JSON: $file"

    [[ "$(jq -r '.schema // 0' "$file")" == "1" ]] || die "unsupported path contract schema"

    while IFS=$'\t' read -r key value; do
        P["$key"]="$value"
    done < <(jq -r '
        {
          stack: .stack, environment: .environment, short: .short,
          root: .vps.root, stack_dir: .vps.stack_dir,
          images_dir: .vps.images_dir,
          compose_file: .vps.compose_file, compose_name: .vps.compose_name,
          env_file: .vps.env_file, env_example: .vps.env_example,
          version_file: .vps.version_file, version_name: .vps.version_name,
          manifest_file: .vps.manifest_file, checksums_file: .vps.checksums_file,
          registry: .vps.registry, data_volume: .vps.data_volume,
          docker: .vps.docker, container_prefix: .vps.container_prefix,
          compose_project: .vps.compose_project, lock_file: .vps.lock_file,
          backup_root: .backup.root,
          rollback_root: .backup.rollback_root,
          rollback_images: .backup.rollback_images,
          logs_root: .backup.logs_root,
          deploy_log: .backup.deploy_log, app_log: .backup.app_log,
          has_database: (.has_database | tostring),
          health_mode: .health.mode,
          health_http_url: (.health.http_url // ""),
          health_settle: (.health.settle_seconds // 20 | tostring),
          health_log_pattern: (.health.log_pattern // ""),
          keep_releases: (.retention.keep_releases | tostring),
          keep_masters: (.retention.keep_instrument_masters // 0 | tostring)
        } | to_entries[] | [.key, (.value // "")] | @tsv' "$file")

    [[ -n "${P[stack_dir]}" ]] || die "contract has no vps.stack_dir"
    bb_assert_safe_dir "${P[stack_dir]}"
    bb_assert_safe_dir "${P[rollback_images]}"
}

# Refuse a path that could be the filesystem root or escape by traversal. The
# rollback tree gets rm -rf'd during pruning, so this is load-bearing.
bb_assert_safe_dir() {
    local dir="${1:-}" depth
    [[ -n "$dir" ]]                     || die "refusing an empty path"
    [[ "$dir" == /* ]]                  || die "refusing a relative path: $dir"
    [[ "$dir" != *".."* ]]              || die "refusing a path with ..: $dir"
    [[ "$dir" =~ ^/[A-Za-z0-9_./-]+$ ]] || die "refusing an unsafe path: $dir"
    depth="$(printf '%s' "${dir//[!\/]/}" | wc -c)"
    (( depth >= 3 )) || die "refusing a shallow path: $dir"
}

docker_bin() { printf '%s\n' "${P[docker]:-docker}"; }

# compose <args...> — docker compose, always with this stack's project and file.
compose() {
    ( cd "${P[stack_dir]}" && \
      BB_VERSION="${BB_VERSION_FOR_COMPOSE:?BB_VERSION_FOR_COMPOSE not set}" \
      "$(docker_bin)" compose \
        --project-name "${P[compose_project]}" \
        --file "${P[compose_file]}" "$@" )
}

# ── locking ─────────────────────────────────────────────────────────────────
# One lock shared by deploy and rollback, so the two can never interleave.
bb_lock() {
    require_cmds flock
    exec {BB_LOCK_FD}>"${P[lock_file]}" \
        || die "cannot open lock file ${P[lock_file]}"
    flock -n "$BB_LOCK_FD" \
        || die "another deploy or rollback holds ${P[lock_file]}"
    ok "acquired ${P[lock_file]}"
}

# ── logging ─────────────────────────────────────────────────────────────────
bb_open_log() {
    local kind="$1" dir
    case "$kind" in
        deploy) dir="${P[deploy_log]}" ;;
        *)      dir="${P[app_log]}" ;;
    esac
    mkdir -p "$dir" 2>/dev/null || true
    BB_LOG_FILE="$dir/$(date -u +%Y%m%dT%H%M%SZ)-$kind.log"
    : > "$BB_LOG_FILE" 2>/dev/null || BB_LOG_FILE=''
    [[ -n "$BB_LOG_FILE" ]] && ok "logging to $BB_LOG_FILE"
}

# ── preconditions ───────────────────────────────────────────────────────────
bb_assert_writable() {
    local dir
    for dir in "$@"; do
        [[ -n "$dir" ]] || continue
        mkdir -p "$dir" 2>/dev/null || die "cannot create $dir"
        [[ -w "$dir" ]] || die "not writable: $dir"
    done
}

# This host has a single root volume and no separate backup mount, so there is
# no mountpoint to assert. Writability is the real requirement; if a dedicated
# volume is attached later, add the mountpoint check to the contract rather than
# hardcoding it here.
bb_assert_backup_tree() {
    bb_assert_writable "${P[backup_root]}" "${P[rollback_root]}" \
                       "${P[rollback_images]}" "${P[logs_root]}" \
                       "${P[deploy_log]}" "${P[app_log]}"
    ok "backup and log tree writable"
}

# bb_assert_space <dir> <need MiB>
bb_assert_space() {
    local dir="$1" need="$2" avail
    avail="$(df -BM --output=avail "$dir" 2>/dev/null | tail -n1 | tr -dc '0-9')"
    [[ -n "$avail" ]] || { warn "cannot determine free space on $dir"; return 0; }
    (( avail >= need )) || die "insufficient space on $dir: ${avail}MiB free, need ${need}MiB"
    info "$dir has ${avail}MiB free (need ${need}MiB)"
}

# Twice the incoming archive size, so the outgoing release can be archived
# alongside it before the new one is loaded.
bb_required_space_mib() {
    local total=0 size
    if [[ -d "${P[images_dir]}" ]]; then
        total="$(du -sm "${P[images_dir]}" 2>/dev/null | cut -f1 || echo 0)"
    fi
    size=$(( total * 2 + 512 ))
    printf '%s\n' "$size"
}

bb_assert_docker() {
    "$(docker_bin)" info >/dev/null 2>&1 || die "docker is not usable by $(id -un)"
    "$(docker_bin)" compose version >/dev/null 2>&1 || die "docker compose plugin missing"
    ok "docker and compose available"
}

# The engine will not start without credentials, so a missing env file is worth
# failing fast on. That is the ONLY thing this checks.
#
# .env belongs entirely to the operator. Nothing in this pipeline reads its
# contents, writes it, changes its mode, or deletes it: deploy.sh excludes it
# from rsync and runs without --delete, and no script greps it. Only Docker reads
# it, via env_file in the compose file, at container start.
bb_assert_env() {
    [[ -e "${P[env_file]}" ]] \
        || die "missing ${P[env_file]} — place it yourself, then redeploy"
    ok "env file present (contents not inspected)"
}

bb_validate_compose() {
    compose config >/dev/null 2>&1 || {
        compose config 2>&1 | tail -20 >&2
        die "compose file is not valid for this release"
    }
    ok "compose file validates"
}

# ── version bookkeeping ─────────────────────────────────────────────────────
bb_current_version() {
    [[ -f "${P[version_file]}" ]] || return 0
    jq -r '.version // empty' "${P[version_file]}" 2>/dev/null || true
}

bb_incoming_version() {
    [[ -f "${P[manifest_file]}" ]] || die "manifest.json missing — has deploy.sh shipped a bundle?"
    local v; v="$(jq -r '.version // empty' "${P[manifest_file]}" 2>/dev/null || true)"
    [[ -n "$v" ]] || die "manifest.json has no version"
    printf '%s\n' "$v"
}

# bb_write_version <version> <previous> <status> [attempted]
#
# A failed deploy must not rewrite history: .version keeps pointing at what is
# actually running and the failed release is recorded separately, so a retry is
# never told the failed version is already deployed.
bb_write_version() {
    local version="$1" previous="$2" status="$3" attempted="${4:-}" tmp
    tmp="$(mktemp "${P[version_file]}.XXXXXX")" || die "cannot write version file"
    jq -n --arg v "$version" --arg p "$previous" --arg s "$status" \
          --arg a "$attempted" --arg at "$(date -Is)" --arg stack "${P[stack]}" \
        '{stack: $stack, version: $v, previous: $p, status: $s,
          last_attempted: $a, updated_at: $at}' > "$tmp" \
        || die "cannot serialise version file"
    mv "$tmp" "${P[version_file]}"
    ok "recorded $status: ${version:-<none>}"
}

bb_update_registry() {
    local version="$1" tmp
    [[ -n "${P[registry]}" ]] || return 0
    [[ -s "${P[registry]}" ]] || printf '{}\n' > "${P[registry]}" 2>/dev/null || return 0
    tmp="$(mktemp "${P[registry]}.XXXXXX")" || return 0
    jq --arg stack "${P[stack]}" --arg v "$version" --arg at "$(date -Is)" \
       '.[$stack] = {version: $v, deployed_at: $at}' "${P[registry]}" > "$tmp" \
        && mv "$tmp" "${P[registry]}" || rm -f "$tmp"
}

# ── images ──────────────────────────────────────────────────────────────────
# bb_images — one "key<TAB>archive" line per declared image.
bb_images() {
    jq -r '.images[] | [.key, .archive] | @tsv' "${P[paths_file]:-${P[stack_dir]}/paths.json}"
}

bb_verify_checksums() {
    [[ -f "${P[checksums_file]}" ]] || die "checksums.sha256 missing"
    ( cd "${P[stack_dir]}" && sha256sum -c --quiet "${P[checksums_file]}" ) \
        || die "checksum verification failed — the upload is corrupt, re-ship"
    ok "checksums verified on arrival"
}

# bb_archive_current_images <dest> <version>
#
# Saves the running images and the compose file that started them, so a rollback
# restores a matched pair rather than an old image under a new compose.
bb_archive_current_images() {
    local dest="$1" version="$2" key archive tag
    bb_assert_writable "$dest"
    while IFS=$'\t' read -r key archive; do
        tag="blackbox-${P[short]}-${key}:${version}"
        if ! "$(docker_bin)" image inspect "$tag" >/dev/null 2>&1; then
            warn "running image $tag not present locally — cannot archive it"
            continue
        fi
        if [[ -f "$dest/$archive" ]]; then
            info "already archived: $archive"
            continue
        fi
        "$(docker_bin)" save "$tag" | gzip -n > "$dest/$archive" \
            || die "failed to archive $tag"
        info "archived $archive"
    done < <(bb_images)
    [[ -f "${P[compose_file]}" ]] && cp "${P[compose_file]}" "$dest/${P[compose_name]}"
    ( cd "$dest" && find . -maxdepth 1 -type f ! -name checksums.sha256 -print0 \
        | sort -z | xargs -0 -r sha256sum > checksums.sha256 ) || true
    ok "outgoing release archived to $dest"
}

bb_load_images() {
    local key archive path
    while IFS=$'\t' read -r key archive; do
        path="${P[images_dir]}/$archive"
        [[ -f "$path" ]] || die "image archive missing: $path"
        gzip -dc "$path" | "$(docker_bin)" image load >/dev/null \
            || die "failed to load $archive"
        info "loaded $archive"
    done < <(bb_images)
    ok "images loaded"
}

bb_assert_images_present() {
    local version="$1" key archive tag
    while IFS=$'\t' read -r key archive; do
        tag="blackbox-${P[short]}-${key}:${version}"
        "$(docker_bin)" image inspect "$tag" >/dev/null 2>&1 \
            || die "image $tag is not present after load"
    done < <(bb_images)
    ok "expected image tags present for $version"
}

bb_rollback_verify() {
    local dir="$1"
    [[ -d "$dir" ]] || die "no rollback archive at $dir"
    [[ -f "$dir/checksums.sha256" ]] || { warn "rollback archive has no checksums"; return 0; }
    ( cd "$dir" && sha256sum -c --quiet checksums.sha256 ) \
        || die "rollback archive at $dir failed checksum verification"
    ok "rollback archive verified"
}

# ── health ──────────────────────────────────────────────────────────────────
# The engine currently exposes no HTTP surface, so "healthy" cannot mean a 200.
# Contract-driven: mode=container asserts the service is still running after a
# settle window and optionally that a log line appeared; mode=http probes a URL
# once one exists. The gap is declared in paths.json rather than hidden here.
bb_health_gate() {
    local settle="${P[health_settle]:-20}"
    case "${P[health_mode]}" in
        http) bb_health_http "${P[health_http_url]}" "$settle" ;;
        *)    bb_health_container "$settle" ;;
    esac
}

bb_health_container() {
    local settle="$1" state waited=0 name
    name="${P[container_prefix]}"

    info "letting the engine settle for ${settle}s"
    while (( waited < settle )); do
        state="$("$(docker_bin)" inspect -f '{{.State.Status}}' "$name" 2>/dev/null || echo missing)"
        case "$state" in
            running) : ;;
            missing) sleep 1; waited=$(( waited + 1 )); continue ;;
            *) warn "container $name is $state"; return 1 ;;
        esac
        sleep 1
        waited=$(( waited + 1 ))
    done

    state="$("$(docker_bin)" inspect -f '{{.State.Status}}' "$name" 2>/dev/null || echo missing)"
    [[ "$state" == running ]] || { warn "container $name is $state after ${settle}s"; return 1; }

    local restarts
    restarts="$("$(docker_bin)" inspect -f '{{.RestartCount}}' "$name" 2>/dev/null || echo 0)"
    (( restarts == 0 )) || { warn "container restarted $restarts time(s) during settle"; return 1; }
    ok "container running, no restarts in ${settle}s"

    if [[ -n "${P[health_log_pattern]}" ]]; then
        if "$(docker_bin)" logs --tail 200 "$name" 2>&1 | grep -qF "${P[health_log_pattern]}"; then
            ok "log gate matched: ${P[health_log_pattern]}"
        else
            warn "log gate did NOT match: ${P[health_log_pattern]}"
            return 1
        fi
    fi
}

bb_health_http() {
    local url="$1" settle="$2" waited=0
    require_cmds curl
    while (( waited < settle )); do
        curl -fsS -m 3 -o /dev/null "$url" 2>/dev/null && { ok "health endpoint responded"; return 0; }
        sleep 2
        waited=$(( waited + 2 ))
    done
    warn "health endpoint did not respond within ${settle}s: $url"
    return 1
}

bb_capture_app_log() {
    local label="$1" dest
    dest="${P[app_log]}/$(date -u +%Y%m%dT%H%M%SZ)-$label.log"
    mkdir -p "${P[app_log]}" 2>/dev/null || return 0
    "$(docker_bin)" logs --tail 400 "${P[container_prefix]}" > "$dest" 2>&1 || true
    [[ -s "$dest" ]] && info "container log captured to $dest"
}

# ── retention ───────────────────────────────────────────────────────────────
# bb_prune_rollbacks <dir> <keep>
#
# Sorted by name, which is the version string, and only ever removes immediate
# subdirectories of a path already proven safe by bb_assert_safe_dir.
bb_prune_rollbacks() {
    local dir="$1" keep="$2" count victim
    [[ -d "$dir" ]] || return 0
    bb_assert_safe_dir "$dir"
    count="$(find "$dir" -mindepth 1 -maxdepth 1 -type d | wc -l)"
    (( count > keep )) || return 0
    while IFS= read -r victim; do
        [[ -n "$victim" ]] || continue
        rm -rf -- "$dir/$victim" && info "pruned rollback $victim"
    done < <(find "$dir" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
             | sort | head -n $(( count - keep )))
}

# Dated instrument masters accumulate at roughly 34 MiB per trading day inside
# the data volume, which would fill a 23 GiB root disk in under a year.
bb_prune_instrument_masters() {
    local keep="${P[keep_masters]:-0}"
    (( keep > 0 )) || return 0
    "$(docker_bin)" run --rm -v "${P[data_volume]}:/data" \
        --entrypoint sh alpine:3.22 -c "
            cd /data/instruments 2>/dev/null || exit 0
            ls -1 *.csv 2>/dev/null | sort | head -n -$keep | while read -r f; do
                rm -f -- \"\$f\" && echo \"pruned \$f\"
            done
        " 2>/dev/null | while IFS= read -r line; do info "$line"; done || true
}

bb_summary() {
    local title="$1"; shift
    printf '\n%s══ %s ══%s\n' "$_c_bold" "$title" "$_c_rst"
    local pair
    for pair in "$@"; do printf '     %-14s %s\n' "${pair%%=*}" "${pair#*=}"; done
    printf '\n'
    log "SUMMARY $title $*"
}
