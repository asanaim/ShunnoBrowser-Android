//! Auto-update.
//!
//! CI publishes `latest.json` next to the EXE in the GitHub release:
//! `{"version":"2.12.1","url":"https://github.com/asanaim/ShunnoBrowser/releases/download/latest/ShunnoBrowser.exe",
//!   "sha256":"<hex>","signature":"<hex ed25519 over signed_message()>"}`
//!
//! Security model
//! - The manifest is signed with the owner's Ed25519 key (CI secret UPDATE_SIGNING_KEY). The public key
//!   is compiled in below. A compromised CDN, release asset or redirect cannot ship a fake update.
//! - Until `UPDATE_PUBLIC_KEY_HEX` is set, the updater only *notifies* (link to the release page);
//!   it never downloads or replaces the EXE.
//! - The EXE must match the signed SHA-256 and come from this repository's releases.
//! - Install = rename the running EXE to `.old.exe`, put the new one in its place; it runs on restart.

use crate::state::APP_VERSION;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Hex Ed25519 public key (32 bytes) of the release signing key. Empty = notify-only mode.
/// See docs/RELEASING.md. Rotating it requires one release signed with the old key.
pub const UPDATE_PUBLIC_KEY_HEX: &str = "d39e65379f6f4ed0edefc7b3bfef0a642e23cbaa6375bdc2d420b943c4027478";
pub const MANIFEST_URL: &str = "https://github.com/asanaim/ShunnoBrowser/releases/latest/download/latest.json";
pub const RELEASES_PAGE: &str = "https://github.com/asanaim/ShunnoBrowser/releases/latest";
const ALLOWED_URL_PREFIX: &str = "https://github.com/asanaim/ShunnoBrowser/releases/download/";
const MAX_EXE_BYTES: usize = 120 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    UpToDate,
    /// Newer version exists; install is not possible (notify-only mode or EXE folder not writable).
    Available { version: String, reason: String },
    /// New EXE is in place; restart to run it.
    Ready { version: String },
    Failed(String),
}

#[derive(Debug, serde::Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub signature: String,
}

pub fn signed_message(m: &Manifest) -> String {
    format!("shunno-update-v1\n{}\n{}\n{}", m.version.trim(), m.sha256.trim().to_ascii_lowercase(), m.url.trim())
}

/// "v2.12.0" / "2.12.0" -> (2, 12, 0). Missing parts count as 0.
pub fn parse_version(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

/// Verifies the manifest signature against `public_key_hex`. Returns false for any malformed input.
pub fn verify_manifest(m: &Manifest, public_key_hex: &str) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};
    let (Ok(pk), Ok(sig)) = (hex::decode(public_key_hex.trim()), hex::decode(m.signature.trim())) else {
        return false;
    };
    let (Ok(pk), Ok(sig)) = (<[u8; 32]>::try_from(pk.as_slice()), <[u8; 64]>::try_from(sig.as_slice())) else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else {
        return false;
    };
    vk.verify_strict(signed_message(m).as_bytes(), &Signature::from_bytes(&sig)).is_ok()
}

pub struct Updater {
    state: Mutex<UpdateState>,
}

pub fn global() -> &'static Updater {
    static U: OnceLock<Updater> = OnceLock::new();
    U.get_or_init(|| Updater { state: Mutex::new(UpdateState::Idle) })
}

fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| fs::canonicalize(&p).ok().or(Some(p)))
}

fn old_exe_path(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

impl Updater {
    pub fn state(&self) -> UpdateState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set(&self, s: UpdateState) {
        if let Ok(mut g) = self.state.lock() {
            *g = s;
        }
    }

    /// Removes the previous EXE left by an update. Call at startup.
    pub fn cleanup_previous(&self) {
        if let Some(exe) = current_exe() {
            let _ = fs::remove_file(old_exe_path(&exe));
        }
    }

    pub async fn check(&self, client: &reqwest::Client) {
        if matches!(self.state(), UpdateState::Ready { .. }) {
            return;
        }
        match self.check_inner(client).await {
            Ok(s) | Err(s) => self.set(s),
        }
    }

    async fn check_inner(&self, client: &reqwest::Client) -> Result<UpdateState, UpdateState> {
        let fail = |e: String| UpdateState::Failed(e);
        let resp = client
            .get(MANIFEST_URL)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| fail(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(UpdateState::UpToDate); // no manifest published yet
        }
        if !resp.status().is_success() {
            return Err(fail(format!("HTTP {}", resp.status())));
        }
        let text = resp.text().await.map_err(|e| fail(e.to_string()))?;
        let m: Manifest = serde_json::from_str(&text).map_err(|e| fail(format!("bad manifest: {}", e)))?;
        if !is_newer(&m.version, APP_VERSION) {
            return Ok(UpdateState::UpToDate);
        }
        let version = m.version.trim().trim_start_matches('v').to_string();
        if !m.url.starts_with(ALLOWED_URL_PREFIX) {
            return Err(fail("manifest points outside this repository".to_string()));
        }
        if UPDATE_PUBLIC_KEY_HEX.is_empty() {
            return Ok(UpdateState::Available { version, reason: "signing key not configured".to_string() });
        }
        if m.signature.trim().is_empty() {
            // CI has no UPDATE_SIGNING_KEY secret yet: never install, just point to the release page.
            return Ok(UpdateState::Available { version, reason: "release is not signed".to_string() });
        }
        if !verify_manifest(&m, UPDATE_PUBLIC_KEY_HEX) {
            return Err(fail("update signature invalid, ignored".to_string()));
        }
        let Some(exe) = current_exe() else {
            return Ok(UpdateState::Available { version, reason: "cannot locate the running EXE".to_string() });
        };

        let bytes = client
            .get(&m.url)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| fail(e.to_string()))?
            .error_for_status()
            .map_err(|e| fail(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| fail(e.to_string()))?;
        if bytes.len() > MAX_EXE_BYTES || bytes.len() < 1024 || &bytes[..2] != b"MZ" {
            return Err(fail("downloaded file is not a Windows program".to_string()));
        }
        if hex::encode(Sha256::digest(&bytes)) != m.sha256.trim().to_ascii_lowercase() {
            return Err(fail("checksum mismatch, update discarded".to_string()));
        }

        let exe2 = exe.clone();
        let installed = tokio::task::spawn_blocking(move || install(&exe2, &bytes))
            .await
            .map_err(|e| fail(e.to_string()))?;
        match installed {
            Ok(()) => Ok(UpdateState::Ready { version }),
            Err(e) => Ok(UpdateState::Available { version, reason: e }),
        }
    }
}

/// Swaps the running EXE: Windows allows renaming a running executable, not overwriting it.
pub fn install(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let staged = exe.with_extension("new.exe");
    let old = old_exe_path(exe);
    fs::write(&staged, bytes).map_err(|e| format!("folder not writable ({}); download manually", e))?;
    let _ = fs::remove_file(&old);
    if let Err(e) = fs::rename(exe, &old) {
        let _ = fs::remove_file(&staged);
        return Err(format!("cannot replace the EXE ({}); download manually", e));
    }
    if let Err(e) = fs::rename(&staged, exe) {
        let _ = fs::rename(&old, exe); // roll back
        return Err(format!("install failed ({}); kept the current version", e));
    }
    Ok(())
}
