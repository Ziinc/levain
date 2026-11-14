const { test, expect } = require('@playwright/test');

test('clicks reach the runtime and the count re-renders', async ({ page }) => {
  await expect(page.getByText('Count: 0')).toBeVisible();
  await page.getByRole('button', { name: '+' }).click();
  await page.getByRole('button', { name: '+' }).click();
  await expect(page.getByText('Count: 2')).toBeVisible();
  await page.getByRole('button', { name: '-' }).click();
  await expect(page.getByText('Count: 1')).toBeVisible();
});
