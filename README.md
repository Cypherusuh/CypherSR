# CypherSR

A portable Twitch song request player for Windows, built for streamers using OBS.

## Download

**[Download the latest release](https://github.com/Cypherusuh/CypherSR/releases/latest)**

- **FULL** — For users who want the desktop app with the video player.
- **LITE** — For potato laptops or low-end PCs. Lightweight version without the desktop video player.

Download the latest version from the [Releases page](https://github.com/Cypherusuh/CypherSR/releases/latest) and choose the package that fits your needs.

## Features

- **Song requests:** `!sr <URL/title>` supports YouTube and Spotify links.
- **Default song:** Keep a song playing when the request queue is empty.
- **Queue management:** `!queue`, `!skip`, and `!wrongsong`.
- **Radio mode:** Toggle song requests and playback with `!radio`.
- **OBS overlays:** Browser sources for the player, queue, and recently played track.
- **Twitch connection status:** Check authentication and connection state through the local HTTP API.
- **Rust launcher:** Checks for app and bundled media-tool updates before starting the app.

## Installation

1. Download the latest full portable ZIP from the [latest release](https://github.com/Cypherusuh/CypherSR/releases/latest).
2. Extract the ZIP into a writable folder.
3. Run `CypherSR-Updater.exe`. The Rust launcher checks for app and bundled media-tool updates, then starts `CypherSR-Core.exe`. On first run, follow the Twitch login instructions.
4. Add the printed browser source URLs to OBS.

For the standalone LITE package, keep `CypherSR-Updater.exe`,
`CypherSR-Core.exe`, `public/`, and `vendor/` together, and launch the app with
`CypherSR-Updater.exe` to receive updates. If an update check fails, the launcher
reports the error and tries to start the installed version.

The FULL desktop package keeps the updater, core, `public/`, and `vendor/`
together inside `backend/`. Its launcher sets `SONG_REQUEST_RUNTIME_HOME` to
that directory so the backend resolves assets and update files beside its
binaries. Persistent data is stored in `backend/data/`, shared with the
standalone backend version through `SONG_REQUEST_HOME`. On first launch after
upgrading, files from the legacy Desktop-level `data/` directory are merged
into `backend/data/` without overwriting files already there. Do not delete
`backend/data/`; it stores backend configuration, authentication, and cached
media.

## Commands

| Command | Description |
|---|---|
| `!sr <URL/title>` | Request a song |
| `!radio` | Toggle radio and requests |
| `!queue` | Show the current track and queue |
| `!skip` | Skip the current track (moderators/broadcaster) |
| `!default` | Show the default song |
| `!default <URL>` | Set the default song (broadcaster) |
| `!default clear` | Clear the default song (broadcaster) |
| `!wrongsong` | Remove your latest pending request |

## Twitch connection status

The local HTTP API reports Twitch authentication and connection status. By default, the server listens on `http://127.0.0.1:8765` (the address and port can be changed in `data/config.json`).

| Method | Endpoint | Description |
|---|---|---|
| `GET` | `/api/twitch/status` | Returns `authenticated`, `connecting`, `connected`, `username`, and `error` |
| `POST` | `/api/twitch/connect` | Starts a Twitch connection |
| `POST` | `/api/twitch/disconnect` | Disconnects from Twitch |

## Requirements

- Windows x64
- An internet connection and a Twitch account
- OBS Studio for browser-source overlays

## Building releases

Set the release version in the repository-root `app-version.json` and
`desktop\Cargo.toml` to the same value (for example, `1.3.0`), then run these
scripts from PowerShell:

```powershell
.\scripts\build-backend.ps1
.\scripts\build-desktop.ps1
.\scripts\publish-package.ps1
```

Before building the backend, put `mpv.exe` and `mpv-register.bat` in
`vendor\mpv\`, or set `CYPHERSR_MPV_SOURCE` to their folder.
Artifacts are grouped under `release\v<version>\`: `Published App\` contains
the runnable package, `Publishing zip\` contains release ZIPs and `latest.json`,
`Backend Build\` contains backend binaries and assets, and `Desktop Build\`
contains the desktop executable. The `versioning\` folder contains
`desktop.json`, `backend.json`, and `vendor.json`. Existing `release\data\`
and package-local `data\` are preserved, and data directories are excluded
from release archives.

For a combined Desktop release, upload the contents of `Publishing zip\` plus
the versioned Desktop updater EXE from `Desktop Build\` to a GitHub release
whose tag is exactly `v<version>` (for example, `v1.2.1`). The EXE is a
separate required asset named exactly
`CypherSR-Desktop-GUI-v<version>-win-x64.exe`; the similarly named Desktop GUI
ZIP is only for manual installation. Include `latest.json` and all the ZIPs
from `Publishing zip\`, then mark the combined release as the latest release
so the backend updater can find its manifest.

After uploading the release assets and marking the release as latest, run
`.\scripts\verify-release.ps1`. It checks the public GitHub release, exact
updater EXE filename, all required assets, manifest URLs, and SHA-256 digests.
Do not announce the release unless this verification passes.

To repair the existing 1.2.1 release for already-installed 1.2.0 updaters,
rerun `.\scripts\publish-package.ps1` after generating the required build
artifacts. Upload the regenerated `latest.json` and all package ZIPs, and upload
`Desktop Build\CypherSR-Desktop-GUI-v1.2.1-win-x64.exe` with that exact name.
Make the `v1.2.1` release the latest release. Remove or replace the mistyped
`...win-x6.exe` release asset; the updater ignores it.

For a standalone backend release, upload the ZIPs from `Publishing zip\` and
`latest.json` from `Backend Build\`. `Published App\` is the ready-to-run
portable package.

## Support

Found a bug or have a suggestion? [Open an issue](https://github.com/Cypherusuh/CypherSR/issues).

## License

See the repository's [LICENSE](https://github.com/Cypherusuh/CypherSR/blob/main/LICENSE).
