import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fixFor, runFix } from '../../src/fixes.js';
import { golden } from './golden.mjs';

// Error texts the engines really produce (see src-tauri/src/errors and the engines' bail! messages).
const ERRORS = [
  'Video and audio actions need FFmpeg, which isn’t installed yet. (need_ffmpeg)',
  'need_ffmpeg',
  'this PDF is already as small as it gets',
  'report.pdf: this PDF is already as small as it gets',
  'a.png: unsupported file type',
  'access is denied',
  'The process cannot access the file because it is being used by another process. (os error 32)',
  'not valid JSON',
  'wrong password',
  'the file is encrypted',
  'no space left on device',
  'timed out',
  'This PC isn’t on a private local network',
  'took too long to read this picture',
  'settings file was busy',
  "couldn't protect the key",
  'none of these could be uploaded',
  'something nobody has seen before',
  '',
  'first line\nsecond line',
];

test('fixFor turns engine errors into a why/how card', () => {
  const out = Object.fromEntries(ERRORS.map((e) => [e, fixFor(e)]));
  for (const [e, f] of Object.entries(out)) {
    assert.equal(typeof f.why, 'string', e);
    assert.equal(typeof f.how, 'string', e);
    assert.ok(f.why.length > 0 && f.how.length > 0, e);
  }
  golden('fixes', out);
});

test('fixFor: unknown errors offer a report', () => {
  assert.equal(fixFor('zzz').action.kind, 'report');
  assert.equal(fixFor(undefined).action.kind, 'report');
});

test('runFix calls the matching backend command', async () => {
  const calls = [];
  const invoke = async (cmd, args) => calls.push([cmd, args]);
  await runFix({ kind: 'addon', id: 'ffmpeg' }, invoke);
  await runFix({ kind: 'settings' }, invoke);
  await runFix({ kind: 'url', url: 'https://example.com' }, invoke);
  await runFix({ kind: 'report' }, invoke, 'ctx');
  await runFix(null, invoke);
  assert.deepEqual(calls, [
    ['tools_install', { id: 'ffmpeg' }],
    ['open_hub', { paths: [], view: 'settings', item: null }],
    ['open_link', { url: 'https://example.com' }],
    ['report_bug', { context: 'ctx' }],
  ]);
});
