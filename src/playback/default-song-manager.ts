
import { randomUUID } from 'node:crypto';
import {
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { join, resolve } from 'node:path';

import type { ResolvedTrack } from '../queue/types.js';
import type { TrackResolver } from '../queue/track-resolver.js';
import type {
  MediaTranscoder,
  PreparedMedia,
} from './media-transcoder.js';

interface DefaultSongMetadata {
  version: 1;
  query: string;
  track: ResolvedTrack;
  mediaId: string;
}

export interface DefaultSong {
  query: string;
  track: ResolvedTrack;
  media: PreparedMedia;
}

const MEDIA_ID_PATTERN = /^[a-f0-9-]{36}$/i;

export class DefaultSongManager {
  private readonly directory: string;
  private readonly metadataPath: string;
  private current: DefaultSong | null = null;
  private readonly mediaById = new Map<string, PreparedMedia>();
  private operationInProgress = false;

  constructor(
    private readonly resolver: TrackResolver,
    private readonly transcoder: MediaTranscoder,
    directory: string,
  ) {
    this.directory = resolve(directory);
    this.metadataPath = join(this.directory, 'metadata.json');
  }

  getDefault(): DefaultSong | null {
    return this.current;
  }

  getMediaById(mediaId: string): PreparedMedia | null {
    if (!MEDIA_ID_PATTERN.test(mediaId)) {
      return null;
    }

    return this.mediaById.get(mediaId) ?? null;
  }

  async load(): Promise<void> {
    await mkdir(this.directory, { recursive: true });

    let metadata: DefaultSongMetadata | null = null;

    try {
      const content = await readFile(this.metadataPath, 'utf8');
      const parsed: unknown = JSON.parse(content);

      if (this.isMetadata(parsed)) {
        metadata = parsed;
      } else {
        console.warn(
          '[DefaultSong] Saved metadata is invalid. Default playback is disabled.',
        );
      }
    } catch (error) {
      if (this.isFileNotFound(error)) {
        console.log('[DefaultSong] No saved default song found.');
      } else {
        console.error(
          '[DefaultSong] Failed to read saved metadata:',
          error,
        );
      }
    }

    if (metadata) {
      const mediaPath = join(
        this.directory,
        `${metadata.mediaId}.mp4`,
      );

      const file = await stat(mediaPath).catch(() => null);

      if (file?.isFile()) {
        const media: PreparedMedia = {
          id: metadata.mediaId,
          path: mediaPath,
          contentType: 'video/mp4',
          duration: metadata.track.duration,
        };

        this.current = {
          query: metadata.query,
          track: metadata.track,
          media,
        };

        this.mediaById.set(media.id, media);

        console.log(
          `[DefaultSong] Loaded cached default: ${metadata.track.title}`,
        );
      } else {
        console.warn(
          '[DefaultSong] Saved default media is missing. Playback is disabled; the song will not be downloaded automatically.',
        );
      }
    }

    await this.removeOrphanedFiles(this.current?.media.id ?? null);
  }

  async setDefault(query: string): Promise<DefaultSong> {
    if (this.operationInProgress) {
      throw new Error(
        'A default-song operation is already in progress. Please try again shortly.',
      );
    }

    this.operationInProgress = true;

    let prepared: PreparedMedia | null = null;

    try {
      const normalizedQuery = query.trim();

      if (!normalizedQuery) {
        throw new Error('Provide a YouTube URL or search query.');
      }

      console.log(`[DefaultSong] Resolving: ${normalizedQuery}`);
      const track = await this.resolver.resolve(normalizedQuery);

      // A fresh ID prevents overwriting a file the player may still be reading.
      const mediaId = randomUUID();
      prepared = await this.transcoder.prepare(track, mediaId);

      const metadata: DefaultSongMetadata = {
        version: 1,
        query: normalizedQuery,
        track,
        mediaId: prepared.id,
      };

      // Write the new metadata before switching the in-memory default.
      // If preparation or metadata persistence fails, the old default remains.
      const temporaryMetadataPath = join(
        this.directory,
        `.metadata-${randomUUID()}.tmp`,
      );

      try {
        await writeFile(
          temporaryMetadataPath,
          JSON.stringify(metadata, null, 2),
          'utf8',
        );

        await rename(temporaryMetadataPath, this.metadataPath);
      } catch (error) {
        await rm(temporaryMetadataPath, { force: true }).catch(() => {});
        throw error;
      }

      const previous = this.current;

      this.current = {
        query: normalizedQuery,
        track,
        media: prepared,
      };

      this.mediaById.set(prepared.id, prepared);
      prepared = null;

      console.log(`[DefaultSong] Default saved: ${track.title}`);

      if (previous) {
        this.scheduleMediaCleanup(previous.media);
      }

      return this.current;
    } catch (error) {
      if (prepared) {
        await rm(prepared.path, { force: true }).catch(() => {});
      }

      throw error;
    } finally {
      this.operationInProgress = false;
    }
  }

  async clearDefault(): Promise<boolean> {
    if (this.operationInProgress) {
      throw new Error(
        'A default-song operation is already in progress. Please try again shortly.',
      );
    }

    this.operationInProgress = true;

    try {
      if (!this.current) {
        return false;
      }

      // Clear the persisted pointer before changing in-memory state.
      await rm(this.metadataPath, { force: true });

      const previous = this.current;
      this.current = null;

      this.scheduleMediaCleanup(previous.media);

      console.log('[DefaultSong] Default song cleared.');
      return true;
    } finally {
      this.operationInProgress = false;
    }
  }

  private isMetadata(value: unknown): value is DefaultSongMetadata {
    if (!value || typeof value !== 'object') {
      return false;
    }

    const metadata = value as Partial<DefaultSongMetadata>;
    const track = metadata.track;

    return (
      metadata.version === 1 &&
      typeof metadata.query === 'string' &&
      metadata.query.trim().length > 0 &&
      typeof metadata.mediaId === 'string' &&
      MEDIA_ID_PATTERN.test(metadata.mediaId) &&
      !!track &&
      typeof track === 'object' &&
      typeof track.id === 'string' &&
      typeof track.title === 'string' &&
      typeof track.url === 'string' &&
      (typeof track.duration === 'number' || track.duration === null) &&
      (typeof track.thumbnail === 'string' || track.thumbnail === null) &&
      (typeof track.uploader === 'string' || track.uploader === null)
    );
  }

  private async removeOrphanedFiles(activeMediaId: string | null): Promise<void> {
    const entries = await readdir(this.directory, {
      withFileTypes: true,
    });

    for (const entry of entries) {
      if (!entry.isFile()) {
        continue;
      }

      const isMedia = entry.name.endsWith('.mp4');
      const isWorkDirectoryArtifact = entry.name.startsWith('.work-');

      if (isMedia) {
        const mediaId = entry.name.slice(0, -4);

        if (mediaId === activeMediaId) {
          continue;
        }

        if (MEDIA_ID_PATTERN.test(mediaId)) {
          await rm(join(this.directory, entry.name), { force: true })
            .catch(error => {
              console.warn(
                `[DefaultSong] Could not remove stale media ${entry.name}:`,
                error,
              );
            });
        }
      } else if (isWorkDirectoryArtifact) {
        await rm(join(this.directory, entry.name), {
          recursive: true,
          force: true,
        }).catch(() => {});
      } else if (/^\.metadata-.*\.tmp$/.test(entry.name)) {
        await rm(join(this.directory, entry.name), { force: true })
          .catch(() => {});
      }
    }
  }

  private scheduleMediaCleanup(media: PreparedMedia): void {
    // Give the browser time to switch away from the previous media URL.
    // If Windows still has the file open, cleanup is retried on next startup.
    setTimeout(() => {
      void (async () => {
        try {
          await rm(media.path, { force: true });
          this.mediaById.delete(media.id);
        } catch (error) {
          console.warn(
            `[DefaultSong] Could not remove old media ${media.id}; it will be cleaned up on next startup.`,
            error,
          );
        }
      })();
    }, 120_000).unref();
  }

  private isFileNotFound(error: unknown): boolean {
    return (
      typeof error === 'object' &&
      error !== null &&
      'code' in error &&
      error.code === 'ENOENT'
    );
  }
}
