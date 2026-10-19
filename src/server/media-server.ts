import { createReadStream } from 'node:fs';
import { readFile, stat } from 'node:fs/promises';
import {
  createServer,
  type IncomingMessage,
  type ServerResponse,
} from 'node:http';
import { isAbsolute, resolve } from 'node:path';
import { pipeline } from 'node:stream/promises';

import type { TrackResolver } from '../queue/track-resolver.js';
import type { PreparedMedia, MediaTranscoder } from '../playback/media-transcoder.js';

const MAX_BODY_BYTES = 8 * 1024;

export class MediaServer {
  private readonly server;
  private readonly mediaDir: string;
  private readonly playerPath: string;
  private readonly media = new Map<string, PreparedMedia>();

  constructor(
    private readonly host: string,
    private readonly port: number,
    mediaDir: string,
    private readonly trackResolver: TrackResolver,
    private readonly transcoder: MediaTranscoder,
  ) {
    this.mediaDir = resolve(mediaDir);
    this.playerPath = resolve('public/player-test.html');

    this.server = createServer((request, response) => {
      void this.handleRequest(request, response).catch((error: unknown) => {
        console.error('[MediaServer] Request failed:', error);

        if (!response.headersSent) {
          this.sendError(response, 500, 'Internal server error');
        } else {
          response.destroy();
        }
      });
    });
  }

  async start(): Promise<void> {
    await new Promise<void>((resolveListen, reject) => {
      this.server.once('error', reject);

      this.server.listen(this.port, this.host, () => {
        this.server.off('error', reject);
        resolveListen();
      });
    });

    console.log(
      `[MediaServer] Listening on http://${this.host}:${this.port}`,
    );
  }

  async stop(): Promise<void> {
    if (!this.server.listening) {
      return;
    }

    await new Promise<void>((resolveClose, reject) => {
      this.server.close((error) => {
        if (error) {
          reject(error);
        } else {
          resolveClose();
        }
      });
    });
  }

  register(media: PreparedMedia): void {
    const expectedPath = resolve(this.mediaDir, `${media.id}.mp4`);

    if (!isAbsolute(media.path) || resolve(media.path) !== expectedPath) {
      throw new Error('Media path is outside the configured media directory.');
    }

    this.media.set(media.id, media);
  }

  unregister(mediaId: string): void {
    this.media.delete(mediaId);
  }

  private async handleRequest(
    request: IncomingMessage,
    response: ServerResponse,
  ): Promise<void> {
    const url = new URL(request.url ?? '/', 'http://localhost');

    if (url.pathname === '/' || url.pathname === '/player-test.html') {
      if (request.method !== 'GET' && request.method !== 'HEAD') {
        response.setHeader('Allow', 'GET, HEAD');
        this.sendError(response, 405, 'Method not allowed');
        return;
      }

      const html = await readFile(this.playerPath);

      response.writeHead(200, {
        'Content-Type': 'text/html; charset=utf-8',
        'Content-Length': html.length,
        'Cache-Control': 'no-store',
        'X-Content-Type-Options': 'nosniff',
      });

      response.end(request.method === 'HEAD' ? undefined : html);
      return;
    }

    if (url.pathname === '/api/prepare') {
      if (request.method !== 'POST') {
        response.setHeader('Allow', 'POST');
        this.sendError(response, 405, 'Method not allowed');
        return;
      }

      await this.handlePrepare(request, response);
      return;
    }

    if (url.pathname.startsWith('/api/media/')) {
      await this.handleMediaRequest(request, response, url.pathname);
      return;
    }

    this.sendError(response, 404, 'Not found');
  }

