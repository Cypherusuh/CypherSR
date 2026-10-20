
import {
    createServer,
    type IncomingMessage,
    type ServerResponse,
} from 'node:http';
import { resolve } from 'node:path';

import type { TrackResolver } from '../queue/track-resolver.js';
import type {
    MediaTranscoder,
    PreparedMedia,
} from '../playback/media-transcoder.js';
import type { PlaybackCoordinator } from '../playback/playback-coordinator.js';

import {
    isObject,
    isPrematureClose,
    type JsonObject,
} from './media-server.types.js';

import { MediaHandler } from './media-handler.js';
import { PageHandler } from './page-handler.js';
import { PlayerState } from './player-state.js';

const MAX_BODY_BYTES = 8 * 1024;
const MEDIA_ROUTE_PREFIX = '/api/media/';

const ALLOWED_YOUTUBE_HOSTS = new Set([
    'youtube.com',
    'www.youtube.com',
    'm.youtube.com',
    'music.youtube.com',
    'youtu.be',
    'youtube-nocookie.com',
    'www.youtube-nocookie.com',
]);

export class MediaServer {
    private readonly server;
    private readonly mediaHandler: MediaHandler;
    private readonly pageHandler: PageHandler;
    private readonly playerState: PlayerState;

    private playbackCoordinator: PlaybackCoordinator | null = null;

    constructor(
        private readonly host: string,
        private readonly port: number,
        mediaDir: string,
        private readonly trackResolver: TrackResolver,
        private readonly transcoder: MediaTranscoder,
    ) {
        const resolvedMediaDir = resolve(mediaDir);

        this.mediaHandler = new MediaHandler(resolvedMediaDir);

        this.pageHandler = new PageHandler({
            root: resolve('public/player-test.html'),
            player: resolve('public/player.html'),
            testPlayer: resolve('public/player-test.html'),
            queue: resolve('public/queue.html'),
            playedNow: resolve('public/played-now.html'),
        });

        this.playerState = new PlayerState(
            () => this.playbackCoordinator,
        );

        this.server = createServer((request, response) => {
            void this.handleRequest(request, response).catch(
                (error: unknown) => {
                    this.handleRequestError(request, response, error);
                },
            );
        });
    }

