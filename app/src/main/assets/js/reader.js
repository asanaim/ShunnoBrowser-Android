// Shunno Reader (Safari Reader-style). Injected on demand together with Mozilla Readability
// (`Readability` is in scope). The original page is never modified: the article is shown in an
// overlay with a closed shadow root and is fully sanitised before insertion.
// The browser fills in the saved appearance (JSON object) below.

if (window.__shunnoReader) {
  window.__shunnoReader.toggle();
  return;
}

const PREFS_DEFAULT = { size: 19, font: 'serif', theme: 'auto', width: 'normal' };
let prefs = Object.assign({}, PREFS_DEFAULT, __SHUNNO_READER_PREFS__ || {});
const notify = (kind, data) => {
  try { if (window.__shunnoReaderNotify) window.__shunnoReaderNotify(kind, data); } catch (e) {}
};

// ---------------------------------------------------------------- sanitising
const TAGS = new Set(['P', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'A', 'IMG', 'FIGURE', 'FIGCAPTION', 'PICTURE', 'SOURCE',
  'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'CODE', 'EM', 'STRONG', 'B', 'I', 'U', 'S', 'SUB', 'SUP', 'BR', 'HR',
  'TABLE', 'THEAD', 'TBODY', 'TFOOT', 'TR', 'TH', 'TD', 'CAPTION', 'SPAN', 'DIV', 'SECTION', 'ARTICLE', 'SMALL',
  'MARK', 'TIME', 'ABBR', 'CITE', 'Q', 'DL', 'DT', 'DD', 'DEL', 'INS', 'KBD', 'SAMP', 'VAR']);
const DROP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'IFRAME', 'OBJECT', 'EMBED', 'FORM', 'INPUT', 'BUTTON', 'TEXTAREA',
  'SELECT', 'SVG', 'MATH', 'TEMPLATE', 'LINK', 'META', 'BASE', 'VIDEO', 'AUDIO', 'CANVAS', 'DIALOG']);
