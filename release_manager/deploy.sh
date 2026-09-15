#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# deploy.sh — SHIP stage. Runs on this computer; deploys nothing itself.
#
# The division of labour is deliberate and strict:
#
#   THIS SCRIPT                      THE VPS-NATIVE SCRIPT
#   ──────────────────────────       ──────────────────────────────────
#   verify the bundle                acquire the deploy lock
#   verify the VPS is reachable      verify checksums again, on arrival
#   verify remote preconditions      archive the outgoing release
#   upload artifacts + scripts       docker load / compose up
#   invoke <stack>_deploy.sh  ───▶   health-gate and record the version
#   stream its output back           prune old rollbacks
#
# This script runs NO docker command against the VPS. Every container operation
# happens inside the VPS-native script, which is the only thing that touches the
# docker daemon. That is a hard requirement, not a stylistic choice: it keeps one
# place responsible for the live system, and that place is the one holding the
# lock.
#
# Transfer is rsync over SSH, writing ONLY the files this pipeline owns, with no
# --delete. Nothing removes a remote file it did not put there, so .env, the data
# volume and any sibling stack are untouched.
#
# Usage:
#   ./release_manager/deploy.sh --engine
#   ./release_manager/deploy.sh --engine --ship-only
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RM_DIR="$ROOT_DIR/release_manager"
BUILD_DIR="$RM_DIR/build"

# shellcheck source=lib/ui.sh
source "$RM_DIR/lib/ui.sh"
# shellcheck source=lib/version.sh
source "$RM_DIR/lib/version.sh"
# shellcheck source=lib/stacks.sh
source "$RM_DIR/lib/stacks.sh"
# shellcheck source=lib/paths.sh
source "$RM_DIR/lib/paths.sh"

STACK=''
BUNDLE_ARG=''
SHIP_ONLY=false
ASSUME_YES=false
REMOTE_ARGS=()

usage() {
    cat <<'USAGE'
Usage: ./release_manager/deploy.sh --engine [options]

Uploads the latest staged bundle to the VPS and then runs the stack's native
deploy script there. All docker work happens on the VPS.

Options:
  --bundle DIR    ship a specific bundle instead of the newest
  --ship-only     upload the artifacts but do not run the remote deploy
  --yes, -y       skip local confirmation, and pass --yes to the remote script
  --force         pass --force to the remote script (redeploy the same version)
  --skip-checks   pass --skip-checks to the remote script
  --help, -h      this message

Requires that export.sh has already staged a bundle.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine|engine|algo_engine) STACK="$(resolve_stack "$1")" || exit 1; shift ;;
        --bundle)      BUNDLE_ARG="${2:-}"; shift 2 ;;
        --ship-only)   SHIP_ONLY=true; shift ;;
        --yes|-y)      ASSUME_YES=true; REMOTE_ARGS+=(--yes); shift ;;
        --force)       REMOTE_ARGS+=(--force); shift ;;
        --skip-checks) REMOTE_ARGS+=(--skip-checks); shift ;;
        --help|-h)     usage; exit 0 ;;
        *) err "unknown argument: $1"; usage >&2; exit 1 ;;
    esac
done

[[ -n "$STACK" ]] || { err "a stack is required: --engine"; usage >&2; exit 1; }

for c in ssh rsync jq sha256sum; do
    command -v "$c" >/dev/null || { err "$c is required"; exit 1; }
done

# The tracked contract is the sole path authority. Validate it, then read every
# remote location from it — nothing is derived here.
PATHS_FILE="$(stack_paths_file "$STACK")" || exit 1
paths_validate "$STACK" "$PATHS_FILE" \
    || { err "the $STACK path contract failed validation"; exit 1; }
REMOTE_DIR="$(paths_get "$PATHS_FILE" .vps.stack_dir)" || exit 1
BACKUP_ROOT="$(paths_get "$PATHS_FILE" .backup.root)" || exit 1
assert_safe_remote_dir "$REMOTE_DIR" || exit 1

