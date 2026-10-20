
import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { mkdir, mkdtemp, rename, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { promisify } from 'node:util';

import type { ResolvedTrack } from '../queue/types.js';

const execFileAsync = promisify(execFile);

export interface PreparedMedia {
  id: string;
  path: string;
  contentType: 'video/mp4';
  duration: number | null;
}

export class MediaTranscoder {
  constructor(
    private readonly options: {
      ytDlpPath: string;
      ffmpegPath: string;
      mediaDir: string;
      maxVideoHeight: number;
      timeoutMs?: number;
    },
  ) {}

  async prepare(
    track: ResolvedTrack,
    mediaId = randomUUID(),
  ): Promise<PreparedMedia> {
    const totalStartedAt = performance.now();

    if (!/^[a-zA-Z0-9_-]+$/.test(mediaId)) {
      throw new Error('Invalid media ID.');
    }

    const {
      ytDlpPath,
      ffmpegPath,
      mediaDir,
      maxVideoHeight,
      timeoutMs = 15 * 60 * 1000,
    } = this.options;

    const absoluteMediaDir = resolve(mediaDir);
    const absoluteFfmpegPath = resolve(ffmpegPath);
    const absoluteFfmpegDir = resolve(absoluteFfmpegPath, '..');

    await mkdir(absoluteMediaDir, { recursive: true });

    const destination = join(absoluteMediaDir, `${mediaId}.mp4`);
    const workDir = await mkdtemp(join(absoluteMediaDir, '.work-'));

    const sourcePath = join(workDir, 'source.mkv');
    const temporaryOutput = join(workDir, 'output.mp4');

    try {
      const preferredFormat =
        `bestvideo[height<=${maxVideoHeight}][vcodec^=avc1]+` +
        'bestaudio[acodec^=mp4a]/' +
        `best[height<=${maxVideoHeight}]` +
        '[vcodec^=avc1][acodec^=mp4a]';

      const fallbackFormat =
        `bestvideo[height<=${maxVideoHeight}]+bestaudio/` +
        `best[height<=${maxVideoHeight}]`;

      const downloadStartedAt = performance.now();
      let compatibleSource = true;

      try {
        await this.downloadSource({
          ytDlpPath,
          ffmpegDir: absoluteFfmpegDir,
          format: preferredFormat,
          trackUrl: track.url,
          outputPath: join(workDir, 'source.%(ext)s'),
          timeoutMs,
        });
      } catch {
        compatibleSource = false;

        console.warn(
          `[MediaTranscoder] H.264/AAC formats unavailable for ` +
            `"${track.title}". Retrying with fallback formats.`,
        );

        await rm(sourcePath, { force: true });

        await this.downloadSource({
          ytDlpPath,
          ffmpegDir: absoluteFfmpegDir,
          format: fallbackFormat,
          trackUrl: track.url,
          outputPath: join(workDir, 'source.%(ext)s'),
          timeoutMs,
        });
      }

      this.logTiming('yt-dlp download and remux', downloadStartedAt);

      let copiedWithoutEncoding = false;

      if (compatibleSource) {
        const copyStartedAt = performance.now();

        try {
          await execFileAsync(
            absoluteFfmpegPath,
            [
              '-hide_banner',
              '-loglevel',
              'error',
              '-y',
              '-i',
              sourcePath,
              '-map',
              '0:v:0',
              '-map',
              '0:a:0',
              '-c',
              'copy',
              '-movflags',
              '+faststart',
              temporaryOutput,
            ],
            {
              timeout: timeoutMs,
              maxBuffer: 20 * 1024 * 1024,
              windowsHide: true,
            },
          );

          copiedWithoutEncoding = true;
          this.logTiming('FFmpeg stream copy', copyStartedAt);
        } catch (error) {
          const message =
            error instanceof Error ? error.message : String(error);

          console.warn(
            `[MediaTranscoder] Stream copy failed for ` +
              `"${track.title}": ${message}. Falling back to encoding.`,
          );

          await rm(temporaryOutput, { force: true });
        }
      }

      if (!copiedWithoutEncoding) {
        const encodeStartedAt = performance.now();

        await execFileAsync(
          absoluteFfmpegPath,
          [
            '-hide_banner',
            '-loglevel',
            'error',
            '-y',
            '-i',
            sourcePath,
            '-map',
            '0:v:0',
            '-map',
            '0:a:0',
            '-vf',
            `scale=-2:min(ih\\,${maxVideoHeight})`,
            '-c:v',
            'libx264',
            '-preset',
            'ultrafast',
            '-crf',
            '28',
            '-pix_fmt',
            'yuv420p',
            '-c:a',
            'aac',
            '-b:a',
            '128k',
            '-movflags',
            '+faststart',
            temporaryOutput,
          ],
          {
            timeout: timeoutMs,
            maxBuffer: 20 * 1024 * 1024,
            windowsHide: true,
          },
        );

        this.logTiming('FFmpeg encoding fallback', encodeStartedAt);
      }

      const publishStartedAt = performance.now();

      await rename(temporaryOutput, destination);

      this.logTiming('Publish prepared media', publishStartedAt);

      return {
        id: mediaId,
        path: destination,
        contentType: 'video/mp4',
        duration: track.duration,
      };
    } catch (error) {
      const message =
        error instanceof Error ? error.message : String(error);

      console.error(
        `[MediaTranscoder] Preparation failed for "${track.title}": ${message}`,
      );

      await rm(destination, { force: true }).catch(() => undefined);
      throw error;
    } finally {
      await rm(workDir, {
        recursive: true,
        force: true,
      }).catch(() => undefined);

      this.logTiming('Total media preparation', totalStartedAt);
    }
  }

  private async downloadSource(options: {
    ytDlpPath: string;
    ffmpegDir: string;
    format: string;
    trackUrl: string;
    outputPath: string;
    timeoutMs: number;
  }): Promise<void> {
    const {
      ytDlpPath,
      ffmpegDir,
      format,
      trackUrl,
      outputPath,
      timeoutMs,
    } = options;

    await execFileAsync(
      ytDlpPath,
      [
        '--no-playlist',
        '--no-warnings',
        '--js-runtimes',
        'node',
        '--remote-components',
        'ejs:github',
        '--ffmpeg-location',
        ffmpegDir,
        '--format',
        format,
        '--remux-video',
        'mkv',
        '--output',
        outputPath,
        '--',
        trackUrl,
      ],
      {
        timeout: timeoutMs,
        maxBuffer: 20 * 1024 * 1024,
        windowsHide: true,
      },
    );
  }

  private logTiming(label: string, startedAt: number): void {
    const elapsedMs = Math.round(performance.now() - startedAt);
    console.log(`[Timing] ${label}: ${elapsedMs} ms`);
  }
}
