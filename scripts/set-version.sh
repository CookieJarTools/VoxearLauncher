#!/usr/bin/env bash
# Sets the launcher's version everywhere it is written: the workspace, the window's crate, its Tauri config,
# and its package.json and lock file. The window's updater and the core's own version must agree, or an update
# would be offered again after it was installed.
#
#   scripts/set-version.sh 0.2.0
set -euo pipefail

version="${1:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: scripts/set-version.sh <version>, such as 0.2.0 or 0.2.0-beta.1" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"

# Each pattern matches one line: the package's own version, never a dependency's.
replace() {
  local file="$1" pattern="$2" line="$3"
  local count
  count="$(grep -cE "$pattern" "$file" || true)"
  if [[ "$count" != 1 ]]; then
    echo "error: expected one version line in $file, found $count" >&2
    exit 1
  fi
  sed -i -E "s|$pattern|$line|" "$file"
}

replace "$root/Cargo.toml" '^version = "[^"]*"$' "version = \"$version\""
replace "$root/app/src-tauri/Cargo.toml" '^version = "[^"]*"$' "version = \"$version\""
replace "$root/app/src-tauri/tauri.conf.json" '^  "version": "[^"]*",$' "  \"version\": \"$version\","
# package.json and its lock file.
(cd "$root/app" && npm version "$version" --no-git-tag-version --allow-same-version >/dev/null)
echo "The launcher is now version $version."
