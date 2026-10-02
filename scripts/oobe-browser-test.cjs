// SPDX-License-Identifier: GPL-3.0-or-later
// Run against a rebuilt local server: NODE_PATH=<playwright modules> node scripts/oobe-browser-test.cjs
const assert = require('node:assert/strict');
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  const url = process.env.BLORA_TEST_URL || 'http://127.0.0.1:18787';
  try {
    for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport });
      const page = await context.newPage();
      if (process.env.BLORA_TEST_SOURCE === "1") {
        const fs = require('node:fs');
        for (const [path, file, contentType] of [['/', 'web/index.html', 'text/html'], ['/app.css', 'web/app.css', 'text/css'], ['/onboarding.js', 'web/onboarding.js', 'text/javascript']]) {
          await context.route(`${url}${path}`, route => route.fulfill({ body: fs.readFileSync(file), contentType }));
        }
      }
      let loggedIn = false, starts = 0, cancels = 0, polls = 0, failure = null, gateway = false;
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await context.route('**/api/auth/**', async route => {
        const path = new URL(route.request().url()).pathname;
        const json = body => route.fulfill({ json: body });
        if (path.endsWith('/me')) return loggedIn ? json({ name: '测试账号', username: 'test', credentials_valid: true }) : route.fulfill({ status: 401, body: 'Unauthorized' });
        if (path.endsWith('/logout')) { loggedIn = false; return route.fulfill({ status: 204 }); }
        if (path.endsWith('/cancel')) { cancels++; return route.fulfill({ status: 204 }); }
        if (path.endsWith('/device')) {
          starts++; polls = 0;
          return json({ attempt_id: `attempt-${starts}`, user_code: 'TEST-1234', verification_uri: 'https://passport.example/authorize', expires_in: failure === 'expiry' ? 1 : 60, interval: 1 });
        }
        if (path.endsWith('/poll')) {
          polls++;
          if (failure === 'denied') return route.fulfill({ status: 400, body: 'access_denied' });
          if (polls === 1) return json({ status: 'pending', interval: 1 });
          loggedIn = true;
          return json({ status: 'authenticated', username: 'test', name: '测试账号' });
        }
        return route.continue();
      });
      await context.route('**/api/settings', async route => {
        const response = await route.fetch();
        const body = await response.json();
        return route.fulfill({ json: { ...body, gateway } });
      });
      await page.goto(`${url}/#/tasks`);
      await page.locator('#onboarding-next').click();
      await page.locator('#onboarding-skip').waitFor({ state: 'visible' });
      assert.equal(starts, 0, 'startup must not start authorization');
      await page.locator('#onboarding-skip').click();
      assert.equal(await page.locator('#onboarding-complete').isVisible(), true);
      assert.equal(await page.locator('#onboarding-summary-account').textContent(), '稍后连接');
      assert.ok(await page.locator('#onboarding-summary-workspace').textContent());
      await page.locator('#onboarding-next').click();
      await page.locator('#view-tasks').waitFor({ state: 'visible' });
      await page.reload();
      await page.locator('#view-tasks').waitFor({ state: 'visible' });
      assert.equal(await page.locator('#view-onboarding').isVisible(), false);

      for (const view of ['session', 'tasks', 'approvals', 'timeline', 'artifacts', 'workspace', 'settings']) {
        await page.evaluate(view => { location.hash = `#/${view}`; }, view);
        await page.locator(`#view-${view}`).waitFor({ state: 'visible' });
      }
      // Keep real session and Imagine data paths in the regression run.
      await page.evaluate(() => { location.hash = '#/session'; });
      await page.locator('#view-session').waitFor({ state: 'visible' });
      await page.evaluate(() => document.querySelector('#new-session').click());
      await page.waitForFunction(() => location.hash.includes('/session/ses_'));
      await page.locator('#prompt').fill('OOBE regression input');
      const sessionHash = await page.evaluate(() => location.hash);
      await page.evaluate(() => document.querySelector('#passport-login').click());
      await page.locator('#onboarding-cancel').click();
      await page.waitForFunction(hash => location.hash === hash, sessionHash);
      assert.equal(await page.locator('#prompt').inputValue(), 'OOBE regression input');
      await page.locator('#run-mode').selectOption('imagine');
      await page.waitForFunction(() => document.querySelector('#view-session').classList.contains('ba-session--imagine'));
      await page.locator('#run-mode').selectOption('code');
      await page.waitForFunction(() => !document.querySelector('#view-session').classList.contains('ba-session--imagine'));
      await page.evaluate(() => { location.hash = '#/settings'; });
      await page.locator('#view-settings').waitFor({ state: 'visible' });
      await page.locator('#settings-onboarding').click();
      await page.locator('#onboarding-next').click();
      await page.locator('#onboarding-login').click();
      await page.locator('#onboarding-code').waitFor({ state: 'visible' });
      assert.equal(await page.locator('#onboarding-account-badge').textContent(), '等待授权');
      const beforeRetry = starts;
      await page.locator('#onboarding-retry').click();
      await page.waitForFunction(() => document.querySelector('#onboarding-code').offsetParent !== null);
      await page.waitForTimeout(150);
      assert.equal(starts, beforeRetry + 1);
      await page.locator('#onboarding-back').click();
      await page.waitForFunction(() => document.querySelector('#onboarding-title').textContent.includes('欢迎使用'));
      await page.waitForTimeout(200);
      assert.ok(cancels > 0);
      assert.equal(loggedIn, false);
      await page.locator('#onboarding-next').click();
      failure = 'denied';
      await page.locator('#onboarding-login').click();
      await page.waitForFunction(() => document.querySelector('#onboarding-status').textContent.includes('access_denied'));
      failure = 'expiry';
      await page.locator('#onboarding-login').click();
      await page.waitForFunction(() => document.querySelector('#onboarding-status').textContent.includes('过期'));
      failure = null;
      await page.locator('#onboarding-login').click();
      await page.waitForFunction(() => document.querySelector('#onboarding-next').hidden === false);
      await page.locator('#onboarding-next').click();
      await page.locator('#onboarding-next').click();
      await page.locator('#view-settings').waitFor({ state: 'visible' });
      await page.waitForFunction(() => document.querySelector('#settings-account').textContent.includes('测试账号'));
      const beforeLogout = starts;
      // The settings account controls remain available at mobile widths.
      await page.evaluate(() => document.querySelector('#passport-logout').click());
      await page.waitForFunction(() => !document.querySelector('#passport-login').hidden);
      assert.equal(starts, beforeLogout, 'logout must wait for explicit login');
      await page.locator('#settings-login').click();
      await page.locator('#onboarding-login').waitFor({ state: 'visible' });
      assert.equal(starts, beforeLogout, 'opening account step must not start authorization');
      // A second tab observes shared account changes on focus.
      const second = await context.newPage();
      await second.goto(`${url}/#/settings`);
      await second.locator('#view-settings').waitFor({ state: 'visible' });
      loggedIn = true;
      await second.evaluate(() => window.dispatchEvent(new Event('focus')));
      await second.waitForFunction(() => document.querySelector('#settings-account').textContent.includes('测试账号'));
      loggedIn = false;
      await second.evaluate(() => window.dispatchEvent(new Event('focus')));
      await second.waitForFunction(() => document.querySelector('#settings-account').textContent.includes('尚未连接'));
      await second.close();
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
      assert.equal(overflow, false, 'onboarding must fit the viewport');
      await page.locator('#onboarding-cancel').click();
      gateway = true;
      await page.locator('#settings-login').click();
      await page.waitForFunction(() => document.querySelector('#onboarding-description').textContent.includes('网关'));
      assert.equal(await page.locator('#onboarding-skip').isVisible(), false);
      assert.deepEqual(errors, []);
      await context.close();
      console.log(`PASS OOBE ${viewport.width}x${viewport.height}: skip, reload, routes, cancel, denied, expiry, retry, login, logout, gateway`);
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
