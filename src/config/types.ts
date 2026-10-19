
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

  server: {
    host: string;
    port: number;
  };

  playback: {
    radioUrl?: string;
    mpvPath: string;
    ytDlpPath: string;
    ffmpegPath: string;
    mediaDir: string;
    maxVideoHeight: number;
  };
}
