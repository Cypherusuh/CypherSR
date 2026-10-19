
import { rm } from 'node:fs/promises';

import type { ChatMessage } from '../commands/types.js';
import type { SongRequestQueue } from '../queue/song-request-queue.js';
import type { QueueSnapshot } from '../queue/types.js';
import type {
  PreparedMedia,
  MediaTranscoder,
} from './media-transcoder.js';

interface PlayerTrack {
  requestId: string;
  title: string;
  uploader: string | null;
  requester: string;
  duration: number | null;
  mediaUrl: string;
}

export interface PlayerState {
  current: PlayerTrack | null;
}

export class PlaybackCoordinator {
  private readonly prepared = new Map<string, PreparedMedia>();
  private currentMedia: PreparedMedia | null = null;

  private pumping = false;
  private rerun = false;

  constructor(
    private readonly queue: SongRequestQueue,
    private readonly transcoder: MediaTranscoder,
    private readonly registerMedia: (media: PreparedMedia) => void,
    private readonly unregisterMedia: (mediaId: string) => void,
  ) {}

  async requestSong(
    message: ChatMessage,
    query: string,
  ): Promise<string> {
    const request = await this.queue.addRequest(message, query);

    this.schedulePump();

    return `Added "${request.track?.title ?? request.query}" to the song queue.`;
  }

  getQueue(): QueueSnapshot {
    return this.queue.getSnapshot();
  }

  getPlayerState(origin: string): PlayerState {
    const current = this.queue.getCurrent();

    if (!current || !current.track || !this.currentMedia) {
      return { current: null };
    }

    return {
      current: {
        requestId: current.id,
        title: current.track.title,
        uploader: current.track.uploader,
        requester: current.requester.displayName,
        duration: current.track.duration,
        mediaUrl: `${origin}/api/media/${this.currentMedia.id}`,
      },
    };
  }

  async trackEnded(requestId: string): Promise<boolean> {
    return this.finishTrack(requestId);
  }

  async skipTrack(): Promise<boolean> {
    const current = this.queue.getCurrent();

    if (!current) {
      return false;
    }

    return this.finishTrack(current.id);
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

    // Preserve FIFO order. Never skip over a request still resolving.
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

    // Only promote a prepared request to the current track.
    if (!this.queue.getCurrent()) {
      const selected = this.queue.takeNextReady();

      if (selected?.id === next.id) {
        const selectedMedia = this.prepared.get(selected.id);

        if (selectedMedia) {
          this.prepared.delete(selected.id);
          this.currentMedia = selectedMedia;

          console.log(`[Playback] Now ready to play: ${selected.track?.title}`);
        }
      }

      // With a current track established, prepare one track ahead.
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
