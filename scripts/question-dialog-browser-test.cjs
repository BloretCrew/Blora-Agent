// SPDX-License-Identifier: GPL-3.0-or-later
// Browser check for a mixed ask_user dialog: choices, other text, an in-dialog
// folder/file picker, submit, skip, and a browser notification.
// Build first: cargo build -p blora-cli --offline
// Run: NODE_PATH=/root/node_modules node scripts/question-dialog-browser-test.cjs
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { chromium } = require('playwright');

const root = path.resolve(__dirname, '..');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function waitFile(file) {
  for (let i = 0; i < 200; i++) {
    if (fs.existsSync(file) && fs.readFileSync(file, 'utf8')) return fs.readFileSync(file, 'utf8');
    await sleep(50);
  }
  throw new Error(`timed out waiting for ${file}`);
}

async function waitHttp(url) {
  for (let i = 0; i < 200; i++) {
    try { if ((await fetch(url)).ok) return; } catch (_) { /* not up yet */ }
    await sleep(100);
  }
  throw new Error(`server did not start at ${url}`);
}

const mixed = {
  questions: [
    { type: 'single', header: '数据库', question: '用哪个数据库？', options: [{ label: 'SQLite', description: '嵌入' }, { label: 'Postgres', description: '服务' }] },
    { type: 'multi', header: '功能', question: '要哪些功能？', options: [{ label: '日志' }, { label: '自建追踪' }] },
    { type: 'boolean', header: '继续', question: '现在继续吗？' },
    { type: 'text', header: '说明', question: '还有什么要补充？' },
    { type: 'folder', header: '目录', question: '代码放在哪？' },
    { type: 'file', header: '文件', question: '读哪个文件？' },
  ],
};

