#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# nginx_ship.sh — ships the nginx site configs and tells the operator exactly
# what to install. Callers must source ui.sh, paths.sh and stacks.sh first.
#
# ── WHY THIS EXISTS ─────────────────────────────────────────────────────────
# The nginx configs were the one part of the release no script carried. The bundle
# shipped images, compose, paths.json and the native scripts; the vhost was copied
# up by hand, so the box could drift from the repo indefinitely with every other
# check still passing. The port is the sharpest case: the env file moves the
# engine, and a vhost still pointing at the old port leaves the interface dead
# while health, readiness and the deploy all report success.
#
# ── WHY IT STOPS AT THE STAGING FOLDER ──────────────────────────────────────
# This ships to <vps.root>/nginx and goes no further. Installing into /etc/nginx
# needs root, and a bad file there takes down every site on the box at once,
# including the ones this pipeline does not own. So the pipeline stages the files
# and prints the precise commands; a human runs them, reads `nginx -t`, and
# decides to reload. Same division deploy.sh already draws around docker.
#
# ── WHY THE NAMES ARE MAPPED ────────────────────────────────────────────────
# The repo names configs by hostname and nginx installs them by site. Nothing
# derives one from the other, so the mapping is declared once, here, and both the
# shipper and the guide read it. A wrong guess installs a vhost over another site.
# ─────────────────────────────────────────────────────────────────────────────

NGINX_ETC="${NGINX_ETC:-/etc/nginx}"

# ── the routing table ───────────────────────────────────────────────────────
# <repo filename>|<path under /etc/nginx>|<kind>
#
# kind:
#   site    a server block in sites-available; needs a sites-enabled symlink
#   http    http-context include in conf.d — loaded by EVERY site on the box
#   snippet an include fragment, inert until a site includes it
#
# Only files listed here are shipped. A config in release_manager/nginx/ with no
# row is reported as unroutable rather than uploaded to be forgotten about.
nginx_ship_map() {
    cat <<'MAP'
index.algo.boe.internal.conf|sites-available/algo-index-engine|site
MAP
}

nginx_ship_source_dir() { printf '%s\n' "$RM_DIR/nginx"; }

# nginx_ship_remote_dir <paths.json> — the staging folder on the VPS.
#
# Taken from the contract so the path is declared in one place, and defaulted to
# <vps.root>/nginx because the folder is a property of the box rather than of any
# one release.
nginx_ship_remote_dir() {
    local paths_file="$1" configured root
    configured="$(paths_get_opt "$paths_file" '.nginx.staging_dir')"
    if [[ -n "$configured" ]]; then
        printf '%s\n' "$configured"
        return 0
    fi
    root="$(paths_get "$paths_file" '.vps.root')" || return 1
    printf '%s/nginx\n' "$root"
}

