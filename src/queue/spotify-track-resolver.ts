
export interface SpotifyTrackMetadata {
  title: string;
  thumbnail: string | null;
}

interface SpotifyOEmbedResponse {
  title?: string;
  thumbnail_url?: string;
  provider_name?: string;
}

export class SpotifyTrackResolver {
  constructor(private readonly timeoutMs = 10_000) {}

  isSpotifyTrackUrl(input: string): boolean {
    try {
      const url = new URL(input);

      return (
        url.protocol === 'https:' &&
        url.hostname.toLowerCase() === 'open.spotify.com' &&
        /^\/(?:intl-[a-z]{2}\/)?track\/[A-Za-z0-9]{22}\/?$/.test(
          url.pathname,
        ) &&
        !url.username &&
        !url.password
      );
    } catch {
      return false;
    }
  }

  async resolve(input: string): Promise<SpotifyTrackMetadata> {
    if (!this.isSpotifyTrackUrl(input)) {
      throw new Error('Expected a Spotify track URL.');
    }

    const trackUrl = new URL(input);

    // Remove sharing parameters such as ?si=... from the canonical URL.
    trackUrl.search = '';
    trackUrl.hash = '';

    const endpoint = new URL('https://open.spotify.com/oembed');
    endpoint.searchParams.set('url', trackUrl.toString());

    const response = await fetch(endpoint, {
      headers: {
        Accept: 'application/json',
      },
      signal: AbortSignal.timeout(this.timeoutMs),
    });

    if (!response.ok) {
      throw new Error(
        `Spotify link metadata lookup failed with HTTP ${response.status}.`,
      );
    }

    const data = (await response.json()) as SpotifyOEmbedResponse;

    if (data.provider_name && data.provider_name !== 'Spotify') {
      throw new Error('The metadata response was not from Spotify.');
    }

    const title = data.title?.trim();

    if (!title) {
      throw new Error('Spotify did not return a track title.');
    }

    return {
      title,
      thumbnail: data.thumbnail_url ?? null,
    };
  }
}
