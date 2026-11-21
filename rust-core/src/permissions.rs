//! Per-site permissions (camera, microphone, location, notifications, ...).
//!
//! Decisions are Shunno's own (WebView2 is told not to save them in its profile), stored per
//! origin in `permissions.json`. Private tabs never save an "Allow" but do honour saved "Block"s.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Block,
}

/// Kinds Shunno knows: (key, label, icon). The key is what is stored.
pub const KINDS: &[(&str, &str, &str)] = &[
    ("camera", "use your camera", "📷"),
    ("microphone", "use your microphone", "🎙️"),
    ("location", "know your location", "📍"),
    ("notifications", "show notifications", "🔔"),
    ("clipboard", "read your clipboard", "📋"),
    ("sensors", "use motion sensors", "🧭"),
    ("downloads", "download multiple files", "⬇️"),
    ("files", "edit files on your PC", "📁"),
    ("fonts", "use your installed fonts", "🔤"),
    ("midi", "control MIDI devices", "🎹"),
    ("windows", "manage windows on your screens", "🪟"),
    ("autoplay", "play media automatically", "▶️"),
    ("other", "use a device feature", "⚙️"),
];

pub fn label(kind: &str) -> (&'static str, &'static str) {
    KINDS.iter().find(|k| k.0 == kind).map(|k| (k.1, k.2)).unwrap_or(("use a device feature", "⚙️"))
}

/// What to do without asking: autoplay is allowed (Shunno already limits background media);
/// everything else asks the first time.
pub fn default_decision(kind: &str) -> Option<Decision> {
    match kind {
        "autoplay" => Some(Decision::Allow),
        _ => None,
    }
}

/// Decision to apply now for a request, or None to ask.
pub fn resolve(saved: Option<Decision>, kind: &str, private: bool) -> Option<Decision> {
    match saved {
        Some(Decision::Block) => Some(Decision::Block),
        // A private tab does not inherit "Allow" for sensitive kinds; it asks again.
        Some(Decision::Allow) if !private => Some(Decision::Allow),
        _ => default_decision(kind),
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub sites: BTreeMap<String, BTreeMap<String, Decision>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub origin: String,
    pub kind: String,
    pub label: &'static str,
    pub icon: &'static str,
    pub decision: Decision,
}

impl Store {
    pub fn get(&self, origin: &str, kind: &str) -> Option<Decision> {
        self.sites.get(origin).and_then(|m| m.get(kind)).copied()
    }
    pub fn set(&mut self, origin: &str, kind: &str, d: Decision) {
        if origin.is_empty() || !KINDS.iter().any(|k| k.0 == kind) {
            return;
        }
        self.sites.entry(origin.to_string()).or_default().insert(kind.to_string(), d);
    }
    pub fn reset(&mut self, origin: &str, kind: Option<&str>) {
        match kind {
            Some(k) => {
                if let Some(m) = self.sites.get_mut(origin) {
                    m.remove(k);
                    if m.is_empty() {
                        self.sites.remove(origin);
                    }
                }
            }
            None => {
                self.sites.remove(origin);
            }
        }
    }
    pub fn entries(&self) -> Vec<Entry> {
        let mut v = Vec::new();
        for (origin, m) in &self.sites {
            for (kind, d) in m {
                let (label, icon) = label(kind);
                v.push(Entry { origin: origin.clone(), kind: kind.clone(), label, icon, decision: *d });
            }
        }
        v
    }
}

fn path() -> std::path::PathBuf {
    crate::state::BrowserStorage::app_data_dir().join("permissions.json")
}

pub fn global() -> &'static Mutex<Store> {
    static S: std::sync::OnceLock<Mutex<Store>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let s = std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Mutex::new(s)
    })
}

pub fn save(store: &Store) {
    if let Ok(t) = serde_json::to_string_pretty(store) {
        let p = path();
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

/// A request waiting for the user (shown in the toolbar).
#[derive(Clone, Debug, Serialize)]
pub struct PromptView {
    pub id: u64,
    pub origin: String,
    pub host: String,
    pub kind: String,
    pub label: &'static str,
    pub icon: &'static str,
    pub private: bool,
}
