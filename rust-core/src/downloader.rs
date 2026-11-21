use crate::state::{hidden_command, BrowserStorage};
use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, RANGE, REFERER, USER_AGENT};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";

const FFMPEG_ZIP_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
const FFMPEG_SHA_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256";
const YTDLP_EXE_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const YTDLP_SUMS_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";

/// Task kinds. Empty string means a plain HTTP(S) file download.
pub const KIND_HTTP: &str = "";
pub const KIND_HLS: &str = "HLS";
pub const KIND_DASH: &str = "DASH";
pub const KIND_YTDLP: &str = "YTDLP";
/// yt-dlp, audio only, original codec (Opus or AAC; YouTube has no lossless audio).
pub const KIND_YTDLP_AUDIO: &str = "YTDLP_AUDIO";
/// yt-dlp, audio only, re-encoded to FLAC.
pub const KIND_YTDLP_FLAC: &str = "YTDLP_FLAC";
pub const KIND_NATIVE: &str = "NATIVE";

/// URL prefix that forces yt-dlp resolution of a web page (used by the floating bar for blob:/MSE players).
pub const YTDLP_PREFIX: &str = "ytdlp:";
pub const YTDLP_AUDIO_PREFIX: &str = "ytdlp-audio:";
pub const YTDLP_FLAC_PREFIX: &str = "ytdlp-flac:";

fn is_ytdlp_kind(kind: &str) -> bool {
    kind == KIND_YTDLP || kind == KIND_YTDLP_AUDIO || kind == KIND_YTDLP_FLAC
}

/// Split a routing prefix off a queued URL: (kind forced by the prefix, bare URL).
pub fn split_route_prefix(raw: &str) -> (Option<&'static str>, &str) {
    for (prefix, kind) in [(YTDLP_AUDIO_PREFIX, KIND_YTDLP_AUDIO), (YTDLP_FLAC_PREFIX, KIND_YTDLP_FLAC), (YTDLP_PREFIX, KIND_YTDLP)] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return (Some(kind), rest);
        }
    }
    (None, raw)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: u64,
    pub url: String,
    pub file_name: String,
    pub total_bytes: i64,
    pub downloaded_bytes: u64,
    pub status: String, // "DOWNLOADING", "QUEUED", "PAUSED", "COMPLETED", "FAILED", "MUXING"
    pub speed_bps: u64,
    pub threads: usize,
    pub category: String,
    pub supports_ranges: bool,
    pub referer: Option<String>,
    pub cookie: Option<String>,
    pub sha256: Option<String>,
    pub saved_path: Option<String>,
    pub error_message: Option<String>,
    pub segment_map: String,
    #[serde(default)]
    pub scheduled_start_epoch: Option<u64>,
    /// "", "HLS", "DASH", "YTDLP" or "NATIVE"
    #[serde(default)]
    pub kind: String,
    /// Second input for DASH video+audio muxing.
    #[serde(default)]
    pub audio_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamProbeResult {
    pub url: String,
    pub size_bytes: Option<u64>,
    pub size_label: String,
    pub resolved_name: Option<String>,
}

pub enum EnqueueOutcome {
    Started,
    DuplicateDetected(DownloadTask),
}

pub fn sanitize_download_filename(raw: &str) -> String {
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("download.bin")
        .split('?')
        .next()
        .unwrap_or("download.bin")
        .trim();

    let mut cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'..='\x1f' => '_',
            _ => c,
        })
        .collect();

    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", "_");
    }
    cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        cleaned = "download.bin".to_string();
    }
    // Keep names well under MAX_PATH once joined with the downloads folder.
    if cleaned.chars().count() > 180 {
        let ext = Path::new(&cleaned)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();
        let stem: String = cleaned.chars().take(180usize.saturating_sub(ext.chars().count())).collect();
        cleaned = format!("{}{}", stem.trim_end(), ext);
    }

    // Prefix Windows reserved device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    let is_reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    );
    if is_reserved {
        cleaned = format!("_{}", cleaned);
    }

    // Prevent writing into any scripts subdirectory
    if cleaned.to_ascii_lowercase().ends_with(".js") {
        cleaned.push_str(".download");
    }

    cleaned
}

pub fn format_human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

pub fn detect_download_category(file_name: &str) -> String {
    let ext = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "ts" | "m3u8" | "m4s" | "mpd" => "Videos".to_string(),
        "flac" | "alac" | "wav" | "aiff" | "aif" | "ape" | "wv" => "Lossless Audio".to_string(),
        "mp3" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wma" => "Audio".to_string(),
        "zip" | "rar" | "7z" | "tar" | "gz" | "iso" => "Archives".to_string(),
        "exe" | "msi" => "Programs".to_string(),
        _ => "Documents".to_string(),
    }
}

fn infer_extension_from_mime(mime: &str) -> Option<&'static str> {
    let m = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match m.as_str() {
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/x-matroska" => Some("mkv"),
        _ if m.ends_with("mpegurl") => Some("m3u8"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/ogg" => Some("ogg"),
        "application/zip" => Some("zip"),
        "application/x-7z-compressed" => Some("7z"),
        "application/x-rar-compressed" | "application/vnd.rar" => Some("rar"),
        "application/pdf" => Some("pdf"),
        "application/x-msdownload" => Some("exe"),
        _ => None,
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn parse_content_disposition_filename(header_val: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    for part in header_val.split(';') {
        let trimmed = part.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("filename*=") {
            if let Some((_, val)) = trimmed.split_once('=') {
                // RFC 5987: charset'lang'percent-encoded
                let v = val.trim().trim_matches('"');
                let encoded = v.splitn(3, '\'').nth(2).unwrap_or(v);
                let cleaned = percent_decode(encoded);
                if !cleaned.trim().is_empty() {
                    // filename* takes precedence over filename
                    return Some(sanitize_download_filename(&cleaned));
                }
            }
        } else if lower.starts_with("filename=") && plain.is_none() {
            if let Some((_, val)) = trimmed.split_once('=') {
                let cleaned = val.trim().trim_matches('"').trim_matches('\'').to_string();
                if !cleaned.is_empty() {
                    plain = Some(sanitize_download_filename(&cleaned));
                }
            }
        }
    }
    plain
}

/// Parses `Content-Range: bytes START-END/TOTAL` (TOTAL may be `*`).
pub fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = rest.split_once('/')?;
    let (s, e) = range.trim().split_once('-')?;
    let start = s.trim().parse::<u64>().ok()?;
    let end = e.trim().parse::<u64>().ok()?;
    let total = total.trim().parse::<u64>().ok();
    if end < start {
        return None;
    }
    Some((start, end, total))
}

/// URLs that need the yt-dlp extractor instead of a plain HTTP transfer.
pub fn is_ytdlp_url(url_str: &str) -> bool {
    let Ok(u) = url::Url::parse(url_str) else {
        return false;
    };
    let host = u.host_str().unwrap_or("").trim_start_matches("www.").to_ascii_lowercase();
    let path = u.path().to_ascii_lowercase();
    let yt = host == "youtube.com" || host == "m.youtube.com" || host == "music.youtube.com";
    if yt && (path == "/watch" || path.starts_with("/shorts/") || path.starts_with("/live/") || path.starts_with("/embed/")) {
        return true;
    }
    if host == "youtu.be" && path.len() > 1 {
        return true;
    }
    // DASH manifests need segment assembly plus merging; yt-dlp handles them generically.
    path.ends_with(".mpd")
}

pub fn is_hls_url(url_str: &str) -> bool {
    let path = url_str.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    path.ends_with(".m3u8") || path.ends_with(".m3u")
}

#[derive(Debug, Default, PartialEq)]
pub struct HlsPlaylist {
    /// (bandwidth, url) for master playlists
    pub variants: Vec<(u64, url::Url)>,
    pub init_segment: Option<url::Url>,
    pub segments: Vec<url::Url>,
    pub encrypted: bool,
    pub byte_ranges: bool,
}

pub fn parse_hls_playlist(text: &str, base: &url::Url) -> HlsPlaylist {
    let mut pl = HlsPlaylist::default();
    let mut pending_bw: Option<u64> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("#EXT-X-STREAM-INF") {
            let bw = attr_value(line, "BANDWIDTH").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            pending_bw = Some(bw);
        } else if upper.starts_with("#EXT-X-KEY") {
            let method = attr_value(line, "METHOD").unwrap_or_default().to_ascii_uppercase();
            if method != "NONE" && !method.is_empty() {
                pl.encrypted = true;
            }
        } else if upper.starts_with("#EXT-X-MAP") {
            if let Some(uri) = attr_value(line, "URI") {
                pl.init_segment = base.join(&uri).ok();
            }
            if attr_value(line, "BYTERANGE").is_some() {
                pl.byte_ranges = true;
            }
        } else if upper.starts_with("#EXT-X-BYTERANGE") {
            pl.byte_ranges = true;
        } else if line.starts_with('#') {
            continue;
        } else if let Ok(u) = base.join(line) {
            if let Some(bw) = pending_bw.take() {
                pl.variants.push((bw, u));
            } else {
                pl.segments.push(u);
            }
        }
    }
    pl
}

fn attr_value(line: &str, key: &str) -> Option<String> {
    let attrs = line.split_once(':')?.1;
    let mut rest = attrs;
    while !rest.is_empty() {
        let (k, after_k) = rest.split_once('=')?;
        let (val, next) = if let Some(stripped) = after_k.strip_prefix('"') {
            let end = stripped.find('"')?;
            let v = &stripped[..end];
            let after = &stripped[end + 1..];
            (v, after.strip_prefix(',').unwrap_or(after))
        } else {
            match after_k.split_once(',') {
                Some((v, n)) => (v, n),
                None => (after_k, ""),
            }
        };
        if k.trim().eq_ignore_ascii_case(key) {
            return Some(val.trim().to_string());
        }
        rest = next;
    }
    None
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn new_task_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
        .wrapping_mul(1000)
        + rand::random::<u16>() as u64 % 1000
}

