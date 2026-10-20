import { readFile } from 'node:fs/promises';
import type {
    IncomingMessage,
    ServerResponse,
} from 'node:http';

export class PageHandler {
    constructor(
        private readonly pages: {
            root: string;
            player: string;
            testPlayer: string;
            queue: string;
            playedNow: string;
        },
    ) {}

    async handle(
        request: IncomingMessage,
        response: ServerResponse,
        pathname: string,
    ): Promise<boolean> {
        const filePath = this.resolvePage(pathname);

        if (!filePath) {
            return false;
        }

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
            return true;
        }

        const html = await readFile(filePath);

        response.writeHead(200, {
            'Content-Type':
                'text/html; charset=utf-8',
            'Content-Length': html.length,
            'Cache-Control': 'no-store',
            'X-Content-Type-Options': 'nosniff',
        });

        response.end(
            request.method === 'HEAD'
                ? undefined
                : html,
        );

        return true;
    }

    private resolvePage(
        pathname: string,
    ): string | null {
        switch (pathname) {
            case '/':
                return this.pages.testPlayer;

            case '/player':
            case '/player.html':
                return this.pages.player;

            case '/player-test.html':
                return this.pages.testPlayer;

            case '/played-now.html':
                return this.pages.playedNow;

            case '/queue.html':
                return this.pages.queue;

            default:
                return null;
        }
    }

    private sendError(
        response: ServerResponse,
        status: number,
        message: string,
    ): void {
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