banner "DEPLOY · $STACK"

# ── 1. locate and validate the bundle ───────────────────────────────────────
section "1/6  BUNDLE"

if [[ -n "$BUNDLE_ARG" ]]; then
    BUNDLE="$(cd "$BUNDLE_ARG" 2>/dev/null && pwd)" || { err "no such bundle: $BUNDLE_ARG"; exit 1; }
else
    BUNDLE="$(bundle_path_newest "$BUILD_DIR/$STACK")"
    [[ -n "$BUNDLE" ]] || {
        err "no bundle staged for $STACK — run: ./release_manager/export.sh --engine"
        exit 1; }
fi

COMPOSE_NAME="$(stack_attr "$STACK" compose)"
DEPLOY_NAME="$(stack_attr "$STACK" deploy)"
ROLLBACK_NAME="$(stack_attr "$STACK" rollback)"
GUIDE_NAME="$(stack_attr "$STACK" guide)"

for f in manifest.json paths.json checksums.sha256 "$COMPOSE_NAME" \
         "$DEPLOY_NAME" "$ROLLBACK_NAME" \
         _bb_lib.sh _bb_deploy.sh _bb_rollback.sh .env.example; do
    [[ -f "$BUNDLE/$f" ]] || { err "bundle is incomplete, missing: $f"; exit 1; }
done

# Bind the bundle to the tracked contract before any upload. The bundle's copy
# must validate as this stack's contract, match the digest its own manifest
# recorded at export time, and be byte-identical to the tracked file. A contract
# edited after export fails closed here, not on the VPS.
step "binding the bundle to the tracked path contract"
paths_validate "$STACK" "$BUNDLE/paths.json" \
    || { err "bundle paths.json is not a valid $STACK contract — re-export"; exit 1; }
BUNDLE_PATHS_SHA="$(sha256sum "$BUNDLE/paths.json" | cut -d' ' -f1)"
MANIFEST_PATHS_SHA="$(jq -r '.paths.sha256 // empty' "$BUNDLE/manifest.json")"
if [[ -n "$MANIFEST_PATHS_SHA" && "$MANIFEST_PATHS_SHA" != "$BUNDLE_PATHS_SHA" ]]; then
    err "bundle paths.json does not match the digest in its own manifest"
    err "the bundle was altered after export — re-export it"
    exit 1
fi
TRACKED_PATHS_SHA="$(sha256sum "$PATHS_FILE" | cut -d' ' -f1)"
if [[ "$BUNDLE_PATHS_SHA" != "$TRACKED_PATHS_SHA" ]]; then
    err "bundle paths.json does not match the tracked stacks/$STACK/paths.json"
    err "the path contract changed after this bundle was exported — re-export it"
    exit 1
fi
ok "bundle path contract matches the tracked contract"

VERSION="$(jq -r '.version // empty' "$BUNDLE/manifest.json")"
KIND="$(jq -r '.kind // "unknown"' "$BUNDLE/manifest.json")"
DIRTY="$(jq -r '.git_dirty // false' "$BUNDLE/manifest.json")"
[[ -n "$VERSION" ]] || { err "bundle manifest has no version"; exit 1; }

if [[ -z "$BUNDLE_ARG" && "$DIRTY" == true ]]; then
    warn "auto-selected bundle was built from a DIRTY tree: $VERSION"
    warn "it is not reproducible from git; pass --bundle to choose a specific one"
fi

field "bundle"  "${BUNDLE#"$ROOT_DIR"/}"
field "version" "$VERSION"
field "kind"    "$KIND"
field "remote"  "${BB_SSH_ALIAS}:${REMOTE_DIR}"

# Verify locally before spending bandwidth on a corrupt archive.
step "verifying bundle checksums locally"
IMAGE_ENTRIES="$(jq -r '.images | to_entries[] | [.key, .value.sha256, .value.archive] | @tsv' \
    "$BUNDLE/manifest.json")" || { err "cannot read image checksums from the manifest"; exit 1; }
