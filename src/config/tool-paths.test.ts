import { describe, expect, it } from 'vitest';

import { defaultToolPaths } from './tool-paths.js';

describe('defaultToolPaths', () => {
  it('keeps the existing Windows vendor paths', () => {
    expect(defaultToolPaths('win32')).toEqual({
      ytDlpPath: 'vendor/yt-dlp.exe',
      ffmpegPath: 'vendor/ffmpeg/bin/ffmpeg.exe',
    });
  });

  it('uses the separate Linux vendor directory', () => {
    expect(defaultToolPaths('linux')).toEqual({
      ytDlpPath: 'vendor-linux/yt-dlp',
      ffmpegPath: 'vendor-linux/ffmpeg',
    });
  });
});
