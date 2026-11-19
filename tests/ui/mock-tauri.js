// Fake Tauri IPC so the windows run in a normal browser for the UI tests.
// A scenario is injected before load as window.__SCN = { settings, history, hub_take, ... }.
// Every invoke is recorded in window.__calls; tests fire backend events with window.__emit(name, payload).

const SCN = (window.__SCN ??= {});
const calls = (window.__calls ??= []);
const listeners = (window.__listeners ??= {});

const SAMPLES = SCN.samples || '/samples';
const baseSettings = {
  enabled: true, explorer_only: false, peek: true, zone_edge: 'right', reduce_transparency: false,
  theme: 'light', lang: 'en', accent: 'gold', anthropic_key: '', r2_account: '', r2_bucket: '', r2_key: '',
  r2_secret: '', r2_public: '', vt_key: '', yara_rules: '', watches: [], pentest: false, pentest_consented_at: 0,
};
let settings = { ...baseSettings, ...(SCN.settings || {}) };
const now = Math.floor(Date.now() / 1000);

const fixtures = {
  get_settings: () => settings,
  update_settings: ({ patch }) => {
    settings = { ...settings, ...patch };
    window.__emit('settings', settings);
    return settings;
  },
  get_autostart: () => true,
  set_autostart: () => null,
  get_apps: () => SCN.apps ?? [{ id: 'hxd', label: 'HxD', kinds: ['any'] }],
  sandbox_ok: () => true,
  tools_status: () =>
    SCN.tools ?? ['capa', 'floss', 'upx', 'ai', 'upscale', 'ffmpeg', 'yara', 'qpdf', 'whisper'].map((id, i) => ({ id, installed: i % 3 !== 0 })),
  tools_install: () => null,
  pentest_tools: () => SCN.pentest_tools ?? null,
  clipboard_files: () => (SCN.clipboard ? SCN.clipboard : Promise.reject('no files')),
  hub_take: () => SCN.hub_take ?? null,
  get_history: () =>
    SCN.history ?? [
      { id: 3, state: 'error', title: 'Make smaller', time: now - 120, files: ['C:\\samples\\report.pdf'], outputs: [], error: 'this PDF is already as small as it gets', action: { op: 'compress' } },
      { id: 2, state: 'done', title: 'Convert to PNG', time: now - 3600, files: ['C:\\samples\\photo.heic'], outputs: ['C:\\samples\\photo.png'], note: '2.1 MB → 1.4 MB' },
      { id: 1, state: 'partial', title: 'Rename', time: now - 86400, files: ['a.txt', 'b.txt'], outputs: ['a 1.txt'], error: '1 of 2 failed' },
    ],
  clear_history: () => null,
  reveal: () => null, report_bug: () => null, open_logs: () => null, open_link: () => null,
  context_menu_on: () => false, context_menu: () => null,
  folder_exists: () => true,
  rename_files: ({ pairs }) => pairs.length,
  share_lan: () => ({ qr: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#000"/><rect x="2" y="2" width="6" height="6" fill="#fff"/></svg>', minutes: 10, url: 'http://192.168.1.2:5000/x' }),
  share_r2: () => ['https://example.com/a'],
  ui_log: () => null,
  run_action: () => null, open_editor: () => null, open_with: () => null, shelf_add: () => null, open_hub: () => null,
  overlay_hide: () => null,
  pdf_thumbs: ({ only }) => {
    const n = SCN.pages ?? 3;
    const img = `${SAMPLES}/page.png`;
    return Array.from({ length: n }, (_, i) => (only === undefined || only === i ? img : ''));
  },
  file_info: () => SCN.file_info ?? { is_dir: false, items: 0, size: 123456, modified: now - 3600 },
  read_text: () => SCN.text ?? '{\n  "hello": "world",\n  "list": [1, 2, 3]\n}\n',
  toast_height: () => null, toast_hide: () => null, toast_show: () => null,
  drag_icon: () => 'C:\\icon.png',
  trash: () => null,
  shelf_take: () => SCN.shelf ?? [],
};

export async function invoke(cmd, args = {}) {
  calls.push([cmd, args]);
  const f = fixtures[cmd];
  if (!f) throw new Error(`mock: no fixture for ${cmd}`);
  return f(args);
}

export function convertFileSrc(path) {
  const name = String(path).split(/[\\/]/).pop();
  return `${SAMPLES}/${encodeURIComponent(name)}`;
}

export async function listen(name, cb) {
  (listeners[name] ??= []).push(cb);
  return () => (listeners[name] = listeners[name].filter((x) => x !== cb));
}

window.__emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ event: name, payload }));

const win = {
  label: SCN.label || 'main',
  hide: async () => calls.push(['window.hide']),
  show: async () => calls.push(['window.show']),
  close: async () => calls.push(['window.close']),
  setFocus: async () => null,
  onFocusChanged: async () => () => {},
};
export const getCurrentWindow = () => win;
export const getCurrentWebview = () => ({
  onDragDropEvent: async (cb) => {
    window.__dragDrop = cb;
    return () => {};
  },
});
export const getVersion = async () => '1.2.0';
export const check = async () => SCN.update ?? null;
export const relaunch = async () => null;
export const startDrag = async () => calls.push(['startDrag']);
