
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import type { ResolvedTrack } from './types.js';

const execFileAsync = promisify(execFile);

interface YtDlpResult {
  id?: string;
  title?: string;
  webpage_url?: string;
  original_url?: string;
  duration?: number;
  thumbnail?: string;
  uploader?: string;
  channel?: string;
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

    const url = result.webpage_url ?? result.original_url;

    if (!result.id || !result.title || !url) {
      throw new Error('yt-dlp could not resolve this track.');
    }

    return {
      id: result.id,
      title: result.title,
      url,
      duration:
        typeof result.duration === 'number' ? result.duration : null,
      thumbnail: result.thumbnail ?? null,
      uploader: result.uploader ?? result.channel ?? null,
    };
  }
}
