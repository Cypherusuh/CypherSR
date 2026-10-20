
import { randomUUID } from 'node:crypto';

import type { ChatMessage } from '../commands/types.js';
import type {
  QueueSnapshot,
  SongRequest,
} from './types.js';
import { TrackResolver } from './track-resolver.js';

export class SongRequestLimitError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SongRequestLimitError';
  }
}

export class SongRequestQueue {
  private current: SongRequest | null = null;
  private readonly pending: SongRequest[] = [];

  private readonly perUserLimit = 2;

  constructor(
    private readonly resolver: TrackResolver,
    private readonly limit: number,
  ) {
    if (!Number.isSafeInteger(limit) || limit < 1) {
      throw new Error('Queue limit must be a positive integer.');
    }
  }

  reserveRequest(message: ChatMessage, query: string): SongRequest {
    const normalizedQuery = query.trim();

    if (!normalizedQuery) {
      throw new Error('Provide a URL or search query.');
    }

    const userPendingCount = this.pending.filter(
      (request) => request.requester.id === message.userId,
    ).length;

    if (userPendingCount >= this.perUserLimit) {
      throw new SongRequestLimitError(
        `You can only have ${this.perUserLimit} pending song requests at a time.`,
      );
    }

    if (this.pending.length >= this.limit) {
      throw new SongRequestLimitError(
        'The song request queue is full. Please try again later.',
      );
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

    // Reserve capacity before asynchronous track resolution.
    this.pending.push(request);

    return request;
  }

  async resolveRequest(requestId: string): Promise<SongRequest> {
    const request = this.pending.find(
      (item) => item.id === requestId,
    );

    if (!request) {
      throw new Error('Song request is no longer in the queue.');
    }

    if (request.status !== 'resolving') {
      throw new Error('Song request is not awaiting resolution.');
    }

    try {
      const track = await this.resolver.resolve(request.query);

      if (!this.pending.some((item) => item.id === requestId)) {
        throw new Error('Song request was removed during resolution.');
      }

      request.track = track;
      request.status = 'ready';

      return request;
    } catch (error) {
      const index = this.pending.findIndex(
        (item) => item.id === requestId,
      );

      if (index !== -1) {
        this.pending.splice(index, 1);
      }

      request.status = 'failed';
      request.error =
        error instanceof Error
          ? error.message
          : 'Unknown resolution error';

      throw error;
    }
  }

  async addRequest(
    message: ChatMessage,
    query: string,
  ): Promise<SongRequest> {
    const request = this.reserveRequest(message, query);
    return this.resolveRequest(request.id);
  }

  removePending(id: string): boolean {
    const index = this.pending.findIndex(
      (request) => request.id === id,
    );

    if (index === -1) {
      return false;
    }

    this.pending.splice(index, 1);
    return true;
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
