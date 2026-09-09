import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { closeSync, openSync } from 'node:fs';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const web = fileURLToPath(new URL('..', import.meta.url));
const root = resolve(web, '../..');
const children = new Set();
const stop = new AbortController();
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => stop.abort());

async function run(command, args, options = {}) {
  const child = spawn(command, args, {
    cwd: web,
    stdio: 'inherit',
    signal: stop.signal,
    ...options,
  });
  children.add(child);
  try {
    const [code] = await once(child, 'exit');
    if (code !== 0) throw new Error(`${command} exited with status ${code}`);
  } finally {
    children.delete(child);
  }
}

async function unusedPort() {
  const listener = createServer();
  listener.listen(0, '127.0.0.1');
  await once(listener, 'listening');
  const { port } = listener.address();
  await new Promise((resolveClose) => listener.close(resolveClose));
  return port;
}

async function waitReady(server, baseURL) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (server.exitCode !== null || server.signalCode || stop.signal.aborted)
      throw new Error('Temporary Rust server stopped before becoming ready');
    try {
      const response = await fetch(`${baseURL}/readyz`, { signal: AbortSignal.timeout(1000) });
      if (response.ok && (await response.json()).ready) return;
    } catch {
      // Binding and SQLite initialization may still be in progress.
    }
    await delay(100, undefined, { signal: stop.signal });
  }
  throw new Error('Temporary Rust server did not become ready within 30 seconds');
}

async function terminate(child) {
  if (child.exitCode !== null || child.signalCode) return;
  const exited = once(child, 'exit').catch(() => {});
  child.kill('SIGTERM');
  const force = setTimeout(() => child.kill('SIGKILL'), 5000);
  try {
    await exited;
  } finally {
    clearTimeout(force);
  }
}

let directory;
try {
  if (typeof WebSocket === 'undefined') throw new Error('Live E2E requires Node.js 22 or newer');
  let binary = process.env.NODELITE_E2E_SERVER_BIN;
  if (!binary) {
    await run('pnpm', ['build']);
    await run('cargo', ['build', '--locked', '-p', 'nodelite-server'], {
      cwd: root,
      env: { ...process.env, NODELITE_SKIP_WEB_BUILD: '1' },
    });
    binary = resolve(root, 'target/debug/nodelite-server');
  }
  directory = await mkdtemp(resolve(tmpdir(), 'nodelite-live-e2e-'));
  const port = await unusedPort();
  const baseURL = `http://127.0.0.1:${port}`;
  const username = `e2e-${randomBytes(8).toString('hex')}`;
  const password = `Aa1!${randomBytes(32).toString('hex')}`;
  const config = resolve(directory, 'server.toml');
  await writeFile(
    config,
    `
[server]
listen = "127.0.0.1:${port}"
public_base_url = "${baseURL}"
node_registry_path = ${JSON.stringify(resolve(directory, 'registry.json'))}
history_db_path = ${JSON.stringify(resolve(directory, 'history.sqlite3'))}
snapshot_path = ${JSON.stringify(resolve(directory, 'snapshot.json'))}
ping_interval_secs = 2
[ui]
refresh_interval_secs = 1
[auth]
username = "${username}"
password = "${password}"
[audit]
db_path = ${JSON.stringify(resolve(directory, 'audit.sqlite3'))}
[geoip]
enabled = false
`,
    { mode: 0o600 },
  );
  const log = openSync(resolve(directory, 'server.log'), 'w', 0o600);
  const server = spawn(resolve(binary), ['--config', config], {
    cwd: directory,
    stdio: ['ignore', log, log],
  });
  children.add(server);
  closeSync(log);
  server.on('error', () => stop.abort());
  await waitReady(server, baseURL);
  console.log('Live E2E: temporary Rust server ready; credentials and data are isolated.');
  await run(
    'pnpm',
    [
      'exec',
      'playwright',
      'test',
      '--config',
      'playwright.live.config.ts',
      ...process.argv.slice(2),
    ],
    {
      env: {
        ...process.env,
        NODELITE_E2E_BASE_URL: baseURL,
        NODELITE_E2E_USER: username,
        NODELITE_E2E_PASS: password,
      },
    },
  );
  const report = JSON.parse(await readFile(resolve(web, 'test-results/live-results.json'), 'utf8'));
  if (report.stats.skipped !== 0 || report.stats.expected === 0)
    throw new Error('Live E2E must execute tests with skipped=0');
  console.log(`Live E2E: ${report.stats.expected} passed, skipped=0.`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await Promise.all([...children].map(terminate));
  if (directory) {
    await mkdir(resolve(web, 'test-results'), { recursive: true });
    await copyFile(
      resolve(directory, 'server.log'),
      resolve(web, 'test-results/live-server.log'),
    ).catch(() => {});
    await rm(directory, { recursive: true, force: true });
  }
}
