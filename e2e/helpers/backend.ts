import { spawn, execSync, ChildProcess } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';

const PROJECT_ROOT = path.resolve(__dirname, '../..');
const BINARY = path.join(PROJECT_ROOT, 'target/debug/rustnzb');
const TEST_DATA_DIR = path.join(PROJECT_ROOT, 'e2e/test-data');
const TEST_CONFIG = path.join(PROJECT_ROOT, 'e2e/fixtures/test-config.toml');
const SEED_SQL = path.join(PROJECT_ROOT, 'e2e/fixtures/seed.sql');
const MIGRATIONS_DIR = path.join(PROJECT_ROOT, 'crates/nzb-core/src');
const DB_PATH = path.join(TEST_DATA_DIR, 'rustnzb.db');

let backendProcess: ChildProcess | null = null;

export async function startBackend(): Promise<void> {
  if (fs.existsSync(TEST_DATA_DIR)) fs.rmSync(TEST_DATA_DIR, { recursive: true });
  fs.mkdirSync(TEST_DATA_DIR, { recursive: true });
  fs.mkdirSync(path.join(TEST_DATA_DIR, 'incomplete'), { recursive: true });
  fs.mkdirSync(path.join(TEST_DATA_DIR, 'complete'), { recursive: true });

  // Start backend (it creates DB and runs migrations)
  backendProcess = spawn(BINARY, ['--config', TEST_CONFIG], {
    cwd: PROJECT_ROOT,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  backendProcess.stderr?.on('data', (data) => {
    const msg = data.toString();
    if (msg.includes('ERROR')) process.stderr.write(`[backend] ${msg}`);
  });

  await waitForHealthy('http://localhost:9190/api/health', 15000);

  // Seed test data via sqlite3
  execSync(`sqlite3 "${DB_PATH}" < "${SEED_SQL}"`, { cwd: PROJECT_ROOT });

  // Verify
  const count = execSync(`sqlite3 "${DB_PATH}" "SELECT count(*) FROM groups;"`, { cwd: PROJECT_ROOT }).toString().trim();
  if (count === '0') throw new Error('Seeding failed');
}

export function stopBackend(): void {
  if (backendProcess) { backendProcess.kill('SIGTERM'); backendProcess = null; }
}

export function cleanTestData(): void {
  if (fs.existsSync(TEST_DATA_DIR)) fs.rmSync(TEST_DATA_DIR, { recursive: true });
}

async function waitForHealthy(url: string, timeoutMs: number): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await new Promise(r => setTimeout(r, 200));
  }
  throw new Error(`Backend not healthy within ${timeoutMs}ms`);
}
