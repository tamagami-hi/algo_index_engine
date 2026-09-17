# algo_index_engine

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

The image builds the frontend and Rust binary in separate stages, uses
`debian:bookworm-slim` at runtime, runs as a non-root user, and
carries no configuration or market data. `.dockerignore` keeps `.env`, `data/` and
`target/` out of the build context, so nothing secret is ever baked into a layer.

State lives in the `engine-data` named volume mounted at `/app/data`, holding the dated
instrument masters and the session file. Use a named volume rather than a bind mount:
the container runs as uid 10001, and a host bind mount would carry the host's ownership
and fail to write.

## Deployment

`release_manager/` holds the deployment pipeline. See `release_manager/README.md`.

`./release_manager/status.sh` with no arguments is the control centre: it prints
what is built here, what is live there and whether they agree, then offers every
operation from one menu — build, ship, deploy, reload, roll back, ship the nginx
vhost, tail deploy logs, inspect containers, ask the engine its readiness,
diagnose the VPS, and run the offline suites. Opening it changes nothing, and
every action that restarts the engine says so before asking. The individual
scripts remain the entry points and keep working on their own:

```sh
./release_manager/status.sh                  # interactive control centre
./release_manager/status.sh --status         # dashboard only, read-only
./release_manager/status.sh --diagnose       # VPS tooling, paths, permissions
./release_manager/status.sh --verify         # offline suites
./release_manager/status.sh --cut-release    # bump the version, commit, tag, push
./release_manager/provision.sh --engine      # once per host
./release_manager/export.sh    --engine      # build + stage a bundle
./release_manager/deploy.sh    --engine      # upload + deploy
./release_manager/rollback.sh  --engine --list
```

Images are built here and shipped as tarballs; the VPS never compiles anything. Every
remote path comes from `release_manager/stacks/index_engine/paths.json`, which is the sole
path authority.

`.env` on the server is placed and owned by the operator. The pipeline does not
modify or copy it; release manifests record its digest to detect configuration drift.

### Versions and cutting a release

`Cargo.toml` is the only authority for the version. `export.sh` labels a bundle
`X.Y.Z` when the tree is clean and HEAD is exactly the tag `vX.Y.Z`, and
`<next patch>-dev.<commits>.g<sha>[.dirty]` otherwise — the patch is bumped for a
dev label so it always sorts after the release it descends from. `--status` prints
the label the next export would produce, so you can check before spending a build.

`status.sh --cut-release` is the only place the version advances and the only place
a tag is made. It offers patch, minor, major, or tagging the current version as it
stands, then:

- refuses anything but a clean `main` in step with `origin/main`, and refuses a tag
  that already exists locally or on origin
- writes `Cargo.toml`, regenerates `Cargo.lock`, and writes `web/package.json` and
  `web/package-lock.json` including the lock's own `packages[""].version`
- proves the result with `cargo check --locked`, which is what CI runs, and restores
  every file if any step fails
- commits `chore(release): vX.Y.Z`, makes an annotated tag, and pushes the branch and
  the tag in one `git push --atomic`

The lock file is the reason this is scripted rather than a manual edit. `Cargo.lock`
records the package's own version, and CI runs clippy, tests and the release build
with `--locked`, which refuses to update it — so a bump that edits only `Cargo.toml`
fails every Rust job. `npm ci` likewise refuses a tree where `package.json` and
`package-lock.json` disagree. Four files, two of which break the build when they
drift.

`export.sh` and `deploy.sh` make no git writes at all; they only read the tag.

## Web interface and access

The engine serves its React interface and HTTP API from the same process. Build the
interface with `cd web && npm ci && npm run build`; containers include it already.
Native runs require `BLACKBOX_HTTP_ADDR`; the env example derives it from
`BLACKBOX_HTTP_PORT`. Docker requires `BLACKBOX_HTTP_PORT` from `.env` for both
the host and container port (`8787`, the same everywhere), with
host publishing bound only to `127.0.0.1`. There are no code or Compose port
defaults. Compose sets the listener to `0.0.0.0:${BLACKBOX_HTTP_PORT}`; the UI and API use this
one port. In `web` mode, `/dhan/callback` is served by this same backend listener;
no separate callback port is opened. Both examples derive `DHAN_REDIRECT_URL`
from `BLACKBOX_HTTP_PORT`, so changing that value moves the UI/API and callback
together. Register the expanded redirect URL with Dhan after changing the port.

