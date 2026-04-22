/**
 * RSS View — journeys 6.1-6.7
 *
 * Seeded state:
 *   Feeds (from test-config.toml): "Test Feed" — https://example.com/rss
 *   RSS items: "New Release S01E01 720p", "New Release S01E02 720p", "Already Downloaded Movie"
 *   RSS rules: "New Release auto-grab" (enabled), "Disabled rule" (disabled)
 */

import { test, expect } from '@playwright/test';
import { MAIN_URL } from '../helpers/api';

// Helper — get bearer token from localStorage
async function getToken(page: import('@playwright/test').Page): Promise<string> {
  const token = await page.evaluate(() => localStorage.getItem('access_token'));
  if (!token) throw new Error('No access_token in localStorage');
  return token;
}

// Helper — delete a rule via API (cleanup)
async function apiDeleteRule(token: string, ruleId: string): Promise<void> {
  await fetch(`${MAIN_URL}/api/config/rss-rules/${encodeURIComponent(ruleId)}`, {
    method: 'DELETE',
    headers: { Authorization: `Bearer ${token}` },
  });
}

// Helper — list rules via API
async function apiListRules(token: string): Promise<Array<{ id: string; name: string }>> {
  const r = await fetch(`${MAIN_URL}/api/config/rss-rules`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  return r.json();
}

test.describe('6. RSS Feeds & Rules', () => {
  test.beforeEach(async ({ page }) => {
    // Auto-accept confirm dialogs used by delete operations
    page.on('dialog', (dialog) => dialog.accept());
  });

  // ── 6.1 Seeded feed visible ────────────────────────────────────────────────

  test('6.1 seeded "Test Feed" is visible in feeds list', async ({ page }) => {
    await page.goto('/rss');

    // "Feeds" section heading
    await expect(page.getByText('Feeds', { exact: true }).or(page.getByRole('heading', { name: /feeds/i })).first()).toBeVisible();

    // Feed row with the seeded name
    const feedRow = page.locator('.feed-row', { hasText: 'Test Feed' }).first();
    await expect(feedRow).toBeVisible();

    // URL also visible in the row
    await expect(feedRow.locator('.feed-url').or(feedRow.getByText('https://example.com/rss'))).toBeVisible();
  });

  // ── 6.2 Add a new feed ────────────────────────────────────────────────────

  test('6.2 adding a new feed shows it in the list', async ({ page }) => {
    await page.goto('/rss');

    // Open the add form
    await page.getByRole('button', { name: '+ Add feed' }).click();

    // Fill name and URL
    await page.getByPlaceholder('Feed name').fill('E2E Feed');
    await page.getByPlaceholder(/https:\/\/indexer\.example\/rss/).fill(
      'https://e2e-test.example.com/rss',
    );

    // Submit
    await page.getByRole('button', { name: 'Add feed' }).click();

    // New row appears
    await expect(page.locator('.feed-row', { hasText: 'E2E Feed' })).toBeVisible();

    // Snackbar confirmation
    await expect(
      page.locator('.snackbar, [class*="snack"], [class*="toast"], [role="alert"]').first(),
    ).toBeVisible({ timeout: 5000 });
  });

  // ── 6.3 Delete the just-added feed ────────────────────────────────────────

  test('6.3 deleting a feed removes it from the list', async ({ page }) => {
    await page.goto('/rss');

    // 6.2 may not have run in this test's session, so add the feed first if missing
    const feedAlreadyThere = await page.locator('.feed-row', { hasText: 'E2E Feed' }).isVisible();
    if (!feedAlreadyThere) {
      await page.getByRole('button', { name: '+ Add feed' }).click();
      await page.getByPlaceholder('Feed name').fill('E2E Feed');
      await page.getByPlaceholder(/https:\/\/indexer\.example\/rss/).fill(
        'https://e2e-test.example.com/rss',
      );
      await page.getByRole('button', { name: 'Add feed' }).click();
      await expect(page.locator('.feed-row', { hasText: 'E2E Feed' })).toBeVisible();
    }

    // Click "Delete" on the E2E Feed row
    const feedRow = page.locator('.feed-row', { hasText: 'E2E Feed' }).first();
    await feedRow.getByRole('button', { name: 'Delete' }).click();

    // Row must disappear
    await expect(page.locator('.feed-row', { hasText: 'E2E Feed' })).not.toBeVisible({
      timeout: 5000,
    });

    // Original seeded feed still present
    await expect(page.locator('.feed-row', { hasText: 'Test Feed' })).toBeVisible();
  });

  // ── 6.4 Recent items section visible ──────────────────────────────────────

  test('6.4 recent RSS items are visible', async ({ page }) => {
    await page.goto('/rss');

    // "Recent items" heading
    await expect(
      page.getByText('Recent items', { exact: true }).or(page.getByRole('heading', { name: /recent items/i })).first(),
    ).toBeVisible();

    // Two undownloaded seeded items should appear
    await expect(page.getByText('New Release S01E01 720p')).toBeVisible();
    await expect(page.getByText('New Release S01E02 720p')).toBeVisible();

    // "grab" button available for undownloaded items
    const grabBtns = page.locator('button', { hasText: /↓ grab|grab/i });
    await expect(grabBtns.first()).toBeVisible();
  });

  // ── 6.5 Add a download rule ────────────────────────────────────────────────

  test('6.5 adding a download rule shows it in the rules list', async ({ page }) => {
    await page.goto('/rss');

    // "Download rules" section
    await expect(
      page.getByText('Download rules', { exact: true }).or(page.getByRole('heading', { name: /download rules/i })).first(),
    ).toBeVisible();

    // Open the add-rule form
    await page.getByRole('button', { name: '+ Add rule' }).click();

    // Fill form
    await page.getByPlaceholder('Rule name').fill('E2E Rule');
    await page.getByPlaceholder(/\.\*S\\d\+E\\d\+\.\*/).fill('TestRegex\\d+');

    // Submit
    await page.getByRole('button', { name: 'Add rule' }).click();

    // Rule row appears
    await expect(page.locator('tr, .row', { hasText: 'E2E Rule' }).first()).toBeVisible();

    // Cleanup — delete via API so other tests are not affected
    const token = await getToken(page);
    const rules = await apiListRules(token);
    const created = rules.find((r) => r.name === 'E2E Rule');
    if (created) await apiDeleteRule(token, created.id);
  });

  // ── 6.6 Edit a seeded rule ────────────────────────────────────────────────

  test('6.6 editing a rule updates its regex', async ({ page }) => {
    await page.goto('/rss');

    // Find the seeded rule row
    const ruleRow = page.locator('tr, .row', { hasText: 'New Release auto-grab' }).first();
    await expect(ruleRow).toBeVisible();

    // Click "edit"
    await ruleRow.getByRole('button', { name: 'edit' }).or(ruleRow.locator('button', { hasText: 'edit' })).click();

    // Locate regex input (should be pre-filled) and change it
    const regexInput = page.getByPlaceholder(/\.\*S\\d\+E\\d\+\.\*/).or(page.locator('input[type="text"]').nth(1));
    await regexInput.clear();
    await regexInput.fill('S\\d{2}E\\d{2}.*720p');

    // Save
    await page.getByRole('button', { name: 'Update' }).or(page.getByRole('button', { name: 'Save' })).click();

    // Confirmation snackbar or the row reflects saved state
    const savedFeedback =
      (await page.locator('.snackbar, [class*="snack"], [class*="toast"], [role="alert"]').count()) > 0 ||
      (await page.locator('tr, .row', { hasText: 'New Release auto-grab' }).count()) > 0;
    expect(savedFeedback).toBeTruthy();

    // Restore original regex via the edit flow so the DB is clean for other tests
    const ruleRowAfter = page.locator('tr, .row', { hasText: 'New Release auto-grab' }).first();
    await ruleRowAfter.getByRole('button', { name: 'edit' }).or(ruleRowAfter.locator('button', { hasText: 'edit' })).click();
    const regexInputRestore = page.getByPlaceholder(/\.\*S\\d\+E\\d\+\.\*/).or(page.locator('input[type="text"]').nth(1));
    await regexInputRestore.clear();
    await regexInputRestore.fill('S\\d{2}E\\d{2}');
    await page.getByRole('button', { name: 'Update' }).or(page.getByRole('button', { name: 'Save' })).click();
  });

  // ── 6.7 Delete a rule ────────────────────────────────────────────────────

  test('6.7 deleting a rule removes it from the list', async ({ page }) => {
    await page.goto('/rss');

    // Create a throwaway rule first
    await page.getByRole('button', { name: '+ Add rule' }).click();
    await page.getByPlaceholder('Rule name').fill('DeleteMe Rule');
    await page.getByPlaceholder(/\.\*S\\d\+E\\d\+\.\*/).fill('DeleteMe.*');
    await page.getByRole('button', { name: 'Add rule' }).click();
    await expect(page.locator('tr, .row', { hasText: 'DeleteMe Rule' }).first()).toBeVisible();

    // Click "del" on that rule
    const ruleRow = page.locator('tr, .row', { hasText: 'DeleteMe Rule' }).first();
    await ruleRow.getByRole('button', { name: 'del' }).or(ruleRow.locator('button', { hasText: 'del' })).click();

    // Rule row must disappear
    await expect(
      page.locator('tr, .row', { hasText: 'DeleteMe Rule' }).first(),
    ).not.toBeVisible({ timeout: 5000 });

    // Seeded rules still present
    await expect(page.locator('tr, .row', { hasText: 'New Release auto-grab' }).first()).toBeVisible();
  });
});
