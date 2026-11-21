//! Import bookmarks and history from Chromium-family browsers (Chrome, Edge, Brave, Vivaldi, Opera).
//!
//! Bookmarks are the `Bookmarks` JSON file. History is the `History` SQLite database, read with a
//! minimal read-only B-tree reader below instead of linking SQLite (keeps the binary lean).
//! Every input is treated as untrusted: all reads are bounds-checked and cycles are rejected.
//! Saved passwords are not imported from profile files: Chromium 127+ protects them with
//! app-bound encryption, so the supported path is the browser's own CSV export.

use crate::state::{BookmarkEntry, HistoryEntry};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Microseconds between 1601-01-01 (Chromium/Windows epoch) and 1970-01-01.
const WEBKIT_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;
const MAX_BOOKMARKS: usize = 20_000;
const MAX_FOLDER_DEPTH: usize = 64;

#[derive(Clone, Debug, Serialize)]
pub struct ImportSource {
    pub browser: String,
    pub profile: String,
    #[serde(skip)]
    pub dir: PathBuf,
    pub has_bookmarks: bool,
    pub has_history: bool,
}

// ---------------------------------------------------------------- UI state

#[derive(Default)]
struct UiState {
    sources: Vec<ImportSource>,
    status: String,
}

fn ui() -> &'static Mutex<UiState> {
    static S: std::sync::OnceLock<Mutex<UiState>> = std::sync::OnceLock::new();
    S.get_or_init(|| Mutex::new(UiState::default()))
}

pub fn ui_snapshot() -> (Vec<ImportSource>, String) {
    let g = ui().lock().unwrap_or_else(|e| e.into_inner());
    (g.sources.clone(), g.status.clone())
}

pub fn set_status(s: impl Into<String>) {
    ui().lock().unwrap_or_else(|e| e.into_inner()).status = s.into();
}

pub fn scan() {
    let found = detect_sources();
    let mut g = ui().lock().unwrap_or_else(|e| e.into_inner());
    g.status = if found.is_empty() {
        "No Chrome, Edge, Brave, Vivaldi or Opera profiles found.".into()
    } else {
        String::new()
    };
    g.sources = found;
}

pub fn source_at(idx: usize) -> Option<ImportSource> {
    ui().lock().unwrap_or_else(|e| e.into_inner()).sources.get(idx).cloned()
}

// ---------------------------------------------------------------- discovery

fn user_data_roots() -> Vec<(&'static str, PathBuf, bool)> {
    // (browser, path, path_is_profile_itself)
    let mut v = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
    if let Some(l) = &local {
        v.push(("Chrome", l.join(r"Google\Chrome\User Data"), false));
        v.push(("Edge", l.join(r"Microsoft\Edge\User Data"), false));
        v.push(("Brave", l.join(r"BraveSoftware\Brave-Browser\User Data"), false));
        v.push(("Vivaldi", l.join(r"Vivaldi\User Data"), false));
    }
    if let Some(r) = &roaming {
        v.push(("Opera", r.join(r"Opera Software\Opera Stable"), true));
        v.push(("Opera GX", r.join(r"Opera Software\Opera GX Stable"), true));
    }
    v
}

/// Display names from `Local State` → profile.info_cache.<dir>.name.
fn profile_names(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(root.join("Local State")) else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
    if let Some(map) = v.pointer("/profile/info_cache").and_then(|x| x.as_object()) {
        for (dir, info) in map {
            if let Some(n) = info.get("name").and_then(|x| x.as_str()) {
                out.insert(dir.clone(), n.to_string());
            }
        }
    }
    out
}

pub fn detect_sources() -> Vec<ImportSource> {
    let mut out = Vec::new();
    for (browser, root, is_profile) in user_data_roots() {
        if !root.is_dir() {
            continue;
        }
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        if is_profile {
            dirs.push(("Default".into(), root.clone()));
        } else if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name == "Default" || name.starts_with("Profile ") {
                    dirs.push((name, e.path()));
                }
            }
        }
        dirs.sort();
        let names = profile_names(&root);
        for (dir_name, dir) in dirs {
            let has_bookmarks = dir.join("Bookmarks").is_file();
            let has_history = dir.join("History").is_file();
            if !has_bookmarks && !has_history {
                continue;
            }
            out.push(ImportSource {
                browser: browser.to_string(),
                profile: names.get(&dir_name).cloned().unwrap_or(dir_name),
                dir,
                has_bookmarks,
                has_history,
            });
        }
    }
    out
}

// ---------------------------------------------------------------- bookmarks

