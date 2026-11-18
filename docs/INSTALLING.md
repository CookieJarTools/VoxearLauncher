# Installing, updating and uninstalling

Releases of the Voxear Launcher are published on
[VoxearReleases](https://github.com/CookieJarTools/VoxearReleases/releases), under tags such as `launcher-v0.2.0`.

## Installing on Windows

1. Download `Voxear.Launcher_<version>_x64-setup.exe`.
2. Run it. It installs for your user only, so it never asks for admin rights. The program goes in
   `%LOCALAPPDATA%\Voxear Launcher`.
3. The launcher is added to the Start menu, and to the desktop unless you untick **Create desktop shortcut** on
   the last page.

The setup program isn't code-signed yet, so Windows SmartScreen may warn about it: choose **More info**, then
**Run anyway**. The launcher's window needs WebView2. Windows 10 and 11 come with it, and the setup installs it
if it's missing.

## Installing on Linux

The AppImage runs on any x86-64 distro with a desktop, on Wayland and X11.

1. Download `Voxear.Launcher_<version>_amd64.AppImage`.
2. Make it executable and install it:

   ```bash
   chmod +x Voxear.Launcher_*_amd64.AppImage
   ```

   ```bash
   ./Voxear.Launcher_*_amd64.AppImage --install
   ```

   Or open the AppImage and click **Install** in the sidebar. The launcher then reopens from its installed copy.

Installing copies the AppImage to `~/.local/bin/voxear-launcher`, adds `voxear-launcher.desktop` to
`~/.local/share/applications`, and puts its icons in `~/.local/share/icons/hicolor`. It doesn't need root.
Because `~/.local/bin` is on the PATH, `voxear-launcher` also runs from a terminal. The AppImage you downloaded
isn't needed afterwards.

An AppImage mounts itself with FUSE, which desktops almost always have. If it won't start, install your distro's
`fuse3` package, or run it with `--appimage-extract-and-run`.

A `.deb` is published too. It installs the launcher for everyone, and it doesn't update itself: install the next
`.deb` over it instead.

## Updating

The launcher checks for a newer version of itself when it opens and every six hours after that. When one is out,
a card in the sidebar shows it. **Update** downloads and installs it, and **Restart now** starts the new version.
**Settings › Launcher updates** has the channel, **Stable** or **Beta**, and **Check now**.

From a terminal, `voxear-launcher --update` does the same without opening the window. On Windows, run
`"%LOCALAPPDATA%\Voxear Launcher\voxear-launcher.exe" --update`.

Every update is checked before anything is replaced:

- Its size and SHA-256 must match the release's manifest.
- It must be signed with the release key, and the signature must name the version on offer, so an old
  build can't be passed off as a new one.
- It must be newer than the running launcher.

The installed launcher stays as it was until all of these pass. A download that fails or stops part way changes
nothing. On Linux the update then replaces the AppImage in one step. On Windows the new setup program runs with a
progress bar only, then starts the launcher again.

Engine versions, the project list and settings are never touched by an update. They live apart from the program,
in `~/.local/share/voxear` on Linux and `%LOCALAPPDATA%\Voxear` on Windows.

## Uninstalling

**Windows:** open **Settings › Apps › Installed apps**, and uninstall **Voxear Launcher**. **Delete the
application data** removes only the window's own data, in `%LOCALAPPDATA%\com.cookiejar.voxear.launcher`.

**Linux:**

```bash
voxear-launcher --uninstall
```

That removes the program, its desktop entry and its icons. The window's own data stays in
`~/.local/share/com.cookiejar.voxear.launcher`, and you can delete that folder too. Installed from the `.deb`,
uninstall with `sudo apt remove voxear-launcher`.

Either way, engine versions, projects and settings stay. To remove them as well, delete `~/.local/share/voxear`
or `%LOCALAPPDATA%\Voxear`. Your projects' own folders are never removed.

## Making a release

The engine release key signs every launcher release, as it signs the engine. Its public key is built into
`voxear_launcher::self_update::LAUNCHER_UPDATE_KEY`, and the secret key is a CI secret of the engine repository,
so releases are made there.

1. **Set the version** and commit it. It is written in four files; this sets them all:

   ```bash
   scripts/set-version.sh 0.2.0
   ```

2. **Push** the launcher's `master`.
3. **Run the Launcher Release workflow** in the engine repository (`CookieJarTools/Voxear`, Actions, Launcher
   Release, Run workflow), with Publish ticked. It:
   - builds the AppImage and `.deb` on Ubuntu 22.04, and the Windows setup and both `voxup` builds;
   - signs each one with `voxear-release launcher add --key`, which writes `launcher/<channel>.json` with each
     download's URL, signature, size and SHA-256;
   - uploads them to a `launcher-v<version>` release of VoxearReleases, then publishes the manifest on its
     `gh-pages` branch, in that order, so the manifest never offers a file that isn't there yet.

   Without Publish ticked it only builds, and keeps the installers as the run's artifacts.

To make one by hand instead, build with `scripts/build-installers.sh`, then list each file:

```bash
cargo run -p voxear_release -- launcher add out/Voxear.Launcher_0.2.0_amd64.AppImage --site site --channel stable --version 0.2.0 --target linux-x86_64 --key ~/.voxear-keys/engine.key
```

Upload the files and their `.sig` files to the `launcher-v0.2.0` release, then commit `site/launcher/` to
VoxearReleases' `gh-pages` branch.

The manifest each launcher reads is at `https://cookiejartools.github.io/VoxearReleases/launcher/stable.json`
(and `beta.json`):

```json
{
  "version": "0.2.0",
  "notes": "What changed",
  "pub_date": "2026-11-15T12:00:00Z",
  "platforms": {
    "linux-x86_64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6…",
      "url": "https://github.com/CookieJarTools/VoxearReleases/releases/download/launcher-v0.2.0/Voxear.Launcher_0.2.0_amd64.AppImage",
      "sha256": "9f2c…",
      "size": 84123456
    },
    "windows-x86_64": { "signature": "…", "url": "…/Voxear.Launcher_0.2.0_x64-setup.exe", "sha256": "…", "size": 5123456 }
  }
}
```

### Trying an update on your own machine

1. Make a key of your own with `voxear-release keygen --tauri`.
2. Build two versions with it, passing its public key as `VOXEAR_LAUNCHER_UPDATE_KEY`.
3. List the newer one with `launcher add … --download-base http://127.0.0.1:8000`.
4. Serve the site folder with `python3 -m http.server -b 127.0.0.1 8000`.
5. Run the older one with `VOXUP_LAUNCHER_UPDATES=http://127.0.0.1:8000/launcher`. Plain `http://` is accepted
   for this machine only; anywhere else, update URLs must be `https://`.

Set `HOME` to an empty folder as well, and the trial install, the app menu entry and the launcher folder all stay
inside it.
