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

Configuration lives in `.env` at the manifest root. Copy `.env.example` and fill it in.

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

## Layout

```
src/
  access_token/   token route fetch, expiry resolution, on-disk token cache
  config.rs       .env loading
  dhan_api/
    dhan_auth.rs  mode selection and saved-token reuse
    dhan_oauth/   browser consent flow and its session file
    instruments/  instrument master parsing, option chains, subscription planning
    dhan_ws.rs    live feed socket
    instrument_dl.rs  instrument master download
data/
  instruments/    downloaded instrument master CSV (gitignored)
  sessions/       saved tokens (gitignored, owner-only)
tests/            mirrors src, for the sensitive areas only
```
