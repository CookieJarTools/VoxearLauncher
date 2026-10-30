# Voxear Launcher

The launcher for the [Voxear](https://github.com/CookieJarTools/Voxear) game engine, the equivalent of Unity Hub. It
installs engine versions side by side, opens each project with the engine version it is pinned to, and creates new
projects from templates with the engine's own `vox new`.

The engine releases it installs are verified before anything is unpacked: each one's size, SHA-256 and
signature. The design, including every format the launcher and the engine share, is in
[docs/DESIGN.md](docs/DESIGN.md).

**Status:** the core and the `voxup` command line work. The launcher window (Tauri) and self-update are next.
There are no engine releases to install until engine milestone M3, so for now the launcher can be tried against a
fake release folder (below).

## Requirements

- Rust (stable, 1.89 or newer)
- A C compiler, for zstd

## Build and test

```bash
cargo build
```

```bash
cargo test
```

## Using `voxup`

```
voxup list [--available]          installed engine versions, and those that can be installed
voxup install stable              the newest stable version (or beta, nightly, 0.3, 0.3.1)
voxup install --file <archive>    an archive from disk; its .minisig must sit next to it
voxup uninstall 0.3.0             refuses while projects use it, unless --force
voxup locate <folder>             use an engine installed elsewhere, such as a source build

voxup projects                    the project list, with what each project needs
voxup add <folder>                add a project to the list
voxup remove <folder>             take it off the list (its files stay)
voxup open [folder] [--install]   open with the pinned engine version
voxup new Orbit -t third-person --git --ide rider
voxup templates                   what an engine version offers for new projects
```

`VOXUP_HOME` moves the launcher's folder, which is `~/.local/share/voxear` on Linux and `%LOCALAPPDATA%\Voxear`
on Windows.

## Trying it with fake releases

This writes a folder of signed fake releases. Each one has a `vox` that answers `vox new`, so the whole flow can
be tried on Linux:

```bash
cargo run -p voxear_launcher --features testing --example fake_release -- /tmp/voxear-mirror
```

It prints two lines for the launcher's settings. To keep the trial away from your real launcher folder, put them
in `/tmp/voxup-home/settings.toml` and point `voxup` there:

```bash
export VOXUP_HOME=/tmp/voxup-home
```

Then try `voxup list --available`, `voxup install stable`, `voxup new Orbit -l /tmp` and `voxup projects`.

## Layout

```
crates/voxear_launcher   the core: manifests, signatures, installs, projects, the new-project flow
crates/voxup             the command line
docs/                    the design
```

## Licence

The Voxear Launcher is licensed under the [PolyForm Shield License 1.0.0](LICENSE.md): you may use and change
it, but not use it to build a product that competes with it.

Required Notice: Copyright 2026 Joseph Cook