fn webkit_us_to_unix_s(us: i64) -> u64 {
    if us <= WEBKIT_EPOCH_OFFSET_US {
        return 0;
    }
    ((us - WEBKIT_EPOCH_OFFSET_US) / 1_000_000) as u64
}

fn is_importable_url(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && u.len() <= 8192
}

/// Parse a Chromium `Bookmarks` file. Folder labels are `<label>` for top-level items and
/// `<label>/<first-level folder>` below that, so the flat folder bar stays short.
pub fn parse_chromium_bookmarks(json: &str, label: &str) -> Result<Vec<BookmarkEntry>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("bookmarks: {e}"))?;
    let roots = v.get("roots").and_then(|r| r.as_object()).ok_or("bookmarks: no roots")?;
    let mut out = Vec::new();
    for key in ["bookmark_bar", "other", "synced"] {
        if let Some(node) = roots.get(key) {
            walk(node, label, None, 0, &mut out);
        }
    }
    Ok(out)
}

fn walk(node: &serde_json::Value, label: &str, top: Option<&str>, depth: usize, out: &mut Vec<BookmarkEntry>) {
    if depth > MAX_FOLDER_DEPTH || out.len() >= MAX_BOOKMARKS {
        return;
    }
    let Some(children) = node.get("children").and_then(|c| c.as_array()) else { return };
    for c in children {
        match c.get("type").and_then(|t| t.as_str()) {
            Some("url") => {
                let url = c.get("url").and_then(|u| u.as_str()).unwrap_or("");
                if !is_importable_url(url) {
                    continue;
                }
                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("").trim();
                let added = c
                    .get("date_added")
                    .and_then(|d| d.as_str())
                    .and_then(|d| d.parse::<i64>().ok())
                    .map(webkit_us_to_unix_s)
                    .unwrap_or(0);
                out.push(BookmarkEntry {
                    title: if name.is_empty() { url.to_string() } else { name.chars().take(300).collect() },
                    url: url.to_string(),
                    folder: match top {
                        Some(t) => format!("{label}/{t}"),
                        None => label.to_string(),
                    },
                    created_at: added,
                });
                if out.len() >= MAX_BOOKMARKS {
                    return;
                }
            }
            Some("folder") => {
                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("Folder").trim();
                let name: String = name.chars().filter(|ch| *ch != '/').take(60).collect();
                let next_top = top.map(str::to_string).unwrap_or(name);
                walk(c, label, Some(&next_top), depth + 1, out);
            }
            _ => {}
        }
    }
}

/// Merge, skipping URLs already present. Returns the number added.
pub fn merge_bookmarks(existing: &mut Vec<BookmarkEntry>, incoming: Vec<BookmarkEntry>) -> usize {
    let mut seen: HashSet<String> = existing.iter().map(|b| b.url.clone()).collect();
    let mut added = 0;
    for b in incoming {
        if seen.insert(b.url.clone()) {
            existing.push(b);
            added += 1;
        }
    }
    added
}

// ---------------------------------------------------------------- history

/// Read the `urls` table of a Chromium `History` database: most recent first, hidden rows skipped.
pub fn parse_chromium_history(db: &[u8], limit: usize) -> Result<Vec<HistoryEntry>, String> {
    let r = SqliteReader::new(db)?;
    let (root, sql) = r.table("urls")?;
    let cols = column_names(&sql);
    let idx = |n: &str| cols.iter().position(|c| c.eq_ignore_ascii_case(n));
    let (iu, it, iv) = (
        idx("url").ok_or("history: no url column")?,
        idx("title").ok_or("history: no title column")?,
        idx("last_visit_time").ok_or("history: no last_visit_time column")?,
    );
    let ih = idx("hidden");
    let mut rows: Vec<HistoryEntry> = Vec::new();
    r.scan_table(root, &mut |vals| {
        if let Some(h) = ih {
            if matches!(vals.get(h), Some(Val::Int(x)) if *x != 0) {
                return;
            }
        }
        let url = match vals.get(iu) { Some(Val::Text(s)) => s.clone(), _ => return };
        if !is_importable_url(&url) {
            return;
        }
        let title = match vals.get(it) { Some(Val::Text(s)) if !s.trim().is_empty() => s.chars().take(300).collect(), _ => url.clone() };
        let t = match vals.get(iv) { Some(Val::Int(x)) => webkit_us_to_unix_s(*x), _ => 0 };
        rows.push(HistoryEntry { title, url, visited_at: t });
    })?;
    rows.sort_by(|a, b| b.visited_at.cmp(&a.visited_at));
    rows.truncate(limit);
    Ok(rows)
}

