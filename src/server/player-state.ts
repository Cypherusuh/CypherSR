import type { PlaybackCoordinator } from '../playback/playback-coordinator.js';
import type {
    JsonObject,
    PlayerControl,
    PlaybackRuntime,
} from './media-server.types.js';

export class PlayerState {
    private runtime: PlaybackRuntime = {
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
        private readonly getCoordinator: () => PlaybackCoordinator | null,
    ) {}

    getRuntime(): PlaybackRuntime {
        return this.runtime;
    }

    getControl(): PlayerControl | null {
        return this.pendingControl;
    }

    getState() {
        const coordinator = this.getCoordinator();

        if (!coordinator) {
            return null;
        }

        const playerState = coordinator.getPlayerState('');
        const current = playerState.current;

        /*
         * Initialize state for a new track if it changes before the next
         * telemetry report arrives from Rust/mpv.
         */
        if (current && this.runtime.requestId !== current.requestId) {
            this.runtime = {
                requestId: current.requestId,
                position: 0,
                duration: current.duration ?? 0,
                volume: this.runtime.volume,
                paused: false,
                updatedAt: Date.now(),
            };
        } else if (!current && this.runtime.requestId !== null) {
            this.runtime = {
                requestId: null,
                position: 0,
                duration: 0,
                volume: this.runtime.volume,
                paused: true,
                updatedAt: Date.now(),
            };
        }

        return {
            ...playerState,
            playback: this.runtime,
            control: this.pendingControl,
        };
    }

    async handleEnded(requestId: string): Promise<boolean> {
        const coordinator = this.getCoordinator();

        if (!coordinator) {
            return false;
        }

        const current = coordinator.getPlayerState('').current;

        // Ignore stale end notifications.
        if (!current || current.requestId !== requestId) {
            return false;
        }

        const accepted = await coordinator.trackEnded(requestId);

        if (
            accepted &&
            this.runtime.requestId === requestId
        ) {
            this.runtime = {
                ...this.runtime,
                requestId: null,
                position: 0,
                duration: 0,
                paused: true,
                updatedAt: Date.now(),
            };
        }

        return accepted;
    }

    handleTelemetry(payload: JsonObject): {
        accepted: boolean;
        status: 200 | 400 | 409;
        error?: string;
    } {
        const requestId = payload.requestId;
        const position = payload.position;
        const duration = payload.duration;
        const volume = payload.volume;
        const paused = payload.paused;

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
            return {
                accepted: false,
                status: 400,
                error: 'Invalid playback telemetry',
            };
        }

        const coordinator = this.getCoordinator();

        if (!coordinator) {
            return {
                accepted: false,
                status: 409,
            };
        }

        const current = coordinator.getPlayerState('').current;

        if (!current || current.requestId !== requestId) {
            return {
                accepted: false,
                status: 409,
            };
        }

        this.runtime = {
            requestId,
            position: duration > 0
                ? Math.min(position, duration)
                : position,
            duration,
            volume,
            paused,
            updatedAt: Date.now(),
        };

        return {
            accepted: true,
            status: 200,
        };
    }

    handleControl(
        payload: JsonObject,
    ):
        | {
              accepted: true;
              control: PlayerControl;
          }
        | {
              accepted: false;
              status: 400;
              error: string;
          } {
        const action = payload.action;
        const value = payload.value;

        if (
            action !== 'play' &&
            action !== 'pause' &&
            action !== 'seek' &&
            action !== 'volume'
        ) {
            return {
                accepted: false,
                status: 400,
                error: 'Unsupported player control action',
            };
        }

        if (
            action === 'seek' &&
            (
                typeof value !== 'number' ||
                !Number.isFinite(value) ||
                value < 0
            )
        ) {
            return {
                accepted: false,
                status: 400,
                error: 'Seek value must be non-negative',
            };
        }

        if (
            action === 'volume' &&
            (
                typeof value !== 'number' ||
                !Number.isFinite(value) ||
                value < 0 ||
                value > 1
            )
        ) {
            return {
                accepted: false,
                status: 400,
                error: 'Volume must be between 0 and 1',
            };
        }

        const control: PlayerControl = {
            revision: ++this.controlRevision,
            action,
            ...(typeof value === 'number' ? { value } : {}),
        };

        this.pendingControl = control;

        return {
            accepted: true,
            control,
        };
    }
}