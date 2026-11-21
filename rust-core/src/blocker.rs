//! Ad and tracker blocking with Brave's `adblock` engine (MPL-2.0, used unmodified).
//!
//! - Filter lists (EasyList + EasyPrivacy) are downloaded at runtime, so the EXE ships no list data.
//! - The compiled engine is cached on disk and refreshed weekly in the background.
//! - Until lists are available, the small built-in matcher (`sanitizer::is_blocked_ad_url`) is used.

use crate::sanitizer::is_blocked_ad_url;
use crate::state::BrowserStorage;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::Engine;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

pub const FILTER_LISTS: [(&str, &str); 2] = [
    ("EasyList", "https://easylist.to/easylist/easylist.txt"),
    ("EasyPrivacy", "https://easylist.to/easylist/easyprivacy.txt"),
];
const REFRESH_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_COSMETIC_SELECTORS: usize = 4000;
// Serialized engines are only readable by the same crate version.
const CACHE_FILE: &str = "engine-adblock-0.13.dat";

#[derive(Default)]
pub struct Blocker {
    engine: RwLock<Option<Arc<Engine>>>,
    status: Mutex<String>,
}

pub fn global() -> &'static Blocker {
    static B: OnceLock<Blocker> = OnceLock::new();
    B.get_or_init(Blocker::default)
}

fn cache_path() -> PathBuf {
    let dir = BrowserStorage::shared_data_dir().join("filters");
    let _ = fs::create_dir_all(&dir);
    dir.join(CACHE_FILE)
}

/// Compiles filter list texts into an engine. Pure; used by refresh and by tests.
pub fn build_engine(list_texts: &[String]) -> (Engine, usize) {
    let mut set = FilterSet::new(false);
    let mut rules = 0usize;
    for text in list_texts {
        rules += text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('!')).count();
        set.add_filter_list(text.clone(), ParseOptions::default());
    }
    (Engine::new_with_filter_set(set), rules)
}

/// Builds the CSS that hides ad elements. Each selector becomes its own rule so one invalid
/// selector cannot disable the rest.
pub fn cosmetic_css_for(engine: &Engine, page_url: &str, classes: &[String], ids: &[String]) -> String {
    let res = engine.url_cosmetic_resources(page_url);
    let mut selectors: Vec<String> = res.hide_selectors.into_iter().collect();
    if !res.generichide && (!classes.is_empty() || !ids.is_empty()) {
        selectors.extend(engine.hidden_class_id_selectors(
            classes.iter().cloned(),
            ids.iter().cloned(),
            &res.exceptions,
        ));
    }
    let mut seen = HashSet::new();
    let mut css = String::new();
    for sel in selectors.into_iter().filter(|s| seen.insert(s.clone())).take(MAX_COSMETIC_SELECTORS) {
        // Filter lists are third-party input: refuse anything that could escape a rule body.
        if sel.contains(['{', '}', '<']) || sel.trim().is_empty() {
            continue;
        }
        css.push_str(&sel);
        css.push_str("{display:none !important}\n");
    }
    css
}

impl Blocker {
    pub fn set_status(&self, s: String) {
        if let Ok(mut st) = self.status.lock() {
            *st = s;
        }
    }

    pub fn status(&self) -> String {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn has_lists(&self) -> bool {
        self.engine.read().map(|e| e.is_some()).unwrap_or(false)
    }

    /// Loads the cached engine from disk (fast, no network). Call off the UI thread.
    pub fn load_cache(&self) {
        let path = cache_path();
        let Ok(bytes) = fs::read(&path) else {
            self.set_status("Built-in list (downloading filter lists)".to_string());
            return;
        };
        let mut engine = Engine::default();
        if engine.deserialize(&bytes).is_ok() {
            if let Ok(mut slot) = self.engine.write() {
                *slot = Some(Arc::new(engine));
            }
            self.set_status(format!("EasyList + EasyPrivacy ({})", age_label(&path)));
        } else {
            let _ = fs::remove_file(&path);
            self.set_status("Built-in list (cache outdated, refreshing)".to_string());
        }
    }

    pub fn is_stale(&self) -> bool {
        fs::metadata(cache_path())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|age| age > REFRESH_AFTER)
            .unwrap_or(true)
    }

    /// Downloads the lists, compiles them off the async workers, caches and swaps the engine in.
    pub async fn refresh(&self, client: &reqwest::Client) -> Result<usize, String> {
        let mut texts = Vec::with_capacity(FILTER_LISTS.len());
        for (name, url) in FILTER_LISTS {
            let resp = client
                .get(url)
                .timeout(Duration::from_secs(60))
                .send()
                .await
                .map_err(|e| format!("{}: {}", name, e))?;
            if !resp.status().is_success() {
                return Err(format!("{}: HTTP {}", name, resp.status()));
            }
            let text = resp.text().await.map_err(|e| format!("{}: {}", name, e))?;
            // Guard against captive portals and error pages masquerading as lists.
            if !text.trim_start().starts_with("[Adblock") && text.lines().count() < 1000 {
                return Err(format!("{}: response is not a filter list", name));
            }
            texts.push(text);
        }
        let (engine, rules) = tokio::task::spawn_blocking(move || build_engine(&texts))
            .await
            .map_err(|e| e.to_string())?;
        let bytes = engine.serialize();
        let path = cache_path();
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, &bytes).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
        // Swap in the compact deserialized form: ~13 MB instead of the much larger freshly built engine.
        let mut compact = Engine::default();
        let engine = if compact.deserialize(&bytes).is_ok() { compact } else { engine };
        if let Ok(mut slot) = self.engine.write() {
            *slot = Some(Arc::new(engine));
        }
        self.set_status(format!("EasyList + EasyPrivacy ({} rules, updated today)", rules));
        Ok(rules)
    }

    fn engine(&self) -> Option<Arc<Engine>> {
        self.engine.read().ok().and_then(|e| e.clone())
    }

    /// `req_type` uses adblock names: script, image, stylesheet, font, media, xmlhttprequest,
    /// websocket, ping, subdocument, other.
    pub fn should_block(&self, url: &str, source_url: &str, req_type: &str) -> bool {
        match self.engine() {
            Some(engine) => match Request::new(url, source_url, req_type, "GET") {
                Ok(req) => engine.check_network_request(&req).should_block(),
                Err(_) => false,
            },
            None => is_blocked_ad_url(url),
        }
    }

    /// CSS hiding ad containers on `page_url`; empty when lists are not loaded yet.
    pub fn cosmetic_css(&self, page_url: &str, classes: &[String], ids: &[String]) -> String {
        self.engine()
            .map(|e| cosmetic_css_for(&e, page_url, classes, ids))
            .unwrap_or_default()
    }
}

fn age_label(path: &PathBuf) -> String {
    let days = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    match days {
        0 => "updated today".to_string(),
        1 => "updated yesterday".to_string(),
        n => format!("updated {} days ago", n),
    }
}