/// Merge by URL (newest visit wins), keep newest first, cap the total.
pub fn merge_history(existing: &mut Vec<HistoryEntry>, incoming: Vec<HistoryEntry>, cap: usize) -> usize {
    let mut seen: HashSet<String> = existing.iter().map(|h| h.url.clone()).collect();
    let mut added = 0;
    for h in incoming {
        if seen.insert(h.url.clone()) {
            existing.push(h);
            added += 1;
        }
    }
    existing.sort_by(|a, b| b.visited_at.cmp(&a.visited_at));
    existing.truncate(cap);
    added
}

/// Column names from a `CREATE TABLE name(col type ..., ...)` statement (constraints skipped).
fn column_names(sql: &str) -> Vec<String> {
    let Some(open) = sql.find('(') else { return vec![] };
    let Some(close) = sql.rfind(')') else { return vec![] };
    if close <= open {
        return vec![];
    }
    let body = &sql[open + 1..close];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
        .into_iter()
        .filter_map(|p| {
            let w = p.split_whitespace().next()?.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']');
            let up = w.to_ascii_uppercase();
            if ["PRIMARY", "UNIQUE", "CHECK", "FOREIGN", "CONSTRAINT"].contains(&up.as_str()) {
                None
            } else {
                Some(w.to_string())
            }
        })
        .collect()
}

// ---------------------------------------------------------------- minimal SQLite reader

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob,
}

struct SqliteReader<'a> {
    db: &'a [u8],
    page_size: usize,
    usable: usize,
    pages: usize,
}

fn varint(b: &[u8], mut p: usize) -> Option<(i64, usize)> {
    let mut v: u64 = 0;
    for i in 0..9 {
        let byte = *b.get(p)?;
        p += 1;
        if i == 8 {
            v = (v << 8) | byte as u64;
            return Some((v as i64, p));
        }
        v = (v << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return Some((v as i64, p));
        }
    }
    None
}

fn be(b: &[u8], p: usize, n: usize) -> Option<u64> {
    let s = b.get(p..p + n)?;
    Some(s.iter().fold(0u64, |a, x| (a << 8) | *x as u64))
}

impl<'a> SqliteReader<'a> {
    fn new(db: &'a [u8]) -> Result<Self, String> {
        if db.len() < 100 || &db[..16] != b"SQLite format 3\0" {
            return Err("history: not an SQLite database".into());
        }
        let ps = be(db, 16, 2).unwrap() as usize;
        let page_size = if ps == 1 { 65536 } else { ps };
        if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() {
            return Err("history: bad page size".into());
        }
        let reserved = db[20] as usize;
        let usable = page_size.checked_sub(reserved).filter(|u| *u >= 480).ok_or("history: bad reserved size")?;
        Ok(Self { db, page_size, usable, pages: db.len() / page_size })
    }

    fn page(&self, n: u32) -> Option<&'a [u8]> {
        let n = n as usize;
        if n == 0 || n > self.pages {
            return None;
        }
        self.db.get((n - 1) * self.page_size..n * self.page_size)
    }

    fn table(&self, name: &str) -> Result<(u32, String), String> {
        let mut found = None;
        self.scan_table(1, &mut |v| {
            if found.is_some() {
                return;
            }
            if let (Some(Val::Text(t)), Some(Val::Text(n)), Some(Val::Int(root)), Some(Val::Text(sql))) =
                (v.first(), v.get(1), v.get(3), v.get(4))
            {
                if t == "table" && n.eq_ignore_ascii_case(name) {
                    found = Some((*root as u32, sql.clone()));
                }
            }
        })?;
        found.ok_or_else(|| format!("history: no {name} table"))
    }

    fn scan_table(&self, root: u32, f: &mut dyn FnMut(Vec<Val>)) -> Result<(), String> {
        let mut visited = HashSet::new();
        let mut stack = vec![root];
        while let Some(pn) = stack.pop() {
            if !visited.insert(pn) || visited.len() > self.pages {
                return Err("history: corrupt b-tree (cycle)".into());
            }
            let page = self.page(pn).ok_or("history: page out of range")?;
            let h = if pn == 1 { 100 } else { 0 };
            let kind = *page.get(h).ok_or("history: short page")?;
            let ncells = be(page, h + 3, 2).ok_or("history: short page")? as usize;
            match kind {
                0x05 => {
                    let right = be(page, h + 8, 4).ok_or("history: short page")? as u32;
                    let ptrs = h + 12;
                    // Push in reverse so children are visited left to right.
                    stack.push(right);
                    for i in (0..ncells).rev() {
                        let off = be(page, ptrs + 2 * i, 2).ok_or("history: bad cell pointer")? as usize;
                        let child = be(page, off, 4).ok_or("history: bad cell")? as u32;
                        stack.push(child);
                    }
                }
                0x0d => {
                    let ptrs = h + 8;
                    for i in 0..ncells {
                        let off = be(page, ptrs + 2 * i, 2).ok_or("history: bad cell pointer")? as usize;
                        if let Some(rec) = self.leaf_payload(page, off) {
                            if let Some(vals) = decode_record(&rec) {
                                f(vals);
                            }
                        }
                    }
                }
                _ => return Err("history: unexpected page type".into()),
            }
        }
        Ok(())
    }

    /// Payload of a table-leaf cell, following overflow pages.
    fn leaf_payload(&self, page: &[u8], off: usize) -> Option<Vec<u8>> {
        let (size, p) = varint(page, off)?;
        let (_rowid, p) = varint(page, p)?;
        let size = usize::try_from(size).ok()?;
        if size > 64 * 1024 * 1024 {
            return None;
        }
        let u = self.usable;
        let x = u - 35;
        let local = if size <= x {
            size
        } else {
            let m = ((u - 12) * 32 / 255) - 23;
            let k = m + (size - m) % (u - 4);
            if k <= x { k } else { m }
        };
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(page.get(p..p + local)?);
        if local < size {
            let mut next = be(page, p + local, 4)? as u32;
            let mut guard = 0usize;
            while out.len() < size {
                guard += 1;
                if next == 0 || guard > self.pages {
                    return None;
                }
                let op = self.page(next)?;
                next = be(op, 0, 4)? as u32;
                let take = (size - out.len()).min(u - 4);
                out.extend_from_slice(op.get(4..4 + take)?);
            }
        }
        Some(out)
    }
}

