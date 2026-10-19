
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

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
  entries?: YtDlpResult[];
}

export class TrackResolver {
  constructor(
    private readonly ytDlpPath: string,
    private readonly timeoutMs = 30_000,
  ) {}

  async resolve(query: string): Promise<ResolvedTrack> {
    const input = query.trim();

    if (!input) {
      throw new Error('A URL or search query is required.');
    }

    const isUrl = /^https?:\/\//i.test(input);
    const target = isUrl ? input : `ytsearch1:${input}`;

    const { stdout } = await execFileAsync(
      this.ytDlpPath,
      [
        '--dump-single-json',
        '--skip-download',
        '--no-playlist',
        '--no-warnings',
        '--no-call-home',
        target,
      ],
      {
        timeout: this.timeoutMs,
        maxBuffer: 10 * 1024 * 1024,
        windowsHide: true,
      },
    );

    let result: YtDlpResult;

    try {
      result = JSON.parse(stdout) as YtDlpResult;
    } catch {
      throw new Error('yt-dlp returned invalid track metadata.');
    }

    // Search results contain a playlist wrapper. Resolve the actual video
    // instead of mistakenly using the search query as the track metadata.
    const track = isUrl ? result : result.entries?.[0];

    if (!track) {
      throw new Error('YouTube search returned no matching videos.');
    }

    const url =
      track.webpage_url ??
      track.original_url ??
      (track.url?.startsWith('https://') ? track.url : undefined);

    if (!track.id || !track.title || !url) {
      throw new Error('yt-dlp could not resolve this track.');
    }

    const thumbnail =
      track.thumbnail ??
      track.thumbnails?.find((item) => item.url)?.url ??
      null;

    return {
      id: track.id,
      title: track.title,
      url,
      duration:
        typeof track.duration === 'number' ? track.duration : null,
      thumbnail,
      uploader: track.uploader ?? track.channel ?? null,
    };
  }
}
