import {
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { dirname } from 'node:path';

import type { SongRequestConfig } from './types.js';
import { dataPath, runtimePath } from './runtime-paths.js';

const DEFAULT_TWITCH_CLIENT_ID = 'leq7ln0qrqwvdxkrr8qtqmnxz412k8';

interface PersistedConfig {
  twitch?: {
    clientId?: string;
    channel?: string;
  };
  queue?: {
    limit?: number;
  };
  commands?: {
    prefix?: string;
  };
  server?: {
    host?: string;
    port?: number;
  };
  playback?: {
    maxVideoHeight?: number;
  };
}

const configPath = dataPath('config.json');

function defaultPersistedConfig(): PersistedConfig {
  return {
    twitch: {
      clientId: DEFAULT_TWITCH_CLIENT_ID,
      channel: '',
    },
    queue: {
      limit: 10,
    },
    commands: {
      prefix: '!',
    },
    server: {
      host: '127.0.0.1',
      port: 8765,
    },
    playback: {
      maxVideoHeight: 360,
    },
  };
}

function readPersistedConfig(): PersistedConfig {
  mkdirSync(dirname(configPath), { recursive: true });

  if (!existsSync(configPath)) {
    const defaults = defaultPersistedConfig();
    writeConfigFile(defaults);
    return defaults;
  }

  let parsed: unknown;

  try {
    parsed = JSON.parse(readFileSync(configPath, 'utf8'));
  } catch (error) {
    throw new Error(
      `Unable to read ${configPath}. Check that it contains valid JSON.`,
      { cause: error },
    );
  }

  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new Error(`Invalid configuration file: ${configPath}`);
  }

  const stored = parsed as PersistedConfig;
  const defaults = defaultPersistedConfig();

  return {
    twitch: {
      clientId:
        typeof stored.twitch?.clientId === 'string'
          ? stored.twitch.clientId
          : defaults.twitch!.clientId,
      channel:
        typeof stored.twitch?.channel === 'string'
          ? stored.twitch.channel
          : defaults.twitch!.channel,
    },
    queue: {
      limit:
        typeof stored.queue?.limit === 'number'
          ? stored.queue.limit
          : defaults.queue!.limit,
    },
    commands: {
      prefix:
        typeof stored.commands?.prefix === 'string'
          ? stored.commands.prefix
          : defaults.commands!.prefix,
    },
    server: {
      host:
        typeof stored.server?.host === 'string'
          ? stored.server.host
          : defaults.server!.host,
      port:
        typeof stored.server?.port === 'number'
          ? stored.server.port
          : defaults.server!.port,
    },
    playback: {
      maxVideoHeight:
        typeof stored.playback?.maxVideoHeight === 'number'
          ? stored.playback.maxVideoHeight
          : defaults.playback!.maxVideoHeight,
    },
  };
}

function writeConfigFile(config: PersistedConfig): void {
  mkdirSync(dirname(configPath), { recursive: true });

  const temporaryPath = `${configPath}.tmp`;
  if (existsSync(temporaryPath)) {
    unlinkSync(temporaryPath);
  }
  try {
    writeFileSync(temporaryPath, `${JSON.stringify(config, null, 2)}\n`, {
      encoding: 'utf8',
      mode: 0o600,
    });

    renameSync(temporaryPath, configPath);
  } catch (error) {
    try {
      if (existsSync(temporaryPath)) {
        const { unlinkSync } = require('node:fs') as typeof import('node:fs');
        unlinkSync(temporaryPath);
      }
    } catch {
      // Preserve the original write error.
    }

    throw error;
  }
}

export function saveConfig(config: SongRequestConfig): void {
  writeConfigFile({
    twitch: {
      clientId: config.twitch.clientId,
      channel: config.twitch.channel,
    },
    queue: {
      limit: config.queue.limit,
    },
    commands: {
      prefix: config.commands.prefix,
    },
    server: {
      host: config.server.host,
      port: config.server.port,
    },
    playback: {
      maxVideoHeight: config.playback.maxVideoHeight,
    },
  });
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
  const stored = readPersistedConfig();

  const spotifyClientId = optionalEnv('SPOTIFY_CLIENT_ID');
  const spotifyClientSecret = optionalEnv('SPOTIFY_CLIENT_SECRET');

  if (Boolean(spotifyClientId) !== Boolean(spotifyClientSecret)) {
    throw new Error(
      'Set both SPOTIFY_CLIENT_ID and SPOTIFY_CLIENT_SECRET, or leave both unset.',
    );
  }

  return {
    twitch: {
      clientId:
        optionalEnv('TWITCH_CLIENT_ID') ??
        stored.twitch!.clientId!,
      channel: (
        optionalEnv('TWITCH_CHANNEL') ??
        stored.twitch!.channel!
      )
        .replace(/^@/, '')
        .toLowerCase(),
      scopes: ['user:read:chat', 'user:write:chat'],
      tokenPath: process.env.TWITCH_TOKEN_PATH?.trim()
        ? runtimePath(process.env.TWITCH_TOKEN_PATH.trim())
        : dataPath('twitch-tokens.json'),
    },

    spotify: {
      clientId: spotifyClientId,
      clientSecret: spotifyClientSecret,
    },

    queue: {
      limit: positiveIntegerEnv(
        'SONG_REQUEST_QUEUE_LIMIT',
        stored.queue!.limit!,
      ),
    },

    commands: {
      prefix: optionalEnv('COMMAND_PREFIX') ?? stored.commands!.prefix!,
    },

    server: {
      host: optionalEnv('SONG_REQUEST_HOST') ?? stored.server!.host!,
      port: positiveIntegerEnv('SONG_REQUEST_PORT', stored.server!.port!),
    },

    playback: {
      radioUrl: optionalEnv('RADIO_URL'),
      mpvPath: runtimePath(optionalEnv('MPV_PATH') || 'mpv'),
      ytDlpPath: runtimePath(
        optionalEnv('YT_DLP_PATH') || 'vendor/yt-dlp.exe',
      ),
      ffmpegPath: runtimePath(
        optionalEnv('FFMPEG_PATH') || 'vendor/ffmpeg/bin/ffmpeg.exe',
      ),
      mediaDir: process.env.SONG_REQUEST_MEDIA_DIR?.trim()
        ? runtimePath(process.env.SONG_REQUEST_MEDIA_DIR.trim())
        : dataPath('cache/media'),
      maxVideoHeight: positiveIntegerEnv(
        'SONG_REQUEST_MAX_VIDEO_HEIGHT',
        stored.playback!.maxVideoHeight!,
      ),
    },
  };
}