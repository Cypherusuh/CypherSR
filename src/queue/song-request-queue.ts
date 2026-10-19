
import { randomUUID } from 'node:crypto';

import type { ChatMessage } from '../commands/types.js';
import type {
  QueueSnapshot,
  ResolvedTrack,
  SongRequest,
} from './types.js';
import { TrackResolver } from './track-resolver.js';

export class SongRequestQueue {
  removePending(id: string) {
      throw new Error('Method not implemented.');
  }
  private current: SongRequest | null = null;
  private readonly pending: SongRequest[] = [];

  constructor(
    private readonly resolver: TrackResolver,
    private readonly limit: number,
  ) {
    if (!Number.isSafeInteger(limit) || limit < 1) {
      throw new Error('Queue limit must be a positive integer.');
    }
  }

  async addRequest(
    message: ChatMessage,
    query: string,
  ): Promise<SongRequest> {
    const normalizedQuery = query.trim();

    if (!normalizedQuery) {
      throw new Error('Provide a URL or search query.');
    }

    if (this.pending.length >= this.limit) {
      throw new Error('The song request queue is full.');
    }

    const request: SongRequest = {
      id: randomUUID(),
      query: normalizedQuery,
      requester: {
        id: message.userId,
        username: message.username,
        displayName: message.displayName,
      },
      requestedAt: new Date().toISOString(),
      status: 'resolving',
    };

    // Reserve capacity before starting the asynchronous resolution.
    this.pending.push(request);

    try {
      const track = await this.resolver.resolve(normalizedQuery);

      request.track = track;
      request.status = 'ready';

      return request;
    } catch (error) {
      const index = this.pending.findIndex(
        (item) => item.id === request.id,
      );

      if (index !== -1) {
        this.pending.splice(index, 1);
      }

      request.status = 'failed';
      request.error =
        error instanceof Error ? error.message : 'Unknown resolution error';

      throw error;
    }
  }

  getSnapshot(): QueueSnapshot {
    return {
      current: this.current,
      pending: [...this.pending],
      limit: this.limit,
    };
  }

  takeNextReady(): SongRequest | null {
    if (this.current || this.pending[0]?.status !== 'ready') {
      return null;
    }

    const request = this.pending.shift();

    if (!request?.track) {
      return null;
    }

    this.current = request;
    return request;
  }

  completeCurrent(requestId: string): boolean {
    if (this.current?.id !== requestId) {
      return false;
    }

    this.current = null;
    return true;
  }

  getCurrent(): SongRequest | null {
    return this.current;
  }

  clearPending(): void {
    this.pending.length = 0;
  }
}