while read -r key sha archive; do
    [[ -n "$key" ]] || continue
    [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || { err "no checksum recorded for image $key"; exit 1; }
    actual="$(sha256sum "$BUNDLE/$archive" | cut -d' ' -f1)"
    [[ "$actual" == "$sha" ]] || { err "local checksum mismatch for $archive"; exit 1; }
done <<< "$IMAGE_ENTRIES"
ok "bundle checksums verified locally"

# ── 2. remote preflight ─────────────────────────────────────────────────────
section "2/6  REMOTE PREFLIGHT"

step "checking SSH connectivity"
bb_ssh true 2>/dev/null || { err "cannot reach $BB_SSH_ALIAS over SSH"; exit 1; }
ok "SSH ok"

# One round trip that reports everything needed before uploading. Read-only: it
# changes nothing on the VPS.
PREFLIGHT="$(bb_ssh "bash -s -- '$REMOTE_DIR' '$BACKUP_ROOT'" <<'REMOTE' || true
set -u
stack_dir="$1"; backup_root="$2"
printf 'stack_dir_exists=%s\n'   "$([[ -d "$stack_dir" ]] && echo yes || echo no)"
printf 'stack_dir_writable=%s\n' "$([[ -w "$stack_dir" ]] && echo yes || echo no)"
printf 'backup_writable=%s\n'    "$([[ -w "$backup_root" ]] && echo yes || echo no)"
# Existence only. .env belongs to the operator; nothing here reads it, stats its
# mode, or changes it.
printf 'env_present=%s\n'        "$([[ -e "$stack_dir/.env" ]] && echo yes || echo no)"
printf 'docker_ok=%s\n'     "$(docker info >/dev/null 2>&1 && echo yes || echo no)"
printf 'compose_ok=%s\n'    "$(docker compose version >/dev/null 2>&1 && echo yes || echo no)"
printf 'rsync_ok=%s\n'      "$(command -v rsync >/dev/null 2>&1 && echo yes || echo no)"
printf 'flock_ok=%s\n'      "$(command -v flock >/dev/null 2>&1 && echo yes || echo no)"
printf 'jq_ok=%s\n'         "$(command -v jq >/dev/null 2>&1 && echo yes || echo no)"
printf 'disk_avail_mib=%s\n' "$(df -BM --output=avail "$stack_dir" 2>/dev/null | tail -n1 | tr -dc '0-9')"
REMOTE
)"

get_flag() { printf '%s\n' "$PREFLIGHT" | sed -n "s/^$1=//p" | tail -n1; }

for f in docker_ok compose_ok rsync_ok flock_ok jq_ok; do
    [[ "$(get_flag "$f")" == "yes" ]] \
        || { err "remote preflight failed: $f is unavailable on the VPS"; exit 1; }
done
ok "docker, compose, rsync, flock and jq available"

[[ "$(get_flag stack_dir_exists)" == "yes" ]] || {
    err "remote stack directory missing: $REMOTE_DIR"
    err "run: ./release_manager/provision.sh --engine"
    exit 1; }
[[ "$(get_flag stack_dir_writable)" == "yes" ]] || { err "not writable: $REMOTE_DIR"; exit 1; }
[[ "$(get_flag backup_writable)" == "yes" ]] || {
    err "$BACKUP_ROOT is not writable by the deploy user"
    err "run: ./release_manager/provision.sh --engine"
    exit 1; }
ok "remote stack and backup trees writable"

if [[ "$(get_flag env_present)" != "yes" ]]; then
    warn "remote $REMOTE_DIR/.env is not present"
    if [[ "$SHIP_ONLY" != true ]]; then
        err "the engine cannot start without it. Place it yourself:"
        err "  ssh $BB_SSH_ALIAS"
        err "  cd $REMOTE_DIR   # then create .env however you prefer"
        err "Nothing in this pipeline reads, writes or removes that file."
        exit 1
    fi
