// SPDX-License-Identifier: GPL-3.0-or-later
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const url = process.env.BLORA_TEST_URL || 'http://127.0.0.1:18787';
(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  try {
    for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport });
      await context.addInitScript(() => localStorage.setItem('blora-web-onboarding-version', '1'));
      if (process.env.BLORA_TEST_SOURCE === '1') {
        const fs = require('node:fs');
        for (const file of ['index.html', 'app.js', 'app.css', 'dialogs.js', 'workbench.js', 'commands.js']) {
          await context.route(`${url}/${file === 'index.html' ? '' : file}`, route => route.fulfill({ body: fs.readFileSync(`web/${file}`), contentType: file.endsWith('.css') ? 'text/css' : file.endsWith('.html') ? 'text/html' : 'text/javascript' }));
        }
      }
      const page = await context.newPage();
      page.setDefaultTimeout(60000);
      page.setDefaultNavigationTimeout(90000);
      const errors = []; page.on('pageerror', error => errors.push(error.message));
      await page.goto(url); await page.locator('#view-session').waitFor({ state: 'visible' });
      if (viewport.width < 600) await page.getByRole('button', { name: '打开导航', exact: true }).click();
      await page.locator('#new-session').click();
      if (viewport.width < 600) await page.getByRole('button', { name: '打开导航', exact: true }).click();
      await page.waitForFunction(() => !!window.BloraWeb.getState().sessionId);
      const original = await page.evaluate(() => window.BloraWeb.getState().sessionId);
      async function command(text) {
        await page.locator('#prompt').fill(text); await page.locator('#prompt').press('Enter');
      }
      async function opened(name) {
        await page.waitForFunction(name => window.BloraDialogs.top()?.name === name && !window.BloraDialogs.top()?.closed, name);
        return page.locator('.ba-window-layer--active .ba-window');
      }
      async function close() { await page.keyboard.press('Escape'); await page.waitForTimeout(220); }
      const sendBounds = await page.locator('#send').boundingBox();
      assert.ok(sendBounds && sendBounds.x >= 0 && sendBounds.x + sendBounds.width <= viewport.width, 'send action remains on screen');
      await page.locator('#prompt').fill('保留的草稿');
      await page.evaluate(() => window.BloraWorkbench.open('settings'));
      let panel = await opened('settings');
      await panel.getByRole('tab', { name: '外观', exact: true }).click();
      await panel.getByRole('button', { name: '主题预览', exact: true }).click();
      panel = await opened('theme');
      await panel.getByRole('button', { name: 'indigo', exact: true }).click();
      assert.equal(await page.locator('html').getAttribute('data-blora-theme'), 'indigo');
      await close(); await opened('settings');
      assert.equal(await page.locator('html').getAttribute('data-blora-theme'), 'coral');
      await close(); assert.equal(await page.locator('#prompt').inputValue(), '保留的草稿');
      await command('/permissions'); panel = await opened('permission');
      await panel.getByRole('button', { name: /跳过权限确认|全部允许/, exact: false }).click();
      await page.waitForFunction(() => window.BloraDialogs.depth === 2);
      await close(); assert.notEqual(await page.evaluate(() => window.BloraWeb.getState().permission), 'yolo');
      await close();
      await command('/yes on');
      panel = page.locator('.ba-window-layer--active .ba-window');
      await panel.getByRole('checkbox').check();
      await panel.getByRole('button', { name: '确认', exact: true }).click();
      await page.waitForFunction(() => !window.BloraDialogs.top());
      assert.equal(await page.evaluate(() => window.BloraWeb.getState().permission), 'yolo');
      await page.locator('#permission-mode').selectOption('plan');
      await page.waitForFunction(() => window.BloraWeb.getState().permission === 'plan');
      await command('/no');
      await page.waitForFunction(() => window.BloraWeb.getState().permission === 'ask');
      await command('/harness'); panel = await opened('harness');
      await panel.getByRole('button', { name: '使用 Blora', exact: true }).waitFor();
      await panel.getByRole('button', { name: '最小化', exact: true }).click();
      await page.keyboard.press('x');
      assert.equal(await page.locator('#prompt').inputValue(), '');
      await page.keyboard.press('Enter');
      assert.equal(await panel.locator('.ba-window__body').isVisible(), true); await close();
      await command('/model'); await opened('model'); await close();
      await command('/git'); panel = await opened('git');
      await page.waitForFunction(() => !document.querySelector('.ba-window-layer--active .ba-window__body').textContent.includes('加载中'));
      assert.equal(await panel.locator('[role="tab"]').count(), 3);
      await panel.getByRole('tab', { name: 'Git 图', exact: true }).click();
      assert.ok((await panel.textContent()).includes('test fixture'));
      await close();
      await command('/remember parity-test 浏览器回归记忆');
      await command('/recall parity-test');
      await command('/checkpoint');
      await command('/task 浏览器后台任务');
      for (const view of ['tasks', 'approvals', 'workspace', 'timeline', 'artifacts', 'settings', 'session']) {
        await page.evaluate(view => { location.hash = `#/${view}`; }, view);
        await page.locator(`#view-${view}`).waitFor({ state: 'visible' });
      }
      await page.evaluate(() => window.BloraWeb.store('blora-provider', 'mock'));
      await command('浏览器运行测试');
      await page.waitForFunction(() => window.BloraWeb.getState().session?.transcript?.some(row => row.kind === 'assistant'), { timeout: 20000 });
      assert.equal(await page.evaluate(() => window.BloraWeb.getState().sessionId), original);
      await page.locator('#prompt').press('ArrowUp');
      assert.equal(await page.locator('#prompt').inputValue(), '浏览器运行测试');
      await page.locator('#prompt').press('ArrowDown'); assert.equal(await page.locator('#prompt').inputValue(), '');
      await command('/yes on');
      panel = page.locator('.ba-window-layer--active .ba-window');
      await panel.getByRole('checkbox').check();
      await panel.getByRole('button', { name: '确认', exact: true }).click();
      await page.waitForFunction(() => window.BloraWeb.getState().permission === 'yolo');
      await command('/plan');
      await page.waitForFunction(() => window.BloraWeb.getState().permission === 'plan');
      assert.equal(await page.evaluate(() => window.BloraWeb.getState().sessionId), original, '/plan keeps the current session');
      await command('/agent');
      await page.waitForFunction(id => window.BloraWeb.getState().sessionId !== id && window.BloraWeb.getState().session?.mode === 'agent', original);
      assert.equal(await page.evaluate(() => window.BloraWeb.getState().permission), 'plan', 'plan mode is not a bypass and carries over');
      for (const name of ['session', 'project', 'folder', 'mode', 'harness', 'provider', 'model', 'settings', 'context', 'tools', 'memory', 'plugins', 'diagnostics', 'help']) {
        await page.evaluate(name => window.BloraWorkbench.open(name), name);
        await opened(name);
        await page.keyboard.press('Tab');
        assert.equal(await page.evaluate(() => !!document.activeElement.closest('.ba-window')), true);
        await close();
        await page.waitForFunction(() => !window.BloraDialogs.top());
      }
      // Some empty capability lists show dialog-local errors; no fatal app errors.
      assert.equal(await page.locator('#page-alert').isVisible(), false);
      assert.equal(await page.locator('#imagine-aspect').isVisible(), false);
      await page.locator('#run-mode').selectOption('imagine');
      await page.waitForFunction(() => window.BloraWeb.getState().session?.mode === 'imagine');
      assert.equal(await page.locator('#imagine-aspect').isVisible(), true);
      await page.locator('#run-mode').selectOption('code');
      await page.waitForFunction(() => window.BloraWeb.getState().session?.mode === 'code');
      assert.equal(await page.locator('#imagine-aspect').isVisible(), false);
      assert.deepEqual(errors, [], `page errors at ${viewport.width}`);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
      console.log(`PASS web parity ${viewport.width}x${viewport.height}`);
      await context.close();
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
