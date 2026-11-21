use crate::state::BrowserStorage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserScriptInfo {
    pub file_name: String,
    pub sha256: String,
    pub match_pattern: String,
    pub approved: bool,
}

pub fn user_scripts_dir() -> PathBuf {
    // Stored in %LOCALAPPDATA%\ShunnoBrowser\scripts\ (never inside Downloads!)
    let dir = BrowserStorage::app_data_dir().join("scripts");
    let _ = fs::create_dir_all(&dir);
    let readme = dir.join("README_UserScripts.txt");
    if !readme.exists() {
        let _ = fs::write(
            readme,
            "Place .js user-script files here and approve them in Shunno Browser Controls.\n\
             Optional header: // @match example.com\n\
             Unapproved scripts will never execute automatically.",
        );
    }
    dir
}

pub fn parse_match_header(code: &str) -> String {
    if let Some(match_line) = code
        .lines()
        .find(|l| l.trim_start().starts_with("// @match"))
    {
        match_line
            .trim_start()
            .trim_start_matches("// @match")
            .trim()
            .trim_matches('*')
            .trim_matches('/')
            .to_ascii_lowercase()
    } else {
        "*".to_string()
    }
}

pub fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let p = pattern.trim().trim_matches('*').trim_matches('/').to_ascii_lowercase();
    if p.is_empty() || p == "*" {
        return true;
    }
    let h = host.to_ascii_lowercase();
    h == p || h.ends_with(&format!(".{}", p)) || h.contains(&p)
}

pub fn compute_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

pub fn list_user_scripts(approved_hashes: &[String]) -> Vec<UserScriptInfo> {
    let dir = user_scripts_dir();
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("js") {
                if let (Some(fname), Ok(bytes)) = (
                    path.file_name().and_then(|n| n.to_str()),
                    fs::read(&path),
                ) {
                    let sha = compute_sha256(&bytes);
                    let code = String::from_utf8_lossy(&bytes);
                    let pat = parse_match_header(&code);
                    let approved = approved_hashes.iter().any(|h| h == &sha);
                    list.push(UserScriptInfo {
                        file_name: fname.to_string(),
                        sha256: sha,
                        match_pattern: pat,
                        approved,
                    });
                }
            }
        }
    }
    list.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    list
}

pub fn load_approved_user_scripts(host: &str, approved_hashes: &[String]) -> String {
    if approved_hashes.is_empty() {
        return String::new();
    }
    let dir = user_scripts_dir();
    let mut combined = String::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("js") {
                if let Ok(bytes) = fs::read(&path) {
                    let sha = compute_sha256(&bytes);
                    if !approved_hashes.iter().any(|h| h == &sha) {
                        continue;
                    }
                    let code = String::from_utf8_lossy(&bytes);
                    let pat = parse_match_header(&code);
                    if host_matches_pattern(host, &pat) {
                        combined.push_str("\ntry {\n");
                        combined.push_str(&code);
                        combined.push_str("\n} catch(e) { console.error('Shunno UserScript:', e); }\n");
                    }
                }
            }
        }
    }
    combined
}