fi

avail="$(get_flag disk_avail_mib)"
bundle_mib="$(du -sm "$BUNDLE" | cut -f1)"
need=$(( bundle_mib * 3 ))
if [[ -n "$avail" ]] && (( avail < need )); then
    err "insufficient remote disk: ${avail}MiB free, need ~${need}MiB"
    exit 1
fi
field "remote disk" "${avail:-?}MiB free (bundle ${bundle_mib}MiB)"

# ── 3. confirm ──────────────────────────────────────────────────────────────
if [[ "$ASSUME_YES" != true && "$UI_INTERACTIVE" == true ]]; then
    printf '\n'
    confirm "Ship $VERSION to $STACK on $BB_SSH_ALIAS?" || { warn "aborted"; exit 0; }
fi

# ── 4. upload ───────────────────────────────────────────────────────────────
section "4/6  UPLOAD"

# --checksum re-verifies content rather than trusting size+mtime. No --delete:
# nothing removes a remote file it did not put there. --exclude='/.env' is the
# hard guarantee that this pipeline never writes the operator's credentials;
# combined with no --delete, .env can be neither overwritten nor removed.
RSYNC_OPTS=(-az --checksum --human-readable --partial
            --chmod=F644,D755 --exclude='/.env')
bb_ssh_opts
printf -v RSYNC_SSH '%q ' ssh "${BB_SSH_OPTS[@]}"

step "uploading release metadata"
# The guide is listed in checksums.sha256 when export staged it, so it must be
# uploaded here or the remote verification fails on a file that never arrived.
METADATA=("$BUNDLE/manifest.json" "$BUNDLE/paths.json" "$BUNDLE/checksums.sha256"
          "$BUNDLE/$COMPOSE_NAME" "$BUNDLE/.env.example")
[[ -f "$BUNDLE/$GUIDE_NAME" ]] && METADATA+=("$BUNDLE/$GUIDE_NAME")
[[ -f "$BUNDLE/Caddyfile" ]]   && METADATA+=("$BUNDLE/Caddyfile")
rsync "${RSYNC_OPTS[@]}" -e "$RSYNC_SSH" \
    "${METADATA[@]}" \
    "${BB_SSH_ALIAS}:${REMOTE_DIR}/" \
    || { err "failed to upload release metadata"; exit 1; }

# The served content is versioned with the release. --delete is safe HERE and
# only here, scoped to the web directory this pipeline wholly owns, so a file
# removed from the repo stops being served instead of lingering in public.
if [[ -d "$BUNDLE/web" ]]; then
    step "uploading web content ($(find "$BUNDLE/web" -type f | wc -l) file(s))"
    bb_ssh "mkdir -p '$REMOTE_DIR/web'" || { err "cannot create remote web dir"; exit 1; }
    rsync -az --checksum --delete --chmod=F644,D755 -e "$RSYNC_SSH" \
        "$BUNDLE/web/" "${BB_SSH_ALIAS}:${REMOTE_DIR}/web/" \
        || { err "failed to upload web content"; exit 1; }
fi

step "uploading VPS-native scripts"
rsync -az --checksum --chmod=F755,D755 -e "$RSYNC_SSH" \
    "$BUNDLE/$DEPLOY_NAME" "$BUNDLE/$ROLLBACK_NAME" \
    "$BUNDLE/_bb_lib.sh" "$BUNDLE/_bb_deploy.sh" "$BUNDLE/_bb_rollback.sh" \
    "${BB_SSH_ALIAS}:${REMOTE_DIR}/" \
    || { err "failed to upload native scripts"; exit 1; }

