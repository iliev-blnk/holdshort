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

test('a day that fails to load leaves nothing of the previous day in the panel', async ({ page }) => {
  // Offer a second day whose files don't exist.
  await page.route('**/fixture/index.json', async (route) => {
    const r = await route.fetch();
    const idx = await r.json();
    idx.days.push({ ...idx.days[0], date: '2026-10-03' });
    await route.fulfill({ response: r, json: idx });
  });
  await page.goto('/?t=06:20');
  await page.keyboard.press('Space');
  await expect(page.locator('#panel .feed li')).toHaveCount(3);

  await page.selectOption('#top select', '2026-10-03');
  await expect(page.locator('#notice')).toHaveText('This day could not be loaded.');
  await expect(page.locator('#panel .feed li')).toHaveCount(0);
  await expect(page.locator('#panel .config')).toHaveText('—');
  await expect(page.locator('#panel .s-hour')).toHaveText('0');
});
