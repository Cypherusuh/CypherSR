import { FileTokenStore } from './auth/file-token-store.js';
import { loadConfig } from './config/load-config.js';
import { MediaTranscoder } from './playback/media-transcoder.js';
import { MediaServer } from './server/media-server.js';
import { TrackResolver } from './queue/track-resolver.js';
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

  await mediaServer.start();

  try {
    await twitch.connect();
  } catch (error) {
    await mediaServer.stop();
    throw error;
  }

  twitch.onMessage((message) => {
    console.log(`[Twitch] ${message.displayName}: ${message.message}`);
  });

  console.log(
    `[SongRequest] Connected to Twitch channel: ${config.twitch.channel}`,
  );
  console.log('[SongRequest] Listening for chat messages.');

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
        console.error('[SongRequest] Shutdown failed:', result.reason);
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