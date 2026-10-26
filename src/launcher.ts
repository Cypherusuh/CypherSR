import { createHash } from 'node:crypto';
import { createWriteStream, existsSync } from 'node:fs';
import {
  access,
  mkdir,
  readFile,
  rename,
  rm,
} from 'node:fs/promises';
import { spawn, spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';

const REPOSITORY = 'Cypherusuh/CypherSR';

const TEST_MODE = process.env.CYPHERSR_TEST_UPDATE === '1';

const MANIFEST_URL = TEST_MODE
  ? (
      process.env.CYPHERSR_UPDATE_MANIFEST_URL ??
      'http://127.0.0.1:8765/latest.json'
    )
  : 'https://github.com/Cypherusuh/CypherSR/releases/latest/download/latest.json';

const UPDATE_ASSET_URL =
  'https://github.com/Cypherusuh/CypherSR/releases/latest/download/CypherSR-update-win-x64.zip';

const CORE_EXE = 'CypherSR-core.exe';
const VERSION_FILE = 'version.json';

interface UpdateManifest {
  version: string;
  url: string;
  sha256: string;
}

interface InstalledVersion {
  version: string;
}

const appDirectory = dirname(process.execPath);
const dataDirectory = join(appDirectory, 'data');
const updateDirectory = join(appDirectory, '.update');

function log(message: string): void {
  console.log(`[CypherSR] ${message}`);
}

function compareVersions(left: string, right: string): number {
  const a = left.split('.').map(Number);
  const b = right.split('.').map(Number);

  for (let i = 0; i < 3; i++) {
    const difference = (a[i] ?? 0) - (b[i] ?? 0);

    if (difference !== 0) {
      return difference > 0 ? 1 : -1;
    }
  }

  return 0;
}

async function exists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function getInstalledVersion(): Promise<string> {
  try {
    const file = join(appDirectory, VERSION_FILE);
    const installed = JSON.parse(
      await readFile(file, 'utf8'),
    ) as InstalledVersion;

    return installed.version;
  } catch {
    return '0.0.0';
  }
}

async function fetchManifest(): Promise<UpdateManifest> {
  const response = await fetch(MANIFEST_URL, {
    headers: { 'Cache-Control': 'no-cache' },
    signal: AbortSignal.timeout(30_000),
  });

  if (!response.ok) {
    throw new Error(`GitHub returned HTTP ${response.status}`);
  }

  const manifest = await response.json() as UpdateManifest;

  if (
    typeof manifest.version !== 'string' ||
    !/^\d+\.\d+\.\d+$/.test(manifest.version)
  ) {
    throw new Error('Invalid version in latest.json');
  }

if (typeof manifest.url !== 'string') {
  throw new Error('Missing update download URL');
}

if (TEST_MODE) {
  const url = new URL(manifest.url);

  if (
    url.protocol !== 'http:' ||
    !['127.0.0.1', 'localhost'].includes(url.hostname)
  ) {
    throw new Error('Test updates must use a localhost URL');
  }
} else if (manifest.url !== UPDATE_ASSET_URL) {
  throw new Error('Unexpected update download URL');
}

  if (
    typeof manifest.sha256 !== 'string' ||
    !/^[a-fA-F0-9]{64}$/.test(manifest.sha256)
  ) {
    throw new Error('Invalid SHA-256 checksum in latest.json');
  }

  return manifest;
}

async function downloadUpdate(
  manifest: UpdateManifest,
): Promise<string> {
  await mkdir(updateDirectory, { recursive: true });

  const zipPath = join(updateDirectory, 'update.zip');
  await rm(zipPath, { force: true });

  log(`Downloading version ${manifest.version}...`);

  const response = await fetch(manifest.url, {
    signal: AbortSignal.timeout(15 * 60 * 1000),
  });

  if (!response.ok || !response.body) {
    throw new Error(`Update download failed: HTTP ${response.status}`);
  }

  await pipeline(
    Readable.fromWeb(response.body as never),
    createWriteStream(zipPath),
  );

  const hash = createHash('sha256');

  const file = await readFile(zipPath);
  hash.update(file);

  const actualHash = hash.digest('hex');
  const expectedHash = manifest.sha256.toLowerCase();

  if (actualHash !== expectedHash) {
    await rm(zipPath, { force: true });
    throw new Error('SHA-256 verification failed; update rejected');
  }

  log('Download checksum verified.');

  return zipPath;
}

function quotePowerShell(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

async function extractUpdate(
  zipPath: string,
  stagingDirectory: string,
): Promise<void> {
  await rm(stagingDirectory, { recursive: true, force: true });
  await mkdir(stagingDirectory, { recursive: true });

  const command =
    `Expand-Archive -LiteralPath ${quotePowerShell(zipPath)} ` +
    `-DestinationPath ${quotePowerShell(stagingDirectory)} -Force`;

  const result = spawnSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-ExecutionPolicy',
      'Bypass',
      '-Command',
      command,
    ],
    { encoding: 'utf8', windowsHide: true },
  );

  if (result.status !== 0) {
    throw new Error(
      `Could not extract update: ${result.stderr || result.stdout}`,
    );
  }
}

