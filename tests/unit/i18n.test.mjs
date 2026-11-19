import assert from 'node:assert/strict';
import { test } from 'node:test';
import { golden, memoryStorage } from './golden.mjs';

// just enough of a DOM for setLanguage(); the live translation of pages is covered by the UI tests
globalThis.document = { documentElement: {}, body: { querySelectorAll: () => [] }, createTreeWalker: () => ({ nextNode: () => null }) };
globalThis.NodeFilter = { SHOW_TEXT: 4 };
globalThis.MutationObserver = class { observe() {} };
globalThis.localStorage = memoryStorage();
Object.defineProperty(globalThis, 'navigator', { value: { language: 'en-US' }, configurable: true });

const { t, setLanguage } = await import('../../src/i18n.js');
const { menuFor, flatActions, SAMPLES } = await import('../../src/menu.js');

test('English leaves text alone', () => {
  setLanguage('en');
  assert.equal(t('Convert'), 'Convert');
  assert.equal(t(''), '');
});

test('Vietnamese: rules, ellipsis, breadcrumbs and "$" in names', () => {
  setLanguage('vi');
  assert.equal(t('Version 1.2.0'), 'Phiên bản 1.2.0');
  assert.equal(t('Page 3'), 'Trang 3');
  assert.equal(t('12 items'), '12 mục');
  assert.equal(t('Installing ffmpeg…'), 'Đang cài ffmpeg…');
  assert.equal(t('a $& b'), 'a $& b');
  setLanguage('en');
});

test('Vietnamese labels of every wheel action stay translated', () => {
  setLanguage('vi');
  const labels = new Set();
  for (const p of Object.values(SAMPLES)) for (const it of flatActions(menuFor(p).all)) labels.add(it.name);
  golden('vi-wheel', Object.fromEntries([...labels].sort().map((l) => [l, t(l)])));
  setLanguage('en');
});

test('"system" follows the computer language', () => {
  Object.defineProperty(globalThis, 'navigator', { value: { language: 'vi-VN' }, configurable: true });
  setLanguage('system');
  assert.equal(t('Page 2'), 'Trang 2');
  Object.defineProperty(globalThis, 'navigator', { value: { language: 'en-US' }, configurable: true });
  setLanguage('system');
  assert.equal(t('Page 2'), 'Page 2');
});
