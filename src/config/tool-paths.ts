export interface DefaultToolPaths {
  ytDlpPath: string;
  ffmpegPath: string;
}

export function defaultToolPaths(platform: NodeJS.Platform): DefaultToolPaths {
  if (platform === 'win32') {
    return {
      ytDlpPath: 'vendor/yt-dlp.exe',
      ffmpegPath: 'vendor/ffmpeg/bin/ffmpeg.exe',
    };
  }

  return {
    ytDlpPath: 'vendor-linux/yt-dlp',
    ffmpegPath: 'vendor-linux/ffmpeg',
  };
}