nginx_ship_unmapped() {
    local src mapped file base
    src="$(nginx_ship_source_dir)"
    mapped="$(nginx_ship_map | cut -d'|' -f1)"
    for file in "$src"/*.conf; do
        [[ -e "$file" ]] || continue
        base="$(basename "$file")"
        printf '%s\n' "$mapped" | grep -qxF "$base" || printf '%s\n' "$base"
    done
}

# nginx_ship_stage <bundle_dir> — copy the mapped configs into <bundle>/nginx/.
#
# Deliberately NOT covered by checksums.sha256: that manifest is verified with
# `cd <stack_dir> && sha256sum -c`, and these land outside it at
# <vps.root>/nginx. Listing them there would make the release's own integrity
# check fail on files that were never meant to be in the stack directory, so the
# upload is proven by nginx_ship_verify instead.
nginx_ship_stage() {
    local bundle="$1" src staged=0
    local -a missing=()
    src="$(nginx_ship_source_dir)"
    [[ -d "$src" ]] || { warn "no nginx directory at $src — nothing to stage"; return 0; }

    mkdir -p "$bundle/nginx"
    local file dest kind
    while IFS='|' read -r file dest kind; do
        [[ -n "$file" ]] || continue
        if [[ -f "$src/$file" && ! -L "$src/$file" ]]; then
            cp -- "$src/$file" "$bundle/nginx/$file"
            staged=$((staged + 1))
        else
            missing+=("$file")
        fi
    done < <(nginx_ship_map)

    # A mapped-but-absent file is reported, not fatal: a config can be
    # legitimately retired, and the row is then removed in the same commit.
    if ((${#missing[@]})); then
        warn "mapped but not in the tree: ${missing[*]}"
        info "remove the row from nginx_ship_map if the config was retired"
    fi

    local orphan
    orphan="$(nginx_ship_unmapped)"
    [[ -z "$orphan" ]] || {
        warn "unroutable config(s), NOT shipped: $(printf '%s ' "$orphan")"
        info "add a row to nginx_ship_map in release_manager/lib/nginx_ship.sh"
    }

    ok "staged $staged nginx config(s)"
}

# nginx_ship_verify <local_nginx_dir> <remote_nginx_dir>
#
# A remote-only extra file is not an error: the folder may hold retired configs
# this pipeline does not own. Only the files just shipped must match.
nginx_ship_verify() {
    local local_dir="$1" remote_dir="$2"
    local local_sums remote_sums mismatch="" line

    compgen -G "$local_dir/*.conf" >/dev/null || { warn "nothing to verify"; return 0; }

    local_sums="$(cd "$local_dir" && sha256sum ./*.conf | sort)"
    remote_sums="$(bb_ssh "cd ${remote_dir@Q} && sha256sum ./*.conf 2>/dev/null | sort" || true)"

    while IFS= read -r line; do
        [[ -n "$line" ]] || continue
        printf '%s\n' "$remote_sums" | grep -qxF "$line" || mismatch+="${line##* }"$'\n'
    done <<< "$local_sums"

    if [[ -n "$mismatch" ]]; then
        err "nginx config digests do not match after upload:"
        printf '%s' "$mismatch" | sed 's/^/     /' >&2
        return 1
    fi
    ok "nginx configs verified in $remote_dir"
}

# ── the guide ───────────────────────────────────────────────────────────────

# nginx_ship_probe <remote_nginx_dir> — classify every mapped config as
# SAME, CHANGED, NEW or ABSENT, in one SSH round trip.
nginx_ship_probe() {
    local remote_nginx_dir="$1" src
    src="$(nginx_ship_source_dir)"

    local -a dests=()
    local file dest kind
    while IFS='|' read -r file dest kind; do
        [[ -n "$file" ]] || continue
        dests+=("$NGINX_ETC/$dest")
    done < <(nginx_ship_map)

    # 2>/dev/null on sha256sum: a destination that does not exist yet is NEW, not
    # an error. Missing lines are handled below.
    local remote_digests
    remote_digests="$(bb_ssh "sha256sum ${dests[*]@Q} 2>/dev/null; ls -1 ${remote_nginx_dir@Q} 2>/dev/null | sed 's/^/STAGED /'" || true)"

    # Distinguish "the box answered and has nothing" from "we never reached the
    # box". Without this an SSH failure reports every config as ABSENT, which
    # reads as a shipping bug and sends the operator looking in the wrong place.
    if [[ -z "$remote_digests" ]]; then
        printf '__UNREACHABLE__\n'
        return 0
    fi

    local installed local_sha
    while IFS='|' read -r file dest kind; do
        [[ -n "$file" ]] || continue

        if ! printf '%s\n' "$remote_digests" | grep -qx "STAGED $file"; then
            printf '%s|ABSENT|%s|%s\n' "$file" "$dest" "$kind"
            continue
        fi

        installed="$(printf '%s\n' "$remote_digests" \
            | awk -v path="$NGINX_ETC/$dest" '$2 == path { print $1; exit }')"
        if [[ -z "$installed" ]]; then
            printf '%s|NEW|%s|%s\n' "$file" "$dest" "$kind"
            continue
        fi

        local_sha="$(sha256sum "$src/$file" 2>/dev/null | cut -d' ' -f1)"
        if [[ "$local_sha" == "$installed" ]]; then
            printf '%s|SAME|%s|%s\n' "$file" "$dest" "$kind"
        else
            printf '%s|CHANGED|%s|%s\n' "$file" "$dest" "$kind"
        fi
    done < <(nginx_ship_map)
}

# nginx_ship_domains — the hostnames the mapped site configs answer to.
nginx_ship_domains() {
    local src file dest kind
    src="$(nginx_ship_source_dir)"
    while IFS='|' read -r file dest kind; do
        [[ "$kind" == site && -f "$src/$file" ]] || continue
        sed -n 's/^[[:space:]]*server_name[[:space:]]\+\([^;]*\);.*/\1/p' "$src/$file" \
            | tr ' ' '\n' | grep -v '^$' || true
    done < <(nginx_ship_map)
}

