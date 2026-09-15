# algo_engine — operator guide

This file lives next to the deployed release on the VPS. Everything here is run
**on the VPS**, from the stack directory.

```sh
cd /home/ubuntu/blackbox_trage/algo_engine
```

## What is running

```sh
jq . engine-version.json          # current, previous, status
docker compose -p blackbox_engine ps
docker logs -f bb-engine
```

## Deploy and roll back

The operator machine normally drives both. By hand:

```sh
./algo_engine_deploy.sh                  # deploy the staged bundle
./algo_engine_deploy.sh --force          # redeploy the same version
./algo_engine_deploy.sh --skip-checks    # start without gating on health

./algo_engine_rollback.sh --list         # what can be restored
./algo_engine_rollback.sh --to VERSION
```

Deploy and rollback share one lock (`/run/lock/blackbox-algo_engine.lock`), so
they can never interleave.

## Credentials

`.env` lives here and only here, and it is **entirely yours**. Nothing in this
pipeline reads its contents, writes it, changes its mode, or deletes it:

- `deploy.sh` excludes it from rsync and runs without `--delete`, so it can be
  neither overwritten nor removed
- no script greps it, and the deploy checks only that it *exists*
- only Docker reads it, via `env_file` in the compose file, at container start

`.env.example` next to it is reference material, never copied over `.env`.

`DHAN_AUTH_MODE=web` does not work on this host: it shells out to `xdg-open` and
waits on a loopback callback no browser can reach. Use `token_url` for
unattended running. `manual` works but a Dhan token expires within 24 hours.

## Data

The named volume `blackbox_engine_data` holds `/app/data`: the Dhan session
token and the dated instrument masters. A rollback never touches it, so the
session survives and the engine does not need to re-authenticate.

```sh
docker run --rm -v blackbox_engine_data:/data alpine:3.22 ls -la /data/instruments
```

Masters accumulate at roughly 34 MiB per trading day. The deploy prunes them to
`retention.keep_instrument_masters` from `paths.json`.

## Health

There is no HTTP endpoint yet, so `health.mode` is `container`: the gate asserts
the container is still running after a settle window with no restarts, and that
the log contains the pattern from `paths.json`. When the engine grows an HTTP
surface, switch `health.mode` to `http` and set `health.http_url` — no code
change is needed.

A failed health gate triggers an automatic image-level rollback, which is safe
here only because the engine owns no database.

## Logs

```sh
ls -t ../logs/deploy | head      # one log per deploy run
ls -t ../logs/app | head         # container logs captured on failure
```

## Paths

Every path above comes from `paths.json` in this directory. It is the sole
authority and is copied byte-for-byte from the repository; the operator machine
refuses to deploy a bundle whose copy has drifted from the tracked one. To change
a path, edit the tracked contract and re-export — never edit this copy.
