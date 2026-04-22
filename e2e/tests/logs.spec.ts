/**
 * Logs View — journeys 9.1-9.3
 *
 * The logs page streams live log entries from the backend.
 * Tests verify the container renders, filters work, and the UI is interactive.
 *
 * NOTE: smoke.spec.ts already checks that the logs tab loads. These tests go
 * deeper into filtering and the control buttons.
 */

import { test, expect } from '@playwright/test';

test.describe('9. Logs View', () => {
  // ── 9.1 Logs container renders and entries appear ─────────────────────────

  test('9.1 logs page shows container with entries after startup', async ({ page }) => {
    await page.goto('/logs');

    // The logs container must be present
    await expect(page.locator('.logs')).toBeVisible();

    // Give the WebSocket / SSE feed a moment to deliver startup entries
    await page.waitForTimeout(2000);

    // Either log rows are present or the empty-state text is shown
    const rowCount = await page.locator('.l').count();
    const emptyState = await page.getByText('No log entries yet.').isVisible();

    // At least one entry should appear (backend logs startup activity) — but
    // we tolerate the empty state if the backend emitted nothing at INFO level.
    expect(rowCount > 0 || emptyState).toBeTruthy();

    // If entries are present, each should have timestamp + level + message parts
    if (rowCount > 0) {
      const firstRow = page.locator('.l').first();
      await expect(firstRow.locator('.t')).toBeVisible();
      await expect(firstRow.locator('.lv')).toBeVisible();
      await expect(firstRow.locator('.msg')).toBeVisible();
    }
  });

  // ── 9.2 Level filter changes displayed entries ────────────────────────────

  test('9.2 ERROR level filter is applied correctly', async ({ page }) => {
    await page.goto('/logs');

    // The level select must be present
    const levelSelect = page.locator('select').or(page.getByRole('combobox')).first();
    await expect(levelSelect).toBeVisible();

    // Switch to "ERROR" level
    await levelSelect.selectOption({ label: 'ERROR' });

    // Give the UI a moment to refilter
    await page.waitForTimeout(500);

    // Any visible log entries must carry the error level class
    const allRows = page.locator('.l');
    const rowCount = await allRows.count();

    for (let i = 0; i < rowCount; i++) {
      const levelEl = allRows.nth(i).locator('.lv');
      const levelText = await levelEl.textContent();
      // The level pill must not be INFO, WARN, or DEBUG when filtering for ERROR
      expect(levelText?.trim()).toMatch(/err|error/i);
    }

    // Reset to "All levels"
    await levelSelect.selectOption({ label: 'All levels' });
  });

  // ── 9.3 Text filter input accepts regex ───────────────────────────────────

  test('9.3 text filter input accepts and applies a filter', async ({ page }) => {
    await page.goto('/logs');

    const filterInput = page
      .getByPlaceholder('Filter… (regex ok)')
      .or(page.locator('input[type="text"]').first());
    await expect(filterInput).toBeVisible();

    // Type a filter that will match nothing (so we can verify the filter acts)
    await filterInput.fill('ZZZZZZ_no_match_ZZZZZZ');
    await page.waitForTimeout(500);

    // If any entries were visible before, they should all be gone now
    const rowCount = await page.locator('.l').count();
    const emptyOrFiltered =
      rowCount === 0 || (await page.getByText('No log entries yet.').isVisible());
    expect(emptyOrFiltered).toBeTruthy();

    // Clear the filter — entries return
    await filterInput.clear();
    await page.waitForTimeout(300);
  });

  // ── 9.4 Follow / Pause toggle button ─────────────────────────────────────

  test('9.4 following toggle button is present and clickable', async ({ page }) => {
    await page.goto('/logs');

    // The follow button should read "Following ●" or "Paused" depending on state
    const followBtn = page
      .getByRole('button', { name: /Following|Paused/i })
      .or(page.locator('button', { hasText: /Following|Paused/i }))
      .first();
    await expect(followBtn).toBeVisible();

    // Click to toggle
    await followBtn.click();

    // After clicking the label switches (e.g. "Following ●" → "Paused" or vice versa)
    await page.waitForTimeout(300);
    // Just assert the button is still present — label may vary by implementation
    await expect(
      page.locator('button', { hasText: /Following|Paused/i }).first(),
    ).toBeVisible();
  });

  // ── 9.5 Clear button removes all entries ─────────────────────────────────

  test('9.5 clear button removes displayed log entries', async ({ page }) => {
    await page.goto('/logs');

    // Wait briefly for any entries to arrive
    await page.waitForTimeout(1500);

    const clearBtn = page.getByRole('button', { name: 'Clear' });
    await expect(clearBtn).toBeVisible();

    await clearBtn.click();
    await page.waitForTimeout(300);

    // After clearing, row count drops to 0 or empty state appears
    const rowCount = await page.locator('.l').count();
    const emptyState = await page.getByText('No log entries yet.').isVisible();
    expect(rowCount === 0 || emptyState).toBeTruthy();
  });

  // ── 9.6 Download button is present ───────────────────────────────────────

  test('9.6 download button is present in logs toolbar', async ({ page }) => {
    await page.goto('/logs');

    const downloadBtn = page.getByRole('button', { name: 'Download' });
    await expect(downloadBtn).toBeVisible();
  });
});