async function installUpdate(
  manifest: UpdateManifest,
  zipPath: string,
): Promise<void> {
  const stagingDirectory = join(updateDirectory, 'staging');
  const backupDirectory = join(updateDirectory, 'backup');

  await extractUpdate(zipPath, stagingDirectory);

  const stagedVersion = JSON.parse(
    await readFile(join(stagingDirectory, VERSION_FILE), 'utf8'),
  ) as InstalledVersion;

  if (stagedVersion.version !== manifest.version) {
    throw new Error('Update ZIP version does not match its manifest');
  }

  const requiredPaths = [
    CORE_EXE,
    VERSION_FILE,
    'public',
    'vendor',
  ];

  for (const name of requiredPaths) {
    if (!(await exists(join(stagingDirectory, name)))) {
      throw new Error(`Update ZIP is missing required item: ${name}`);
    }
  }

  await rm(backupDirectory, { recursive: true, force: true });
  await mkdir(backupDirectory, { recursive: true });

  const installed: Array<{
    name: string;
    hadBackup: boolean;
    newInstalled: boolean;
  }> = [];

  try {
    for (const name of requiredPaths) {
      const destination = join(appDirectory, name);
      const backup = join(backupDirectory, name);
      const staged = join(stagingDirectory, name);

      const hadBackup = await exists(destination);

      if (hadBackup) {
        await rename(destination, backup);
      }

      installed.push({
        name,
        hadBackup,
        newInstalled: false,
      });

      await rename(staged, destination);
      installed[installed.length - 1].newInstalled = true;
    }
  } catch (error) {
    log('Installation failed; attempting rollback...');

    for (const item of [...installed].reverse()) {
      const destination = join(appDirectory, item.name);
      const backup = join(backupDirectory, item.name);

      if (item.newInstalled) {
        await rm(destination, { recursive: true, force: true });
      }

      if (item.hadBackup && await exists(backup)) {
        await rename(backup, destination);
      }
    }

    throw error;
  }

  await rm(backupDirectory, { recursive: true, force: true });
  log(`Updated successfully to ${manifest.version}.`);
}

function startApplication(): void {
  const corePath = join(appDirectory, CORE_EXE);

  if (!requireCoreExists(corePath)) {
    throw new Error(
      `Cannot find ${CORE_EXE}. Re-download the full portable release.`,
    );
  }

  log('Starting song-request app...');

  const child = spawn(corePath, [], {
    cwd: appDirectory,
    env: {
      ...process.env,
      SONG_REQUEST_HOME: dataDirectory,
    },
    stdio: 'inherit',
    windowsHide: false,
  });

  child.on('error', (error) => {
    console.error(`[CypherSR] Could not start app: ${error.message}`);
  });

  child.on('exit', (code, signal) => {
    if (signal) {
      log(`App exited after signal ${signal}.`);
    } else {
      log(`App exited with code ${code ?? 'unknown'}.`);
    }
  });
}

function requireCoreExists(path: string): boolean {
  return existsSync(path);
}

async function main(): Promise<void> {
  console.log('');
  log('Portable Twitch Song Request');
  log(`App directory: ${appDirectory}`);
  log(`Data directory: ${dataDirectory}`);
  console.log('');

  await mkdir(dataDirectory, { recursive: true });

  const installedVersion = await getInstalledVersion();
  log(`Installed version: ${installedVersion}`);

  try {
    const manifest = await fetchManifest();

    if (compareVersions(manifest.version, installedVersion) > 0) {
      log(`New version available: ${manifest.version}`);

      const zipPath = await downloadUpdate(manifest);
      await installUpdate(manifest, zipPath);

      await rm(updateDirectory, { recursive: true, force: true });
    } else {
      log('Already up to date.');
    }
  } catch (error) {
    const message =
      error instanceof Error ? error.message : String(error);

    console.error(`[CypherSR] Update check failed: ${message}`);
    log('Continuing with the installed version, if available.');
  }

  startApplication();
}

main().catch((error: unknown) => {
  console.error(
    '[CypherSR] Fatal error:',
    error instanceof Error ? error.message : error,
  );

  process.exitCode = 1;
});