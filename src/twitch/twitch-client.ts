import {
    TwitchClient as TwitchConnectorClient,
    type TwitchTokenStore,
} from 'streamops-twitch-connector';

import type { ChatMessage } from '../commands/types.js';
import type { TwitchClient as TwitchClientInterface } from './types.js';

interface TwitchUser {
    id: string;
    login: string;
}

interface TwitchUsersResponse {
    data: TwitchUser[];
}

interface RawChatEvent {
    badges?: Array<{
        set_id?: string;
    }>;
}

export interface TwitchAdapterOptions {
    clientId: string;
    channel: string;
    scopes: string[];
    tokenStore: TwitchTokenStore;
}

export class TwitchClient implements TwitchClientInterface {
    private readonly client: TwitchConnectorClient;
    private readonly channel: string;

    private broadcasterId: string | null = null;
    private authenticatedUserId: string | null = null;
    private removeMessageListener: (() => void) | null = null;
    private connecting: Promise<void> | null = null;

    constructor(options: TwitchAdapterOptions) {
        this.channel = options.channel.replace(/^@/, '').toLowerCase();

        this.client = new TwitchConnectorClient({
            clientId: options.clientId,
            scopes: options.scopes,
            tokenStore: options.tokenStore,
        });
    }

    async login(): Promise<void> {
        const login = await this.client.tokens.startLogin();

        await this.client.tokens.completeLogin();

        console.log('[Twitch] Authorization successful.');
    }


    async connect(): Promise<void> {
        if (this.connecting) {
            return this.connecting;
        }

        this.connecting = this.connectInternal();

        try {
            await this.connecting;
        } catch (error) {
            this.removeMessageListener?.();
            this.removeMessageListener = null;

            await this.client.disconnectEventSub().catch(() => undefined);

            this.broadcasterId = null;
            this.authenticatedUserId = null;

            throw error;
        } finally {
            this.connecting = null;
        }
    }


    private async connectInternal(): Promise<void> {
        const token = await this.client.tokens.getValidatedToken();

        this.authenticatedUserId = token.user_id;

        const response = await this.client.api.get<TwitchUsersResponse>(
            `/users?login=${encodeURIComponent(this.channel)}`,
        );

        const broadcaster = response.data[0];

        if (!broadcaster) {
            throw new Error(`Twitch channel "${this.channel}" was not found.`);
        }

        this.broadcasterId = broadcaster.id;

        this.removeMessageListener = this.client.chat.onMessage(
            async (message) => {
                const raw = message.raw as RawChatEvent;
                const badges = raw.badges ?? [];

                const chatMessage: ChatMessage = {
                    userId: message.chatterId,
                    username: message.chatterName.toLowerCase(),
                    displayName: message.chatterName,
                    message: message.text,
                    isBroadcaster: message.chatterId === this.broadcasterId,
                    isModerator: badges.some(
                        (badge) =>
                            badge.set_id === 'moderator' ||
                            badge.set_id === 'broadcaster',
                    ),
                };

                for (const listener of this.messageListeners) {
                    listener(chatMessage);
                }
            },
        );

        this.client.eventSub.onSession(
            async (sessionId: string, isReconnectHandover: any) => {
                if (isReconnectHandover) {
                    console.log(
                        '[Twitch] Session handover completed; retaining existing subscriptions.',
                    );
                    return;
                }

                if (!this.broadcasterId || !this.authenticatedUserId) {
                    throw new Error(
                        'Cannot subscribe to chat without authenticated and broadcaster IDs.',
                    );
                }

                const subscription =
                    await this.client.subscriptions.subscribeToChatMessages(
                        this.broadcasterId,
                        this.authenticatedUserId,
                        sessionId,
                    );

                const status = subscription.data?.[0]?.status;

                if (status !== 'enabled') {
                    throw new Error(
                        `Twitch chat subscription was not enabled (status: ${status ?? 'missing'
                        }).`,
                    );
                }
            },
        );

        await this.client.connectEventSub();

        await this.waitForSessionId();
    }


    async disconnect(): Promise<void> {
        this.removeMessageListener?.();
        this.removeMessageListener = null;

        await this.client.disconnectEventSub();

        this.broadcasterId = null;
        this.authenticatedUserId = null;
    }

    async sendMessage(message: string): Promise<void> {
        if (!this.broadcasterId || !this.authenticatedUserId) {
            throw new Error('Twitch client is not connected.');
        }

        const result = await this.client.chat.sendMessage({
            broadcasterId: this.broadcasterId,
            senderId: this.authenticatedUserId,
            message,
        });

        if (!result.isSent) {
            throw new Error(
                `Twitch rejected the chat message: ${result.dropReason ?? 'unknown reason'}`,
            );
        }
    }

    onMessage(listener: (message: ChatMessage) => void): () => void {
        this.messageListeners.add(listener);

        return () => {
            this.messageListeners.delete(listener);
        };
    }

    private readonly messageListeners = new Set<
        (message: ChatMessage) => void
    >();

    private async waitForSessionId(): Promise<string> {
        const timeoutMs = 10_000;
        const intervalMs = 100;
        const deadline = Date.now() + timeoutMs;

        while (Date.now() < deadline) {
            const sessionId = this.client.eventSub.currentSessionId;

            if (sessionId) {
                return sessionId;
            }

            if (
                this.client.eventSub.connectionState === 'disconnected'
            ) {
                throw new Error('Twitch EventSub disconnected before a session was established.');
            }

            await new Promise<void>((resolve) => {
                setTimeout(resolve, intervalMs);
            });
        }

        throw new Error('Timed out waiting for the Twitch EventSub session.');
    }
}