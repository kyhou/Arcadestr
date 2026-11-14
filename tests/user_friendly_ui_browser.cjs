/* UI regression: real WASM, isolated account, no external traffic or payments.
 * Build: cd web && trunk build --features web
 * Run: ARCADESTR_CHROMIUM=/usr/bin/chromium node tests/user_friendly_ui_browser.cjs
 */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const fixture = require('./fixtures/login_metadata.json');
const root = process.env.ARCADESTR_WEB_BUILD || path.join(__dirname, '../web/dist');
const server = http.createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const file = path.resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
  if (!file.startsWith(`${path.resolve(root)}${path.sep}`)) {
    response.writeHead(403); response.end(); return;
  }
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
    browser = await chromium.launch({ headless: true,
      ...(process.env.ARCADESTR_CHROMIUM ? { executablePath: process.env.ARCADESTR_CHROMIUM } : {}),
    });
    const context = await browser.newContext();
    await context.addInitScript(({ fixture }) => {
      window.nostr = { async getPublicKey() { return fixture.pubkey; } };
      window.WebSocket = class {
        constructor() {
          this.readyState = 0;
          setTimeout(() => { this.readyState = 1; this.onopen?.(new Event('open')); }, 0);
        }
        send(raw) {
          const request = JSON.parse(raw);
          if (request[0] !== 'REQ') return;
          setTimeout(() => {
            if (this.readyState !== 1) return;
            const filters = request.slice(2);
            if (filters.some(filter => filter.kinds?.includes(0))) {
              this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(['EVENT', request[1], fixture.event]) }));
            }
            this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(['EOSE', request[1]]) }));
          }, 0);
        }
        close() { this.readyState = 3; }
      };
    }, { fixture });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('https://**/*', route => route.abort());
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.getByRole('heading', { name: 'Sign in to Arcadestr', exact: true }).waitFor();
    for (const width of [320, 360, 768]) {
      await page.setViewportSize({ width, height: 1000 });
      await noOverflow(`sign-in ${width}`);
    }
    await page.setViewportSize({ width: 1280, height: 1000 });
    const secret = page.getByLabel('Secret key', { exact: true });
    assert.equal(await secret.isVisible(), false, 'secret-key entry is advanced, not the default');
    const options = page.locator('summary').filter({ hasText: 'OTHER SIGN-IN OPTIONS' });
    await options.focus();
    await page.keyboard.press('Enter');
    await secret.fill('not-a-real-secret');
    await options.click();
    assert.equal(await secret.isVisible(), false);
    await options.click();
    assert.equal(await secret.inputValue(), 'not-a-real-secret', 'disclosure preserves inputs');
    await secret.fill('');
    await options.click();
    await page.getByRole('button', { name: 'Connect browser extension', exact: true }).click();
    await page.getByRole('navigation', { name: 'Primary navigation', exact: true }).waitFor();

    async function noOverflow(label) {
      const dimensions = await page.evaluate(() => ({ width: innerWidth, content: document.documentElement.scrollWidth }));
      if (dimensions.content > dimensions.width + 1) {
        const overflowing = await page.locator('body *').evaluateAll(elements => elements
          .filter(element => element.getBoundingClientRect().right > innerWidth + 1 && element.getBoundingClientRect().width)
          .slice(-12).map(element => ({ tag: element.tagName, class: element.className, text: element.textContent.slice(0, 80) })));
        assert.fail(`${label}: horizontal overflow ${JSON.stringify({ ...dimensions, overflowing })}`);
      }
    }
    async function accountDestination(button, heading) {
      await page.getByRole('button', { name: /^Open account menu for/ }).click();
      await page.getByRole('button', { name: button, exact: true }).click();
      await page.getByRole('heading', { name: heading, exact: true }).waitFor();
    }
    for (const width of [320, 360, 768, 1440, 1920]) {
      await page.setViewportSize({ width, height: 1000 });
      for (const name of ['Store', 'Library', 'Community', 'Publish']) {
        const button = page.getByRole('navigation', { name: 'Primary navigation', exact: true }).getByRole('button', { name, exact: true });
        assert.equal(await button.isVisible(), true, `${name} navigation visible at ${width}`);
      }
      await noOverflow(`store ${width}`);
      const search = page.getByRole('searchbox', { name: 'Search games, creators, and tags' });
      await search.fill('puzzle');
      await search.press('Enter');
      await page.getByRole('heading', { name: 'Browse games', exact: true }).waitFor();
      await noOverflow(`browse ${width}`);
      await page.getByRole('button', { name: 'Library', exact: true }).click();
      await page.getByRole('heading', { name: 'Play your games in the desktop app', exact: true }).waitFor();
      await noOverflow(`library ${width}`);
      await page.getByRole('button', { name: 'Community', exact: true }).click();
      await page.getByRole('heading', { name: 'Community', exact: true }).waitFor();
      await noOverflow(`community ${width}`);
      await page.getByRole('button', { name: 'Publish', exact: true }).click();
      await page.getByRole('heading', { name: 'Publishing unavailable on the web', exact: true }).waitFor();
      await noOverflow(`publish ${width}`);
      await accountDestination('Accounts & settings', 'Settings');
      await noOverflow(`settings ${width}`);
      const troubleshooting = page.locator('summary').filter({ hasText: 'TROUBLESHOOTING' });
      assert.equal(await troubleshooting.locator('..').getAttribute('open'), null);
      await troubleshooting.click();
      assert.equal(await page.getByRole('button', { name: 'Copy troubleshooting details', exact: true }).isVisible(), true);
      await troubleshooting.click();
      await accountDestination('Purchases', 'Purchases and access');
      assert.equal(await page.getByTestId('load-refunds').isVisible(), false, 'recipient tools collapsed');
      await noOverflow(`purchases ${width}`);
      await accountDestination('Achievements', 'Achievements');
      await noOverflow(`achievements ${width}`);
      await accountDestination('View profile', 'Cached Player');
      await noOverflow(`profile ${width}`);
      await page.getByRole('button', { name: 'Store', exact: true }).click();
      if (width === 1920) {
        const bar = await page.locator('.arc-topbar-inner').boundingBox();
        assert.ok(bar.width <= 1440 && Math.abs(bar.x - (width - bar.width) / 2) < 1, 'top bar capped and centered');
      }
    }
    await page.getByRole('button', { name: 'Show network status', exact: true }).click();
    await page.getByRole('heading', { name: 'Network status', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Close relay connections', exact: true }).click();
    assert.deepEqual(errors, [], 'no browser runtime errors');
    console.log('PASS: advanced disclosure, input retention, search, all web destinations, navigation and layouts at 320/360/768/1440/1920px');
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