const ATTRS = new Set(['href', 'src', 'srcset', 'alt', 'title', 'colspan', 'rowspan', 'dir', 'lang', 'width', 'height', 'datetime', 'start', 'reversed', 'type', 'media', 'sizes']);
const safeUrl = (v, img) => {
  const s = String(v || '').trim();
  if (/^(https?:|mailto:|#)/i.test(s)) return true;
  if (img && /^data:image\/(png|jpe?g|gif|webp|avif);/i.test(s)) return true;
  return false;
};
function clean(node, outDoc) {
  if (node.nodeType === 3) return outDoc.createTextNode(node.nodeValue);
  if (node.nodeType !== 1) return null;
  const tag = node.tagName.toUpperCase();
  if (DROP.has(tag)) return null;
  const frag = outDoc.createDocumentFragment();
  if (!TAGS.has(tag)) {
    // Unknown wrapper: keep its children only.
    node.childNodes.forEach((c) => { const x = clean(c, outDoc); if (x) frag.appendChild(x); });
    return frag;
  }
  const el = outDoc.createElement(tag.toLowerCase());
  for (const a of Array.from(node.attributes)) {
    const n = a.name.toLowerCase();
    if (!ATTRS.has(n)) continue;
    if ((n === 'href' || n === 'src') && !safeUrl(a.value, tag === 'IMG' || tag === 'SOURCE')) continue;
    if (n === 'srcset' && !a.value.split(',').every((p) => safeUrl(p.trim().split(/\s+/)[0], true))) continue;
    el.setAttribute(n, a.value);
  }
  if (tag === 'A') { el.setAttribute('rel', 'noopener noreferrer'); }
  if (tag === 'IMG') { el.setAttribute('loading', 'lazy'); el.setAttribute('decoding', 'async'); el.removeAttribute('width'); el.removeAttribute('height'); }
  node.childNodes.forEach((c) => { const x = clean(c, outDoc); if (x) el.appendChild(x); });
  return el;
}
function sanitize(html) {
  const parsed = new DOMParser().parseFromString('<body>' + html + '</body>', 'text/html');
  const out = document.createDocumentFragment();
  parsed.body.childNodes.forEach((c) => { const x = clean(c, document); if (x) out.appendChild(x); });
  return out;
}

// ---------------------------------------------------------------- extraction
function extract() {
  const doc = document.cloneNode(true);
  // Lazy-loaded images: promote common data-* sources before Readability runs.
  doc.querySelectorAll('img').forEach((img) => {
    const lazy = img.getAttribute('data-src') || img.getAttribute('data-lazy-src') || img.getAttribute('data-original') || img.getAttribute('data-url');
    if (lazy && (!img.getAttribute('src') || /^data:/.test(img.getAttribute('src')))) img.setAttribute('src', lazy);
    const lazySet = img.getAttribute('data-srcset');
    if (lazySet && !img.getAttribute('srcset')) img.setAttribute('srcset', lazySet);
  });
  // Resolve relative links and images against the page before cloning loses the base.
  doc.querySelectorAll('a[href]').forEach((a) => { try { a.setAttribute('href', new URL(a.getAttribute('href'), location.href).href); } catch (e) {} });
  doc.querySelectorAll('img[src]').forEach((i) => { try { i.setAttribute('src', new URL(i.getAttribute('src'), location.href).href); } catch (e) {} });
  try {
    return new Readability(doc, { charThreshold: 400, keepClasses: false }).parse();
  } catch (e) {
    return null;
  }
}

// ---------------------------------------------------------------- UI
const CSS = `
:host { all: initial; }
.root { position: fixed; inset: 0; overflow-y: auto; z-index: 2147483646; --bg: #fff; --fg: #1d1d1f; --muted: #6e6e73; --rule: rgba(0,0,0,.1); --link: #0066cc; --panel: rgba(255,255,255,.92);
  background: var(--bg); color: var(--fg); -webkit-font-smoothing: antialiased; animation: rin .28s cubic-bezier(.32,.72,0,1) both; }
@keyframes rin { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
.root.closing { animation: rout .16s cubic-bezier(.4,0,1,1) both; }
@keyframes rout { to { opacity: 0; } }
.root.t-sepia { --bg: #f8f1e3; --fg: #4f321c; --muted: #8b6f57; --rule: rgba(79,50,28,.15); --link: #9a5b1c; --panel: rgba(248,241,227,.94); }
.root.t-gray { --bg: #4a4a4d; --fg: #e8e8ea; --muted: #b5b5b8; --rule: rgba(255,255,255,.14); --link: #8fc4ff; --panel: rgba(74,74,77,.94); }
.root.t-dark { --bg: #121214; --fg: #d9d9de; --muted: #8e8e93; --rule: rgba(255,255,255,.12); --link: #64a8ff; --panel: rgba(30,30,32,.94); }
@media (prefers-color-scheme: dark) { .root.t-auto { --bg: #121214; --fg: #d9d9de; --muted: #8e8e93; --rule: rgba(255,255,255,.12); --link: #64a8ff; --panel: rgba(30,30,32,.94); } }
.bar { position: sticky; top: 0; display: flex; justify-content: flex-end; gap: 8px; padding: 12px 16px; z-index: 2; pointer-events: none; }
.bar button { pointer-events: auto; border: none; height: 32px; min-width: 32px; padding: 0 12px; border-radius: 16px; cursor: pointer;
  font: 600 13px -apple-system, "Segoe UI Variable Text", "Segoe UI", sans-serif; color: var(--fg); background: var(--panel);
  box-shadow: 0 1px 4px rgba(0,0,0,.12), 0 0 0 .5px var(--rule); backdrop-filter: blur(12px); }
.panel { position: fixed; top: 54px; right: 16px; width: 260px; padding: 14px; border-radius: 14px; background: var(--panel); backdrop-filter: blur(20px);
  box-shadow: 0 12px 40px rgba(0,0,0,.22), 0 0 0 .5px var(--rule); font: 13px -apple-system, "Segoe UI Variable Text", "Segoe UI", sans-serif; display: none; z-index: 3; }
.panel.open { display: block; animation: rin .2s cubic-bezier(.32,.72,0,1) both; }
.row { display: flex; gap: 6px; margin-bottom: 10px; }
.row:last-child { margin-bottom: 0; }
.row button { flex: 1; height: 34px; border-radius: 9px; border: 1px solid var(--rule); background: transparent; color: var(--fg); cursor: pointer; font: inherit; }
.row button.on { border-color: var(--link); box-shadow: inset 0 0 0 1px var(--link); }
.sw { height: 30px !important; border-radius: 50% !important; flex: 0 0 30px !important; }
.sw.light { background: #fff !important; } .sw.sepia { background: #f8f1e3 !important; } .sw.gray { background: #4a4a4d !important; } .sw.dark { background: #121214 !important; }
.sw.auto { background: linear-gradient(135deg, #fff 50%, #121214 50%) !important; }
article.rd { margin: 0 auto; padding: 8px 28px 96px; max-width: var(--w, 680px); font-size: var(--fs, 19px); line-height: 1.65; }
article.rd.serif { font-family: "Iowan Old Style", Charter, Georgia, "Nirmala UI", "Noto Serif Bengali", serif; }
article.rd.sans { font-family: -apple-system, "Segoe UI Variable Text", "Segoe UI", "Nirmala UI", "Noto Sans Bengali", sans-serif; }
.site { font: 600 13px -apple-system, "Segoe UI", sans-serif; letter-spacing: .02em; color: var(--muted); text-transform: uppercase; margin: 8px 0 10px; }
h1.title { font-size: 1.75em; line-height: 1.2; margin: 0 0 12px; font-weight: 700; letter-spacing: -.01em; }
.meta { color: var(--muted); font-size: .8em; margin-bottom: 28px; padding-bottom: 20px; border-bottom: 1px solid var(--rule); font-family: -apple-system, "Segoe UI", sans-serif; }
.body h1, .body h2, .body h3, .body h4 { line-height: 1.3; margin: 1.6em 0 .5em; }
.body h2 { font-size: 1.3em; } .body h3 { font-size: 1.12em; }
.body p { margin: 0 0 1.05em; }
.body a { color: var(--link); text-decoration: none; } .body a:hover { text-decoration: underline; }
.body img { display: block; max-width: 100%; height: auto; margin: 1.3em auto; border-radius: 6px; }
.body figure { margin: 1.5em 0; } .body figcaption { color: var(--muted); font-size: .8em; text-align: center; margin-top: -.6em; }
.body blockquote { margin: 1.3em 0; padding: 0 0 0 1em; border-left: 3px solid var(--rule); color: var(--muted); }
.body pre { overflow-x: auto; padding: 12px 14px; border-radius: 8px; background: var(--rule); font-size: .82em; line-height: 1.5; }
.body code { font-family: "Cascadia Code", Consolas, monospace; font-size: .9em; }
.body table { border-collapse: collapse; width: 100%; font-size: .85em; margin: 1.2em 0; display: block; overflow-x: auto; }
.body th, .body td { border: 1px solid var(--rule); padding: 6px 9px; text-align: left; }
.body ul, .body ol { padding-left: 1.4em; margin: 0 0 1.05em; } .body li { margin: .3em 0; }
.body hr { border: none; border-top: 1px solid var(--rule); margin: 2em 0; }
@media (prefers-reduced-motion: reduce) { .root, .root.closing, .panel.open { animation: none; } }
`;

let host = null, root = null, articleEl = null, panel = null, savedOverflow = '';

function applyPrefs() {
  if (!root) return;
  root.className = 'root t-' + prefs.theme;
  articleEl.className = 'rd ' + (prefs.font === 'sans' ? 'sans' : 'serif');
  articleEl.style.setProperty('--fs', Math.max(14, Math.min(30, prefs.size)) + 'px');
  articleEl.style.setProperty('--w', prefs.width === 'narrow' ? '580px' : prefs.width === 'wide' ? '820px' : '680px');
  panel.querySelectorAll('[data-k]').forEach((b) => b.classList.toggle('on', String(prefs[b.dataset.k]) === b.dataset.v));
}
function setPref(k, v) {
  prefs[k] = v;
  applyPrefs();
  notify('prefs', prefs);
}

function open() {
  const art = extract();
  if (!art || !art.content || (art.length || 0) < 200) {
    notify('state', false);
    return false;
  }
  host = document.createElement('div');
  host.style.cssText = 'position:fixed;inset:0;z-index:2147483647;';
  const shadow = host.attachShadow({ mode: 'closed' });
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(CSS);
  shadow.adoptedStyleSheets = [sheet];
  root = document.createElement('div');
  const bar = document.createElement('div');
  bar.className = 'bar';
  const mk = (label, title, fn) => { const b = document.createElement('button'); b.textContent = label; b.title = title; b.addEventListener('click', fn); return b; };
  bar.appendChild(mk('Aa', 'Text and appearance', () => panel.classList.toggle('open')));
  bar.appendChild(mk('✕', 'Close Reader (Esc or F9)', close));
  panel = document.createElement('div');
  panel.className = 'panel';
  const row = (items) => { const r = document.createElement('div'); r.className = 'row'; items.forEach((i) => r.appendChild(i)); return r; };
  const opt = (k, v, label, extra) => { const b = document.createElement('button'); b.textContent = label; b.dataset.k = k; b.dataset.v = v; if (extra) b.className = extra; b.title = label || v; b.addEventListener('click', () => setPref(k, v)); return b; };
  const sizeBtn = (d, label) => { const b = document.createElement('button'); b.textContent = label; b.addEventListener('click', () => setPref('size', Math.max(14, Math.min(30, prefs.size + d)))); return b; };
  panel.appendChild(row([sizeBtn(-1, 'A−'), sizeBtn(1, 'A+')]));
  panel.appendChild(row([opt('font', 'serif', 'Serif'), opt('font', 'sans', 'Sans-serif')]));
  panel.appendChild(row([opt('width', 'narrow', 'Narrow'), opt('width', 'normal', 'Normal'), opt('width', 'wide', 'Wide')]));
  panel.appendChild(row(['auto', 'light', 'sepia', 'gray', 'dark'].map((t) => opt('theme', t, '', 'sw ' + t))));
  articleEl = document.createElement('article');
  if (art.dir) articleEl.setAttribute('dir', art.dir);
  if (art.lang) articleEl.setAttribute('lang', art.lang);
  const site = document.createElement('div');
  site.className = 'site';
  site.textContent = art.siteName || location.hostname.replace(/^www\./, '');
  const h1 = document.createElement('h1');
  h1.className = 'title';
  h1.textContent = art.title || document.title;
  const meta = document.createElement('div');
  meta.className = 'meta';
  const words = (art.textContent || '').trim().split(/\s+/).length;
  const mins = Math.max(1, Math.round(words / 220));
  meta.textContent = [art.byline, mins + ' min read'].filter(Boolean).join(' · ');
  const body = document.createElement('div');
  body.className = 'body';
  body.appendChild(sanitize(art.content));
  // Prefer the article's own heading over a "Headline | Site" document title, and never show both.
  const first = body.querySelector('h1, h2');
  if (first) {
    const ft = first.textContent.trim();
    const tt = h1.textContent.trim();
    if (ft && (ft === tt || (tt.includes(ft) && ft.length >= 8))) {
      h1.textContent = ft;
      first.remove();
    }
  }
  articleEl.append(site, h1, meta, body);
  root.append(bar, panel, articleEl);
  shadow.appendChild(root);
  shadow.addEventListener('click', (e) => { if (!panel.contains(e.target) && !e.target.closest('.bar')) panel.classList.remove('open'); });
  savedOverflow = document.documentElement.style.overflow;
  document.documentElement.style.overflow = 'hidden';
  // The floating download bar / Drop Link belong to the page view, not to Reader.
  const fh = document.getElementById('shunno-float-host');
  if (fh) { fh.dataset.readerPrev = fh.style.visibility; fh.style.visibility = 'hidden'; }
  (document.body || document.documentElement).appendChild(host);
  applyPrefs();
  root.tabIndex = -1;
  root.focus({ preventScroll: true });
  window.addEventListener('keydown', onKey, true);
  notify('state', true);
  return true;
}

function close() {
  if (!host) return;
  const h = host;
  root.classList.add('closing');
  setTimeout(() => h.remove(), 160);
  host = root = articleEl = panel = null;
  document.documentElement.style.overflow = savedOverflow;
  const fh = document.getElementById('shunno-float-host');
  if (fh && 'readerPrev' in fh.dataset) { fh.style.visibility = fh.dataset.readerPrev; delete fh.dataset.readerPrev; }
  window.removeEventListener('keydown', onKey, true);
  notify('state', false);
}

function onKey(e) {
  if (e.key === 'Escape') {
    if (panel && panel.classList.contains('open')) panel.classList.remove('open');
    else close();
    e.stopPropagation();
  }
}

Object.defineProperty(window, '__shunnoReader', {
  value: Object.freeze({ toggle: () => (host ? close() : open()), isOpen: () => !!host }),
  writable: false,
  configurable: false,
});
window.__shunnoReader.toggle();
