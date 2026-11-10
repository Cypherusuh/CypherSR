export interface ChatMessage {
  userId: string;
  username: string;
  displayName: string;
  message: string;
  isBroadcaster?: boolean;
  isModerator?: boolean;
}

export interface ChatReply {
  message: string;
}

export type CommandStatus = (message: string) => Promise<void>;

export type CommandHandler = (
  message: ChatMessage,
  args: string[],
  sendStatus?: CommandStatus,
) => Promise<ChatReply | null>;

export interface CommandDefinition {
  name: string;
  aliases?: string[];
  description: string;
  handler: CommandHandler;
}