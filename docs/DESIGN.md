# Voxear Launcher design

The Voxear Launcher is the desktop app for the [Voxear](https://github.com/CookieJarTools/Voxear) engine. It
installs engine versions side by side, keeps itself up to date, lists projects and opens each one with the engine
version it is pinned to, and creates new projects from templates.

This document defines the formats the launcher and the engine share: the release manifest, the install layout, the
project pin and the `vox new` contract. The engine's side of the contract is summarised in "Versions, installs
and the launcher" in the engine's `docs/ARCHITECTURE.md`; this document is the detailed version.

## Decisions

Settled on 2026-10-30:

- **Repository:** `CookieJarTools/VoxearLauncher`, private, next to the engine.
- **UI:** a Rust core with a web UI through **Tauri 2** (React 19, Vite, TypeScript).
  - It ships before engine milestone M3, which is when the first engine release needs installing.
  - The look, OKLCH themes and short animations are cheap in CSS.
  - It still opens on a machine whose GPU lacks Vulkan 1.3, so it can explain what is missing.
  - The second UI stack is confined to this one app. The core has no UI in it, so the front end could move to
    Voxear UI later without touching the logic.
- **Command line:** `voxup`, short to type. It installs engine versions side by side, from stable, beta and
  nightly channels.
- **Licence:** PolyForm Shield 1.0.0, the same as the engine.
- **Hosting:** a public `CookieJarTools/VoxearReleases` repository. Archives and launcher builds are its release
  assets, and the manifests are on its GitHub Pages site (see "Publishing releases").
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
 │ manifest · releases · sources · signing · fetch · archive · store     │
 │ registry · project · new_project · settings · layout                  │
 └──────────────────┬───────────────────────────────────┬────────────────┘
          release mirror (https / file)        engine installs: bin/vox new …
```

```
crates/
  voxear_launcher   the core: everything the launcher does, with no UI
  voxup             the command line
app/                the Tauri window: React front end in src/, Rust commands in src-tauri/
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

[[release.download.module]]         # optional parts built for this platform
name = "server"
title = "Dedicated Server"
url = "voxear-0.3.0-linux-x86_64-server.tar.zst"
size = 9123456
sha256 = "…"
signature = """…"""

[[release.download]]
platform = "windows-x86_64"
url = "voxear-0.3.0-windows-x86_64.tar.zst"
size = 398765432
sha256 = "…"
signature = """…"""

[[release.download.module]]
name = "server"
title = "Dedicated Server"
url = "voxear-0.3.0-windows-x86_64-server.tar.zst"
size = 8765432
sha256 = "…"
signature = """…"""

[[release.module]]                  # optional parts, the same on every platform
name = "manual"
title = "Voxear Manual"
url = "voxear-0.3.0-manual.tar.zst"
size = 18412345
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
| `download.module` | Optional parts built for that platform: `server`, the Dedicated Server. Fields as for `module`. Older launchers ignore them. |
| `module` | Optional parts of the release that are the same on every platform: `manual`, the Voxear Manual. Each has a `name`, a `title` to show, and a `url`, `size`, `sha256` and `signature` like a download. A part is listed in one place only: `manual` on the release, `server` on each download. A name the launcher doesn't know is skipped, so new parts don't break older launchers. Releases from before modules existed have none; older launchers ignore them. |

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

## Release lists

The official releases are always listed. **Custom releases** add someone else's, such as those of a fork of the
engine: another `releases.toml` in the same format, published the same way, usually by the engine's own release
workflow run from the fork. Each list is fetched, checked and saved for offline use on its own (`sources`), and the
engine list merges them, newest first, saying which list each release came from.

- **Addresses.** Custom releases can be given as their `releases.toml` (an `https://` URL or a file), the site or
  folder it is in (`<site>/engine/releases.toml`, as `voxear-release` writes a site, or `<folder>/releases.toml`), or
  the GitHub repository they publish from: `owner/repo` or `https://github.com/owner/repo` means
  `https://owner.github.io/repo/engine/releases.toml`. The settings keep the manifest's own address.
- **Keys.** A list's manifest and archives must be signed by the built-in engine keys, by `trusted_keys`, or by the
  list's own `key`. A list's own key is trusted for that list alone, so a fork's key can't sign anything in the
  official list.
- **One version, one engine.** An install is its version, and a project pins a version, so a version number has to
  mean one engine. A release two lists publish with the same archive (a mirror) shows once, under the first list.
  When two lists publish different engines under one version, both show, and installing that version needs the
  list named: `voxup install <version> --from <name>`, or the row picked in the window. Forks are best off giving
  their builds versions of their own, with build metadata: `0.2.0+lumen`, `0.3.0-nightly.20261117+lumen`.
- **Finding a version.** An exact version, such as a project's pin, is looked for in every list, so a project made
  with a fork's engine installs from the fork. A channel or a range (`stable`, `0.3`) picks from the official
  releases, or from the first list after them that has a match, unless `--from` names the list.
- **Where an install came from.** The receipt keeps the address of the list an engine was installed from
  (`releases`), so the window and `voxup list` can say so, and an optional part added later comes from the same
  list.
- **A list that can't be read** is reported on its own; the others are still listed.
- **Nightlies** show in the engine list when `nightlies` is on. Unset, they show once a nightly is installed.
  `voxup install nightly` and a project pinned to a nightly work either way.

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
- `trusted_keys` in the settings adds more keys, trusted for every release list and for archives installed from a
  file.
- Custom releases can bring a `key` of their own, trusted for that list alone.
- The launcher's own updates use a **separate key**, so a leaked engine key can't push a launcher update, and
  the reverse.

**Signing keys.** The engine key exists (`4B25CB4CB88433F4`, in `ENGINE_RELEASE_KEYS`) and is kept offline. The
launcher key does not exist yet, so until it does the launcher can't update itself.

The secret keys belong offline or in CI secrets, never in a repository. `voxear-release` makes both (it asks for
a password to encrypt each one):

```bash
cargo run -p voxear_release -- keygen ~/.voxear-keys/engine
```

```bash
cargo run -p voxear_release -- keygen ~/.voxear-keys/launcher --tauri
```

Each prints its public key:
- the engine key's goes in `ENGINE_RELEASE_KEYS` in `crates/voxear_launcher/src/signing.rs`
- the launcher key's, in Tauri's base64 form, goes in `LAUNCHER_UPDATE_KEY` in
  `crates/voxear_launcher/src/self_update.rs`

The launcher key is also what `tauri build` signs with, through `TAURI_SIGNING_PRIVATE_KEY`. `minisign -G` and
`tauri signer generate` make compatible keys, if you'd rather use those.

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

**Modules.** Optional parts ship as archives of their own, beside the platform archives. Each adds one path to an
install, and every entry in its archive must be that path, something inside it, or a folder on the way to it; the
launcher refuses an archive with anything else, so a module can never change the engine's files.

| Module | Archive | Adds |
|---|---|---|
| Voxear Manual (`manual`) | `voxear-<version>-manual.tar.zst`, one for every platform | `manual/…` |
| Dedicated Server (`server`) | `voxear-<version>-<platform>-server.tar.zst`, one per platform | `bin/voxear-server[.exe]` |

The editor shows the manual from `<install>/manual/` when that folder exists, and leaves it out when it doesn't.
The Dedicated Server is the program a game's dedicated-server build ships; it never opens a window or a GPU. Without
it, server builds use the player program instead and say to add the Dedicated Server in the launcher. Engine
archives from before a part became a module may still carry it inside, and that copy is kept.

Every engine install, including a build from source that the launcher is pointed at, has an `engine.toml` at its
root:

```toml
format = 1
version = "0.3.0"
channel = "stable"
platform = "linux-x86_64"
commit = "0ffa698"          # optional

[supports]                  # the launcher contracts this install's vox implements, and their formats
new = 1                     # vox new, and the format of vox new --describe
upgrade = 1                 # vox upgrade
```

The launcher reads this file and never runs anything to find out what an install is. A contract missing from
`[supports]` means the install doesn't have that command; every engine built before the contract existed is
treated that way.

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
  tools/whisk/Whisk.AppImage    Whisk, when the launcher installed it (Linux)
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

**Modules.** Installing a release brings its modules for this platform too, unless they're left out: the window's
checkboxes, or `voxup install <version> --without-manual --without-server`. Each module is downloaded and checked
like the engine archive, and unpacked into the same staging folder, so the install appears in one step with or
without them. Later, a module can be added (`voxup manual add <version>`, `voxup server add <version>`), unpacked
into staging and its path renamed into place, or removed (`voxup manual remove <version>`, `voxup server remove
<version>`): its path, the `manual` folder or the one `bin/voxear-server` file, is renamed into the trash and
deleted. Only the launcher's own installs change this way; a located engine's files are left as they are.

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

`engine` is an exact semver version, never a range, so a project always opens with the version it was made for.
`vox new` writes it with its own version. A project without it, such as `samples/Hello` today, shows as "not pinned".

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
| Not pinned | "not pinned" | Pin it to a version (see "Changing a project's engine version") |
| Missing | "folder not found" | Locate or remove from list |
| Unreadable | the parse error | Show the file |

The list is sorted with favourites first, then by last opened (or added). Removing a project from the list never
touches its files.

**Opening.** The launcher resolves the pinned version and starts `<install>/bin/voxear-editor --project <folder>`
with `VOXEAR_HOME` set. If the version isn't installed, it offers to install it first (`voxup open --install`).
The editor arrives at M3, so for now opening explains that the version has no editor.

### Changing a project's engine version

Moving a project to another installed version (or pinning one that has none) works the same from the window
(the project's menu, or its "Not pinned" chip) and from the command line:

```bash
voxup upgrade <folder> --to <version> [--install] [--allow-downgrade]
```

1. **Choose.** The window lists the installed versions marked Newer, Older or Current. It preselects the newest
   stable version that moves the project forward, and links the release notes of every version in between (the
   manifest's `notes`).
2. **Refuse the risky case.** Moving to the version the project is on is refused. Moving to an older one is
   refused unless it's asked for: `--allow-downgrade`, or the window's "Downgrade anyway" toggle beside a warning.
3. **Back up.** The project is backed up first (below), with progress.
4. **Convert.**
   - If the target lists `upgrade` in `[supports]`, the launcher runs its `vox upgrade` (below), which converts
     the project and rewrites `engine`.
   - If it doesn't, the launcher changes only the `engine` line in `voxear.toml`, keeping the rest of the file,
     comments included. It then says plainly that nothing else was converted.
5. **Check.** Afterwards, `voxear.toml` must be pinned to the target version, or the upgrade counts as failed.
6. **Summarise.** The window shows each change, where the backup is (with Show), and an **Open** button. The
   changes are also saved with the backup.

When anything fails after the backup, the error ends with where the backup is, so nothing is ever lost.

### `vox upgrade <folder> …`

```
vox upgrade <folder> [--from <version>] --json
```

- Converts the project at `<folder>` from `--from` to this `vox`'s own version, and sets `engine` to it.
  `--from` is left out for a project that wasn't pinned.
- It prints one JSON object per line on stdout, as `vox new` does:

  ```
  {"event":"step","message":"Converting scenes"}
  {"event":"change","message":"Converted 4 scenes to the new scene format"}
  {"event":"done"}
  ```

  Each `change` is a line in the summary the user sees, so it should say what changed, in plain words. On failure
  it prints `{"event":"error","message":"…"}` and exits with a non-zero code. The launcher has already backed the
  project up, but `vox upgrade` should still leave the project as it was if it fails.

### Backups

A backup is a `.tar.zst` of the project folder in `backups/<name>-<hash of path>/<UTC time>.tar.zst`, in the
launcher folder, with a `.toml` beside it:

```toml
project = "Orbit"
path = "/home/jeo/Games/Orbit"
from = "0.2.0"              # left out when the project wasn't pinned
to = "0.3.0"
created = "2026-10-30T17:25:10Z"
size = 50331648
files = 318
changes = ["Converted 4 scenes to the new scene format"]
```

- **Left out:**
  - what can be rebuilt: `.voxear/`, `bin/` and `obj/` beside a `.csproj`, and `.vs/`
  - `.git/`, which is history already
  - links, which are listed as skipped
- **Restoring never overwrites.** It unpacks the backup into a new folder beside the project, such as
  `Orbit (before 0.3.0)`, and adds that to the project list. The project it came from isn't touched.
- **Backups are kept until deleted**, from the window's Backups dialog. On the command line, `voxup backups` lists
  them and `voxup restore <archive>` restores one.

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

## Whisk

Whisk, the code editor the engine's editor opens scripts and projects in, is installed beside the engines rather
than with one: one copy serves every engine version, and Whisk updates itself. Core: `voxear_launcher::whisk`.

- **Releases.** Whisk's own GitHub releases, described by `latest.json` in the format Tauri's updater reads, keyed
  by the same platform names as engine releases. `VOXUP_WHISK_UPDATES` overrides where it is read from.
- **Checks.** Each download is checked against Whisk's update key, built into the launcher, and its trusted
  comment must name the version the manifest announces, as for the launcher's own updates.
- **Linux.** The AppImage goes to `tools/whisk/Whisk.AppImage`. If nothing yet says where Whisk is,
  `~/.whisk/install-path` is written too; Whisk rewrites that file each time it starts, and the editor reads it.
- **Windows.** Whisk's per-user installer runs silently (`/S`) and Whisk lands in `%LOCALAPPDATA%\Whisk`.
- **Finding it.** The launcher's copy, then `~/.whisk/install-path` (or another Whisk AppImage in that folder when
  the recorded one has gone), then `/usr/bin/whisk` from Whisk's packages, or on Windows the folder Whisk's
  installer records under `HKCU\Software\CookieJarTools\Whisk`.
- **Opening.** `whisk <folder> [<file>[:<line>]…]`. A running Whisk takes the arguments and the new process quits.
  Started from the launcher's AppImage, Whisk gets the environment the launcher was started with.
- **Removing** takes away only the launcher's copy. A Whisk installed any other way is left alone.

## Launcher self-update

The launcher updates itself from **its own manifest, signed with its own key**, separately from engine versions.

**Packaging.** How to install, update and uninstall is in [INSTALLING.md](INSTALLING.md); this is how it works.
- Linux: an AppImage, the only Linux format Tauri's updater can replace in place, so it runs on any distro.
  - `--install`, or **Install** on the sidebar card an uninstalled AppImage shows, puts it in the app menu
    (`voxear_launcher::self_install`). It copies the AppImage to `~/.local/bin/voxear-launcher` with one
    rename, writes `voxear-launcher.desktop` to `~/.local/share/applications` and the icons to
    `~/.local/share/icons/hicolor`. The window then closes and reopens from the installed copy, once the old one
    has let go of the single-instance lock.
  - `--uninstall` removes those three and keeps the launcher folder.
  - A copy installed any other way (a `.deb`, or later an AUR package) says that it should be updated the way it
    was installed.
- Windows: a per-user NSIS setup program, so no admin rights are needed. It installs to
  `%LOCALAPPDATA%\Voxear Launcher`, and adds a Start menu shortcut, a desktop shortcut (a box on its last page)
  and an uninstall entry in Settings › Apps. The updater runs the new setup program in passive mode after the
  launcher closes, and the setup starts the launcher again.
- The launcher folder (engines, projects, settings) is apart from the program on both, so updates and uninstalls
  leave it alone.

**Manifest.** One static JSON file per channel, in the format Tauri's updater reads:
`https://cookiejartools.github.io/VoxearReleases/launcher/stable.json`, and `beta.json`. `VOXUP_LAUNCHER_UPDATES`
points the launcher at another folder or URL.

```json
{
  "version": "0.2.0",
  "notes": "What changed, in Markdown",
  "pub_date": "2026-11-15T12:00:00Z",
  "platforms": {
    "linux-x86_64": {
      "url": "https://…/Voxear.Launcher_0.2.0_amd64.AppImage",
      "signature": "<base64 of the .sig>",
      "sha256": "<hex>",
      "size": 84123456
    },
    "windows-x86_64": { "url": "https://…/Voxear.Launcher_0.2.0_x64-setup.exe", "signature": "…", "sha256": "…", "size": 5123456 },
    "voxup-linux-x86_64": { "url": "https://…/voxup", "signature": "…", "sha256": "…", "size": 9123456 }
  }
}
```

- The `voxup-<platform>` entries are standalone `voxup` builds. Tauri's updater ignores them.
- Signatures are minisign signatures, base64-encoded as Tauri writes them.
- `sha256` and `size` are the launcher's own fields, which Tauri's updater ignores. Both the window and `voxup`
  check them, so a download cut short says so, rather than failing its signature check. A manifest without them
  is checked by signature alone.
- URLs are `https://`. Plain `http://` is accepted only for this machine (`localhost`, `127.0.0.1`, `[::1]`), for
  trying an update from a local folder. The launcher checks both the manifest's URL and each download's itself, so
  Tauri's own `http://` refusal is switched off (`dangerousInsecureTransportProtocol`).
- **The signed version must match.** The manifest itself isn't signed, so each signature's trusted comment carries
  `version:<version>` and must equal the version the manifest announces. Otherwise an old build could be offered
  as a new one. The window turns on Tauri's `requireSignedVersion` for this, and `voxup` checks the same way.
- An update must be newer than the running version.

**The key.**
- The launcher update key is built into `voxear_launcher::self_update::LAUNCHER_UPDATE_KEY`, beside the engine
  release keys, and handed to Tauri's updater when the app starts.
- `tauri.conf.json` holds no key, so a build without one still starts; it just can't update itself.
- A build can take a key through `VOXEAR_LAUNCHER_UPDATE_KEY` when it's compiled, for trying the update flow
  with a key of your own.

**The window.**
- It checks when it opens and every six hours while it's open, quietly. The launcher reads the manifest itself
  first, so "nothing published yet" and "up to date" aren't reported as failures; Tauri's updater then fetches and
  checks the update.
- When there's a newer version, a card in the sidebar shows it with its notes and **Update**. That downloads the
  update into memory, checks its size, SHA-256 and signature, and only then installs it, with progress. Until
  then nothing on disk has changed, so a failed or cut-off download leaves the launcher as it was. The card then
  offers **Restart now**, which waits while an engine install or a project upgrade is running. The restart is an
  exit request, so plugins let go of the single-instance lock before the new copy starts.
- `voxear-launcher --update` runs the same check and install from a terminal, with no window: Tauri starts with
  its window list emptied, updates and exits.
- Settings › Launcher updates has the channel (Stable or Beta, saved at once), **Check now**, and what the last
  check found. Development builds, and copies that weren't installed from the AppImage or the installer, say why
  they don't update themselves.

**The command line.**
- `voxup self update` updates a standalone `voxup` from its `voxup-<platform>` entry. It downloads beside itself,
  checks the signature and the signed version, and swaps itself in one rename. On Windows the running program is
  renamed aside first and removed the next time `voxup` runs.
- `--check` only says whether there's an update.
- A copy in a folder it can't write to, such as one installed by a package manager, says to update it there
  instead.

**Planned.** Settings › "Command line" will put a `voxup` shim on the PATH that calls the launcher's own copy, so
launcher updates update it too: `~/.local/bin/voxup` on Linux, and `%LOCALAPPDATA%\Voxear\bin\voxup.cmd` on
Windows.

## Publishing releases

Releases are published to the public `CookieJarTools/VoxearReleases` repository:
- **Release assets** hold the archives and builds, under tags such as `engine-v0.3.0` and `launcher-v0.2.0`:
  `https://github.com/CookieJarTools/VoxearReleases/releases/download/<tag>/<file>`.
- **GitHub Pages** serves the manifests from the repository's `gh-pages` branch, at
  `https://cookiejartools.github.io/VoxearReleases/`. "Publish the site" below means committing the site folder's
  contents to that branch:

```
engine/releases.toml            the engine release manifest
engine/releases.toml.minisig    its signature
launcher/stable.json            launcher updates, per channel
launcher/beta.json
```

`voxear-release` (in `crates/voxear_release`) writes all of it. Key passwords come from
`VOXEAR_SIGNING_PASSWORD`, or it asks for them.

**An engine release.** Each archive's own `engine.toml` gives the version, channel and platform. A module archive,
which has no `engine.toml`, is known by its name: `voxear-<version>-manual.tar.zst` is listed on its version, and
`voxear-<version>-<platform>-server.tar.zst` on that platform's download. Engine archives are listed first, so the
files can come in any order, and adding an engine archive again keeps its platform's modules listed. The command
signs each archive (writing its `.minisig` beside it), lists it in the manifest with a newer `generated` time, and
signs the manifest:

```bash
voxear-release engine add voxear-0.3.0-*.tar.zst --site site --key ~/.voxear-keys/engine.key --notes https://…
```

Then upload the archives and their `.minisig` files to the `engine-v0.3.0` release, and publish the site.
`engine yank <version>` pulls a release (`--undo` puts it back), and `engine prune --keep 30` drops old
nightlies.

**A launcher release.** "Making a release" in [INSTALLING.md](INSTALLING.md) has the steps.
`scripts/set-version.sh` sets the version in the four files that hold it, and `scripts/build-installers.sh`
bundles the window with signing on when `TAURI_SIGNING_PRIVATE_KEY` is set, gathering each installer and its
`.sig` in `out/`. The Installers workflow does the same on Linux and Windows. List each installer, and the
standalone `voxup` builds, in a channel:

```bash
voxear-release launcher add out/Voxear.Launcher_0.2.0_amd64.AppImage --site site --channel stable --version 0.2.0 --target linux-x86_64 --sig out/Voxear.Launcher_0.2.0_amd64.AppImage.sig
```

```bash
voxear-release launcher add voxup --site site --channel stable --version 0.2.0 --target voxup-linux-x86_64 --key ~/.voxear-keys/launcher.key
```

Then upload the bundles, the `voxup` builds and their signatures to the `launcher-v0.2.0` release, and publish
the site. A new version starts a fresh channel manifest, and a version older than the one already published is
refused.

## The launcher window

The window lives in `app/`: a React 19, Vite and TypeScript front end in `app/src`, and the Tauri 2 shell in
`app/src-tauri`, whose commands call `voxear_launcher`. The shell is its own Cargo workspace, so building and
testing the core never needs Node.

**Look.** The same direction as the engine's editor: dark, calm and dense.
- Surfaces are separated by lightness (elevation) rather than borders, with 1 px separators.
- Every colour comes from three OKLCH inputs, `--base-hue`, `--accent-hue` and `--contrast`, in
  `app/src/theme/tokens.css`. The editor's theme generator should use the same scheme.
- Inter, bundled, with its optical-size axis, so large headings get the Inter Display cut.
- Transitions of 100–180 ms with ease-out curves, which never block input. Reduced-motion settings turn them off.
- A frameless window with its own title bar.

**Screens.**
- **Projects:**
  - A list with fuzzy search (`/` focuses it, arrow keys move, `Enter` opens) and favourites first.
  - Each row has a coloured initial, the path, an engine-version chip and the time it was last opened.
  - A missing version shows an amber **Install 0.2.0** chip that queues the install.
  - Each row has a menu: open, show in files, favourite, change engine version, backups, remove from list.
  - A "Not pinned" chip opens the pin dialog.
  - **New project** is the primary button and **Add** the secondary one.
- **Installs:**
  - Each version shows its channel badge, size on disk, when it was installed, and a "Located" tag, or a tag
    for each module installed with it: "Manual", "Server".
  - The row menu adds or removes each module the release offers: "Add the Voxear Manual", "Add the Dedicated
    Server", or "Remove the …" after asking. Adding one runs in the install queue, with its own progress card.
  - Installs run one at a time from a queue (the core's lock allows only one). Each one shows as a card with
    its phase, progress and **Cancel**; a failed one offers **Try again**.
  - Uninstalling asks first and names the projects pinned to the version. Forgetting a located engine says that
    its files stay.
- **Install a version:** a sheet with Stable, Beta and Nightly tabs, sizes, ages and release notes. A release with
  modules has a checkbox for each, with its size, ticked by default: "Voxear Manual", "Dedicated Server". It
  explains when no release source is set, and shows the saved list, with a note, when the source can't be reached.
  Installing from a file and locating an engine are in its footer.
- **New project:**
  - A template gallery. The thumbnails are line drawings in the launcher until the engine ships thumbnails of its
    own.
  - Beside it: name and location, the engine version, the default-setup toggles, and collapsed extras (IDE, Git,
    modding, Steam App ID).
  - Every field comes from that version's `vox new --describe`.
  - The name and folder are checked as they're typed. Ctrl+Enter creates the project, and the steps stream in
    from `vox new`.
  - When it's done, the project is selected in the list, and a toast offers **Open**.
- **Change engine version:** the installed versions marked Newer, Older or Current, with release notes, then the
  backup's progress, the steps from `vox upgrade`, and a summary of what changed. A downgrade needs its own toggle.
- **Backups:** each one's versions, age, size and recorded changes, with **Restore a copy**, Show and Delete.
- **Settings:** the release source, extra trusted keys, the folder new projects go in, and where the launcher
  keeps its files.

**Speed.**
- Ctrl+K opens a command palette over every action, project and version.
- Ctrl+N starts a new project; Ctrl+1, Ctrl+2 and Ctrl+, switch screens.
- F5 refreshes, and the lists also refresh whenever the window gains focus, to pick up changes made through
  `voxup`.

**Bridge.**
- Each command runs `voxear_launcher` on a worker thread, so a slow disk never freezes the window.
- Install progress and `vox new` steps stream through Tauri channels.
- Cancelling sets the install's cancel flag. The core then reports `Cancelled`, which the window shows as
  cancelled rather than failed.
- Errors arrive as `{ message, cancelled }`. The core's `backticked` names render as code.

**Previews and tests.**
- Opened in a plain browser (`npm run dev`), the front end uses an in-memory stand-in for the commands
  (`app/src/mock.ts`), so every screen can be previewed without Tauri.
- `cargo test` in `app/src-tauri` calls the real commands through Tauri's IPC, with the front end's command
  names, camelCase arguments and channel format, against a fake signed release.

**Still to come in the window:**
- the command-line shim setting
- template thumbnails from the engine

## Settings and environment

`settings.toml`, in the launcher folder:

```toml
nightlies = true                        # list nightly builds; unset, they show once one is installed
trusted_keys = ["RWQ…"]                 # extra keys, for every release list and for archives from a file
project_location = "/home/jeo/VoxearProjects"   # remembered from the last new project
launcher_channel = "beta"               # launcher updates: stable (the default) or beta

[[custom_releases]]                     # someone else's releases, listed beside the official ones
name = "Lumen"
url = "https://lumen.github.io/VoxearReleases/engine/releases.toml"
key = "RWQ…"                            # optional: the key their releases are signed with
```

The official releases are always listed, from `https://cookiejartools.github.io/VoxearReleases/engine/releases.toml`.
Until something is published at a list, the launcher says nothing is published there yet, rather than reporting an
error. `voxup releases` lists, adds and removes custom releases and turns nightlies on and off; the window does the
same in Settings. Adding custom releases reads them first, so a wrong address or key shows at once.

**The old `releases` setting**, from before engine releases were published, named one list that took the place of
the official one. A saved value moves over when the settings are read: the official list's own address is dropped,
and anything else becomes custom releases, named after where they are, and is saved that way the next time the
settings are saved. Its keys were in `trusted_keys`, which still apply.

| Variable | Effect |
|---|---|
| `VOXUP_HOME` | Moves the whole launcher folder (portable setups, tests). |
| `VOXUP_RELEASES` | Lists the official releases from somewhere else, such as a mirror or a test folder. `--releases` does the same for one command. Custom releases are still listed beside them. |
| `VOXUP_LAUNCHER_UPDATES` | Overrides where the launcher's update manifests are read from: an `https://` URL, `http://` on this machine, or (for `voxup`) a folder. |
| `VOXUP_WHISK_UPDATES` | Overrides where Whisk's release manifest is read from. |
| `VOXEAR_HOME` | Set by the launcher for every engine process it starts. |

Release lists are `https://` URLs or local paths (`file://` URLs or plain paths). Plain HTTP is refused.

## What the engine needs to change

These are the engine-side parts of the contract. The launcher stubs them in its tests with a fake engine
(`voxear_launcher::testing`).

1. **Pin the version.** Add `engine` (an exact semver version) to `[project]` in `voxear.toml`, as
   `ProjectInfo.engine: Option<String>`. `vox new` writes its own version there. `vox run` and the editor warn
   when it differs from their own version.
2. **Write `engine.toml`** at the root of every install and release archive: format, version, channel, platform,
   commit, and `[supports]` listing the contracts its `vox` implements (`new = 1`, `upgrade = 1`).
3. **Find the install from the executable.** When `VOXEAR_HOME` is unset, `vox` should use the folder above its
   own `bin/` (`current_exe()`), instead of `CARGO_MANIFEST_DIR`. `core_assembly()` should look in
   `<home>/managed/` for installs, and keep `target/managed/<configuration>/` for source checkouts.
4. **Lay out release archives** as described under "Engine archives": `voxear-<version>-<platform>.tar.zst`,
   entries at the root, files and folders only.
5. **`vox new`**, with `--describe --json`, the creation flags, NDJSON events on stdout, and a non-zero exit code
   on failure, as defined above. Use the template, setup and IDE IDs listed above.
6. **Version numbers**: semver, with no tag for stable, `-beta.N` or `-rc.N` for beta, and
   `-nightly.YYYYMMDD` for nightly.
7. **A release pipeline.** Build an archive for each platform, then run `voxear-release engine add` on them (see
   "Publishing releases"). That signs each archive, lists it, and signs the manifest. Upload the archives and
   their `.minisig` files as release assets, and publish the site.
8. **`vox upgrade`**, as defined under "Changing a project's engine version": it converts the project, rewrites
   `engine`, and reports each change as NDJSON. Until an engine has it, the launcher changes only the pin.
9. **Later:** `voxear-editor --project <folder>` (M3).
10. **Nice to have:** a command that builds an install folder from a source checkout (with `engine.toml`), so
   `voxup locate` can use a development build of the engine.

## Open questions

1. **Who holds the signing keys,** and where: offline, or as CI secrets.
2. **Windows code signing** for the launcher installer, so SmartScreen doesn't warn.
3. **Backups:** whether to delete old ones on their own after a while. For now they are kept until deleted.
4. **The Jar:** whether the launcher should also appear in The Jar's catalogue.
5. **Release automation:** the Installers workflow builds and signs both platforms' installers. Listing them with
   `voxear-release`, uploading and publishing the site are still done by hand, as described above.

## Status

| Part | State |
|---|---|
| Release manifest: parsing, validation, selection | Done, tested |
| Signature checks, rollback protection, offline cache | Done, tested |
| Release lists: official, nightlies, custom releases such as a fork's, merged | Done, tested; `voxup releases` and Settings |
| Install, list, remove, locate, offline archives; atomic, locked | Done, tested |
| Project registry and project status | Done, tested |
| New-project flow against `vox new` | Done, tested against a fake engine |
| `voxup` command line | Done |
| Opening projects | Done; waits for the editor (M3) |
| Launcher window (Tauri): projects, installs, new project, settings, palette | Done; commands tested through IPC |
| Launcher self-update: window (Tauri's updater), `--update` and `voxup self update` | Done; size, hash and signature checks tested; Linux tested end to end with a local fake release |
| Installers: Windows setup program, Linux AppImage with `--install` / `--uninstall` | Done; Linux tested live, the Windows setup waits for a run on Windows |
| Release tool (`voxear-release`): keys, engine and launcher manifests | Done, tested against the launcher's own install and update code |
| Changing engine versions, with backups and restore | Done, tested; the conversion itself waits for `vox upgrade` |
