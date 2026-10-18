import type { ResolvedTrack } from '../queue/types.js';

export interface PlaybackEngine {
  playTrack(
    track: ResolvedTrack,
    requestId: string,
  ): Promise<void>;

  stopTrack(): Promise<void>;

  startRadio(): Promise<void>;

  onTrackEnded(
    listener: (requestId: string) => void,
  ): () => void;
}