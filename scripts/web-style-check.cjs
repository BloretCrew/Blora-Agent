// SPDX-License-Identifier: GPL-3.0-or-later
// Verify the Web UI renders with the vendored Blora Design assets.
//
// The UI once shipped with no stylesheet at all because every /vendor asset
// pointed at a gitignored checkout. This drives a real browser and asserts the
// stylesheet actually parsed and the design tokens resolved to the same values
// the TUI uses, so that regression cannot come back unnoticed.
//
// Usage: node scripts/web-style-check.cjs [url]
//   url defaults to $BLORA_TEST_URL or http://127.0.0.1:8787
//   Browser: $CHROME_PATH, else Chrome/Edge on the usual install paths.
const assert = require('node:assert/strict');
const http = require('node:http');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const url = process.argv[2] || process.env.BLORA_TEST_URL || 'http://127.0.0.1:8787';
const port = 19555 + (process.pid % 500);

function findBrowser() {
  const candidates = [
    process.env.CHROME_PATH,
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  ].filter(Boolean);
  const found = candidates.find((candidate) => fs.existsSync(candidate));
  assert.ok(found, `no Chrome/Edge found; set CHROME_PATH (tried ${candidates.join(', ')})`);
  return found;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function devtoolsJson(route) {
  return new Promise((resolve, reject) => {
    http
      .get({ host: '127.0.0.1', port, path: route }, (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => {
          try {
            resolve(JSON.parse(body));
          } catch (error) {
            reject(error);
          }
        });
      })
      .on('error', reject);
  });
}

