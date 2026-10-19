
import type {
  ChatMessage,
  CommandDefinition,
} from './types.js';

export class CommandRouter {
  private readonly commands = new Map<string, CommandDefinition>();

  constructor(
    private readonly prefix: string,
    commands: CommandDefinition[],
    private readonly sendReply: (message: string) => Promise<void>,
  ) {
    if (!prefix.trim()) {
      throw new Error('Command prefix cannot be empty.');
    }

    for (const command of commands) {
      for (const name of [command.name, ...(command.aliases ?? [])]) {
        const normalized = name.toLowerCase();

        if (this.commands.has(normalized)) {
          throw new Error(`Duplicate command or alias: ${normalized}`);
        }

        this.commands.set(normalized, command);
      }
    }
  }

  async handleMessage(message: ChatMessage): Promise<void> {
    const content = message.message.trim();

    if (!content.startsWith(this.prefix)) {
      return;
    }

    const input = content.slice(this.prefix.length).trim();

    if (!input) {
      return;
    }

const [name, ...args] = input.split(/\s+/);

console.log(
  `[Commands] Parsed command: name="${name}", args="${args.join(' ')}"`,
);

const command = this.commands.get(name.toLowerCase());

if (!command) {
  console.log(`[Commands] Unknown command: ${name}`);
  return;
}

console.log(`[Commands] Executing: ${command.name}`);

    try {
      const reply = await command.handler(message, args);

      if (reply?.message.trim()) {
        await this.sendReply(reply.message);
      }
    } catch (error) {
      console.error(`[Commands] Failed to execute ${command.name}:`, error);
    }
  }
}
