import { describe, expect, it } from 'vitest';
import { PlayerState } from './player-state.js';

describe('PlayerState volume', () => {
    it('defaults to 40 percent', () => {
        const state = new PlayerState(() => null);

        expect(state.getRuntime().volume).toBe(0.4);
    });

    it('updates its volume without an active track', () => {
        const state = new PlayerState(() => null);

        const result = state.handleControl({
            action: 'volume',
            value: 0.65,
        });

        expect(result.accepted).toBe(true);
        expect(state.getRuntime().volume).toBe(0.65);
    });
});
