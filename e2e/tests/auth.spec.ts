/**
 * Authentication — journeys 10.1 through 10.5
 *
 * These tests run serially against the fresh backend (port 9191).
 * They depend on first-boot.spec.ts having already run (credentials for
 * testadmin/testpassword123 exist on port 9191 before this file executes).
 *
 * Because Playwright's "fresh" project runs first-boot.spec.ts and auth.spec.ts
 * in file-alphabetical order within the same worker (workers: 1, fullyParallel:
 * false), first-boot always precedes auth. The credential state is backend-side
 * persistence — it does not depend on browser storage carried across files.
 */

import { test, expect, Page } from '@playwright/test';
import { FRESH_URL, TEST_USER, TEST_PASS } from '../helpers/api';

/** Inject a valid access token into localStorage before the page loads. */
async function injectToken(page: Page, token: string): Promise<void> {
  await page.addInitScript((tok: string) => {
    localStorage.setItem('access_token', tok);
    localStorage.setItem('refresh_token', 'dummy');
  }, token);
}

/** Obtain a fresh access token via the login API. */
async function apiLogin(username = TEST_USER, password = TEST_PASS): Promise<string> {
  const r = await fetch(`${FRESH_URL}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });
  if (!r.ok) throw new Error(`Login API call failed: ${r.status} ${await r.text()}`);
  const body = await r.json();
  if (!body.access_token) throw new Error('No access_token in login response');
  return body.access_token as string;
}

test.describe.serial('Authentication', () => {
  // ── 10.1 Protected route /queue redirects to /login without a token ─────────
  test('10.1 protected route redirects to /login when unauthenticated', async ({ page }) => {
    // Navigate directly — no token in localStorage
    await page.goto('/queue');
    await expect(page).toHaveURL(/\/login/);
  });

  // ── 10.2 Wrong password shows error ────────────────────────────────────────
  test('10.2 wrong password shows invalid credentials error', async ({ page }) => {
    await page.goto('/login');

    // Credentials exist now so the login form should be in "Sign In" mode
    await expect(page.getByRole('button', { name: 'Sign In' })).toBeVisible();

    await page.getByLabel(/^username/i).fill(TEST_USER);
    await page.getByLabel(/^password$/i).fill('wrong-password-xyz');
    await page.getByRole('button', { name: 'Sign In' }).click();

    await expect(page.getByText(/invalid username or password/i)).toBeVisible();
    await expect(page).toHaveURL(/\/login/);
  });

  // ── 10.3 Valid login → /queue with nav tabs visible ─────────────────────────
  test('10.3 valid login lands on /queue with nav tabs', async ({ page }) => {
    await page.goto('/login');
    await expect(page.getByRole('button', { name: 'Sign In' })).toBeVisible();

    await page.getByLabel(/^username/i).fill(TEST_USER);
    await page.getByLabel(/^password$/i).fill(TEST_PASS);
    await page.getByRole('button', { name: 'Sign In' }).click();

    // After login the app redirects — either to /welcome (no servers) or /queue.
    // The fresh backend has no servers so /welcome is shown; skip to /queue.
    const url = page.url();
    if (/\/welcome/.test(url)) {
      await page.getByText(/skip for now/i).click();
    }

    await expect(page).toHaveURL(/\/queue/);

    // Nav tabs should all be present for an authenticated user
    await expect(page.getByRole('link', { name: 'Queue' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'History' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Settings' })).toBeVisible();
  });

  // ── 10.4 Logout clears session → redirected to /login ───────────────────────
  test('10.4 logout clears session and subsequent /queue access redirects to /login', async ({ page }) => {
    // Start authenticated
    const token = await apiLogin();
    await injectToken(page, token);
    await page.goto('/queue');

    // May land on /welcome if no servers — navigate to /queue via skip
    const currentUrl = page.url();
    if (/\/welcome/.test(currentUrl)) {
      await page.getByText(/skip for now/i).click();
    }
    await expect(page).toHaveURL(/\/queue/);

    // Find and click the logout button (could be an icon, link, or menu item)
    const logoutButton = page
      .getByRole('button', { name: /log.?out|sign.?out/i })
      .or(page.getByRole('link', { name: /log.?out|sign.?out/i }))
      .or(page.getByTitle(/log.?out|sign.?out/i))
      .first();
    await logoutButton.click();

    // Should be back at /login
    await expect(page).toHaveURL(/\/login/);

    // Verify session is actually gone: navigating to /queue should redirect again.
    // NB: injectToken() used addInitScript which re-runs on every navigation, so
    // queue the opposite (clear storage) to simulate a real logged-out user.
    await page.addInitScript(() => {
      localStorage.removeItem('access_token');
      localStorage.removeItem('refresh_token');
    });
    await page.goto('/queue');
    await expect(page).toHaveURL(/\/login/);
  });

  // ── 10.5 Session persists on reload ─────────────────────────────────────────
  test('10.5 session persists on page reload', async ({ page }) => {
    const token = await apiLogin();
    await injectToken(page, token);
    await page.goto('/queue');

    // Handle possible /welcome redirect for no-server state
    const initialUrl = page.url();
    if (/\/welcome/.test(initialUrl)) {
      await page.getByText(/skip for now/i).click();
      await expect(page).toHaveURL(/\/queue/);
    }

    // Reload and confirm we stay authenticated (no redirect to /login)
    await page.reload();
    await expect(page).not.toHaveURL(/\/login/);

    // The authenticated shell should still be visible
    await expect(page.getByRole('link', { name: 'Queue' })).toBeVisible();
  });
});
