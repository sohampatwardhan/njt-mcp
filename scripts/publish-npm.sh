#!/usr/bin/env bash
# Publish the unscoped njt-mcp package to npmjs.org when this version is
# not already there. Safe to rerun. Publishes only the unscoped package.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"$/\1/p' Cargo.toml | head -n 1)
name=$(python3 -c 'import json; print(json.load(open("npm/njt-mcp/package.json"))["name"])')
if [[ "$name" != "njt-mcp" ]]; then
  echo "npm/njt-mcp must stay the unscoped njt-mcp package (found ${name})" >&2
  exit 1
fi

# setup-node@v4 used to inject this when registry-url was set. npm 11 rejects it,
# and the paired placeholder token hides OIDC trusted publishing.
if [[ -n "${NPM_CONFIG_USERCONFIG:-}" && -f "$NPM_CONFIG_USERCONFIG" ]]; then
  grep -v '^always-auth=' "$NPM_CONFIG_USERCONFIG" > "${NPM_CONFIG_USERCONFIG}.tmp" || true
  mv "${NPM_CONFIG_USERCONFIG}.tmp" "$NPM_CONFIG_USERCONFIG"
fi
if [[ "${NODE_AUTH_TOKEN:-}" == "XXXXX-XXXXX-XXXXX-XXXXX" ]]; then
  unset NODE_AUTH_TOKEN
fi

if npm view "${name}@${version}" version >/dev/null 2>&1; then
  echo "${name}@${version} is already on npm"
  exit 0
fi

echo "publishing ${name}@${version}"
(cd npm/njt-mcp && npm publish --provenance --access public --ignore-scripts)
