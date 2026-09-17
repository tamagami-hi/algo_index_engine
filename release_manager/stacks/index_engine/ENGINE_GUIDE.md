# index_engine — operator guide

This file lives next to the deployed release on the VPS. Everything here is run
**on the VPS**, from the stack directory.

```sh
cd /srv/dev_stack/ALGO_INDEX_ENGINE/index_engine
```

## What is running

```sh
jq . engine-version.json          # current, previous, status
docker compose -p algo_index_engine ps
docker logs -f aie-engine
```

## Deploy and roll back

The operator machine normally drives both. By hand:

```sh
./index_engine_deploy.sh                  # deploy the staged bundle
./index_engine_deploy.sh --force          # redeploy the same version
./index_engine_deploy.sh --skip-checks    # start without gating on health

./index_engine_rollback.sh --list         # what can be restored
./index_engine_rollback.sh --to VERSION
```

Deploy and rollback share one lock (`/tmp/algo-index-engine.lock`), so
they can never interleave.

## Credentials

`.env` lives here and only here, and it is **entirely yours**. Nothing in this
pipeline reads its contents, writes it, changes its mode, or deletes it:

- `deploy.sh` excludes it from rsync and runs without `--delete`, so it can be
  neither overwritten nor removed
- no script greps it, and the deploy checks only that it *exists*
- Docker reads it for Compose interpolation and container configuration;
  release manifests record its checksum to detect drift

`.env.example` next to it is reference material, never copied over `.env`.

The example uses `DHAN_AUTH_MODE=web`, with the Dhan callback on the backend
port. Set `DHAN_API_SECRET` and register the expanded `DHAN_REDIRECT_URL` with
Dhan: `http://127.0.0.1:8787/dhan/callback`. Keep
`ssh -N -L 8787:127.0.0.1:8787 beonedge` open on your browser's machine,
open the Dhan consent URL printed in the container logs, and complete login.
The saved session is reused until it needs renewal. `token_url` remains an
alternative for fetching tokens without browser login. No second callback
port is published, and no public exposure is required.

`DHAN_POSTBACK_URL` records the URL registered in the Postback URL field on
`web.dhan.co`, where Dhan posts live order status changes. The example ships it
empty and the engine serves `POST /dhan/postback` either way; the variable exists
so startup can refuse a URL that disagrees with the route actually served,
because a 404 there loses real order events with nothing to replay. Leave it
empty on this stack: Dhan does not deliver to loopback URLs and signs nothing, so
a value here is only meaningful once a deliberate decision has been made about
exposing an unauthenticated write path.

## Data

The named volume `algo_index_engine_data` holds `/app/data`: the Dhan session
token, the dated instrument masters, and the order postback journals under
`data/execution`. A rollback never touches it, so the session survives and the
engine does not need to re-authenticate.

```sh
docker run --rm -v algo_index_engine_data:/data alpine:3.22 ls -la /data/instruments
```

Masters accumulate at roughly 34 MiB per trading day. The deploy prunes them to
`retention.keep_instrument_masters` from `paths.json`.

Postbacks append to `data/execution/postbacks-<day>.jsonl`, one JSON line per
arrival, each holding the broker's body unmodified. These are the only record of
what happened to live orders and nothing prunes them.

```sh
docker run --rm -v algo_index_engine_data:/data alpine:3.22 \
  sh -c 'wc -l /data/execution/*.jsonl 2>/dev/null || echo "no postbacks yet"'
```

## Health

`health.mode` is `http`. The gate probes `/health` during the settle window.
`health.compose_service` selects the service whose rendered Compose mapping
supplies the probe host and port; `health.http_url` supplies the scheme and path
and is the fallback for contracts without `compose_service`.

Set `BLACKBOX_HTTP_PORT=8787` in the operator-owned file at `vps.env_file`.
This is the port the repository uses everywhere, locally and here, and it is not
a per-host choice: the Dhan app registration pins `DHAN_REDIRECT_URL` to it, so a
different port means browser login fails with a redirect mismatch. Docker reads
this file for Compose interpolation as well as container configuration. The
engine uses the same env port inside the container, and the host binding stays on
loopback. The API and built UI share that port. Reach it with
`ssh -N -L 8787:127.0.0.1:8787 beonedge`, then open `http://127.0.0.1:8787`.
The nginx vhost proxies to the same port, and every deploy refuses to continue if
the two disagree.

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
