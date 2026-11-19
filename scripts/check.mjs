// Everything CI checks, in one command: npm run check
// Front-end unit and UI tests, the web build, then Rust format, lint and tests.
// Pass --fast to skip the Rust steps (they take minutes on a cold cache).
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const fast = process.argv.includes('--fast');
const out = mkdtempSync(join(tmpdir(), 'rotae-build-'));
const steps = [
  ['front-end unit tests', 'node', ['--test', 'tests/unit/*.test.mjs']],
  ['web build', 'npx', ['vite', 'build', '--logLevel', 'warn', '--outDir', out, '--emptyOutDir']],
  ['UI flow tests', 'node', ['tests/ui/run.mjs']],
  ...(fast
    ? []
    : [
        ['Rust format', 'cargo', ['fmt', '--check'], 'src-tauri'],
        ['Rust lint', 'cargo', ['clippy', '--all-targets', '--locked', '--', '-D', 'warnings'], 'src-tauri'],
        ['Rust tests', 'cargo', ['test', '--lib', '--locked'], 'src-tauri'],
      ]),
];

const failed = [];
for (const [name, cmd, args, cwd] of steps) {
  console.log(`\n▶ ${name}`);
  // npx and cargo are .cmd/.exe shims on Windows, so they go through the shell there; quote
  // anything with spaces (a temp folder under a user name with a space) for cmd.exe
  const win = process.platform === 'win32';
  const r = spawnSync(cmd, win ? args.map((a) => (/[\s&()^]/.test(a) ? `"${a}"` : a)) : args, { stdio: 'inherit', cwd: cwd || process.cwd(), shell: win });
  if (r.status !== 0) failed.push(name);
}
rmSync(out, { recursive: true, force: true });
console.log(failed.length ? `\n✗ failed: ${failed.join(', ')}` : '\n✓ all checks passed');
process.exit(failed.length ? 1 : 0);
