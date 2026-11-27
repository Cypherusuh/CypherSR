import { describe, expect, it } from 'vitest';

import { CommandRouter } from './command-router.js';
import { createSongRequestCommands } from './song-request-commands.js';

describe('!default command status', () => {
  it('sends a lookup status before setting the default song', async () => {
    const events: string[] = [];
    const router = new CommandRouter(
      '!',
      createSongRequestCommands({
        isRequestModeEnabled: () => true,
        toggleRequestMode: async () => true,
        requestSong: async () => '',
        getQueue: () => ({ current: null, pending: [], limit: 5 }),
        removeLatestRequest: () => null,
        skipTrack: async () => true,
        getDefaultSong: () => null,
        setDefaultSong: async query => {
          events.push(`set:${query}`);
          return 'Default song set.';
        },
        clearDefaultSong: async () => 'Default song cleared.',
      }),
      async message => {
        events.push(`reply:${message}`);
      },
    );

    await router.handleMessage({
      userId: 'broadcaster',
      username: 'broadcaster',
      displayName: 'Broadcaster',
      message: '!default https://example.com/song',
      isBroadcaster: true,
    });

    expect(events).toEqual([
      'reply:Looking up default song, might takes awhile',
      'set:https://example.com/song',
      'reply:Default song set.',
    ]);
  });
});
