// Newest installer link, language and theme switches, the word-by-word lede and the clock.

const REPO = 'nhaajtt/rotae';
const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
const DL = {
  win: `https://github.com/${REPO}/releases/latest`,
  mac: `https://github.com/${REPO}/releases/latest/download/Rotae-mac-arm64.dmg`,
  linux: `https://github.com/${REPO}/releases/latest`,
};
let os = 'win';

fetch(`https://api.github.com/repos/${REPO}/releases/latest`)
  .then((r) => (r.ok ? r.json() : null))
  .then((rel) => {
    const exe = rel?.assets?.find((a) => /-setup\.exe$/i.test(a.name));
    if (!exe) return;
    DL.win = exe.browser_download_url;
    document.querySelectorAll('#download, .dl').forEach((a) => (a.href = DL.win));
    setOS(os);
    document.getElementById('version').textContent = `v${rel.tag_name.replace(/^v/, '')}`;
    document.querySelectorAll('.exe-size').forEach((s) => (s.textContent = `x64 · ${(exe.size / 1048576).toFixed(0)} MB`));
  })
  .catch(() => {});

// --- Windows / macOS / Linux: show the command and download for the visitor's system;
// the tabs switch it, and both sets of tabs on the page stay in step
function guessOS() {
  const p = `${navigator.userAgentData?.platform || ''} ${navigator.platform || ''} ${navigator.userAgent}`;
  if (/Mac/i.test(p)) return 'mac';
  if (/Linux|X11|CrOS/i.test(p) && !/Android/i.test(p)) return 'linux';
  return 'win';
}
const detected = guessOS();
// a Mac with an Intel chip, or an ARM Linux machine, can't run the build offered here
let archNote = '';
function setOS(which) {
  os = which;
  document.getElementById('hdr-dl').href = DL[os];
  document.querySelectorAll('[data-os-show]').forEach((n) => (n.hidden = !n.dataset.osShow.split(' ').includes(os)));
  document.querySelectorAll('[data-os]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.os === os)));
  document.querySelectorAll('.os-found').forEach((n) => (n.hidden = os !== detected || !!archNote));
  document.querySelectorAll('[data-found]').forEach((n) => (n.hidden = n.dataset.found !== os));
  document.querySelectorAll('[data-warn]').forEach((n) => (n.hidden = os !== detected || n.dataset.warn !== archNote));
}
setOS(['win', 'mac', 'linux'].includes(localStorage.getItem('os')) ? localStorage.getItem('os') : detected);
(async () => {
  try {
    const h = await navigator.userAgentData?.getHighEntropyValues?.(['architecture']);
    if (detected === 'mac' && h?.architecture === 'x86') archNote = 'intel';
    if (detected === 'linux' && h?.architecture === 'arm') archNote = 'arm';
  } catch {}
  if (!archNote && detected === 'linux' && /aarch64|arm64|armv/i.test(navigator.userAgent)) archNote = 'arm';
  if (!archNote && detected === 'mac') {
    // browsers without client hints: an Intel Mac reports an Intel, AMD or NVIDIA graphics chip
    try {
      const gl = document.createElement('canvas').getContext('webgl');
      const ext = gl?.getExtension('WEBGL_debug_renderer_info');
      if (/Intel|AMD|Radeon|NVIDIA/i.test(ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : '')) archNote = 'intel';
    } catch {}
  }
  if (archNote) setOS(os);
})();
document.querySelectorAll('[data-os]').forEach((b) =>
  b.addEventListener('click', () => {
    localStorage.setItem('os', b.dataset.os);
    setOS(b.dataset.os);
  }),
);

// --- the lede arrives word by word; highlighted phrases and links move as one piece
let wordsShown = false;
function splitWords(el) {
  let i = 0;
  const wrap = (node) => {
    const w = document.createElement('span');
    w.className = 'lab-intro-w';
    w.style.setProperty('--d', `${120 + i++ * 45}ms`);
    w.append(node);
    return w;
  };
  const out = [];
  [...el.childNodes].forEach((n) => {
    if (n.nodeType !== 3) return out.push(wrap(n));
    n.textContent.split(/(\s+)/).forEach((t) => t && out.push(/^\s+$/.test(t) ? ' ' : wrap(t)));
  });
  el.replaceChildren(...out);
  el.classList.toggle('no-anim', wordsShown || reduce);
  wordsShown = true;
}

// --- English / Tiếng Việt: elements keep their Vietnamese text in data-vi
const nodes = [...document.querySelectorAll('[data-vi]')];
nodes.forEach((n) => (n.dataset.en = n.innerHTML));
function setLang(lang) {
  const vi = lang === 'vi';
  nodes.forEach((n) => (n.innerHTML = vi ? n.dataset.vi : n.dataset.en));
  document.querySelectorAll('[data-words]').forEach(splitWords);
  document.querySelectorAll('[data-vi-ph]').forEach((n) => {
    n.dataset.enPh ??= n.placeholder;
    n.placeholder = vi ? n.dataset.viPh : n.dataset.enPh;
  });
  document.dispatchEvent(new Event('lang'));
  document.documentElement.lang = lang;
  document.querySelectorAll('[data-lang]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.lang === lang)));
  localStorage.setItem('lang', lang);
}
setLang(localStorage.getItem('lang') || ((navigator.language || '').toLowerCase().startsWith('vi') ? 'vi' : 'en'));
document.querySelectorAll('[data-lang]').forEach((b) => (b.onclick = () => setLang(b.dataset.lang)));

// --- light / dark
// a circle grows from the button into the new theme (View Transitions API where supported)
document.getElementById('theme').onclick = (e) => {
  const dark = document.documentElement.dataset.theme !== 'dark';
  const apply = () => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    localStorage.setItem('theme', dark ? 'dark' : 'light');
  };
  if (!document.startViewTransition || reduce) return apply();
  const x = e.clientX || innerWidth - 60;
  const y = e.clientY || 30;
  const r = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y));
  document.startViewTransition(apply).ready.then(() =>
    document.documentElement.animate(
      { clipPath: [`circle(0 at ${x}px ${y}px)`, `circle(${r}px at ${x}px ${y}px)`] },
      { duration: 520, easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)', pseudoElement: '::view-transition-new(root)' },
    ),
  );
};