(async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'blora-question-web-'));
  const script = path.join(dir, 'script.json');
  fs.writeFileSync(script, JSON.stringify([
    { tool: 'ask_user', arguments: mixed },
    { text: 'ANSWERED-MIXED' },
    { tool: 'ask_user', arguments: { question: 'Skip me?' } },
    { text: 'SKIPPED-OK' },
  ]));
  const portFile = path.join(dir, 'port');
  const fake = spawn('python3', [path.join(root, 'scripts/fake_openai.py'), portFile, script], { stdio: 'inherit' });
  const children = [fake];
  let browser;
  try {
    const fakePort = await waitFile(portFile);
    const workspace = path.join(dir, 'ws');
    fs.mkdirSync(path.join(workspace, 'sub'), { recursive: true });
    fs.writeFileSync(path.join(workspace, 'note.txt'), 'hi');
    fs.writeFileSync(path.join(workspace, 'sub', 'inner.txt'), 'inside');
    const port = 19100 + Math.floor(Math.random() * 500);
    const url = `http://127.0.0.1:${port}`;
    const server = spawn(path.join(root, 'target/debug/blora'), ['web', '--bind', `127.0.0.1:${port}`, '--workspace', workspace], {
      env: { ...process.env, BLORA_HOME: path.join(dir, 'home'), BLORA_PROVIDER: 'openai', BLORA_API_KEY: 'test', BLORA_API_BASE: `http://127.0.0.1:${fakePort}/v1`, BLORA_MODEL: 'fake-model' },
      stdio: 'ignore',
    });
    children.push(server);
    await waitHttp(`${url}/api/sessions`);
    browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', args: ['--no-sandbox'] });
    for (const [index, viewport] of [{ width: 1440, height: 900 }, { width: 390, height: 844 }].entries()) {
      const context = await browser.newContext({ viewport });
      await context.addInitScript(() => {
        localStorage.setItem('blora-web-onboarding-version', '1');
        localStorage.setItem('blora-provider', 'openai');
        localStorage.setItem('blora-model', 'fake-model');
        window.__bloraNotifications = [];
        window.Notification = class {
          static permission = 'granted';
          static requestPermission() { return Promise.resolve('granted'); }
          constructor(title, options) {
            window.__bloraNotifications.push({ title, body: options && options.body });
          }
        };
      });
      const page = await context.newPage();
      page.setDefaultTimeout(30000);
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(url);
      await page.locator('#view-session').waitFor({ state: 'visible' });
      const mobile = viewport.width < 600;
      if (mobile) await page.getByRole('button', { name: '打开导航', exact: true }).click();
      await page.locator('#new-session').click();
      if (mobile) await page.waitForFunction(() => document.querySelector('aside')?.getAttribute('aria-hidden') === 'true');
      await page.waitForFunction(() => !!window.BloraWeb.getState().sessionId);
      await page.locator('#prompt').fill(index === 0 ? 'ask mixed' : 'ask skip');
      await page.locator('#prompt').press('Enter');
      const dialog = page.getByRole('dialog', { name: 'Blora 提问' });
      await dialog.waitFor({ state: 'visible' });
      const box = await dialog.boundingBox();
      assert.ok(box && box.x >= 0 && box.x + box.width <= viewport.width + 1, `dialog fits ${viewport.width}`);
      await page.waitForFunction(() => (window.__bloraNotifications || []).some(item => item.title === 'Blora 提问'));

      if (index === 0) {
        const denied = await page.evaluate(async () => (await fetch('/api/workspace/list?path=' + encodeURIComponent('..'))).status);
        assert.equal(denied, 400);
        const database = dialog.locator('.ba-ask__question').filter({ hasText: '用哪个数据库？' });
        await database.getByRole('button', { name: 'SQLite', exact: true }).click();
        await database.getByRole('button', { name: '其他', exact: true }).click();
        await database.getByRole('textbox', { name: '其他' }).fill('Tu');
        const features = dialog.locator('.ba-ask__question').filter({ hasText: '要哪些功能？' });
        await features.getByRole('button', { name: '日志', exact: true }).click();
        await features.getByRole('button', { name: '自建追踪', exact: true }).click();
        const yesno = dialog.locator('.ba-ask__question').filter({ hasText: '现在继续吗？' });
        await yesno.getByRole('button', { name: '是', exact: true }).click();
        await dialog.getByRole('textbox', { name: '说明' }).fill('先写测试');
        const folder = dialog.locator('.ba-ask__question').filter({ hasText: '代码放在哪？' });
        await folder.getByRole('button', { name: 'sub/', exact: true }).click();
        await folder.getByRole('button', { name: '选择此文件夹', exact: true }).click();
        const file = dialog.locator('.ba-ask__question').filter({ hasText: '读哪个文件？' });
        await file.getByRole('button', { name: 'sub/', exact: true }).click();
        await file.getByRole('button', { name: 'inner.txt', exact: true }).click();
        await dialog.getByRole('button', { name: '提交', exact: true }).click();
        await page.waitForFunction(() => document.querySelector('#thread').innerText.includes('ANSWERED-MIXED'));
        const answered = page.locator('[data-kind="question.answered"]');
        await answered.waitFor();
        const text = await answered.innerText();
        for (const part of ['Tu', '日志', '自建追踪', '是', '先写测试', 'sub', 'inner.txt']) {
          assert.ok(text.includes(part), text);
        }
      } else {
        await page.keyboard.press('Escape');
        await page.waitForFunction(() => document.querySelector('#thread').innerText.includes('SKIPPED-OK'));
        assert.equal(await dialog.isVisible(), false);
      }
      assert.deepEqual(errors, []);
      await page.screenshot({ path: path.join(os.tmpdir(), `blora-question-${viewport.width}.png`), fullPage: false });
      await context.close();
    }
    console.log('question dialog browser test passed');
  } finally {
    if (browser) await browser.close();
    for (const child of children) child.kill('SIGTERM');
  }
})().catch(error => { console.error(error); process.exit(1); });
