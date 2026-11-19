// UI flow tests: each window runs in headless Chrome against the real front-end code, with the
// Tauri backend replaced by tests/ui/mock-tauri.js. A flow drives the page like a user and checks
// which backend commands it sent.
// Usage: npm run test:ui   (CHROME=/path/to/chrome to pick the browser)
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import { flows } from './flows.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '../..');
const ONLY = process.argv[2] || '';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function chromePath() {
  if (process.env.CHROME) return process.env.CHROME;
  const c = {
    win32: ['C:/Program Files/Google/Chrome/Application/chrome.exe', 'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe', 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'],
    darwin: ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'],
    linux: ['/usr/bin/google-chrome', '/usr/bin/google-chrome-stable', '/usr/bin/chromium', '/usr/bin/chromium-browser'],
  }[process.platform] || [];
  const found = c.find(existsSync);
  if (!found) throw new Error('no Chrome found; set CHROME=/path/to/chrome');
  return found;
}

const mock = join(HERE, 'mock-tauri.js');
const aliases = ['@tauri-apps/api/core', '@tauri-apps/api/event', '@tauri-apps/api/window', '@tauri-apps/api/webview', '@tauri-apps/api/app', '@tauri-apps/plugin-updater', '@tauri-apps/plugin-process', '@crabnebula/tauri-plugin-drag'];
const server = await createServer({
  configFile: false,
  root: join(ROOT, 'src'),
  logLevel: 'error',
  clearScreen: false,
  cacheDir: join(ROOT, 'node_modules/.vite-ui-tests'),
  publicDir: join(HERE, 'public'),
  resolve: { alias: aliases.map((find) => ({ find, replacement: mock })) },
  server: { host: '127.0.0.1', port: 0, fs: { allow: [ROOT] } },
});
await server.listen();
const base = `http://127.0.0.1:${server.httpServer.address().port}`;

const profile = mkdtempSync(join(tmpdir(), 'rotae-ui-'));
const chrome = spawn(chromePath(), ['--headless=new', '--remote-debugging-port=0', '--remote-debugging-address=127.0.0.1', `--user-data-dir=${profile}`, '--no-first-run', '--mute-audio', '--disable-gpu', ...(process.platform === 'linux' ? ['--no-sandbox'] : []), 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
const wsUrl = await new Promise((res, rej) => {
  let buf = '';
  chrome.stderr.on('data', (d) => {
    buf += d;
    const m = buf.match(/DevTools listening on (ws:\/\/\S+)/);
    if (m) res(m[1]);
  });
  chrome.on('exit', () => rej(new Error(`chrome exited: ${buf.slice(-400)}`)));
  setTimeout(() => rej(new Error('chrome did not start')), 20000);
});

const ws = new WebSocket(wsUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0;
const pending = new Map();
const subs = new Set();
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) {
    const { r, j } = pending.get(d.id);
    pending.delete(d.id);
    d.error ? j(new Error(d.error.message)) : r(d.result);
  } else subs.forEach((h) => h(d));
};
const cdp = (method, params = {}, sessionId) => new Promise((r, j) => {
  const id = ++seq;
  pending.set(id, { r, j });
  ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
});

/** One page: open a window of the app with a scenario, then drive it. */
async function openPage(path, scn = {}, size = [960, 680]) {
  const { targetId } = await cdp('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await cdp('Target.attachToTarget', { targetId, flatten: true });
  const s = (m, p) => cdp(m, p, sessionId);
  const errors = [];
  const onEvent = (d) => {
    if (d.sessionId !== sessionId) return;
    if (d.method === 'Runtime.exceptionThrown') errors.push(d.params.exceptionDetails.exception?.description || d.params.exceptionDetails.text);
  };
  subs.add(onEvent);
  await s('Runtime.enable');
  await s('Page.enable');
  await s('Emulation.setDeviceMetricsOverride', { width: size[0], height: size[1], deviceScaleFactor: 1, mobile: false });
  await s('Page.addScriptToEvaluateOnNewDocument', { source: `window.__SCN = ${JSON.stringify(scn)};` });
  await s('Page.navigate', { url: base + path });
  const evaluate = async (expr) => {
    const r = await s('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`${expr.slice(0, 120)}\n→ ${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
    return r.result.value;
  };
  const waitFor = async (expr, ms = 5000) => {
    const end = Date.now() + ms;
    let last;
    while (Date.now() < end) {
      try { last = await evaluate(expr); if (last) return last; } catch { /* not there yet */ }
      await sleep(50);
    }
    throw new Error(`timed out waiting for: ${expr}`);
  };
  await waitFor(`document.readyState === 'complete' && !!window.__calls`, 15000);
  await sleep(300);
  const page = {
    eval: evaluate,
    waitFor,
    sleep,
    calls: (cmd) => evaluate(`window.__calls.filter(c => ${JSON.stringify(cmd || '')} === '' || c[0] === ${JSON.stringify(cmd || '')})`),
    emit: (name, payload) => evaluate(`window.__emit(${JSON.stringify(name)}, ${JSON.stringify(payload)})`),
    click: (selector) => evaluate(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) throw new Error('no element ${selector.replace(/'/g, '')}'); e.click(); return true; })()`),
    clickText: (text, selector = 'button, a, [role=button], .row, li') => evaluate(`(() => { const e = [...document.querySelectorAll(${JSON.stringify(selector)})].find(x => x.textContent.trim().includes(${JSON.stringify(text)})); if (!e) throw new Error('no element with text ${text.replace(/'/g, '')}'); e.click(); return true; })()`),
    key: (key, opts = {}) => evaluate(`document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: ${JSON.stringify(key)}, bubbles: true, ...${JSON.stringify(opts)} }))`),
    text: (selector) => evaluate(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`),
    errors,
    close: async () => { subs.delete(onEvent); await cdp('Target.closeTarget', { targetId }); },
  };
  return page;
}

let failed = 0;
let passed = 0;
const started = Date.now();
try {
  for (const f of flows.filter((x) => x.name.includes(ONLY))) {
    let page;
    try {
      page = await openPage(f.path, f.scn, f.size);
      await f.run(page, assert);
      assert.deepEqual(page.errors, [], 'uncaught errors in the page');
      if (f.todo) throw Object.assign(new Error('marked todo but passes now: remove todo'), { todoPassed: true });
      passed++;
      console.log(`ok   ${f.name}`);
    } catch (e) {
      if (f.todo && !e.todoPassed) {
        console.log(`todo ${f.name} (${f.todo})`);
        continue;
      }
      failed++;
      console.log(`FAIL ${f.name}\n     ${String(e.stack || e).split('\n').slice(0, 4).join('\n     ')}`);
    } finally {
      await page?.close().catch(() => {});
    }
  }
} finally {
  console.log(`\n${passed} passed, ${failed} failed (${((Date.now() - started) / 1000).toFixed(1)} s)`);
  ws.close();
  chrome.kill();
  await server.close();
  await sleep(200);
  rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
process.exit(failed ? 1 : 0);
