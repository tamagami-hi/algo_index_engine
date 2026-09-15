# blackbox_trage

Box-spread arbitrage engine for Indian equity and index options, using Dhan (DhanHQ v2)
for authentication, the instrument master, and the live market feed.

Private project. Not a published crate, not a hosted service, no untrusted callers.

## Conventions

These apply to all code in this repository.

### 1. No comments

Do not add comments or doc comments unless explicitly asked for them. Write code that
reads on its own: name things clearly, keep functions short, prefer an obvious
implementation over a clever one that needs explaining.

This includes `//` comments, `///` and `//!` doc comments, and section-divider banners.

Existing comments stay unless removing them is requested.

One deliberate exception: the shell scripts under `release_manager/`. Each safety gate
there encodes a reason that is expensive to rediscover mid-incident, so they carry a
contract block per file and inline notes on the non-obvious refusals. The Rust source
stays comment-free.

### 2. Tests only where it matters

Do not write a test per function or a test file per source file. Test only highly
sensitive areas, where a silent failure would be expensive or hard to trace:

- authentication and token handling
- money, quantity, and strike arithmetic
- order construction and execution paths

Everything else goes untested by default. Prefer a few tests that cover real behaviour
over broad coverage of trivial code.

### 3. Keep it simple

Plain, direct Rust. No defensive hardening for threat models that do not apply here —
no input sanitising against hostile callers, no URL or size gauntlets, no permission
enforcement beyond what the OS gives by default. Skip abstraction layers and traits
until something concrete needs them.

## Running

```sh
cargo run                  # normal startup
cargo run -- --dhan-login  # force a fresh browser login
```

Configuration lives in `.env`. Paths resolve at runtime against `BLACKBOX_HOME`, which
defaults to the current working directory — so `cargo run` from the repository root finds
`.env` and `data/` as expected, and a container sets `BLACKBOX_HOME=/app`. A missing
`.env` is not an error: environment variables injected directly take precedence anyway,
which is how the container is configured.

## Containers

```sh
docker compose up --build -d
docker compose logs -f
```

The image is a two-stage build on `debian:bookworm-slim`, runs as a non-root user, and
carries no configuration or market data. `.dockerignore` keeps `.env`, `data/` and
`target/` out of the build context, so nothing secret is ever baked into a layer.

State lives in the `engine-data` named volume mounted at `/app/data`, holding the dated
instrument masters and the session file. Use a named volume rather than a bind mount:
the container runs as uid 10001, and a host bind mount would carry the host's ownership
and fail to write.

## Deployment

`release_manager/` holds the deployment pipeline. See `release_manager/README.md`.

```sh
./release_manager/provision.sh --engine   # once per host
./release_manager/export.sh    --engine   # build + stage a bundle
./release_manager/deploy.sh    --engine   # upload + deploy
./release_manager/status.sh    --engine   # local vs live
./release_manager/rollback.sh  --engine --list
```

Images are built here and shipped as tarballs; the VPS never compiles anything. Every
remote path comes from `release_manager/stacks/algo_engine/paths.json`, which is the sole
path authority.

`.env` on the server is placed and owned by the operator. No script in the pipeline
reads, writes, chmods or removes it.

Deploy to AWS `ap-south-1` (Mumbai). Dhan's infrastructure is in Mumbai, and a US region
adds roughly 200ms round trip, which is longer than the opportunities this strategy is
looking for.

### Authentication in a container

`web` mode cannot run headless — it shells out to `xdg-open` and waits on a loopback
callback that nothing can reach. Two workable options:

- `token_url`, the only mode that runs fully unattended.
- Bootstrap once: run `cargo run -- --dhan-login` locally, then copy
  `data/sessions/dhan_oauth.json` into the volume. The container reuses it until it
  expires, which for a Dhan token means within 24 hours.

## Authentication

A Dhan access token is valid for 24 hours and is saved under `data/sessions/`.
Startup reuses a saved token when one is still valid, so neither login route runs.
Only when there is no usable token does it fall back to `DHAN_AUTH_MODE`:

| `DHAN_AUTH_MODE` | Source | Needs |
| --- | --- | --- |
| `web` (or `oauth`) | Dhan browser consent, with a local callback | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_API_SECRET`, `DHAN_REDIRECT_URL` |
| `token_url` | Cal Spread token route, passcode in a header | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_TOKEN_URL`, `TOKEN_PASSCODE` |
| `manual` | A token you paste in yourself | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_ACCESS_TOKEN` |

An explicit mode always wins. With no mode set, a nonempty `DHAN_ACCESS_TOKEN` means
`manual`, otherwise `token_url`.

`manual` is never short-circuited by a saved token: a token you supplied is an
instruction, not a fallback. `--dhan-login` always forces a fresh browser login.

Two session files exist because the two routes save separately — `token_url` writes
`dhan_access_token.json`, `web` writes `dhan_oauth.json`. Both are checked, so switching
modes does not discard a token that is still good. A saved token is only reused when its
recorded client ID and API key match `.env`.

Register this redirect URL with your Dhan app for `web` mode:

```
http://127.0.0.1:8787/dhan/callback
```

## Instrument master

The instrument master is saved per date as `data/instruments/<YYYY-MM-DD>.csv`, using
the IST trading day. If today's file is already there it is reused and no download
happens, so repeated restarts cost nothing. The same date drives the option-chain expiry
filter, so the file and the chains can never disagree about which day it is.

## Layout

```
src/
  access_token/   token route fetch, expiry resolution, on-disk token cache
  config.rs       .env loading and BLACKBOX_HOME path resolution
  dhan_api/
    dhan_auth.rs  mode selection and saved-token reuse
    dhan_oauth/   browser consent flow and its session file
    instruments/  instrument master parsing, option chains, subscription planning
    dhan_ws.rs    live feed socket
    instrument_dl.rs  instrument master download
data/
  instruments/    instrument master CSV per date (gitignored)
  sessions/       saved tokens (gitignored, owner-only)
tests/            mirrors src, for the sensitive areas only
Dockerfile        two-stage release build
compose.yaml      engine service and its data volume
```
