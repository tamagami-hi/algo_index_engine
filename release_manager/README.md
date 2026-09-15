# release_manager

Deployment pipeline for the `index_engine` stack. One algo per repository, so this
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

`stacks/index_engine/paths.json` owns every path a deployment touches. No script
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

`stacks/index_engine/.env.example` is reference material and is never copied over
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

Not configured for this engine. `web.enabled` is false in the path contract, no
hostname is assigned, and the deployed compose file runs the engine alone.

This host already serves BOE_APP from nginx on 80/443, so a Caddy container here
would fail to bind and break every deploy. The engine instead publishes its HTTP
port on loopback only, `127.0.0.1:47601`, matching the convention the other stacks
on this box follow. If this engine is ever given a hostname, proxy that port from
the existing host nginx rather than adding a second web server.

`/api/stream` has no authentication, so it must not be exposed publicly as-is.

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
  index_engine/ paths.json, compose, thin entry points, guide, env example
build/         staged bundles (gitignored)
state/         local ship ledger (gitignored)
```

## A note on comments

The repository convention is no comments. These shell scripts are the deliberate
exception: each safety gate here encodes a reason that is expensive to rediscover
during an incident, and the top-level block in each file states its contract. The
Rust source stays comment-free.