step "uploading image archives ($(du -sh "$BUNDLE/images" | cut -f1))"
bb_ssh "mkdir -p '$REMOTE_DIR/images'" || { err "cannot create remote images dir"; exit 1; }
rsync "${RSYNC_OPTS[@]}" --info=progress2 -e "$RSYNC_SSH" \
    "$BUNDLE/images/" "${BB_SSH_ALIAS}:${REMOTE_DIR}/images/" \
    || { err "failed to upload image archives"; exit 1; }

step "verifying uploads on the VPS"
REMOTE_VERIFY="$(bb_ssh "cd '$REMOTE_DIR' && sha256sum -c --quiet checksums.sha256 2>&1 && echo VERIFY_OK" || true)"
if ! printf '%s' "$REMOTE_VERIFY" | grep -q VERIFY_OK; then
    err "remote checksum verification FAILED after upload"
    printf '%s\n' "$REMOTE_VERIFY" | head -20 >&2
    exit 1
fi
ok "remote checksums match — upload intact"

if [[ "$SHIP_ONLY" == true ]]; then
    banner "SHIPPED (not deployed)"
    field "version" "$VERSION"
    field "remote"  "$REMOTE_DIR"
    printf '\n   Deploy on the VPS with:\n     ssh %s "cd %s && ./%s"\n\n' \
        "$BB_SSH_ALIAS" "$REMOTE_DIR" "$DEPLOY_NAME"
    exit 0
fi

# ── 5. hand off to the VPS-native deploy script ─────────────────────────────
section "5/6  REMOTE DEPLOY" "all docker work happens on the VPS from here"

# -t so the remote script can prompt for its confirmation and so its output is
# streamed live rather than buffered until it finishes.
bb_ssh_opts
printf '\n'
if ssh -t "${BB_SSH_OPTS[@]}" "$BB_SSH_ALIAS" \
        "cd '$REMOTE_DIR' && ./'$DEPLOY_NAME' ${REMOTE_ARGS[*]:-}"; then
    REMOTE_RC=0
else
    REMOTE_RC=$?
fi
printf '\n'

# ── 6. reconcile ────────────────────────────────────────────────────────────
section "6/6  RECONCILE"

VERSION_NAME="$(stack_attr "$STACK" version_file)"
DEPLOYED="$(bb_ssh "jq -r '.version // empty' '$REMOTE_DIR/$VERSION_NAME' 2>/dev/null" || true)"
STATUS="$(bb_ssh "jq -r '.status // empty' '$REMOTE_DIR/$VERSION_NAME' 2>/dev/null" || true)"

mkdir -p "$RM_DIR/state"
LEDGER="$RM_DIR/state/versions.json"
[[ -s "$LEDGER" ]] || printf '{}\n' > "$LEDGER"
tmp="$(mktemp "${LEDGER}.XXXXXX")"
jq --arg stack "$STACK" --arg built "$VERSION" --arg deployed "${DEPLOYED:-}" \
   --arg status "${STATUS:-unknown}" --arg at "$(date -Is)" \
   '.[$stack] = {built: $built, deployed: $deployed, status: $status, shipped_at: $at}' \
   "$LEDGER" > "$tmp" && mv "$tmp" "$LEDGER"

field "shipped"  "$VERSION"
field "deployed" "${DEPLOYED:-<unknown>}"
field "status"   "${STATUS:-<unknown>}"

if (( REMOTE_RC != 0 )); then
    err "remote deploy exited with status $REMOTE_RC"
    err "inspect the log on the VPS:"
    err "  ssh $BB_SSH_ALIAS 'ls -t $(paths_get "$PATHS_FILE" .backup.deploy_log)/ | head'"
    exit "$REMOTE_RC"
fi

if [[ "$DEPLOYED" != "$VERSION" ]]; then
    warn "remote reports '$DEPLOYED' but we shipped '$VERSION' — investigate"
    exit 1
fi

banner "DEPLOYED"
field "stack"   "$STACK"
field "version" "$VERSION"
field "status"  "${STATUS:-active}"
printf '\n'
