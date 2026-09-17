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
          health_compose_service: (.health.compose_service // ""),
          health_settle: (.health.settle_seconds // 20 | tostring),
          health_log_pattern: (.health.log_pattern // ""),
          keep_releases: (.retention.keep_releases | tostring),
          keep_masters: (.retention.keep_instrument_masters // 0 | tostring),
          web_enabled: (.web.enabled // false | tostring),
          web_domain: (.web.domain // ""),
          web_probe: (.web.local_probe_url // ""),
          nginx_enabled: (.nginx.enabled // false | tostring),
          nginx_vhost: (.nginx.vhost_file // "")
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
    (
        cd "${P[stack_dir]}" || exit 1
        # Shell variables take precedence over --env-file in Compose. The
        # operator-managed file is the sole authority for this stack's port.
        unset BLACKBOX_HTTP_PORT
        # The container's own environment must come from that same file rather
        # than whatever .env happens to sit beside the compose file, so the
        # backend and its callback URL agree with the ports rendered above.
        BLACKBOX_ENV_FILE="${P[env_file]}" \
        BB_VERSION="${BB_VERSION_FOR_COMPOSE:?BB_VERSION_FOR_COMPOSE not set}" \
          "$(docker_bin)" compose \
            --env-file "${P[env_file]}" \
            --project-name "${P[compose_project]}" \
            --file "${P[compose_file]}" "$@"
    )
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
# it for Compose interpolation and container configuration. Release manifests
# also record its checksum to detect drift.
bb_assert_env() {
    [[ -e "${P[env_file]}" ]] \
        || die "missing ${P[env_file]} — place it yourself, then redeploy"
    ok "env file present (contents not inspected)"
}

# The engine's API can arm and disarm trading strategies, so a port published on
# anything but loopback is an unauthenticated control surface. Access control
# lives at the nginx edge; the container must never be reachable around it.
bb_assert_loopback_only() {
    local exposed
    exposed="$(compose config --format json 2>/dev/null | jq -r '
        (.services // {}) | to_entries[] as $s
        | ($s.value.ports // [])[]
        | select(((.host_ip // "0.0.0.0") | . != "127.0.0.1" and . != "::1"))
        | "\($s.key): \(.host_ip // "0.0.0.0"):\(.published // "?") -> \(.target // "?")"
    ' 2>/dev/null || true)"

    if [[ -n "$exposed" ]]; then
        printf '%s\n' "$exposed" | sed 's/^/     /' >&2
        die "this release would publish the engine beyond loopback — bind it to 127.0.0.1 and put access control at the nginx edge instead"
    fi
    ok "every published port is bound to loopback"
}

# Fail loudly rather than deploying an open control surface. With no edge the
# engine is reachable only from the box itself, which is closed but also means the
# operator UI needs an SSH tunnel; with an edge, that edge must actually restrict.
bb_assert_access_control() {
    if [[ "${P[nginx_enabled]}" != "true" ]]; then
        warn "no nginx edge is configured for this stack"
        warn "the engine is reachable only on VPS loopback — reach the UI with:"
        if [[ -n "${P[health_compose_service]:-}" ]]; then
            local url scheme authority
            url="$(bb_health_http_url)" || die "cannot resolve the engine access port"
            scheme="${url%%://*}"
            authority="${url#*://}"; authority="${authority%%/*}"
            warn "  ssh -N -L ${authority##*:}:$authority <this-host>   then $scheme://$authority"
        else
            warn "  ssh -N -L <local-port>:127.0.0.1:<published-port> <this-host>"
        fi
        return 0
    fi

    local vhost="${P[nginx_vhost]}"
    [[ -n "$vhost" ]] \
        || die "nginx.enabled is true but nginx.vhost_file is not set in paths.json"
    [[ -f "$vhost" ]] \
        || die "nginx.enabled is true but $vhost is not installed — install release_manager/nginx/ first; refusing to deploy an unprotected control surface"

    grep -qE '^[[:space:]]*deny[[:space:]]+all[[:space:]]*;' "$vhost" \
        || die "$vhost has no 'deny all' — refusing to deploy an open control surface"
    grep -qE '^[[:space:]]*allow[[:space:]]+100\.64\.0\.0/10[[:space:]]*;' "$vhost" \
        || die "$vhost does not restrict access to the tailnet — refusing to deploy"

    bb_assert_edge_port "$vhost"

    ok "nginx edge restricts the control surface to the tailnet"
}

# nginx cannot read the env file, so the vhost is the one place a port has to be
# written out a second time. It is therefore verified against the env file rather
# than trusted: a vhost proxying to a port the engine no longer publishes would
# leave the UI dead with every other check passing.
bb_assert_edge_port() {
    local vhost="$1" authority port found

    found="$(grep -oE 'proxy_pass[[:space:]]+https?://[^;[:space:]]+' "$vhost" || true)"

    if [[ -z "$found" ]]; then
        # A redirect-only bookmark vhost proxies nothing, so it names no port and
        # there is nothing to drift. It must actually be a redirect, though.
        grep -qE '^[[:space:]]*return[[:space:]]+30[1-8][[:space:]]' "$vhost" \
            || die "$vhost neither proxies to the engine nor redirects — it would serve nothing"
        ok "nginx edge is a redirect only, so it carries no port to drift"
        return 0
    fi

    authority="$(bb_published_authority)" \
        || die "cannot resolve the published port to check $vhost against"
    port="${authority##*:}"

    local ports wrong=''
    ports="$(printf '%s\n' "$found" | grep -oE ':[0-9]+' | tr -d ':' | sort -u)"
    [[ -n "$ports" ]] \
        || die "$vhost has no proxy_pass with an explicit port — cannot confirm it reaches the engine"

    while IFS= read -r candidate; do
        [[ -z "$candidate" || "$candidate" == "$port" ]] || wrong+=" $candidate"
    done <<< "$ports"

    if [[ -n "$wrong" ]]; then
        die "$vhost does not proxy to the configured backend address: it proxies to port(s)${wrong} but ${P[env_file]} publishes $port — update the vhost to match the env file, which is the only source of truth for the port"
    fi

    ok "nginx edge proxies to $port, matching the env file"
}

bb_validate_compose() {
    compose config >/dev/null 2>&1 || {
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

# bb_snapshot_outgoing_config <dest> <version>
#
# The live compose file is the INCOMING one by the time this script runs: the
# operator machine rsyncs it into place before invoking us. So it must never be
# archived here, or the rollback bundle becomes an old image under a new config.
# deploy.sh snapshots the outgoing compose over SSH before that upload; this
# only checks the snapshot arrived, and fails closed if it did not.
bb_snapshot_outgoing_config() {
    local dest="$1" version="$2"
    if [[ -f "$dest/${P[compose_name]}" ]]; then
        ok "outgoing compose for $version already archived"
        return 0
    fi
    die "no archived compose for the running release $version at $dest/${P[compose_name]} — deploy.sh must snapshot it before uploading the incoming one; refusing to build a mismatched rollback bundle"
}

# bb_release_manifest <dest> <version> <status>
#
# Binds one release together: version, the compose that started it, the digest
# of the env file that configured it, and the image tags. The env file itself is
# never copied - it holds broker credentials and the backup tree is not the
# place for them - so only its digest is recorded, enough to detect drift.
bb_release_manifest() {
    local dest="$1" version="$2" status="$3" tmp compose_sha env_sha images
    compose_sha=''
    env_sha=''
    [[ -f "$dest/${P[compose_name]}" ]] \
        && compose_sha="$(sha256sum "$dest/${P[compose_name]}" | cut -d' ' -f1)"
    [[ -f "${P[env_file]}" ]] \
        && env_sha="$(sha256sum "${P[env_file]}" | cut -d' ' -f1)"
    images="$(bb_images | while IFS=$'\t' read -r key archive; do
        printf '%s\t%s\talgo-index-%s:%s\n' "$key" "$archive" "$key" "$version"
    done)"

    tmp="$(mktemp "$dest/release.json.XXXXXX")" || return 0
    jq -n \
        --arg stack "${P[stack]}" \
        --arg environment "${P[environment]}" \
        --arg version "$version" \
        --arg status "$status" \
        --arg compose_name "${P[compose_name]}" \
        --arg compose_sha256 "$compose_sha" \
        --arg env_sha256 "$env_sha" \
        --arg archived_at "$(date -Is)" \
        --arg images "$images" \
        '{stack: $stack, environment: $environment, version: $version,
          status: $status, compose_name: $compose_name,
          compose_sha256: $compose_sha256, env_sha256: $env_sha256,
          archived_at: $archived_at,
          images: ($images | split("\n") | map(select(length > 0) | split("\t")
                   | {key: .[0], archive: .[1], tag: .[2]}))}' > "$tmp" \
        || { rm -f "$tmp"; return 0; }
    mv "$tmp" "$dest/release.json"
}

# bb_archive_current_images <dest> <version>
#
# Saves the running images beside the compose file that started them, which
# deploy.sh snapshotted before the incoming one landed, so a rollback restores a
# matched pair rather than an old image under a new compose.
bb_archive_current_images() {
    local dest="$1" version="$2" key archive tag
    bb_assert_writable "$dest"
    bb_snapshot_outgoing_config "$dest" "$version"
    while IFS=$'\t' read -r key archive; do
        tag="algo-index-${key}:${version}"
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
    bb_release_manifest "$dest" "$version" archived
    ( cd "$dest" && find . -maxdepth 1 -type f ! -name checksums.sha256 -print0 \
        | sort -z | xargs -0 -r sha256sum > checksums.sha256 ) || true
    ok "outgoing release archived to $dest"
}

# bb_self_archive <version>
#
# Records the release that has just gone live into its own rollback slot: the
# compose now in place really is the one running it, so the pair is coherent by
# construction and the next deploy has nothing to reconstruct.
bb_self_archive() {
    local version="$1"
    [[ -n "$version" ]] || return 0
    local dest="${P[rollback_images]}/$version"
    mkdir -p "$dest" 2>/dev/null || { warn "cannot create $dest"; return 0; }
    cp "${P[compose_file]}" "$dest/${P[compose_name]}" 2>/dev/null \
        || { warn "cannot archive the live compose for $version"; return 0; }
    bb_release_manifest "$dest" "$version" live
    ( cd "$dest" && find . -maxdepth 1 -type f ! -name checksums.sha256 -print0 \
        | sort -z | xargs -0 -r sha256sum > checksums.sha256 ) || true
    ok "release $version archived with the compose that started it"
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
        tag="algo-index-${key}:${version}"
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

# bb_rollback_describe <dir> <version>
#
# States which coherent release is about to be restored, and warns when the live
# env file no longer matches the one that release ran under. The env file is not
# archived, so a rollback cannot undo a credential or setting change - the
# operator has to know that before the stack comes back up.
bb_rollback_describe() {
    local dir="$1" version="$2"
    local manifest="$dir/release.json"
    if [[ ! -f "$manifest" ]]; then
        warn "archive for $version predates release manifests — cannot confirm the image and config belong together"
        return 0
    fi

    local recorded_version compose_sha env_sha archived_at live_compose_sha live_env_sha
    recorded_version="$(jq -r '.version // empty' "$manifest" 2>/dev/null || true)"
    compose_sha="$(jq -r '.compose_sha256 // empty' "$manifest" 2>/dev/null || true)"
    env_sha="$(jq -r '.env_sha256 // empty' "$manifest" 2>/dev/null || true)"
    archived_at="$(jq -r '.archived_at // empty' "$manifest" 2>/dev/null || true)"

    if [[ -n "$recorded_version" && "$recorded_version" != "$version" ]]; then
        die "archive directory $version holds a manifest for $recorded_version — refusing to restore a mismatched bundle"
    fi

    info "restoring release $version archived at ${archived_at:-<unknown>}"

    if [[ -n "$compose_sha" && -f "$dir/${P[compose_name]}" ]]; then
        live_compose_sha="$(sha256sum "$dir/${P[compose_name]}" | cut -d' ' -f1)"
        [[ "$live_compose_sha" == "$compose_sha" ]] \
            || die "the archived compose no longer matches its manifest digest — archive is damaged"
        ok "compose matches the manifest recorded for $version"
    fi

    if [[ -n "$env_sha" && -f "${P[env_file]}" ]]; then
        live_env_sha="$(sha256sum "${P[env_file]}" | cut -d' ' -f1)"
        if [[ "$live_env_sha" != "$env_sha" ]]; then
            warn "the env file has changed since $version was deployed"
            warn "rollback restores the image and compose but NOT credentials or settings — review ${P[env_file]}"
        else
            ok "env file unchanged since $version was deployed"
        fi
    fi
}

# ── health ──────────────────────────────────────────────────────────────────
# Contract-driven: mode=http probes the engine's /health endpoint, mode=container
# asserts the service is still running after a settle window and optionally that
# a log line appeared. Which one applies is declared in paths.json, not here.
# Resolve only published port metadata; never print the rendered environment.
# Older contracts retain their literal URL. New contracts follow the env-driven
# Compose mapping, including when rollback restores an older compose file.
bb_published_authority() {
    local service="${P[health_compose_service]:-}" config authority
    [[ -n "$service" ]] || { warn "no health.compose_service in the path contract"; return 1; }
    config="$(compose config --format json 2>/dev/null)" \
        || { warn "cannot render Compose health mapping"; return 1; }
    authority="$(jq -er --arg service "$service" '
        .services[$service].ports
        | if length == 1 then .[0] else error("expected one HTTP mapping") end
        | select((.protocol // "tcp") == "tcp")
        | select(.host_ip == "127.0.0.1" or .host_ip == "::1")
        | (.published | tostring) as $port
        | select($port | test("^[0-9]+$"))
        | select(($port | tonumber) >= 1 and ($port | tonumber) <= 65535)
        | if .host_ip == "::1" then "[::1]:" + $port else .host_ip + ":" + $port end
    ' <<< "$config" 2>/dev/null)" \
        || { warn "health service must publish exactly one TCP port on loopback"; return 1; }
    printf '%s\n' "$authority"
}

bb_health_http_url() {
    local url="${P[health_http_url]}" service="${P[health_compose_service]:-}" authority
    [[ -n "$service" ]] || { printf '%s\n' "$url"; return 0; }
    [[ "$url" =~ ^https?://[^/]+/ ]] || { warn "invalid health HTTP URL in paths.json"; return 1; }
    authority="$(bb_published_authority)" || return 1
    printf '%s://%s/%s\n' "${url%%://*}" "$authority" "${url#*://*/}"
}

bb_health_gate() {
    local settle="${P[health_settle]:-20}"
    case "${P[health_mode]}" in
        http)
            local url
            url="$(bb_health_http_url)" || return 1
            bb_health_http "$url" "$settle" ;;
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

# The public edge, checked over loopback so it needs neither DNS nor an open
# security group. A failure here does not fail the deploy: the engine is the
# critical service and must not be rolled back because a static page is down.
bb_check_web() {
    [[ "${P[web_enabled]}" == "true" ]] || return 0
    [[ -n "${P[web_probe]}" ]] || return 0
    require_cmds curl

    local waited=0
    while (( waited < 20 )); do
        if curl -fsS -m 3 -o /dev/null "${P[web_probe]}" 2>/dev/null; then
            ok "web edge serving on ${P[web_probe]}"
            local tls
            tls="$("$(docker_bin)" exec bb-web sh -c 'ls /data/caddy/certificates 2>/dev/null | head -1' 2>/dev/null || true)"
            if [[ -n "$tls" ]]; then
                ok "TLS certificates present for ${P[web_domain]}"
            else
                warn "no TLS certificate yet for ${P[web_domain]}"
                warn "Let's Encrypt needs inbound :80 — open it in the EC2 security group"
            fi
            return 0
        fi
        sleep 2
        waited=$(( waited + 2 ))
    done
    warn "web edge did not respond on ${P[web_probe]} within 20s (engine unaffected)"
    return 0
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
        rm -rf -- "${dir:?}/${victim:?}" && info "pruned rollback $victim"
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
