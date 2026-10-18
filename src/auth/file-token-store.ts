import {
    mkdir,
    readFile,
    rename,
    unlink,
    writeFile,
} from 'node:fs/promises';
import { dirname } from 'node:path';

import type {
    TwitchTokenStore,
    TwitchTokens,
} from 'streamops-twitch-connector';

export class FileTokenStore implements TwitchTokenStore {
    constructor(private readonly filePath: string) { }

    async load(): Promise<TwitchTokens | null> {
        let content: string;

        try {
            content = await readFile(this.filePath, 'utf8');
        } catch (error) {
            if (
                error instanceof Error &&
                'code' in error &&
                error.code === 'ENOENT'
            ) {
                return null;
            }

            throw error;
        }

        const value: unknown = JSON.parse(content);

        if (!this.isTokens(value)) {
            throw new Error(`Invalid Twitch token file: ${this.filePath}`);
        }

        return value;
    }

    async save(tokens: TwitchTokens): Promise<void> {
        const directory = dirname(this.filePath);
        await mkdir(directory, { recursive: true, mode: 0o700 });

        const temporaryPath = `${this.filePath}.tmp`;

        try {
            await writeFile(
                temporaryPath,
                JSON.stringify(tokens, null, 2),
                { encoding: 'utf8', mode: 0o600 },
            );

            await rename(temporaryPath, this.filePath);
        } catch (error) {
            await unlink(temporaryPath).catch(() => undefined);
            throw error;
        }
    }

    async clear(): Promise<void> {
        try {
            await unlink(this.filePath);
        } catch (error) {
            if (
                error instanceof Error &&
                'code' in error &&
                error.code === 'ENOENT'
            ) {
                return;
            }

            throw error;
        }
    }

    private isTokens(value: unknown): value is TwitchTokens {
        if (typeof value !== 'object' || value === null) {
            return false;
        }

        const tokens = value as Record<string, unknown>;

        return (
            typeof tokens.accessToken === 'string' &&
            typeof tokens.refreshToken === 'string' &&
            typeof tokens.expiresAt === 'number' &&
            Array.isArray(tokens.scopes) &&
            tokens.scopes.every((scope) => typeof scope === 'string')
        );
    }
}