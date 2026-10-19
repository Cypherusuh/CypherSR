
import type {
  ChatMessage,
  CommandDefinition,
  ChatReply,
} from './types.js';

import type { QueueSnapshot } from '../queue/types.js';

export interface SongRequestCommandActions {
  isRequestModeEnabled(): boolean;

  toggleRequestMode(): Promise<boolean>;

  requestSong(
    message: ChatMessage,
    query: string,
  ): Promise<string>;

  getQueue(): QueueSnapshot;

  skipTrack(): Promise<boolean>;
}

function reply(message: string): ChatReply {
  return { message };
}

function formatTrack(
  title: string | undefined,
  fallback: string,
): string {
  return title?.trim() || fallback;
}

export function createSongRequestCommands(
  actions: SongRequestCommandActions,
): CommandDefinition[] {
  return [
    {
      name: 'radio',
      description: 'Toggle radio and song-request mode.',
      handler: async (message) => {
        if (!message.isBroadcaster) {
          return reply('Only the broadcaster can use !radio.');
        }

        const enabled = await actions.toggleRequestMode();

        return reply(
          enabled
            ? 'Radio enabled. Song requests are now open!'
            : 'Radio and song requests disabled.',
        );
      },
    },
    {
      name: 'sr',
      aliases: ['song'],
      description: 'Request a song by URL or search query.',
      handler: async (message, args) => {
        if (!actions.isRequestModeEnabled()) {
          return reply('sr not enabled');
        }

        const query = args.join(' ').trim();

        if (!query) {
          return reply('Usage: !sr <URL or search query>');
        }

        console.log(`[SongRequest] Resolving request: ${query}`);
return reply(await actions.requestSong(message, query));
      },
    },
    {
      name: 'queue',
      aliases: ['q'],
      description: 'Show the current track and pending requests.',
      handler: async () => {
        const queue = actions.getQueue();

        if (!queue.current && queue.pending.length === 0) {
          return reply('The song queue is empty.');
        }

        const lines: string[] = [];

        if (queue.current) {
          const current = queue.current;

          lines.push(
            `Now playing: ${formatTrack(current.track?.title, current.query)}`,
          );
        }

        if (queue.pending.length > 0) {
          const pending = queue.pending
            .slice(0, 5)
            .map((request, index) => {
              const title = formatTrack(
                request.track?.title,
                request.query,
              );

              return `${index + 1}. ${title} (${request.requester.displayName})`;
            });

          lines.push(`Up next: ${pending.join(' | ')}`);
        }

        const remaining = queue.pending.length - 5;

        if (remaining > 0) {
          lines.push(`And ${remaining} more.`);
        }

        lines.push(
          `Queue: ${queue.pending.length}/${queue.limit} pending.`,
        );

        return reply(lines.join(' '));
      },
    },
    {
      name: 'skip',
      description: 'Skip the currently playing track.',
      handler: async (message) => {
        if (!message.isModerator && !message.isBroadcaster) {
          return reply('Only moderators and the broadcaster can use !skip.');
        }

        const skipped = await actions.skipTrack();

        return reply(
          skipped
            ? 'Skipping the current track.'
            : 'There is no track to skip.',
        );
      },
    },
  ];
}