// --- reveal on scroll
const io = new IntersectionObserver(
  (entries) =>
    entries.forEach((e) => {
      if (!e.isIntersecting) return;
      e.target.classList.add('in');
      io.unobserve(e.target);
    }),
  { rootMargin: '0px 0px -12% 0px' },
);
document.querySelectorAll('.reveal').forEach((el, i) => {
  el.style.transitionDelay = `${(i % 2) * 70}ms`;
  io.observe(el);
});

// --- numbers count up once
const counter = new IntersectionObserver((entries) =>
  entries.forEach((e) => {
    if (!e.isIntersecting) return;
    counter.unobserve(e.target);
    const el = e.target;
    const to = Number(el.dataset.to);
    const suffix = el.dataset.suffix || '';
    if (reduce || to === 0) return (el.textContent = `${to}${suffix}`);
    const start = performance.now();
    const tick = (now) => {
      const p = Math.min(1, (now - start) / 1200);
      el.textContent = `${Math.round(to * (1 - Math.pow(1 - p, 3)))}${suffix}`;
      if (p < 1) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }),
);
document.querySelectorAll('.count').forEach((el) => counter.observe(el));

// --- the clock in the readout
const clock = document.getElementById('clock');
const tick = () => (clock.textContent = new Date().toLocaleTimeString('en-US'));
tick();
setInterval(tick, 1000);

// --- phones: send the download page to a PC; no autoplay on mobile data
const phone = matchMedia('(max-width: 880px)').matches;
const page = 'https://rotae.nhaajt.com';
async function shareLink(btn) {
  const vi = document.documentElement.lang === 'vi';
  const text = vi ? 'Rotae: kéo file, giữ Shift. Cài trên máy tính Windows hoặc Linux:' : 'Rotae: drag a file, hold Shift. Install it on a Windows or Linux computer:';
  try {
    if (navigator.share) return await navigator.share({ title: 'Rotae', text, url: page });
  } catch {
    return;
  }
  copyLink(btn);
}
async function copyText(btn, text) {
  const vi = document.documentElement.lang === 'vi';
  try {
    await navigator.clipboard.writeText(text);
    const old = btn.textContent;
    btn.textContent = vi ? 'Đã chép ✓' : 'Copied ✓';
    setTimeout(() => (btn.textContent = old), 1600);
  } catch {}
}
const copyLink = (btn) => copyText(btn, page);
document.querySelectorAll('.copy-cmd').forEach((b) =>
  b.addEventListener('click', () => copyText(b, b.parentElement.querySelector('.cmdtext').textContent)),
);
document.getElementById('shareLink')?.addEventListener('click', (e) => shareLink(e.currentTarget));
document.getElementById('copyLink')?.addEventListener('click', (e) => copyLink(e.currentTarget));
document.querySelectorAll('.share-again').forEach((b) => b.addEventListener('click', () => shareLink(b)));
const heroVideo = document.getElementById('heroVideo');
if (phone && heroVideo) {
  heroVideo.autoplay = false;
  heroVideo.preload = 'none';
  heroVideo.pause();
  heroVideo.controls = true;
}

// --- videos pause when off screen; the film starts by itself once half of it is on screen
document.querySelectorAll('video').forEach((v) => {
  const auto = 'auto' in v.dataset && !reduce && !phone;
  new IntersectionObserver(([e]) => {
    if (!e.isIntersecting) v.pause();
    else if (v.autoplay || (auto && e.intersectionRatio >= 0.5)) v.play().catch(() => {});
  }, { threshold: [0, 0.5] }).observe(v);
});

// --- timeline: years across, one bar per thing, a line at today
const T0 = 2024;
const YEARS = 6;
const PX = 150;
const BARS = [
  { from: 2026.82, row: 0, cls: 'is-current', en: ['Rotae 1.0', 'Windows · 2026'], vi: ['Rotae 1.0', 'Windows · 2026'] },
  { from: 2026.86, row: 1, cls: '', en: ['One-line install', 'PowerShell'], vi: ['Cài bằng một dòng lệnh', 'PowerShell'] },
  { from: 2026.87, row: 3, cls: 'is-current', en: ['Rotae for Linux', 'Ubuntu · Kali · Fedora'], vi: ['Rotae cho Linux', 'Ubuntu · Kali · Fedora'] },
  { from: 2026.88, row: 2, cls: 'is-current', en: ['Rotae for Mac', 'Apple silicon · 1.2'], vi: ['Rotae cho Mac', 'chip Apple · 1.2'] },
  { from: 2024.76, row: 3, cls: 'is-muted', href: 'https://tauri.app', en: ['Tauri 2', 'the shell'], vi: ['Tauri 2', 'khung app'] },
  { from: 2025.6, row: 4, cls: 'is-muted is-faint', href: 'https://ffmpeg.org', en: ['FFmpeg', 'video and audio add-on'], vi: ['FFmpeg', 'tiện ích video, nhạc'] },
];
const tlTrack = document.getElementById('tlTrack');
const tlScroll = tlTrack.parentElement;
const tlX = (year) => 24 + (year - T0) * PX;
function buildTimeline() {
  const vi = document.documentElement.lang === 'vi';
  const d = new Date();
  const now = tlX(d.getFullYear() + (d.getMonth() + d.getDate() / 31) / 12);
  const mk = (tag, cls, left) => {
    const n = document.createElement(tag);
    n.className = cls;
    if (left != null) n.style.left = `${left}px`;
    return n;
  };
  const out = [];
  for (let y = 0; y < YEARS; y++) {
    const tick = mk('span', tlX(T0 + y) > now ? 'tl-tick is-future' : 'tl-tick', tlX(T0 + y));
    tick.textContent = T0 + y;
    out.push(tick, mk('span', 'tl-grid', tlX(T0 + y)));
    if (y < YEARS - 1) for (let q = 1; q < 4; q++) out.push(mk('span', 'tl-grid tl-grid--q', tlX(T0 + y + q / 4)));
  }
  out.push(mk('span', 'tl-future', now));
  const line = mk('span', 'tl-now', now);
  line.append(document.createElement('i'));
  out.push(line);
  BARS.forEach((b) => {
    const bar = mk(b.href ? 'a' : 'div', `tl-bar ${b.cls}`, tlX(b.from));
    if (b.href) Object.assign(bar, { href: b.href, target: '_blank', rel: 'noopener' });
    bar.style.top = `${49 + b.row * 56}px`;
    const [name, sub] = vi ? b.vi : b.en;
    const label = document.createElement('b');
    label.textContent = name;
    const note = document.createElement('span');
    note.textContent = sub;
    bar.append(label, note);
    out.push(bar);
  });
  tlTrack.style.width = `${tlX(T0 + YEARS - 1) + 24}px`;
  tlTrack.style.height = `${49 + 5 * 56}px`;
  tlTrack.replaceChildren(...out);
  tlScroll.scrollLeft = now - tlScroll.clientWidth / 2;
}
buildTimeline();
document.addEventListener('lang', buildTimeline);
// drag to move through the years
let dragFrom = null;
tlScroll.addEventListener('pointerdown', (e) => {
  if (e.pointerType !== 'mouse' || e.target.closest('a')) return;
  dragFrom = e.clientX + tlScroll.scrollLeft;
  tlScroll.classList.add('is-drag');
});
addEventListener('pointermove', (e) => dragFrom != null && (tlScroll.scrollLeft = dragFrom - e.clientX));
addEventListener('pointerup', () => ((dragFrom = null), tlScroll.classList.remove('is-drag')));

// --- jump palette: Ctrl+K, type to filter, arrows and Enter
const JUMPS = [
  ['#top', 'Top', 'Đầu trang'],
  ['#demo', 'Demo', 'Demo'],
  ['#timeline', 'Timeline', 'Dòng thời gian'],
  ['#features', 'What it does', 'Làm được gì'],
  ['#how', 'Three steps', 'Ba bước'],
  ['#private', 'Privacy', 'Riêng tư'],
  ['#film', 'The 26-second film', 'Phim 26 giây'],
  ['#faq', 'FAQ', 'FAQ'],
  ['#install', 'Install', 'Cài đặt'],
  ['#feedback', 'Feedback', 'Góp ý'],
  ['https://github.com/nhaajtt/rotae', 'Source on GitHub', 'Mã nguồn trên GitHub'],
];
const jump = document.getElementById('jump');
const jumpIn = document.getElementById('jumpIn');
const jumpList = document.getElementById('jumpList');
let jumpAt = 0;
function drawJump() {
  const vi = document.documentElement.lang === 'vi';
  const q = jumpIn.value.trim().toLowerCase();
  const rows = JUMPS.filter((r) => `${r[1]} ${r[2]}`.toLowerCase().includes(q));
  jumpAt = Math.max(0, Math.min(jumpAt, rows.length - 1));
  jumpList.replaceChildren(
    ...rows.map((r, i) => {
      const li = document.createElement('li');
      li.setAttribute('role', 'option');
      li.setAttribute('aria-selected', String(i === jumpAt));
      li.dataset.to = r[0];
      li.textContent = vi ? r[2] : r[1];
      const hint = document.createElement('span');
      hint.textContent = r[0].startsWith('#') ? r[0] : '↗';
      li.append(hint);
      return li;
    }),
  );
  jumpList.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
}
function goJump(to) {
  if (!to) return;
  jump.close();
  if (to.startsWith('#')) document.querySelector(to)?.scrollIntoView();
  else window.open(to, '_blank', 'noopener');
}
function openJump() {
  if (jump.open) return jump.close();
  jumpIn.value = '';
  jumpAt = 0;
  drawJump();
  jump.showModal();
}
document.getElementById('jumpOpen').addEventListener('click', openJump);
addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') (e.preventDefault(), openJump());
});
jumpIn.addEventListener('input', () => ((jumpAt = 0), drawJump()));
jumpIn.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') (e.preventDefault(), (jumpAt += e.key === 'ArrowDown' ? 1 : -1), drawJump());
  if (e.key === 'Enter') goJump(jumpList.querySelector('[aria-selected="true"]')?.dataset.to);
});
jumpList.addEventListener('click', (e) => goJump(e.target.closest('li')?.dataset.to));
jump.addEventListener('click', (e) => e.target === jump && jump.close());

