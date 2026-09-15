# release_manager

Deployment pipeline for the `algo_engine` stack. One algo per repository, so this
folder deploys exactly one thing — and is copied verbatim into the next algo's
repo, where only `stacks/` needs retargeting.

## Three stages, strictly separated

```
export.sh          BUILD   this computer   builds the image, stages a bundle
deploy.sh          SHIP    this computer   uploads it, invokes the remote script
<stack>_deploy.sh  RUN     the VPS         every docker command lives here
```

`deploy.sh` runs **no** docker command against the VPS. Every container operation
happens in the VPS-native script, which is the one thing holding the lock. That
is a hard requirement, not a style preference: it keeps a single place
responsible for the live system.

`export.sh` is the only script that advances the version. No script here writes
to git.

## Usage

```sh
./release_manager/provision.sh --engine        # once per host: create the tree
./release_manager/export.sh    --engine        # build + stage a bundle
./release_manager/deploy.sh    --engine        # upload + deploy
./release_manager/status.sh    --engine        # local vs live, read-only
./release_manager/rollback.sh  --engine --list # what can be restored
```

`deploy.sh --engine --ship-only` uploads without deploying, which is the safe way
to stage a release ahead of a market open.

## paths.json is the only path authority

`stacks/algo_engine/paths.json` owns every path a deployment touches. No script
derives a remote path; each is read from the contract after validation, so a path
change is an edit to that one file.

`lib/stacks.sh` owns identity only — ids, image naming, container prefix,
retention — and deliberately owns no paths.

The contract is triple-bound before any upload. The bundle's copy must validate
as this stack's contract, match the digest its own manifest recorded at export
time, and be byte-identical to the tracked file. A contract edited after export
fails closed on your laptop rather than halfway through a deploy.

## .env is yours alone

Nothing here reads its contents, writes it, changes its mode, or removes it.
`deploy.sh` excludes it from rsync and runs without `--delete`, so it can be
neither overwritten nor removed. The deploy checks only that it *exists*, because
otherwise the container just crash-loops. Only Docker reads it, via `env_file`,
at container start.

`stacks/algo_engine/.env.example` is reference material and is never copied over
`.env`.

## Integrity

Images are saved with `gzip -n` so identical input yields an identical archive and
the checksum means something across rebuilds. Checksums are verified three times:
locally before upload, remotely after upload via `sha256sum -c`, and again inside
the VPS script on arrival.

## Health gating

The engine has no HTTP surface yet, so `health.mode` is `container`: the gate
asserts the container is still running after a settle window with no restarts,
and that a log pattern appeared. When an HTTP endpoint exists, set
`health.mode: "http"` and `health.http_url` in the contract — no code changes.

A failed gate triggers an automatic image-level rollback. That is safe only
because the engine owns no database; the data volume, holding the session token
and dated instrument masters, is never touched by a deploy or a rollback.

## The public edge

`algogon.xyz` is served by Caddy, running as a second service in the same stack.
Static files only: the engine has no HTTP surface, and nothing about the account,
positions or strategy reaches the page.

- content lives in `web/` at the repo root and is **versioned with the release** —
  it travels in the bundle and is covered by the same checksum manifest
- `stacks/algo_engine/Caddyfile` configures the edge, validated by `caddy validate`
- TLS is automatic via Let's Encrypt; certificates persist in the `caddy-data`
  volume so a redeploy never re-issues and cannot trip a rate limit
- `www` redirects to the apex; HTTP redirects to HTTPS
- the web rsync is the one place `--delete` is used, scoped to the directory this
  pipeline wholly owns, so a file removed from the repo stops being served

Caddy is a pinned upstream image pulled on the VPS, so it is not in the bundle and
is not versioned with the engine — only the content it serves is.

`compose.algo_engine.yml` also publishes `127.0.0.1:8080`, serving the same root.
That is loopback-only and lets the box verify its own edge without DNS, TLS, or an
open port, which is what `bb_check_web` probes after a deploy. A web failure is a
warning, never a deploy failure: the engine is the critical service and must not be
rolled back because a static page is down.

DNS is Namecheap BasicDNS pointing straight at the origin. If it is ever moved
behind Cloudflare, set the SSL mode to **Full (strict)** — on Flexible, Cloudflare
speaks HTTP to an origin that redirects to HTTPS, which is an infinite loop.

## Layout

```
export.sh  deploy.sh  rollback.sh  status.sh  provision.sh
lib/
  ui.sh        output helpers
  stacks.sh    stack identity and ssh plumbing (no paths)
  paths.sh     contract validation and lookup
  version.sh   version labelling, from Cargo.toml
stacks/
  _shared/     the VPS-native runtime, shared so stacks cannot drift
  algo_engine/ paths.json, compose, thin entry points, guide, env example
build/         staged bundles (gitignored)
state/         local ship ledger (gitignored)
```

## A note on comments

The repository convention is no comments. These shell scripts are the deliberate
exception: each safety gate here encodes a reason that is expensive to rediscover
during an incident, and the top-level block in each file states its contract. The
Rust source stays comment-free.
