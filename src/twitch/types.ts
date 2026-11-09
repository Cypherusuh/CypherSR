import type { ChatMessage } from '../commands/types.js';
import type { EventSubConnectionState } from 'streamops-twitch-connector';

export interface TwitchConnectionOptions {
  channel: string;
}

export interface TwitchClient {
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  sendMessage(message: string): Promise<void>;
  getConnectionState(): EventSubConnectionState;
  onConnectionStatus(
    listener: (connected: boolean, error: Error | null) => void,
  ): () => void;

  onMessage(
    listener: (message: ChatMessage) => void,
  ): () => void;
}