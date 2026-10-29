
import { dirname, isAbsolute, resolve } from 'node:path';
import { isSea } from 'node:sea';

function getAppDirectory(): string {
  if (isSea()) {
    return dirname(process.execPath);
  }

  // Development builds are launched from the project root.
  // Avoid import.meta.url because the SEA bundle uses CommonJS.
  return resolve(process.cwd());
}

export const appDirectory = getAppDirectory();

const configuredHome = process.env.SONG_REQUEST_HOME?.trim();

export const dataDirectory = configuredHome
  ? resolve(configuredHome)
  : resolve(appDirectory, 'data');

export function runtimePath(path: string): string {
  return isAbsolute(path)
    ? path
    : resolve(appDirectory, path);
}

export function dataPath(path: string): string {
  return resolve(dataDirectory, path);
}
