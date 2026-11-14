/* Browser regression test using an isolated context and signed, public test fixtures.
 * Requires Playwright and Chromium. Build: (cd web && trunk build --features web).
 * Run: node tests/login_metadata_browser.cjs
 * Optional: ARCADESTR_WEB_BUILD=/path/to/dist ARCADESTR_CHROMIUM=/usr/bin/chromium
 * No real extension, credentials, relay traffic, or application data is used.
 */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const fixture = require('./fixtures/login_metadata.json');
const root = process.env.ARCADESTR_WEB_BUILD || path.join(__dirname, '../web/dist');
assert.ok(fs.existsSync(path.join(root, 'index.html')), 'Build the standalone web frontend first');

const server = http.createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const file = path.join(root, pathname === '/' ? 'index.html' : pathname);
  try {
    response.setHeader('Content-Type', {
      '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css',
    }[path.extname(file)] || 'application/octet-stream');
    response.end(fs.readFileSync(file));
  } catch { response.writeHead(404); response.end(); }
});

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({
      headless: true,
      ...(process.env.ARCADESTR_CHROMIUM ? { executablePath: process.env.ARCADESTR_CHROMIUM } : {}),
    });
    const context = await browser.newContext();
    await context.addInitScript(({ fixture }) => {
      window.__profileTestResponses = 0;
      window.nostr = {
        async getPublicKey() {
          if (this !== window.nostr) throw new Error('NIP-07 provider receiver was lost');
          return fixture.pubkey;
        },
      };
      window.WebSocket = class {
        constructor() {
          this.readyState = 0;
          setTimeout(() => { this.readyState = 1; this.onopen?.(new Event('open')); }, 10);
        }
        send(raw) {
          const request = JSON.parse(raw);
          if (request[0] !== 'REQ') return;
          const mode = sessionStorage.getItem('profile-test-mode') || 'initial';
          setTimeout(() => {
            if (this.readyState !== 1) return;
            if (mode !== 'empty') {
              const event = mode === 'updated' ? fixture.updated : fixture.event;
              this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(['EVENT', request[1], event]) }));
              window.__profileTestResponses++;
            }
            this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(['EOSE', request[1]]) }));
          }, 50);
        }
        close() { this.readyState = 3; }
      };
    }, { fixture });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('https://**/*', route => route.abort());
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.getByRole('button', { name: 'Connect browser extension', exact: true }).click();
    await page.getByText('Cached Player', { exact: true }).first().waitFor();
    const saved = () => page.evaluate(npub => JSON.parse(localStorage.getItem(`arcadestr.web.profile.${npub}`)), fixture.npub);
    assert.equal((await saved()).id, fixture.event.id, 'first login saves signed metadata');

    await page.evaluate(() => sessionStorage.setItem('profile-test-mode', 'empty'));
    await page.reload();
    await page.getByText('Cached Player', { exact: true }).first().waitFor();
    assert.equal((await saved()).id, fixture.event.id, 'offline reopen preserves old metadata');

    await page.evaluate(() => sessionStorage.setItem('profile-test-mode', 'updated'));
    await page.reload();
    await page.getByText('Updated Player', { exact: true }).first().waitFor();
    assert.equal((await saved()).id, fixture.updated.id, 'background refresh updates the active profile');

    await page.evaluate(() => {
      sessionStorage.setItem('profile-test-mode', 'initial');
      const key = 'arcadestr.web.accounts.v1';
      const accounts = JSON.parse(localStorage.getItem(key));
      accounts.accounts.forEach(account => { account.is_current = false; });
      localStorage.setItem(key, JSON.stringify(accounts));
    });
    await page.reload();
    await page.getByText('Choose an account', { exact: true }).waitFor();
    await page.getByText('Updated Player', { exact: true }).first().waitFor();
    await page.getByRole('button', { name: 'Switch', exact: true }).click();
    await page.getByText('Updated Player', { exact: true }).first().waitFor();
    // Wait on the relay response, not just the already-rendered local snapshot.
    await page.waitForFunction(() => window.__profileTestResponses > 0);
    assert.equal((await saved()).id, fixture.updated.id, 'stale relay data cannot roll back metadata');
    assert.deepEqual(errors, [], 'no browser exceptions during login, refresh, restore, or switching');
    console.log('PASS: first login, offline reopen, background refresh, account picker, stale-event protection');
  } finally {
    if (browser) await browser.close();
    server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
