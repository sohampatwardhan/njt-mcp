# njt-mcp

[![npm](https://img.shields.io/npm/v/njt-mcp)](https://www.npmjs.com/package/njt-mcp)
[![crates.io](https://img.shields.io/crates/v/njt-mcp)](https://crates.io/crates/njt-mcp)
[![docs.rs](https://docs.rs/njt-mcp/badge.svg)](https://docs.rs/njt-mcp)

MCP server for NJ Transit rail, bus, and light rail. The current release is 0.1.0. Source and the full guide: https://github.com/sohampatwardhan/njt-mcp

NJ Transit data terms apply. Register at https://developer.njtransit.com/registration and request access to each API. `replace-me` below is not a real credential.

## Install as an MCP server

```bash
npx -y njt-mcp --version
```

The package is a launcher with no install scripts. The first run downloads the binary for this version from GitHub Releases, checks `sha256sums.txt`, and caches it. `NJT_MCP_BINARY` points at a local binary and skips that download. `HTTPS_PROXY` (or `https_proxy`, then `HTTP_PROXY`) is used when set. `NO_PROXY` skips the proxy for matching hosts.

Serving MCP needs a transport. `stdio` speaks MCP on stdin and stdout. `http` serves streamable HTTP. `npx -y njt-mcp` with no subcommand exits. Credentials are environment variables only. There are no username or password flags.

```bash
NJT_USERNAME=replace-me NJT_PASSWORD=replace-me npx -y njt-mcp stdio
```

Or `NJT_TOKEN=replace-me`. A token-only process cannot mint a replacement when NJ Transit rejects the token.

| Variable | Required | Meaning |
| --- | --- | --- |
| `NJT_USERNAME` | with password | Developer-portal username. |
| `NJT_PASSWORD` | with username | Developer-portal password. Never logged. |
| `NJT_TOKEN` | or the pair above | Pre-minted token. Tried on every API. Never logged. |

### Claude Desktop

`claude_desktop_config.json` (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS, `%APPDATA%\Claude\claude_desktop_config.json` on Windows):

```json
{
  "mcpServers": {
    "njt-mcp": {
      "command": "npx",
      "args": ["-y", "njt-mcp", "stdio"],
      "env": {
        "NJT_USERNAME": "replace-me",
        "NJT_PASSWORD": "replace-me"
      }
    }
  }
}
```

### Claude Code

```bash
claude mcp add njt-mcp \
  -e NJT_USERNAME=replace-me \
  -e NJT_PASSWORD=replace-me \
  -- npx -y njt-mcp stdio
```

Token-only:

```bash
claude mcp add njt-mcp -e NJT_TOKEN=replace-me -- npx -y njt-mcp stdio
```

### Cursor

`.cursor/mcp.json` or `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "njt-mcp": {
      "command": "npx",
      "args": ["-y", "njt-mcp", "stdio"],
      "env": {
        "NJT_USERNAME": "replace-me",
        "NJT_PASSWORD": "replace-me"
      }
    }
  }
}
```

### VS Code

`.vscode/mcp.json` uses `servers` and `"type": "stdio"`:

```json
{
  "servers": {
    "njt-mcp": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "njt-mcp", "stdio"],
      "env": {
        "NJT_USERNAME": "replace-me",
        "NJT_PASSWORD": "replace-me"
      }
    }
  }
}
```

### Codex CLI

`~/.codex/config.toml`:

```toml
[mcp_servers.njt-mcp]
command = "npx"
args = ["-y", "njt-mcp", "stdio"]

[mcp_servers.njt-mcp.env]
NJT_USERNAME = "replace-me"
NJT_PASSWORD = "replace-me"
```

Token-only, same file, replace the env table with `NJT_TOKEN = "replace-me"`.

### Streamable HTTP

`http` listens on `127.0.0.1:8092`. MCP is `http://127.0.0.1:8092/mcp`. Liveness is `GET /healthz`. Put the credential variables on this process.

```bash
NJT_USERNAME=replace-me NJT_PASSWORD=replace-me npx -y njt-mcp http
```

## Other ways to install

```bash
cargo install njt-mcp --locked
```

Release archives and installers are on the [GitHub Releases](https://github.com/sohampatwardhan/njt-mcp/releases) page. `NJT_MCP_BINARY=/path/to/njt-mcp` uses that binary instead of the downloaded one.

The same launcher is published as `@sohampatwardhan/njt-mcp` on GitHub Packages. `npx -y njt-mcp` on npmjs.org is the usual install. To use the scoped package, add this to `.npmrc` (the token needs `read:packages`):

```text
@sohampatwardhan:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=YOUR_GITHUB_TOKEN
```

```bash
npx -y @sohampatwardhan/njt-mcp --version
```
