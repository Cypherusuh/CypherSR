# CypherSR

A portable Twitch song request player for Windows, built for streamers using OBS.

## Features

- **Song requests:** `!sr <URL/title>` supports YouTube and Spotify links.
- **Default song:** Keep a song playing when the request queue is empty.
- **Queue management:** `!queue`, `!skip`, and `!wrongsong`.
- **Radio mode:** Toggle song requests and playback with `!radio`.
- **OBS overlays:** Browser sources for the player, queue, and recently played track.
- **Automatic updates:** Updates the app while preserving your configuration and cached data.

## Installation

1. Download `CypherSR-1.0.0-win-x64.zip` from the [latest release](https://github.com/Cypherusuh/CypherSR/releases/latest).
2. Extract the ZIP into a writable folder.
3. Run `CypherSR.exe` and follow the Twitch login instructions.
4. Add the printed browser source URLs to OBS.

Keep `CypherSR.exe`, `CypherSR-core.exe`, `public/`, and `vendor/` together. Do not delete the `data/` folder; it stores your configuration, authentication, and cached media.

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

## Requirements

- Windows x64
- An internet connection and a Twitch account
- OBS Studio for browser-source overlays

## Support

Found a bug or have a suggestion? [Open an issue](https://github.com/Cypherusuh/CypherSR/issues).

## License

See the repository's [LICENSE](https://github.com/Cypherusuh/CypherSR/blob/main/LICENSE).
