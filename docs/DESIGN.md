# Voxear Launcher design

The Voxear Launcher is the equivalent of Unity Hub for the [Voxear](https://github.com/CookieJarTools/Voxear)
engine. It installs engine versions side by side, keeps itself up to date, lists projects and opens each one with
the engine version it is pinned to, and creates new projects from templates.

This document defines the formats the launcher and the engine share: the release manifest, the install layout, the
project pin and the `vox new` contract. The engine's side of the contract is summarised in "Versions, installs
and the launcher" in the engine's `docs/ARCHITECTURE.md`; this document is the detailed version.

## Decisions

Settled on 2026-10-30:

- **Repository:** `CookieJarTools/VoxearLauncher`, private, next to the engine.
- **UI:** a Rust core with a web UI through **Tauri 2**, the same stack as Whisk (React 19, Vite, TypeScript).
  - It ships before engine milestone M3, which is when the first engine release needs installing.
  - The Linear look, OKLCH themes and short animations are cheap in CSS.
  - It still opens on a machine whose GPU lacks Vulkan 1.3, so it can explain what is missing.
  - The second UI stack is confined to this one app. The core has no UI in it, so the front end could move to
    Voxear UI later without touching the logic.
- **Command line:** `voxup`. It is short, like `rustup`, which also installs toolchains side by side with
  stable, beta and nightly channels.
- **Licence:** PolyForm Shield 1.0.0, the same as the engine.
- **Platforms:** Linux (Wayland and X11) and Windows 10/11, on x86-64.

## Principles

The engine's principles apply here as well. Three matter most:

1. **Easy path and advanced path.** `voxup install stable` and the window's Install button take no
   decisions. Exact versions, ranges, offline archives, located engines, mirrors and extra keys are all there
   when they are needed.
2. **Nothing half done.** An engine folder is either complete and verified, or it does not exist. A project file
   is never half written.
3. **One implementation.** The command line and the window call the same `voxear_launcher` functions. Projects
   are only ever created by the engine's own `vox new`.

## Architecture

```
 ┌────── Launcher window (Tauri) ──────┐   ┌──────── voxup (CLI) ────────┐
 │ React UI · Tauri commands + events  │   │ clap · progress bars        │
 └──────────────────┬──────────────────┘   └──────────────┬──────────────┘
 ┌──────────────────┴────────── voxear_launcher (core) ───┴──────────────┐
 │ manifest · releases · signing · fetch · archive · store · registry    │
 │ project · new_project · settings · layout                             │
 └──────────────────┬───────────────────────────────────┬────────────────┘
          release mirror (https / file)        engine installs: bin/vox new …
```

```
crates/
  voxear_launcher   the core: everything the launcher does, with no UI
  voxup             the command line
app/                the Tauri window (planned)
docs/               this document
```

The core is synchronous and blocking. Long operations take a progress callback and a cancel flag, so the window
runs them on a worker thread and forwards progress through a Tauri channel.

## Engine release manifest

Every published engine version is listed in one file, `releases.toml`, signed as a whole by
`releases.toml.minisig` next to it.

```toml
format = 1
generated = "2026-10-30T12:00:00Z"

[[release]]
version = "0.3.0"
channel = "stable"
published = "2026-10-30T12:00:00Z"
notes = "https://…/releases/0.3.0"
min_launcher = "0.1.0"          # optional

[[release.download]]
platform = "linux-x86_64"
url = "voxear-0.3.0-linux-x86_64.tar.zst"
size = 412345678
sha256 = "5c8e89dc7cf977cb9164442b449d51333bd8a8b97a521ef309e5c258525a42f7"
signature = """
untrusted comment: signature from minisign secret key
RUQ…
trusted comment: timestamp:1793361600	file:voxear-0.3.0-linux-x86_64.tar.zst
…
"""

[[release.download]]
platform = "windows-x86_64"
url = "voxear-0.3.0-windows-x86_64.tar.zst"
size = 398765432
sha256 = "…"
signature = """…"""
```

| Field | Meaning |
|---|---|
| `format` | The manifest format. The launcher refuses a higher format and asks to be updated. |
| `generated` | When the manifest was written (RFC 3339). It must increase with every publish. |
| `release.version` | Semver. Each version appears once. |
| `release.channel` | `stable`, `beta` or `nightly`. It must fit the version (see below). |
| `release.published` | When the release was published. |
| `release.notes` | A link to the release notes. Optional. |
| `release.min_launcher` | The oldest launcher that can install it. Optional. Older launchers list the release but explain that they need updating. |
| `release.yanked` | `true` for a pulled release. A channel or range never picks it, but an exact version still installs it, so projects pinned to it keep opening. |
| `download.platform` | `linux-x86_64` or `windows-x86_64`. Kept as text, so a manifest can add platforms that older launchers ignore. |
| `download.url` | Absolute, or relative to the manifest's own URL. Relative URLs let a whole release folder be mirrored as it is. |
| `download.size` | Bytes. The download stops if it exceeds this and fails if it falls short. |
| `download.sha256` | The archive's SHA-256, in hex. |
| `download.signature` | The archive's whole `.minisig` file. |

Unknown fields are ignored, so new optional fields don't need a new format number.

**Versions and channels.**
- Stable versions have no pre-release tag: `0.3.0`.
- Beta versions have any tag except `nightly`: `0.4.0-beta.2`, `0.4.0-rc.1`.
- Nightly versions use the date: `0.4.0-nightly.20261030`.

**Choosing a release.** `voxup install` and the window take a selector:

| Selector | Picks |
|---|---|
| `stable` (or `latest`), `beta`, `nightly` | The newest release in that channel. |
| `0.3.1` | Exactly that version, even if it was yanked. |
| `0.3`, `>=0.3, <0.5` | The newest release in the range. As in Cargo, `0.3` means `^0.3`, and pre-releases only match a range that names one. |

Releases without a download for this platform are skipped.

**Nightly pruning.** The publisher keeps only the last 30 nightlies in the manifest. Projects pinned to an older
nightly still open while it is installed.

## Signing and verification

Everything is signed with **minisign** keys: Ed25519 over a BLAKE2b pre-hash. They can be made with the
`minisign` tool, and checked by the pure-Rust `minisign-verify` crate. Tauri's updater uses the same kind of key.

**What is signed.**
- The manifest, by `releases.toml.minisig`.
- Each archive. Its signature is embedded in the manifest and also published next to it as
  `<archive>.minisig`, so an archive can be installed offline (`voxup install --file`) without the manifest.

**Order of checks when installing from the manifest.** Nothing is unpacked until every check has passed.
1. The manifest's signature, against the trusted keys.
2. The manifest's `generated` time must not be older than the last manifest seen from the same URL, so an old
   manifest can't be replayed to hide a newer release (a rollback attack).
3. While downloading: the size never exceeds `size`, and the SHA-256 is computed.
4. The size equals `size`, and the SHA-256 equals `sha256`.
5. The archive's own signature.
6. After unpacking into staging: `engine.toml` must name the same version, channel and platform as the manifest.

**Trusted keys.**
- The engine release keys are built into the launcher, in `signing::ENGINE_RELEASE_KEYS`.
- A signature names the key that made it, so rotating keys means adding the new key in one launcher release
  and retiring the old one later.
- `trusted_keys` in the settings adds more keys, for self-hosted engine builds.
- The launcher's own updates use a **separate key**, so a leaked engine key can't push a launcher update, and
  the reverse.

**Signing keys (to do).** Neither key exists yet, and until the engine key does, only keys in the settings are
trusted. The secret keys belong offline or in CI secrets, never in a repository.

```bash
minisign -G -p voxear-engine.pub -s voxear-engine.key
```

```bash
npm run tauri signer generate -- -w ~/.tauri/voxear-launcher.key
```

## Engine archives and `engine.toml`

An engine release for one platform is a `.tar.zst` named `voxear-<version>-<platform>.tar.zst`. Its entries sit at
the root (with no wrapping folder) and are files and folders only. The launcher refuses links and any path that
leaves the install folder.

```
engine.toml                 what this install is (below)
bin/vox[.exe]               the command line
bin/voxear-editor[.exe]     the editor (from M3)
managed/Voxear.Core.dll     the C# API, and whatever else the managed side needs
templates/<id>/…            project templates, applied by `vox new`
docs/                       offline documentation
```

Every engine install, including a build from source that the launcher is pointed at, has an `engine.toml` at its
root:

```toml
format = 1
version = "0.3.0"
channel = "stable"
platform = "linux-x86_64"
commit = "0ffa698"          # optional
```

The launcher reads this file and never runs anything to find out what an install is.

## Install layout

```
~/.local/share/voxear/          %LOCALAPPDATA%\Voxear on Windows; VOXUP_HOME overrides
  engines/<version>/            one folder per version: that version's VOXEAR_HOME
    .voxup-receipt.toml         written last, inside staging: its presence marks a finished install
  located.toml                  engines elsewhere that the launcher was pointed at
  projects.toml                 the project list
  settings.toml                 launcher settings
  cache/                        the last verified manifest per URL, with its signature, for offline use
  downloads/                    archives while they download (*.part)
  staging/                      archives while they unpack
  trash/                        engines being removed
  store.lock                    held while installs change
```

**Installing.** The archive downloads into `downloads/` and is checked as described above. It then unpacks into a
fresh folder in `staging/`, gets its receipt, and is renamed into `engines/<version>` in one step. Staging and
engines share a folder, so the rename is atomic. A killed launcher leaves only staging and download leftovers, and
the next install clears them.

**Receipts.**

```toml
version = "0.3.0"
channel = "stable"
platform = "linux-x86_64"
source = "https://…/voxear-0.3.0-linux-x86_64.tar.zst"
sha256 = "…"
installed = "2026-10-30T16:42:13Z"
```

A folder in `engines/` with no receipt, or whose name doesn't match its receipt's version, is not an install. If an
install needs that folder, it is moved to the trash first.

**Removing.** The folder is renamed into `trash/` and then deleted, so a half-deleted engine never looks
installed. On Windows the rename fails while the editor or a game is running from that install, and the launcher
says so. If projects in the list are pinned to the version, `voxup uninstall` lists them and needs `--force`.

**Located installs.** `voxup locate <folder>` records an engine that lives elsewhere, such as one built from
source. It needs an `engine.toml` and `bin/vox`.
- The launcher never changes or deletes a located engine's files. Uninstalling one only forgets it.
- `located.toml` keeps each one's path, version and channel, so an engine on a drive that isn't mounted still
  shows what it was, marked as missing.
- When its folder is there, what `engine.toml` says now wins.

**One writer at a time.** Installing, removing and locating hold an OS file lock on `store.lock`, so the window
and `voxup` can't change installs at the same time. A launcher that finds the lock held waits briefly, then says
another launcher is busy. Listing never takes the lock.

**Running an engine.** Every process the launcher starts from an install gets `VOXEAR_HOME` set to the install
folder.

## Projects

**The pin.** `voxear.toml` records the engine version a project uses:

```toml
[project]
name = "Orbit"
engine = "0.3.0"
scripts = "Scripts/Orbit.Scripts.csproj"
```

`engine` is an exact semver version, never a range, like Unity's `ProjectVersion.txt`. `vox new` writes it with its
own version. A project without it, such as `samples/Hello` today, shows as "not pinned".

**The list.** `projects.toml` records where projects are, when they were added and opened, and which are
favourites. The name and engine version are read from each `voxear.toml` whenever the list is shown, so the list
can't disagree with the project. The name is also cached, so a project on a drive that isn't mounted still shows
what it was.

```toml
[[project]]
path = "/home/jeo/Games/Orbit"
name = "Orbit"
added = "2026-10-30T16:42:29Z"
opened = "2026-10-31T09:12:03Z"
favourite = true
```

Each project in the list has one status:

| Status | Shown as | Action offered |
|---|---|---|
| Ready | its engine version | Open |
| Engine missing | "needs Voxear 0.2.0" | Install 0.2.0, then open |
| Not pinned | "not pinned" | Choose a version (the pin is written by `vox upgrade`; see below) |
| Missing | "folder not found" | Locate or remove from list |
| Unreadable | the parse error | Show the file |

The list is sorted with favourites first, then by last opened (or added). Removing a project from the list never
touches its files.

**Opening.** The launcher resolves the pinned version and starts `<install>/bin/voxear-editor --project <folder>`
with `VOXEAR_HOME` set. If the version isn't installed, it offers to install it first (`voxup open --install`).
The editor arrives at M3, so for now opening explains that the version has no editor.

**Upgrading (planned).** Moving a project to another engine version:
1. Make sure the target version is installed.
2. **Back up the project first**: a `.tar.zst` of the project folder in `backups/<project>/<time>-<old version>.tar.zst`
   in the launcher folder. It leaves out regenerable folders (`.voxear/`, `bin/`, `obj/`). The window lists
   backups with Restore and Show buttons.
3. Run the target version's `vox upgrade <folder> --from <old> --json`. That migrates the project files and
   rewrites `engine`, printing NDJSON events. Each `{"event":"change","message":…}` becomes a line in the
   summary.
4. Show the summary, with links to the release notes of every version in between (from the manifest's `notes`).

Moving to an older version is refused unless asked for explicitly, with a warning that it may not open.

## New-project flow

The launcher never writes project files. It asks the chosen engine version what it offers, builds the form from
the answer, and runs that version's `vox new`. The launcher, `vox` and the editor therefore create identical
projects, and a newer engine can add a template or setup option without a launcher update.

**The form.**
- Name and location. The name is also the folder name, so it must be valid on Linux and Windows: no
  `/ \ : * ? " < > |` and no control characters, at most 64 characters, no trailing full stop, and none of the
  device names Windows reserves. The folder must not exist, or must be empty.
- Template: Empty, 3D, 2D, First-person, Third-person or Top-down, shown as a gallery with thumbnails. 3D is
  preselected.
- Engine version: the newest stable install by default.
- Default setup: lighting and sky, camera, input actions, a sample scene and quality presets. Each is a toggle,
  preset from the template.
- Optional extras, collapsed by default: C# IDE setup (Rider, Visual Studio or VS Code), git with a `.gitignore`,
  modding switched on, and a Steam App ID.

### `vox new --describe --json`

Prints what this engine version offers, as one JSON object:

```json
{
  "format": 1,
  "templates": [
    { "id": "empty", "name": "Empty", "description": "Nothing but a camera", "setup": ["camera"] },
    { "id": "3d", "name": "3D", "description": "A lit 3D scene", "thumbnail": "templates/3d/thumbnail.png",
      "setup": ["lighting", "camera", "input", "sample-scene", "quality"] }
  ],
  "setup": [
    { "id": "lighting", "name": "Lighting and sky", "description": "A directional light, sky and fog" },
    { "id": "camera", "name": "Camera" },
    { "id": "input", "name": "Input actions" },
    { "id": "sample-scene", "name": "Sample scene" },
    { "id": "quality", "name": "Quality presets" }
  ],
  "ides": [
    { "id": "rider", "name": "Rider" },
    { "id": "visual-studio", "name": "Visual Studio" },
    { "id": "vscode", "name": "VS Code" }
  ]
}
```

- `templates[].setup` lists the setup items the template turns on by default.
- `thumbnail` is relative to the install folder.
- The launcher refuses a higher `format`. Unknown fields are ignored.

The first releases offer these IDs:

| Kind | IDs |
|---|---|
| Templates | `empty`, `3d`, `2d`, `first-person`, `third-person`, `top-down` |
| Setup | `lighting`, `camera`, `input`, `sample-scene`, `quality` |
| IDEs | `rider`, `visual-studio`, `vscode` |

### `vox new <folder> …`

```
vox new <folder> --name <name> --template <id>
        [--setup <id>,<id>… | --setup none]   # default: the template's own setup
        [--ide <id>]
        [--git]                               # git init, .gitignore, .gitattributes
        [--modding]                           # [mods] enabled = true
        [--steam-app-id <n>]                  # project settings; steam_appid.txt handled by the engine
        [--json]
```

- `<folder>` is the project folder itself, created by `vox new`. The launcher passes `<location>/<name>`.
- With `--json`, `vox new` prints one JSON object per line on stdout while it works:

  ```
  {"event":"step","message":"Copying the 3D template"}
  {"event":"step","message":"Writing input actions"}
  {"event":"done","path":"/home/jeo/Games/Orbit"}
  ```

  On failure it prints `{"event":"error","message":"…"}` and exits with a non-zero code. Anything else, such as
  log output, goes to stderr.
- The new `voxear.toml` has `engine` set to this `vox`'s own version.

The launcher adds the new project to the list, shows each step as it arrives, and offers to open it.

## Launcher self-update

The launcher updates itself from **its own manifest, signed with its own key**, separately from engine versions.

**Packaging.**
- Linux: an AppImage, the only Linux format Tauri's updater can replace in place. If the launcher was installed
  by a package manager (such as an AUR package), self-update is switched off at build time, and the window says
  that updates come from the package manager.
- Windows: a per-user NSIS installer, so no admin rights are needed. The updater runs the new installer in passive
  mode after the launcher closes.

**Manifest.** Tauri's updater reads a static JSON file per channel: `launcher/stable.json` and
`launcher/beta.json`.

```json
{
  "version": "0.2.0",
  "notes": "What changed, in Markdown",
  "pub_date": "2026-11-15T12:00:00Z",
  "platforms": {
    "linux-x86_64": { "url": "https://…/VoxearLauncher_0.2.0_amd64.AppImage", "signature": "<base64 of the .sig>" },
    "windows-x86_64": { "url": "https://…/VoxearLauncher_0.2.0_x64-setup.exe", "signature": "…" }
  }
}
```

The updater only moves to a higher version, which rules out rollback. The public key is compiled in through
`tauri.conf.json`.

**Flow.**
- The launcher checks when it opens and every six hours while it is open. A quiet card in the sidebar says when
  an update is ready, the way The Jar does it.
- **Update** downloads and verifies the update in the background, then **Restart now** applies it.
- Engine installs in progress block the restart until they finish.

**The command line.** `voxup` is built into the launcher binary: called with a subcommand, it runs without
opening a window. Settings › "Command line" puts a `voxup` shim on the PATH: `~/.local/bin/voxup` on Linux, and
`%LOCALAPPDATA%\Voxear\bin\voxup.cmd` on Windows, added to the user's PATH. The shim calls the launcher, so
launcher updates update `voxup` too. A standalone `voxup` binary is also published, for CI and headless machines.
`voxup self update` (planned) updates a standalone `voxup` from the same manifest, using the same signature check.

## The launcher window (planned)

**Look.** The same direction as the engine's editor: Unity's dark concept crossed with Linear.
- Surfaces are separated by elevation rather than borders, with 1 px separators.
- OKLCH theme tokens generated from base, accent and contrast, shared with the editor's theme generator.
- Inter and Inter Display.
- Transitions of 100–180 ms with ease-out curves, which never block input. Reduced-motion settings are
  respected.

**Screens.**
- **Projects:** a searchable list (fuzzy, `/` to focus) with favourites, an engine version chip on each row (amber
  with an inline **Install** when the version is missing), and the last-opened time. **New project** is the
  primary button and **Add** the secondary one. Rows have a context menu: open, show in files, change engine
  version, favourite, remove from list.
- **Installs:** each version with its channel badge, size on disk and a "located" tag. **Install version** opens a
  sheet with Stable, Beta and Nightly tabs and release notes. Progress shows in place, through download, verify
  and unpack.
- **New project:** a template gallery, with name, location and version beside it, the default-setup toggles, and
  a collapsed extras section. `Enter` creates the project. Steps stream in from `vox new`.
- **Settings:** release source, channels shown, extra trusted keys, the command-line shim, and update checks.

**Speed.** Ctrl+K opens a command palette over every action, project and version. Ctrl+N starts a new project,
and `Enter` opens the selected one.

**Bridge.** Tauri commands call `voxear_launcher` on a worker thread. Install progress streams through a Tauri
`Channel` of `Progress` values. Cancelling sets the install's cancel flag, and the core reports a `Cancelled`
error, which the UI shows as cancelled rather than failed.

## Settings and environment

`settings.toml`, in the launcher folder:

```toml
releases = "https://…/releases.toml"   # where engine releases are listed
trusted_keys = ["RWQ…"]                 # extra keys for self-hosted engine builds
```

| Variable | Effect |
|---|---|
| `VOXUP_HOME` | Moves the whole launcher folder (portable setups, tests). |
| `VOXUP_RELEASES` | Overrides where engine releases are listed. `--releases` does the same for one command. |
| `VOXEAR_HOME` | Set by the launcher for every engine process it starts. |

Release sources are `https://` URLs or local paths (`file://` URLs or plain paths). Plain HTTP is refused.

## What the engine needs to change

These are the engine-side parts of the contract. The launcher stubs them in its tests with a fake engine
(`voxear_launcher::testing`).

1. **Pin the version.** Add `engine` (an exact semver version) to `[project]` in `voxear.toml`, as
   `ProjectInfo.engine: Option<String>`. `vox new` writes its own version there. `vox run` and the editor warn
   when it differs from their own version.
2. **Write `engine.toml`** at the root of every install and release archive (format, version, channel, platform,
   commit).
3. **Find the install from the executable.** When `VOXEAR_HOME` is unset, `vox` should use the folder above its
   own `bin/` (`current_exe()`), instead of `CARGO_MANIFEST_DIR`. `core_assembly()` should look in
   `<home>/managed/` for installs, and keep `target/managed/<configuration>/` for source checkouts.
4. **Lay out release archives** as described under "Engine archives": `voxear-<version>-<platform>.tar.zst`,
   entries at the root, files and folders only.
5. **`vox new`**, with `--describe --json`, the creation flags, NDJSON events on stdout, and a non-zero exit code
   on failure, as defined above. Use the template, setup and IDE IDs listed above.
6. **Version numbers**: semver, with no tag for stable, `-beta.N` or `-rc.N` for beta, and
   `-nightly.YYYYMMDD` for nightly.
7. **A release pipeline.**
   - Build an archive for each platform.
   - Sign each archive with the engine key: `minisign -S -s voxear-engine.key -m <archive>`.
   - Add a `[[release]]` entry to `releases.toml`, with a newer `generated` time.
   - Sign the manifest.
   - Publish the manifest, the archives and every `.minisig` together.
8. **Later:** `voxear-editor --project <folder>` (M3), and `vox upgrade <folder> --from <version> --json` for
   project migrations.
9. **Nice to have:** a command that builds an install folder from a source checkout (with `engine.toml`), so
   `voxup locate` can use a development build of the engine.

## Open questions

1. **Where releases are hosted.** The engine repository is private, and assets on a private repository's GitHub
   releases need a token to download. Options:
   - a public `CookieJarTools/VoxearReleases` repository, with the archives as release assets and the manifest
     on GitHub Pages;
   - a static bucket behind a CDN.

   Signing makes the host irrelevant to trust. Until this is decided, `settings::DEFAULT_RELEASES` is unset and a
   source must be configured.
2. **Who holds the signing keys,** and where: offline, or as CI secrets.
3. **Windows code signing** for the launcher installer, so SmartScreen doesn't warn.
4. **Backups:** how many to keep per project, and whether they can live inside the project folder instead.
5. **The Jar:** whether the launcher should also appear in The Jar's catalogue.

## Status

| Part | State |
|---|---|
| Release manifest: parsing, validation, selection | Done, tested |
| Signature checks, rollback protection, offline cache | Done, tested |
| Install, list, remove, locate, offline archives; atomic, locked | Done, tested |
| Project registry and project status | Done, tested |
| New-project flow against `vox new` | Done, tested against a fake engine |
| `voxup` command line | Done |
| Opening projects | Done; waits for the editor (M3) |
| Launcher window (Tauri) | Planned |
| Self-update | Planned |
| Project upgrades with backups | Planned |
