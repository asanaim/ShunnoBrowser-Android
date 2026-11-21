use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// Stored history entries (newest first).
pub const HISTORY_CAP: usize = 250;

pub const APP_VERSION: &str = "v2.24.0";

/// Build a `Command` that never flashes a console window (this is a GUI subsystem app).
pub fn hidden_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchEngine {
    pub name: String,
    pub query_url: String,
}

fn default_site_zoom() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SitePolicy {
    pub adblock: bool,
    pub js_enabled: bool,
    pub images_enabled: bool,
    /// Request the mobile version of the site (real HTTP User-Agent). Replaces the old
    /// desktop_ua switch, which only spoofed navigator.userAgent and did nothing on desktop.
    #[serde(default)]
    pub mobile_ua: bool,
    pub sponsorblock: bool,
    pub auto_reject_cookies: bool,
    /// Fingerprinting protection (per-site farbling of canvas, WebGL, audio, CPU count).
    #[serde(default = "default_true")]
    pub fp_protect: bool,
    #[serde(default = "default_site_zoom")]
    pub zoom: f64,
}

impl Default for SitePolicy {
    fn default() -> Self {
        Self {
            adblock: true,
            js_enabled: true,
            images_enabled: true,
            mobile_ua: false,
            sponsorblock: true,
            auto_reject_cookies: true,
            fp_protect: true,
            zoom: 1.0,
        }
    }
}

