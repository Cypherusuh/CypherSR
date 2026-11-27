# CypherSR

CypherSR lets viewers request songs in Twitch chat and plays them on your
stream. It includes a Windows desktop app, an optional lightweight package, and
browser overlays for OBS.

## What you need

- A Windows 64-bit computer with an internet connection.
- A Twitch account for the channel that will use song requests.
- OBS Studio if you want to show the player, queue, or song information on
  stream.

## Download and start

Download the latest release from the
[CypherSR Releases page](https://github.com/Cypherusuh/CypherSR/releases/latest).
Choose one package:

- **Full** — Recommended. Includes the desktop app, native video preview,
  playlist controls, and OBS overlay URL buttons.
- **Lite** — Backend only. Choose this if you do not need the desktop window;
  use the browser overlays to play songs and show information in OBS.

1. Download the package ZIP and extract it to a folder you can write to, such as
   `Documents\CypherSR`. Do not run the app from inside the ZIP.
2. For **Full**, open `CypherSR-Desktop.exe`. For **Lite**, open
   `CypherSR-Updater.exe`.
3. Keep the app running while you stream. The Full app starts the song service
   for you; Lite starts it with the updater.
4. On first launch, connect the Twitch account for your channel and approve
   access when Twitch asks. In Full, use **Connect** in the top bar if needed.
   The status should change to **Online** when the connection is ready.

CypherSR checks for available updates when it starts. Keep the extracted
package together so the app can find its files.

### Save your setup

CypherSR keeps its settings, Twitch sign-in, and default-song files in the
package's `data` folder. In Full this is `backend\data`; in Lite it is
`data`. Keep this folder when updating or moving the app. Copy it somewhere
safe if you want a backup.

## Add overlays to OBS

The overlays are transparent browser pages served by CypherSR. Start CypherSR
before adding them. In OBS, add a **Browser** source and paste the overlay URL:

| Overlay | Default URL | What it shows |
|---|---|---|
| Player | `http://127.0.0.1:8765/player.html` | Song video and audio |
| Queue | `http://127.0.0.1:8765/queue.html` | The current request and waiting requests |
| Played Now | `http://127.0.0.1:8765/played-now.html` | Current song title, artist, and progress |

In the Full desktop app, expand **OBS Overlays** and click **Copy URL** next to
the overlay you want. Paste that URL into the OBS Browser source. The copied
URL uses your current server settings; the default addresses above work with a
standard setup.

Use the **Player** source for stream audio/video. Make sure the source is
active in OBS and its audio is not muted. The **Test** page
(`http://127.0.0.1:8765/browser-player.html`) is for checking playback in a
regular browser; it is not normally added as an OBS overlay.

## Full desktop app

The Full app includes:

- **Twitch status** in the title bar, with Connect, Disconnect, and Retry
  controls.
- **Now Playing** information and a native video preview that you can show or
  hide.
- **Playback controls** for play/pause, seeking, and volume, plus a Skip button.
- **Playlist controls** to move waiting songs up or down and remove them.
- **OBS Overlays** buttons that copy the Player, Queue, and Played Now URLs.

The Lite package does not include this desktop window. It can still play songs
through the Player browser source and provide the Queue and Played Now overlays.

## Twitch chat commands

Commands use `!` by default. Viewers can request songs while request mode is
enabled. YouTube links or song searches are supported; individual Spotify track
links are also accepted. Spotify links are matched to a playable YouTube
version; Spotify playlists are not supported.

| Command | Who can use it | What it does |
|---|---|---|
| `!sr <URL or song title>` | Everyone, when requests are enabled | Add a song request. |
| `!song <URL or song title>` | Same as `!sr` | Alias for `!sr`. |
| `!radio` | Broadcaster | Turn new song requests on or off. Requests are enabled when the service starts. |
| `!queue` | Everyone | Show the current request and waiting requests. |
| `!q` | Everyone | Alias for `!queue`. |
| `!skip` | Moderators and broadcaster | Skip the current requested song. If the default song is playing with no waiting requests, it starts again. |
| `!default` | Everyone | Show the configured default song. |
| `!default <URL or song title>` | Broadcaster | Set the default song. The bot posts a lookup status while it prepares the song. |
| `!default clear` | Broadcaster | Remove the default song. |
| `!wrongsong` | The person who requested it | Remove your most recent waiting request. It cannot remove a song that is already playing. |

The default song plays when no requested song is active and repeats when it
reaches the end. Requests can take a little while to look up and prepare. The
queue allows up to 10 waiting songs in total and 2 waiting songs per viewer by
default; requests are rejected when either limit is reached.

## Troubleshooting

- **The app is not responding to chat:** Check that Twitch shows **Online**.
  If requests were turned off with `!radio`, the broadcaster can use `!radio`
  again to turn them back on.
- **Twitch is offline or reconnecting:** Check your internet connection. In the
  Full app, click **Retry** if it appears.
- **A song does not play:** Check the link or search text, then use `!queue` to
  see whether the request was accepted. A full queue or an unavailable song can
  prevent a request from being added.
- **OBS has no song audio:** Confirm the Player browser source is active and
  not muted in OBS. The desktop preview by itself is not a substitute for the
  Player browser source in your stream.
- **Need to report a problem:** On Full, attach the latest log at
  `CypherSR Desktop\logs\debug.log` (beside the desktop app) when
  [opening an issue](https://github.com/Cypherusuh/CypherSR/issues). It contains
  both desktop (`[front]`) and backend (`[back]`) messages; the previous two
  logs are kept as `debug.1.log` and `debug.2.log`.

## Support

For help or to report a problem, [open an issue](https://github.com/Cypherusuh/CypherSR/issues).
