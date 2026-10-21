
import { FileTokenStore } from './auth/file-token-store.js';
import { CommandRouter } from './commands/command-router.js';
import { createSongRequestCommands } from './commands/song-request-commands.js';
import { loadConfig, saveConfig } from './config/load-config.js';
import { PlaybackCoordinator } from './playback/playback-coordinator.js';
import { MediaTranscoder } from './playback/media-transcoder.js';
import { SongRequestQueue } from './queue/song-request-queue.js';
import { TrackResolver } from './queue/track-resolver.js';
import { MediaServer } from './server/media-server.js';
import { TwitchClient } from './twitch/twitch-client.js';
import { SpotifyTrackResolver } from './queue/spotify-track-resolver.js';
import { spawn } from 'node:child_process';
import { DefaultSongManager } from './playback/default-song-manager.js';
import { dataPath } from './config/runtime-paths.js'
import { ResolvedTrack } from './queue/types.js';

async function copyToClipboard(text: string): Promise<void> {
  if (process.platform !== 'win32') {
    return;
  }

  await new Promise<void>((resolve, reject) => {
    const child = spawn('clip.exe', [], {
      windowsHide: true,
      stdio: ['pipe', 'ignore', 'ignore'],
    });

    child.once('error', reject);
    child.once('close', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`clip.exe exited with code ${code}`));
      }
    });
    child.stdin.end(text);
  });
}

