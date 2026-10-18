export interface SongRequestConfig {
  twitch: {
    clientId: string;
    channel: string;
    scopes: string[];
    tokenPath: string;
  };

  queue: {
    limit: number;
  };

  commands: {
    prefix: string;
  };

  playback: {
    radioUrl?: string;
    mpvPath: string;
    ytDlpPath: string;
  };
}