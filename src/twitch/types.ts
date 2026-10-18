import type { ChatMessage } from '../commands/types.js';

export interface TwitchConnectionOptions {
  channel: string;
}

export interface TwitchClient {
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  sendMessage(message: string): Promise<void>;

  onMessage(
    listener: (message: ChatMessage) => void,
  ): () => void;
}