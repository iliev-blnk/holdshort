import { expect, test } from '@playwright/test';

test('replays the fixture day', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));

  await page.goto('/?t=06:05');
  await expect(page.locator('canvas.maplibregl-canvas')).toBeVisible();
  await expect(page.locator('#top .caption')).toContainText('2 Oct 2026');

  // Playback starts on its own and moves the clock (60×: a minute per second).
  const clock = page.locator('#panel .clock');
  await expect(clock).toHaveText(/^06:0\d$/);
  const first = await clock.textContent();
  await expect.poll(() => clock.textContent(), { timeout: 5_000 }).not.toBe(first);

  // Pause, then pick an event from the feed: it selects that flight.
  await page.goto('/?t=06:20');
  await page.keyboard.press('Space');
  await page.locator('#panel .feed li', { hasText: 'THY1 landed 35R' }).click();
  await expect(page.locator('#card .cs')).toHaveText('THY1');
  await expect(page.locator('#card')).toContainText('35R');
  await expect(page).toHaveURL(/f=4bb208-1/);

  expect(errors).toEqual([]);
});