fn update_task<F: FnOnce(&mut DownloadTask)>(tasks: &Arc<Mutex<Vec<DownloadTask>>>, id: u64, f: F) {
    if let Ok(mut list) = tasks.lock() {
        if let Some(t) = list.iter_mut().find(|i| i.id == id) {
            f(t);
        }
    }
}

fn with_headers(mut rb: RequestBuilder, ua: &str, referer: &Option<String>, cookie: &Option<String>) -> RequestBuilder {
    rb = rb.header(USER_AGENT, ua).header("Accept", "*/*");
    if let Some(r) = referer {
        if r.starts_with("http") {
            rb = rb.header(REFERER, r);
        }
    }
    if let Some(c) = cookie {
        if !c.is_empty() {
            rb = rb.header(COOKIE, c);
        }
    }
    rb
}

async fn throttle(limit: &AtomicU64, bytes: usize, share: u64) {
    let max_bps = limit.load(Ordering::Relaxed);
    if max_bps == 0 {
        return;
    }
    let per = (max_bps / share.max(1)).max(4096);
    let ms = (bytes as u64 * 1000) / per;
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return None,
        }
    }
    Some(hex::encode(hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Shared state for one running job.
#[derive(Clone)]
struct JobCtx {
    client: Client,
    ua: String,
    running: Arc<AtomicBool>,
    speed_limit: Arc<AtomicU64>,
    tasks: Arc<Mutex<Vec<DownloadTask>>>,
    ffmpeg_status: Arc<Mutex<String>>,
    ytdlp_status: Arc<Mutex<String>>,
}

impl JobCtx {
    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

pub struct NativeDownloadEngine {
    client: Client,
    pub tasks: Arc<Mutex<Vec<DownloadTask>>>,
    active_flags: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    pub speed_limit_bps: Arc<AtomicU64>,
    pub auto_clean_completed: Arc<AtomicBool>,
    pub max_concurrent: Arc<AtomicUsize>,
    pub ffmpeg_install_status: Arc<Mutex<String>>,
    pub ytdlp_install_status: Arc<Mutex<String>>,
    pub user_agent: Arc<Mutex<String>>,
}

impl NativeDownloadEngine {
    pub fn new(initial_speed_limit_bps: u64, initial_auto_clean: bool, initial_max_concurrent: usize) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(25))
            .read_timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(20))
            .pool_max_idle_per_host(64)
            .build()
            .unwrap_or_else(|_| Client::new());

        let mut loaded = Self::load_queue();
        // "Auto-clean finished": finished items stay visible for the session and are dropped at the next start.
        if initial_auto_clean {
            loaded.retain(|t| t.status != "COMPLETED");
        }
        Self {
            client,
            tasks: Arc::new(Mutex::new(loaded)),
            active_flags: Arc::new(Mutex::new(HashMap::new())),
            speed_limit_bps: Arc::new(AtomicU64::new(initial_speed_limit_bps)),
            auto_clean_completed: Arc::new(AtomicBool::new(initial_auto_clean)),
            max_concurrent: Arc::new(AtomicUsize::new(initial_max_concurrent.clamp(1, 8))),
            ffmpeg_install_status: Arc::new(Mutex::new(String::new())),
            ytdlp_install_status: Arc::new(Mutex::new(String::new())),
            user_agent: Arc::new(Mutex::new(DEFAULT_UA.to_string())),
        }
    }

    pub fn install_ffmpeg_bundle(&self, rt: &tokio::runtime::Handle) {
        let client = self.client.clone();
        let status = self.ffmpeg_install_status.clone();
        rt.spawn(async move {
            let _ = ensure_ffmpeg(&client, &status).await;
        });
    }

    pub fn install_ytdlp_bundle(&self, rt: &tokio::runtime::Handle) {
        let client = self.client.clone();
        let status = self.ytdlp_install_status.clone();
        rt.spawn(async move {
            let _ = ensure_ytdlp(&client, &status).await;
        });
    }

    /// The tab's real navigator.userAgent, so CDNs that bind tokens to the UA accept engine requests.
    pub fn set_user_agent(&self, ua: &str) {
        let trimmed = ua.trim();
        if !trimmed.is_empty() && trimmed.is_ascii() && !trimmed.contains(['\r', '\n']) {
            if let Ok(mut guard) = self.user_agent.lock() {
                *guard = trimmed.to_string();
            }
        }
    }

    pub fn current_user_agent(&self) -> String {
        self.user_agent.lock().map(|g| g.clone()).unwrap_or_else(|_| DEFAULT_UA.to_string())
    }

    fn job_ctx(&self, running: Arc<AtomicBool>) -> JobCtx {
        JobCtx {
            client: self.client.clone(),
            ua: self.current_user_agent(),
            running,
            speed_limit: self.speed_limit_bps.clone(),
            tasks: self.tasks.clone(),
            ffmpeg_status: self.ffmpeg_install_status.clone(),
            ytdlp_status: self.ytdlp_install_status.clone(),
        }
    }

    pub fn task_kind(&self, id: u64) -> Option<String> {
        self.tasks.lock().ok()?.iter().find(|t| t.id == id).map(|t| t.kind.clone())
    }

    // ---------------- WebView2 native (browser-owned) downloads ----------------

    pub fn register_native_download(&self, id: u64, url: String, file_name: String, saved_path: String, total_bytes: i64) {
        let safe_name = sanitize_download_filename(&file_name);
        let category = detect_download_category(&safe_name);
        let task = DownloadTask {
            id,
            url,
            file_name: safe_name,
            total_bytes,
            downloaded_bytes: 0,
            status: "DOWNLOADING".to_string(),
            speed_bps: 0,
            threads: 1,
            category,
            supports_ranges: false,
            referer: None,
            cookie: None,
            sha256: None,
            saved_path: Some(saved_path),
            error_message: None,
            segment_map: "[Browser Native]".to_string(),
            scheduled_start_epoch: None,
            kind: KIND_NATIVE.to_string(),
            audio_url: None,
        };
        if let Ok(mut list) = self.tasks.lock() {
            list.insert(0, task);
        }
        self.save_queue();
    }

    pub fn update_native_download_progress(&self, id: u64, received: u64, total: i64, speed_bps: u64) {
        update_task(&self.tasks, id, |t| {
            t.downloaded_bytes = received;
            if total > 0 {
                t.total_bytes = total;
            }
            t.speed_bps = speed_bps;
            if t.status == "PAUSED" || t.status == "FAILED" {
                t.status = "DOWNLOADING".to_string();
                t.error_message = None;
            }
        });
    }

    pub fn set_native_status(&self, id: u64, status: &str, err: Option<String>) {
        update_task(&self.tasks, id, |t| {
            t.status = status.to_string();
            t.speed_bps = 0;
            t.error_message = err;
        });
        self.save_queue();
    }

    pub fn finish_native_download(&self, id: u64, ok: bool, final_path: Option<String>, err_msg: Option<String>) {
        let sha_hex = if ok { final_path.as_deref().and_then(|p| sha256_file(Path::new(p))) } else { None };
        let final_size = final_path.as_ref().and_then(|p| fs::metadata(p).ok().map(|m| m.len()));
        if let Ok(mut list) = self.tasks.lock() {
            if let Some(t) = list.iter_mut().find(|i| i.id == id) {
                t.speed_bps = 0;
                if ok {
                    t.status = "COMPLETED".to_string();
                    if let Some(sz) = final_size {
                        t.total_bytes = sz as i64;
                        t.downloaded_bytes = sz;
                    }
                    t.sha256 = sha_hex;
                    if let Some(p) = final_path {
                        t.saved_path = Some(p);
                    }
                } else {
                    t.status = "FAILED".to_string();
                    t.error_message = err_msg.or_else(|| Some("Interrupted".to_string()));
                }
            }
        }
        self.save_queue();
    }

    // ---------------- Queue persistence & housekeeping ----------------

    pub fn export_history_csv(&self) -> Result<PathBuf, String> {
        let out_path = BrowserStorage::downloads_root_dir().join("download_history.csv");
        let list = self.tasks.lock().map_err(|e| e.to_string())?;
        let mut csv = String::from("ID,FileName,Category,Status,TotalBytes,SHA256,SavedPath,URL\n");
        // Quote every field and neutralise spreadsheet formula injection.
        let esc = |s: &str| {
            let guarded = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{}", s) } else { s.to_string() };
            format!("\"{}\"", guarded.replace('"', "\"\""))
        };
        for t in list.iter() {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                t.id,
                esc(&t.file_name),
                esc(&t.category),
                esc(&t.status),
                t.total_bytes,
                esc(t.sha256.as_deref().unwrap_or("")),
                esc(t.saved_path.as_deref().unwrap_or("")),
                esc(&t.url),
            ));
        }
        fs::write(&out_path, csv).map_err(|e| e.to_string())?;
        Ok(out_path)
    }

    /// Removes finished entries (completed, failed) from the list. Files on disk are never touched.
    pub fn clear_finished(&self) -> usize {
        let removed = if let Ok(mut list) = self.tasks.lock() {
            let before = list.len();
            list.retain(|t| t.status != "COMPLETED" && t.status != "FAILED");
            before.saturating_sub(list.len())
        } else {
            0
        };
        if removed > 0 {
            self.save_queue();
        }
        removed
    }

    pub fn pump_scheduler(&self, rt: &tokio::runtime::Handle) {
        let now = now_secs();
        let max_conc = self.max_concurrent.load(Ordering::Relaxed).max(1);

        let to_start: Vec<DownloadTask> = if let Ok(mut list) = self.tasks.lock() {
            let active_count = list
                .iter()
                .filter(|t| (t.status == "DOWNLOADING" || t.status == "MUXING") && t.kind != KIND_NATIVE)
                .count();
            if active_count >= max_conc {
                Vec::new()
            } else {
                let slots = max_conc - active_count;
                let mut picked = Vec::new();
                for t in list.iter_mut().rev() {
                    if picked.len() >= slots {
                        break;
                    }
                    if t.status == "QUEUED" {
                        let due = t.scheduled_start_epoch.map(|ep| now >= ep).unwrap_or(true);
                        if due {
                            t.status = "DOWNLOADING".to_string();
                            t.scheduled_start_epoch = None;
                            if t.segment_map == "[Scheduled]" {
                                t.segment_map.clear();
                            }
                            picked.push(t.clone());
                        }
                    }
                }
                picked
            }
        } else {
            Vec::new()
        };

        if !to_start.is_empty() {
            self.save_queue();
            for task in to_start {
                self.spawn_worker(rt, task);
            }
        }
    }

    /// Can Shunno's engine fetch this browser download itself? True only if a ranged GET with the
    /// page's cookies returns 206 for the same file (same total size when the browser knows it) and
    /// not an HTML page. Anything else (POST forms, one-time links, sign-in walls) stays with WebView2.
    pub async fn probe_takeover(&self, url: &str, referer: Option<String>, cookie: Option<String>, expected_total: Option<u64>) -> Option<u64> {
        let ua = self.current_user_agent();
        // One retry: a pooled connection the server already closed fails the first send.
        let mut resp = None;
        for _ in 0..2 {
            let rb = with_headers(self.client.get(url), &ua, &referer, &cookie).header(RANGE, "bytes=0-0");
            if let Ok(r) = rb.timeout(Duration::from_secs(12)).send().await {
                resp = Some(r);
                break;
            }
        }
        let resp = resp?;
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            return None;
        }
        if resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|t| t.to_ascii_lowercase().starts_with("text/html"))
            .unwrap_or(false)
        {
            return None;
        }
        let total = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range)
            .and_then(|(start, _, total)| if start == 0 { total } else { None })?;
        match expected_total {
            Some(e) if e != total => None,
            _ if total == 0 => None,
            _ => Some(total),
        }
    }

    pub async fn probe_stream_metadata(&self, url: String, referer: Option<String>, cookie: Option<String>) -> StreamProbeResult {
        let ua = self.current_user_agent();
        let mut size_bytes: Option<u64> = None;
        let mut resolved_name: Option<String> = None;

        let head = with_headers(self.client.head(&url), &ua, &referer, &cookie);
        if let Ok(resp) = head.timeout(Duration::from_secs(20)).send().await {
            if resp.status().is_success() {
                size_bytes = resp
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|&n| n > 0);
                if let Some(cd) = resp.headers().get("content-disposition").and_then(|v| v.to_str().ok()) {
                    resolved_name = parse_content_disposition_filename(cd);
                }
            }
        }

        if size_bytes.is_none() {
            let rb = with_headers(self.client.get(&url), &ua, &referer, &cookie).header(RANGE, "bytes=0-0");
            if let Ok(resp) = rb.timeout(Duration::from_secs(20)).send().await {
                if resp.status() == StatusCode::PARTIAL_CONTENT {
                    size_bytes = resp
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_content_range)
                        .and_then(|(_, _, total)| total)
                        .filter(|&n| n > 0);
                }
                if resolved_name.is_none() {
                    if let Some(cd) = resp.headers().get("content-disposition").and_then(|v| v.to_str().ok()) {
                        resolved_name = parse_content_disposition_filename(cd);
                    }
                }
            }
        }

        let size_label = match size_bytes {
            Some(b) => format_human_size(b),
            None => "Stream".to_string(),
        };

        StreamProbeResult { url, size_bytes, size_label, resolved_name }
    }

    pub fn parts_dir() -> PathBuf {
        let dir = BrowserStorage::app_data_dir().join("parts");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn queue_file_path() -> PathBuf {
        BrowserStorage::app_data_dir().join("downloads_queue.json")
    }

    fn load_queue() -> Vec<DownloadTask> {
        let path = Self::queue_file_path();
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(mut items) = serde_json::from_str::<Vec<DownloadTask>>(&raw) {
                for it in &mut items {
                    if it.status == "DOWNLOADING" || it.status == "MUXING" {
                        // Browser-native transfers cannot survive a restart; engine jobs resume from .offset files.
                        it.status = "PAUSED".to_string();
                        it.speed_bps = 0;
                    }
                    if it.kind.is_empty() && is_hls_url(&it.url) {
                        it.kind = KIND_HLS.to_string();
                    }
                }
                return items;
            }
        }
        Vec::new()
    }

    pub fn save_queue(&self) {
        let json = match self.tasks.lock() {
            Ok(list) => serde_json::to_string_pretty(&*list).ok(),
            Err(_) => None,
        };
        if let Some(json) = json {
            write_atomic(&Self::queue_file_path(), json.as_bytes());
        }
    }

    pub fn pause(&self, id: u64) {
        if let Ok(mut map) = self.active_flags.lock() {
            if let Some(flag) = map.remove(&id) {
                flag.store(false, Ordering::SeqCst);
            }
        }
        update_task(&self.tasks, id, |t| {
            if t.status == "DOWNLOADING" || t.status == "MUXING" || t.status == "QUEUED" {
                t.status = "PAUSED".to_string();
                t.speed_bps = 0;
            }
        });
        self.save_queue();
    }

    pub fn delete(&self, id: u64) {
        self.pause(id);
        let part_prefix = format!("{}_", id);
        if let Ok(entries) = fs::read_dir(Self::parts_dir()) {
            for e in entries.flatten() {
                if let Some(name) = e.file_name().to_str() {
                    if name.starts_with(&part_prefix) {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
        }
        if let Ok(mut list) = self.tasks.lock() {
            list.retain(|i| i.id != id);
        }
        self.save_queue();
    }

    pub fn resume_or_restart(&self, rt: &tokio::runtime::Handle, id: u64, new_url: Option<String>) {
        let task_opt = if let Ok(mut list) = self.tasks.lock() {
            if let Some(t) = list.iter_mut().find(|i| i.id == id) {
                if t.status == "DOWNLOADING" || t.status == "MUXING" {
                    None
                } else {
                    if let Some(u) = new_url {
                        let u = u.trim().to_string();
                        if u != t.url {
                            t.url = u;
                            // A new mirror invalidates partial data: segment boundaries depend on its size.
                            let prefix = format!("{}_", t.id);
                            if let Ok(entries) = fs::read_dir(Self::parts_dir()) {
                                for e in entries.flatten() {
                                    if e.file_name().to_str().map(|n| n.starts_with(&prefix)).unwrap_or(false) {
                                        let _ = fs::remove_file(e.path());
                                    }
                                }
                            }
                        }
                    }
                    if t.kind == KIND_NATIVE {
                        // The WebView2 operation is gone (cancelled or app restarted): re-fetch with the engine.
                        t.kind = KIND_HTTP.to_string();
                        t.segment_map.clear();
                        if let Some(p) = t.saved_path.take() {
                            let _ = fs::remove_file(p);
                        }
                    }
                    if t.kind == KIND_HTTP && is_hls_url(&t.url) {
                        t.kind = KIND_HLS.to_string();
                    }
                    t.status = "DOWNLOADING".to_string();
                    t.error_message = None;
                    Some(t.clone())
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(task) = task_opt {
            self.save_queue();
            self.spawn_worker(rt, task);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &self,
        rt: &tokio::runtime::Handle,
        url: String,
        threads: usize,
        referer: Option<String>,
        cookie: Option<String>,
        dup_mode: &str, // "CHECK", "RENAME", "OVERWRITE"
        delay_secs: Option<u64>,
    ) -> EnqueueOutcome {
        self.enqueue_named(rt, url, threads, referer, cookie, dup_mode, delay_secs, None)
    }

    /// As `enqueue`, but plain HTTP downloads are saved under `name_hint` (the name the page or
    /// server chose, e.g. from a browser download) instead of one derived from the URL.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_named(
        &self,
        rt: &tokio::runtime::Handle,
        url: String,
        threads: usize,
        referer: Option<String>,
        cookie: Option<String>,
        dup_mode: &str,
        delay_secs: Option<u64>,
        name_hint: Option<String>,
    ) -> EnqueueOutcome {
        let raw = url.trim();
        let (forced_kind, trimmed_u) = split_route_prefix(raw);
        let trimmed_u = trimmed_u.trim();
        let normalized_url = if trimmed_u.starts_with("http://") || trimmed_u.starts_with("https://") {
            trimmed_u.to_string()
        } else {
            format!("https://{}", trimmed_u)
        };

        let kind = if let Some(k) = forced_kind {
            k
        } else if is_ytdlp_url(&normalized_url) {
            KIND_YTDLP
        } else if is_hls_url(&normalized_url) {
            KIND_HLS
        } else {
            KIND_HTTP
        };

        let mut raw_name = if kind == KIND_YTDLP {
            "Video (resolving title).mp4".to_string()
        } else if kind == KIND_YTDLP_AUDIO {
            "Audio (resolving title).m4a".to_string()
        } else if kind == KIND_YTDLP_FLAC {
            "Audio (resolving title).flac".to_string()
        } else if let Some(n) = name_hint.as_deref().map(sanitize_download_filename).filter(|n| kind == KIND_HTTP && !n.is_empty()) {
            n
        } else {
            normalized_url
                .split(['?', '#'])
                .next()
                .and_then(|p| p.rsplit('/').next())
                .map(percent_decode)
                .filter(|s| !s.is_empty() && s.contains('.'))
                .map(|s| sanitize_download_filename(&s))
                .unwrap_or_else(|| format!("shunno_download_{}.bin", now_secs() % 100_000))
        };
        if kind == KIND_HLS {
            let stem = raw_name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(raw_name.clone());
            raw_name = format!("{}.mp4", stem);
        }

        let category = detect_download_category(&raw_name);
        let dest_candidate = BrowserStorage::downloads_root_dir().join(&category).join(&raw_name);

        if dup_mode == "CHECK" {
            if let Ok(list) = self.tasks.lock() {
                if let Some(existing) = list.iter().find(|t| {
                    // Video and audio of the same page are different downloads.
                    let same = (t.url == normalized_url && t.kind == kind)
                        || (!is_ytdlp_kind(kind) && !is_ytdlp_kind(&t.kind) && t.file_name.eq_ignore_ascii_case(&raw_name));
                    same && (t.status == "DOWNLOADING"
                        || t.status == "QUEUED"
                        || t.status == "MUXING"
                        || t.status == "PAUSED"
                        || (t.status == "COMPLETED"
                            && t.saved_path.as_ref().map(|p| Path::new(p).exists()).unwrap_or(false)))
                }) {
                    return EnqueueOutcome::DuplicateDetected(existing.clone());
                }
            }
            if !is_ytdlp_kind(kind) && dest_candidate.exists() {
                return EnqueueOutcome::DuplicateDetected(DownloadTask {
                    id: 0,
                    url: normalized_url,
                    file_name: raw_name,
                    total_bytes: fs::metadata(&dest_candidate).map(|m| m.len() as i64).unwrap_or(-1),
                    downloaded_bytes: 0,
                    status: "COMPLETED".to_string(),
                    speed_bps: 0,
                    threads: threads.clamp(1, 32),
                    category,
                    supports_ranges: true,
                    referer,
                    cookie,
                    sha256: None,
                    saved_path: Some(dest_candidate.to_string_lossy().to_string()),
                    error_message: None,
                    segment_map: String::new(),
                    scheduled_start_epoch: None,
                    kind: kind.to_string(),
                    audio_url: None,
                });
            }
        } else if dup_mode == "OVERWRITE" {
            if dest_candidate.exists() {
                let _ = fs::remove_file(&dest_candidate);
            }
            let stale: Vec<u64> = self
                .tasks
                .lock()
                .map(|l| l.iter().filter(|t| t.url == normalized_url).map(|t| t.id).collect())
                .unwrap_or_default();
            for id in stale {
                self.delete(id);
            }
        } else if dup_mode == "RENAME" && !is_ytdlp_kind(kind) {
            let stem = Path::new(&raw_name).file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
            let ext = Path::new(&raw_name).extension().and_then(|s| s.to_str()).unwrap_or("bin").to_string();
            raw_name = format!("{}_{}.{}", stem, now_secs() % 100_000, ext);
        }

        let sched_epoch = delay_secs.filter(|&d| d > 0).map(|d| now_secs() + d);
        let id = new_task_id();

        let max_conc = self.max_concurrent.load(Ordering::Relaxed).max(1);
        let active_now = self
            .tasks
            .lock()
            .map(|l| {
                l.iter()
                    .filter(|t| (t.status == "DOWNLOADING" || t.status == "MUXING") && t.kind != KIND_NATIVE)
                    .count()
            })
            .unwrap_or(0);

        let should_queue = sched_epoch.is_some() || active_now >= max_conc;

        let task = DownloadTask {
            id,
            url: normalized_url,
            file_name: raw_name,
            total_bytes: -1,
            downloaded_bytes: 0,
            status: if should_queue { "QUEUED".to_string() } else { "DOWNLOADING".to_string() },
            speed_bps: 0,
            threads: threads.clamp(1, 32),
            category,
            supports_ranges: kind == KIND_HTTP,
            referer,
            cookie,
            sha256: None,
            saved_path: None,
            error_message: None,
            segment_map: if sched_epoch.is_some() { "[Scheduled]".to_string() } else { String::new() },
            scheduled_start_epoch: sched_epoch,
            kind: kind.to_string(),
            audio_url: None,
        };

        if let Ok(mut list) = self.tasks.lock() {
            list.insert(0, task.clone());
        }
        self.save_queue();
        if !should_queue {
            self.spawn_worker(rt, task);
        }
        EnqueueOutcome::Started
    }

    pub fn enqueue_dash_mux(
        &self,
        rt: &tokio::runtime::Handle,
        video_url: String,
        audio_url: String,
        output_title: String,
        referer: Option<String>,
        cookie: Option<String>,
    ) {
        let clean_title = sanitize_download_filename(&output_title.replace(' ', "_"));
        let base_name = if clean_title.is_empty() || clean_title == "download.bin" {
            format!("shunno_muxed_{}", now_secs() % 100_000)
        } else {
            clean_title.trim_end_matches(".mp4").to_string()
        };

        let task = DownloadTask {
            id: new_task_id(),
            url: video_url,
            file_name: format!("{}.mp4", base_name),
            total_bytes: -1,
            downloaded_bytes: 0,
            status: "DOWNLOADING".to_string(),
            speed_bps: 0,
            threads: 16,
            category: "Videos".to_string(),
            supports_ranges: true,
            referer,
            cookie,
            sha256: None,
            saved_path: None,
            error_message: None,
            segment_map: "[DASH Video+Audio]".to_string(),
            scheduled_start_epoch: None,
            kind: KIND_DASH.to_string(),
            audio_url: Some(audio_url),
        };

        if let Ok(mut list) = self.tasks.lock() {
            list.insert(0, task.clone());
        }
        self.save_queue();
        self.spawn_worker(rt, task);
    }

    fn spawn_worker(&self, rt: &tokio::runtime::Handle, task: DownloadTask) {
        // Stop any previous worker for this id; remember whether one existed so we let it release the files.
        let superseded = if let Ok(mut map) = self.active_flags.lock() {
            map.remove(&task.id).map(|f| f.store(false, Ordering::SeqCst)).is_some()
        } else {
            false
        };

        let running = Arc::new(AtomicBool::new(true));
        if let Ok(mut map) = self.active_flags.lock() {
            map.insert(task.id, running.clone());
        }
        update_task(&self.tasks, task.id, |t| {
            t.status = "DOWNLOADING".to_string();
            t.error_message = None;
        });

        let ctx = self.job_ctx(running.clone());
        let flags_ref = self.active_flags.clone();
        let mut task = task;
        if task.kind.is_empty() && is_hls_url(&task.url) {
            task.kind = KIND_HLS.to_string();
        }

        rt.spawn(async move {
            if superseded {
                tokio::time::sleep(Duration::from_millis(600)).await;
            }
            let res = match task.kind.as_str() {
                KIND_YTDLP | KIND_YTDLP_AUDIO | KIND_YTDLP_FLAC => run_ytdlp(&ctx, &task).await,
                KIND_DASH => run_dash(&ctx, &task).await,
                KIND_HLS => run_hls(&ctx, &task).await,
                _ => run_http(&ctx, &task).await,
            };

            // Only touch shared state if this worker was not replaced by a newer one.
            let still_owner = flags_ref
                .lock()
                .map(|m| m.get(&task.id).map(|f| Arc::ptr_eq(f, &running)).unwrap_or(true))
                .unwrap_or(true);

            if still_owner {
                if let Err(err) = res {
                    let paused = !running.load(Ordering::SeqCst) || err == "Paused";
                    update_task(&ctx.tasks, task.id, |t| {
                        t.speed_bps = 0;
                        if paused {
                            if t.status != "COMPLETED" {
                                t.status = "PAUSED".to_string();
                            }
                        } else {
                            t.status = "FAILED".to_string();
                            t.error_message = Some(err);
                        }
                    });
                }
                if let Ok(mut map) = flags_ref.lock() {
                    if map.get(&task.id).map(|f| Arc::ptr_eq(f, &running)).unwrap_or(false) {
                        map.remove(&task.id);
                    }
                }
            }
            let json = ctx.tasks.lock().ok().and_then(|l| serde_json::to_string_pretty(&*l).ok());
            if let Some(json) = json {
                write_atomic(&NativeDownloadEngine::queue_file_path(), json.as_bytes());
            }
        });
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_err() {
        let _ = fs::write(path, bytes);
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(windows)]
fn write_positioned(file: &File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "zero write"));
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(not(windows))]
fn write_positioned(file: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

// =====================================================================
// HTTP engine: probe, segmented multi-connection transfer, resumable.
// =====================================================================

struct ProbeInfo {
    total: u64,
    supports_range: bool,
    /// A live 200 response when the server ignored `Range`; reused to avoid a second request.
    full_response: Option<Response>,
}

async fn probe_target(ctx: &JobCtx, task: &DownloadTask, url: &str, rename_from_headers: bool) -> ProbeInfo {
    let rb = with_headers(ctx.client.get(url), &ctx.ua, &task.referer, &task.cookie).header(RANGE, "bytes=0-0");
    let mut info = ProbeInfo { total: 0, supports_range: false, full_response: None };
    let Ok(resp) = rb.send().await else {
        return info;
    };
    let status = resp.status();
    if !(status.is_success()) {
        return info;
    }

    if rename_from_headers {
        let cd_name = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_disposition_filename);
        let ct_ext = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(infer_extension_from_mime);
        update_task(&ctx.tasks, task.id, |t| {
            if let Some(n) = cd_name {
                t.file_name = n;
                t.category = detect_download_category(&t.file_name);
            } else if t.file_name.ends_with(".bin") {
                if let Some(ext) = ct_ext {
                    t.file_name = format!("{}.{}", &t.file_name[..t.file_name.len() - 4], ext);
                    t.category = detect_download_category(&t.file_name);
                }
            }
        });
    }

    if status == StatusCode::PARTIAL_CONTENT {
        if let Some((0, _, Some(total))) = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range)
        {
            if total > 0 {
                info.total = total;
                info.supports_range = true;
            }
        }
    } else {
        info.total = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        info.full_response = Some(resp);
    }
    info
}

fn offset_path(key: &str, idx: usize) -> PathBuf {
    NativeDownloadEngine::parts_dir().join(format!("{}_t{}.offset", key, idx))
}

fn single_offset_path(key: &str) -> PathBuf {
    NativeDownloadEngine::parts_dir().join(format!("{}_s.offset", key))
}

fn clear_offsets(key: &str) {
    let prefix = format!("{}_", key);
    if let Ok(entries) = fs::read_dir(NativeDownloadEngine::parts_dir()) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if name.starts_with(&prefix) && name.ends_with(".offset") {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Downloads `url` into `part_path`. `key` namespaces the resume sidecar files.
async fn download_http(
    ctx: &JobCtx,
    task: &DownloadTask,
    url: &str,
    part_path: &Path,
    key: &str,
    rename_from_headers: bool,
) -> Result<(), String> {
    let probe = probe_target(ctx, task, url, rename_from_headers).await;
    let total = probe.total;
    let threads = if probe.supports_range && total > 512 * 1024 { task.threads.clamp(1, 32) } else { 1 };

    update_task(&ctx.tasks, task.id, |t| {
        t.total_bytes = if total > 0 { total as i64 } else { -1 };
        t.supports_ranges = threads > 1;
        t.threads = threads;
    });

    if threads > 1 {
        let mut result = download_segmented(ctx, task, url, part_path, key, total, threads).await;
        if matches!(&result, Err(e) if e != "Paused") && ctx.is_running() {
            // One more pass: offsets are persisted, so only the missing ranges are fetched again.
            tokio::time::sleep(Duration::from_secs(3)).await;
            if ctx.is_running() {
                result = download_segmented(ctx, task, url, part_path, key, total, threads).await;
            }
        }
        match result {
            Ok(()) => return Ok(()),
            Err(e) if e == "Paused" || !ctx.is_running() => return Err("Paused".to_string()),
            Err(e) => {
                // CDN refused parallel ranges (429/403/503 or ignored Range): restart cleanly on one connection.
                clear_offsets(key);
                update_task(&ctx.tasks, task.id, |t| {
                    t.threads = 1;
                    t.supports_ranges = false;
                    t.segment_map = format!("[1T Fallback: {}]", e);
                });
                let _ = fs::remove_file(part_path);
                return download_single(ctx, task, url, part_path, key, total, false, None).await;
            }
        }
    }
    download_single(ctx, task, url, part_path, key, total, probe.supports_range, probe.full_response).await
}

async fn download_segmented(
    ctx: &JobCtx,
    task: &DownloadTask,
    url: &str,
    part_path: &Path,
    key: &str,
    total: u64,
    threads: usize,
) -> Result<(), String> {
    // Saved offsets are only valid if the preallocated part file still has the expected size.
    let part_ok = fs::metadata(part_path).map(|m| m.len() == total).unwrap_or(false);
    if !part_ok {
        clear_offsets(key);
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(false)
        .open(part_path)
        .map_err(|e| e.to_string())?;
    if !part_ok {
        file.set_len(total).map_err(|e| e.to_string())?;
    }
    let shared_file = Arc::new(file);
    let chunk = total / (threads as u64);
    let progress: Arc<Vec<AtomicU64>> = Arc::new((0..threads).map(|_| AtomicU64::new(0)).collect());
    let mut seg_totals = vec![0u64; threads];
    let mut bounds = Vec::with_capacity(threads);

    for (i, seg_total) in seg_totals.iter_mut().enumerate() {
        let start = (i as u64) * chunk;
        let end = if i == threads - 1 { total - 1 } else { start + chunk - 1 };
        let seg_len = end - start + 1;
        *seg_total = seg_len;
        bounds.push((start, end));
        if let Ok(txt) = fs::read_to_string(offset_path(key, i)) {
            if let Ok(saved) = txt.trim().parse::<u64>() {
                progress[i].store(saved.saturating_sub(start).min(seg_len), Ordering::SeqCst);
            }
        }
    }

    let done_flag = Arc::new(AtomicBool::new(false));
    let mon = {
        let progress = progress.clone();
        let tasks = ctx.tasks.clone();
        let done_flag = done_flag.clone();
        let task_id = task.id;
        tokio::spawn(async move {
            let mut last: u64 = progress.iter().map(|a| a.load(Ordering::Relaxed)).sum();
            let mut last_t = Instant::now();
            while !done_flag.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let cur: u64 = progress.iter().map(|a| a.load(Ordering::Relaxed)).sum();
                let ms = last_t.elapsed().as_millis().max(1) as u64;
                let speed = cur.saturating_sub(last) * 1000 / ms;
                last = cur;
                last_t = Instant::now();
                let mut bar = String::with_capacity(seg_totals.len() + 2);
                bar.push('[');
                for (idx, tot) in seg_totals.iter().enumerate() {
                    let r = if *tot > 0 { progress[idx].load(Ordering::Relaxed) as f64 / *tot as f64 } else { 1.0 };
                    bar.push(if r >= 0.999 { '█' } else if r >= 0.5 { '▓' } else if r > 0.05 { '▒' } else { '░' });
                }
                bar.push(']');
                update_task(&tasks, task_id, |t| {
                    t.downloaded_bytes = cur;
                    t.speed_bps = speed;
                    t.segment_map = bar;
                });
            }
        })
    };

    let mut workers = Vec::with_capacity(threads);
    for (i, &(start, end)) in bounds.iter().enumerate() {
        let mut offset = start + progress[i].load(Ordering::SeqCst);
        let ctx = ctx.clone();
        let url = url.to_string();
        let referer = task.referer.clone();
        let cookie = task.cookie.clone();
        let f = shared_file.clone();
        let prog = progress.clone();
        let off_path = offset_path(key, i);

        workers.push(tokio::spawn(async move {
            let mut last_err = String::new();
            for attempt in 0..4u32 {
                if offset > end {
                    let _ = fs::write(&off_path, offset.to_string());
                    return Ok::<(), String>(());
                }
                if !ctx.is_running() {
                    let _ = fs::write(&off_path, offset.to_string());
                    return Err("Paused".to_string());
                }
                if attempt > 0 {
                    tokio::time::sleep(Duration::from_secs(1u64 << attempt)).await;
                }

                let rb = with_headers(ctx.client.get(&url), &ctx.ua, &referer, &cookie)
                    .header(RANGE, format!("bytes={}-{}", offset, end));
                let resp = match rb.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        last_err = e.to_string();
                        continue;
                    }
                };
                let st = resp.status();
                if st != StatusCode::PARTIAL_CONTENT {
                    // 200 here means the server sent the WHOLE file: writing it at `offset` would corrupt the output.
                    let msg = format!("HTTP {} on ranged request", st.as_u16());
                    if st.is_server_error() || st == StatusCode::TOO_MANY_REQUESTS || st == StatusCode::REQUEST_TIMEOUT {
                        last_err = msg;
                        continue;
                    }
                    let _ = fs::write(&off_path, offset.to_string());
                    return Err(msg);
                }
                let range_ok = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_content_range)
                    .map(|(s, _, _)| s == offset)
                    .unwrap_or(false);
                if !range_ok {
                    let _ = fs::write(&off_path, offset.to_string());
                    return Err("Server returned a mismatched Content-Range".to_string());
                }

                let mut stream = resp.bytes_stream();
                let mut since_save = 0usize;
                let mut stream_err: Option<String> = None;
                while let Some(chunk_res) = stream.next().await {
                    if !ctx.is_running() {
                        let _ = fs::write(&off_path, offset.to_string());
                        return Err("Paused".to_string());
                    }
                    match chunk_res {
                        Ok(bytes) => {
                            // Never write past this segment's end even if the server over-delivers.
                            let room = (end + 1).saturating_sub(offset) as usize;
                            let slice = &bytes[..bytes.len().min(room)];
                            write_positioned(&f, slice, offset).map_err(|e| e.to_string())?;
                            offset += slice.len() as u64;
                            prog[i].fetch_add(slice.len() as u64, Ordering::Relaxed);
                            since_save += slice.len();
                            if since_save >= 256 * 1024 {
                                let _ = fs::write(&off_path, offset.to_string());
                                since_save = 0;
                            }
                            throttle(&ctx.speed_limit, slice.len(), threads as u64).await;
                            if offset > end {
                                break;
                            }
                        }
                        Err(e) => {
                            stream_err = Some(e.to_string());
                            break;
                        }
                    }
                }
                let _ = fs::write(&off_path, offset.to_string());
                if let Some(err_msg) = stream_err {
                    last_err = err_msg;
                    continue;
                }
                if offset <= end {
                    last_err = "Connection closed before segment completed".to_string();
                    continue;
                }
                return Ok(());
            }
            Err(last_err)
        }));
    }

    let mut first_err: Option<String> = None;
    for w in workers {
        match w.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() || e == "Paused" {
                    first_err = Some(e);
                }
            }
            Err(e) => {
                first_err.get_or_insert(e.to_string());
            }
        }
    }
    done_flag.store(true, Ordering::Relaxed);
    let _ = mon.await;
    let _ = shared_file.sync_all();

    match first_err {
        None => {
            // Only a fully successful transfer discards resume data.
            clear_offsets(key);
            update_task(&ctx.tasks, task.id, |t| {
                t.downloaded_bytes = total;
                t.speed_bps = 0;
            });
            Ok(())
        }
        Some(e) => Err(e),
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_single(
    ctx: &JobCtx,
    task: &DownloadTask,
    url: &str,
    part_path: &Path,
    key: &str,
    total: u64,
    supports_range: bool,
    reuse: Option<Response>,
) -> Result<(), String> {
    let side = single_offset_path(key);
    let mut resume_from: u64 = 0;
    if supports_range && total > 0 {
        let saved = fs::read_to_string(&side).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
        let on_disk = fs::metadata(part_path).map(|m| m.len()).unwrap_or(0);
        resume_from = saved.min(on_disk);
        if resume_from >= total {
            let _ = fs::remove_file(&side);
            return Ok(());
        }
    }

    let (resp, mut written) = if resume_from > 0 {
        let rb = with_headers(ctx.client.get(url), &ctx.ua, &task.referer, &task.cookie)
            .header(RANGE, format!("bytes={}-", resume_from));
        let r = rb.send().await.map_err(|e| e.to_string())?;
        let ok = r.status() == StatusCode::PARTIAL_CONTENT
            && r.headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range)
                .map(|(s, _, _)| s == resume_from)
                .unwrap_or(false);
        if ok {
            (r, resume_from)
        } else if r.status().is_success() {
            (r, 0) // server restarted from zero
        } else {
            return Err(format!("HTTP {}", r.status()));
        }
    } else if let Some(r) = reuse {
        (r, 0)
    } else {
        let r = with_headers(ctx.client.get(url), &ctx.ua, &task.referer, &task.cookie)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("HTTP {}", r.status()));
        }
        (r, 0)
    };

    let mut out = if written > 0 {
        let mut f = OpenOptions::new().write(true).open(part_path).map_err(|e| e.to_string())?;
        f.set_len(written).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        f
    } else {
        File::create(part_path).map_err(|e| e.to_string())?
    };

    let expected_total = if total > 0 {
        total
    } else {
        resp.content_length().map(|l| l + written).unwrap_or(0)
    };
    if expected_total > 0 {
        update_task(&ctx.tasks, task.id, |t| t.total_bytes = expected_total as i64);
    }

    let mut stream = resp.bytes_stream();
    let mut last_tick = Instant::now();
    let mut bytes_tick = 0u64;
    let mut since_save = 0u64;
    while let Some(chunk_res) = stream.next().await {
        if !ctx.is_running() {
            let _ = out.flush();
            if supports_range {
                let _ = fs::write(&side, written.to_string());
            }
            return Err("Paused".to_string());
        }
        let bytes = match chunk_res {
            Ok(b) => b,
            Err(e) => {
                let _ = out.flush();
                if supports_range {
                    let _ = fs::write(&side, written.to_string());
                }
                return Err(format!("{} (resumable)", e));
            }
        };
        out.write_all(&bytes).map_err(|e| e.to_string())?;
        written += bytes.len() as u64;
        bytes_tick += bytes.len() as u64;
        since_save += bytes.len() as u64;
        if supports_range && since_save >= 1024 * 1024 {
            let _ = out.flush();
            let _ = fs::write(&side, written.to_string());
            since_save = 0;
        }
        throttle(&ctx.speed_limit, bytes.len(), 1).await;
        let elapsed = last_tick.elapsed().as_millis() as u64;
        if elapsed >= 500 {
            let speed = (bytes_tick * 1000) / elapsed.max(1);
            last_tick = Instant::now();
            bytes_tick = 0;
            update_task(&ctx.tasks, task.id, |t| {
                t.downloaded_bytes = written;
                t.speed_bps = speed;
            });
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);

    if expected_total > 0 && written < expected_total {
        if supports_range {
            let _ = fs::write(&side, written.to_string());
        }
        return Err(format!(
            "Connection closed early ({} of {})",
            format_human_size(written),
            format_human_size(expected_total)
        ));
    }
    let _ = fs::remove_file(&side);
    update_task(&ctx.tasks, task.id, |t| {
        t.downloaded_bytes = written;
        t.speed_bps = 0;
    });
    Ok(())
}

async fn run_http(ctx: &JobCtx, task: &DownloadTask) -> Result<(), String> {
    let key = task.id.to_string();
    // Name-independent part path, so a Content-Disposition rename never orphans resume data.
    let part_path = NativeDownloadEngine::parts_dir().join(format!("{}_data.part", task.id));
    download_http(ctx, task, &task.url, &part_path, &key, true).await?;
    finalize_completed(ctx, task.id, &part_path, None).await
}

/// Moves the finished part into `<Downloads>\<Category>\`, hashes it and marks the task COMPLETED.
async fn finalize_completed(ctx: &JobCtx, task_id: u64, part_path: &Path, name_override: Option<String>) -> Result<(), String> {
    let current_name = ctx
        .tasks
        .lock()
        .ok()
        .and_then(|l| l.iter().find(|t| t.id == task_id).map(|t| t.file_name.clone()))
        .unwrap_or_else(|| "download.bin".to_string());
    let final_name = sanitize_download_filename(&name_override.unwrap_or(current_name));
    let category = detect_download_category(&final_name);
    let part = part_path.to_path_buf();

    let (dest, sha, size) = tokio::task::spawn_blocking(move || -> Result<(PathBuf, Option<String>, u64), String> {
        let target_dir = BrowserStorage::downloads_root_dir().join(&category);
        fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
        let mut dest = target_dir.join(&final_name);
        if dest.exists() {
            let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
            let ext = dest.extension().and_then(|s| s.to_str()).unwrap_or("bin").to_string();
            let mut n = 1;
            while dest.exists() {
                dest = target_dir.join(format!("{} ({}).{}", stem, n, ext));
                n += 1;
            }
        }
        if fs::rename(&part, &dest).is_err() {
            fs::copy(&part, &dest).map_err(|e| e.to_string())?;
            let _ = fs::remove_file(&part);
        }
        let sha = sha256_file(&dest);
        let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        Ok((dest, sha, size))
    })
    .await
    .map_err(|e| e.to_string())??;

    mark_completed(ctx, task_id, &dest, sha, size);
    Ok(())
}

fn mark_completed(ctx: &JobCtx, task_id: u64, dest: &Path, sha: Option<String>, size: u64) {
    if let Ok(mut list) = ctx.tasks.lock() {
        if let Some(t) = list.iter_mut().find(|i| i.id == task_id) {
            t.file_name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("download.bin").to_string();
            t.category = detect_download_category(&t.file_name);
            t.total_bytes = size as i64;
            t.downloaded_bytes = size;
            t.status = "COMPLETED".to_string();
            t.speed_bps = 0;
            t.sha256 = sha;
            t.saved_path = Some(dest.to_string_lossy().to_string());
            t.error_message = None;
            if t.segment_map.starts_with('[') && t.segment_map.contains('░') {
                t.segment_map.clear();
            }
        }
    }
}

// =====================================================================
// External tools: verified install, hidden execution, cancellation.
// =====================================================================

fn set_status(status: &Arc<Mutex<String>>, text: &str) {
    if let Ok(mut s) = status.lock() {
        *s = text.to_string();
    }
}

fn tool_busy(status: &Arc<Mutex<String>>) -> bool {
    status
        .lock()
        .map(|s| s.starts_with("Downloading") || s.starts_with("Verifying") || s.starts_with("Extracting"))
        .unwrap_or(false)
}

async fn find_tool_async(f: fn() -> Option<PathBuf>) -> Option<PathBuf> {
    tokio::task::spawn_blocking(f).await.ok().flatten()
}

async fn fetch_bytes(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .header(USER_AGENT, DEFAULT_UA)
        .header("Accept", "*/*")
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} from {}", resp.status(), url));
    }
    Ok(resp.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

/// Returns a usable ffmpeg path, installing gyan.dev's essentials build (SHA-256 verified) if needed.
async fn ensure_ffmpeg(client: &Client, status: &Arc<Mutex<String>>) -> Result<PathBuf, String> {
    let deadline = Instant::now() + Duration::from_secs(1200);
    loop {
        if let Some(p) = find_tool_async(BrowserStorage::find_ffmpeg).await {
            set_status(status, "Ready");
            return Ok(p);
        }
        if !tool_busy(status) {
            break;
        }
        if Instant::now() > deadline {
            return Err("Timed out waiting for FFmpeg install".to_string());
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    set_status(status, "Downloading FFmpeg...");
    let res: Result<PathBuf, String> = async {
        let sha_txt = String::from_utf8_lossy(&fetch_bytes(client, FFMPEG_SHA_URL).await?).to_string();
        let expected = sha_txt
            .split_whitespace()
            .next()
            .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or("Checksum file unreadable")?
            .to_ascii_lowercase();
        let bytes = fetch_bytes(client, FFMPEG_ZIP_URL).await?;
        set_status(status, "Verifying FFmpeg...");
        if sha256_bytes(&bytes) != expected {
            return Err("SHA-256 mismatch, download rejected".to_string());
        }
        set_status(status, "Extracting ffmpeg.exe...");
        tokio::task::spawn_blocking(move || -> Result<PathBuf, String> {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
            let out_exe = BrowserStorage::ffmpeg_bin_dir().join("ffmpeg.exe");
            let tmp = out_exe.with_extension("exe.tmp");
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
                let name = entry.name().replace('\\', "/");
                if name.ends_with("/bin/ffmpeg.exe") || name.eq_ignore_ascii_case("ffmpeg.exe") {
                    let mut out_f = File::create(&tmp).map_err(|e| e.to_string())?;
                    std::io::copy(&mut entry, &mut out_f).map_err(|e| e.to_string())?;
                    drop(out_f);
                    fs::rename(&tmp, &out_exe).map_err(|e| e.to_string())?;
                    return Ok(out_exe);
                }
            }
            Err("ffmpeg.exe not found in archive".to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;

    match &res {
        Ok(_) => set_status(status, "Installed!"),
        Err(e) => set_status(status, &format!("Failed: {}", e)),
    }
    res
}

/// Returns a usable yt-dlp path, installing the official release (SHA-256 verified) if needed.
async fn ensure_ytdlp(client: &Client, status: &Arc<Mutex<String>>) -> Result<PathBuf, String> {
    let deadline = Instant::now() + Duration::from_secs(900);
    loop {
        if let Some(p) = find_tool_async(BrowserStorage::find_ytdlp).await {
            set_status(status, "Ready");
            return Ok(p);
        }
        if !tool_busy(status) {
            break;
        }
        if Instant::now() > deadline {
            return Err("Timed out waiting for yt-dlp install".to_string());
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    set_status(status, "Downloading yt-dlp...");
    let res: Result<PathBuf, String> = async {
        let sums = String::from_utf8_lossy(&fetch_bytes(client, YTDLP_SUMS_URL).await?).to_string();
        let expected = sums
            .lines()
            .find_map(|l| {
                let mut it = l.split_whitespace();
                let h = it.next()?;
                let name = it.next()?.trim_start_matches('*');
                (name == "yt-dlp.exe").then(|| h.to_ascii_lowercase())
            })
            .ok_or("yt-dlp.exe missing from SHA2-256SUMS")?;
        let bytes = fetch_bytes(client, YTDLP_EXE_URL).await?;
        set_status(status, "Verifying yt-dlp...");
        if sha256_bytes(&bytes) != expected {
            return Err("SHA-256 mismatch, download rejected".to_string());
        }
        let out_exe = BrowserStorage::ffmpeg_bin_dir().join("yt-dlp.exe");
        let tmp = out_exe.with_extension("exe.tmp");
        fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &out_exe).map_err(|e| e.to_string())?;
        Ok(out_exe)
    }
    .await;

    match &res {
        Ok(_) => set_status(status, "Installed!"),
        Err(e) => set_status(status, &format!("Failed: {}", e)),
    }
    res
}

struct ToolOutcome {
    success: bool,
    stderr_tail: String,
}

fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        // PyInstaller builds (yt-dlp.exe) run a child process; kill the whole tree.
        let _ = hidden_command("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Runs a tool without a console window, feeding each stdout line to `on_line`.
/// Clearing `running` kills the process tree.
fn run_tool_blocking<F: FnMut(&str)>(mut cmd: Command, running: Arc<AtomicBool>, mut on_line: F) -> Result<ToolOutcome, String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Could not start tool: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    let finished = Arc::new(AtomicBool::new(false));

    let killer = {
        let child = child.clone();
        let finished = finished.clone();
        let running = running.clone();
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if !running.load(Ordering::Relaxed) {
                    if let Ok(mut c) = child.lock() {
                        kill_tree(&mut c);
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
    };

    let err_reader = std::thread::spawn(move || {
        let mut tail: Vec<String> = Vec::new();
        if let Some(se) = stderr {
            let mut r = BufReader::new(se);
            let mut buf = Vec::new();
            while r.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
                let line = String::from_utf8_lossy(&buf).trim().to_string();
                if !line.is_empty() {
                    tail.push(line);
                    if tail.len() > 12 {
                        tail.remove(0);
                    }
                }
                buf.clear();
            }
        }
        tail
    });

    if let Some(so) = stdout {
        let mut r = BufReader::new(so);
        let mut buf = Vec::new();
        while r.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
            let line = String::from_utf8_lossy(&buf);
            for part in line.split('\r') {
                let p = part.trim();
                if !p.is_empty() {
                    on_line(p);
                }
            }
            buf.clear();
        }
    }

    let status = loop {
        let polled = child.lock().map_err(|e| e.to_string())?.try_wait().map_err(|e| e.to_string())?;
        if let Some(s) = polled {
            break s;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    finished.store(true, Ordering::Relaxed);
    let _ = killer.join();
    let tail = err_reader.join().unwrap_or_default();
    let tail_txt = tail
        .iter()
        .rev()
        .find(|l| l.starts_with("ERROR") || l.contains("rror"))
        .or_else(|| tail.last())
        .cloned()
        .unwrap_or_default();
    Ok(ToolOutcome { success: status.success(), stderr_tail: tail_txt })
}

fn ffmpeg_header_blob(task: &DownloadTask) -> String {
    let mut h = String::new();
    if let Some(r) = &task.referer {
        if r.starts_with("http") && !r.contains(['\r', '\n']) {
            h.push_str(&format!("Referer: {}\r\n", r));
        }
    }
    if let Some(c) = &task.cookie {
        if !c.is_empty() && !c.contains(['\r', '\n']) {
            h.push_str(&format!("Cookie: {}\r\n", c));
        }
    }
    h
}

/// Polls an output file's size into the task while an external tool writes it.
fn spawn_size_monitor(tasks: Arc<Mutex<Vec<DownloadTask>>>, task_id: u64, path: PathBuf, stop: Arc<AtomicBool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last = 0u64;
        let mut last_t = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let cur = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let ms = last_t.elapsed().as_millis().max(1) as u64;
            let speed = cur.saturating_sub(last) * 1000 / ms;
            last = cur;
            last_t = Instant::now();
            update_task(&tasks, task_id, |t| {
                t.downloaded_bytes = cur;
                t.speed_bps = speed;
            });
        }
    })
}

// =====================================================================
// HLS (.m3u8)
// =====================================================================

async fn run_hls(ctx: &JobCtx, task: &DownloadTask) -> Result<(), String> {
    let out_name = {
        let n = ctx
            .tasks
            .lock()
            .ok()
            .and_then(|l| l.iter().find(|t| t.id == task.id).map(|t| t.file_name.clone()))
            .unwrap_or_else(|| task.file_name.clone());
        let stem = n.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(n);
        stem
    };

    // FFmpeg handles encryption, byte ranges, discontinuities and remuxes to a real MP4.
    update_task(&ctx.tasks, task.id, |t| t.segment_map = "[HLS: preparing FFmpeg]".to_string());
    if let Ok(ffmpeg) = ensure_ffmpeg(&ctx.client, &ctx.ffmpeg_status).await {
        let part_path = NativeDownloadEngine::parts_dir().join(format!("{}_hls.part", task.id));
        let _ = fs::remove_file(&part_path);
        update_task(&ctx.tasks, task.id, |t| {
            t.segment_map = "[HLS via FFmpeg]".to_string();
            t.threads = 1;
            t.supports_ranges = false;
        });
        let mut cmd = hidden_command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-nostdin"]);
        cmd.args(["-user_agent", &ctx.ua]);
        let headers = ffmpeg_header_blob(task);
        if !headers.is_empty() {
            cmd.args(["-headers", &headers]);
        }
        cmd.args(["-i", &task.url, "-map", "0:v?", "-map", "0:a?", "-c", "copy", "-f", "mp4"]);
        cmd.arg(&part_path);

        let stop = Arc::new(AtomicBool::new(false));
        let mon = spawn_size_monitor(ctx.tasks.clone(), task.id, part_path.clone(), stop.clone());
        let running = ctx.running.clone();
        let outcome = tokio::task::spawn_blocking(move || run_tool_blocking(cmd, running, |_| {}))
            .await
            .map_err(|e| e.to_string())?;
        stop.store(true, Ordering::Relaxed);
        let _ = mon.await;
        let outcome = outcome?;
        if !ctx.is_running() {
            let _ = fs::remove_file(&part_path);
            return Err("Paused".to_string());
        }
        if outcome.success && part_path.exists() {
            return finalize_completed(ctx, task.id, &part_path, Some(format!("{}.mp4", out_name))).await;
        }
        let _ = fs::remove_file(&part_path);
        return Err(format!("FFmpeg HLS download failed: {}", outcome.stderr_tail));
    }

    // Fallback without FFmpeg: plain (unencrypted) segment concatenation.
    let base = url::Url::parse(&task.url).map_err(|e| e.to_string())?;
    let get_text = |u: url::Url| {
        let rb = with_headers(ctx.client.get(u.as_str()), &ctx.ua, &task.referer, &task.cookie);
        async move {
            let r = rb.send().await.map_err(|e| e.to_string())?;
            if !r.status().is_success() {
                return Err(format!("HTTP {} for playlist", r.status()));
            }
            r.text().await.map_err(|e| e.to_string())
        }
    };
    let mut pl = parse_hls_playlist(&get_text(base.clone()).await?, &base);
    if !pl.variants.is_empty() {
        let best = pl.variants.iter().max_by_key(|(bw, _)| *bw).map(|(_, u)| u.clone()).ok_or("No variant")?;
        pl = parse_hls_playlist(&get_text(best.clone()).await?, &best);
    }
    if pl.encrypted || pl.byte_ranges {
        return Err("This HLS stream is encrypted or byte-ranged; FFmpeg is required (install failed)".to_string());
    }
    if pl.segments.is_empty() {
        return Err("No media segments in M3U8 playlist".to_string());
    }

    let fmp4 = pl.init_segment.is_some();
    let part_path = NativeDownloadEngine::parts_dir().join(format!("{}_hls.part", task.id));
    let mut out = File::create(&part_path).map_err(|e| e.to_string())?;
    let mut all: Vec<url::Url> = Vec::new();
    if let Some(init) = pl.init_segment.clone() {
        all.push(init);
    }
    all.extend(pl.segments.iter().cloned());
    let n = all.len();
    let mut done = 0u64;
    let started = Instant::now();
    for (idx, seg) in all.iter().enumerate() {
        if !ctx.is_running() {
            return Err("Paused".to_string());
        }
        let mut attempt = 0;
        let bytes = loop {
            let rb = with_headers(ctx.client.get(seg.as_str()), &ctx.ua, &task.referer, &task.cookie);
            match rb.send().await {
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) => break b,
                    Err(e) if attempt >= 3 => return Err(e.to_string()),
                    Err(_) => {}
                },
                Ok(r) if attempt >= 3 => return Err(format!("HTTP {} for segment {}", r.status(), idx + 1)),
                Err(e) if attempt >= 3 => return Err(e.to_string()),
                _ => {}
            }
            attempt += 1;
            tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
        };
        out.write_all(&bytes).map_err(|e| e.to_string())?;
        done += bytes.len() as u64;
        throttle(&ctx.speed_limit, bytes.len(), 1).await;
        let est = (done * n as u64) / (idx as u64 + 1);
        let speed = done * 1000 / (started.elapsed().as_millis().max(1) as u64);
        update_task(&ctx.tasks, task.id, |t| {
            t.downloaded_bytes = done;
            t.total_bytes = est as i64;
            t.speed_bps = speed;
            t.segment_map = format!("[HLS {}/{}]", idx + 1, n);
        });
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    // Raw MPEG-TS segments are a .ts file, not an .mp4; fMP4 segments with an init header are .mp4.
    let ext = if fmp4 { "mp4" } else { "ts" };
    finalize_completed(ctx, task.id, &part_path, Some(format!("{}.{}", out_name, ext))).await
}

// =====================================================================
// DASH video+audio pair -> FFmpeg mux
// =====================================================================

async fn run_dash(ctx: &JobCtx, task: &DownloadTask) -> Result<(), String> {
    let audio_url = task
        .audio_url
        .clone()
        .ok_or("Missing audio URL for DASH task; remove it and mux again")?;
    let final_name = task.file_name.clone();

    update_task(&ctx.tasks, task.id, |t| t.segment_map = "[DASH: preparing FFmpeg]".to_string());
    let ffmpeg = ensure_ffmpeg(&ctx.client, &ctx.ffmpeg_status)
        .await
        .map_err(|e| format!("FFmpeg is required to merge video and audio: {}", e))?;

    let parts = NativeDownloadEngine::parts_dir();
    let vid_part = parts.join(format!("{}_video.part", task.id));
    let aud_part = parts.join(format!("{}_audio.part", task.id));
    let vid_done = parts.join(format!("{}_video.done", task.id));
    let aud_done = parts.join(format!("{}_audio.done", task.id));
    let muxed = parts.join(format!("{}_muxed.mp4", task.id));

    for (label, url, part, done, key) in [
        ("video", task.url.clone(), &vid_part, &vid_done, format!("{}_v", task.id)),
        ("audio", audio_url, &aud_part, &aud_done, format!("{}_a", task.id)),
    ] {
        if done.exists() && part.exists() {
            continue;
        }
        update_task(&ctx.tasks, task.id, |t| t.segment_map = format!("[DASH {}]", label));
        download_http(ctx, task, &url, part, &key, false).await?;
        let _ = fs::write(done, b"ok");
    }

    update_task(&ctx.tasks, task.id, |t| {
        t.status = "MUXING".to_string();
        t.segment_map = "[FFmpeg Muxing...]".to_string();
        t.speed_bps = 0;
    });
    let mut cmd = hidden_command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&vid_part)
        .arg("-i")
        .arg(&aud_part)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy", "-f", "mp4"])
        .arg(&muxed);
    let running = ctx.running.clone();
    let outcome = tokio::task::spawn_blocking(move || run_tool_blocking(cmd, running, |_| {}))
        .await
        .map_err(|e| e.to_string())??;
    if !ctx.is_running() {
        let _ = fs::remove_file(&muxed);
        return Err("Paused".to_string());
    }
    if !(outcome.success && muxed.exists()) {
        // Keep the downloaded parts so Resume retries the mux without downloading again.
        let _ = fs::remove_file(&muxed);
        return Err(format!("FFmpeg muxing failed: {}", outcome.stderr_tail));
    }
    for p in [&vid_part, &aud_part, &vid_done, &aud_done] {
        let _ = fs::remove_file(p);
    }
    finalize_completed(ctx, task.id, &muxed, Some(final_name)).await
}

// =====================================================================
// yt-dlp (YouTube and DASH manifests)
// =====================================================================

fn parse_num(s: &str) -> Option<u64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64)
}

async fn run_ytdlp(ctx: &JobCtx, task: &DownloadTask) -> Result<(), String> {
    update_task(&ctx.tasks, task.id, |t| {
        t.segment_map = "[yt-dlp: preparing]".to_string();
        t.threads = 1;
        t.supports_ranges = false;
    });
    let ytdlp = ensure_ytdlp(&ctx.client, &ctx.ytdlp_status)
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
    // FFmpeg lets yt-dlp merge the best video and audio; without it we fall back to a single progressive file.
    let ffmpeg = ensure_ffmpeg(&ctx.client, &ctx.ffmpeg_status).await.ok();
    let audio_only = task.kind == KIND_YTDLP_AUDIO || task.kind == KIND_YTDLP_FLAC;
    if audio_only && ffmpeg.is_none() {
        return Err("FFmpeg is required to save audio; install it from Downloads settings".to_string());
    }

    let target_dir = BrowserStorage::downloads_root_dir().join(match task.kind.as_str() {
        KIND_YTDLP_FLAC => "Lossless Audio",
        KIND_YTDLP_AUDIO => "Audio",
        _ => "Videos",
    });
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    let temp_dir = NativeDownloadEngine::parts_dir().join(format!("{}_ytdlp", task.id));
    fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let mut cmd = hidden_command(&ytdlp);
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    // Logged-in sites (Facebook, Instagram, Vimeo private) need the tab's session. Not sent for YouTube:
    // yt-dlp documents that using account cookies there can get the account flagged.
    let cookie_file = temp_dir.join("cookies.txt");
    let host = url::Url::parse(&task.url).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())).unwrap_or_default();
    let is_youtube = host.ends_with("youtube.com") || host == "youtu.be";
    if let (Some(c), false) = (&task.cookie, is_youtube || host.is_empty()) {
        if let Some(jar) = netscape_cookie_jar(&host, c) {
            if fs::write(&cookie_file, jar).is_ok() {
                cmd.arg("--cookies").arg(&cookie_file);
            }
        }
    }
    cmd.args(["--encoding", "utf-8", "--newline", "--no-colors", "--no-playlist", "--no-mtime", "--progress"]);
    cmd.args(["--retries", "10", "--fragment-retries", "10", "--concurrent-fragments", "4"]);
    cmd.args(["--user-agent", &ctx.ua]);
    if let Some(r) = &task.referer {
        if r.starts_with("http") && !is_ytdlp_url(&task.url) {
            cmd.args(["--referer", r]);
        }
    }
    let limit = ctx.speed_limit.load(Ordering::Relaxed);
    if limit > 0 {
        cmd.args(["--limit-rate", &limit.to_string()]);
    }
    if let Some(ff) = ffmpeg.as_ref().filter(|f| f.is_absolute()) {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    match (task.kind.as_str(), &ffmpeg) {
        // Best audio stream, kept in its original codec (no generational loss).
        (KIND_YTDLP_AUDIO, _) => {
            cmd.args(["-f", "ba/b", "-x", "--audio-format", "best"]);
        }
        // Best audio stream re-encoded losslessly to FLAC. Bit-exact to the decoded source; adds no detail.
        (KIND_YTDLP_FLAC, _) => {
            cmd.args(["-f", "ba/b", "-x", "--audio-format", "flac"]);
        }
        (_, Some(_)) => {
            cmd.args(["-f", "bv*+ba/b", "-S", "res,ext:mp4:m4a", "--merge-output-format", "mp4"]);
        }
        (_, None) => {
            cmd.args(["-f", "b"]);
        }
    }
    cmd.arg("-P").arg(&target_dir);
    cmd.arg("-P").arg(format!("temp:{}", temp_dir.to_string_lossy()));
    cmd.args(["-o", "%(title).150B [%(id)s].%(ext)s"]);
    cmd.args([
        "--progress-template",
        "download:SHUNNO %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s",
    ]);
    cmd.args(["--print", "after_move:SHUNNOFILE %(filepath)s"]);
    cmd.arg("--").arg(&task.url);

    let tasks = ctx.tasks.clone();
    let task_id = task.id;
    let running = ctx.running.clone();
    let started = SystemTime::now();
    let outcome_and_path = tokio::task::spawn_blocking(move || {
        let mut final_path: Option<String> = None;
        let mut phase = 0u32;
        let mut last_dl = 0u64;
        let outcome = run_tool_blocking(cmd, running, |line| {
            if let Some(rest) = line.strip_prefix("SHUNNOFILE ") {
                final_path = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("SHUNNO ") {
                let f: Vec<&str> = rest.split_whitespace().collect();
                let dl = f.first().and_then(|s| parse_num(s)).unwrap_or(0);
                let total = f.get(1).and_then(|s| parse_num(s)).or_else(|| f.get(2).and_then(|s| parse_num(s)));
                let speed = f.get(3).and_then(|s| parse_num(s)).unwrap_or(0);
                if dl + 1024 < last_dl {
                    phase += 1; // video finished, audio stream started
                }
                last_dl = dl;
                update_task(&tasks, task_id, |t| {
                    t.downloaded_bytes = dl;
                    t.total_bytes = total.map(|v| v as i64).unwrap_or(-1);
                    t.speed_bps = speed;
                    t.segment_map = if phase == 0 { "[yt-dlp]".to_string() } else { format!("[yt-dlp stream {}]", phase + 1) };
                });
            } else if line.starts_with("[ExtractAudio]") {
                update_task(&tasks, task_id, |t| {
                    t.status = "MUXING".to_string();
                    t.segment_map = "[Converting audio]".to_string();
                    t.speed_bps = 0;
                });
            } else if line.starts_with("[Merger]") || line.starts_with("[VideoRemuxer]") {
                update_task(&tasks, task_id, |t| {
                    t.status = "MUXING".to_string();
                    t.segment_map = "[FFmpeg Muxing...]".to_string();
                    t.speed_bps = 0;
                });
            }
        });
        (outcome, final_path)
    })
    .await
    .map_err(|e| e.to_string())?;
    let (outcome, final_path) = outcome_and_path;
    let outcome = outcome?;

    let _ = fs::remove_file(&cookie_file);
    if !ctx.is_running() {
        return Err("Paused".to_string());
    }
    if !outcome.success {
        let msg = if outcome.stderr_tail.is_empty() { "yt-dlp failed".to_string() } else { outcome.stderr_tail };
        let hint = if msg.contains("Unsupported URL") {
            " (this site is not supported by yt-dlp; try Grab or the stream list)"
        } else {
            ""
        };
        return Err(format!("{}{}", msg, hint));
    }

    let path = final_path
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| newest_file_since(&target_dir, started))
        .ok_or("yt-dlp finished but the output file was not found")?;
    let _ = fs::remove_dir_all(&temp_dir);

    let p2 = path.clone();
    let (sha, size) = tokio::task::spawn_blocking(move || (sha256_file(&p2), fs::metadata(&p2).map(|m| m.len()).unwrap_or(0)))
        .await
        .map_err(|e| e.to_string())?;
    mark_completed(ctx, task.id, &path, sha, size);
    Ok(())
}

/// Builds a Netscape cookies.txt for one host from a `Cookie:` header value.
pub fn netscape_cookie_jar(host: &str, cookie_header: &str) -> Option<String> {
    let host = host.trim().trim_start_matches('.');
    if host.is_empty() {
        return None;
    }
    let base = host.strip_prefix("www.").unwrap_or(host);
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    let mut n = 0;
    for pair in cookie_header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() || name.contains(['\t', '\n', '\r']) || value.contains(['\t', '\n', '\r']) {
            continue;
        }
        out.push_str(&format!(".{}\tTRUE\t/\tFALSE\t0\t{}\t{}\n", base, name, value));
        n += 1;
    }
    (n > 0).then_some(out)
}

fn newest_file_since(dir: &Path, since: SystemTime) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            let m = md.modified().ok()?;
            (md.is_file() && m >= since).then(|| (m, e.path()))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}