fn default_bookmark_folder() -> String {
    "General".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub title: String,
    pub url: String,
    #[serde(default = "default_bookmark_folder")]
    pub folder: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub title: String,
    pub url: String,
    pub visited_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub url: String,
    /// Space the tab belongs to (0 = the first Space, for sessions saved before Spaces existed).
    #[serde(default)]
    pub space: u32,
    /// Tab group id (see tab_groups.json), if the tab was in a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
    /// Window the tab was in (0 = first window).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub window: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Split a saved session into windows, in window order (window 0 first). Empty windows are dropped.
pub fn session_windows(tabs: Vec<SavedTab>) -> Vec<Vec<SavedTab>> {
    let mut idx: Vec<u32> = tabs.iter().map(|t| t.window).collect();
    idx.sort_unstable();
    idx.dedup();
    let mut out: Vec<Vec<SavedTab>> = idx.iter().map(|_| Vec::new()).collect();
    for t in tabs {
        if let Ok(k) = idx.binary_search(&t.window) {
            out[k].push(t);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Tab groups: named, coloured runs of tabs inside one Space that can be collapsed.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: u32,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub collapsed: bool,
}

pub const GROUP_COLORS: &[&str] = &["#0A84FF", "#30D158", "#FF9F0A", "#FF375F", "#BF5AF2", "#64D2FF", "#FFD60A", "#8E8E93"];

/// Trim a group name (may be empty: an unnamed group shows as a coloured dot) and check the colour.
pub fn clean_group(name: &str, color: &str) -> (String, String) {
    let n: String = name.trim().chars().take(30).collect();
    let c = if GROUP_COLORS.contains(&color) { color.to_string() } else { GROUP_COLORS[0].to_string() };
    (n, c)
}

/// A stable reordering that makes every group's tabs contiguous, gathered at the position of the
/// group's first tab. Input: each tab's group. Output: old indices in their new order.
pub fn group_contiguous_order(groups: &[Option<u32>]) -> Vec<usize> {
    let mut out = Vec::with_capacity(groups.len());
    let mut placed = vec![false; groups.len()];
    for i in 0..groups.len() {
        if placed[i] {
            continue;
        }
        match groups[i] {
            None => {
                out.push(i);
                placed[i] = true;
            }
            Some(g) => {
                for j in i..groups.len() {
                    if !placed[j] && groups[j] == Some(g) {
                        out.push(j);
                        placed[j] = true;
                    }
                }
            }
        }
    }
    out
}

/// After a tab is dragged to index `to`, which group should it be in? Dropped between two tabs of
/// one group: that group. Dropped next to a tab of its own group: unchanged. Otherwise: none.
pub fn group_after_drop(groups: &[Option<u32>], to: usize) -> Option<u32> {
    let left = to.checked_sub(1).and_then(|i| groups.get(i).copied().flatten());
    let right = groups.get(to + 1).copied().flatten();
    let own = groups.get(to).copied().flatten();
    if left.is_some() && left == right {
        return left;
    }
    match own {
        Some(g) if left == Some(g) || right == Some(g) => Some(g),
        _ => None,
    }
}

/// Arc-style Space: a named, coloured set of tabs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: u32,
    pub name: String,
    /// "#RRGGBB" accent.
    pub color: String,
    /// One emoji or character.
    pub icon: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpacesFile {
    pub spaces: Vec<Space>,
    pub active: u32,
}

impl Default for SpacesFile {
    fn default() -> Self {
        SpacesFile {
            spaces: vec![Space { id: 1, name: "Personal".into(), color: "#0A84FF".into(), icon: "🏠".into() }],
            active: 1,
        }
    }
}

impl SpacesFile {
    /// Valid after load: at least one Space, `active` exists, names/colours sane.
    pub fn normalized(mut self) -> Self {
        self.spaces.retain(|s| s.id != 0);
        if self.spaces.is_empty() {
            return SpacesFile::default();
        }
        for sp in &mut self.spaces {
            sp.name = sanitize_space_name(&sp.name);
            if !is_hex_color(&sp.color) {
                sp.color = "#0A84FF".into();
            }
            sp.icon = sp.icon.chars().take(2).collect();
        }
        if !self.spaces.iter().any(|s| s.id == self.active) {
            self.active = self.spaces[0].id;
        }
        self
    }

    pub fn next_id(&self) -> u32 {
        self.spaces.iter().map(|s| s.id).max().unwrap_or(0) + 1
    }

    /// Map a saved Space id onto an existing one (0 or unknown → the first Space).
    pub fn resolve(&self, id: u32) -> u32 {
        if self.spaces.iter().any(|s| s.id == id) { id } else { self.spaces[0].id }
    }
}

pub fn sanitize_space_name(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(24).collect();
    let t = t.trim().to_string();
    if t.is_empty() { "Space".into() } else { t }
}

pub fn is_hex_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())
}

/// Which tab (index) to show when entering `space`: the remembered one if it is still in that
/// Space, else the first tab there. `tabs` = (tab id, space) in strip order.
pub fn pick_tab_for_space(tabs: &[(usize, u32)], space: u32, remembered: Option<usize>) -> Option<usize> {
    if let Some(id) = remembered {
        if let Some(i) = tabs.iter().position(|&(t, s)| t == id && s == space) {
            return Some(i);
        }
    }
    tabs.iter().position(|&(_, s)| s == space)
}

/// Next/previous tab index within the same Space as `current`, wrapping around.
pub fn step_in_space(tabs: &[(usize, u32)], current: usize, forward: bool) -> usize {
    let Some(&(_, space)) = tabs.get(current) else { return current };
    let members: Vec<usize> = tabs.iter().enumerate().filter(|(_, &(_, s))| s == space).map(|(i, _)| i).collect();
    let pos = members.iter().position(|&i| i == current).unwrap_or(0);
    let n = members.len();
    if n <= 1 {
        return current;
    }
    members[if forward { (pos + 1) % n } else { (pos + n - 1) % n }]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuickShortcut {
    pub title: String,
    pub url: String,
}

fn default_max_concurrent() -> usize { 3 }
fn default_on_complete() -> String { "NONE".to_string() }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalSettings {
    pub full_dark_mode: bool,
    pub background_media_play: bool,
    pub strip_tracking_params: bool,
    pub block_popups: bool,
    pub drop_basket_enabled: bool,
    #[serde(default)]
    pub auto_pip_on_blur: bool,
    #[serde(default)]
    pub auto_clean_completed: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_downloads: usize,
    #[serde(default = "default_on_complete")]
    pub on_queue_complete: String,
    #[serde(default)]
    pub custom_downloads_dir: Option<String>,
    /// WebView2 built-in password manager (DPAPI-encrypted in the profile). Never active in private tabs.
    #[serde(default = "default_true")]
    pub save_passwords: bool,
    /// Check for signed updates daily.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Upgrade http:// navigations to https:// and warn when a site has no secure version.
    #[serde(default = "default_true")]
    pub https_only: bool,
    /// Edge tracking prevention: Strict when true, Balanced when false.
    #[serde(default)]
    pub strict_tracking: bool,
    /// Close tabs unused for this many hours (Arc-style auto-archive). 0 = never.
    #[serde(default)]
    pub auto_archive_hours: u32,
    /// Shift+click a link opens it in a floating Peek preview (Arc-style).
    #[serde(default = "default_true")]
    pub peek_links: bool,
    /// "system" (follow Windows), "light" or "dark".
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// Chromium auto dark mode for sites without a dark theme (applies at next start).
    #[serde(default)]
    pub force_dark_pages: bool,
    /// Hand page-started downloads to Shunno's multi-connection engine when it can fetch them.
    #[serde(default = "default_true")]
    pub download_takeover: bool,
    /// Reader appearance (JSON: size, font, theme, width), validated by the page script.
    #[serde(default = "default_reader_prefs")]
    pub reader_prefs: String,
    /// Links opened from other apps go to Little Shunno (Arc's Little Arc) instead of a main-window tab.
    #[serde(default = "default_true")]
    pub little_for_links: bool,
    #[serde(default)]
    pub approved_script_hashes: Vec<String>,
    #[serde(default)]
    pub quick_shortcuts: Vec<QuickShortcut>,
    pub default_threads: usize,
    pub speed_limit_bps: u64,
    pub active_search_engine: usize,
    pub search_engines: Vec<SearchEngine>,
}

fn default_true() -> bool {
    true
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            full_dark_mode: false,
            background_media_play: true,
            strip_tracking_params: true,
            block_popups: true,
            drop_basket_enabled: true,
            auto_pip_on_blur: false,
            auto_clean_completed: false,
            max_concurrent_downloads: 3,
            on_queue_complete: "NONE".to_string(),
            custom_downloads_dir: None,
            save_passwords: true,
            https_only: true,
            auto_update: true,
            strict_tracking: false,
            auto_archive_hours: 0,
            peek_links: true,
            theme_mode: default_theme_mode(),
            force_dark_pages: false,
            download_takeover: true,
            reader_prefs: default_reader_prefs(),
            little_for_links: true,
            approved_script_hashes: Vec::new(),
            quick_shortcuts: vec![
                QuickShortcut { title: "DuckDuckGo".to_string(), url: "https://duckduckgo.com".to_string() },
                QuickShortcut { title: "YouTube".to_string(), url: "https://www.youtube.com".to_string() },
                QuickShortcut { title: "GitHub".to_string(), url: "https://github.com".to_string() },
                QuickShortcut { title: "Wikipedia".to_string(), url: "https://www.wikipedia.org".to_string() },
                QuickShortcut { title: "Internet Archive".to_string(), url: "https://archive.org".to_string() },
                QuickShortcut { title: "Krishi Bank".to_string(), url: "https://krishibank.org.bd".to_string() },
            ],
            default_threads: 16,
            speed_limit_bps: 0,
            active_search_engine: 0,
            search_engines: vec![
                SearchEngine {
                    name: "DuckDuckGo".to_string(),
                    query_url: "https://duckduckgo.com/?q=%s".to_string(),
                },
                SearchEngine {
                    name: "Google".to_string(),
                    query_url: "https://www.google.com/search?q=%s".to_string(),
                },
                SearchEngine {
                    name: "Startpage".to_string(),
                    query_url: "https://www.startpage.com/sp/search?query=%s".to_string(),
                },
                SearchEngine {
                    name: "Bing".to_string(),
                    query_url: "https://www.bing.com/search?q=%s".to_string(),
                },
                SearchEngine {
                    name: "YouTube".to_string(),
                    query_url: "https://www.youtube.com/results?search_query=%s".to_string(),
                },
            ],
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Profiles (Safari-style): each profile is a separate Shunno process with its own data folder:
// cookies, logins, history, bookmarks, passwords, permissions, settings, Spaces and downloads.
// The default profile keeps the original folder, so existing users need no migration.
// ---------------------------------------------------------------------------------------------

pub const DEFAULT_PROFILE: &str = "default";
static PROFILE_ID: OnceLock<String> = OnceLock::new();

/// A profile id is a short slug: lowercase ASCII letters, digits and '-', 1..=32 chars.
pub fn valid_profile_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !id.starts_with('-')
}

/// `--profile=<id>` among the process arguments; anything invalid means the default profile.
pub fn profile_from_args<I: IntoIterator<Item = String>>(args: I) -> String {
    args.into_iter()
        .find_map(|a| a.strip_prefix("--profile=").map(|v| v.trim().to_string()))
        .filter(|v| valid_profile_id(v))
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string())
}

/// Fix the profile for this process. Call once, before anything reads app data.
pub fn set_profile(id: &str) {
    let _ = PROFILE_ID.set(if valid_profile_id(id) { id.to_string() } else { DEFAULT_PROFILE.to_string() });
}

pub fn profile_id() -> &'static str {
    PROFILE_ID.get().map(|s| s.as_str()).unwrap_or(DEFAULT_PROFILE)
}

