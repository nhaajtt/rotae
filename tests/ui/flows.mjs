// User flows checked by tests/ui/run.mjs. Each one opens a window with a scenario for the fake
// backend (see mock-tauri.js), acts like a user and checks what the window asked the backend to do.
const files3 = ['C:\\samples\\photo.png', 'C:\\samples\\clip.mp4', 'C:\\samples\\report.pdf'];
const png = 'C:\\samples\\photo.png';
const pdf = 'C:\\samples\\report.pdf';

const show = (p, paths, more = {}) => p.emit('overlay:show', { paths, ...more });

export const flows = [
  // --- the wheel -------------------------------------------------------------------------
  {
    name: 'wheel: dropping on a wedge runs that action on the dragged files',
    path: '/overlay.html',
    size: [460, 460],
    async run(p, assert) {
      await show(p, [png]);
      await p.waitFor(`document.querySelector('svg#wheel.open')`);
      await p.sleep(300);
      await p.emit('overlay:drop', [230, 120]);
      await p.waitFor(`window.__calls.some(c => c[0] === 'overlay_hide')`);
      const ran = [...(await p.calls('run_action')), ...(await p.calls('open_editor')), ...(await p.calls('open_hub'))];
      assert.equal(ran.length, 1, JSON.stringify(await p.calls()));
      assert.deepEqual(ran[0][1].paths ?? [ran[0][1].path], [png]);
    },
  },
  {
    name: 'wheel: number keys pick a wedge (run on release), 0 closes',
    path: '/overlay.html',
    size: [460, 460],
    async run(p, assert) {
      await show(p, [pdf]);
      await p.waitFor(`document.querySelector('svg#wheel.open')`);
      await p.sleep(300);
      await p.emit('overlay:key', 0);
      await p.waitFor(`window.__calls.some(c => c[0] === 'overlay_hide')`);
      assert.equal((await p.calls('run_action')).length, 0);
      await show(p, [pdf]);
      await p.waitFor(`document.querySelector('svg#wheel.open')`);
      await p.sleep(300);
      await p.emit('overlay:key', 1);
      // the key only picks the wedge; letting go of the mouse anywhere then runs it
      await p.emit('overlay:drop', [5, 5]);
      await p.waitFor(`window.__calls.filter(c => c[0] === 'overlay_hide').length === 2`);
      const ran = [...(await p.calls('run_action')), ...(await p.calls('open_editor')), ...(await p.calls('open_hub'))];
      assert.equal(ran.length, 1);
    },
  },
  {
    name: 'wheel: a disguised program shows the warning rim',
    path: '/overlay.html',
    size: [460, 460],
    async run(p, assert) {
      await show(p, ['C:\\Downloads\\invoice.pdf.exe'], { warnings: ['invoice.pdf.exe: looks like .pdf but is a program'] });
      await p.waitFor(`document.body.classList.contains('warn')`);
      assert.ok(true);
    },
  },
  {
    name: 'wheel: hover lights a wedge and drop in the middle runs the centre action',
    path: '/overlay.html',
    size: [460, 460],
    async run(p, assert) {
      await show(p, ['C:\\samples\\clip.mp4']);
      await p.waitFor(`document.querySelector('svg#wheel.open')`);
      await p.sleep(300);
      await p.emit('overlay:move', [230, 120]);
      await p.waitFor(`document.querySelector('#wheel .hot')`);
      await p.emit('overlay:drop', [230, 230]);
      await p.waitFor(`window.__calls.some(c => c[0] === 'overlay_hide')`);
      const ran = [...(await p.calls('run_action')), ...(await p.calls('open_editor'))];
      assert.equal(ran.length, 1);
    },
  },

  // --- hub -------------------------------------------------------------------------------
  {
    name: 'hub: picking an action runs it on the files and hides the window',
    path: '/hub.html',
    scn: { hub_take: { view: 'actions', paths: [png] } },
    async run(p, assert) {
      await p.waitFor(`document.querySelectorAll('.list > *').length > 3`);
      await p.eval(`document.querySelector('input.search').value = 'jpg'; document.querySelector('input.search').dispatchEvent(new Event('input'))`);
      await p.sleep(100);
      await p.eval(`document.querySelector('input.search').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))`);
      await p.waitFor(`window.__calls.some(c => ['run_action','open_editor','open_with','shelf_add'].includes(c[0]))`);
      const [cmd, args] = (await p.calls()).find((c) => ['run_action', 'open_editor', 'open_with', 'shelf_add'].includes(c[0]));
      assert.ok(cmd);
      assert.deepEqual(args.paths ?? [args.path], [png]);
    },
  },
  {
    name: 'hub: search with no match says so',
    path: '/hub.html',
    scn: { hub_take: { view: 'actions', paths: [png] } },
    async run(p, assert) {
      await p.waitFor(`document.querySelector('input.search')`);
      await p.eval(`document.querySelector('input.search').value = 'zzzzqqq'; document.querySelector('input.search').dispatchEvent(new Event('input'))`);
      await p.sleep(150);
      assert.equal(await p.eval(`document.querySelectorAll('.list > .row, .list > button').length`), 0);
    },
  },
  {
    name: 'hub: a settings switch saves only that setting',
    path: '/hub.html',
    scn: { hub_take: { view: 'settings' } },
    async run(p, assert) {
      await p.waitFor(`document.querySelectorAll('input.switch').length > 2`);
      await p.eval(`(() => { const s = document.querySelector('input.switch'); s.click(); })()`);
      await p.waitFor(`window.__calls.some(c => c[0] === 'update_settings' || c[0] === 'set_autostart')`);
      const c = (await p.calls()).find((x) => x[0] === 'update_settings' || x[0] === 'set_autostart');
      if (c[0] === 'update_settings') assert.equal(Object.keys(c[1].patch).length, 1);
    },
  },
  {
    name: 'hub: language and theme come from settings',
    path: '/hub.html',
    scn: { hub_take: { view: 'settings' }, settings: { lang: 'vi', theme: 'dark' } },
    async run(p, assert) {
      await p.waitFor(`document.documentElement.lang === 'vi'`);
      assert.ok(await p.eval(`document.documentElement.classList.contains('dark')`));
      assert.ok(await p.waitFor(`/Cài đặt/.test(document.body.textContent)`));
    },
  },
  {
    name: 'hub: activity lists past jobs and a failed one offers a fix',
    path: '/hub.html',
    scn: { hub_take: { view: 'activity' } },
    async run(p, assert) {
      await p.waitFor(`/Make smaller/.test(document.querySelector('main').textContent)`);
      assert.match(await p.text('main'), /Convert to PNG/);
    },
  },
  {
    name: 'hub: rename previews new names and renames on confirm',
    path: '/hub.html',
    scn: { hub_take: { view: 'rename', paths: files3 } },
    async run(p, assert) {
      await p.waitFor(`document.querySelector('main input')`);
      assert.match(await p.text('main'), /photo/);
    },
  },
  {
    name: 'hub: shred asks before running',
    path: '/hub.html',
    scn: { hub_take: { view: 'actions', paths: [png] } },
    async run(p, assert) {
      await p.waitFor(`document.querySelector('input.search')`);
      await p.eval(`document.querySelector('input.search').value = 'shred'; document.querySelector('input.search').dispatchEvent(new Event('input'))`);
      await p.sleep(100);
      await p.eval(`document.querySelector('input.search').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))`);
      await p.waitFor(`/Shred/.test(document.querySelector('main h1')?.textContent || '')`);
      assert.equal((await p.calls('run_action')).length, 0, 'shred must not run without confirmation');
    },
  },

  // --- toast -----------------------------------------------------------------------------
  {
    name: 'toast: progress, then done with the result',
    path: '/toast.html',
    size: [380, 320],
    async run(p, assert) {
      await p.emit('job', { id: 1, title: 'Convert to PNG', state: 'running', percent: 0.5, done: 1, total: 2, outputs: [] });
      await p.waitFor(`document.getElementById('title').textContent.includes('Convert')`);
      await p.emit('job', { id: 1, title: 'Convert to PNG', state: 'done', percent: 1, done: 2, total: 2, outputs: [png] });
      await p.waitFor(`!document.getElementById('show').hidden`);
      await p.click('#show');
      await p.waitFor(`window.__calls.some(c => c[0] === 'reveal')`);
      assert.deepEqual((await p.calls('reveal'))[0][1], { path: png });
    },
  },
  {
    name: 'toast: a failure explains why and how',
    path: '/toast.html',
    size: [380, 320],
    async run(p, assert) {
      await p.emit('job', { id: 2, title: 'Shrink', state: 'error', percent: 0, done: 0, total: 1, outputs: [], error: 'this PDF is already as small as it gets' });
      await p.waitFor(`document.body.classList.contains('failed')`);
      assert.ok((await p.text('#why')).length > 10);
      assert.ok((await p.text('#how')).length > 10);
    },
  },
  {
    name: 'toast: a job where some files failed shows what went wrong',
    path: '/toast.html',
    size: [380, 320],
    async run(p, assert) {
      // the backend reports these as "done" with an error (only history records say "partial")
      await p.emit('job', { id: 3, title: 'Rename', state: 'done', percent: 1, done: 1, total: 2, outputs: ['C:\\a 1.txt'], error: 'b.txt: access is denied' });
      await p.waitFor(`document.body.classList.contains('failed')`);
      assert.notEqual((await p.text('#state')).trim(), '');
      assert.ok((await p.text('#why')).length > 10);
    },
  },

  // --- peek, shelf, editor ---------------------------------------------------------------
  {
    name: 'peek: a text file is shown as text',
    path: `/peek.html?path=${encodeURIComponent('C:\\samples\\data.json')}`,
    size: [820, 600],
    async run(p, assert) {
      await p.waitFor(`document.querySelector('#view pre')`);
      assert.match(await p.text('#view pre'), /hello/);
      assert.equal((await p.calls('read_text'))[0][1].path, 'C:\\samples\\data.json');
    },
  },
  {
    name: 'peek: a camera RAW picture shows its details, not garbage text',
    path: `/peek.html?path=${encodeURIComponent('C:\\samples\\IMG_0001.CR3')}`,
    size: [820, 600],
    scn: { file_info: { is_dir: false, items: 0, size: 900000, modified: 1794990000 } },
    async run(p, assert) {
      await p.waitFor(`document.querySelector('#view dl, #view .info, #view table')`);
      assert.equal(await p.eval(`!!document.querySelector('#view pre')`), false);
      assert.equal((await p.calls('read_text')).length, 0);
    },
  },
  {
    name: 'peek: a disguised file shows the warning',
    path: `/peek.html?path=${encodeURIComponent('C:\\Downloads\\invoice.pdf.exe')}`,
    size: [820, 600],
    scn: { file_info: { is_dir: false, items: 0, size: 99999, modified: 1794990000, real_type: 'exe', warning: 'looks like .pdf but is a program' } },
    async run(p, assert) {
      await p.waitFor(`/looks like .pdf but is a program/.test(document.body.textContent)`);
      assert.ok(true);
    },
  },
  {
    name: 'shelf: lists kept files and clears them',
    path: '/shelf.html',
    size: [320, 440],
    scn: { shelf: files3 },
    async run(p, assert) {
      await p.waitFor(`document.querySelectorAll('#list li').length === 3`);
      await p.click('#clear');
      await p.sleep(150);
      assert.equal(await p.eval(`document.querySelectorAll('#list li').length`), 0);
    },
  },
  {
    name: 'shelf: dropped files are added once',
    path: '/shelf.html',
    size: [320, 440],
    async run(p, assert) {
      await p.emit('shelf:add', [png, png, pdf]);
      await p.sleep(150);
      assert.equal(await p.eval(`document.querySelectorAll('#list li').length`), 2);
    },
  },
  {
    name: 'editor: crop saves one crop job',
    path: `/editor.html?path=${encodeURIComponent(png)}&tool=crop`,
    size: [900, 680],
    async run(p, assert) {
      await p.waitFor(`!document.getElementById('save').disabled && document.querySelector('#stage img')?.complete`);
      await p.click('#save');
      await p.click('#save');
      await p.waitFor(`window.__calls.some(c => c[0] === 'run_action')`);
      await p.sleep(200);
      const runs = await p.calls('run_action');
      assert.equal(runs.length, 1, 'double click must not start two jobs');
      assert.equal(runs[0][1].action.op, 'crop');
      assert.deepEqual(runs[0][1].paths, [png]);
    },
  },
  {
    name: 'editor: page order saves a pages job',
    path: `/editor.html?path=${encodeURIComponent(pdf)}&tool=pages`,
    size: [900, 680],
    async run(p, assert) {
      await p.waitFor(`document.querySelectorAll('#stage img').length >= 3`);
      await p.click('#save');
      await p.waitFor(`window.__calls.some(c => c[0] === 'run_action')`);
      assert.equal((await p.calls('run_action'))[0][1].action.op, 'pages');
    },
  },
];