For the VPS, use `release_manager/stacks/index_engine/.env.example` as the
reference for the operator-owned file at `vps.env_file` in the authoritative
`paths.json` (currently `/srv/dev_stack/ALGO_INDEX_ENGINE/index_engine/.env`).
The deployment health probe follows the configured Compose host port. After
changing it, recreate the container through deployment; no image rebuild is needed.
Connect using
`ssh -N -L 8787:127.0.0.1:8787 beonedge` and open `http://127.0.0.1:8787`.
`8787` is the port this repository uses locally and on the VPS. It is not a
per-environment setting: the Dhan app registration pins the callback URL to it, so
moving it on one side breaks browser login. If nginx is enabled, its `proxy_pass`
must name the same port: nginx cannot read the env file, so it is the one
place the number appears twice, and every deploy resolves the port Compose
publishes and refuses to continue if a `proxy_pass` names a different one.

### The hostname

`release_manager/nginx/` holds the vhost that serves the engine at
`index.algo.boe.internal`. A deploy stages it to
`/srv/dev_stack/ALGO_INDEX_ENGINE/nginx` and prints exactly what `/etc/nginx`
still needs; it installs nothing itself, because that needs root and a bad file
there takes down every site on the box at once. `./release_manager/status.sh` →
Edge ships it on its own, which is worth doing when the vhost is the only change.

`.internal` is ICANN-reserved, so nothing resolves it and no public CA will
certify it. Each machine needs a hosts entry — `127.0.0.1` on the VPS, the VPS's
tailnet address on anything you browse from — and the site is plain HTTP with the
tailnet as its access control. That is acceptable here only because the surface
has no login form and no password to intercept; it is not open, and the
`allow 100.64.0.0/10` / `deny all` guard is asserted on every deploy.

For frontend development, `cd web && npm run dev` serves the interface on `5178`
and proxies `/api`, `/health` and `/ready` to the engine. It reads
`BLACKBOX_HTTP_ADDR` from the repository `.env`, expanding `${BLACKBOX_HTTP_PORT}`
the way Compose does, so the port comes from the same file as everything else
rather than from an exported shell variable. Without that file the dev server
still starts and says on stdout that it is not proxying.

The interface shows market telemetry, option chains, and saved strategy definitions
with entry blockers. Live order routing is not implemented. `/health` reports process
liveness and, in `profile`, whether the running binary was compiled `release` or
`debug`; `/ready` returns 503 with reasons when the engine lacks usable market data.
By default, SSE updates publish at most every 50 ms, while the engine processes every
feed frame. `BLACKBOX_PUBLISH_INTERVAL_MS` configures the publish interval.

Access control belongs at the Tailscale/nginx edge. Deployment checks require loopback
port bindings and, when configured, a restrictive nginx vhost. The edge configurations
and installation instructions live in [release_manager/nginx](release_manager/nginx/).
The separate public-site stack remains disabled. See
[the deployment guide](release_manager/README.md).

### Order postbacks

`POST /dhan/postback` receives Dhan's order status notifications. It stores each
body verbatim and answers `200` only once the bytes are on disk; a webhook reads
any other status as a delivery failure and this endpoint must not claim to hold
something it dropped. Records append to `data/execution/postbacks-<day>.jsonl`,
one JSON line per arrival carrying `received_at_ms`, `source`, `bytes` and the
unmodified `body`. Bodies over 16 KiB are refused with `413`, and the transport
itself stops reading past 64 KiB.

Nothing interprets the payload yet. Parsing belongs with the live order execution
mechanics, which do not exist, and guessing now how orders are keyed and
reconciled would mean writing that guess into the journal. A postback is the
broker's only unsolicited account of a real-money event, sent once per status
change with no way to request it again, so the payload is kept exactly as sent
and whatever parses it later parses the broker's own words. Arrivals are counted
in `postbacks` on `/api/state`.

`DHAN_POSTBACK_URL` in the env file records the URL you register in the Postback
URL field on `web.dhan.co` when generating an access token. The engine does not
register it for you; it validates it. Startup refuses a URL whose path is not
`/dhan/postback`, since the engine serves postbacks nowhere else and any other
path answers 404 while real order events are lost, and refuses a loopback URL
whose port is not the backend's. The variable may be left empty, and the engine
still serves the route.

Two things must be settled before this can receive live traffic. Dhan will not
deliver to a `localhost` or `127.0.0.1` postback URL, and this engine listens on
loopback behind a tailnet-only vhost, so nothing public routes to it today. Dhan
also signs nothing: no HMAC, no shared secret, no header to verify. Making the
endpoint publicly reachable therefore creates an unauthenticated public write
path, which needs a deliberate decision rather than a configuration change.

Deploy to AWS `ap-south-1` (Mumbai). Dhan's infrastructure is in Mumbai, and a US region
adds roughly 200ms round trip, which is longer than the opportunities this strategy is
looking for.

### Authentication in a container

