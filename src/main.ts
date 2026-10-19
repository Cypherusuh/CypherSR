
import { FileTokenStore } from './auth/file-token-store.js';
import { CommandRouter } from './commands/command-router.js';
import { createSongRequestCommands } from './commands/song-request-commands.js';
import { loadConfig } from './config/load-config.js';
import { PlaybackCoordinator } from './playback/playback-coordinator.js';
import { MediaTranscoder } from './playback/media-transcoder.js';
import { SongRequestQueue } from './queue/song-request-queue.js';
import { TrackResolver } from './queue/track-resolver.js';
import { MediaServer } from './server/media-server.js';
import { TwitchClient } from './twitch/twitch-client.js';

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
    await twitch.login();
  }

  const trackResolver = new TrackResolver(config.playback.ytDlpPath);

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
  );

  const playback = new PlaybackCoordinator(
    queue,
    transcoder,
    (media) => mediaServer.register(media),
    (mediaId) => mediaServer.unregister(mediaId),
  );

  mediaServer.attachPlaybackCoordinator(playback);

  let requestModeEnabled = false;

  const commandRouter = new CommandRouter(
    config.commands.prefix,
    createSongRequestCommands({
      isRequestModeEnabled: () => requestModeEnabled,

      toggleRequestMode: async () => {
        requestModeEnabled = !requestModeEnabled;
        return requestModeEnabled;
      },

      requestSong: (message, query) =>
        playback.requestSong(message, query),

      getQueue: () => playback.getQueue(),

      skipTrack: () => playback.skipTrack(),
    }),
    (message) => twitch.sendMessage(message),
  );

  await mediaServer.start();

  try {
    await twitch.connect();
  } catch (error) {
    await mediaServer.stop();
    throw error;
  }

  twitch.onMessage((message) => {
    console.log(
      `[Twitch] ${message.displayName}: ${message.message}`,
    );

    void commandRouter.handleMessage(message);
  });

  console.log(
    `[SongRequest] Connected to Twitch channel: ${config.twitch.channel}`,
  );
  console.log('[SongRequest] Listening for chat messages.');
  console.log('[Playback] Open the local player page to start playback.');

  let shuttingDown = false;

  const shutdown = async () => {
    if (shuttingDown) {
      return;
    }

    shuttingDown = true;
    console.log('\n[SongRequest] Disconnecting...');

    const results = await Promise.allSettled([
      twitch.disconnect(),
      mediaServer.stop(),
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
  };

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
