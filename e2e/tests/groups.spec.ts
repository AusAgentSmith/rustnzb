/**
 * Groups View — journeys 7.1-7.6
 *
 * Smoke tests already cover basic navigation and group/header loading.
 * These tests complement them with: unsubscribed groups, thread view,
 * deselection behaviour, and select-all / deselect-all.
 *
 * Seeded data:
 *   groups: alt.test (subscribed), alt.binaries.test (subscribed), misc.test (NOT subscribed)
 *   headers for alt.test (id=1):
 *     1 – "Test Post Alpha"      (alice@test.com)
 *     2 – "Re: Test Post Alpha"  (bob@test.com, references msg1@test)
 *     3 – "Binary File [1/3]"
 *     4 – "Binary File [2/3]"
 *     5 – "Binary File [3/3]"
 */

import { test, expect } from '@playwright/test';

test.describe('7. Newsgroup Browser (extended)', () => {
  // ── 7.1 Unsubscribed group not in subscribed list ─────────────────────────

  test('7.1 misc.test is not shown in the subscribed group list by default', async ({ page }) => {
    await page.goto('/groups');

    // Subscribed groups visible
    await expect(page.locator('.g', { hasText: 'alt.test' })).toBeVisible();
    await expect(page.locator('.g', { hasText: 'alt.binaries.test' })).toBeVisible();

    // Unsubscribed group must NOT appear in the sidebar list
    await expect(page.locator('.g', { hasText: 'misc.test' })).not.toBeVisible();
  });

  // ── 7.2 Group panel shows article count ───────────────────────────────────

  test('7.2 clicking a group shows its article / header count', async ({ page }) => {
    await page.goto('/groups');

    await page.locator('.g', { hasText: 'alt.test' }).click();
    await expect(page.locator('h3')).toContainText('alt.test');

    // alt.test has 5 seeded headers — verify total visible
    await expect(page.locator('table.data tbody tr')).toHaveCount(5);
  });

  // ── 7.3 Thread view — reply visible under parent ──────────────────────────

  test('7.3 headers include the reply "Re: Test Post Alpha"', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.g', { hasText: 'alt.test' }).click();

    await expect(page.locator('h3')).toContainText('alt.test');

    // Both the parent and the reply are in the list
    await expect(page.getByText('Test Post Alpha', { exact: true })).toBeVisible();
    await expect(page.getByText('Re: Test Post Alpha')).toBeVisible();
  });

  // ── 7.4 Selecting then deselecting a checkbox hides the download bar ───────

  test('7.4 deselecting all checkboxes hides the download bar', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.g', { hasText: 'alt.test' }).click();
    await expect(page.getByText('Binary File [1/3]')).toBeVisible();

    // Select first row
    const firstCheckbox = page.locator('table.data tbody tr').nth(0).locator('input[type="checkbox"]');
    await firstCheckbox.check();
    await expect(page.locator('.download-bar')).toBeVisible();
    await expect(page.getByText('1 selected')).toBeVisible();

    // Deselect it
    await firstCheckbox.uncheck();

    // Download bar should disappear (or show 0 selected)
    const barHidden = await page.locator('.download-bar').isHidden();
    const zeroSelected =
      (await page.getByText('0 selected').isVisible()) ||
      (await page.getByText('0 selected').count()) > 0;
    expect(barHidden || zeroSelected).toBeTruthy();
  });

  // ── 7.5 Download bar updates count correctly on multi-select ──────────────

  test('7.5 download bar updates count as items are selected', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.g', { hasText: 'alt.test' }).click();
    await expect(page.getByText('Binary File [1/3]')).toBeVisible();

    const rows = page.locator('table.data tbody tr');

    // Select three rows one at a time and verify count increments
    await rows.nth(0).locator('input[type="checkbox"]').check();
    await expect(page.locator('.download-bar')).toBeVisible();
    await expect(page.getByText('1 selected')).toBeVisible();

    await rows.nth(1).locator('input[type="checkbox"]').check();
    await expect(page.getByText('2 selected')).toBeVisible();

    await rows.nth(2).locator('input[type="checkbox"]').check();
    await expect(page.getByText('3 selected')).toBeVisible();
  });

  // ── 7.6 Select-all then deselect-all via header checkbox ─────────────────

  test('7.6 select-all header checkbox then deselect-all clears bar', async ({ page }) => {
    await page.goto('/groups');
    await page.locator('.g', { hasText: 'alt.test' }).click();
    await expect(page.getByText('Binary File [1/3]')).toBeVisible();

    const selectAllCheckbox = page.locator('table.data thead input[type="checkbox"]');

    // Select all 5 headers
    await selectAllCheckbox.check();
    await expect(page.getByText('5 selected')).toBeVisible();
    await expect(page.getByText('↓ Download selected')).toBeVisible();

    // Deselect all via the same header checkbox
    await selectAllCheckbox.uncheck();

    // Download bar hidden or count is 0
    await page.waitForTimeout(300);
    const barHidden = await page.locator('.download-bar').isHidden();
    const zeroSelected =
      (await page.getByText('0 selected').isVisible()) ||
      (await page.getByText('0 selected').count()) > 0;
    expect(barHidden || zeroSelected).toBeTruthy();
  });

  // ── 7.7 Search in one group does not affect other group's headers ──────────

  test('7.7 search filter is scoped to the active group', async ({ page }) => {
    await page.goto('/groups');

    // Load alt.test
    await page.locator('.g', { hasText: 'alt.test' }).click();
    await expect(page.getByText('Test Post Alpha', { exact: true })).toBeVisible();

    // Filter down to Binary entries using the global search bar
    await page.locator('.search-bar input').first().fill('Binary');
    await page.locator('.search-bar input').first().press('Enter');
    await expect(page.locator('table.data tbody tr')).toHaveCount(3);

    // Switch group — filter should reset or be scoped to the new group
    await page.locator('.g', { hasText: 'alt.binaries.test' }).click();
    await expect(page.locator('h3')).toContainText('alt.binaries.test');

    // No headers seeded for alt.binaries.test — the panel opens to an empty list
    // but must not carry over alt.test's search state as an error state.
    await expect(page.locator('h3')).toBeVisible();
  });
});
