// The headless path of the echo host: `levain-echo sum 2 3` runs as a command,
// prints, exits with a status, and never opens a window. Plain `node --test`:
// no Playwright server is involved. Build the host first:
// `cargo build -p levain-echo --features playwright`.
const test = require('node:test');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const exe = process.platform === 'win32' ? 'levain-echo.exe' : 'levain-echo';
const bin = process.env.LEVAIN_ECHO ?? path.resolve(__dirname, '..', '..', 'target', 'debug', exe);

function echo(...args) {
  const env = { ...process.env, DBUS_SESSION_BUS_ADDRESS: 'unix:path=/nonexistent/bus' };
  // A Playwright server would mean a window was wanted.
  delete env.GPUI_PLAYWRIGHT;
  const started = Date.now();
  const run = spawnSync(bin, args, { env, encoding: 'utf8', timeout: 30_000 });
  return { ...run, ms: Date.now() - started };
}

// `levain: first frame` is logged when the app first renders to a window.
const rendered = (run) => run.stderr.includes('levain: first frame');

test('sum prints the total, exits 0, and opens no window', () => {
  const run = echo('sum', '2', '3');
  assert.equal(run.signal, null, run.stderr);
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.stdout, '5\n');
  assert.equal(rendered(run), false, run.stderr);
  assert.ok(run.ms < 20_000, `took ${run.ms} ms`);
});

test('--json prints an object', () => {
  const run = echo('sum', '2', '3', '--json');
  assert.equal(run.status, 0, run.stderr);
  assert.deepEqual(JSON.parse(run.stdout), { sum: 5 });
  assert.equal(rendered(run), false, run.stderr);
});

test('numbers that are not numbers fail with code 2 on stderr', () => {
  const run = echo('sum', 'x', 'y');
  assert.equal(run.status, 2, run.stderr);
  assert.equal(run.stdout, '');
  assert.match(run.stderr, /not a number: x/);
  assert.equal(rendered(run), false, run.stderr);
});

test('arguments the CLI does not accept fail with code 2', () => {
  const run = echo('sum', '2');
  assert.equal(run.status, 2, run.stderr);
  assert.match(run.stderr, /failed to parse arguments/);
  assert.equal(rendered(run), false, run.stderr);
});

test('--help prints the usage and exits 0', () => {
  const run = echo('--help');
  assert.equal(run.status, 0, run.stderr);
  assert.match(run.stdout, /sum/);
  assert.equal(rendered(run), false, run.stderr);
});
