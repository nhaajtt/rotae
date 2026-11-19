// Golden files: the expected output is kept next to the tests and compared on every run.
// After an intended change, refresh them with `npm run test:update` and review the diff.
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = join(dirname(fileURLToPath(import.meta.url)), 'golden');

export function golden(name, value) {
  const file = join(DIR, `${name}.json`);
  const text = `${JSON.stringify(value, null, 1)}\n`;
  if (process.env.UPDATE_GOLDEN || process.env.npm_lifecycle_event === 'test:update' || !existsSync(file)) {
    mkdirSync(DIR, { recursive: true });
    writeFileSync(file, text);
    return;
  }
  assert.deepEqual(JSON.parse(text), JSON.parse(readFileSync(file, 'utf8')), `${name}.json no longer matches; run npm run test:update if the change is intended`);
}

/** localStorage for modules that keep preferences there. */
export function memoryStorage() {
  const m = new Map();
  return {
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => m.set(k, String(v)),
    removeItem: (k) => m.delete(k),
    clear: () => m.clear(),
  };
}
