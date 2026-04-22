import * as path from 'path';
import * as fs from 'fs';
import { startBackend } from './helpers/backend';
import { setupAuth, buildStorageState, MAIN_URL, FRESH_URL } from './helpers/api';

const PROJECT_ROOT = path.resolve(__dirname, '..');

export default async function globalSetup() {
  // ── Main backend (port 9190): auth + seeded data ───────────────────────────
  console.log('[setup] Starting main backend (port 9190)...');
  await startBackend({
    name: 'main',
    port: 9190,
    config: 'e2e/fixtures/test-config.toml',
    dataDir: 'e2e/test-data',
    seedFile: path.join(PROJECT_ROOT, 'e2e/fixtures/seed.sql'),
  });

  console.log('[setup] Setting up auth on main backend...');
  const tokens = await setupAuth(MAIN_URL);
  const storageState = buildStorageState(MAIN_URL, tokens);
  fs.writeFileSync(
    path.join(PROJECT_ROOT, 'e2e/auth-state.json'),
    JSON.stringify(storageState, null, 2),
  );

  // ── Fresh backend (port 9191): no credentials, no servers ─────────────────
  console.log('[setup] Starting fresh backend (port 9191)...');
  await startBackend({
    name: 'fresh',
    port: 9191,
    config: 'e2e/fixtures/fresh-config.toml',
    dataDir: 'e2e/test-data-fresh',
  });

  console.log('[setup] Both backends ready.');
}
