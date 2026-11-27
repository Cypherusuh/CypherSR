import { describe, expect, it, vi } from 'vitest';

import { createSongRequestCommands } from './song-request-commands.js';

describe('!default command status', () => {
  it('sends a lookup status before setting the default song', async () => {
    const events: string[] = [];
    const command = createSongRequestCommands({
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
    }).find(item => item.name === 'default');

    if (!command) {
      throw new Error('Expected the !default command to exist.');
    }

    const sendStatus = vi.fn(async (message: string) => {
      events.push(message);
    });

    const response = await command.handler(
      {
        userId: 'broadcaster',
        username: 'broadcaster',
        displayName: 'Broadcaster',
        message: '!default https://example.com/song',
        isBroadcaster: true,
      },
      ['https://example.com/song'],
      sendStatus,
    );

    expect(sendStatus).toHaveBeenCalledWith(
      'Looking up default song, might takes awhile',
    );
    expect(events).toEqual([
      'Looking up default song, might takes awhile',
      'set:https://example.com/song',
    ]);
    expect(response).toEqual({ message: 'Default song set.' });
  });
});
