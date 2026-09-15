#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# _bb_rollback.sh — the generic VPS-native rollback flow.
#
# Runs ON THE VPS. Restores a previously archived release: its images and the
# compose file that started them, as a matched pair.
#
# The data volume is never touched. It holds the Dhan session token and the
# dated instrument masters, both of which any version reads, and discarding them
# would force a fresh browser login on a headless box.
# ─────────────────────────────────────────────────────────────────────────────

bb_rollback_main() {
    local paths_file="$1"; shift

    local TARGET='' ASSUME_YES=false SKIP_CHECKS=false LIST_ONLY=false
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --to)          TARGET="${2:-}"; shift 2 ;;
            --list)        LIST_ONLY=true; shift ;;
            --yes|-y)      ASSUME_YES=true; shift ;;
            --skip-checks) SKIP_CHECKS=true; shift ;;
            --help|-h)     bb_rollback_usage; return 0 ;;
            *) printf 'Unknown argument: %s\n' "$1" >&2; bb_rollback_usage >&2; return 1 ;;
        esac
    done

    require_cmds jq sha256sum gzip find sort
    bb_load_paths "$paths_file"
    P[paths_file]="$paths_file"

    printf '\n%s═══ blackbox rollback · %s ═══%s\n' "$_c_bold" "${P[stack]}" "$_c_rst"

    local current available
    current="$(bb_current_version)"
    available="$(find "${P[rollback_images]}" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort || true)"

    if [[ "$LIST_ONLY" == true ]]; then
        printf '\n   current: %s\n\n   available rollback targets:\n' "${current:-<none>}"
        [[ -n "$available" ]] && printf '     %s\n' $available || printf '     <none>\n'
        printf '\n'
        return 0
    fi

    [[ -n "$available" ]] || die "no rollback archives under ${P[rollback_images]}"

    if [[ -z "$TARGET" ]]; then
        # Newest archived release that is not the one running.
        TARGET="$(printf '%s\n' $available | grep -vx -- "${current:-}" | tail -n1 || true)"
        [[ -n "$TARGET" ]] || die "no rollback target other than the running version"
        info "no --to given; selecting $TARGET"
    fi

    [[ "$TARGET" != "$current" ]] || die "$TARGET is already the running version"

    local rb="${P[rollback_images]}/$TARGET"
    [[ -d "$rb" ]] || die "no rollback archive for $TARGET at $rb"

    bb_lock
    bb_open_log deploy
    bb_assert_docker

    step "1/6 verify the rollback archive"
    bb_rollback_verify "$rb"

    step "2/6 confirm"
    if [[ "$ASSUME_YES" != true ]]; then
        if [[ -t 0 ]]; then
            local reply
            printf '\n%s  ➜ Roll %s back to %s? [y/N] %s' \
                "$_c_bold" "${current:-<none>}" "$TARGET" "$_c_rst"
            read -r reply || reply=''
            [[ "$reply" == [yY] || "$reply" == [yY][eE][sS] ]] \
                || { warn "aborted by operator"; return 0; }
        else
            die "rollback needs --yes when running non-interactively"
        fi
    fi

    step "3/6 load the archived images"
    local key archive path
    while IFS=$'\t' read -r key archive; do
        path="$rb/$archive"
        [[ -f "$path" ]] || die "rollback archive is missing $archive"
        gzip -dc "$path" | "$(docker_bin)" image load >/dev/null \
            || die "failed to load $archive from the rollback archive"
        info "restored $archive"
    done < <(bb_images)

    step "4/6 restore the matching compose file"
    if [[ -f "$rb/${P[compose_name]}" ]]; then
        cp "$rb/${P[compose_name]}" "${P[compose_file]}"
        ok "compose file restored from the archive"
    else
        warn "archive has no compose file — keeping the current one"
    fi

    step "5/6 start the restored release"
    BB_VERSION_FOR_COMPOSE="$TARGET"
    bb_validate_compose
    compose up -d --remove-orphans || die "failed to start the restored release"
    compose ps

    step "6/6 health gate"
    if [[ "$SKIP_CHECKS" == true ]]; then
        warn "health gate SKIPPED by flag"
    elif ! bb_health_gate; then
        bb_capture_app_log "rollback-failed-$TARGET"
        bb_write_version "$TARGET" "$current" unhealthy
        die "rolled back to $TARGET but it did not come up healthy — manual intervention required"
    fi

    bb_write_version "$TARGET" "$current" rolled-back
    bb_update_registry "$TARGET"

    bb_summary "Rollback complete" \
        "stack=${P[stack]}" \
        "restored=$TARGET" \
        "was=${current:-<none>}" \
        "log=${BB_LOG_FILE:-<none>}"
}

bb_rollback_usage() {
    cat <<'USAGE'
Restore a previously archived release.

  --to VERSION    which release to restore (default: newest archived, not current)
  --list          show the current version and available targets, change nothing
  --yes, -y       skip the confirmation prompt
  --skip-checks   start the release but do not gate on health
  --help, -h      this message

The data volume is never touched: the session token and dated instrument
masters survive a rollback untouched.
USAGE
}