`web` mode works through the backend's callback route. Keep
`ssh -N -L 8787:127.0.0.1:8787 beonedge` running on your browser's machine.
Open the Dhan consent URL printed in the engine logs and complete login within
five minutes. Your browser redirects to `http://127.0.0.1:8787/dhan/callback`,
which the tunnel forwards to the same backend that serves the UI/API. `/health`
remains available while authentication is pending; `/ready` stays unavailable.
The engine saves and reuses the resulting session. When a fresh login is needed,
complete browser consent again. `token_url` remains available for fetching an
existing session token without interactive browser login.

## Authentication

A Dhan access token is valid for 24 hours and is saved under `data/sessions/`.
Startup reuses a saved token when one is still valid, so neither login route runs.
Only when there is no usable token does it fall back to `DHAN_AUTH_MODE`:

| `DHAN_AUTH_MODE` | Source | Needs |
| --- | --- | --- |
| `web` (or `oauth`) | Dhan browser consent, callback on the backend listener | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_API_SECRET`, `DHAN_REDIRECT_URL` |
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

Register the expanded `DHAN_REDIRECT_URL` with your Dhan app for `web` mode.
One URL covers both local runs and the VPS, because both use port 8787:

```
http://127.0.0.1:8787/dhan/callback
```
The standalone `cargo run -- --dhan-login` command still owns its own temporary
listener; stop the backend before using it on the same port. For Docker/VPS use
the running backend's login flow described above.

## Instrument master

The instrument master is saved per date as `data/instruments/<YYYY-MM-DD>.csv`, using
the IST trading day. If today's file is already there it is reused and no download
happens, so repeated restarts cost nothing. The same date drives the option-chain expiry
filter, so the file and the chains can never disagree about which day it is.

## CI and verification

[The CI workflow](.github/workflows/ci.yml) runs on pushes to `main`, pull requests,
and manual dispatch. It checks Rust formatting, Clippy with warnings denied, tests,
and a release build for `x86-64-v3`; frontend types, tests with coverage, and production
build; shell syntax, ShellCheck, and offline rollback/access-control regressions.
It also builds the container and checks that it serves liveness, reports an optimised
release build, refuses readiness without broker credentials, receives and journals an
order postback, refuses an oversized one, and includes the frontend. No deployment is
performed.

Rust LCOV and frontend coverage summaries are uploaded as workflow artifacts.
Coverage is reported without a percentage gate. The recorded Rust baseline at
commit `36cf54b` is 58.61% lines and 57.48% regions; frontend coverage is
52.89% lines/statements. These figures do not meet the 80% target. Tests currently
focus on quote validity and freshness, strategy constraints, task supervision,
broker/session handling, and the operator's view of stale data.

Run the checks locally with:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo llvm-cov --locked --all-features --summary-only
npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web run coverage
npm --prefix web run build
find release_manager -name '*.sh' -print0 | xargs -0 shellcheck -x -P SCRIPTDIR -S warning
bash release_manager/tests/rollback_pairing.sh
bash release_manager/tests/access_control.sh
bash release_manager/tests/port_configuration.sh
bash release_manager/tests/nginx_ship.sh
bash release_manager/tests/release_profile.sh
bash release_manager/tests/version_bump.sh
bash release_manager/tests/unbound_variables.sh
cargo build --locked --release
```

`release_manager/status.sh --verify` runs the shell suites and offers the Rust and
frontend ones, ending with a release build. The image is compiled with
`cargo build --release`, so a release-only failure that the debug test build never
sees would otherwise surface during `export.sh`, after the version label has already
been advanced.

The binary reports its own profile from `cfg!(debug_assertions)`, on startup and on
`/health`. `export.sh` refuses to bundle an image whose transcript does not say
`release`, and CI asks the running container the same question. A debug build starts,
serves, deploys and rolls back correctly while running the option-chain hot path
several times slower than it was measured at, so nothing else in the pipeline would
notice.

Coverage requires `cargo-llvm-cov` and the Rust `llvm-tools-preview` component;
deployment checks require Bash, ShellCheck, and jq. Broker tests use local mocks.
The container smoke test verifies startup without credentials, not live broker
connectivity or order execution.

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
  option_chain/  chain book, metrics, and quote freshness checks
  risk_engine/   strategy definitions, persistence, and entry resolution
  server/        HTTP/SSE, engine state, and task supervision
data/
  instruments/    instrument master CSV per date (gitignored)
  sessions/       saved tokens (gitignored, owner-only)
  strategies/     saved strategy definitions
  state/          active strategy IDs
tests/            mirrors src, for the sensitive areas only
web/src/          React interface and stores
web/tests/        frontend behaviour tests mirroring web/src
.github/workflows/ci.yml  automated verification
Dockerfile        frontend and Rust release build
compose.yaml      engine service and its data volume
```
