# Tests

All Rust test implementations live here, mirroring the `src` directory structure.
Run the complete suite with `cargo test`.

Source modules include these files using `#[cfg(test)]` and `#[path]` so tests
retain access to private implementation details without exposing production APIs.
They remain unit-test modules in the binary's test harness; their names and
`cargo test` filters are unchanged. The mocked end-to-end consent flow lives in
`dhan_api/dhan_oauth/flow_tests.rs`.

Tests use local HTTP mocks and temporary files, not live broker credentials.
