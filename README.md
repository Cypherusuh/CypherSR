# CypherSR

A portable Twitch song request player for Windows, built for streamers using OBS.

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
3. Run `CypherSR.exe`. The Rust launcher checks for app and bundled media-tool updates, then starts `CypherSR-core.exe`. On first run, follow the Twitch login instructions.
4. Add the printed browser source URLs to OBS.

Keep `CypherSR.exe`, `CypherSR-core.exe`, `public/`, and `vendor/` together, and launch the app with `CypherSR.exe` to receive updates. If an update check fails, the launcher reports the error and tries to start the installed version. Do not delete the `data/` folder; it stores your configuration, authentication, and cached media.

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

## Support

Found a bug or have a suggestion? [Open an issue](https://github.com/Cypherusuh/CypherSR/issues).

## License

See the repository's [LICENSE](https://github.com/Cypherusuh/CypherSR/blob/main/LICENSE).
