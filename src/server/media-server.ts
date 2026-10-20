
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
import type {
    PreparedMedia,
    MediaTranscoder,
} from '../playback/media-transcoder.js';
import type { PlaybackCoordinator } from '../playback/playback-coordinator.js';

const MAX_BODY_BYTES = 8 * 1024;

type PlayerControlAction = 'play' | 'pause' | 'seek' | 'volume';

interface PlayerControl {
    revision: number;
    action: PlayerControlAction;
    value?: number;
}

interface PlaybackRuntime {
    requestId: string | null;
    position: number;
    duration: number;
    volume: number;
    paused: boolean;
    updatedAt: number;
}

export class MediaServer {
    private readonly server;
    private readonly mediaDir: string;
    private readonly playerPath: string;
    private readonly testPlayerPath: string;
    private readonly queuePath: string;
    private readonly playedNowPath: string;
    private readonly media = new Map<string, PreparedMedia>();
    private playbackCoordinator: PlaybackCoordinator | null = null;

    /**
     * This state is updated by the native Rust/mpv player.
     * The browser player is a follower and must not write authoritative
     * playback telemetry or advance the queue.
     */
    private playbackRuntime: PlaybackRuntime = {
        requestId: null,
        position: 0,
        duration: 0,
        volume: 1,
        paused: true,
        updatedAt: Date.now(),
    };

    private controlRevision = 0;
    private pendingControl: PlayerControl | null = null;