    async start(): Promise<void> {
        await new Promise<void>((resolveListen, reject) => {
            const onError = (error: Error) => {
                reject(error);
            };

            this.server.once('error', onError);

            this.server.listen(this.port, this.host, () => {
                this.server.off('error', onError);
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

    attachPlaybackCoordinator(
        coordinator: PlaybackCoordinator,
    ): void {
        this.playbackCoordinator = coordinator;
    }

    register(media: PreparedMedia): void {
        this.mediaHandler.register(media);
    }

    unregister(mediaId: string): void {
        this.mediaHandler.unregister(mediaId);
    }

    private async handleRequest(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        const url = new URL(
            request.url ?? '/',
            'http://localhost',
        );
        const { pathname } = url;

        if (await this.pageHandler.handle(request, response, pathname)) {
            return;
        }

        if (pathname === '/api/prepare') {
            if (!this.requireMethod(request, response, 'POST')) {
                return;
            }

            await this.handlePrepare(request, response);
            return;
        }

        if (pathname === '/api/queue') {
            if (!this.requireMethod(request, response, 'GET')) {
                return;
            }

            const coordinator = this.getCoordinator(response);

            if (!coordinator) {
                return;
            }

            this.sendJson(response, 200, coordinator.getQueue());
            return;
        }

        if (pathname === '/api/player/state') {
            if (!this.requireMethod(request, response, 'GET')) {
                return;
            }

            this.handlePlayerState(response);
            return;
        }

        if (pathname === '/api/player/skip') {
            if (!this.requireMethod(request, response, 'POST')) {
                return;
            }

            const coordinator = this.getCoordinator(response);

            if (!coordinator) {
                return;
            }

            const skipped = await coordinator.skipTrack();

            this.sendJson(response, 200, {
                skipped,
                message: skipped
                    ? 'Skipping the current track.'
                    : 'There is no track to skip.',
            });
            return;
        }

        if (pathname === '/api/player/ended') {
            if (!this.requireMethod(request, response, 'POST')) {
                return;
            }

            await this.handlePlayerEnded(request, response);
            return;
        }

        if (pathname === '/api/player/telemetry') {
            if (!this.requireMethod(request, response, 'POST')) {
                return;
            }

            await this.handlePlayerTelemetry(request, response);
            return;
        }

        if (pathname === '/api/player/control') {
            if (!this.requireMethod(request, response, 'POST')) {
                return;
            }

            await this.handlePlayerControl(request, response);
            return;
        }

        if (pathname.startsWith(MEDIA_ROUTE_PREFIX)) {
            await this.mediaHandler.handle(request, response, pathname);
            return;
        }

        this.sendError(response, 404, 'Not found');
    }

    private handleRequestError(
        request: IncomingMessage,
        response: ServerResponse,
        error: unknown,
    ): void {
        const url = new URL(
            request.url ?? '/',
            'http://localhost',
        );

        if (
            url.pathname.startsWith(MEDIA_ROUTE_PREFIX) &&
            isPrematureClose(error) &&
            (
                request.aborted ||
                response.destroyed ||
                !response.writable
            )
        ) {
            return;
        }

        if (
            request.aborted ||
            response.destroyed ||
            response.writableEnded
        ) {
            return;
        }

        console.error('[MediaServer] Request failed:', error);

        if (!response.headersSent) {
            this.sendError(response, 500, 'Internal server error');
            return;
        }

        response.destroy();
    }

    private requireMethod(
        request: IncomingMessage,
        response: ServerResponse,
        method: string,
    ): boolean {
        if (request.method === method) {
            return true;
        }

        response.setHeader('Allow', method);
        this.sendError(response, 405, 'Method not allowed');
        return false;
    }

    private getCoordinator(
        response: ServerResponse,
    ): PlaybackCoordinator | null {
        if (this.playbackCoordinator) {
            return this.playbackCoordinator;
        }

        this.sendError(response, 503, 'Playback is not initialized');
        return null;
    }

    private handlePlayerState(response: ServerResponse): void {
        const state = this.playerState.getState();

        if (!state) {
            this.sendError(response, 503, 'Playback is not initialized');
            return;
        }

        this.sendJson(response, 200, state);
    }

    private async readJsonBody(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<JsonObject | null> {
        const chunks: Buffer[] = [];
        let totalBytes = 0;

        for await (const chunk of request) {
            const buffer = Buffer.isBuffer(chunk)
                ? chunk
                : Buffer.from(chunk);

            totalBytes += buffer.length;

            if (totalBytes > MAX_BODY_BYTES) {
                this.sendError(response, 413, 'Request body too large');

                // Stop consuming this request without destroying the
                // connection before the error response can be sent.
                request.resume();
                return null;
            }

            chunks.push(buffer);
        }

        let payload: unknown;

        try {
            const body = Buffer.concat(chunks).toString('utf8');
            payload = JSON.parse(body);
        } catch {
            this.sendError(response, 400, 'Invalid JSON body');
            return null;
        }

        if (!isObject(payload)) {
            this.sendError(response, 400, 'Expected a JSON object');
            return null;
        }

        return payload;
    }

    private async handlePlayerEnded(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        if (!this.getCoordinator(response)) {
            return;
        }

        const payload = await this.readJsonBody(request, response);

        if (!payload) {
            return;
        }

        if (typeof payload.requestId !== 'string') {
            this.sendError(response, 400, 'A requestId is required');
            return;
        }

        const accepted = await this.playerState.handleEnded(
            payload.requestId,
        );

        this.sendJson(response, 200, { accepted });
    }

    private async handlePlayerTelemetry(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        if (!this.getCoordinator(response)) {
            return;
        }

        const payload = await this.readJsonBody(request, response);

        if (!payload) {
            return;
        }

        const result = this.playerState.handleTelemetry(payload);

        if (!result.accepted) {
            if (result.error) {
                this.sendError(response, result.status, result.error);
            } else {
                this.sendJson(response, result.status, {
                    accepted: false,
                });
            }

            return;
        }

        this.sendJson(response, 200, { accepted: true });
    }

    private async handlePlayerControl(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        if (!this.getCoordinator(response)) {
            return;
        }

        const payload = await this.readJsonBody(request, response);

        if (!payload) {
            return;
        }

        const result = this.playerState.handleControl(payload);

        if (!result.accepted) {
            this.sendError(response, result.status, result.error);
            return;
        }

        this.sendJson(response, 202, result);
    }

    private async handlePrepare(
        request: IncomingMessage,
        response: ServerResponse,
    ): Promise<void> {
        const payload = await this.readJsonBody(request, response);

        if (!payload) {
            return;
        }

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

            const hostname = parsedUrl.hostname.toLowerCase();

            const isYouTubeUrl = ALLOWED_YOUTUBE_HOSTS.has(hostname);

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
                mediaUrl: `${baseUrl}${MEDIA_ROUTE_PREFIX}${media.id}`,
            });
        } catch (error) {
            console.error('[MediaServer] Prepare failed:', error);

            if (response.destroyed || response.writableEnded) {
                return;
            }

            this.sendJson(response, 422, {
                error: error instanceof Error
                    ? error.message
                    : 'Unable to prepare this track',
            });
        }
    }

    private sendJson(
        response: ServerResponse,
        status: number,
        value: unknown,
    ): void {
        if (response.destroyed || response.writableEnded) {
            return;
        }

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
