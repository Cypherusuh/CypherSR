import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import type { ResolvedTrack } from '../queue/types.js';
import type { MediaMode, ResolvedMedia } from './media-types.js';

const execFileAsync = promisify(execFile);

interface YtDlpFormat {
  url?: string;
  ext?: string;
  vcodec?: string;
  acodec?: string;
  height?: number;
  protocol?: string;
}

interface YtDlpResult {
  duration?: number;
  formats?: YtDlpFormat[];
}

export class MediaResolver {
  constructor(
    private readonly ytDlpPath: string,
    private readonly timeoutMs = 30_000,
  ) {}

  async resolve(
    track: ResolvedTrack,
    mode: MediaMode,
  ): Promise<ResolvedMedia> {
    const { stdout } = await execFileAsync(
      this.ytDlpPath,
      [
        '--dump-single-json',
        '--skip-download',
        '--no-playlist',
        '--no-warnings',
        '--no-call-home',
        '--js-runtimes',
        'node',
        '--remote-components',
        'ejs:github',
        '--format',
        this.getFormatSelector(mode),
        '--',
        track.url,
      ],
      {
        timeout: this.timeoutMs,
        maxBuffer: 20 * 1024 * 1024,
        windowsHide: true,
      },
    );

    let result: YtDlpResult;

    try {
      result = JSON.parse(stdout) as YtDlpResult;
    } catch {
      throw new Error('yt-dlp returned invalid media information.');
    }

    const formats = result.formats ?? [];

    const selected = formats.find(
      (format) => format.url && this.isCompatible(format, mode),
    );

    if (!selected?.url) {
      throw new Error(
        `yt-dlp did not return a compatible ${mode}-playback URL.`,
      );
    }

    return {
      url: selected.url,
      mode,
      contentType: this.getContentType(selected.ext),
      duration:
        typeof result.duration === 'number'
          ? result.duration
          : track.duration,
    };
  }

  private getFormatSelector(mode: MediaMode): string {
    if (mode === 'audio') {
      return 'bestaudio[protocol^=https]/bestaudio';
    }

    return (
      'best[height>=240][acodec!=none][vcodec!=none][protocol^=https]/' +
      'best[acodec!=none][vcodec!=none][protocol^=https]'
    );
  }

  private isCompatible(
    format: YtDlpFormat,
    mode: MediaMode,
  ): boolean {
    if (!format.url || !format.url.startsWith('https://')) {
      return false;
    }

    if (mode === 'audio') {
      return format.acodec !== 'none' && format.acodec !== undefined;
    }

    return (
      format.vcodec !== 'none' &&
      format.vcodec !== undefined &&
      format.acodec !== 'none' &&
      format.acodec !== undefined
    );
  }

  private getContentType(ext?: string): string | null {
    switch (ext) {
      case 'mp4':
        return 'video/mp4';
      case 'webm':
        return 'video/webm';
      case 'm4a':
        return 'audio/mp4';
      case 'mp3':
        return 'audio/mpeg';
      case 'opus':
        return 'audio/ogg; codecs=opus';
      case 'ogg':
        return 'audio/ogg';
      default:
        return null;
    }
  }
}