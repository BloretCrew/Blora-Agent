// SPDX-License-Identifier: GPL-3.0-or-later
// Browser check for ask_user cards, reasoning folding, compaction notes,
// mode labels, /plan, and /reasoning. Starts `blora web` against
// scripts/fake_openai.py. Build first: cargo build -p blora-cli
// Run: NODE_PATH=/root/node_modules node scripts/ask-user-browser-test.cjs
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

(async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'blora-ask-web-'));
  const script = path.join(dir, 'script.json');
  const turns = [];
  // Two viewports, one question each.
  for (const region of ['EU', 'US']) {
    turns.push({ reasoning: 'SECRET-THOUGHT about regions', tool: 'ask_user', arguments: { question: 'Which region should I deploy to?', options: ['EU', 'US'] } });
    turns.push({ text: `DEPLOYED-${region}` });
  }
  fs.writeFileSync(script, JSON.stringify(turns));
  const portFile = path.join(dir, 'port');
  const fake = spawn('python3', [path.join(root, 'scripts/fake_openai.py'), portFile, script], { stdio: 'inherit' });
  const children = [fake];
  let browser;
  try {
    const fakePort = await waitFile(portFile);
    const workspace = path.join(dir, 'ws');
    fs.mkdirSync(workspace);
    const port = 18900 + Math.floor(Math.random() * 500);
    const url = `http://127.0.0.1:${port}`;
    const server = spawn(path.join(root, 'target/debug/blora'), ['web', '--bind', `127.0.0.1:${port}`, '--workspace', workspace], {
      env: { ...process.env, BLORA_HOME: path.join(dir, 'home'), BLORA_PROVIDER: 'openai', BLORA_API_KEY: 'test', BLORA_API_BASE: `http://127.0.0.1:${fakePort}/v1`, BLORA_MODEL: 'fake-model' },
      stdio: 'ignore',
    });
    children.push(server);
    await waitHttp(`${url}/api/sessions`);
    browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', args: ['--no-sandbox'] });
    const expected = ['EU', 'US'];
    for (const [index, viewport] of [{ width: 1440, height: 900 }, { width: 390, height: 844 }].entries()) {
      const context = await browser.newContext({ viewport });
      await context.addInitScript(() => {
        localStorage.setItem('blora-web-onboarding-version', '1');
        localStorage.setItem('blora-provider', 'openai');
        localStorage.setItem('blora-model', 'fake-model');
      });
      const page = await context.newPage();
      page.setDefaultTimeout(30000);
      const errors = []; page.on('pageerror', error => errors.push(error.message));
      await page.goto(url);
      await page.locator('#view-session').waitFor({ state: 'visible' });
      const mobile = viewport.width < 600;
      if (mobile) await page.getByRole('button', { name: '打开导航', exact: true }).click();
      await page.locator('#new-session').click();
      if (mobile) await page.waitForFunction(() => document.querySelector('aside')?.getAttribute('aria-hidden') === 'true');
      await page.waitForFunction(() => !!window.BloraWeb.getState().sessionId);
      const session = await page.evaluate(() => window.BloraWeb.getState().sessionId);

      // Mode selector explains each mode.
      const labels = await page.locator('#run-mode option').allTextContents();
      assert.deepEqual(labels.slice(0, 3), ['Code · 编码', 'Work · 工作产出', 'Agent · 自主助理']);

      await page.locator('#prompt').fill('pick a region');
      await page.locator('#prompt').press('Enter');
      const card = page.getByRole('dialog', { name: 'Blora 提问' });
      await card.waitFor({ state: 'visible' });
      assert.equal(await page.locator('#session-question').isVisible(), false);
      assert.match(await card.innerText(), /Which region should I deploy to\?/);
      const box = await card.boundingBox();
      assert.ok(box.x >= 0 && box.x + box.width <= viewport.width + 1, `question dialog fits the ${viewport.width}px viewport`);
      const optionButtons = card.locator('.ba-ask__options button');
      assert.equal(await optionButtons.count(), 3);
      for (const button of await optionButtons.all()) {
        const b = await button.boundingBox();
        assert.ok(b && b.x + b.width <= viewport.width + 1, 'option buttons stay on screen');
      }
      // Reasoning stays folded behind the modal. Expanding it waits until the dialog closes.
      const reasoning = page.locator('details.ba-reasoning').first();
      await reasoning.waitFor();
      assert.equal(await reasoning.evaluate(node => node.open), false);
      assert.equal(await page.evaluate(() => window.BloraWeb.getState().session.run_status), 'waiting_input');

      const pick = expected[index];
      if (index === 0) {
        await card.getByRole('button', { name: pick, exact: true }).click();
      } else {
        await card.getByRole('button', { name: '其他', exact: true }).click();
        await card.getByRole('textbox', { name: '其他' }).fill(pick);
      }
      await card.getByRole('button', { name: '提交', exact: true }).click();
      await page.waitForFunction(text => document.querySelector('#thread').innerText.includes(text), `DEPLOYED-${pick}`);
      assert.equal(await card.isVisible(), false, 'dialog closes after answering');
      await reasoning.locator('summary').click();
      assert.equal(await page.locator('.ba-reasoning__body').first().isVisible(), true);
      assert.match(await page.locator('.ba-reasoning__body').first().innerText(), /SECRET-THOUGHT/);
      const answered = page.locator('[data-kind="question.answered"]');
      await answered.waitFor();
      const answeredText = await answered.innerText();
      assert.match(answeredText, /→ /);
      assert.ok(answeredText.includes(pick), answeredText);
      const detail = await (await fetch(`${url}/api/sessions/${session}`)).json();
      assert.equal(detail.pending_input, null);

      // /reasoning expands reasoning on re-render; /plan toggles permission only.
      await page.locator('#prompt').fill('/reasoning');
      await page.locator('#prompt').press('Enter');
      await page.waitForFunction(() => localStorage.getItem('blora-show-reasoning') === 'on');
      await page.waitForFunction(() => document.querySelector('details.ba-reasoning')?.open === true);
      await page.locator('#prompt').fill('/plan');
      await page.locator('#prompt').press('Enter');
      await page.waitForFunction(() => window.BloraWeb.getState().permission === 'plan');
      assert.equal(await page.evaluate(() => window.BloraWeb.getState().sessionId), session);
      await page.waitForFunction(() => document.querySelector('[data-status="permission"]')?.textContent === '权限 · 计划');

      // A session with compactions shows the count in the usage line.
      await page.evaluate(() => {
        const state = window.BloraWeb.getState();
        state.session.compaction_count = 3;
      });
      await page.evaluate(() => window.BloraWeb.renderUsage(window.BloraWeb.getState().session));
      assert.match(await page.locator('#session-usage').innerText(), /已压缩 3 次/);
      assert.equal(await page.locator('#session-usage').getAttribute('data-warn'), 'true');
      await page.screenshot({ path: path.join(os.tmpdir(), `blora-ask-${viewport.width}.png`), fullPage: false });
      assert.deepEqual(errors, [], 'no page errors');
      await context.close();
    }
    console.log('ask-user browser test passed');
  } finally {
    if (browser) await browser.close();
    for (const child of children) child.kill('SIGTERM');
  }
})().catch(error => { console.error(error); process.exit(1); });
