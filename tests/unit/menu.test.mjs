import assert from 'node:assert/strict';
import { test } from 'node:test';
import { golden, memoryStorage } from './golden.mjs';

globalThis.localStorage = memoryStorage();
const menu = await import('../../src/menu.js');
const { menuFor, buildMenu, kindOf, extOf, menuKind, flatActions, SAMPLES, ctx, prefs, savePrefs, noteUse } = menu;

const APPS = [{ id: 'hxd', label: 'HxD', kinds: ['any'] }, { id: 'ghidra', label: 'Ghidra', kinds: ['exe'] }, { id: 'wireshark', label: 'Wireshark', kinds: ['pcap'] }];
const CASES = {
  png: ['C:\\p\\a.png'], heic: ['a.HEIC'], raw: ['a.cr3'], gif: ['a.gif'], images: ['a.png', 'b.jpg', 'c.webp'],
  mp4: ['a.mp4'], videos: ['a.mp4', 'b.mov'], mp3: ['a.mp3'], audios: ['a.mp3', 'b.wav'],
  pdf: ['a.pdf'], pdfs: ['a.pdf', 'b.pdf'], imageAndPdf: ['a.png', 'b.pdf'],
  txt: ['a.txt'], json: ['a.json'], csv: ['a.csv'], md: ['a.md'], html: ['a.html'], yaml: ['a.yaml'], xml: ['a.xml'], code: ['a.rs'], texts: ['a.txt', 'b.txt'],
  docx: ['a.docx'], xlsx: ['a.xlsx'], pptx: ['a.pptx'],
  exe: ['a.exe'], dll: ['a.dll'], exes: ['a.exe', 'b.exe'], zip: ['a.zip'], zips: ['a.zip', 'b.7z'], pcap: ['a.pcap'], apk: ['a.apk'],
  age: ['a.age'], other: ['a.bin'], noext: ['C:\\x\\README'], folderish: ['C:\\x\\dir'], mixed: ['a.png', 'b.mp3'], two: ['a.bin', 'b.bin'],
};

test('extOf and kindOf classify paths', () => {
  assert.equal(extOf('C:\\a.b\\c.PNG'), 'png');
  assert.equal(extOf('C:\\a.b\\README'), '');
  assert.equal(extOf('/home/u/.bashrc'), 'bashrc');
  golden('kinds', Object.fromEntries(Object.entries(CASES).map(([k, p]) => [k, { kind: menuKind(p), kinds: p.map(kindOf) }])));
});

test('buildMenu: every action the wheel can offer, per file kind', () => {
  golden('build-menu', Object.fromEntries(Object.entries(CASES).map(([k, p]) => [k, buildMenu(p, APPS)])));
});

test('menuFor: what the wheel shows by default', () => {
  localStorage.clear();
  golden('menu-for', Object.fromEntries(Object.entries(CASES).map(([k, p]) => [k, menuFor(p, APPS)])));
});

test('menuFor: never more than 8 wedges, More holds the rest and ends with All…', () => {
  localStorage.clear();
  for (const [k, p] of Object.entries(CASES)) {
    const m = menuFor(p, APPS);
    assert.ok(m.items.length <= 8, `${k}: ${m.items.length} wedges`);
    const more = m.items.find((it) => it.more);
    if (more) assert.equal(more.sub.at(-1).label, 'All…', k);
  }
});

test('usage moves an action onto the wheel and hidden ones disappear', () => {
  localStorage.clear();
  const before = menuFor(['a.png'], APPS);
  const extra = before.all.find((it) => !before.items.some((x) => x.id === it.id));
  assert.ok(extra, 'png has actions beyond the wheel');
  for (let i = 0; i < 3; i++) noteUse('image', extra.id);
  assert.ok(menuFor(['a.png'], APPS).items.some((it) => it.id === extra.id), 'used action is on the wheel');
  const p = prefs();
  p.hidden = { image: [extra.id] };
  savePrefs(p);
  assert.ok(!menuFor(['a.png'], APPS).items.some((it) => it.id === extra.id), 'hidden action is off the wheel');
  localStorage.clear();
});

test('recipes and custom commands appear for matching files only', () => {
  localStorage.clear();
  savePrefs({ recipes: [{ id: 'r1', name: 'Web', kind: 'image', steps: [{ name: 'Resize', action: { op: 'resize', max: 1600 } }] }], commands: [{ id: 'c1', name: 'Echo', exts: 'txt md', command: 'echo {file}' }] });
  const img = menuFor(['a.png'], APPS).all.map((i) => i.id);
  assert.ok(img.includes('recipe-r1') && !img.includes('cmd-c1'));
  const txt = menuFor(['a.txt'], APPS).all;
  assert.deepEqual(txt.find((i) => i.id === 'cmd-c1').action, { op: 'custom', command: 'echo {file}' });
  localStorage.clear();
});

test('a broken preference store does not break the wheel', () => {
  localStorage.setItem('rotae.menu', '{not json');
  assert.ok(menuFor(['a.png'], APPS).items.length > 0);
  localStorage.clear();
});

test('every action the UI can send, for the Rust side to check it parses', () => {
  localStorage.clear();
  ctx.pentest = null;
  const seen = new Map();
  // fields the hub asks for before running (password, size…) are filled in the way it does
  const add = (a, ask) => {
    if (!a) return;
    const v = { ...a };
    for (const f of ask || []) v[f.key] = f.number ? 8 : 'x';
    seen.set(JSON.stringify(v), v);
  };
  for (const p of [...Object.values(CASES), ...Object.values(SAMPLES)]) {
    const m = menuFor(p, APPS);
    add(m.center?.action, m.center?.ask);
    for (const it of flatActions(m.all)) add(it.action, it.ask);
  }
  const all = [...seen.values()].sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  golden('actions', all);
  assert.ok(all.length > 40);
});
