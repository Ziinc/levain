// Stock @playwright/test driving the levain-echo host through gpui-playwright.
// No browser is launched: the app itself is the Playwright server. Build it
// first: `cargo build -p levain-echo --features playwright`.
const { defineConfig } = require('@playwright/test');
const path = require('path');

const port = process.env.GPUI_PLAYWRIGHT_PORT ?? '9333';
const exe = process.platform === 'win32' ? 'levain-echo.exe' : 'levain-echo';
const bin = process.env.LEVAIN_ECHO ?? path.resolve(__dirname, '..', 'target', 'debug', exe);

module.exports = defineConfig({
  testDir: './tests',
  // One app, one window.
  workers: 1,
  fullyParallel: false,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  expect: { timeout: 10_000 },
  use: { connectOptions: { wsEndpoint: `ws://127.0.0.1:${port}/` } },
  webServer: {
    command: `"${bin}"`,
    url: `http://127.0.0.1:${port}/`,
    // TAO's portal colour-scheme lookup stalls without a session bus.
    env: { GPUI_PLAYWRIGHT: port, DBUS_SESSION_BUS_ADDRESS: 'unix:path=/nonexistent/bus' },
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    stdout: 'pipe',
    stderr: 'pipe',
  },
});
