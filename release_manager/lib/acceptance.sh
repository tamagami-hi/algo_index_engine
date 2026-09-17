#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# acceptance.sh — what a built image must prove about itself before it is
# archived into a bundle.
#
# These run against the image, not against the source that produced it. Grepping
# the Dockerfile would confirm what we asked for; running the binary confirms what
# we got. Both checks read a single captured startup transcript, so the image is
# started once.
#
# Kept in lib/ rather than inline in export.sh so the decisions can be driven
# directly by tests/release_profile.sh without a Docker daemon.
# ─────────────────────────────────────────────────────────────────────────────

# accept_starts_cleanly <startup-output>
#
# With no configuration present the engine must fail on configuration, and say
# so. A panic, a missing shared library or a silent exit all look like "it did
# not start" from outside, and all mean the image is broken regardless of .env.
accept_starts_cleanly() {
    [[ "$1" == *'Missing BLACKBOX_HTTP_ADDR'* ]]
}

# accept_release_profile <startup-output>
#
# The binary reports its own profile, derived from cfg!(debug_assertions), so it
# cannot disagree with how it was compiled. A debug build would start, satisfy
# every other check in the pipeline and deploy cleanly, while running the option
# chain hot path several times slower than it was ever measured at. Nothing
# downstream inspects optimisation level, so this is the only place it is caught.
accept_release_profile() {
    [[ "$1" == *'profile="release"'* ]]
}

# accept_profile_reported <startup-output> — the transcript names a profile at
# all. Distinguishes "built debug" from "too old to say", which need different
# fixes: one is the Dockerfile, the other is a stale image.
accept_profile_reported() {
    [[ "$1" == *'profile="'* ]]
}
