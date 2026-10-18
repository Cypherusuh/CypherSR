export type SongRequestStatus = 'resolving' | 'ready' | 'failed';

export interface ResolvedTrack {
  id: string;
  title: string;
  url: string;
  duration: number | null;
  thumbnail: string | null;
  uploader: string | null;
}

export interface SongRequest {
  id: string;
  query: string;
  requester: {
    id: string;
    username: string;
    displayName: string;
  };
  requestedAt: string;
  status: SongRequestStatus;
  track?: ResolvedTrack;
  error?: string;
}

export interface QueueSnapshot {
  current: SongRequest | null;
  pending: SongRequest[];
  limit: number;
}