/// Data folder of a profile, relative to the shared folder.
pub fn profile_dir_in(shared: &std::path::Path, id: &str) -> PathBuf {
    if id == DEFAULT_PROFILE {
        shared.to_path_buf()
    } else {
        shared.join("Profiles").join(id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// Swatches offered for profiles (also accepted when saved).
pub const PROFILE_COLORS: &[&str] = &["#0A84FF", "#30D158", "#FF9F0A", "#FF375F", "#BF5AF2", "#64D2FF", "#8E8E93"];

pub fn default_profiles() -> Vec<Profile> {
    vec![Profile { id: DEFAULT_PROFILE.into(), name: "Personal".into(), color: PROFILE_COLORS[0].into() }]
}

/// Clean a profile list read from disk: valid unique ids, names 1..=40 chars, known colours,
/// and the default profile always present and first.
pub fn sanitize_profiles(list: Vec<Profile>) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    for mut p in list {
        p.name = p.name.trim().chars().take(40).collect();
        if !valid_profile_id(&p.id) || p.name.is_empty() || out.iter().any(|q| q.id == p.id) {
            continue;
        }
        if !PROFILE_COLORS.contains(&p.color.as_str()) {
            p.color = PROFILE_COLORS[0].into();
        }
        out.push(p);
    }
    match out.iter().position(|p| p.id == DEFAULT_PROFILE) {
        Some(0) => {}
        Some(i) => {
            let d = out.remove(i);
            out.insert(0, d);
        }
        None => out.insert(0, default_profiles().remove(0)),
    }
    out.truncate(12);
    out
}

/// A new id from a name ("Work Stuff" -> "work-stuff"), unique within `taken`.
pub fn new_profile_id(name: &str, taken: &[Profile]) -> String {
    let mut slug: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    slug.truncate(24);
    let slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() || slug == DEFAULT_PROFILE { "profile".to_string() } else { slug };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|p| p.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Where links opened from other apps go: "last" (the profile used most recently) or a profile id.
pub const LINKS_LAST: &str = "last";

/// Pick the profile for a launch. An explicit `--profile=` wins. Otherwise a launch carrying a
/// link follows the links setting; a plain launch opens the default profile.
pub fn choose_launch_profile(arg_profile: Option<&str>, has_url: bool, links_pref: &str, last_used: Option<&str>, known: &[Profile]) -> String {
    let exists = |id: &str| known.iter().any(|p| p.id == id);
    if let Some(a) = arg_profile.filter(|a| valid_profile_id(a)) {
        return a.to_string();
    }
    if has_url {
        let want = if links_pref == LINKS_LAST { last_used.unwrap_or(DEFAULT_PROFILE) } else { links_pref };
        if valid_profile_id(want) && exists(want) {
            return want.to_string();
        }
    }
    DEFAULT_PROFILE.to_string()
}

static ANDROID_BASE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_android_base_dir(path: &str) {
    let _ = ANDROID_BASE_DIR.set(PathBuf::from(path));
}

pub struct BrowserStorage;

impl BrowserStorage {
    /// Folder shared by every profile: helper tools, block lists and the profile list.
    pub fn shared_data_dir() -> PathBuf {
        let base = ANDROID_BASE_DIR
            .get()
            .cloned()
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("ShunnoBrowser");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// This process's profile folder.
    pub fn app_data_dir() -> PathBuf {
        let dir = profile_dir_in(&Self::shared_data_dir(), profile_id());
        let _ = fs::create_dir_all(&dir);
        dir
    }

    pub fn load_profiles() -> Vec<Profile> {
        let path = Self::shared_data_dir().join("profiles.json");
        let list = fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        sanitize_profiles(list)
    }

    /// The links setting (shared by all profiles).
    pub fn load_links_profile() -> String {
        fs::read_to_string(Self::shared_data_dir().join("links_profile.txt"))
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| t == LINKS_LAST || valid_profile_id(t))
            .unwrap_or_else(|| LINKS_LAST.to_string())
    }
    pub fn save_links_profile(v: &str) {
        if v == LINKS_LAST || valid_profile_id(v) {
            let _ = fs::write(Self::shared_data_dir().join("links_profile.txt"), v);
        }
    }
    /// The profile whose window was focused most recently.
    pub fn load_last_profile() -> Option<String> {
        fs::read_to_string(Self::shared_data_dir().join("last_profile.txt")).ok().map(|t| t.trim().to_string()).filter(|t| valid_profile_id(t))
    }
    pub fn save_last_profile(id: &str) {
        let p = Self::shared_data_dir().join("last_profile.txt");
        if fs::read_to_string(&p).ok().as_deref().map(str::trim) != Some(id) {
            let _ = fs::write(p, id);
        }
    }

    pub fn save_profiles(list: &[Profile]) {
        let path = Self::shared_data_dir().join("profiles.json");
        if let Ok(t) = serde_json::to_string_pretty(list) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, t).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
    }

    fn custom_dl_lock() -> &'static Mutex<Option<String>> {
        static CUSTOM_DL_DIR: OnceLock<Mutex<Option<String>>> = OnceLock::new();
        CUSTOM_DL_DIR.get_or_init(|| Mutex::new(None))
    }

    pub fn set_custom_downloads_dir(dir: Option<String>) {
        let cleaned = dir
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Ok(mut guard) = Self::custom_dl_lock().lock() {
            *guard = cleaned;
        }
    }

    pub fn downloads_root_dir() -> PathBuf {
        if let Ok(guard) = Self::custom_dl_lock().lock() {
            if let Some(ref custom) = *guard {
                let p = PathBuf::from(custom);
                if fs::create_dir_all(&p).is_ok() {
                    return p;
                }
            }
        }
        let base = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("ShunnoBrowser");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// Helper tools (ffmpeg.exe, yt-dlp.exe) live in app data, so changing the
    /// downloads folder never "loses" them.
    pub fn ffmpeg_bin_dir() -> PathBuf {
        let dir = Self::shared_data_dir().join("bin");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn legacy_bin_dir() -> PathBuf {
        // Versions <= 2.9.6 stored tools under <Downloads>\ShunnoBrowser\bin
        Self::downloads_root_dir().join("bin")
    }

    fn find_tool(exe_name: &str, probe_arg: &str) -> Option<PathBuf> {
        let local_exe = Self::ffmpeg_bin_dir().join(exe_name);
        if local_exe.is_file() {
            return Some(local_exe);
        }
        let legacy = Self::legacy_bin_dir().join(exe_name);
        if legacy.is_file() {
            return Some(legacy);
        }
        let bare = exe_name.trim_end_matches(".exe");
        if let Ok(out) = hidden_command(bare).arg(probe_arg).output() {
            if out.status.success() {
                return Some(PathBuf::from(bare));
            }
        }
        None
    }

    pub fn find_ffmpeg() -> Option<PathBuf> {
        Self::find_tool("ffmpeg.exe", "-version")
    }

    pub fn find_ytdlp() -> Option<PathBuf> {
        Self::find_tool("yt-dlp.exe", "--version")
    }

    pub fn load_global_settings() -> GlobalSettings {
        let path = Self::app_data_dir().join("global_settings.json");
        let mut cfg: GlobalSettings = fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if cfg.search_engines.is_empty() {
            cfg.search_engines = GlobalSettings::default().search_engines;
        }
        if cfg.active_search_engine >= cfg.search_engines.len() {
            cfg.active_search_engine = 0;
        }
        if cfg.quick_shortcuts.is_empty() {
            cfg.quick_shortcuts = GlobalSettings::default().quick_shortcuts;
        }
        Self::set_custom_downloads_dir(cfg.custom_downloads_dir.clone());
        cfg
    }

    pub fn save_global_settings(settings: &GlobalSettings) {
        Self::set_custom_downloads_dir(settings.custom_downloads_dir.clone());
        let path = Self::app_data_dir().join("global_settings.json");
        if let Ok(json) = serde_json::to_string_pretty(settings) {
            let _ = fs::write(path, json);
        }
    }

    pub fn load_site_policies() -> HashMap<String, SitePolicy> {
        let path = Self::app_data_dir().join("site_policies.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_site_policies(policies: &HashMap<String, SitePolicy>) {
        let path = Self::app_data_dir().join("site_policies.json");
        if let Ok(json) = serde_json::to_string_pretty(policies) {
            let _ = fs::write(path, json);
        }
    }

    pub fn load_bookmarks() -> Vec<BookmarkEntry> {
        let path = Self::app_data_dir().join("bookmarks.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| {
                vec![
                    BookmarkEntry {
                        title: "DuckDuckGo Search".to_string(),
                        url: "https://duckduckgo.com".to_string(),
                        folder: "Search".to_string(),
                        created_at: 1,
                    },
                    BookmarkEntry {
                        title: "YouTube".to_string(),
                        url: "https://www.youtube.com".to_string(),
                        folder: "Media".to_string(),
                        created_at: 2,
                    },
                    BookmarkEntry {
                        title: "Internet Archive".to_string(),
                        url: "https://archive.org".to_string(),
                        folder: "Media".to_string(),
                        created_at: 3,
                    },
                ]
            })
    }

    pub fn save_bookmarks(bookmarks: &[BookmarkEntry]) {
        let path = Self::app_data_dir().join("bookmarks.json");
        if let Ok(json) = serde_json::to_string_pretty(bookmarks) {
            let _ = fs::write(path, json);
        }
    }

    pub fn export_portable_txt_bookmarks(bookmarks: &[BookmarkEntry]) -> PathBuf {
        let out_path = Self::downloads_root_dir().join("shunno_bookmarks.txt");
        let mut lines = Vec::with_capacity(bookmarks.len());
        for b in bookmarks {
            lines.push(format!("{} | {}", b.title.replace('|', "-"), b.url));
        }
        let _ = fs::write(&out_path, lines.join("\n"));
        out_path
    }

    pub fn import_portable_txt_bookmarks(existing: &mut Vec<BookmarkEntry>) -> usize {
        let in_path = Self::downloads_root_dir().join("shunno_bookmarks.txt");
        let Ok(content) = fs::read_to_string(in_path) else {
            return 0;
        };
        let mut added = 0;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (title, url) = if let Some((t, u)) = trimmed.split_once('|') {
                (t.trim().to_string(), u.trim().to_string())
            } else {
                (trimmed.to_string(), trimmed.to_string())
            };
            if url.starts_with("http") && !existing.iter().any(|b| b.url == url) {
                existing.insert(
                    0,
                    BookmarkEntry {
                        title,
                        url,
                        folder: "General".to_string(),
                        created_at: 0,
                    },
                );
                added += 1;
            }
        }
        if added > 0 {
            Self::save_bookmarks(existing);
        }
        added
    }

    pub fn load_history() -> Vec<HistoryEntry> {
        let path = Self::app_data_dir().join("history.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_history(history: &[HistoryEntry]) {
        let path = Self::app_data_dir().join("history.json");
        let capped: Vec<HistoryEntry> = history.iter().take(HISTORY_CAP).cloned().collect();
        if let Ok(json) = serde_json::to_string_pretty(&capped) {
            let _ = fs::write(path, json);
        }
    }

    pub fn load_session() -> Vec<SavedTab> {
        let path = Self::app_data_dir().join("session.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn load_spaces() -> SpacesFile {
        let path = Self::app_data_dir().join("spaces.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<SpacesFile>(&s).ok())
            .unwrap_or_default()
            .normalized()
    }

    pub fn save_spaces(sp: &SpacesFile) {
        let path = Self::app_data_dir().join("spaces.json");
        if let Ok(json) = serde_json::to_string_pretty(sp) {
            let _ = fs::write(path, json);
        }
    }

    pub fn load_tab_groups() -> Vec<TabGroup> {
        let path = Self::app_data_dir().join("tab_groups.json");
        fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save_tab_groups(groups: &[TabGroup]) {
        if let Ok(t) = serde_json::to_string_pretty(groups) {
            let _ = fs::write(Self::app_data_dir().join("tab_groups.json"), t);
        }
    }

    pub fn save_session(tabs: &[SavedTab]) {
        let path = Self::app_data_dir().join("session.json");
        if let Ok(json) = serde_json::to_string_pretty(tabs) {
            let _ = fs::write(path, json);
        }
    }
}

/// Inputs for one tab when deciding auto-archive.
pub struct ArchiveProbe {
    pub last_active_secs: u64,
    pub pinned: bool,
    pub playing_audio: bool,
    pub visible: bool,
    pub incognito: bool,
}

/// Indices of tabs to auto-archive, highest first (safe to remove in order).
/// Pinned, visible, audible and private tabs are never archived; at least one tab always stays.
pub fn tabs_to_archive(now: u64, hours: u32, tabs: &[ArchiveProbe]) -> Vec<usize> {
    if hours == 0 {
        return Vec::new();
    }
    let limit = hours as u64 * 3600;
    let mut out: Vec<usize> = tabs
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.pinned && !t.playing_audio && !t.visible && !t.incognito && now.saturating_sub(t.last_active_secs) >= limit)
        .map(|(i, _)| i)
        .collect();
    if out.len() >= tabs.len() {
        out.clear();
    }
    out.reverse();
    out
}

fn default_theme_mode() -> String {
    "system".to_string()
}

/// Whether the UI should be dark for a theme mode, given the Windows app mode.
pub fn effective_dark(theme_mode: &str, system_dark: bool) -> bool {
    match theme_mode {
        "dark" => true,
        "light" => false,
        _ => system_dark,
    }
}

/// WebView2 browser arguments. Every web view in a session must use the same string
/// (they share one environment), so this depends only on settings read at startup.
pub fn browser_args(force_dark_pages: bool) -> String {
    let mut a = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if force_dark_pages {
        // Chromium auto dark mode: perceptual (CIELAB) inversion, photos left alone.
        a.push_str(" --enable-features=WebContentsForceDark:inversion_method/cielab_based/image_behavior/selective_image_inversion");
    }
    a
}

fn default_reader_prefs() -> String {
    r#"{"size":19,"font":"serif","theme":"auto","width":"normal"}"#.to_string()
}

/// Reader preferences to store, or None if the JSON is not a small object.
pub fn valid_reader_prefs(json: &str) -> Option<String> {
    if json.len() > 300 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.as_object()?;
    Some(v.to_string())
}
