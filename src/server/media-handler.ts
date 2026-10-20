import { createReadStream } from 'node:fs';
import { resolve } from 'node:path';
import { stat } from 'node:fs/promises';
import { pipeline } from 'node:stream/promises';
import { IncomingMessage, ServerResponse } from 'node:http';
import { PreparedMedia } from '../playback/media-transcoder.js';
import { ByteRange, isPrematureClose } from './media-server.types.js';

export class MediaHandler {
    private readonly media = new Map<string, PreparedMedia>();

    constructor(
        private readonly mediaDir: string,
    ) { }

    register(media: PreparedMedia): void {
        const expectedPath = resolve(this.mediaDir, `${media.id}.mp4`);

        if (resolve(media.path) !== expectedPath) {
            throw new Error(
                'Media path is outside the configured media directory.',
            );
        }

        this.media.set(media.id, media);
    }

    unregister(mediaId: string): void {
        this.media.delete(mediaId);
    }

    async handle(
        request: IncomingMessage,
        response: ServerResponse,
        pathname: string,
    ): Promise<void> {
        if (
            request.method !== 'GET' &&
            request.method !== 'HEAD'
        ) {
            response.setHeader('Allow', 'GET, HEAD');
            this.sendError(
                response,
                405,
                'Method not allowed',
            );
            return;
        }
        
        const match = /^\/api\/media\/([a-f0-9-]{36})$/.exec(pathname);

        if (!match) {
            this.sendError(response, 404, 'Not found');
            return;
        }

        const media = this.media.get(match[1]);

        if (!media) {
            this.sendError(response, 404, 'Media not found');
            return;
        }

        const file = await stat(media.path).catch(() => null);

        if (!file?.isFile()) {
            this.sendError(
                response,
                404,
                'Media file not found',
            );
            return;
        }

        const size = file.size;
        const rangeHeader = request.headers.range;

        response.setHeader(
            'Content-Type',
            media.contentType,
        );
        response.setHeader('Accept-Ranges', 'bytes');
        response.setHeader('Cache-Control', 'no-store');
        response.setHeader(
            'X-Content-Type-Options',
            'nosniff',
        );

        if (!rangeHeader) {
            response.writeHead(200, {
                'Content-Length': size,
            });

            if (request.method === 'HEAD') {
                response.end();
                return;
            }

            await this.stream(
                media.path,
                response,
                request,
            );

            return;
        }

        const range = this.parseRange(
            rangeHeader,
            size,
        );

        if (!range) {
            response.setHeader(
                'Content-Range',
                `bytes */${size}`,
            );

            this.sendError(
                response,
                416,
                'Range not satisfiable',
            );

            return;
        }

        const { start, end } = range;

        response.writeHead(206, {
            'Content-Length': end - start + 1,
            'Content-Range':
                `bytes ${start}-${end}/${size}`,
        });

        if (request.method === 'HEAD') {
            response.end();
            return;
        }

        await this.stream(
            media.path,
            response,
            request,
            range,
        );
    }

    private async stream(
        filePath: string,
        response: ServerResponse,
        request: IncomingMessage,
        range?: ByteRange,
    ): Promise<void> {
        const fileStream = range
            ? createReadStream(filePath, range)
            : createReadStream(filePath);

        try {
            await pipeline(
                fileStream,
                response,
            );
        } catch (error) {
            if (
                isPrematureClose(error) &&
                (
                    request.aborted ||
                    response.destroyed ||
                    !response.writable
                )
            ) {
                return;
            }

            throw error;
        }
    }

    private parseRange(
        header: string,
        size: number,
    ): ByteRange | null {
       
        const match = /^bytes=(\d*)-(\d*)$/.exec(header.trim());

        if (!match || size === 0) {
            return null;
        }

        const [, startText, endText] = match;

        if (!startText && !endText) {
            return null;
        }

        let start: number;
        let end: number;

        if (!startText) {
            const suffixLength = Number(endText);

            if (
                !Number.isSafeInteger(suffixLength) ||
                suffixLength <= 0
            ) {
                return null;
            }

            start = Math.max(
                0,
                size - suffixLength,
            );
            end = size - 1;
        } else {
            start = Number(startText);
            end = endText
                ? Number(endText)
                : size - 1;

            if (
                !Number.isSafeInteger(start) ||
                !Number.isSafeInteger(end) ||
                start < 0 ||
                start >= size ||
                end < start
            ) {
                return null;
            }

            end = Math.min(end, size - 1);
        }

        return { start, end };
    }

    private sendError(
        response: ServerResponse,
        status: number,
        message: string,
    ): void {
        if (
            response.destroyed ||
            response.writableEnded
        ) {
            return;
        }

        const body = JSON.stringify({
            error: message,
        });

        response.writeHead(status, {
            'Content-Type':
                'application/json; charset=utf-8',
            'Content-Length':
                Buffer.byteLength(body),
            'Cache-Control': 'no-store',
        });

        response.end(body);
    }
}