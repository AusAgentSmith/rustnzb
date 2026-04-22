import { test, expect } from '@playwright/test';

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe('11. Navigation & Shell', () => {
  // ── 11.1 All nav links present when authenticated ─────────────────────────

  test('11.1 all nav tabs present when authenticated', async ({ page }) => {
    await page.goto('/');

    // Should redirect to /queue
    await expect(page).toHaveURL(/\/queue/);

    // All primary nav links must be visible
    await expect(page.getByRole('link', { name: 'Queue' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'History' })).toBeVisible();
    await expect(page.getByRole('link', { name: /Groups|Search/i })).toBeVisible();
    await expect(page.getByRole('link', { name: 'RSS' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Logs' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Settings' })).toBeVisible();
  });

  // ── 11.2 Active tab highlighted ───────────────────────────────────────────

  test('11.2 active tab is highlighted on /history', async ({ page }) => {
    await page.goto('/history');

    const historyLink = page.getByRole('link', { name: 'History' });
    await expect(historyLink).toBeVisible();

    // The active link should have an "active" class (Angular router adds routerLinkActive)
    await expect(historyLink).toHaveClass(/active/);

    // Queue link should NOT be active
    const queueLink = page.getByRole('link', { name: 'Queue' });
    await expect(queueLink).not.toHaveClass(/active/);

    // Settings link should NOT be active
    const settingsLink = page.getByRole('link', { name: 'Settings' });
    await expect(settingsLink).not.toHaveClass(/active/);
  });

  // ── 11.3 Status bar visible with connection/daemon info ───────────────────

  test('11.3 status bar shows daemon state pills', async ({ page }) => {
    await page.goto('/queue');

    // The status bar contains .pill elements
    const pills = page.locator('.pill');
    await expect(pills.first()).toBeVisible();

    // At least one pill should contain connection/daemon state text
    const statusText = await pills.allInnerTexts();
    const hasConnectionState = statusText.some((t) =>
      /Daemon running|Connected|Paused|Connecting|Live/i.test(t),
    );
    expect(hasConnectionState).toBeTruthy();
  });

  // ── 11.4 Tab links navigate to the correct routes ─────────────────────────

  test('11.4 clicking nav links changes route', async ({ page }) => {
    await page.goto('/queue');

    // Queue → History
    await page.getByRole('link', { name: 'History' }).click();
    await expect(page).toHaveURL(/\/history/);

    // History → Settings
    await page.getByRole('link', { name: 'Settings' }).click();
    await expect(page).toHaveURL(/\/settings/);

    // Settings → Queue
    await page.getByRole('link', { name: 'Queue' }).click();
    await expect(page).toHaveURL(/\/queue/);
  });
});
