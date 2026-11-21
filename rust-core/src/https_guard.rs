//! HTTPS-Only mode: upgrade top-level http:// navigations to https://, and show a warning page
//! when the secure version fails or redirects back to http.
//!
//! Security notes
//! - The "continue over http" link carries a one-time random nonce. Without it, any page could link
//!   to `http://bank.example#allow` and strip HTTPS for a man-in-the-middle.
//! - Exceptions last for the session only.
//! - Loopback, LAN, link-local and single-label hosts are never upgraded (routers, NAS, intranet).

use crate::sanitizer::is_public_http_url;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use url::Url;

const ALLOW_MARK: &str = "shunno-allow-http-";
const LOOP_WINDOW: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Cancel this navigation and load the https URL instead.
    Upgrade(String),
    /// Cancel and show the warning page for this http URL.
    Interstitial(String),
}

#[derive(Default)]
pub struct HttpsGuard {
    enabled: AtomicBool,
    allowed_hosts: Mutex<HashSet<String>>,
    nonces: Mutex<HashMap<String, String>>,          // nonce -> host
    in_flight_hosts: Mutex<HashMap<String, Instant>>, // host -> when we upgraded it
    in_flight_tabs: Mutex<HashMap<usize, String>>,    // tab id -> https URL we sent it to
}

pub fn global() -> &'static HttpsGuard {
    static G: OnceLock<HttpsGuard> = OnceLock::new();
    G.get_or_init(HttpsGuard::default)
}

impl HttpsGuard {
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Decide what to do with a top-level navigation in `tab_id`.
    pub fn decide(&self, tab_id: usize, raw: &str) -> Decision {
        if !self.is_enabled() {
            return Decision::Allow;
        }
        let Ok(mut u) = Url::parse(raw) else {
            return Decision::Allow;
        };
        if u.scheme() != "http" || !is_public_http_url(raw) {
            return Decision::Allow;
        }
        // Non-standard ports (8080, 3000...) almost never serve TLS: leave them alone.
        if u.port().is_some_and(|p| p != 80) {
            return Decision::Allow;
        }
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();

        if let Some(nonce) = u.fragment().and_then(|f| f.strip_prefix(ALLOW_MARK)) {
            let granted = self
                .nonces
                .lock()
                .ok()
                .and_then(|mut n| n.remove(nonce))
                .is_some_and(|h| h == host);
            if granted {
                if let Ok(mut a) = self.allowed_hosts.lock() {
                    a.insert(host);
                }
                return Decision::Allow;
            }
        }
        if self.allowed_hosts.lock().map(|a| a.contains(&host)).unwrap_or(false) {
            return Decision::Allow;
        }

        // The secure page redirected straight back to http: the site does not really support https.
        let looped = self
            .in_flight_hosts
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&host))
            .is_some_and(|t| t.elapsed() < LOOP_WINDOW);
        if looped {
            if let Ok(mut t) = self.in_flight_tabs.lock() {
                t.remove(&tab_id);
            }
            return Decision::Interstitial(strip_allow_mark(raw));
        }

        if u.port() == Some(80) {
            let _ = u.set_port(None);
        }
        if u.set_scheme("https").is_err() {
            return Decision::Allow;
        }
        let target = u.to_string();
        if let Ok(mut m) = self.in_flight_hosts.lock() {
            m.insert(host, Instant::now());
        }
        if let Ok(mut t) = self.in_flight_tabs.lock() {
            t.insert(tab_id, target.clone());
        }
        Decision::Upgrade(target)
    }

    /// Called when a tab's navigation finishes. Returns the http URL to warn about when an upgraded
    /// navigation failed (connection refused, timeout, certificate error...).
    pub fn on_navigation_done(&self, tab_id: usize, success: bool, cancelled: bool) -> Option<String> {
        if cancelled {
            return None; // our own cancel of the http attempt, or the user stopped the load
        }
        let https_url = self.in_flight_tabs.lock().ok()?.remove(&tab_id)?;
        if let Some(host) = Url::parse(&https_url).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())) {
            if let Ok(mut m) = self.in_flight_hosts.lock() {
                m.remove(&host);
            }
        }
        if success {
            return None;
        }
        let mut u = Url::parse(&https_url).ok()?;
        u.set_scheme("http").ok()?;
        Some(u.to_string())
    }

    /// One-time permission for the user to continue to `http_url` over plain http.
    pub fn issue_allow_link(&self, http_url: &str) -> Option<String> {
        let mut u = Url::parse(http_url).ok()?;
        let host = u.host_str()?.to_ascii_lowercase();
        let nonce = format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>());
        self.nonces.lock().ok()?.insert(nonce.clone(), host);
        u.set_fragment(Some(&format!("{}{}", ALLOW_MARK, nonce)));
        Some(u.to_string())
    }
}

fn strip_allow_mark(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut u) if u.fragment().is_some_and(|f| f.starts_with(ALLOW_MARK)) => {
            u.set_fragment(None);
            u.to_string()
        }
        _ => raw.to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Warning page shown when a site has no working https version.
pub fn interstitial_html(http_url: &str, allow_link: &str) -> String {
    let host = Url::parse(http_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Not secure: {host}</title>
<style>
  body {{ font: 15px/1.55 -apple-system, "Segoe UI", system-ui, sans-serif; background:#F5F5F7; color:#1D1D1F;
         display:flex; align-items:center; justify-content:center; min-height:100vh; margin:0; }}
  .card {{ max-width:520px; background:#fff; border-radius:18px; padding:32px 34px; box-shadow:0 10px 34px rgba(0,0,0,.08); }}
  h1 {{ font-size:21px; margin:0 0 10px; }}
  p {{ margin:0 0 14px; color:#3a3a3c; }}
  code {{ background:#F2F2F7; padding:2px 6px; border-radius:6px; word-break:break-all; }}
  .row {{ display:flex; gap:10px; margin-top:22px; }}
  a {{ text-decoration:none; padding:9px 16px; border-radius:10px; font-weight:600; }}
  .primary {{ background:#0A84FF; color:#fff; }}
  .ghost {{ color:#C62828; background:#FDECEA; }}
  @media (prefers-color-scheme: dark) {{
    body {{ background:#111114; color:#F2F2F7; }} .card {{ background:#1C1C1E; }} p {{ color:#C7C7CC; }} code {{ background:#2C2C2E; }}
  }}
</style></head><body><div class="card">
<h1>This site has no secure connection</h1>
<p><code>{host}</code> did not load over HTTPS. Over plain HTTP, anyone on your network (public Wi-Fi, your ISP)
can read or change what you send and receive, including passwords.</p>
<p>Continue only if you trust this network and you will not enter anything private.</p>
<div class="row">
  <a class="primary" href="javascript:history.length > 1 ? history.back() : window.close()">Go back</a>
  <a class="ghost" href="{allow}">Continue to {host} (not secure)</a>
</div></div></body></html>"#,
        host = html_escape(&host),
        allow = html_escape(allow_link),
    )
}
