
import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

function getRuntimeDirectory(): string {
  const configuredHome = process.env.SONG_REQUEST_HOME?.trim();

  if (configuredHome) {
    return resolve(configuredHome);
  }

  if (process.env.NODE_ENV === 'production') {
    return dirname(process.execPath);
  }

  return resolve(
    dirname(fileURLToPath(import.meta.url)),
    '../..',
  );
}

export const runtimeDirectory = getRuntimeDirectory();

export function runtimePath(path: string): string {
  return isAbsolute(path) ? path : resolve(runtimeDirectory, path);
}

export const dataDirectory = runtimePath('data');

export function dataPath(path: string): string {
  return resolve(dataDirectory, path);
}
