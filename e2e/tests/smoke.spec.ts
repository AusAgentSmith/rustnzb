import { test, expect } from '@playwright/test';

test.describe('Smoke Tests', () => {
  test('health endpoint', async ({ request }) => {
    const r = await request.get('/api/health');
    expect(r.status()).toBe(200);
  });

  test('app loads with tabs', async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveURL(/\/queue/);
    await expect(page.getByText('⚡ rustnzbd')).toBeVisible();
    // All tabs visible
    await expect(page.getByRole('link', { name: 'Queue' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Groups' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'History' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Settings' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Logs' })).toBeVisible();
  });

  test('queue tab shows empty state', async ({ page }) => {
    await page.goto('/queue');
    await expect(page.getByText('No downloads in queue')).toBeVisible();
  });

  test('groups tab shows subscribed groups', async ({ page }) => {
    await page.goto('/groups');
    await expect(page.getByText('alt.test')).toBeVisible();
    await expect(page.getByText('alt.binaries.test')).toBeVisible();
  });

  test('clicking a group loads headers', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.group-item', { hasText: 'alt.test' }).click();
    await expect(page.locator('.panel-title')).toContainText('alt.test');
    await expect(page.getByText('Test Post Alpha', { exact: true })).toBeVisible();
    await expect(page.getByText('Binary File [1/3]')).toBeVisible();
  });

  test('header search filters results', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.group-item', { hasText: 'alt.test' }).click();
    await expect(page.getByText('Test Post Alpha', { exact: true })).toBeVisible();

    await page.locator('.search-box input').fill('Binary');
    await page.locator('.search-box input').press('Enter');

    await expect(page.locator('.header-row')).toHaveCount(3);
    await expect(page.getByText('Test Post Alpha', { exact: true })).not.toBeVisible();
  });

  test('checkbox selection shows download bar', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.group-item', { hasText: 'alt.test' }).click();
    await expect(page.getByText('Binary File [1/3]')).toBeVisible();

    // Select first checkbox
    await page.locator('.header-row').nth(0).locator('input[type="checkbox"]').check();
    await expect(page.locator('.download-bar')).toBeVisible();
    await expect(page.getByText('1 selected')).toBeVisible();

    // Select all
    await page.locator('.header-table-head input[type="checkbox"]').check();
    await expect(page.getByText('5 selected')).toBeVisible();

    // Download Selected button visible
    await expect(page.getByText('Download Selected')).toBeVisible();
  });

  test('settings tab shows servers', async ({ page }) => {
    await page.goto('/settings');
    await expect(page.getByText('News Servers')).toBeVisible();
  });

  test('history tab loads', async ({ page }) => {
    await page.goto('/history');
    await expect(page.getByText('Download History', { exact: true })).toBeVisible();
  });

  test('logs tab loads', async ({ page }) => {
    await page.goto('/logs');
    // Should have at least some startup logs
    await page.waitForTimeout(3000);
    // Just verify the page rendered
    await expect(page.locator('.log-container')).toBeVisible();
  });

  test('tabs navigate correctly', async ({ page }) => {
    await page.goto('/queue');
    await page.getByRole('link', { name: 'Groups' }).click();
    await expect(page).toHaveURL(/\/groups/);
    await page.getByRole('link', { name: 'Settings' }).click();
    await expect(page).toHaveURL(/\/settings/);
    await page.getByRole('link', { name: 'Queue' }).click();
    await expect(page).toHaveURL(/\/queue/);
  });

  test('status bar shows info', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.statusbar')).toBeVisible();
    await expect(page.locator('.status-ok')).toContainText('Connected');
  });

  test('groups API returns seeded data', async ({ request }) => {
    const r = await request.get('/api/groups?subscribed=true');
    const data = await r.json();
    expect(data.total).toBe(2);
    expect(data.groups[0].name).toBeTruthy();
  });

  test('headers API returns seeded data', async ({ request }) => {
    const r = await request.get('/api/groups/1/headers');
    const data = await r.json();
    expect(data.total).toBe(5);
    expect(data.headers.length).toBe(5);
  });

  test('group status API works', async ({ request }) => {
    const r = await request.get('/api/groups/1/status');
    const data = await r.json();
    expect(data.group_id).toBe(1);
    expect(data.new_available).toBe(50);
  });
});
