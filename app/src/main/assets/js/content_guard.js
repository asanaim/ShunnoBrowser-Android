(function() {
  const IPC_TOKEN = "__SHUNNO_IPC_TOKEN__";
  const ADBLOCK_ON = __SHUNNO_ADBLOCK__;
  const SPONSORBLOCK_ON = __SHUNNO_SPONSORBLOCK__;
  const REJECT_COOKIES_ON = __SHUNNO_REJECT_COOKIES__;
  const BASKET_ON = __SHUNNO_BASKET__;
  const BG_MEDIA_ON = __SHUNNO_BG_MEDIA__;
  const FULL_DARK_ON = __SHUNNO_FULL_DARK__;
  const IMAGES_DISABLED = __SHUNNO_IMAGES_DISABLED__;
  const FP_ON = __SHUNNO_FP__;
  const FP_SESSION = "__SHUNNO_FP_SESSION__";
  window.__shunnoPeekOn = __SHUNNO_PEEK__;

  // 0. Fingerprinting protection ("farbling", the approach Brave uses).
  // Each site sees small, stable-per-site, different-per-launch changes to the values trackers hash
  // (canvas pixels, WebGL GPU strings, audio samples, CPU core count). Pages work normally, but a
  // fingerprint taken on one site cannot be matched on another or after a restart.
  // Wrappers are Proxies around the native functions, so Function.prototype.toString still reports
  // native code. Limits: Web Workers and some sandboxed about:blank frames are not covered.
  if (FP_ON) {
    try {
      const siteKey = (() => {
        try {
          const anc = location.ancestorOrigins;
          const topOrigin = anc && anc.length ? anc[anc.length - 1] : location.origin;
          const host = new URL(topOrigin).hostname;
          const parts = host.split('.');
          return parts.length > 2 ? parts.slice(-2).join('.') : host;
        } catch (e) { return location.hostname || 'local'; }
      })();
      let h = 2166136261 >>> 0;
      const keyStr = FP_SESSION + '|' + siteKey;
      for (let i = 0; i < keyStr.length; i++) { h ^= keyStr.charCodeAt(i); h = Math.imul(h, 16777619) >>> 0; }
      const seed = h;
      const rnd = (n) => {
        let t = (seed + Math.imul(n >>> 0, 0x6D2B79F5)) >>> 0;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
      };
      const replaceMethod = (proto, name, makeImpl) => {
        if (!proto) return;
        const desc = Object.getOwnPropertyDescriptor(proto, name);
        if (!desc || typeof desc.value !== 'function') return;
        const orig = desc.value;
        const impl = makeImpl(orig);
        Object.defineProperty(proto, name, {
          value: new Proxy(orig, { apply: (target, thisArg, args) => impl(thisArg, args) }),
          configurable: desc.configurable, enumerable: desc.enumerable, writable: desc.writable
        });
      };
      const replaceGetter = (proto, name, makeGet) => {
        if (!proto) return;
        const desc = Object.getOwnPropertyDescriptor(proto, name);
        if (!desc || typeof desc.get !== 'function') return;
        const origGet = desc.get;
        const impl = makeGet(origGet);
        Object.defineProperty(proto, name, {
          get: new Proxy(origGet, { apply: (target, thisArg) => impl(thisArg) }),
          set: desc.set, configurable: desc.configurable, enumerable: desc.enumerable
        });
      };

      // Canvas: flip the lowest bit of one channel in a few seeded pixel positions.
      const farblePixels = (data, w, hgt) => {
        const total = w * hgt;
        if (!total || data.length < total * 4) return;
        const n = Math.max(1, Math.min(48, total >>> 6));
        for (let i = 0; i < n; i++) {
          const px = Math.floor(rnd(i * 2 + 1) * total);
          const ch = Math.floor(rnd(i * 2 + 2) * 3);
          data[px * 4 + ch] ^= 1;
        }
      };
      const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
      replaceMethod(CanvasRenderingContext2D.prototype, 'getImageData', (orig) => (ctx, args) => {
        const img = Reflect.apply(orig, ctx, args);
        try { farblePixels(img.data, img.width, img.height); } catch (e) {}
        return img;
      });
      // Exports go through a farbled copy; the visible canvas is never modified.
      const farbledCopy = (canvas) => {
        const w = canvas.width, hgt = canvas.height;
        if (!w || !hgt || w * hgt > 4000000) return null;
        const copy = document.createElement('canvas');
        copy.width = w; copy.height = hgt;
        const cctx = copy.getContext('2d');
        if (!cctx) return null;
        cctx.drawImage(canvas, 0, 0);
        const img = Reflect.apply(origGetImageData, cctx, [0, 0, w, hgt]);
        farblePixels(img.data, w, hgt);
        cctx.putImageData(img, 0, 0);
        return copy;
      };
      replaceMethod(HTMLCanvasElement.prototype, 'toDataURL', (orig) => (canvas, args) => {
        let copy = null;
        try { copy = farbledCopy(canvas); } catch (e) {}
        return Reflect.apply(orig, copy || canvas, args);
      });
      replaceMethod(HTMLCanvasElement.prototype, 'toBlob', (orig) => (canvas, args) => {
        let copy = null;
        try { copy = farbledCopy(canvas); } catch (e) {}
        return Reflect.apply(orig, copy || canvas, args);
      });

      // WebGL: hide the exact GPU model, add pixel noise to readPixels.
      const glVendor = 'Google Inc.';
      const glRenderer = 'ANGLE (Generic GPU, Direct3D11)';
      [window.WebGLRenderingContext, window.WebGL2RenderingContext].forEach((Ctx) => {
        if (!Ctx) return;
        replaceMethod(Ctx.prototype, 'getParameter', (orig) => (gl, args) => {
          const real = Reflect.apply(orig, gl, args);
          // Mask only real answers: inventing a value where the browser returns null is itself a tell.
          if (real == null) return real;
          if (args[0] === 0x9245) return glVendor;   // UNMASKED_VENDOR_WEBGL
          if (args[0] === 0x9246) return glRenderer; // UNMASKED_RENDERER_WEBGL
          return real;
        });
        replaceMethod(Ctx.prototype, 'readPixels', (orig) => (gl, args) => {
          const r = Reflect.apply(orig, gl, args);
          try {
            const out = args[6];
            if (out && out.length && args[2] * args[3] * 4 <= out.length) farblePixels(out, args[2], args[3]);
          } catch (e) {}
          return r;
        });
      });

      // Audio: multiply samples by a factor within 1e-7 of 1 (inaudible), once per buffer channel.
      const farbledAudio = new WeakMap();
      replaceMethod(window.AudioBuffer && AudioBuffer.prototype, 'getChannelData', (orig) => (buf, args) => {
        const data = Reflect.apply(orig, buf, args);
        try {
          if (data.length <= 500000) {
            let done = farbledAudio.get(buf);
            if (!done) { done = new Set(); farbledAudio.set(buf, done); }
            const ch = args[0] | 0;
            if (!done.has(ch)) {
              done.add(ch);
              const f = 1 + (rnd(9001 + ch) - 0.5) * 2e-7;
              for (let i = 0; i < data.length; i++) data[i] *= f;
            }
          }
        } catch (e) {}
        return data;
      });
      replaceMethod(window.AnalyserNode && AnalyserNode.prototype, 'getFloatFrequencyData', (orig) => (node, args) => {
        const r = Reflect.apply(orig, node, args);
        try {
          const arr = args[0];
          const f = 1 + (rnd(9100) - 0.5) * 2e-7;
          for (let i = 0; i < arr.length; i++) arr[i] *= f;
        } catch (e) {}
        return r;
      });

      // CPU cores: a stable per-site value between 2 and the real count.
      replaceGetter(window.Navigator && Navigator.prototype, 'hardwareConcurrency', (origGet) => (nav) => {
        const real = Reflect.apply(origGet, nav, []) || 4;
        if (real <= 2) return real;
        return 2 + Math.floor(rnd(4242) * (real - 1));
      });
    } catch (e) {}
  }

  // Capture the native channel at document start, before any page script runs.
  // window.ipc forwards to window.chrome.webview.postMessage, which a page could replace later
  // to read the token; holding our own bound reference keeps the token out of page code.
  const NATIVE_POST = (function () {
    try {
      const wv = window.chrome && window.chrome.webview;
      if (wv && typeof wv.postMessage === 'function') return wv.postMessage.bind(wv);
    } catch (e) {}
    return null;
  })();

  function postSecure(msg) {
    const payload = IPC_TOKEN + "|" + msg;
    if (NATIVE_POST) {
      NATIVE_POST(payload);
    } else if (window.ipc) {
      window.ipc.postMessage(payload);
    }
  }

  // WebView2 runs this script when the global object is created, before <html> exists.
  // Appending to document.head/documentElement at that moment throws and used to abort the
  // whole script (floating bar, page metadata, find, shortcuts). Defer until a root exists.
  function appendWhenReady(el) {
    const target = document.head || document.documentElement;
    if (target) {
      target.appendChild(el);
      return;
    }
    const obs = new MutationObserver(() => {
      const t = document.head || document.documentElement;
      if (t) {
        obs.disconnect();
        t.appendChild(el);
      }
    });
    obs.observe(document, { childList: true, subtree: true });
  }

  function formatBytesJS(bytes) {
    const b = Number(bytes);
    if (!b || isNaN(b) || b <= 0) return null;
    const kb = 1024, mb = kb * 1024, gb = mb * 1024;
    if (b >= gb) return (b / gb).toFixed(2) + ' GB';
    if (b >= mb) return (b / mb).toFixed(1) + ' MB';
    if (b >= kb) return (b / kb).toFixed(0) + ' KB';
    return b + ' B';
  }

  // 1. Background Media Playback (Page Visibility API lock)
  if (BG_MEDIA_ON) {
    try {
      Object.defineProperty(document, 'hidden', { get: () => false, configurable: true });
      Object.defineProperty(document, 'visibilityState', { get: () => 'visible', configurable: true });
      window.addEventListener('visibilitychange', e => e.stopImmediatePropagation(), true);
      document.addEventListener('visibilitychange', e => e.stopImmediatePropagation(), true);
    } catch (e) {}
  }

  // 2. (Mobile site is now a real HTTP User-Agent set natively; no JS spoofing here.)

  // 3. Dark mode: no CSS filters. The browser sets the page's preferred colour scheme natively
  //    (prefers-color-scheme), so sites with a real dark theme use it with their true colours;
  //    "Darken All Sites" uses Chromium's built-in auto dark mode, which keeps images intact.

  // 4. Image Blocker
  if (IMAGES_DISABLED) {
    const st = document.createElement('style');
    st.textContent = 'img, picture, svg, video[poster], [style*="background-image"] { display: none !important; background-image: none !important; }';
    appendWhenReady(st);
  }

  // 5. Reader availability (Safari-style Reader button). The Reader itself (Mozilla Readability +
  //    UI) is injected by the browser only when opened, so normal pages do not carry it.
  //    isProbablyReaderable: Mozilla Readability 0.6.0, Apache-2.0 (Copyright (c) 2010 Arc90 Inc).
  (function readerAvailability() {
    var module;
    var REGEXPS={unlikelyCandidates:/-ad-|ai2html|banner|breadcrumbs|combx|comment|community|cover-wrap|disqus|extra|footer|gdpr|header|legends|menu|related|remark|replies|rss|shoutbox|sidebar|skyscraper|social|sponsor|supplemental|ad-break|agegate|pagination|pager|popup|yom-remote/i,okMaybeItsACandidate:/and|article|body|column|content|main|shadow/i};function isNodeVisible(e){return(!e.style||e.style.display!="none")&&!e.hasAttribute("hidden")&&(!e.hasAttribute("aria-hidden")||e.getAttribute("aria-hidden")!="true"||e.className&&e.className.includes&&e.className.includes("fallback-image"))}function isProbablyReaderable(e,t={}){typeof t=="function"&&(t={visibilityChecker:t});var u={minScore:20,minContentLength:140,visibilityChecker:isNodeVisible};t=Object.assign(u,t);var r=e.querySelectorAll("p, pre, article"),i=e.querySelectorAll("div > br");if(i.length){var n=new Set(r);[].forEach.call(i,function(a){n.add(a.parentNode)}),r=Array.from(n)}var l=0;return[].some.call(r,function(a){if(!t.visibilityChecker(a))return!1;var s=a.className+" "+a.id;if(REGEXPS.unlikelyCandidates.test(s)&&!REGEXPS.okMaybeItsACandidate.test(s)||a.matches("li p"))return!1;var c=a.textContent.trim().length;return c<t.minContentLength?!1:(l+=Math.sqrt(c-t.minContentLength),l>t.minScore)})}typeof module=="object"&&(module.exports=isProbablyReaderable);
    // A fresh document never starts in Reader.
    if (window.top === window) postSecure('READER_STATE:0');
    let last = null;
    const check = () => {
      if (!document.body || window.top !== window) return;
      let ok = false;
      try { ok = isProbablyReaderable(document, { minContentLength: 140, minScore: 20 }); } catch (e) {}
      if (ok !== last) { last = ok; postSecure('READER_AVAILABLE:' + (ok ? 1 : 0)); }
    };
    const schedule = () => { setTimeout(check, 600); setTimeout(check, 2500); };
    if (document.readyState === 'complete') schedule(); else window.addEventListener('load', schedule, { once: true });
    // Single-page sites change articles without a reload.
    const wrap = (k) => { const o = history[k]; history[k] = function () { const r = o.apply(this, arguments); setTimeout(check, 1200); return r; }; };
    try { wrap('pushState'); wrap('replaceState'); } catch (e) {}
    window.addEventListener('popstate', () => setTimeout(check, 1200));
    // The on-demand Reader reports through this fixed entry point (state and appearance only).
    Object.defineProperty(window, '__shunnoReaderNotify', {
      value: function (kind, data) {
        if (kind === 'state') postSecure('READER_STATE:' + (data ? 1 : 0));
        else if (kind === 'prefs' && data && typeof data === 'object') {
          const p = {
            size: Math.max(14, Math.min(30, Number(data.size) || 19)),
            font: data.font === 'sans' ? 'sans' : 'serif',
            theme: ['auto', 'light', 'sepia', 'gray', 'dark'].includes(data.theme) ? data.theme : 'auto',
            width: ['narrow', 'normal', 'wide'].includes(data.width) ? data.width : 'normal',
          };
          postSecure('READER_PREFS:' + JSON.stringify(p));
        }
      },
      writable: false,
      configurable: false,
    });
  })();

  // 6. YouTube Pre-Roll & Song-Switch Ad Stripper + Stream Harvester
  const isYouTube = /(^|\.)youtube\.com$|(^|\.)youtu\.be$/i.test(location.hostname);
  window.__shunnoYtStreams = [];
  const probedSizes = new Map();

  function harvestYtStreams(obj) {
    try {
      const sd = obj && obj.streamingData;
      if (!sd) return;
      const all = [].concat(sd.formats || [], sd.adaptiveFormats || []);
      const rawTitle = (obj.videoDetails && obj.videoDetails.title) || document.title.replace(' - YouTube', '') || 'YouTube_Video';
      const title = rawTitle.trim();
      const found = [];
      for (const f of all) {
        if (f && f.url && String(f.url).startsWith('http')) {
          const mime = String(f.mimeType || '');
          const isAudio = mime.startsWith('audio/');
          const ext = isAudio ? '.M4A' : (mime.includes('webm') ? '.WEBM' : '.MP4');
          const qual = f.qualityLabel || (isAudio ? (Math.round((f.bitrate || 128000) / 1000) + 'kbps Audio') : 'Video');
          if (f.contentLength) {
            const szLabel = formatBytesJS(f.contentLength);
            if (szLabel) probedSizes.set(f.url, szLabel);
          }
          found.push({
            url: f.url,
            ext: ext,
            name: qual + ' - ' + title + ext.toLowerCase()
          });
        }
      }
      if (found.length > 0) {
        window.__shunnoYtStreams = found;
        if (window.__shunnoSyncYtStreams) window.__shunnoSyncYtStreams(found);
      }
    } catch (e) {}
  }

  function stripAdFields(obj) {
    if (!obj || typeof obj !== 'object') return obj;
    try {
      if ('streamingData' in obj) harvestYtStreams(obj);
      if ('adPlacements' in obj) { obj.adPlacements = []; delete obj.adPlacements; }
      if ('playerAds' in obj) { obj.playerAds = []; delete obj.playerAds; }
      if ('adSlots' in obj) { obj.adSlots = []; delete obj.adSlots; }
      if ('adBreakHeartbeatParams' in obj) delete obj.adBreakHeartbeatParams;
      if (obj.auxiliaryUi && obj.auxiliaryUi.messageRenderers) delete obj.auxiliaryUi.messageRenderers;
      if (obj.playerConfig && obj.playerConfig.ssapConfig) delete obj.playerConfig.ssapConfig;
      if (obj.playerConfig && obj.playerConfig.adRequestConfig) delete obj.playerConfig.adRequestConfig;
    } catch (e) {}
    return obj;
  }

  if (isYouTube && (ADBLOCK_ON || SPONSORBLOCK_ON)) {
    let _ytPlayerResp = undefined;
    try {
      Object.defineProperty(window, 'ytInitialPlayerResponse', {
        configurable: true,
        get() { return _ytPlayerResp; },
        set(val) { _ytPlayerResp = stripAdFields(val); }
      });
    } catch (e) {}

    const origParse = JSON.parse;
    JSON.parse = function(text, reviver) {
      const res = origParse.apply(this, arguments);
      if (res && typeof res === 'object' && ('adPlacements' in res || 'playerAds' in res || 'adSlots' in res || 'streamingData' in res)) {
        stripAdFields(res);
      }
      return res;
    };

    const origRespJson = Response.prototype.json;
    Response.prototype.json = function() {
      return origRespJson.apply(this, arguments).then(data => stripAdFields(data));
    };

    const origRespText = Response.prototype.text;
    Response.prototype.text = function() {
      return origRespText.apply(this, arguments).then(txt => {
        if (typeof txt === 'string' && (txt.includes('"adPlacements"') || txt.includes('"playerAds"') || txt.includes('"adSlots"'))) {
          try {
            const parsed = origParse(txt);
            stripAdFields(parsed);
            return JSON.stringify(parsed);
          } catch (e) {}
        }
        return txt;
      });
    };

    const origXhrText = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, 'responseText');
    if (origXhrText && origXhrText.get) {
      Object.defineProperty(XMLHttpRequest.prototype, 'responseText', {
        configurable: true,
        get() {
          const txt = origXhrText.get.call(this);
          if (typeof txt === 'string' && (txt.includes('"adPlacements"') || txt.includes('"playerAds"') || txt.includes('"adSlots"') || txt.includes('"streamingData"'))) {
            try {
              const parsed = origParse(txt);
              stripAdFields(parsed);
              return JSON.stringify(parsed);
            } catch (e) {}
          }
          return txt;
        }
      });
    }

    window.addEventListener('yt-navigate-finish', () => {
      window.__shunnoYtStreams = [];
      if (window.ytInitialPlayerResponse) stripAdFields(window.ytInitialPlayerResponse);
      const mp = document.getElementById('movie_player');
      if (mp && typeof mp.getPlayerResponse === 'function') {
        stripAdFields(mp.getPlayerResponse());
      }
      skipYtAdImmediately();
    });
  }

  function skipYtAdImmediately() {
    if (!isYouTube || !SPONSORBLOCK_ON) return;
    const sk = document.querySelector('.ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-skip-ad-button, .ytp-ad-overlay-close-button');
    if (sk) sk.click();
    const player = document.getElementById('movie_player');
    const adShowing = (player && (player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting')))
      || document.querySelector('.ad-showing, .ad-interrupting');
    if (adShowing) {
      const v = document.querySelector('video');
      if (v) {
        v.muted = true;
        if (isFinite(v.duration) && v.duration > 0 && v.currentTime < v.duration - 0.1) {
          v.currentTime = v.duration;
        } else {
          v.playbackRate = 16.0;
        }
      }
    }
  }

  // 7. Network Sniffer + Built-in IDM Floating Pill (Coordinate Hit-Test on Videos + Selection + Drop Basket)
  const AD_HOSTS = [
    'doubleclick.net', 'googlesyndication.com', 'googleadservices.com',
    'adnxs.com', 'adsrvr.org', 'taboola.com', 'outbrain.com',
    'popads.net', 'popcash.net', 'propellerads.com', 'exoclick.com',
    'scorecardresearch.com', 'criteo.com', 'pubmatic.com', 'rubiconproject.com',
    'youtube.com/pagead/', 'youtube.com/api/stats/ads', 'youtube.com/ptracking',
    'youtube.com/get_midroll_'
  ];
  const MEDIA_EXTS = ['.mp4', '.mkv', '.webm', '.mov', '.m3u8', '.mpd', '.flac', '.alac', '.m4a', '.wav', '.mp3', '.ogg', '.opus'];
  const FILE_EXTS = ['.zip', '.7z', '.rar', '.tar', '.gz', '.iso', '.exe', '.msi', '.pdf', '.dmg', '.deb'];
  const ALL_EXTS = MEDIA_EXTS.concat(FILE_EXTS);
  const capturedStreams = [];

  function requestSizeProbe(targetUrl) {
    if (!targetUrl || probedSizes.has(targetUrl) || targetUrl === location.href) return;
    probedSizes.set(targetUrl, 'Probing...');
    postSecure('PROBE_STREAM:' + JSON.stringify({
      url: targetUrl,
      referer: location.href,
      cookie: document.cookie || ''
    }));
  }

  function addCapturedStream(rawUrl, kindHint) {
    if (!rawUrl || !String(rawUrl).startsWith('http')) return;
    const urlStr = String(rawUrl);
    if (urlStr.startsWith('blob:')) return;
    if (capturedStreams.some(s => s.url === urlStr)) return;
    const cleanPath = urlStr.split('?')[0].toLowerCase();
    const ext = ALL_EXTS.find(e => cleanPath.endsWith(e)) || kindHint || '.mp4';
    let labelName = 'Stream ' + ext.toUpperCase();
    try {
      const seg = cleanPath.split('/').pop();
      if (seg && seg.length > 2) labelName = decodeURIComponent(seg);
    } catch (e) {}
    capturedStreams.unshift({ url: urlStr, ext: ext.toUpperCase(), name: labelName });
    if (capturedStreams.length > 20) capturedStreams.pop();
    requestSizeProbe(urlStr);
    postSecure('STREAM:' + urlStr);
  }

  window.__shunnoOnNativeStream = function(info) {
    if (!info || !info.url || !String(info.url).startsWith('http')) return;
    const urlStr = String(info.url);
    if (capturedStreams.some(s => s.url === urlStr)) return;
    const ext = String(info.ext || '.MP4').toUpperCase();
    const name = String(info.name || (document.title || 'Media Stream')).trim();
    if (info.size_label) probedSizes.set(urlStr, info.size_label);
    capturedStreams.unshift({ url: urlStr, ext: ext, name: name });
    if (capturedStreams.length > 24) capturedStreams.pop();
  };

  window.__shunnoOnStreamProbed = function(info) {
    if (!info || !info.url) return;
    probedSizes.set(info.url, info.size_label || 'Stream');
    const item = capturedStreams.find(s => s.url === info.url);
    if (item && info.resolved_name) {
      item.name = info.resolved_name;
    }
    if (window.__shunnoRefreshFloatingPill) {
      window.__shunnoRefreshFloatingPill(info.url, info.size_label);
    }
  };

  function inspectUrl(raw) {
    if (!raw) return false;
    const str = String(raw);
    const lower = str.toLowerCase();
    const cleanLower = lower.split('?')[0];
    if (ADBLOCK_ON && AD_HOSTS.some(h => lower.includes(h))) {
      postSecure('AD_BLOCKED');
      return true;
    }
    if (str.startsWith('http')) {
      if (ALL_EXTS.some(ext => cleanLower.endsWith(ext))) {
        addCapturedStream(str);
      } else if (
        lower.includes('googlevideo.com/videoplayback')
        || lower.includes('mime=video')
        || lower.includes('mime=audio')
        || lower.includes('/manifest.mpd')
        || lower.includes('/playlist.m3u8')
      ) {
        let cleanedStream = str;
        if (lower.includes('googlevideo.com/videoplayback')) {
          cleanedStream = str.replace(/&range=[^&]+/g, '').replace(/&rbuf=[^&]+/g, '');
        }
        const hint = lower.includes('mime=audio') ? '.m4a' : (lower.includes('.m3u8') ? '.m3u8' : '.mp4');
        addCapturedStream(cleanedStream, hint);
      }
    }
    return false;
  }

  const origFetch = window.fetch;
  window.fetch = function(input, init) {
    const u = (typeof input === 'string') ? input : (input && input.url);
    if (inspectUrl(u)) return Promise.reject(new Error('Blocked by Shunno AdBlock'));
    return origFetch.apply(this, arguments);
  };

  const origOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(method, url) {
    if (inspectUrl(url)) {
      this._shunnoBlocked = true;
      return;
    }
    return origOpen.apply(this, arguments);
  };
  const origSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function() {
    if (this._shunnoBlocked) return;
    return origSend.apply(this, arguments);
  };

  function triggerNativeDownload(targetUrl, threads, via) {
    if (!targetUrl) return;
    postSecure('FLOAT_DL:' + JSON.stringify({
      url: targetUrl,
      threads: threads || 16,
      via: via || '',
      referer: location.href,
      cookie: document.cookie || ''
    }));
  }

  window.__shunnoTogglePiP = async function() {
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
        return;
      }
      const vids = Array.from(document.querySelectorAll('video')).filter(v => v.readyState >= 1 && v.videoWidth > 0);
      if (vids.length > 0) {
        await vids[0].requestPictureInPicture();
      }
    } catch (e) {}
  };

  function installFloatingOverlay() {
    if (!document.documentElement || document.getElementById('shunno-float-host')) return;

    const host = document.createElement('div');
    host.id = 'shunno-float-host';
    host.style.cssText = 'position:fixed;top:0;left:0;width:0;height:0;z-index:2147483647;pointer-events:none;';
    const shadow = host.attachShadow({ mode: 'open' });

    const styleEl = document.createElement('style');
    styleEl.textContent = `
      @keyframes shunnoPopIn {
        from { opacity: 0; transform: translateY(5px) scale(0.95); }
        to   { opacity: 1; transform: translateY(0) scale(1); }
      }
      .pill-wrap {
        position: fixed;
        pointer-events: auto;
        display: none;
        align-items: center;
        gap: 4px;
        background: rgba(20, 20, 24, 0.95);
        color: #FFFFFF;
        border: 1px solid rgba(255, 255, 255, 0.25);
        border-radius: 10px;
        padding: 4px 8px;
        font-family: system-ui, "Segoe UI", sans-serif;
        font-size: 12px;
        font-weight: 600;
        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.42);
        animation: shunnoPopIn 0.2s cubic-bezier(0.22, 1, 0.36, 1);
      }
      .pill-main {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        cursor: pointer;
        padding: 2px 5px;
        border-radius: 6px;
      }
      .pill-main:hover { background: #0A84FF; }
      .pip-btn {
        background: rgba(255,255,255,0.14);
        color: #fff;
        border: none;
        border-radius: 6px;
        padding: 3px 7px;
        font-size: 11px;
        font-weight: 700;
        cursor: pointer;
      }
      .pip-btn:hover { background: #30D158; color: #000; }
      .badge {
        background: #30D158;
        color: #000;
        font-size: 10px;
        font-weight: 800;
        padding: 1px 6px;
        border-radius: 5px;
      }
      .menu {
        position: fixed;
        pointer-events: auto;
        display: none;
        flex-direction: column;
        gap: 6px;
        min-width: 300px;
        max-width: 400px;
        max-height: 320px;
        overflow-y: auto;
        background: rgba(20, 20, 24, 0.97);
        color: #fff;
        border: 1px solid rgba(255, 255, 255, 0.25);
        border-radius: 12px;
        padding: 10px;
        font-family: system-ui, "Segoe UI", sans-serif;
        font-size: 12px;
        box-shadow: 0 14px 36px rgba(0, 0, 0, 0.50);
      }
      .menu-hdr {
        display: flex;
        justify-content: space-between;
        align-items: center;
        font-weight: 700;
        font-size: 12px;
        color: #30D158;
        border-bottom: 1px solid rgba(255,255,255,0.14);
        padding-bottom: 6px;
      }
      .row {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 8px;
        padding: 6px 7px;
        border-radius: 8px;
        background: rgba(255,255,255,0.08);
      }
      .row:hover { background: rgba(10,132,255,0.35); }
      .name {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        flex: 1;
        font-size: 11.5px;
      }
      .go {
        background: #0A84FF;
        color: #fff;
        border: none;
        border-radius: 6px;
        padding: 4px 9px;
        font-size: 11px;
        font-weight: 700;
        cursor: pointer;
        flex-shrink: 0;
      }
      .go:hover { background: #30D158; color: #000; }
      .basket {
        position: fixed;
        right: 16px;
        bottom: 16px;
        pointer-events: auto;
        display: ${BASKET_ON ? 'flex' : 'none'};
        align-items: center;
        gap: 6px;
        background: rgba(10, 132, 255, 0.92);
        color: #fff;
        border: 1.5px dashed rgba(255, 255, 255, 0.7);
        border-radius: 14px;
        padding: 7px 12px;
        font-family: system-ui, "Segoe UI", sans-serif;
        font-size: 11.5px;
        font-weight: 700;
        box-shadow: 0 8px 24px rgba(10, 132, 255, 0.35);
        cursor: pointer;
      }
      .basket.dragover {
        background: #30D158;
        color: #000;
        transform: scale(1.06);
        border-style: solid;
      }
    `;
    let styledViaSheet = false;
    try {
      if ('adoptedStyleSheets' in shadow && typeof CSSStyleSheet === 'function') {
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(styleEl.textContent);
        shadow.adoptedStyleSheets = [sheet];
        styledViaSheet = true;
      }
    } catch (e) {}
    if (!styledViaSheet) shadow.appendChild(styleEl);

    const pillWrap = document.createElement('div');
    pillWrap.className = 'pill-wrap';
    pillWrap.style.display = 'none';

    const pillMain = document.createElement('div');
    pillMain.className = 'pill-main';

    const badge = document.createElement('span');
    badge.className = 'badge';
    badge.textContent = '0↓';

    const pillText = document.createElement('span');
    pillText.textContent = 'Download';

    const caret = document.createElement('span');
    caret.style.opacity = '0.75';
    caret.textContent = '▾';

    pillMain.appendChild(badge);
    pillMain.appendChild(pillText);
    pillMain.appendChild(caret);

    const pipBtn = document.createElement('button');
    pipBtn.className = 'pip-btn';
    pipBtn.textContent = '📺 PiP';
    pipBtn.title = 'Picture-in-Picture';
    pipBtn.onclick = (e) => {
      e.stopPropagation();
      window.__shunnoTogglePiP();
    };

    pillWrap.appendChild(pillMain);
    pillWrap.appendChild(pipBtn);

    const menu = document.createElement('div');
    menu.className = 'menu';
    menu.style.display = 'none';

    const basket = document.createElement('div');
    basket.className = 'basket';
    basket.title = 'Drag & drop any link or URL here to start a 16-thread download';
    const bIcon = document.createElement('span');
    bIcon.textContent = '0↓';
    const basketLabel = document.createElement('span');
    basketLabel.textContent = 'Drop Link';
    basket.appendChild(bIcon);
    basket.appendChild(basketLabel);

    shadow.appendChild(pillWrap);
    shadow.appendChild(menu);
    shadow.appendChild(basket);
    document.documentElement.appendChild(host);
    window.__shunnoSetDropBasket = function(enabled) { basket.style.display = enabled ? 'flex' : 'none'; };

    let activeItems = [];
    let activeBaseLabel = 'Download';
    let hideTimer = null;

    basket.addEventListener('dragover', (e) => {
      e.preventDefault();
      e.stopPropagation();
      basket.classList.add('dragover');
      basketLabel.textContent = 'Release to Get!';
    });
    basket.addEventListener('dragleave', (e) => {
      e.preventDefault();
      basket.classList.remove('dragover');
      basketLabel.textContent = 'Drop Link';
    });
    basket.addEventListener('drop', (e) => {
      e.preventDefault();
      e.stopPropagation();
      basket.classList.remove('dragover');
      const dropped = (e.dataTransfer && (e.dataTransfer.getData('text/uri-list') || e.dataTransfer.getData('text/plain'))) || '';
      const urls = dropped.split(/\r?\n/).map(s => s.trim()).filter(s => s.startsWith('http'));
      if (urls.length > 0) {
        urls.forEach(u => triggerNativeDownload(u, 16));
        basketLabel.textContent = 'Queued (' + urls.length + ')!';
        setTimeout(() => { basketLabel.textContent = 'Drop Link'; }, 1400);
      } else {
        basketLabel.textContent = 'Drop Link';
      }
    });
    try {
      const savedPos = JSON.parse(localStorage.getItem('shunno_basket_pos') || 'null');
      if (savedPos && typeof savedPos.left === 'number' && typeof savedPos.top === 'number') {
        basket.style.right = 'auto';
        basket.style.bottom = 'auto';
        basket.style.left = Math.min(window.innerWidth - 110, Math.max(8, savedPos.left)) + 'px';
        basket.style.top = Math.min(window.innerHeight - 44, Math.max(8, savedPos.top)) + 'px';
      }
    } catch (e) {}

    let basketDragging = false;
    let basketMoved = false;
    let dragOffX = 0;
    let dragOffY = 0;

    basket.addEventListener('mousedown', (e) => {
      if (e.button !== 0) return;
      basketDragging = true;
      basketMoved = false;
      const r = basket.getBoundingClientRect();
      dragOffX = e.clientX - r.left;
      dragOffY = e.clientY - r.top;
      e.preventDefault();
    });

    window.addEventListener('mousemove', (e) => {
      if (!basketDragging) return;
      basketMoved = true;
      const nx = Math.min(window.innerWidth - 110, Math.max(8, e.clientX - dragOffX));
      const ny = Math.min(window.innerHeight - 44, Math.max(8, e.clientY - dragOffY));
      basket.style.right = 'auto';
      basket.style.bottom = 'auto';
      basket.style.left = nx + 'px';
      basket.style.top = ny + 'px';
    }, { passive: true });

    window.addEventListener('mouseup', () => {
      if (!basketDragging) return;
      basketDragging = false;
      if (basketMoved) {
        const r = basket.getBoundingClientRect();
        try {
          localStorage.setItem('shunno_basket_pos', JSON.stringify({ left: Math.round(r.left), top: Math.round(r.top) }));
        } catch (e) {}
      }
    });

    basket.addEventListener('click', () => {
      if (!basketMoved) postSecure('KEY:j');
    });

    window.__shunnoRefreshFloatingPill = function(url, sizeLabel) {
      if (activeItems.length === 1 && activeItems[0].url === url && pillWrap.style.display !== 'none') {
        pillText.textContent = activeBaseLabel + ' (' + sizeLabel + ')';
      }
    };

    function scheduleHide(delayMs) {
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => {
        if (!pillWrap.matches(':hover') && !menu.matches(':hover')) {
          pillWrap.style.display = 'none';
          menu.style.display = 'none';
        }
      }, delayMs || 1200);
    }

    pillWrap.addEventListener('mouseenter', () => clearTimeout(hideTimer));
    pillWrap.addEventListener('mouseleave', () => scheduleHide());
    menu.addEventListener('mouseenter', () => clearTimeout(hideTimer));
    menu.addEventListener('mouseleave', () => scheduleHide());

    function showPillAt(x, y, baseLabel, items, isVideo) {
      if (!items || items.length === 0) return;
      clearTimeout(hideTimer);
      activeItems = items;
      activeBaseLabel = baseLabel;
      pipBtn.style.display = isVideo ? 'inline-block' : 'none';
      items.slice(0, 6).forEach(it => { if (!it.via) requestSizeProbe(it.url); });

      if (items.length === 1) {
        const sz = probedSizes.get(items[0].url);
        pillText.textContent = sz ? (baseLabel + ' (' + sz + ')') : baseLabel;
      } else {
        pillText.textContent = baseLabel + ' (' + items.length + ')';
      }

      const left = Math.min(window.innerWidth - 270, Math.max(8, x));
      const top = Math.min(window.innerHeight - 48, Math.max(8, y));
      pillWrap.style.left = left + 'px';
      pillWrap.style.top = top + 'px';
      pillWrap.style.display = 'inline-flex';
    }

    pillMain.addEventListener('click', (e) => {
      e.stopPropagation();
      if (activeItems.length === 1) {
        triggerNativeDownload(activeItems[0].url, 16, activeItems[0].via);
        pillWrap.style.display = 'none';
        return;
      }
      while (menu.firstChild) menu.removeChild(menu.firstChild);

      const hdr = document.createElement('div');
      hdr.className = 'menu-hdr';
      const hdrTitle = document.createElement('span');
      hdrTitle.textContent = '⚡ Downloads (' + activeItems.length + ' items)';
      const closeBtn = document.createElement('span');
      closeBtn.style.cssText = 'cursor:pointer;color:#aaa;padding:0 4px;';
      closeBtn.textContent = '✕';
      closeBtn.onclick = () => { menu.style.display = 'none'; };
      hdr.appendChild(hdrTitle);
      hdr.appendChild(closeBtn);
      menu.appendChild(hdr);

      activeItems.forEach((it) => {
        const row = document.createElement('div');
        row.className = 'row';
        const b = document.createElement('span');
        b.className = 'badge';
        const sz = probedSizes.get(it.url);
        b.textContent = it.ext.replace('.', '') + (sz ? (' • ' + sz) : '');
        const nm = document.createElement('span');
        nm.className = 'name';
        nm.title = it.url;
        nm.textContent = it.name;
        const go = document.createElement('button');
        go.className = 'go';
        go.textContent = it.via ? 'Get' : '16T';
        go.onclick = () => {
          triggerNativeDownload(it.url, 16, it.via);
          menu.style.display = 'none';
          pillWrap.style.display = 'none';
        };
        row.appendChild(b);
        row.appendChild(nm);
        row.appendChild(go);
        menu.appendChild(row);
      });

      menu.style.left = pillWrap.style.left;
      menu.style.top = (parseFloat(pillWrap.style.top) + 36) + 'px';
      menu.style.display = 'flex';
    });

    // Items offered for one media element. Streaming players (blob:/MSE, HLS, DASH) have no direct
    // file URL, so the first item hands the page URL to yt-dlp, exactly like IDM's "Download this video".
    function buildMediaItems(m) {
      if (isYouTube && window.__shunnoYtStreams.length === 0) {
        try {
          if (window.ytInitialPlayerResponse) stripAdFields(window.ytInitialPlayerResponse);
          const mp = document.getElementById('movie_player');
          if (mp && typeof mp.getPlayerResponse === 'function') stripAdFields(mp.getPlayerResponse());
        } catch (e) {}
      }
      const isAud = m.tagName === 'AUDIO';
      const items = [];
      const directSrc = m.currentSrc || m.src || '';
      const hasDirectFile = /^https?:/i.test(directSrc);
      if (hasDirectFile) addCapturedStream(directSrc, isAud ? '.m4a' : '.mp4');
      m.querySelectorAll('source').forEach(s => {
        if (s.src && /^https?:/i.test(s.src)) addCapturedStream(s.src);
      });
      const title = (document.title || (isAud ? 'Audio' : 'Video')).replace(' - YouTube', '').trim();
      if (!hasDirectFile || isYouTube) {
        if (!isAud) {
          items.push({
            url: location.href,
            ext: '.VIDEO',
            name: (isYouTube ? 'Best quality: ' : 'This video: ') + title,
            via: 'page'
          });
        }
        // Audio only, resolved by yt-dlp from the page. Original keeps the source codec (no re-encode);
        // FLAC is a lossless copy of that decoded audio, larger but not higher quality.
        items.push({ url: location.href, ext: '.AUDIO', name: 'Audio, original quality: ' + title, via: 'page-audio' });
        items.push({ url: location.href, ext: '.FLAC', name: 'Audio as FLAC: ' + title, via: 'page-flac' });
      }
      if (hasDirectFile && !items.some(x => x.url === directSrc)) {
        items.push({ url: directSrc, ext: isAud ? '.M4A' : '.MP4', name: title });
      }
      (window.__shunnoYtStreams || []).forEach(ys => {
        if (!items.some(x => x.url === ys.url)) items.push(ys);
      });
      capturedStreams.forEach(s => {
        if (!items.some(x => x.url === s.url)) items.push(s);
      });
      return items;
    }

    function mediaIsShowable(m) {
      const r = m.getBoundingClientRect();
      if (m.tagName === 'AUDIO') return r.width >= 140 && r.height >= 20;
      return r.width >= 160 && r.height >= 90 && r.bottom > 0 && r.top < window.innerHeight;
    }

    function showForMedia(m, holdMs) {
      const rect = m.getBoundingClientRect();
      const items = buildMediaItems(m);
      if (items.length === 0) return false;
      const isAud = m.tagName === 'AUDIO';
      // Single item: one click downloads. Several: click opens the quality / stream list.
      const label = items.length === 1 ? (isAud ? 'Download audio' : 'Download this video') : (isAud ? 'Download audio' : 'Download video');
      showPillAt(rect.right - 265, Math.max(8, rect.top + 10), label, items, !isAud);
      if (holdMs) scheduleHide(holdMs);
      return true;
    }

    // IDM behaviour: the bar also appears by itself when a video starts playing (once per element).
    const autoShown = new WeakSet();
    document.addEventListener('play', (e) => {
      const m = e.target;
      if (!m || m.tagName !== 'VIDEO' || autoShown.has(m) || !mediaIsShowable(m)) return;
      autoShown.add(m);
      showForMedia(m, 4500);
    }, true);

    // Coordinate-based hit-testing so transparent overlays on top of <video> never block the floating bar.
    function checkPointForMediaOrLink(clientX, clientY, eventTarget) {
      const mediaEls = Array.from(document.querySelectorAll('video, audio'));
      for (const m of mediaEls) {
        const rect = m.getBoundingClientRect();
        if (mediaIsShowable(m) &&
            clientX >= rect.left && clientX <= rect.right &&
            clientY >= rect.top && clientY <= rect.bottom) {
          if (showForMedia(m)) return;
        }
      }

      const elAtPt = document.elementFromPoint(clientX, clientY) || eventTarget;
      const linkEl = elAtPt && elAtPt.closest ? elAtPt.closest('a[href]') : null;
      if (linkEl && linkEl.href && linkEl.href.startsWith('http')) {
        const cleanHref = linkEl.href.split('?')[0].toLowerCase();
        const matchedExt = ALL_EXTS.find(ext => cleanHref.endsWith(ext));
        if (matchedExt) {
          const rect = linkEl.getBoundingClientRect();
          let fname = 'File ' + matchedExt;
          try {
            fname = decodeURIComponent(cleanHref.split('/').pop() || fname);
          } catch (e) {}
          showPillAt(
            rect.left,
            Math.max(8, rect.top - 36),
            'Download ' + matchedExt.toUpperCase(),
            [{ url: linkEl.href, ext: matchedExt.toUpperCase(), name: fname }],
            false
          );
          return;
        }
      }
      scheduleHide();
    }

    document.addEventListener('mouseup', (e) => {
      setTimeout(() => {
        const sel = window.getSelection();
        if (!sel || sel.isCollapsed) return;
        const foundUrls = new Set();
        const text = sel.toString() || '';
        const urlMatches = text.match(/https?:\/\/[^\s"'<>]+/g);
        if (urlMatches) urlMatches.forEach(u => foundUrls.add(u));
        if (sel.rangeCount > 0) {
          const frag = sel.getRangeAt(0).cloneContents();
          frag.querySelectorAll('a[href]').forEach(a => {
            if (a.href && a.href.startsWith('http')) foundUrls.add(a.href);
          });
        }
        if (foundUrls.size > 0) {
          const items = Array.from(foundUrls).slice(0, 25).map(u => {
            const clean = u.split('?')[0];
            let seg = clean.split('/').pop() || 'link';
            try { seg = decodeURIComponent(seg); } catch (err) {}
            const ext = ALL_EXTS.find(ex => clean.toLowerCase().endsWith(ex)) || '.LINK';
            return { url: u, ext: ext.toUpperCase(), name: seg };
          });
          showPillAt(e.clientX + 10, e.clientY + 12, 'Download Selected', items, false);
        }
      }, 30);
    }, true);

    let lastMoveCheck = 0;
    document.addEventListener('mousemove', (e) => {
      const now = Date.now();
      if (now - lastMoveCheck < 140) return;
      lastMoveCheck = now;
      checkPointForMediaOrLink(e.clientX, e.clientY, e.target);
    }, { passive: true, capture: true });
  }

  // 7b. Element hiding (cosmetic filtering) from the native adblock engine, top frame only.
  // Constructable stylesheets are not blocked by page CSP; each rule is inserted separately so an
  // invalid selector from a filter list cannot disable the others.
  if (ADBLOCK_ON && window.top === window) {
    let cosmeticSheet = null;
    window.__shunnoApplyCosmetic = function(css) {
      if (typeof css !== 'string' || !css) return;
      try {
        if (!cosmeticSheet) {
          cosmeticSheet = new CSSStyleSheet();
          document.adoptedStyleSheets = [...document.adoptedStyleSheets, cosmeticSheet];
        }
        css.split('\n').forEach(rule => {
          if (!rule) return;
          try { cosmeticSheet.insertRule(rule, cosmeticSheet.cssRules.length); } catch (e) {}
        });
      } catch (e) {
        const st = document.createElement('style');
        st.textContent = css;
        appendWhenReady(st);
      }
    };
    const sentClasses = new Set();
    const sentIds = new Set();
    function collectAndSend(rootEl) {
      const classes = [];
      const ids = [];
      const scope = rootEl && rootEl.querySelectorAll ? rootEl : document;
      const nodes = scope.querySelectorAll('[class],[id]');
      for (let i = 0; i < nodes.length && classes.length + ids.length < 3000; i++) {
        const el = nodes[i];
        if (el.id && !sentIds.has(el.id)) { sentIds.add(el.id); ids.push(el.id); }
        const cl = el.classList;
        if (cl) for (let j = 0; j < cl.length; j++) {
          if (!sentClasses.has(cl[j])) { sentClasses.add(cl[j]); classes.push(cl[j]); }
        }
      }
      if (classes.length || ids.length) {
        postSecure('COSMETIC:' + JSON.stringify({ url: location.href, classes: classes, ids: ids }));
      }
    }
    // Page-specific rules as early as possible, generic class/id rules once the DOM exists.
    postSecure('COSMETIC:' + JSON.stringify({ url: location.href, classes: [], ids: [] }));
    let cosmeticTimer = null;
    const cosmeticObserver = new MutationObserver(() => {
      if (cosmeticTimer) return;
      cosmeticTimer = setTimeout(() => { cosmeticTimer = null; collectAndSend(document); }, 800);
    });
    const startCosmetic = () => {
      collectAndSend(document);
      if (document.body) cosmeticObserver.observe(document.body, { childList: true, subtree: true });
    };
    if (document.readyState === 'loading') window.addEventListener('DOMContentLoaded', startCosmetic, { once: true });
    else startCosmetic();
  }

  // 8. Debounced MutationObserver + Video Event Listener for YouTube Ad Skipper & Cookie Auto-Rejector
  if (ADBLOCK_ON || SPONSORBLOCK_ON || REJECT_COOKIES_ON) {
    const styleSheet = document.createElement('style');
    if (ADBLOCK_ON || SPONSORBLOCK_ON) {
      styleSheet.textContent = 'ytd-reel-shelf-renderer, ytd-rich-section-renderer, ytd-ad-slot-renderer, ytd-banner-promo-renderer, #masthead-ad, .ad-showing .ytp-ad-player-overlay, .ytp-ad-module, ins.adsbygoogle, [id^="google_ads_"], [class*="taboola"], [class*="outbrain"] { display: none !important; }';
    }
    appendWhenReady(styleSheet);

    let domScheduled = false;
    function runCleanPass() {
      domScheduled = false;
      skipYtAdImmediately();
      if (REJECT_COOKIES_ON) {
        document.querySelectorAll('#onetrust-reject-all-handler, .fc-cta-do-not-consent, [aria-label*="Reject all" i], button[id*="reject" i]').forEach(b => b.click());
      }
    }

    const observer = new MutationObserver(() => {
      if (!domScheduled) {
        domScheduled = true;
        requestAnimationFrame(runCleanPass);
      }
    });

    const startObserver = () => {
      if (document.body) {
        observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] });
        runCleanPass();
      }
    };
    if (document.body) startObserver();
    else window.addEventListener('DOMContentLoaded', startObserver, { once: true });

    document.addEventListener('loadedmetadata', skipYtAdImmediately, true);
    document.addEventListener('play', skipYtAdImmediately, true);
  }

  // 9. Audio Activity Reporter
  let lastAudioPlaying = false;
  function reportAudioState() {
    const media = Array.from(document.querySelectorAll('video, audio'));
    const playing = media.some(m => !m.paused && !m.ended && m.readyState > 2);
    if (playing !== lastAudioPlaying) {
      lastAudioPlaying = playing;
      postSecure('AUDIO_STATE:' + (playing ? '1' : '0'));
    }
  }
  document.addEventListener('play', reportAudioState, true);
  document.addEventListener('pause', reportAudioState, true);
  document.addEventListener('ended', reportAudioState, true);

  window.__shunnoSyncYtStreams = function(list) {
    if (!Array.isArray(list)) return;
    list.forEach(it => {
      if (it && it.url && !capturedStreams.some(s => s.url === it.url)) {
        capturedStreams.push(it);
        requestSizeProbe(it.url);
      }
    });
  };

  window.__shunnoGrabAllPageLinks = function() {
    const found = [];
    const seen = new Set();
    function pushItem(url, nameHint, extHint) {
      if (!url || !String(url).startsWith('http') || String(url).startsWith('blob:')) return;
      const u = String(url);
      if (seen.has(u)) return;
      seen.add(u);
      const clean = u.split('?')[0];
      let name = nameHint || 'Media / File';
      try {
        const seg = decodeURIComponent(clean.split('/').pop() || '');
        if (seg && seg.length > 2) name = seg;
      } catch (e) {}
      found.push({ url: u, name: name, ext: extHint || 'FILE' });
    }

    if (window.__shunnoYtStreams && window.__shunnoYtStreams.length > 0) {
      window.__shunnoYtStreams.forEach(s => pushItem(s.url, s.name, s.ext));
    }
    capturedStreams.forEach(s => pushItem(s.url, s.name, s.ext));

    document.querySelectorAll('a[href], video[src], audio[src], source[src]').forEach(el => {
      const raw = el.href || el.src;
      if (!raw || !String(raw).startsWith('http')) return;
      const clean = String(raw).split('?')[0].toLowerCase();
      const matchedExt = ALL_EXTS.find(ext => clean.endsWith(ext));
      if (matchedExt || el.hasAttribute('download') || el.tagName === 'VIDEO' || el.tagName === 'AUDIO') {
        const label = (el.textContent || '').trim() || el.getAttribute('download') || '';
        pushItem(raw, label, (matchedExt || '.media').toUpperCase());
      }
    });

    postSecure('GRAB_RESULTS:' + JSON.stringify(found.slice(0, 60)));
  };

  // 10. Non-Destructive Find-in-Page Range Walker & Match Counter (1 / N)
  window.__shunnoFindMatches = [];
  window.__shunnoFindIdx = -1;
  window.__shunnoFindLastQuery = '';

  window.__shunnoFindOnPage = function(query, direction) {
    const q = String(query || '').trim();
    if (!q) {
      window.__shunnoFindMatches = [];
      window.__shunnoFindIdx = -1;
      window.__shunnoFindLastQuery = '';
      if (window.getSelection) window.getSelection().removeAllRanges();
      postSecure('FIND_COUNT:0|0');
      return;
    }

    if (q !== window.__shunnoFindLastQuery || window.__shunnoFindMatches.length === 0) {
      window.__shunnoFindLastQuery = q;
      window.__shunnoFindMatches = [];
      window.__shunnoFindIdx = -1;
      const lowerQ = q.toLowerCase();
      const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          if (!node.parentElement) return NodeFilter.FILTER_REJECT;
          const tag = node.parentElement.tagName;
          if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT') return NodeFilter.FILTER_REJECT;
          return node.nodeValue && node.nodeValue.toLowerCase().includes(lowerQ)
            ? NodeFilter.FILTER_ACCEPT
            : NodeFilter.FILTER_SKIP;
        }
      });
      let n;
      while ((n = walker.nextNode())) {
        const txt = n.nodeValue;
        const lowerTxt = txt.toLowerCase();
        let pos = 0;
        while ((pos = lowerTxt.indexOf(lowerQ, pos)) !== -1) {
          const range = document.createRange();
          range.setStart(n, pos);
          range.setEnd(n, pos + q.length);
          window.__shunnoFindMatches.push(range);
          pos += q.length;
          if (window.__shunnoFindMatches.length >= 500) break;
        }
        if (window.__shunnoFindMatches.length >= 500) break;
      }
    }

    const total = window.__shunnoFindMatches.length;
    if (total === 0) {
      postSecure('FIND_COUNT:0|0');
      return;
    }

    if (direction === 'prev') {
      window.__shunnoFindIdx = window.__shunnoFindIdx <= 0 ? total - 1 : window.__shunnoFindIdx - 1;
    } else {
      window.__shunnoFindIdx = (window.__shunnoFindIdx + 1) % total;
    }

    const activeRange = window.__shunnoFindMatches[window.__shunnoFindIdx];
    try {
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(activeRange);
      const rect = activeRange.getBoundingClientRect();
      if (rect.top < 0 || rect.bottom > window.innerHeight) {
        activeRange.startContainer.parentElement.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    } catch (e) {}

    postSecure('FIND_COUNT:' + (window.__shunnoFindIdx + 1) + '|' + total);
  };

  // 11. Per-Site Cookie & Storage Cleaner
  window.__shunnoClearSiteStorage = async function() {
    try { localStorage.clear(); } catch (e) {}
    try { sessionStorage.clear(); } catch (e) {}
    try {
      const cookies = document.cookie ? document.cookie.split(';') : [];
      const hostParts = location.hostname.split('.');
      for (const c of cookies) {
        const eq = c.indexOf('=');
        const name = (eq > -1 ? c.slice(0, eq) : c).trim();
        if (!name) continue;
        document.cookie = name + '=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/';
        for (let i = 0; i < hostParts.length - 1; i++) {
          const dom = hostParts.slice(i).join('.');
          document.cookie = name + '=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/; domain=.' + dom;
        }
      }
    } catch (e) {}
    try {
      if (window.caches && caches.keys) {
        const keys = await caches.keys();
        await Promise.all(keys.map(k => caches.delete(k)));
      }
    } catch (e) {}
    try {
      if (window.indexedDB && indexedDB.databases) {
        const dbs = await indexedDB.databases();
        dbs.forEach(db => { if (db && db.name) indexedDB.deleteDatabase(db.name); });
      }
    } catch (e) {}
    postSecure('SITE_DATA_CLEARED:' + (location.hostname || 'site'));
  };

  function sendPageMeta() {
    postSecure('META:' + JSON.stringify({
      title: document.title || location.hostname || location.href,
      url: location.href,
      cookie: document.cookie || '',
      ua: navigator.userAgent || ''
    }));
  }

  if (document.documentElement) installFloatingOverlay();
  window.addEventListener('DOMContentLoaded', () => {
    if (isYouTube && window.ytInitialPlayerResponse) {
      stripAdFields(window.ytInitialPlayerResponse);
    }
    sendPageMeta();
    const titleEl = document.querySelector('title');
    if (titleEl) {
      new MutationObserver(sendPageMeta).observe(titleEl, { childList: true, characterData: true, subtree: true });
    }
    document.querySelectorAll('video source, audio source, video[src], audio[src]').forEach(el => {
      if (el.src) inspectUrl(el.src);
    });
    installFloatingOverlay();
  });
  window.addEventListener('load', sendPageMeta);
  window.addEventListener('popstate', sendPageMeta);
  window.addEventListener('hashchange', sendPageMeta);
  try {
    const _origPush = history.pushState;
    history.pushState = function() {
      const r = _origPush.apply(this, arguments);
      setTimeout(sendPageMeta, 30);
      return r;
    };
    const _origReplace = history.replaceState;
    history.replaceState = function() {
      const r = _origReplace.apply(this, arguments);
      setTimeout(sendPageMeta, 30);
      return r;
    };
  } catch (e) {}

  document.addEventListener('keydown', (e) => {
    if (e.key === 'F11') {
      e.preventDefault();
      postSecure('KEY:f11');
    } else if (e.ctrlKey && e.key === 'Tab') {
      e.preventDefault();
      postSecure(e.shiftKey ? 'KEY:prev_tab' : 'KEY:next_tab');
    } else if (e.ctrlKey && !e.shiftKey && !e.altKey) {
      const k = e.key.toLowerCase();
      if (k === 't' || k === 'w' || k === 'j' || k === 'k') {
        e.preventDefault();
        postSecure('KEY:' + k);
      } else if (e.code === 'Equal' || e.code === 'NumpadAdd' || k === '=' || k === '+') {
        e.preventDefault();
        e.stopPropagation();
        postSecure('KEY:zoom_in');
      } else if (e.code === 'Minus' || e.code === 'NumpadSubtract' || k === '-') {
        e.preventDefault();
        e.stopPropagation();
        postSecure('KEY:zoom_out');
      } else if (e.code === 'Digit0' || e.code === 'Numpad0' || k === '0') {
        e.preventDefault();
        e.stopPropagation();
        postSecure('KEY:zoom_reset');
      }
    }
  }, true);

  // Peek (Arc): Shift+click a link previews it in a floating panel instead of a new window.
  document.addEventListener('click', (e) => {
    if (!window.__shunnoPeekOn || !e.shiftKey || e.ctrlKey || e.metaKey || e.altKey || e.button !== 0) return;
    const a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a || a.hasAttribute('download')) return;
    let href = '';
    try { href = new URL(a.href, location.href).href; } catch (err) { return; }
    if (!/^https?:/i.test(href)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    postSecure('PEEK:' + href);
  }, true);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !document.fullscreenElement) postSecure('KEY_ESC');
  }, true);

  // ---------------------------------------------------------------- Password manager (Shunno vault)
  // Saves: a submitted login posts PW_OFFER; the browser asks the user before storing anything.
  // Fills: focusing a login field asks for usernames only; the password reaches the page only after
  // the user picks an account in the menu.
  (function passwordManager() {
    if (!/^https?:$/.test(location.protocol)) return;
    const shown = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    const pwFieldsIn = (scope) => Array.from((scope || document).querySelectorAll('input[type=password]')).filter(shown);
    const isTextLike = (el) => el instanceof HTMLInputElement && /^(text|email|tel|)$/i.test(el.getAttribute('type') || '');
    function userFieldFor(pw) {
      const scope = pw.form || document;
      const cands = Array.from(scope.querySelectorAll('input')).filter((el) => isTextLike(el) && shown(el));
      let best = null;
      for (const c of cands) if (c.compareDocumentPosition(pw) & Node.DOCUMENT_POSITION_FOLLOWING) best = c;
      return best || cands[0] || null;
    }
    let lastOffer = '';
    function capture(fromEl) {
      const scope = (fromEl && (fromEl.form || (fromEl.closest && fromEl.closest('form')))) || null;
      const filled = pwFieldsIn(scope).filter((p) => p.value);
      if (!filled.length) return;
      // Sign-up and change-password forms: the last filled password field is the new one.
      const pw = filled[filled.length - 1];
      const u = userFieldFor(pw);
      const payload = JSON.stringify({ u: u ? u.value.trim() : '', p: pw.value });
      if (payload === lastOffer) return;
      lastOffer = payload;
      postSecure('PW_OFFER:' + payload);
    }
    document.addEventListener('submit', (e) => capture(e.target && e.target.querySelector ? e.target.querySelector('input[type=password]') || e.target : null), true);
    document.addEventListener('click', (e) => {
      const b = e.target && e.target.closest ? e.target.closest('button, input[type=submit], [role=button]') : null;
      if (!b) return;
      const form = b.form || b.closest('form');
      const pw = (form && form.querySelector('input[type=password]')) || document.querySelector('input[type=password]');
      if (pw && pw.value) capture(pw);
    }, true);
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && e.target && e.target.type === 'password' && e.target.value) capture(e.target);
    }, true);

    // ---- fill menu
    let field = null, host = null, root = null, hideTimer = 0;
    function closeMenu() { if (host) host.style.display = 'none'; }
    function ensureMenu() {
      if (host) return;
      host = document.createElement('div');
      host.style.cssText = 'position:fixed;z-index:2147483647;display:none;';
      root = host.attachShadow({ mode: 'closed' });
      const css = new CSSStyleSheet();
      css.replaceSync(`
        .m{min-width:220px;max-width:340px;background:#fff;color:#1d1d1f;border-radius:10px;padding:4px;
           box-shadow:0 10px 30px rgba(0,0,0,.18),0 0 0 .5px rgba(0,0,0,.12);font:13px -apple-system,"Segoe UI",sans-serif}
        .h{font-size:11px;color:#6e6e73;padding:5px 9px 3px}
        .i{display:flex;gap:8px;align-items:center;padding:7px 9px;border-radius:7px;cursor:pointer}
        .i:hover{background:#0a84ff;color:#fff}
        .k{width:20px;height:20px;border-radius:5px;background:#0a84ff1f;display:flex;align-items:center;justify-content:center;font-size:11px}
        .u{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
        @media (prefers-color-scheme: dark){.m{background:#2c2c2e;color:#f5f5f7}}`);
      root.adoptedStyleSheets = [css];
      // Created on focus, so the page body exists; <head> children are never rendered.
      (document.body || document.documentElement).appendChild(host);
    }
    function place() {
      if (!field || !host) return;
      const r = field.getBoundingClientRect();
      host.style.left = Math.max(4, r.left) + 'px';
      host.style.top = (r.bottom + 4) + 'px';
    }
    window.addEventListener('scroll', () => { if (host && host.style.display !== 'none') place(); }, true);
    document.addEventListener('focusin', (e) => {
      const t = e.target;
      if (!(t instanceof HTMLInputElement)) return;
      const isPw = t.type === 'password';
      const hasPwNear = (t.form ? t.form.querySelector('input[type=password]') : document.querySelector('input[type=password]'));
      if (!isPw && !(isTextLike(t) && hasPwNear)) return;
      field = t;
      postSecure('PW_QUERY');
    }, true);
    document.addEventListener('focusout', () => { clearTimeout(hideTimer); hideTimer = setTimeout(closeMenu, 180); }, true);
    document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeMenu(); }, true);

    function setValue(el, v) {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setter.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    }
    const accounts = function (list) {
      if (!field || !Array.isArray(list) || !list.length || !document.contains(field)) return;
      ensureMenu();
      const m = document.createElement('div');
      m.className = 'm';
      const h = document.createElement('div');
      h.className = 'h';
      h.textContent = 'Shunno Passwords';
      m.appendChild(h);
      list.forEach((a) => {
        const i = document.createElement('div');
        i.className = 'i';
        const k = document.createElement('span'); k.className = 'k'; k.textContent = '🔑';
        const u = document.createElement('span'); u.className = 'u'; u.textContent = a.u || '(no username)';
        i.appendChild(k); i.appendChild(u);
        i.addEventListener('mousedown', (ev) => { ev.preventDefault(); postSecure('PW_FILL:' + a.id); closeMenu(); });
        m.appendChild(i);
      });
      root.replaceChildren(m);
      host.style.display = 'block';
      place();
    };
    const fill = function (u, p) {
      const pw = (field && field.type === 'password' && field) || (field && field.form && field.form.querySelector('input[type=password]')) || pwFieldsIn(null)[0];
      if (!pw) return;
      const uf = (field && field.type !== 'password' && field) || userFieldFor(pw);
      if (uf && u) setValue(uf, u);
      setValue(pw, p);
    };
    // Fixed entry points the browser calls; pages cannot replace them.
    try {
      Object.defineProperty(window, '__shunnoPwAccounts', { value: accounts, writable: false, configurable: false });
      Object.defineProperty(window, '__shunnoPwFill', { value: fill, writable: false, configurable: false });
    } catch (e) {}
  })();
})();
