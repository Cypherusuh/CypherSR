
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { SpotifyTrackResolver } from './spotify-track-resolver.js';
import type { ResolvedTrack } from './types.js';

const execFileAsync = promisify(execFile);

interface YtDlpResult {
  id?: string;
  title?: string;
  url?: string;
  webpage_url?: string;
  original_url?: string;
  duration?: number;
  thumbnail?: string;
  thumbnails?: Array<{ url?: string }>;
  uploader?: string;
  channel?: string;
  entries?: Array<YtDlpResult | null>;
}

export class TrackResolver {
  constructor(
    private readonly ytDlpPath: string,
    private readonly timeoutMs = 30_000,
    private readonly spotifyResolver?: SpotifyTrackResolver,
  ) {}

  async resolve(query: string): Promise<ResolvedTrack> {
    const input = query.trim();

    if (!input) {
      throw new Error('A URL or search query is required.');
    }

    if (this.spotifyResolver?.isSpotifyTrackUrl(input)) {
      const spotifyTrack = await this.spotifyResolver.resolve(input);

      console.log(
        `[TrackResolver] Spotify metadata resolved: ${spotifyTrack.title}`,
      );

      return this.resolveYouTubeSearch(spotifyTrack.title);
    }

    if (/^https?:\/\//i.test(input)) {
      return this.resolveSingleVideo(input);
    }

    return this.resolveYouTubeSearch(input);
  }

  private async resolveYouTubeSearch(
    query: string,
  ): Promise<ResolvedTrack> {
    const stdout = await this.runYtDlp([
      '--dump-single-json',
      '--skip-download',
      '--flat-playlist',
      '--no-warnings',
      `ytsearch5:${query}`,
    ]);

    let result: YtDlpResult;

    try {
      result = JSON.parse(stdout) as YtDlpResult;
    } catch {
      throw new Error('yt-dlp returned invalid search metadata.');
    }

    const candidates = (result.entries ?? []).filter(
      (entry): entry is YtDlpResult => Boolean(entry?.id),
    );

    if (candidates.length === 0) {
      throw new Error('YouTube search returned no matching videos.');
    }

    const failures: string[] = [];

    for (const candidate of candidates) {
      const candidateUrl =
        candidate.webpage_url ??
        candidate.original_url ??
        `https://www.youtube.com/watch?v=${encodeURIComponent(candidate.id!)}`;

      try {
        return await this.resolveSingleVideo(candidateUrl);
      } catch (error) {
        const message =
          error instanceof Error ? error.message : String(error);

        failures.push(message);

        console.warn(
          `[TrackResolver] Skipping unplayable candidate "${candidate.title ?? candidate.id}": ${message}`,
        );
      }
    }

    const lastFailure = failures.at(-1);

    throw new Error(
      `None of the ${candidates.length} YouTube results could be resolved.` +
        (lastFailure ? ` Last error: ${lastFailure}` : ''),
    );
  }

  private async resolveSingleVideo(
    url: string,
  ): Promise<ResolvedTrack> {
    const stdout = await this.runYtDlp([
      '--dump-single-json',
      '--skip-download',
      '--no-playlist',
      '--no-warnings',
      url,
    ]);

    let track: YtDlpResult;

    try {
      track = JSON.parse(stdout) as YtDlpResult;
    } catch {
      throw new Error('yt-dlp returned invalid track metadata.');
    }

    if (!track.id || !track.title) {
      throw new Error('yt-dlp could not resolve this track.');
    }

    const resolvedUrl =
      track.webpage_url ??
      track.original_url ??
      (track.url?.startsWith('https://') ? track.url : undefined) ??
      url;

    const thumbnail =
      track.thumbnail ??
      track.thumbnails?.find((item) => item.url)?.url ??
      null;

    return {
      id: track.id,
      title: track.title,
      url: resolvedUrl,
      duration:
        typeof track.duration === 'number' ? track.duration : null,
      thumbnail,
      uploader: track.uploader ?? track.channel ?? null,
    };
  }

  private async runYtDlp(args: string[]): Promise<string> {
    try {
      const { stdout } = await execFileAsync(this.ytDlpPath, args, {
        timeout: this.timeoutMs,
        maxBuffer: 10 * 1024 * 1024,
        windowsHide: true,
      });

      return stdout;
    } catch (error) {
      const message =
        error instanceof Error ? error.message : String(error);

      throw new Error(`yt-dlp failed: ${message}`);
    }
  }
}
