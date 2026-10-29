export type PlayerControlAction =
    | 'play'
    | 'pause'
    | 'seek'
    | 'volume';

export interface PlayerControl {
    revision: number;
    action: PlayerControlAction;
    value?: number;
}

export interface PlaybackRuntime {
    requestId: string | null;
    position: number;
    duration: number;
    volume: number;
    paused: boolean;
    updatedAt: number;
}

export interface ByteRange {
    start: number;
    end: number;
}

export type JsonObject = Record<string, unknown>;

export function isObject(
    value: unknown,
): value is Record<string, unknown> {
    return (
        typeof value === 'object' &&
        value !== null &&
        !Array.isArray(value)
    );
}

export function isFiniteNumber(
    value: unknown,
): value is number {
    return typeof value === 'number' && Number.isFinite(value);
}

export function errorCode(
    error: unknown,
): string | undefined {
    if (!isObject(error) || typeof error.code !== 'string') {
        return undefined;
    }

    return error.code;
}

export function isPrematureClose(error: unknown): boolean {
    return errorCode(error) === 'ERR_STREAM_PREMATURE_CLOSE';
}

export interface TwitchApiStatus {
    authenticated: boolean;
    connecting: boolean;
    connected: boolean;
    username: string | null;
    error: string | null;
}

export interface TwitchApiController {
    getStatus(): TwitchApiStatus;
    connect(): Promise<void>;
    disconnect(): Promise<void>;
}