async function main(): Promise<void> {
  const config = loadConfig();
  const tokenStore = new FileTokenStore(config.twitch.tokenPath);

  const twitch = new TwitchClient({
    clientId: config.twitch.clientId,
    channel: config.twitch.channel,
    scopes: config.twitch.scopes,
    tokenStore,
  });

  const savedTokens = await tokenStore.load();

  if (!savedTokens) {
    console.log('[Twitch] No saved credentials found. Starting login...');
    await twitch.login();
  }

  const resolveAndSaveChannel = async (): Promise<void> => {
    const channel = await twitch.getAuthenticatedLogin();

    config.twitch.channel = channel;
    twitch.setChannel(channel);
    saveConfig(config);

    console.log(`[Twitch] Authenticated account: ${channel}`);
    console.log('[Config] Twitch channel saved to data/config.json.');
  };

  const spotifyResolver = new SpotifyTrackResolver();

  const trackResolver = new TrackResolver(
    config.playback.ytDlpPath,
    30_000,
    spotifyResolver,
  );

  const defaultMediaDir = dataPath('default-song');

  const defaultTranscoder = new MediaTranscoder({
    ytDlpPath: config.playback.ytDlpPath,
    ffmpegPath: config.playback.ffmpegPath,
    mediaDir: defaultMediaDir,
    maxVideoHeight: config.playback.maxVideoHeight,
  });

  const defaultSongManager = new DefaultSongManager(
    trackResolver,
    defaultTranscoder,
    defaultMediaDir,
  );

  await defaultSongManager.load();

  const queue = new SongRequestQueue(
    trackResolver,
    config.queue.limit,
  );

  const transcoder = new MediaTranscoder({
    ytDlpPath: config.playback.ytDlpPath,
    ffmpegPath: config.playback.ffmpegPath,
    mediaDir: config.playback.mediaDir,
    maxVideoHeight: config.playback.maxVideoHeight,
  });

  const mediaServer = new MediaServer(
    config.server.host,
    config.server.port,
    config.playback.mediaDir,
    trackResolver,
    transcoder,
    defaultSongManager,
  );

  const playback = new PlaybackCoordinator(
    queue,
    transcoder,
    defaultSongManager,
    media => mediaServer.register(media),
    mediaId => mediaServer.unregister(mediaId),
  );

  mediaServer.attachPlaybackCoordinator(playback);

  let requestModeEnabled = true;

  const commandRouter = new CommandRouter(
    config.commands.prefix,
    createSongRequestCommands({
      isRequestModeEnabled: () => requestModeEnabled,

      toggleRequestMode: async () => {
        requestModeEnabled = !requestModeEnabled;
        return requestModeEnabled;
      },

      requestSong: (message, query) => playback.requestSong(message, query),

      getQueue: () => playback.getQueue(),

      skipTrack: () => playback.skipTrack(),
      removeLatestRequest: (userId) => queue.removeLatestPendingByUser(userId),
      getDefaultSong: () => {
        const song = defaultSongManager.getDefault();

        return song
          ? {
            query: song.query,
            track: song.track,
          }
          : null;
      },

      setDefaultSong: async query => {
        const song = await defaultSongManager.setDefault(query);

        return `Default song set to "${song.track.title}".`;
      },

      clearDefaultSong: async () => {
        const cleared = await defaultSongManager.clearDefault();

        return cleared
          ? 'Default song cleared.'
          : 'No default song is configured.';
      },
    }),
    (message) => twitch.sendMessage(message),
  );

  let mediaServerStarted = false;

  const startMediaServer = async (): Promise<void> => {
    if (mediaServerStarted) {
      return;
    }

    await mediaServer.start();
    mediaServerStarted = true;
  };

  const stopMediaServer = async (): Promise<void> => {
    if (!mediaServerStarted) {
      return;
    }

    await mediaServer.stop();
    mediaServerStarted = false;
  };

  const isAuthenticationFailure = (error: unknown): boolean => {
    const message = error instanceof Error ? error.message : String(error);

    return /token refresh failed|token validation failed|invalid refresh token|refresh token.*invalid/i.test(
      message,
    );
  };

  try {
    await resolveAndSaveChannel();
    await startMediaServer();
    await twitch.connect();
  } catch (error) {
    if (!isAuthenticationFailure(error)) {
      await stopMediaServer();
      throw error;
    }

    console.warn(
      '[Twitch] Saved credentials are no longer valid. Starting login again.',
    );

    try {
      await tokenStore.clear();
      await twitch.login();

      await resolveAndSaveChannel();
      await startMediaServer();
      await twitch.connect();
    } catch (loginError) {
      await stopMediaServer();
      throw loginError;
    }
  }

  twitch.onMessage((message) => {
    console.log(
      `[Twitch] ${message.displayName}: ${message.message}`,
    );

    void commandRouter.handleMessage(message);
  });

  try {
    await twitch.sendMessage('Radio and song requests enabled');
    console.log('[SongRequest] Announced "sr on" in Twitch chat.');
  } catch (error) {
    console.error(
      '[SongRequest] Failed to announce startup status in Twitch chat:',
      error,
    );
  }

  console.log(
    `[SongRequest] Connected to Twitch channel: ${config.twitch.channel}`,
  );
  console.log('[SongRequest] Listening for chat messages.');
  console.log('');
  console.log('=== Song Request Commands ===');
  console.log('  !sr <URL/title>  Request a song by URL or title.');
  console.log('  !radio           Toggle radio and song requests on/off.');
  console.log('  !queue           Show the current track and pending requests.');
  console.log('  !skip            Skip the current track (mods/broadcaster).');
  console.log('  !default         Show the current default song.');
  console.log('  !default <URL>   Set the default song (broadcaster).');
  console.log('  !default clear   Clear the default song (broadcaster).');
  console.log('  !wrongsong       Remove your latest pending request.');
  console.log('');
  console.log('=== OBS Browser Sources ===');

  const baseUrl = `http://${config.server.host}:${config.server.port}`;

  console.log(`  Player:     ${baseUrl}/player.html`);
  console.log(`  Played Now: ${baseUrl}/played-now.html`);
  console.log(`  Queue:      ${baseUrl}/queue.html`);
  console.log(`  Test:       ${baseUrl}/browser-player.html`);
  console.log('');

  let shutdownPromise: Promise<void> | undefined;

  const shutdown = (): Promise<void> => {
    if (shutdownPromise) {
      return shutdownPromise;
    }

    shutdownPromise = (async () => {
      console.log('\n[SongRequest] Disconnecting...');

      const results = await Promise.allSettled([
        twitch.disconnect(),
        stopMediaServer(),
      ]);

      for (const result of results) {
        if (result.status === 'rejected') {
          console.error(
            '[SongRequest] Shutdown failed:',
            result.reason,
          );
          process.exitCode = 1;
        }
      }

      console.log('[SongRequest] Shutdown complete.');
    })();

    return shutdownPromise;
  };

  const handleShutdownSignal = (): void => {
    void shutdown();
  };

  process.once('SIGINT', handleShutdownSignal);
  process.once('SIGTERM', handleShutdownSignal);
  process.once('SIGHUP', handleShutdownSignal);

  process.once('SIGINT', () => {
    void shutdown();
  });

  process.once('SIGTERM', () => {
    void shutdown();
  });
}

main().catch((error: unknown) => {
  console.error('[SongRequest] Startup failed:', error);
  process.exitCode = 1;
});
