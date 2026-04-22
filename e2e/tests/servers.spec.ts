import { test, expect } from '@playwright/test';
import { readToken } from '../helpers/auth';

const BASE_URL = 'http://localhost:9190';

async function apiListServers(token: string): Promise<Array<{ id: string; name: string; host: string }>> {
  const r = await fetch(`${BASE_URL}/api/config/servers`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  return r.json();
}

async function apiAddServer(token: string, name: string, host: string, connections = 4): Promise<void> {
  const r = await fetch(`${BASE_URL}/api/config/servers`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
    body: JSON.stringify({
      id: '',
      name,
      host,
      port: 563,
      ssl: true,
      ssl_verify: false,
      username: null,
      password: null,
      connections,
      priority: 0,
      enabled: true,
      retention: 0,
      pipelining: 16,
      optional: false,
      compress: false,
      ramp_up_delay_ms: 0,
      proxy_url: null,
      trusted_fingerprint: null,
    }),
  });
  if (!r.ok) throw new Error(`apiAddServer failed: ${r.status} ${await r.text()}`);
}

async function apiDeleteServerByName(token: string, name: string): Promise<void> {
  const servers = await apiListServers(token);
  const target = servers.find((s) => s.name === name);
  if (!target) return;
  const r = await fetch(`${BASE_URL}/api/config/servers/${target.id}`, {
    method: 'DELETE',
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!r.ok && r.status !== 404) throw new Error(`apiDeleteServer failed: ${r.status}`);
}

async function navigateToServers(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/settings');
  // "News servers" is the default sidebar tab; ensure it is active
  await expect(page.getByRole('heading', { name: 'News servers' })).toBeVisible();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe('2. News Server Management', () => {
  // Auto-accept browser confirm dialogs for all tests in this suite
  test.beforeEach(async ({ page }) => {
    page.on('dialog', (dialog) => dialog.accept());
  });

  // ── 2.1 Add a news server ─────────────────────────────────────────────────

  test('2.1 add a news server', async ({ page }) => {
    const serverName = 'E2E Server';
    const serverHost = 'e2e.test.com';

    await navigateToServers(page);

    // Open the add-server form
    await page.getByRole('button', { name: '+ Add server' }).click();

    // Fill the form
    await page.getByPlaceholder('news-primary').fill(serverName);
    await page.getByPlaceholder('news.example.com').fill(serverHost);

    // Connections field — find by label proximity or attribute
    const connectionsInput = page.locator('input[type="number"]').first();
    await connectionsInput.fill('8');

    // Save
    await page.getByRole('button', { name: 'Save' }).click();

    // Server row should appear
    await expect(page.locator('.srv-row', { hasText: serverHost })).toBeVisible();

    // Snackbar confirms
    await expect(page.getByText('Server added', { exact: false })).toBeVisible();

    // Cleanup
    const token = readToken();
    await apiDeleteServerByName(token, serverName);
  });

  // ── 2.2 Server form validation — empty host ───────────────────────────────

  test('2.2 server form rejects empty host', async ({ page }) => {
    await navigateToServers(page);

    await page.getByRole('button', { name: '+ Add server' }).click();

    // Clear the host field (it may have a placeholder but no value)
    const hostInput = page.getByPlaceholder('news.example.com');
    await expect(hostInput).toBeVisible();
    await hostInput.clear();

    await page.getByRole('button', { name: 'Save' }).click();

    // Client-side validation keeps the form open — wait (up to 10s) for the
    // host input to remain visible rather than doing an immediate point-in-time check.
    await expect(page).toHaveURL(/\/settings/);
    await expect(hostInput).toBeVisible();
  });

  // ── 2.3 Edit existing server ──────────────────────────────────────────────

  test('2.3 edit server connections count', async ({ page }) => {
    await navigateToServers(page);

    // Find "Test Server" row and click its Edit button
    const testServerRow = page.locator('.srv-row', { hasText: 'Test Server' });
    await expect(testServerRow).toBeVisible();
    await testServerRow.getByRole('button', { name: 'Edit' }).click();

    // Change connections to 12
    const connectionsInput = page.locator('input[type="number"]').first();
    await connectionsInput.fill('12');

    await page.getByRole('button', { name: 'Save' }).click();

    // Snackbar
    await expect(page.getByText('Server updated', { exact: false })).toBeVisible();

    // Restore to original value
    const testServerRowAgain = page.locator('.srv-row', { hasText: 'Test Server' });
    await testServerRowAgain.getByRole('button', { name: 'Edit' }).click();
    const connectionsInputAgain = page.locator('input[type="number"]').first();
    await connectionsInputAgain.fill('4');
    await page.getByRole('button', { name: 'Save' }).click();
  });

  // ── 2.4 Delete server with confirmation ───────────────────────────────────

  test('2.4 delete server with confirm dialog', async ({ page }) => {
    // Add a temp server via API first
    const token = readToken();
    await apiAddServer(token, 'Temp Delete Server', 'temp.delete.test.com');

    await navigateToServers(page);

    // Confirm the temp server appears
    const tempRow = page.locator('.srv-row', { hasText: 'temp.delete.test.com' });
    await expect(tempRow).toBeVisible();

    // Click Remove — dialog is auto-accepted via beforeEach handler
    await tempRow.getByRole('button', { name: 'Remove' }).click();

    // Row should disappear
    await expect(tempRow).not.toBeVisible();

    // Snackbar
    await expect(page.getByText('Server removed', { exact: false })).toBeVisible();
  });

  // ── 2.5 Test server connection — failure ─────────────────────────────────
  // "Test Server" points to news.example.com which is not reachable in E2E.

  test('2.5 test connection failure shows snackbar', async ({ page }) => {
    await navigateToServers(page);

    const testServerRow = page.locator('.srv-row', { hasText: 'Test Server' });
    await expect(testServerRow).toBeVisible();

    await testServerRow.getByRole('button', { name: 'Test' }).click();

    // Expect a failure snackbar — connection to news.example.com will time out or fail
    await expect(
      page.getByText(/Connection failed|timed out|unreachable|error/i),
    ).toBeVisible({ timeout: 20000 });
  });

  // ── 2.6 Toggle server enabled/disabled via row action ─────────────────────
  // One-click Disable/Enable button sits next to Test/Edit/Clone/Remove and
  // flips the `enabled` flag without opening the edit form.
  test('2.6 toggle server enabled state via row button', async ({ page }) => {
    const token = readToken();
    const serverName = 'Toggle Target';
    const serverHost = 'toggle.test.com';
    await apiAddServer(token, serverName, serverHost);

    try {
      await navigateToServers(page);

      const row = page.locator('.srv-row', { hasText: serverHost });
      await expect(row).toBeVisible();

      // Starts enabled
      await expect(row.getByText('● enabled')).toBeVisible();

      // Click Disable → row reflects "disabled", button label flips to "Enable"
      await row.getByRole('button', { name: 'Disable' }).click();
      await expect(page.getByText('Server disabled', { exact: false })).toBeVisible();
      await expect(row.getByText('● disabled')).toBeVisible();
      await expect(row.getByRole('button', { name: 'Enable' })).toBeVisible();

      // Round-trip back to enabled
      await row.getByRole('button', { name: 'Enable' }).click();
      await expect(page.getByText('Server enabled', { exact: false })).toBeVisible();
      await expect(row.getByText('● enabled')).toBeVisible();
      await expect(row.getByRole('button', { name: 'Disable' })).toBeVisible();

      // Persisted on the backend
      const servers = await apiListServers(token);
      const found = servers.find((s) => s.name === serverName) as { enabled?: boolean } | undefined;
      expect(found?.enabled).toBe(true);
    } finally {
      await apiDeleteServerByName(token, serverName);
    }
  });
});
