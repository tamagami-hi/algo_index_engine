#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# unbound_variables.sh — catches a variable used before it is assigned.
#
# Every script here runs under `set -u`, so this is fatal rather than empty. It
# is also invisible until the exact line runs: deploy.sh passed VERSION_NAME to
# the remote snapshot step at line 275 but only assigned it at line 419, and the
# failure appeared for the first time at stage 4 of 6 of a real deploy — after
# the bundle was uploaded, with the outgoing release half-preserved.
#
# `bash -n` does not catch it, because it is a runtime expansion. ShellCheck does
# not either, because the variable IS assigned in the file; it does no ordering
# analysis. The scripts cannot be executed here, since every path past argument
# parsing talks to the VPS. So the ordering is checked statically.
#
# Quoted heredoc BODIES are excluded: `$VAR` inside <<'REMOTE' is expanded on the
# VPS, not here. The heredoc's opening LINE is deliberately included, because
# that is where local values are interpolated into the remote command — and where
# the bug was.
#
# Runs entirely offline. No SSH, no docker.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RM_DIR="$(cd "$HERE/.." && pwd)"

PASS=0
FAIL=0

# find_use_before_assign <script> — one line per finding, empty when clean.
find_use_before_assign() {
    awk -v file="$1" '
        inheredoc != "" {
            if ($0 == inheredoc) inheredoc = ""
            next
        }

        {
            line = $0
            while (match(line, /\$\{?[A-Z_][A-Z0-9_]*/)) {
                ref = substr(line, RSTART, RLENGTH)
                sub(/^\$\{?/, "", ref)
                if (!(ref in used)) used[ref] = NR
                line = substr(line, RSTART + RLENGTH)
            }
        }

        /^[A-Z_][A-Z0-9_]*=/ {
            name = $0; sub(/=.*/, "", name)
            if (!(name in assigned)) assigned[name] = NR
        }

        /<<[[:space:]]*'"'"'[A-Za-z_][A-Za-z0-9_]*'"'"'/ {
            match($0, /<<[[:space:]]*'"'"'[A-Za-z_][A-Za-z0-9_]*'"'"'/)
            tag = substr($0, RSTART, RLENGTH)
            gsub(/^<<[[:space:]]*'"'"'|'"'"'$/, "", tag)
            inheredoc = tag
        }

        END {
            for (v in used) {
                if (!(v in assigned)) continue
                if (used[v] < assigned[v])
                    printf "%s: %s used at line %d, first assigned at line %d\n",
                           file, v, used[v], assigned[v]
            }
        }
    ' "$1"
}

printf '\nThe checker detects the failure it exists for\n'

FIXTURE="$(mktemp)" || exit 1
trap 'rm -f -- "$FIXTURE" "$FIXTURE.ok" "$FIXTURE.heredoc"' EXIT

cat > "$FIXTURE" <<'FIXTURE'
#!/usr/bin/env bash
set -euo pipefail
run_remote "bash -s -- '$STACK_DIR' '$VERSION_NAME'" <<'REMOTE'
echo "$remote_only"
REMOTE
STACK_DIR=/srv/x
VERSION_NAME=version.json
FIXTURE
FOUND="$(find_use_before_assign "$FIXTURE")"
if [[ "$FOUND" == *VERSION_NAME* && "$FOUND" == *STACK_DIR* ]]; then
    printf '  ok    both variables passed into a heredoc command are flagged\n'
    PASS=$((PASS + 1))
else
    printf '  FAIL  a known use-before-assign was missed\n        got: %s\n' "${FOUND:-<nothing>}"
    FAIL=$((FAIL + 1))
fi

if [[ "$FOUND" != *remote_only* ]]; then
    printf '  ok    a variable inside the quoted heredoc body is not flagged\n'
    PASS=$((PASS + 1))
else
    printf '  FAIL  a remote-side variable was wrongly flagged\n'
    FAIL=$((FAIL + 1))
fi

cat > "$FIXTURE.ok" <<'FIXTURE'
#!/usr/bin/env bash
set -euo pipefail
STACK_DIR=/srv/x
VERSION_NAME=version.json
run_remote "bash -s -- '$STACK_DIR' '$VERSION_NAME'"
FIXTURE
if [[ -z "$(find_use_before_assign "$FIXTURE.ok")" ]]; then
    printf '  ok    correct ordering produces no finding\n'
    PASS=$((PASS + 1))
else
    printf '  FAIL  correct ordering was flagged\n'
    FAIL=$((FAIL + 1))
fi

printf '\nEvery release_manager script assigns before it expands\n'

for script in "$RM_DIR"/*.sh; do
    [[ -f "$script" ]] || continue
    name="${script#"$RM_DIR"/}"
    findings="$(find_use_before_assign "$script")"
    if [[ -z "$findings" ]]; then
        printf '  ok    %s\n' "$name"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n' "$name"
        printf '        %s\n' "$findings"
        FAIL=$((FAIL + 1))
    fi
done

printf '\nThe libraries too\n'

for script in "$RM_DIR"/lib/*.sh "$RM_DIR"/stacks/_shared/*.sh; do
    [[ -f "$script" ]] || continue
    name="${script#"$RM_DIR"/}"
    findings="$(find_use_before_assign "$script")"
    if [[ -z "$findings" ]]; then
        printf '  ok    %s\n' "$name"
        PASS=$((PASS + 1))
    else
        printf '  FAIL  %s\n' "$name"
        printf '        %s\n' "$findings"
        FAIL=$((FAIL + 1))
    fi
done

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
(( FAIL == 0 ))
