import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
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
    ) { }

    async prepare(
        track: ResolvedTrack,
        mediaId = randomUUID(),
    ): Promise<PreparedMedia> {
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
                    absoluteFfmpegDir,
                    '--format',
                    `bestvideo[height<=${maxVideoHeight}]+bestaudio/best[height<=${maxVideoHeight}]`,
                    '--remux-video',
                    'mkv',
                    '--output',
                    join(workDir, 'source.%(ext)s'),
                    '--',
                    track.url,
                ],
                {
                    timeout: timeoutMs,
                    maxBuffer: 20 * 1024 * 1024,
                    windowsHide: true,
                },
            );


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
                    'scale=-2:min(ih\\,' + maxVideoHeight + ')',
                    '-c:v',
                    'libx264',
                    '-preset',
                    'veryfast',
                    '-crf',
                    '23',
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

            await rename(temporaryOutput, destination);

            return {
                id: mediaId,
                path: destination,
                contentType: 'video/mp4',
                duration: track.duration,
            };
        } catch (error) {
            await rm(destination, { force: true }).catch(() => undefined);
            throw error;
        } finally {
            await rm(workDir, {
                recursive: true,
                force: true,
            }).catch(() => undefined);
        }
    }
}