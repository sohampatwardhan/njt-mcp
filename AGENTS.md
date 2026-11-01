# Agent notes

This repository is an MCP server for NJ Transit. Treat the upstream APIs as metered and the credentials as secret.

## Do not commit

- NJ Transit usernames, passwords, tokens, `state.json`, or cache directories.
- Captured NJ Transit responses, GTFS zips downloaded from NJ Transit, or protobuf dumps.
- The NJ Transit API PDFs, or prose copied from them. Behavior belongs in code and short original docs.
- Tests use the synthetic fixtures in `src/fixtures.rs` and `tests/common/support.rs` only.
- Do not call live NJ Transit APIs from CI or tests.
- Do not add a Dockerfile or document a container image. Releases are binaries, crates.io, and one npm package.

## Rate limits

- Do not add polling loops. Live tools are for a person asking a question.
- `getStationSchedule` is cached per station per New York day. The local cap is 4 upstream calls per day, under NJ Transit's limit of 5.
- Rail `getToken` is cached on disk (mode `0600`) so a restart does not burn the login budget. This server does not call `isValidToken`, because that call counts toward the same budget. The local mint cap is 8 per rail API per New York day.
- Bus tokens last about a day. The local mint cap is 4 per bus API per New York day.
- Live calls share a local cap of 2000 per New York day, under the published 40,000 current-data budget.

## Pull requests

- Agents never merge, tag, or publish this repository. Open a pull request and stop. Do not push `v*` tags.
- `Cargo.toml` and `npm/njt-mcp/package.json` must stay on the same version. `scripts/check-versions.sh` enforces that.
- Before opening a pull request, from a clean tree, run:
  - `bash scripts/check-versions.sh`
  - `cargo fmt --check`
  - `cargo clippy --locked --all-targets -- -D warnings -D clippy::missing_docs_in_private_items`
  - `cargo test --locked`
  - `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --locked`
  - `node --test npm/njt-mcp/test/release.test.js`
  - `cargo publish --dry-run --locked`

## Documentation

Every public and private item has rustdoc. The crate root has a module map and an example. Functions document `# Arguments`, `# Returns`, `# Errors`, `# Panics`, and `# Examples`. Examples that would open a socket or call NJ Transit are `no_run` and use the placeholder credential `replace-me`.

The crate denies:

- `missing_docs`
- `clippy::missing_docs_in_private_items`
- `rustdoc::missing_errors_doc`
- `rustdoc::missing_panics_doc`
- `rustdoc::broken_intra_doc_links`
- `rustdoc::private_intra_doc_links`

`missing_errors_doc` and `missing_panics_doc` are not shipped by every toolchain (they are absent on rustc 1.99). The crate allows `unknown_lints` so those two names stay denied wherever the compiler provides them. CI still fails on rustdoc warnings via `RUSTDOCFLAGS="-D warnings"`.

`[package.metadata.docs.rs]` builds private items and passes `-D warnings`.

The generated GTFS-Realtime module allows documentation lints. Its comments come from the vendored Apache-2.0 protobuf, not from this crate.
