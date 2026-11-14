/* Real-WASM polish regression, with isolated login and no external traffic.
 * Build: (cd web && trunk build --features web)
 * Run: ARCADESTR_CHROMIUM=/usr/bin/chromium node tests/ui_polish_browser.cjs
 */
const { chromium, expect } = require('playwright/test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const fixture = require('./fixtures/login_metadata.json');
const root = path.resolve(process.env.ARCADESTR_WEB_BUILD || path.join(__dirname, '../web/dist'));
const server = http.createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const file = path.resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
  if (!file.startsWith(`${root}${path.sep}`)) {
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
            if (request.slice(2).some(filter => filter.kinds?.includes(0))) {
              this.onmessage?.(new MessageEvent('message', {
                data: JSON.stringify(['EVENT', request[1], fixture.event]),
              }));
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
    await page.getByRole('button', { name: 'Connect browser extension', exact: true }).click();
    const navigation = page.getByRole('navigation', { name: 'Primary navigation', exact: true });
    await navigation.waitFor();
    await expect(page).toHaveTitle('Store — Arcadestr');

    // A failed profile image must become initials, not a broken-image glyph.
    await expect(page.locator('.arc-account-fallback')).toHaveText('C');
    await expect(page.locator('.arc-account-avatar:visible')).toHaveCount(1);
    assert.equal(await page.locator('img.arc-account-avatar').count(), 0);

    const search = page.getByRole('searchbox', { name: 'Search games, creators, and tags' });
    await search.fill('puzzle');
    await search.press('Escape');
    await expect(search).toHaveValue('');
    await expect(search).toBeFocused();
    await expect(page).toHaveTitle('Store — Arcadestr');
    await search.fill('puzzle');
    await search.press('Enter');
    await expect(page).toHaveTitle('Browse games — Arcadestr');
    await expect(page.getByRole('main')).toBeFocused();

    for (const width of [320, 360, 768, 1440, 1920]) {
      await page.setViewportSize({ width, height: 700 });
      await navigation.getByRole('button', { name: 'Store', exact: true }).click();
      await expect(page).toHaveTitle('Store — Arcadestr');
      await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
      // Programmatic activation preserves the scrolled position until routing
      // handles it, unlike Playwright's automatic scroll-into-view click.
      await navigation.getByRole('button', { name: 'Library', exact: true }).evaluate(button => button.click());
      await expect(page).toHaveTitle('Library — Arcadestr');
      await expect(page.getByRole('main', { name: 'Library', exact: true })).toBeFocused();
      await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
      const empty = page.locator('.arc-feedback-empty').first();
      await expect(empty).toBeVisible();
      const dimensions = await empty.evaluate(element => {
        const heading = element.querySelector('h2');
        return { height: element.getBoundingClientRect().height, headingSize: getComputedStyle(heading).fontSize };
      });
      assert.ok(dimensions.height >= 420, `empty-state breathing room at ${width}px`);
      assert.equal(dimensions.headingSize, '20px');
      const overflow = await page.evaluate(() => ({
        width: innerWidth, content: document.documentElement.scrollWidth, x: scrollX,
        elements: [...document.querySelectorAll('body *')].filter(element => {
          const bounds = element.getBoundingClientRect();
          return bounds.width && (element.scrollWidth > element.clientWidth + 2 || bounds.right > innerWidth + 1);
        }).map(element => ({ tag: element.tagName, class: element.className, width: element.clientWidth, content: element.scrollWidth, right: element.getBoundingClientRect().right })).slice(-15),
      }));
      assert.ok(overflow.content <= width + 1, `no overflow at ${width}px: ${JSON.stringify(overflow)}`);
    }
    assert.deepEqual(errors, [], 'no browser runtime errors');
    console.log('PASS: route titles, focus and scroll reset, search Escape, avatar fallback and responsive empty states');
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
