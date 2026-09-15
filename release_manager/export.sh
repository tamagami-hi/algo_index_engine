#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# export.sh — BUILD stage. Runs ONLY on this computer, never on the VPS.
#
# Responsibilities, and nothing else:
#   1. Determine the version label — the only script that does.
#   2. Build the engine image from the working tree.
#   3. docker save | gzip it into a versioned bundle under build/<stack>/.
#   4. Write manifest.json with SHA-256 checksums and git provenance.
#   5. Copy the VPS-native scripts, compose file, paths.json and .env.example
#      into the bundle, so the bundle is everything the VPS needs.
#
# Explicitly NOT this script's job: touching the VPS (deploy.sh), running docker
# compose anywhere (the VPS-native scripts), or any git write.
#
# Usage:
#   ./release_manager/export.sh --engine
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RM_DIR="$ROOT_DIR/release_manager"
STACKS_SRC="$RM_DIR/stacks"
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
SKIP_BUILD=false
KEEP_BUNDLES=3

usage() {
    cat <<'USAGE'
Usage: ./release_manager/export.sh --engine [options]

Builds the engine image from the current working tree and stages a versioned
release bundle under release_manager/build/<stack>/. Never touches the VPS.

Options:
  --skip-build   reuse an already-built image with this exact tag
  --keep N       how many past bundles to retain (default 3)
  --help, -h     this message

Version labelling (this is the only script that advances the version):
  clean tree on the exact vX.Y.Z tag  ->  X.Y.Z
  anything else                       ->  <next>-dev.N.gSHA[.dirty]

The canonical version comes from Cargo.toml.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine|engine|algo_engine) STACK="$(resolve_stack "$1")" || exit 1; shift ;;
        --skip-build) SKIP_BUILD=true; shift ;;
        --keep)
            [[ $# -ge 2 && "$2" =~ ^[0-9]+$ && "$2" -ge 1 ]] \
                || { err "--keep requires a positive integer"; exit 1; }
            KEEP_BUNDLES="$2"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) err "unknown argument: $1"; usage >&2; exit 1 ;;
    esac
done

[[ -n "$STACK" ]] || { err "a stack is required: --engine"; usage >&2; exit 1; }

for c in docker jq gzip sha256sum git; do
    command -v "$c" >/dev/null || { err "$c is required"; exit 1; }
done

# ── version label ───────────────────────────────────────────────────────────
CANONICAL="$(cargo_version "$ROOT_DIR/Cargo.toml")" || exit 1
assert_semver "$CANONICAL" || exit 1

if on_exact_release_tag "$ROOT_DIR" "$CANONICAL"; then
    VERSION="$CANONICAL"
    RELEASE_KIND=stable
else
    VERSION="$(dev_version "$(bump_version "$CANONICAL" patch)" "$ROOT_DIR")"
    RELEASE_KIND=dev
fi

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BUNDLE="$BUILD_DIR/$STACK/${VERSION}-${STAMP}"

GIT_SHA="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
GIT_BRANCH="$(git -C "$ROOT_DIR" symbolic-ref --short -q HEAD 2>/dev/null || echo detached)"
GIT_DIRTY=false
git_dirty "$ROOT_DIR" && GIT_DIRTY=true

banner "EXPORT · $STACK"
field "environment" "$(stack_attr "$STACK" env)"
field "version"     "$VERSION"
field "kind"        "$RELEASE_KIND"
field "commit"      "${GIT_SHA:0:9}$([[ "$GIT_DIRTY" == true ]] && printf ' (dirty)')"
field "bundle"      "${BUNDLE#"$ROOT_DIR"/}"

PATHS_FILE="$(stack_paths_file "$STACK")" || exit 1
paths_validate "$STACK" "$PATHS_FILE" \
    || { err "the $STACK path contract failed validation — fix stacks/$STACK/paths.json"; exit 1; }
ok "path contract validated"

mkdir -p "$BUNDLE/images"

# A bundle is only meaningful once manifest.json exists. If we abort before that,
# remove the partial directory so deploy.sh's newest-bundle selection can never
# pick up a half-built release.
BUNDLE_COMPLETE=false
cleanup_incomplete_bundle() {
    if [[ "$BUNDLE_COMPLETE" != true && -d "$BUNDLE" ]]; then
        rm -rf -- "$BUNDLE"
        printf '   %sremoved incomplete bundle%s\n' "$c_dim" "$c_rst" >&2
    fi
}
trap cleanup_incomplete_bundle EXIT

# ── build ───────────────────────────────────────────────────────────────────
IMAGE_TAG="$(stack_image_tag "$STACK" engine "$VERSION")"

if [[ "$SKIP_BUILD" == true ]]; then
    section "BUILD IMAGE" "skipped by --skip-build"
    docker image inspect "$IMAGE_TAG" >/dev/null 2>&1 \
        || { err "image not present: $IMAGE_TAG (drop --skip-build?)"; exit 1; }
else
    section "BUILD IMAGE" "$IMAGE_TAG"
    DOCKER_BUILDKIT=1 docker build -t "$IMAGE_TAG" "$ROOT_DIR" \
        || { err "docker build failed"; exit 1; }
    ok "image built"
fi

# Prove the image actually starts and resolves its runtime paths before it is
# archived. Without credentials it must fail on configuration, not on a panic or
# a missing binary — anything else means the image is broken regardless of .env.
section "RUNTIME ACCEPTANCE"
step "image starts and reports missing configuration"
ACCEPT_OUT="$(docker run --rm --entrypoint /usr/local/bin/blackbox_trage "$IMAGE_TAG" 2>&1 || true)"
if printf '%s' "$ACCEPT_OUT" | grep -q 'Missing DHAN_API_KEY'; then
    ok "runtime path resolution and env handling behave as expected"
else
    err "image did not fail cleanly on missing configuration:"
    printf '%s\n' "$ACCEPT_OUT" | head -5 >&2
    exit 1
fi

step "runs as a non-root user"
ACCEPT_USER="$(docker run --rm --entrypoint id "$IMAGE_TAG" -un 2>/dev/null || echo root)"
[[ "$ACCEPT_USER" != root ]] || { err "image runs as root"; exit 1; }
ok "runs as $ACCEPT_USER"

# ── save ────────────────────────────────────────────────────────────────────
section "SAVE IMAGE"
declare -A SHA=()
while IFS=: read -r key archive; do
    [[ -n "$key" ]] || continue
    tag="$(stack_image_tag "$STACK" "$key" "$VERSION")"
    step "saving $tag"
    # gzip -n omits the timestamp so identical input yields an identical archive.
    # The checksum then means something across rebuilds.
    docker save "$tag" | gzip -n > "$BUNDLE/images/$archive"
    SHA["$key"]="$(sha256sum "$BUNDLE/images/$archive" | cut -d' ' -f1)"
    ok "$archive  $(du -h "$BUNDLE/images/$archive" | cut -f1)"
done < <(stack_images "$STACK")

# ── stage the VPS-side artifacts ────────────────────────────────────────────
section "STAGE VPS ARTIFACTS"

COMPOSE_NAME="$(stack_attr "$STACK" compose)"
DEPLOY_NAME="$(stack_attr "$STACK" deploy)"
ROLLBACK_NAME="$(stack_attr "$STACK" rollback)"
GUIDE_NAME="$(stack_attr "$STACK" guide)"

cp "$STACKS_SRC/$STACK/$COMPOSE_NAME"  "$BUNDLE/$COMPOSE_NAME"
cp "$STACKS_SRC/$STACK/$DEPLOY_NAME"   "$BUNDLE/$DEPLOY_NAME"
cp "$STACKS_SRC/$STACK/$ROLLBACK_NAME" "$BUNDLE/$ROLLBACK_NAME"
cp "$STACKS_SRC/$STACK/.env.example"   "$BUNDLE/.env.example"
[[ -f "$STACKS_SRC/$STACK/$GUIDE_NAME" ]] && cp "$STACKS_SRC/$STACK/$GUIDE_NAME" "$BUNDLE/$GUIDE_NAME"

# The shared runtime library travels with every bundle, so a stack directory on
# the VPS is self-sufficient over nothing but SSH.
cp "$STACKS_SRC/_shared/_bb_lib.sh"      "$BUNDLE/_bb_lib.sh"
cp "$STACKS_SRC/_shared/_bb_deploy.sh"   "$BUNDLE/_bb_deploy.sh"
cp "$STACKS_SRC/_shared/_bb_rollback.sh" "$BUNDLE/_bb_rollback.sh"
chmod +x "$BUNDLE/$DEPLOY_NAME" "$BUNDLE/$ROLLBACK_NAME"

# The public edge travels with the release, so the served content is versioned
# with the engine and covered by the same checksum manifest. Caddy itself is a
# pinned upstream image pulled on the VPS, so nothing is built for it here.
if [[ "$(jq -r '.web.enabled // false' "$PATHS_FILE")" == "true" ]]; then
    [[ -f "$STACKS_SRC/$STACK/Caddyfile" ]] \
        || { err "web is enabled but stacks/$STACK/Caddyfile is missing"; exit 1; }
    [[ -f "$ROOT_DIR/web/index.html" ]] \
        || { err "web is enabled but web/index.html is missing"; exit 1; }
    cp "$STACKS_SRC/$STACK/Caddyfile" "$BUNDLE/Caddyfile"
    mkdir -p "$BUNDLE/web"
    # Explicit file types only: never sweep an editor swap file or a stray key
    # into a directory that gets served to the public internet.
    find "$ROOT_DIR/web" -maxdepth 1 -type f \
        \( -name '*.html' -o -name '*.css' -o -name '*.js' -o -name '*.svg' \
           -o -name '*.png' -o -name '*.webp' -o -name '*.ico' -o -name '*.woff2' \) \
        -exec cp -- {} "$BUNDLE/web/" \;
    ok "staged Caddyfile and $(find "$BUNDLE/web" -type f | wc -l) web file(s)"
fi

# paths.json is the hand-edited canonical contract: the sole authority for every
# path this bundle will use. Copied byte for byte — never generated here.
cp "$PATHS_FILE" "$BUNDLE/paths.json"
PATHS_SHA="$(sha256sum "$BUNDLE/paths.json" | cut -d' ' -f1)"
COMPOSE_SHA="$(sha256sum "$BUNDLE/$COMPOSE_NAME" | cut -d' ' -f1)"
ok "staged compose, scripts, .env.example and paths.json"

# ── manifest ────────────────────────────────────────────────────────────────
section "MANIFEST"

IMAGES_JSON='{}'
while IFS=: read -r key archive; do
    [[ -n "$key" ]] || continue
    IMAGES_JSON="$(printf '%s' "$IMAGES_JSON" | jq \
        --arg k "$key" \
        --arg tag "$(stack_image_tag "$STACK" "$key" "$VERSION")" \
        --arg archive "images/$archive" \
        --arg sha "${SHA[$key]:-}" \
        '.[$k] = {tag: $tag, archive: $archive, sha256: $sha}')"
done < <(stack_images "$STACK")

jq -n \
    --arg version "$VERSION" \
    --arg kind "$RELEASE_KIND" \
    --arg stack "$STACK" \
    --arg environment "$(stack_attr "$STACK" env)" \
    --arg created_at "$STAMP" \
    --arg git_sha "$GIT_SHA" \
    --arg git_branch "$GIT_BRANCH" \
    --argjson git_dirty "$GIT_DIRTY" \
    --arg compose_file "$COMPOSE_NAME" \
    --arg compose_sha "$COMPOSE_SHA" \
    --arg paths_sha "$PATHS_SHA" \
    --argjson images "$IMAGES_JSON" \
    '{version: $version, kind: $kind, stack: $stack, environment: $environment,
      created_at: $created_at,
      git_sha: $git_sha, git_branch: $git_branch, git_dirty: $git_dirty,
      compose: {file: $compose_file, sha256: $compose_sha},
      paths: {file: "paths.json", sha256: $paths_sha},
      images: $images}' > "$BUNDLE/manifest.json"

