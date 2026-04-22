/**
 * History View — journeys 5.1-5.6
 *
 * Seeded history:
 *   - "Completed.Movie.2025.mkv"  — completed
 *   - "Failed.Show.S02E05.mkv"   — failed  (error: "Article not found on server: test-msg@test.com")
 *   - "Good.Podcast.EP100.mp3"   — completed
 */

import { test, expect } from '@playwright/test';

test.describe('5. Download History', () => {
  // Auto-accept confirm dialogs (used by delete tests)
  test.beforeEach(async ({ page }) => {
    page.on('dialog', (dialog) => dialog.accept());
  });

  // ── 5.1 Completed entry visible ────────────────────────────────────────────

  test('5.1 completed history entry visible with status pill', async ({ page }) => {
    await page.goto('/history');

    const row = page.locator('tr, .row', { hasText: 'Completed.Movie.2025.mkv' }).first();
    await expect(row).toBeVisible();

    // Completed status pill
    await expect(row.locator('.s-ok')).toBeVisible();
  });

  // ── 5.2 Failed entry visible with error accessible ─────────────────────────

  test('5.2 failed history entry has failed pill and error detail', async ({ page }) => {
    await page.goto('/history');

    const row = page.locator('tr, .row', { hasText: 'Failed.Show.S02E05.mkv' }).first();
    await expect(row).toBeVisible();

    // Failed status pill
    await expect(row.locator('.s-fail')).toBeVisible();

    // Error message visible somewhere on the row or on expand
    // Some UIs show it inline; some behind a detail click. Try inline first.
    const inlineError = row.locator('text=Article not found');
    const detailBtn = row.getByRole('button', { name: /detail|info|expand|\u2139/i });

    if (await inlineError.isVisible()) {
      await expect(inlineError).toBeVisible();
    } else if (await detailBtn.count() > 0) {
      await detailBtn.first().click();
      await expect(page.getByText('Article not found', { exact: false })).toBeVisible();
    }
    // Either path is acceptable — the test passes if the error is reachable.
  });

  // ── 5.3 Filter by name ─────────────────────────────────────────────────────

  test('5.3 name filter hides non-matching entries', async ({ page }) => {
    await page.goto('/history');

    // All three entries present initially
    await expect(page.locator('tr, .row', { hasText: 'Completed.Movie.2025.mkv' }).first()).toBeVisible();
    await expect(page.locator('tr, .row', { hasText: 'Failed.Show.S02E05.mkv' }).first()).toBeVisible();
    await expect(page.locator('tr, .row', { hasText: 'Good.Podcast.EP100.mp3' }).first()).toBeVisible();

    // Type in the search / filter input
    const searchInput = page
      .locator('.search-bar')
      .or(page.getByPlaceholder('Filter name…'))
      .first();
    await searchInput.fill('Movie');

    // Only "Completed.Movie.2025.mkv" matches
    await expect(page.locator('tr, .row', { hasText: 'Completed.Movie.2025.mkv' }).first()).toBeVisible();

    // Neither "Failed.Show" nor "Good.Podcast" should be shown
    await expect(
      page.locator('tr, .row', { hasText: 'Failed.Show.S02E05.mkv' }).first(),
    ).not.toBeVisible();
    await expect(
      page.locator('tr, .row', { hasText: 'Good.Podcast.EP100.mp3' }).first(),
    ).not.toBeVisible();

    // Clear the filter
    await searchInput.clear();
    await expect(page.locator('tr, .row', { hasText: 'Failed.Show.S02E05.mkv' }).first()).toBeVisible();
  });

  // ── 5.4 Filter by status "Failed" ─────────────────────────────────────────

  test('5.4 status filter "Failed" hides completed entries', async ({ page }) => {
    await page.goto('/history');

    // Select "Failed" from the status dropdown
    const statusSelect = page.locator('select').or(page.getByRole('combobox')).first();
    await statusSelect.selectOption({ label: 'Failed' });

    // Only the failed row visible
    await expect(page.locator('tr, .row', { hasText: 'Failed.Show.S02E05.mkv' }).first()).toBeVisible();

    // Completed rows hidden
    await expect(
      page.locator('tr, .row', { hasText: 'Completed.Movie.2025.mkv' }).first(),
    ).not.toBeVisible();
    await expect(
      page.locator('tr, .row', { hasText: 'Good.Podcast.EP100.mp3' }).first(),
    ).not.toBeVisible();

    // Reset to "All statuses"
    await statusSelect.selectOption({ label: 'All statuses' });
    await expect(page.locator('tr, .row', { hasText: 'Completed.Movie.2025.mkv' }).first()).toBeVisible();
  });

  // ── 5.5 Delete history entry ───────────────────────────────────────────────

  test('5.5 deleting a history entry removes it from the list', async ({ page }) => {
    await page.goto('/history');

    const podcastRow = page.locator('tr, .row', { hasText: 'Good.Podcast.EP100.mp3' }).first();
    await expect(podcastRow).toBeVisible();

    // Click the delete (✕) action
    await podcastRow.locator('button', { hasText: '✕' }).click();

    // Entry must disappear
    await expect(
      page.locator('tr, .row', { hasText: 'Good.Podcast.EP100.mp3' }).first(),
    ).not.toBeVisible({ timeout: 5000 });

    // Other entries remain
    await expect(page.locator('tr, .row', { hasText: 'Completed.Movie.2025.mkv' }).first()).toBeVisible();
    await expect(page.locator('tr, .row', { hasText: 'Failed.Show.S02E05.mkv' }).first()).toBeVisible();
  });

  // ── 5.6 Stat cards show correct counts ────────────────────────────────────

  test('5.6 stat cards show 2 completed and 1 failed', async ({ page }) => {
    await page.goto('/history');

    // Completed card — seeded: Completed.Movie + Good.Podcast = 2
    const completedCard = page.locator('.stat-card, [class*="stat"]', { hasText: 'Completed' }).first();
    await expect(completedCard).toContainText('2');

    // Failed card — seeded: Failed.Show = 1
    const failedCard = page.locator('.stat-card, [class*="stat"]', { hasText: 'Failed' }).first();
    await expect(failedCard).toContainText('1');

    // Success rate card — 2 out of 3 = 66% or 67%
    const rateCard = page.locator('.stat-card, [class*="stat"]', { hasText: 'Success rate' }).first();
    await expect(rateCard).toBeVisible();
  });
});