    constructor(
        private readonly host: string,
        private readonly port: number,
        mediaDir: string,
        private readonly trackResolver: TrackResolver,
        private readonly transcoder: MediaTranscoder,
    ) {
        this.mediaDir = resolve(mediaDir);
        this.playerPath = resolve('public/player.html');
        this.testPlayerPath = resolve('public/player-test.html');
        this.queuePath = resolve('public/queue.html');
        this.playedNowPath = resolve('public/played-now.html');

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

    attachPlaybackCoordinator(coordinator: PlaybackCoordinator): void {
        this.playbackCoordinator = coordinator;
    }

    register(media: PreparedMedia): void {
        const expectedPath = resolve(this.mediaDir, `${media.id}.mp4`);

        if (!isAbsolute(media.path) || resolve(media.path) !== expectedPath) {
            throw new Error(
                'Media path is outside the configured media directory.',
            );
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

        if (url.pathname === '/') {
            await this.handleHtmlPage(request, response, this.testPlayerPath);
            return;
        }

        if (
            url.pathname === '/player' ||
            url.pathname === '/player.html'
        ) {
            await this.handleHtmlPage(request, response, this.playerPath);
            return;
        }

        if (url.pathname === '/player-test.html') {
            await this.handleHtmlPage(request, response, this.testPlayerPath);
            return;
        }

        if (url.pathname === '/played-now.html') {
            await this.handleHtmlPage(request, response, this.playedNowPath);
            return;
        }

        if (url.pathname === '/queue.html') {
            await this.handleHtmlPage(request, response, this.queuePath);
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

        if (url.pathname === '/api/queue') {
            if (request.method !== 'GET') {
                response.setHeader('Allow', 'GET');
                this.sendError(response, 405, 'Method not allowed');
                return;
            }

            if (!this.playbackCoordinator) {
                this.sendError(response, 503, 'Playback is not initialized');
                return;
            }

            this.sendJson(response, 200, this.playbackCoordinator.getQueue());
            return;
        }

        if (url.pathname === '/api/player/state') {
            if (request.method !== 'GET') {
                response.setHeader('Allow', 'GET');
                this.sendError(response, 405, 'Method not allowed');
                return;
            }

            if (!this.playbackCoordinator) {
                this.sendError(response, 503, 'Playback is not initialized');
                return;
            }

            const playerState = this.playbackCoordinator.getPlayerState('');
            const current = playerState.current;
            const runtime = this.playbackRuntime;

            /*
             * Track changes can be observed here before the next mpv
             * telemetry report arrives. Initialize a reasonable state for
             * the new track; subsequent state updates come from Rust/mpv.
             */
            if (current && runtime.requestId !== current.requestId) {
                this.playbackRuntime = {
                    requestId: current.requestId,
                    position: 0,
                    duration: current.duration ?? 0,
                    volume: runtime.volume,
                    paused: false,
                    updatedAt: Date.now(),
                };
            } else if (!current && runtime.requestId !== null) {
                this.playbackRuntime = {
                    requestId: null,
                    position: 0,
                    duration: 0,
                    volume: runtime.volume,
                    paused: true,
                    updatedAt: Date.now(),
                };
            }

            this.sendJson(response, 200, {
                ...playerState,
                playback: this.playbackRuntime,
                control: this.pendingControl,
            });
            return;
        }

        if (url.pathname === '/api/player/skip') {
            if (request.method !== 'POST') {
                response.setHeader('Allow', 'POST');
                this.sendError(response, 405, 'Method not allowed');
                return;
            }

            if (!this.playbackCoordinator) {
                this.sendError(response, 503, 'Playback is not initialized');
                return;
            }

            const skipped = await this.playbackCoordinator.skipTrack();

            this.sendJson(response, 200, {
                skipped,
                message: skipped
                    ? 'Skipping the current track.'
                    : 'There is no track to skip.',
            });

            return;
        }

        if (url.pathname === '/api/player/ended') {
            if (request.method !== 'POST') {
                response.setHeader('Allow', 'POST');
                this.sendError(response, 405, 'Method not allowed');
                return;
            }

            /*
             * This endpoint is now intended for the Rust/mpv controller.
             * The OBS browser player must not call it.
             */
            await this.handlePlayerEnded(request, response);
            return;
        }

        if (url.pathname === '/api/player/telemetry') {
            if (request.method !== 'POST') {
                response.setHeader('Allow', 'POST');
                this.sendError(response, 405, 'Method not allowed');
                return;
            }

            /*
             * Rust/mpv reports its actual playback state here.
             * player.html deliberately does not send telemetry.
             */
            await this.handlePlayerTelemetry(request, response);
            return;
        }

        if (url.pathname === '/api/player/control') {
            if (request.method !== 'POST') {
                response.setHeader('Allow', 'POST');
                this.sendError(response, 405, 'Method not allowed');
                return;
            }

            await this.handlePlayerControl(request, response);
            return;
        }

        if (url.pathname.startsWith('/api/media/')) {
            await this.handleMediaRequest(request, response, url.pathname);
            return;
        }

        this.sendError(response, 404, 'Not found');
    }

    private async handleHtmlPage(
        request: IncomingMessage,
        response: ServerResponse,
        filePath: string,
    ): Promise<void> {
        if (request.method !== 'GET' && request.method !== 'HEAD') {
            response.setHeader('Allow', 'GET, HEAD');
            this.sendError(response, 405, 'Method not allowed');
            return;
        }

        const html = await readFile(filePath);

        response.writeHead(200, {
            'Content-Type': 'text/html; charset=utf-8',
            'Content-Length': html.length,
            'Cache-Control': 'no-store',
            'X-Content-Type-Options': 'nosniff',
        });

        response.end(request.method === 'HEAD' ? undefined : html);
    }

    private async readJsonBody(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<Record<string, unknown> | null> {
        let body = '';

        for await (const chunk of request) {
            body += chunk.toString();

            if (Buffer.byteLength(body, 'utf8') > MAX_BODY_BYTES) {
                this.sendError(response, 413, 'Request body too large');
                request.destroy();
                return null;
            }
        }

        try {
            const payload: unknown = JSON.parse(body);

            if (
                typeof payload !== 'object' ||
                payload === null ||
                Array.isArray(payload)
            ) {
                this.sendError(response, 400, 'Expected a JSON object');
                return null;
            }

            return payload as Record<string, unknown>;
        } catch {
            this.sendError(response, 400, 'Invalid JSON body');
            return null;
        }
    }

    private async handlePlayerEnded(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        if (!this.playbackCoordinator) {
            this.sendError(response, 503, 'Playback is not initialized');
            return;
        }

        const payload = await this.readJsonBody(request, response);
        if (!payload) return;

        if (typeof payload.requestId !== 'string') {
            this.sendError(response, 400, 'A requestId is required');
            return;
        }

        const requestId = payload.requestId;
        const current = this.playbackCoordinator.getPlayerState('').current;

        /*
         * Reject stale end notifications. For example, mpv may finish
         * reporting an old track after the user has already skipped it.
         */
        if (!current || current.requestId !== requestId) {
            this.sendJson(response, 200, { accepted: false });
            return;
        }

        const accepted = await this.playbackCoordinator.trackEnded(requestId);

        if (accepted && this.playbackRuntime.requestId === requestId) {
            this.playbackRuntime = {
                ...this.playbackRuntime,
                requestId: null,
                position: 0,
                duration: 0,
                paused: true,
                updatedAt: Date.now(),
            };
        }

        this.sendJson(response, 200, { accepted });
    }

    private async handlePlayerTelemetry(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        if (!this.playbackCoordinator) {
            this.sendError(response, 503, 'Playback is not initialized');
            return;
        }

        const payload = await this.readJsonBody(request, response);
        if (!payload) return;

        const { requestId, position, duration, volume, paused } = payload;

        if (
            typeof requestId !== 'string' ||
            typeof position !== 'number' ||
            !Number.isFinite(position) ||
            position < 0 ||
            typeof duration !== 'number' ||
            !Number.isFinite(duration) ||
            duration < 0 ||
            typeof volume !== 'number' ||
            !Number.isFinite(volume) ||
            volume < 0 ||
            volume > 1 ||
            typeof paused !== 'boolean'
        ) {
            this.sendError(response, 400, 'Invalid playback telemetry');
            return;
        }

        const current = this.playbackCoordinator.getPlayerState('').current;

        if (!current || current.requestId !== requestId) {
            this.sendJson(response, 409, { accepted: false });
            return;
        }

        this.playbackRuntime = {
            requestId,
            position: duration > 0
                ? Math.min(position, duration)
                : position,
            duration,
            volume,
            paused,
            updatedAt: Date.now(),
        };

        this.sendJson(response, 200, { accepted: true });
    }

    private async handlePlayerControl(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        if (!this.playbackCoordinator) {
            this.sendError(response, 503, 'Playback is not initialized');
            return;
        }

        const payload = await this.readJsonBody(request, response);
        if (!payload) return;

        const action = payload.action;
        const value = payload.value;

        if (
            action !== 'play' &&
            action !== 'pause' &&
            action !== 'seek' &&
            action !== 'volume'
        ) {
            this.sendError(response, 400, 'Unsupported player control action');
            return;
        }

        if (action === 'seek') {
            if (
                typeof value !== 'number' ||
                !Number.isFinite(value) ||
                value < 0
            ) {
                this.sendError(response, 400, 'Seek value must be non-negative');
                return;
            }
        }

        if (action === 'volume') {
            if (
                typeof value !== 'number' ||
                !Number.isFinite(value) ||
                value < 0 ||
                value > 1
            ) {
                this.sendError(response, 400, 'Volume must be between 0 and 1');
                return;
            }
        }

        /*
         * Do not optimistically update playbackRuntime here. It represents
         * the observed mpv state, not the requested state. Rust consumes
         * this revisioned command and reports the resulting state through
         * /api/player/telemetry.
         */
        this.pendingControl = {
            revision: ++this.controlRevision,
            action,
            ...(typeof value === 'number' ? { value } : {}),
        };

        this.sendJson(response, 202, {
            accepted: true,
            control: this.pendingControl,
        });
    }

    private async handlePrepare(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        const payload = await this.readJsonBody(request, response);
        if (!payload) return;

        const query = payload.query;

        if (typeof query !== 'string' || !query.trim()) {
            this.sendError(
                response,
                400,
                'A YouTube URL or search query is required',
            );
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
                'open.spotify.com',
            ]);

            const hostname = parsedUrl.hostname.toLowerCase();

            const isYouTubeUrl =
                allowedHosts.has(hostname) && hostname !== 'open.spotify.com';

            const isSpotifyTrackUrl =
                hostname === 'open.spotify.com' &&
                /^\/(?:intl-[a-z]{2}\/)?track\/[A-Za-z0-9]{22}\/?$/.test(
                    parsedUrl.pathname,
                );

            if (
                parsedUrl.protocol !== 'https:' ||
                parsedUrl.username ||
                parsedUrl.password ||
                (!isYouTubeUrl && !isSpotifyTrackUrl)
            ) {
                this.sendError(
                    response,
                    400,
                    'Only YouTube URLs and Spotify track URLs are supported',
                );
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

        await pipeline(
            createReadStream(media.path, { start, end }),
            response,
        );
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