  private async handlePrepare(
    request: IncomingMessage,
    response: ServerResponse,
  ): Promise<void> {
    let body = '';

    for await (const chunk of request) {
      body += chunk.toString();

      if (Buffer.byteLength(body, 'utf8') > MAX_BODY_BYTES) {
        this.sendError(response, 413, 'Request body too large');
        request.destroy();
        return;
      }
    }

    let query: unknown;

    try {
      const parsed: unknown = JSON.parse(body);
      query =
        typeof parsed === 'object' && parsed !== null && 'query' in parsed
          ? parsed.query
          : undefined;
    } catch {
      this.sendError(response, 400, 'Invalid JSON body');
      return;
    }

    if (typeof query !== 'string' || !query.trim()) {
      this.sendError(response, 400, 'A YouTube URL or search query is required');
      return;
    }

    const input = query.trim();

    if (/^https?:\/\//i.test(input)) {
      let parsedUrl: URL;

      try {
        parsedUrl = new URL(input);
      } catch {
        this.sendError(response, 400, 'Invalid URL');
        return;
      }

      const allowedHosts = new Set([
        'youtube.com',
        'www.youtube.com',
        'm.youtube.com',
        'music.youtube.com',
        'youtu.be',
        'youtube-nocookie.com',
        'www.youtube-nocookie.com',
      ]);

      if (
        parsedUrl.protocol !== 'https:' ||
        !allowedHosts.has(parsedUrl.hostname.toLowerCase()) ||
        parsedUrl.username ||
        parsedUrl.password
      ) {
        this.sendError(response, 400, 'Only YouTube URLs are supported');
        return;
      }
    }

    try {
      console.log(`[MediaServer] Resolving: ${input}`);

      const track = await this.trackResolver.resolve(input);
      const media = await this.transcoder.prepare(track);

      this.register(media);

      const baseUrl = `http://${this.host}:${this.port}`;

      this.sendJson(response, 200, {
        id: media.id,
        title: track.title,
        uploader: track.uploader,
        duration: media.duration,
        mediaUrl: `${baseUrl}/api/media/${media.id}`,
      });
    } catch (error) {
      console.error('[MediaServer] Prepare failed:', error);

      this.sendJson(response, 422, {
        error:
          error instanceof Error
            ? error.message
            : 'Unable to prepare this track',
      });
    }
  }

  private async handleMediaRequest(
    request: IncomingMessage,
    response: ServerResponse,
    pathname: string,
  ): Promise<void> {
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      response.setHeader('Allow', 'GET, HEAD');
      this.sendError(response, 405, 'Method not allowed');
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
      this.sendError(response, 404, 'Media file not found');
      return;
    }

    const size = file.size;
    const range = request.headers.range;

    response.setHeader('Content-Type', media.contentType);
    response.setHeader('Accept-Ranges', 'bytes');
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('X-Content-Type-Options', 'nosniff');

    if (!range) {
      response.writeHead(200, { 'Content-Length': size });

      if (request.method === 'HEAD') {
        response.end();
        return;
      }

      await pipeline(createReadStream(media.path), response);
      return;
    }

    const parsed = this.parseRange(range, size);

    if (!parsed) {
      response.setHeader('Content-Range', `bytes */${size}`);
      this.sendError(response, 416, 'Range not satisfiable');
      return;
    }

    const { start, end } = parsed;

    response.writeHead(206, {
      'Content-Length': end - start + 1,
      'Content-Range': `bytes ${start}-${end}/${size}`,
    });

    if (request.method === 'HEAD') {
      response.end();
      return;
    }

    await pipeline(createReadStream(media.path, { start, end }), response);
  }

  private parseRange(
    header: string,
    size: number,
  ): { start: number; end: number } | null {
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

      if (!Number.isSafeInteger(suffixLength) || suffixLength <= 0) {
        return null;
      }

      start = Math.max(0, size - suffixLength);
      end = size - 1;
    } else {
      start = Number(startText);
      end = endText ? Number(endText) : size - 1;

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

  private sendJson(
    response: ServerResponse,
    status: number,
    value: unknown,
  ): void {
    const body = JSON.stringify(value);

    response.writeHead(status, {
      'Content-Type': 'application/json; charset=utf-8',
      'Content-Length': Buffer.byteLength(body),
      'Cache-Control': 'no-store',
    });

    response.end(body);
  }

  private sendError(
    response: ServerResponse,
    status: number,
    message: string,
  ): void {
    this.sendJson(response, status, { error: message });
  }
}