
import { rm } from 'node:fs/promises';

import type { ChatMessage } from '../commands/types.js';
import {
  SongRequestLimitError,
  type SongRequestQueue,
} from '../queue/song-request-queue.js';
import type { QueueSnapshot, SongRequest } from '../queue/types.js';
import type {
  PreparedMedia,
  MediaTranscoder,
} from './media-transcoder.js';
import type { DefaultSongManager } from './default-song-manager.js';

export interface PlayerTrack {
  requestId: string;
  title: string;
  uploader: string | null;
  requester: string;
  duration: number | null;
  mediaUrl: string;
  source: 'request' | 'default';
}

export interface PlayerState {
  current: PlayerTrack | null;
}

export class PlaybackCoordinator {
  private readonly prepared = new Map<string, PreparedMedia>();
  private currentMedia: PreparedMedia | null = null;
  private pumping = false;
  private rerun = false;
  private defaultRestartRevision = 0;

  constructor(
    private readonly queue: SongRequestQueue,
    private readonly transcoder: MediaTranscoder,
    private readonly defaultSong: DefaultSongManager,
    private readonly registerMedia: (media: PreparedMedia) => void,
    private readonly unregisterMedia: (mediaId: string) => void,
  ) { }

  async requestSong(message: ChatMessage, query: string): Promise<string> {
    let request: SongRequest;

    try {
      request = this.queue.reserveRequest(message, query);
    } catch (error) {
      if (error instanceof SongRequestLimitError) {
        return error.message;
      }

      throw error;
    }

    try {
      const resolved = await this.queue.resolveRequest(request.id);
      this.schedulePump();

      const title = resolved.track?.title ?? resolved.query;
      return `Added "${title}" to the song queue.`;
    } catch (error) {
      console.error(
        `[Playback] Failed to resolve "${request.query}":`,
        error,
      );
      this.schedulePump();
      throw error;
    }
  }

  getQueue(): QueueSnapshot {
    return this.queue.getSnapshot();
  }

  getPlayerState(origin: string): PlayerState {
    const snapshot = this.queue.getSnapshot();
    const current = snapshot.current;

    if (current?.track && this.currentMedia) {
      return {
        current: {
          requestId: current.id,
          title: current.track.title,
          uploader: current.track.uploader,
          requester: current.requester.displayName,
          duration: current.track.duration,
          mediaUrl: `${origin}/api/media/${this.currentMedia.id}`,
          source: 'request',
        },
      };
    }

    // Keep the default playing while requests are resolving or preparing.
    // Only an active request without registered media should suppress fallback.
    if (current) {
      return { current: null };
    }

    const fallback = this.defaultSong.getDefault();

    if (!fallback) {
      return { current: null };
    }

    return {
      current: {
        requestId:
          `default-${fallback.media.id}-${this.defaultRestartRevision}`,
        title: fallback.track.title,
        uploader: fallback.track.uploader,
        requester: 'Default song',
        duration: fallback.track.duration,
        mediaUrl: `${origin}/api/default-media/${fallback.media.id}`,
        source: 'default',
      },
    };
  }

  async trackEnded(requestId: string): Promise<boolean> {
    return this.finishTrack(requestId);
  }

  async skipTrack(): Promise<boolean> {
    const current = this.queue.getCurrent();

    if (current) {
      return this.finishTrack(current.id);
    }

    const snapshot = this.queue.getSnapshot();

    // Do not skip unresolved/pending requests. Only restart the fallback
    // when the queue is genuinely empty.
    if (snapshot.pending.length > 0) {
      return false;
    }

    if (!this.defaultSong.getDefault()) {
      return false;
    }

    this.defaultRestartRevision += 1;
    return true;
  }

  private async finishTrack(requestId: string): Promise<boolean> {
    if (!this.queue.completeCurrent(requestId)) {
      return false;
    }

    const media = this.currentMedia;
    this.currentMedia = null;

    if (media) {
      await this.deleteMedia(media);
    }

    this.schedulePump();
    return true;
  }

  private schedulePump(): void {
    if (this.pumping) {
      this.rerun = true;
      return;
    }

    void this.pump();
  }

  private async pump(): Promise<void> {
    if (this.pumping) {
      this.rerun = true;
      return;
    }

    this.pumping = true;

    try {
      do {
        this.rerun = false;
        await this.pumpOnce();
      } while (this.rerun);
    } catch (error) {
      console.error('[Playback] Queue processing failed:', error);
    } finally {
      this.pumping = false;

      if (this.rerun) {
        this.schedulePump();
      }
    }
  }

  private async pumpOnce(): Promise<void> {
    const snapshot = this.queue.getSnapshot();
    const next = snapshot.pending[0];

    // Preserve FIFO order. Never skip a request still resolving.
    if (!next || next.status !== 'ready' || !next.track) {
      return;
    }

    let media = this.prepared.get(next.id);

    if (!media) {
      try {
        console.log(`[Playback] Preparing: ${next.track.title}`);

        media = await this.transcoder.prepare(
          next.track,
          next.id as `${string}-${string}-${string}-${string}-${string}`,
        );

        this.registerMedia(media);
        this.prepared.set(next.id, media);

        console.log(`[Playback] Prepared: ${next.track.title}`);
      } catch (error) {
        console.error(
          `[Playback] Failed to prepare "${next.track.title}":`,
          error,
        );

        this.queue.removePending(next.id);
        this.rerun = true;
        return;
      }
    }

    if (!this.queue.getCurrent()) {
      const selected = this.queue.takeNextReady();

      if (selected?.id === next.id) {
        const selectedMedia = this.prepared.get(selected.id);

        if (selectedMedia) {
          this.prepared.delete(selected.id);
          this.currentMedia = selectedMedia;

          console.log(
            `[Playback] Now ready to play: ${selected.track?.title}`,
          );
        }
      }

      this.rerun = true;
    }
  }

  private async deleteMedia(media: PreparedMedia): Promise<void> {
    this.unregisterMedia(media.id);

    await rm(media.path, { force: true }).catch((error: unknown) => {
      console.error(
        `[Playback] Failed to delete media ${media.id}:`,
        error,
      );
    });
  }
}
