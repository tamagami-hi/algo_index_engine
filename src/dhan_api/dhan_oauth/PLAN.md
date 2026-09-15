# Dhan OAuth simplification

## PRD
Make browser login easier to read with fewer modules and no repeated callback parsing.
Preserve login modes, browser interaction, session reuse, and validation safeguards.

## Architecture and system design
`mod.rs` orchestrates login; `browser_callback` binds and validates callbacks;
`server_callbacks` talks to Dhan; `session` persists tokens; `types` holds data
and shared validation; `source` selects the authentication mode.

## Technical design
Return the decoded tokenId directly from the local callback listener. Remove the
pass-through automated_login helpers and Config getters. Share token validation
and expiry margin. Keep existing dependencies and private atomic persistence.

## Task list
- [x] Inspect existing code and tests; review callback extraction in installed Axum.
- [x] Get independent planning and security review.
- [x] Adapt callback contract tests and verify they fail before refactoring.
- [x] Simplify orchestration and callback validation.
- [x] Run tests, lint, coverage, and review the resulting changes.

Validation: 28 tests pass; OAuth line coverage is 83.91% (365/435).
Clippy passes with two existing warnings outside OAuth. Rustfmt checks pass for
the changed modules. Live Dhan login and browser 2FA were not exercised.

## Credentials return contract
Return `DhanCredentials` from browser authentication so the validated client,
API key, and access token travel together. Convert fresh and cached DhanSession
values through one owned conversion. The mode selector returns the same type
for manual and token URL modes, and dhan_auth re-exports that selector.
Startup uses forced-login credentials directly and propagates login failures.
Validate with contract tests, the mocked consent flow, Clippy, and OAuth coverage.
