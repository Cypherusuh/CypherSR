
import { FileTokenStore } from './auth/file-token-store.js';
import { loadConfig } from './config/load-config.js';
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

  twitch.onMessage((message) => {
    console.log(`[Twitch] ${message.displayName}: ${message.message}`);
  });

  await twitch.connect();

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
    await twitch.disconnect();
  };

  process.once('SIGINT', () => {
    void shutdown().catch((error: unknown) => {
      console.error('[SongRequest] Shutdown failed:', error);
      process.exitCode = 1;
    });
  });

  process.once('SIGTERM', () => {
    void shutdown().catch((error: unknown) => {
      console.error('[SongRequest] Shutdown failed:', error);
      process.exitCode = 1;
    });
  });
}

main().catch((error: unknown) => {
  console.error('[SongRequest] Startup failed:', error);
  process.exitCode = 1;
});
