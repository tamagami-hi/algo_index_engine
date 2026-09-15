# Dhan login

Call the single hybrid entry point after loading `.env`:

```rust
let credentials = dhan_api::dhan_auth::get_dhan_credentials().await?;
```

It reads `DHAN_AUTH_MODE`, selects the authentication method below, and returns
`DhanCredentials { client_id, api_key, access_token }`. Browser OAuth returns the
validated fresh or cached session as credentials; other modes package their token
with the configured client ID and API key.

Choose the token source in the repository `.env`:

| `DHAN_AUTH_MODE` | Source | Required settings |
| --- | --- | --- |
| `web` | Direct Dhan browser consent; automatic local callback and session reuse | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_API_SECRET`, `DHAN_REDIRECT_URL` |
| `token_url` | Cal Spread HTTPS token endpoint with a passcode header | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_TOKEN_URL`, `TOKEN_PASSCODE` |
| `manual` | An access token you supply | `DHAN_CLIENT_ID`, `DHAN_API_KEY`, `DHAN_ACCESS_TOKEN` |

An explicit mode always wins. `oauth` aliases `web`. With no mode configured,
a nonempty manual token wins, otherwise the token endpoint is used, preserving
the previous startup behavior. Unknown modes fail with a configuration error.

## Direct browser login

1. Create a Dhan API key/secret and register this exact redirect URL in Dhan:
   `http://127.0.0.1:8787/dhan/callback`.
2. Set `DHAN_AUTH_MODE=web`, fill `DHAN_API_SECRET`, and set
   `DHAN_REDIRECT_URL` to that registered URL. The callback supports an explicit
   port on numeric loopback HTTP (`127.0.0.1` or `[::1]`).
3. Run `cargo run -- --dhan-login` to force a new browser login and start the feed,
   or `cargo run` to use the configured authentication mode.
4. The app tries `xdg-open` on Linux and also prints the Dhan login link. Complete
   Dhan's credentials and 2FA in a browser on the same machine. The local callback
   receives `tokenId` automatically; no copy/paste is required. It waits five minutes.

Do not point this callback at the Cal Spread frontend: that frontend consumes the
single-use token itself. Use a Dhan app registered for this local callback.
If the callback port is occupied, stop the conflicting listener or register a
different port and update `DHAN_REDIRECT_URL` to match.

The session is atomically saved as `data/sessions/dhan_oauth.json`, with Unix
owner-only permissions. Startup reuses it only for the same client/API key and
while more than 60 seconds remain before Dhan's stated expiry. `--dhan-login`
forces a new browser login even if a session exists, then uses its credentials
to start the feed. This is
browser-assisted login: the app does not store or enter your PIN or OTP.

## Cal Spread token endpoint

Set `DHAN_AUTH_MODE=token_url` and `DHAN_TOKEN_URL=https://YOUR_HOST/api/dhan/token`.
Set `TOKEN_PASSCODE` to the backend's `TOKEN_ROUTE_SECRET` value. Requests send
`GET` with `x-token-passcode`; the passcode does not belong in the URL.

The admin must first connect Dhan in Cal Spread. Its endpoint returns
`authenticated`, `client_id`, `access_token`, `expires_at` (epoch milliseconds),
and `login_date`. A 403 indicates a bad passcode; 409 means no live Dhan session;
503 means the backend token route is not configured. The existing token cache
at `data/sessions/dhan_access_token.json` remains the fallback when a fetch fails,
provided the cached token is not expired or nearly expired. The two modes have
separate session files. No automatic switch to another auth mode occurs.

## Implementation and verification

Start with `mod.rs`: it loads a saved session or runs browser login, exchanges
its token, and saves the result. The remaining modules each have one job:

- `browser_callback`: bind the local server and return one validated `tokenId`.
- `server_callbacks`: generate consent and exchange the token with Dhan.
- `session`: load and save private, account-bound sessions.
- `types`: credentials, response data, and shared token validation.
- `source`: select web, token URL, or manual authentication.

The callback is validated once at the local server. The login flow consumes the
decoded token directly. Browser `get_credentials()` and forced `login()` both return `DhanCredentials`.
`source::get_dhan_credentials()` selects the mode and is re-exported by `dhan_auth`.

Tests use local mock HTTP endpoints and temporary session files; no broker login
or trade requests run during tests. Run `cargo test` and `cargo clippy`.
Validation: 28 passing tests and 83.91% line coverage for the OAuth module.
Clippy has two existing warnings outside the changed authentication code.
Live authentication requires your Dhan credentials and browser 2FA.

Sources: [Dhan authentication docs](https://dhanhq.co/docs/v2/authentication/),
the reference `algo_engine/backend_engine/src/zerodha_oauth`, and Cal Spread's
`src/brokers/dhan/auth.ts`, `src/brokers/routes.ts`, and frontend callback flow.
