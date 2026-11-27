import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import { SongRequestQueue } from '../queue/song-request-queue.js';
import { TrackResolver } from '../queue/track-resolver.js';
import { DefaultSongManager } from './default-song-manager.js';
import { MediaTranscoder } from './media-transcoder.js';
import { PlaybackCoordinator } from './playback-coordinator.js';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map(directory =>
      rm(directory, { recursive: true, force: true }),
    ),
  );
});

describe('PlaybackCoordinator default playback', () => {
  it('restarts the default song on EOF and ignores stale EOF notifications', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'cyphersr-playback-'));
    temporaryDirectories.push(directory);

    const mediaId = '12345678-1234-1234-1234-123456789abc';
    const mediaPath = join(directory, `${mediaId}.mp4`);
    const track = {
      id: 'default-track',
      title: 'Default track',
      url: 'https://example.com/default',
      duration: 120,
      thumbnail: null,
      uploader: null,
    };

    await writeFile(mediaPath, '');
    await writeFile(
      join(directory, 'metadata.json'),
      JSON.stringify({
        version: 1,
        query: track.url,
        track,
        mediaId,
      }),
    );

    const defaultSong = new DefaultSongManager(
      new TrackResolver('unused'),
      new MediaTranscoder({
        ytDlpPath: 'unused',
        ffmpegPath: 'unused',
        mediaDir: directory,
        maxVideoHeight: 720,
      }),
      directory,
    );
    await defaultSong.load();

    const coordinator = new PlaybackCoordinator(
      new SongRequestQueue(new TrackResolver('unused'), 5),
      new MediaTranscoder({
        ytDlpPath: 'unused',
        ffmpegPath: 'unused',
        mediaDir: directory,
        maxVideoHeight: 720,
      }),
      defaultSong,
      () => {},
      () => {},
    );

    const first = coordinator.getPlayerState('').current;
    expect(first?.source).toBe('default');
    expect(first).not.toBeNull();

    if (!first) {
      throw new Error('Expected the default track to be active.');
    }

    await expect(coordinator.trackEnded(first.requestId)).resolves.toBe(true);

    const restarted = coordinator.getPlayerState('').current;
    expect(restarted?.source).toBe('default');
    expect(restarted?.requestId).not.toBe(first.requestId);
    await expect(coordinator.trackEnded(first.requestId)).resolves.toBe(false);
  });
});