async function main() {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'blora-style-'));
  const chrome = spawn(findBrowser(), [
    '--headless=new',
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-gpu',
    '--hide-scrollbars',
    'about:blank',
  ], { stdio: 'ignore' });

  try {
    let targets = null;
    for (let attempt = 0; attempt < 60 && !targets; attempt++) {
      await sleep(250);
      try {
        targets = await devtoolsJson('/json/list');
        if (!targets.some((target) => target.type === 'page')) targets = null;
      } catch {}
    }
    assert.ok(targets, 'browser did not expose a DevTools page');

    const page = targets.find((target) => target.type === 'page');
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener('open', resolve, { once: true });
      ws.addEventListener('error', reject, { once: true });
    });

    let nextId = 0;
    const pending = new Map();
    const failedRequests = [];
    const pageErrors = [];
    ws.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.id && pending.has(message.id)) {
        pending.get(message.id)(message);
        pending.delete(message.id);
        return;
      }
      if (message.method === 'Network.responseReceived') {
        const response = message.params.response;
        // The PassPort probe legitimately 401s when signed out.
        if (response.status >= 400 && !response.url.endsWith('/api/auth/me')) {
          failedRequests.push(`${response.status} ${response.url}`);
        }
      }
      if (message.method === 'Network.loadingFailed') {
        failedRequests.push(`FAILED ${message.params.errorText} ${message.params.documentURL || ''}`);
      }
      if (message.method === 'Runtime.exceptionThrown') {
        const details = message.params.exceptionDetails;
        pageErrors.push(`${details.text}: ${details.exception?.description || ''}`);
      }
    });

    const send = (method, params = {}) => new Promise((resolve) => {
      const id = ++nextId;
      pending.set(id, resolve);
      ws.send(JSON.stringify({ id, method, params }));
    });

    await send('Page.enable');
    await send('Runtime.enable');
    await send('Network.enable');
    await send('Page.addScriptToEvaluateOnNewDocument', {
      source: "try{localStorage.setItem('blora-web-onboarding-version','1');}catch(e){}",
    });
    await send('Page.navigate', { url });
    await sleep(4000);

    const evaluate = async (expression) => {
      const reply = await send('Runtime.evaluate', {
        expression,
        returnByValue: true,
        awaitPromise: true,
      });
      return reply.result?.result?.value;
    };

    const report = await evaluate(`(() => {
      const root = document.documentElement;
      const rootStyle = getComputedStyle(root);
      const token = (name) => rootStyle.getPropertyValue(name).trim();
      const element = document.querySelector('.blora-button');
      const layout = document.querySelector('blora-sidebar-layout');
      const rendered = (el) => {
        if (!el) return null;
        const box = el.getBoundingClientRect();
        return { width: Math.round(box.width), height: Math.round(box.height) };
      };
      return {
        theme: root.getAttribute('data-blora-theme'),
        scheme: root.getAttribute('data-blora-color-scheme'),
        tokens: {
          canvas: token('--blora-color-surface-canvas'),
          text: token('--blora-color-text-primary'),
          primary: token('--blora-color-action-primary-default'),
        },
        compat: {
          text: token('--blora-color-text'),
          border: token('--blora-color-border'),
          action: token('--blora-color-action-primary'),
          borderWidth: token('--blora-border-width'),
        },
        bodyBackground: getComputedStyle(document.body).backgroundColor,
        buttonPadding: element ? getComputedStyle(element).padding : null,
        ruleCount: [...document.styleSheets].reduce((total, sheet) => {
          try { return total + sheet.cssRules.length; } catch { return total; }
        }, 0),
        layout: rendered(layout),
        defined: [
          'blora-sidebar-layout',
          'blora-palette-picker',
          'blora-color-scheme-toggle',
          'blora-markdown',
          'blora-field',
          'blora-sidebar-nav',
          'blora-empty',
          'blora-switch',
        ].filter((tag) => !customElements.get(tag)),
      };
    })()`);

    // Stylesheets parsed, not just requested.
    assert.ok(
      report.ruleCount > 300,
      `expected the design system to parse hundreds of rules, got ${report.ruleCount}`,
    );

    // Coral palette, matching `Theme::of(Palette::Coral, _)` in
    // crates/blora-tui/src/theme.rs. Both schemes must paint.
    const expected = report.scheme === 'dark'
      ? {
        background: 'rgb(23, 22, 28)',
        tokens: { canvas: '#17161C', text: '#F8F2F4', primary: '#D18A94' },
        compat: { text: '#F8F2F4', border: '#413C41', action: '#D18A94' },
      }
      : {
        background: 'rgb(250, 247, 248)',
        tokens: { canvas: '#FAF7F8', text: '#303143', primary: '#9F5964' },
        compat: { text: '#303143', border: '#D9D2D6', action: '#9F5964' },
      };
    assert.equal(
      report.bodyBackground,
      expected.background,
      `body must paint the coral ${report.scheme} canvas, got ${report.bodyBackground}`,
    );
    assert.ok(report.buttonPadding, 'buttons must inherit the design system padding');
    assert.deepEqual(
      report.tokens,
      expected.tokens,
      `design tokens must match the TUI palette, got ${JSON.stringify(report.tokens)}`,
    );

    // The 1.x token names app.css uses must resolve, not silently fall back.
    assert.equal(report.compat.text, expected.compat.text, 'legacy --blora-color-text must map to the 2.x value');
    assert.equal(report.compat.border, expected.compat.border, 'legacy --blora-color-border must map to the 2.x value');
    assert.equal(report.compat.action, expected.compat.action, 'legacy --blora-color-action-primary must map');
    assert.equal(report.compat.borderWidth, '1px', 'legacy --blora-border-width must be defined');

    // The app shell must actually occupy the viewport.
    assert.ok(report.layout, 'blora-sidebar-layout must render');
    assert.ok(report.layout.width > 0 && report.layout.height > 0, 'layout must have a size');

    // Elements the UI depends on must be registered.
    assert.deepEqual(report.defined, [], `unregistered custom elements: ${report.defined.join(', ')}`);

    // An unauthenticated /api/auth/me probe is expected; anything else is not.
    assert.deepEqual(failedRequests, [], `failed requests: ${failedRequests.join('\n')}`);
    assert.deepEqual(pageErrors, [], `page errors: ${pageErrors.join('\n')}`);

    console.log(
      `web-style-check ok: ${report.ruleCount} rules, theme ${report.theme}/${report.scheme}, ` +
      `canvas ${report.tokens.canvas}, layout ${report.layout.width}x${report.layout.height}`,
    );
    ws.close();
  } finally {
    chrome.kill();
    await sleep(300);
    try {
      fs.rmSync(profile, { recursive: true, force: true });
    } catch {}
  }
}

main().catch((error) => {
  console.error('web-style-check failed:', error.message);
  process.exit(1);
});
