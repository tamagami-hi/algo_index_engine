#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# _bb_deploy.sh — the generic VPS-native deployment flow.
#
# Runs ON THE VPS. Sourced by <stack>_deploy.sh, which supplies only identity and
# policy. Everything else is driven by paths.json, so one implementation serves
# every stack.
#
# Contract with the operator machine: by the time this runs, deploy.sh has placed
# manifest.json, paths.json, checksums.sha256, the compose file and
# images/*.tar.gz into the stack directory. This script owns every docker command
# from here on; deploy.sh runs none.
# ─────────────────────────────────────────────────────────────────────────────

bb_deploy_main() {
    local paths_file="$1"; shift

    local ASSUME_YES=false SKIP_CHECKS=false FORCE=false
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --yes|-y)      ASSUME_YES=true; shift ;;
            --skip-checks) SKIP_CHECKS=true; shift ;;
            --force)       FORCE=true; shift ;;
            --help|-h)     bb_deploy_usage; return 0 ;;
            *) printf 'Unknown argument: %s\n' "$1" >&2; bb_deploy_usage >&2; return 1 ;;
        esac
    done

    require_cmds jq sha256sum gzip find sort
    bb_load_paths "$paths_file"
    P[paths_file]="$paths_file"

    printf '\n%s═══ blackbox deploy · %s (%s) ═══%s\n' \
        "$_c_bold" "${P[stack]}" "${P[environment]}" "$_c_rst"

    step "1/12 acquire deployment lock"
    bb_lock

    step "2/12 verify release directory"
    [[ -d "${P[stack_dir]}" ]]     || die "stack directory missing: ${P[stack_dir]}"
    [[ -f "${P[compose_file]}" ]]  || die "compose file missing: ${P[compose_file]}"
    [[ -f "${P[manifest_file]}" ]] || die "manifest.json missing — ship a bundle first"
    ok "release directory present"

    step "3/12 verify backup and log tree"
    bb_assert_backup_tree
    bb_open_log deploy

    step "4/12 check disk space"
    local need; need="$(bb_required_space_mib)"
    bb_assert_space "${P[stack_dir]}" "$need"

    step "5/12 reconcile versions"
    local current incoming
    current="$(bb_current_version)"
    incoming="$(bb_incoming_version)"
    info "currently deployed: ${current:-<none>}"
    info "incoming release:   $incoming"
    if [[ -n "$current" && "$current" == "$incoming" ]]; then
        if [[ "$FORCE" == true ]]; then
            warn "$incoming is already deployed — proceeding because --force was given"
        else
            die "$incoming is already deployed (use --force to redeploy)"
        fi
    fi

    step "6/12 verify checksums"
    bb_verify_checksums

    step "7/12 verify docker and environment"
    bb_assert_docker
    bb_assert_env
    BB_VERSION_FOR_COMPOSE="$incoming"
    bb_validate_compose

    if [[ "${BB_REQUIRE_CONFIRM:-false}" == true && "$ASSUME_YES" != true ]]; then
        if [[ -t 0 ]]; then
            local reply
            printf '\n%s  ➜ Deploy %s to %s? [y/N] %s' \
                "$_c_bold" "$incoming" "${P[environment]}" "$_c_rst"
            read -r reply || reply=''
            [[ "$reply" == [yY] || "$reply" == [yY][eE][sS] ]] \
                || { warn "aborted by operator"; return 0; }
        else
            die "this stack needs --yes when running non-interactively"
        fi
    fi

    step "8/12 archive the outgoing release"
    local rb_dir=''
    if [[ -n "$current" ]]; then
        rb_dir="${P[rollback_images]}/$current"
        bb_archive_current_images "$rb_dir" "$current"
    else
        info "first deploy — nothing to preserve"
    fi

    step "9/12 load new images"
    bb_load_images
    bb_assert_images_present "$incoming"

    step "10/12 start the stack"
    # --remove-orphans so a service renamed between releases does not leave a
    # stray container holding the old image and the same volume.
    compose up -d --remove-orphans || bb_deploy_fail "$current" "$incoming" "compose up failed"
    compose ps

    step "11/12 health gate"
    local healthy=true
    if [[ "$SKIP_CHECKS" == true ]]; then
        warn "health gate SKIPPED by flag — this version is recorded unverified"
    else
        bb_health_gate || healthy=false
    fi
    if [[ "$healthy" != true ]]; then
        bb_capture_app_log "failed-$incoming"
        bb_deploy_fail "$current" "$incoming" "health gate failed"
    fi

    bb_write_version "$incoming" "$current" active
    bb_update_registry "$incoming"

    step "12/12 retention"
    # Last, deliberately: a failure above must never destroy a rollback target.
    bb_prune_rollbacks "${P[rollback_images]}" "${P[keep_releases]}"
    bb_prune_instrument_masters

    bb_check_web

    bb_summary "Deployment complete" \
        "stack=${P[stack]}" \
        "environment=${P[environment]}" \
        "version=$incoming" \
        "previous=${current:-<none>}" \
        "project=${P[compose_project]}" \
        "log=${BB_LOG_FILE:-<none>}"
}

bb_deploy_usage() {
    cat <<'USAGE'
Deploy the staged release in this directory.

  --yes, -y       skip the confirmation prompt
  --skip-checks   start the stack but do not gate on health
  --force         redeploy even if this version is already active
  --help, -h      this message

manifest.json, paths.json, checksums.sha256, the compose file and
images/*.tar.gz must already be present; the operator machine's deploy.sh
places them here.
USAGE
}

# bb_deploy_fail <previous> <attempted> <reason>
#
# Marks the release failed and attempts an image-level rollback. Safe to do
# automatically here in a way it is not for a stateful service: the engine owns
# no database, and its data volume holds only the session token and dated
# instrument masters, which any version reads. The volume is never touched.
bb_deploy_fail() {
    local previous="$1" attempted="$2" reason="$3"
    warn "deployment failed: $reason"
    compose logs --tail 40 2>/dev/null || true

    if [[ -z "$previous" ]]; then
        bb_write_version "" "" failed "$attempted"
        die "deployment failed and there is no previous version to restore"
    fi

    local rb="${P[rollback_images]}/$previous"
    if [[ ! -d "$rb" ]]; then
        bb_write_version "$previous" "" failed "$attempted"
        die "deployment failed and no rollback archive exists at $rb"
    fi

    step "AUTO-ROLLBACK to $previous"
    bb_rollback_verify "$rb"

    local key archive path
    while IFS=$'\t' read -r key archive; do
        path="$rb/$archive"
        if [[ ! -f "$path" ]]; then
            bb_write_version "$previous" "" failed "$attempted"
            die "rollback archive is missing $archive — auto-rollback aborted"
        fi
        if ! gzip -dc "$path" | "$(docker_bin)" image load >/dev/null 2>&1; then
            bb_write_version "$previous" "" failed "$attempted"
            die "rollback archive $archive would not load — auto-rollback aborted"
        fi
        info "restored $archive"
    done < <(bb_images)

    [[ -f "$rb/${P[compose_name]}" ]] && cp "$rb/${P[compose_name]}" "${P[compose_file]}"

    BB_VERSION_FOR_COMPOSE="$previous"
    if compose up -d --remove-orphans >/dev/null 2>&1 && bb_health_gate; then
        bb_write_version "$previous" "$attempted" rolled-back
        bb_update_registry "$previous"
        die "deployment failed ($reason) — automatically rolled back to $previous"
    fi

    bb_write_version "$previous" "" failed "$attempted"
    die "deployment failed ($reason) AND automatic rollback did not come up healthy — manual intervention required"
}