// --- feedback: what's written goes to this site's own /api/feedback and nowhere else
const fbForm = document.getElementById('fb-form');
fbForm.addEventListener('submit', async (event) => {
  event.preventDefault();
  const vi = document.documentElement.lang === 'vi';
  const send = document.getElementById('fb-send');
  const note = document.getElementById('fb-note');
  const label = send.textContent;
  send.disabled = true;
  send.textContent = vi ? 'Đang gửi…' : 'Sending…';
  let said = vi ? 'Chưa gửi được. Bạn thử lại sau nhé.' : "That didn't go through. Try again in a while.";
  try {
    const reply = await fetch('/api/feedback', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        text: document.getElementById('fb-text').value,
        contact: document.getElementById('fb-contact').value,
        language: vi ? 'vi' : 'en',
        version: (document.getElementById('version').textContent.match(/v[\d.]+/) || [''])[0],
      }),
    });
    if (reply.ok) {
      said = vi ? 'Đã gửi. Cảm ơn bạn.' : 'Sent. Thank you.';
      fbForm.reset();
    } else if (reply.status === 429) said = vi ? 'Hôm nay như vậy là đủ rồi. Cảm ơn bạn.' : 'That is enough for today. Thank you.';
  } catch {}
  note.textContent = said;
  send.disabled = false;
  send.textContent = label;
});
