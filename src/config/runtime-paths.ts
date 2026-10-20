
import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const moduleDirectory = dirname(fileURLToPath(import.meta.url));

/**
 * In the compiled development build, this module lives in dist/config.
 * In the bundled SEA executable, process.execPath points to the .exe.
 */
export const runtimeDirectory = process.env.SONG_REQUEST_HOME?.trim()
  ? resolve(process.env.SONG_REQUEST_HOME.trim())
  : process.execPath.toLowerCase().endsWith('.exe')
    ? dirname(process.execPath)
    : resolve(moduleDirectory, '../..');

export function runtimePath(path: string): string {
  return isAbsolute(path) ? path : resolve(runtimeDirectory, path);
}
