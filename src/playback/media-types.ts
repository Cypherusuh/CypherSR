export type MediaMode = 'video' | 'audio';

export interface ResolvedMedia {
  url: string;
  mode: MediaMode;
  contentType: string | null;
  duration: number | null;
}