#!/usr/bin/env bash
# Builds the launcher's installers into out/:
#
#   Linux    Voxear.Launcher_<version>_amd64.AppImage, which runs on any distro as new as the build machine's
#            (build on the oldest distro you support; CI uses Ubuntu 22.04), and a .deb
#   Windows  Voxear.Launcher_<version>_x64-setup.exe, a per-user setup program that needs no admin rights
#
# Usage: scripts/build-installers.sh [linux] [windows]
#
# With no arguments it builds what this machine can: on Linux the AppImage, plus the Windows setup when the cross
# tools are installed (makensis, cargo-xwin, clang and lld; cargo-xwin downloads the Windows SDK on its first run).
# On Windows, from Git Bash, it builds the setup.
#
# Signing for self-update: set TAURI_SIGNING_PRIVATE_KEY (the launcher update key, or a path to it) and
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD, and every installer gets a .sig beside it. Until the public key is built
# into voxear_launcher::self_update::LAUNCHER_UPDATE_KEY, set VOXEAR_LAUNCHER_UPDATE_KEY to it too, or the builds
# can't check their updates.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${OUT:-$root/out}"
target_dir="${CARGO_TARGET_DIR:-$root/app/src-tauri/target}"
version="$(sed -nE 's/^  "version": "([^"]*)",$/\1/p' "$root/app/src-tauri/tauri.conf.json")"

host=linux
case "${OS:-}$(uname -s)" in
  Windows_NT* | *MINGW* | *MSYS* | *CYGWIN*) host=windows ;;
esac

want=("$@")
if [[ ${#want[@]} == 0 ]]; then
  if [[ $host == windows ]]; then
    want=(windows)
  else
    want=(linux)
    if command -v makensis >/dev/null && command -v cargo-xwin >/dev/null; then
      want+=(windows)
    else
      echo "note: skipping the Windows setup; it needs makensis and cargo-xwin here, or a Windows machine" >&2
    fi
  fi
fi

# The launcher update key's public half: the one built into the core, or VOXEAR_LAUNCHER_UPDATE_KEY.
pubkey="${VOXEAR_LAUNCHER_UPDATE_KEY:-$(sed -n '/^pub const LAUNCHER_UPDATE_KEY/,/;$/p' \
  "$root/crates/voxear_launcher/src/self_update.rs" | grep -oE '"[A-Za-z0-9+/=]{40,}"' | tr -d '"')}"
if [[ -z "$pubkey" ]]; then
  echo "note: no launcher update key is built in, so these builds won't update themselves" >&2
fi

config=()
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  if [[ -z "$pubkey" ]]; then
    echo "error: signing needs the update key's public half, in LAUNCHER_UPDATE_KEY or VOXEAR_LAUNCHER_UPDATE_KEY" >&2
    exit 1
  fi
  # Tauri signs the installers only with a public key in its config; the app itself takes the key from the core.
  config=(--config src-tauri/tauri.release.conf.json --config "{\"plugins\":{\"updater\":{\"pubkey\":\"$pubkey\"}}}")
else
  echo "note: TAURI_SIGNING_PRIVATE_KEY isn't set, so the installers aren't signed and can't be published as updates" >&2
fi

cd "$root/app"
[[ -d node_modules ]] || npm ci

mkdir -p "$out"
# GitHub turns spaces in release asset names into dots, so the files are named that way here already, and the
# manifest's URLs match the uploaded assets.
collect() {
  local file
  for file in "$@"; do
    [[ -f "$file" ]] || continue
    local name
    name="$(basename "$file")"
    cp "$file" "$out/${name// /.}"
    echo "  ${out#"$root"/}/${name// /.}"
  done
}

for platform in "${want[@]}"; do
  case "$platform" in
    linux)
      [[ $host == linux ]] || { echo "error: the AppImage can only be built on Linux" >&2; exit 1; }
      npm run tauri build -- --bundles appimage,deb "${config[@]}"
      bundle="$target_dir/release/bundle"
      echo "Linux:"
      collect "$bundle/appimage/"*"_${version}_amd64.AppImage"{,.sig} "$bundle/deb/"*"_${version}_amd64.deb"
      ;;
    windows)
      if [[ $host == windows ]]; then
        npm run tauri build -- --bundles nsis "${config[@]}"
        bundle="$target_dir/release/bundle"
      else
        npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis "${config[@]}"
        bundle="$target_dir/x86_64-pc-windows-msvc/release/bundle"
      fi
      echo "Windows:"
      collect "$bundle/nsis/"*"_${version}_x64-setup.exe"{,.sig}
      ;;
    *)
      echo "error: unknown platform '$platform'; use linux or windows" >&2
      exit 2
      ;;
  esac
done

echo
echo "Next, list each signed installer in a channel (docs/INSTALLING.md, \"Making a release\"):"
echo "  cargo run -p voxear_release -- launcher add <installer> --site <site> --version $version --target <platform> --sig <installer>.sig"
