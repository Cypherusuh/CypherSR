
import type { SongRequestConfig } from './types.js';

function requiredEnv(name: string): string {
  const value = process.env[name]?.trim();

  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }

  return value;
}

function optionalEnv(name: string): string | undefined {
  const value = process.env[name]?.trim();
  return value || undefined;
}

function positiveIntegerEnv(name: string, fallback: number): number {
  const raw = process.env[name];

  if (raw === undefined || raw.trim() === '') {
    return fallback;
  }

  const value = Number(raw);

  if (!Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${name} must be a positive integer.`);
  }

  return value;
}

export function loadConfig(): SongRequestConfig {
  const spotifyClientId = optionalEnv('SPOTIFY_CLIENT_ID');
  const spotifyClientSecret = optionalEnv('SPOTIFY_CLIENT_SECRET');

  if (Boolean(spotifyClientId) !== Boolean(spotifyClientSecret)) {
    throw new Error(
      'Set both SPOTIFY_CLIENT_ID and SPOTIFY_CLIENT_SECRET, or leave both unset.',
    );
  }

  return {
    twitch: {
      clientId: requiredEnv('TWITCH_CLIENT_ID'),
      channel: requiredEnv('TWITCH_CHANNEL').replace(/^@/, '').toLowerCase(),
      scopes: ['user:read:chat', 'user:write:chat'],
      tokenPath: process.env.TWITCH_TOKEN_PATH?.trim() || '.tokens/twitch.json',
    },

    spotify: {
      clientId: spotifyClientId,
      clientSecret: spotifyClientSecret,
    },

    queue: {
      limit: positiveIntegerEnv('SONG_REQUEST_QUEUE_LIMIT', 10),
    },

    commands: {
      prefix: process.env.COMMAND_PREFIX?.trim() || '!',
    },

    server: {
      host: process.env.SONG_REQUEST_HOST?.trim() || '127.0.0.1',
      port: positiveIntegerEnv('SONG_REQUEST_PORT', 8765),
    },

    playback: {
      radioUrl: process.env.RADIO_URL?.trim() || undefined,
      mpvPath: process.env.MPV_PATH?.trim() || 'mpv',
      ytDlpPath: process.env.YT_DLP_PATH?.trim() || 'yt-dlp',
      ffmpegPath: process.env.FFMPEG_PATH?.trim() || 'ffmpeg',
      mediaDir:
        process.env.SONG_REQUEST_MEDIA_DIR?.trim() || '.cache/song-request/media',
      maxVideoHeight: positiveIntegerEnv('SONG_REQUEST_MAX_VIDEO_HEIGHT', 360),
    },
  };
}