jq empty "$BUNDLE/manifest.json" || { err "generated an invalid manifest"; exit 1; }

# A flat checksum list too, so sha256sum -c works on the VPS without jq. Every
# path must resolve relative to the stack directory, because both verifiers run
# `cd <stack_dir> && sha256sum -c checksums.sha256`.
( cd "$BUNDLE" && find . -type f ! -name 'checksums.sha256' -print0 \
    | sort -z | xargs -0 -r sha256sum > checksums.sha256 )

BUNDLE_COMPLETE=true
ok "manifest.json and checksums.sha256 written"

# ── retention ───────────────────────────────────────────────────────────────
mapfile -t old < <(bundle_dirs_oldest_first "$BUILD_DIR/$STACK")
if (( ${#old[@]} > KEEP_BUNDLES )); then
    for v in "${old[@]:0:$(( ${#old[@]} - KEEP_BUNDLES ))}"; do
        rm -rf -- "$BUILD_DIR/$STACK/$v" && info "pruned old bundle $v"
    done
fi

banner "BUNDLE READY"
field "stack"   "$STACK"
field "version" "$VERSION"
field "path"    "${BUNDLE#"$ROOT_DIR"/}"
field "size"    "$(du -sh "$BUNDLE" | cut -f1)"
printf '\n   Next:  ./release_manager/deploy.sh --engine\n\n'