fn decode_record(r: &[u8]) -> Option<Vec<Val>> {
    let (hlen, mut p) = varint(r, 0)?;
    let hlen = usize::try_from(hlen).ok()?;
    if hlen > r.len() {
        return None;
    }
    let mut types = Vec::new();
    while p < hlen {
        let (t, np) = varint(r, p)?;
        types.push(t);
        p = np;
    }
    let mut d = hlen;
    let mut out = Vec::with_capacity(types.len());
    for t in types {
        let (val, n) = match t {
            0 => (Val::Null, 0),
            1 => (Val::Int(*r.get(d)? as i8 as i64), 1),
            2 => (Val::Int(be(r, d, 2)? as i16 as i64), 2),
            3 => {
                let v = be(r, d, 3)? as i64;
                (Val::Int((v << 40) >> 40), 3)
            }
            4 => (Val::Int(be(r, d, 4)? as i32 as i64), 4),
            5 => {
                let v = be(r, d, 6)? as i64;
                (Val::Int((v << 16) >> 16), 6)
            }
            6 => (Val::Int(be(r, d, 8)? as i64), 8),
            7 => (Val::Real(f64::from_bits(be(r, d, 8)?)), 8),
            8 => (Val::Int(0), 0),
            9 => (Val::Int(1), 0),
            t if t >= 12 && t % 2 == 0 => {
                let n = ((t - 12) / 2) as usize;
                r.get(d..d + n)?;
                (Val::Blob, n)
            }
            t if t >= 13 => {
                let n = ((t - 13) / 2) as usize;
                (Val::Text(String::from_utf8_lossy(r.get(d..d + n)?).into_owned()), n)
            }
            _ => return None,
        };
        out.push(val);
        d += n;
    }
    Some(out)
}

// ---------------------------------------------------------------- top level

/// Read a file that the owning browser may hold open; on failure, tell the user to close it.
fn read_shared(path: &Path, browser: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|_| format!("Could not read {browser} data. Close {browser} and try again."))
}

/// Read and parse on a worker thread; merging happens on the UI thread that owns the lists.
pub fn load(
    src: &ImportSource,
    want_bookmarks: bool,
    want_history: bool,
    history_cap: usize,
) -> Result<(Vec<BookmarkEntry>, Vec<HistoryEntry>), String> {
    let label = if src.profile == "Default" || src.profile.is_empty() {
        src.browser.clone()
    } else {
        format!("{} ({})", src.browser, src.profile.replace('/', "-"))
    };
    let mut b = Vec::new();
    let mut h = Vec::new();
    if want_bookmarks && src.has_bookmarks {
        let bytes = read_shared(&src.dir.join("Bookmarks"), &src.browser)?;
        b = parse_chromium_bookmarks(&String::from_utf8_lossy(&bytes), &label)?;
    }
    if want_history && src.has_history {
        let bytes = read_shared(&src.dir.join("History"), &src.browser)?;
        h = parse_chromium_history(&bytes, history_cap)?;
    }
    Ok((b, h))
}
