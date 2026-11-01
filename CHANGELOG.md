# Changelog

## 0.1.0

- MCP server for NJ Transit TrainData, GTFSRT, GTFSG2, and BUSDV2.
- Credentials are `NJT_USERNAME` and `NJT_PASSWORD`, or `NJT_TOKEN`, or both. They are environment variables only.
- Per-API token cache, schedule cache, and local quotas under the published budgets.
- stdio and streamable HTTP. Tests use synthetic upstreams only.
- No container image.
- `cargo install njt-mcp --locked` installs the crate. A vendored `protoc` compiles the GTFS-Realtime schema, so a system protobuf compiler is not required.
- Tag `v0.1.0` builds macOS (arm64 and x64), Linux musl (x64 and arm64), and Windows x64 binaries and attaches them, plus `sha256sums.txt`, to the GitHub Release.
- npmjs.org publishes one package, unscoped `njt-mcp`. On first run it downloads the matching GitHub Release archive, verifies `sha256sums.txt`, and caches the binary. `NJT_MCP_BINARY` selects a local binary instead. `npx -y njt-mcp stdio` is the MCP command.
- The same launcher is also published to GitHub Packages as `@sohampatwardhan/njt-mcp`, from a temporary copy. The npmjs.org package name stays `njt-mcp`.