# nginx_ship_guide <remote_nginx_dir> <paths.json>
#
# Prints commands only for configs that actually differ. A guide that lists every
# file every time trains the reader to paste it unread, which is how an unrelated
# site gets a vhost dropped on it.
nginx_ship_guide() {
    local remote_nginx_dir="$1" paths_file="${2:-}"
    local probe
    probe="$(nginx_ship_probe "$remote_nginx_dir")" || {
        warn "could not compare the staged configs with $NGINX_ETC"
        return 0
    }

    if [[ "$probe" == "__UNREACHABLE__" ]]; then
        warn "could not reach $BB_SSH_ALIAS to compare against $NGINX_ETC"
        info "the configs may be staged correctly — re-run this step to get the guide"
        return 0
    fi

    section "NGINX  what is installed vs what was just shipped"

    local file state dest kind
    local -a to_install=() new_sites=() absent=()
    local shared_changed=false

    while IFS='|' read -r file state dest kind; do
        [[ -n "$file" ]] || continue
        case "$state" in
            SAME)    ok   "$file → $dest  (identical, nothing to do)" ;;
            CHANGED) warn "$file → $dest  (differs — needs installing)"
                     to_install+=("$file|$dest|$kind")
                     [[ "$kind" == "http" ]] && shared_changed=true ;;
            NEW)     warn "$file → $dest  (not installed yet)"
                     to_install+=("$file|$dest|$kind")
                     [[ "$kind" == "site" ]] && new_sites+=("$dest")
                     [[ "$kind" == "http" ]] && shared_changed=true ;;
            ABSENT)  err  "$file  (not in $remote_nginx_dir — shipping failed?)"
                     absent+=("$file") ;;
        esac
    done <<< "$probe"

    ((${#absent[@]} == 0)) \
        || info "re-run the ship step; do not hand-copy a file the pipeline did not place"

    local domain tailnet=''
    [[ -n "$paths_file" ]] && tailnet="$(paths_get_opt "$paths_file" '.nginx.tailnet_address')"

    if ((${#to_install[@]} == 0)); then
        printf '\n'
        ok "$NGINX_ETC is already current — no action needed"
    else
        printf '\n   %sRun these on the VPS, in order:%s\n\n' "$c_bold" "$c_rst"
        printf '     ssh %s\n' "$BB_SSH_ALIAS"
        printf '     sudo cp -a %s /root/nginx-backup-$(date +%%Y%%m%%dT%%H%%M%%S)\n\n' "$NGINX_ETC"

        local entry
        for entry in "${to_install[@]}"; do
            IFS='|' read -r file dest kind <<< "$entry"
            printf '     sudo install -o root -g root -m 644 %s/%s %s/%s\n' \
                "$remote_nginx_dir" "$file" "$NGINX_ETC" "$dest"
        done

        if ((${#new_sites[@]})); then
            printf '\n     # new site(s) — enable them:\n'
            local site
            for site in "${new_sites[@]}"; do
                printf '     sudo ln -sfn %s/%s %s/sites-enabled/%s\n' \
                    "$NGINX_ETC" "$site" "$NGINX_ETC" "$(basename "$site")"
            done
        fi

        printf '\n     sudo nginx -t\n'
        printf '     sudo systemctl reload nginx\n'
        info "reload, not restart: a restart drops every in-flight SSE stream"

        if [[ "$shared_changed" == true ]]; then
            printf '\n'
            warn "an http-context include changed — it is loaded by EVERY site on this box"
            info "compare it against the installed copy before reloading"
        fi
    fi

    # ── DNS, which the pipeline cannot do for you ───────────────────────────
    printf '\n   %sHosts entries (.internal is not in any DNS):%s\n\n' "$c_bold" "$c_rst"
    while IFS= read -r domain; do
        [[ -n "$domain" ]] || continue
        printf '     # on the VPS, so an on-box curl works:\n'
        printf '     echo "127.0.0.1 %s" | sudo tee -a /etc/hosts\n' "$domain"
        printf '     # on each machine you browse from:\n'
        printf '     echo "%s %s" | sudo tee -a /etc/hosts\n' \
            "${tailnet:-<vps-tailnet-address>}" "$domain"
    done < <(nginx_ship_domains)

    printf '\n   %sThen verify, on the VPS:%s\n\n' "$c_bold" "$c_rst"
    while IFS= read -r domain; do
        [[ -n "$domain" ]] || continue
        printf '     curl -sS -o /dev/null -w "%%{http_code}\\n" -H "Host: %s" http://127.0.0.1/health\n' "$domain"
        printf '     curl -sS -H "Host: %s" http://127.0.0.1/ready\n' "$domain"
    done < <(nginx_ship_domains)
    info "health answers 200 whenever the process is up; ready answers 503 until a"
    info "catalog is loaded and market data is arriving, which is expected off-hours"

    printf '\n   %sThen from your own machine, on the tailnet:%s\n\n' "$c_bold" "$c_rst"
    while IFS= read -r domain; do
        [[ -n "$domain" ]] || continue
        printf '     open http://%s\n' "$domain"
    done < <(nginx_ship_domains)

    printf '\n   %sIf anything breaks:%s\n\n' "$c_bold" "$c_rst"
    printf '     sudo cp -a /root/nginx-backup-<timestamp>/. %s/ && sudo nginx -t && sudo systemctl reload nginx\n\n' \
        "$NGINX_ETC"
}
