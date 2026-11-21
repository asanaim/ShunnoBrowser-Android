pub mod blocker;
pub mod downloader;
pub mod https_guard;
pub mod importer;
pub mod permissions;
pub mod sanitizer;
pub mod state;
pub mod updater;
pub mod userscripts;

use downloader::{EnqueueOutcome, NativeDownloadEngine};
use https_guard::Decision as HttpsDecision;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use serde::Serialize;
use state::{
    set_android_base_dir, set_profile, BrowserStorage, GlobalSettings, Profile,
};
use std::sync::{Mutex, OnceLock};
use tokio::runtime::Runtime;

struct CoreRuntime {
    rt: Runtime,
    engine: NativeDownloadEngine,
    settings: Mutex<GlobalSettings>,
}

static CORE: OnceLock<CoreRuntime> = OnceLock::new();

fn core() -> &'static CoreRuntime {
    CORE.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let settings = BrowserStorage::load_global_settings();
        let engine = NativeDownloadEngine::new(
            settings.speed_limit_bps,
            settings.auto_clean_completed,
            settings.max_concurrent_downloads,
        );
        CoreRuntime {
            rt,
            engine,
            settings: Mutex::new(settings),
        }
    })
}

fn get_str(env: &mut JNIEnv, s: JString) -> String {
    env.get_string(&s).map(|v| v.into()).unwrap_or_default()
}

fn new_jstr(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    base_dir: JString,
    downloads_dir: JString,
    profile_id: JString,
) {
    let base = get_str(&mut env, base_dir);
    let dl = get_str(&mut env, downloads_dir);
    let prof = get_str(&mut env, profile_id);
    if !base.is_empty() {
        set_android_base_dir(&base);
    }
    set_profile(if prof.is_empty() { "default" } else { &prof });
    if !dl.is_empty() {
        BrowserStorage::set_custom_downloads_dir(Some(dl));
    }

    let c = core();
    let https_on = c.settings.lock().map(|s| s.https_only).unwrap_or(true);
    https_guard::global().set_enabled(https_on);

    // Load cached adblock-rust engine off the UI thread and refresh EasyList/EasyPrivacy if stale
    c.rt.spawn(async {
        blocker::global().load_cache();
        if !blocker::global().has_lists() || blocker::global().is_stale() {
            refresh_filter_lists_async().await;
        }
    });
}

async fn refresh_filter_lists_async() {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let _ = blocker::global().refresh(&client).await;
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeShouldBlock(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    source_url: JString,
    req_type: JString,
) -> jboolean {
    let u = get_str(&mut env, url);
    let src = get_str(&mut env, source_url);
    let rtype = get_str(&mut env, req_type);
    if blocker::global().should_block(&u, &src, &rtype) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeBlockerStatus(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let st = blocker::global().status();
    new_jstr(&mut env, &st)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeCosmeticCss(
    mut env: JNIEnv,
    _class: JClass,
    page_url: JString,
) -> jstring {
    let u = get_str(&mut env, page_url);
    let css = blocker::global().cosmetic_css(&u, &[], &[]);
    new_jstr(&mut env, &css)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeSanitizeUrl(
    mut env: JNIEnv,
    _class: JClass,
    raw_input: JString,
) -> jstring {
    let raw = get_str(&mut env, raw_input);
    let out = core()
        .settings
        .lock()
        .map(|s| sanitizer::sanitize_url(&raw, &s))
        .unwrap_or(raw);
    new_jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeSearchUrl(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
) -> jstring {
    let q = get_str(&mut env, query);
    let out = core()
        .settings
        .lock()
        .map(|s| sanitizer::search_url(&q, &s))
        .unwrap_or(q);
    new_jstr(&mut env, &out)
}

#[derive(Serialize)]
struct HttpsDecisionDto {
    action: String,
    target: String,
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeHttpsDecide(
    mut env: JNIEnv,
    _class: JClass,
    tab_id: jlong,
    url: JString,
) -> jstring {
    let u = get_str(&mut env, url);
    let dto = match https_guard::global().decide(tab_id.max(0) as usize, &u) {
        HttpsDecision::Allow => HttpsDecisionDto {
            action: "ALLOW".into(),
            target: u,
        },
        HttpsDecision::Upgrade(https_url) => HttpsDecisionDto {
            action: "UPGRADE".into(),
            target: https_url,
        },
        HttpsDecision::Interstitial(http_url) => {
            let allow_link = https_guard::global()
                .issue_allow_link(&http_url)
                .unwrap_or_else(|| http_url.clone());
            let html = https_guard::interstitial_html(&http_url, &allow_link);
            HttpsDecisionDto {
                action: "INTERSTITIAL".into(),
                target: html,
            }
        }
    };
    let json = serde_json::to_string(&dto).unwrap_or_else(|_| r#"{"action":"ALLOW","target":""}"#.into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeHttpsNavigationDone(
    mut env: JNIEnv,
    _class: JClass,
    tab_id: jlong,
    success: jboolean,
    cancelled: jboolean,
) -> jstring {
    let res = https_guard::global().on_navigation_done(
        tab_id.max(0) as usize,
        success == JNI_TRUE,
        cancelled == JNI_TRUE,
    );
    let html = res
        .map(|http_url| {
            let allow_link = https_guard::global()
                .issue_allow_link(&http_url)
                .unwrap_or_else(|| http_url.clone());
            https_guard::interstitial_html(&http_url, &allow_link)
        })
        .unwrap_or_default();
    new_jstr(&mut env, &html)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeGetSettingsJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let json = core()
        .settings
        .lock()
        .ok()
        .and_then(|s| serde_json::to_string(&*s).ok())
        .unwrap_or_else(|| "{}".into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeSaveSettingsJson(
    mut env: JNIEnv,
    _class: JClass,
    json_str: JString,
) -> jboolean {
    let raw = get_str(&mut env, json_str);
    if let Ok(parsed) = serde_json::from_str::<GlobalSettings>(&raw) {
        BrowserStorage::save_global_settings(&parsed);
        https_guard::global().set_enabled(parsed.https_only);
        core()
            .engine
            .speed_limit_bps
            .store(parsed.speed_limit_bps, std::sync::atomic::Ordering::Relaxed);
        core()
            .engine
            .auto_clean_completed
            .store(parsed.auto_clean_completed, std::sync::atomic::Ordering::Relaxed);
        core()
            .engine
            .max_concurrent
            .store(parsed.max_concurrent_downloads.max(1), std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut guard) = core().settings.lock() {
            *guard = parsed;
        }
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeGetBookmarksJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let list = BrowserStorage::load_bookmarks();
    let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeSaveBookmarksJson(
    mut env: JNIEnv,
    _class: JClass,
    json_str: JString,
) {
    let raw = get_str(&mut env, json_str);
    if let Ok(list) = serde_json::from_str::<Vec<state::BookmarkEntry>>(&raw) {
        BrowserStorage::save_bookmarks(&list);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeGetHistoryJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let list = BrowserStorage::load_history();
    let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeRecordHistory(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    url: JString,
) {
    let t = get_str(&mut env, title);
    let u = get_str(&mut env, url);
    if !u.starts_with("http://") && !u.starts_with("https://") {
        return;
    }
    let mut history = BrowserStorage::load_history();
    history.retain(|h| h.url != u);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    history.insert(
        0,
        state::HistoryEntry {
            title: if t.trim().is_empty() { u.clone() } else { t },
            url: u,
            visited_at: now,
        },
    );
    BrowserStorage::save_history(&history);
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeClearHistory(
    _env: JNIEnv,
    _class: JClass,
) {
    BrowserStorage::save_history(&[]);
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeGetProfilesJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let list = BrowserStorage::load_profiles();
    let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeCreateProfile(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
    color: JString,
) -> jstring {
    let n = get_str(&mut env, name);
    let c = get_str(&mut env, color);
    let mut list = BrowserStorage::load_profiles();
    let id = state::new_profile_id(&n, &list);
    let prof = Profile {
        id: id.clone(),
        name: n.trim().chars().take(40).collect(),
        color: if state::PROFILE_COLORS.contains(&c.as_str()) {
            c
        } else {
            state::PROFILE_COLORS[0].to_string()
        },
    };
    list.push(prof);
    let clean = state::sanitize_profiles(list);
    BrowserStorage::save_profiles(&clean);
    new_jstr(&mut env, &id)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeEnqueueDownload(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    threads: jint,
    referer: JString,
    cookie: JString,
    dup_mode: JString,
    name_hint: JString,
) -> jstring {
    let u = get_str(&mut env, url);
    let r = get_str(&mut env, referer);
    let ck = get_str(&mut env, cookie);
    let dm = get_str(&mut env, dup_mode);
    let nh = get_str(&mut env, name_hint);
    let c = core();
    let outcome = c.engine.enqueue_named(
        c.rt.handle(),
        u,
        threads.clamp(1, 32) as usize,
        if r.is_empty() { None } else { Some(r) },
        if ck.is_empty() { None } else { Some(ck) },
        if dm.is_empty() { "CHECK" } else { &dm },
        None,
        if nh.is_empty() { None } else { Some(nh) },
    );
    let status = match outcome {
        EnqueueOutcome::Started => "STARTED",
        EnqueueOutcome::DuplicateDetected(_) => "DUPLICATE_DETECTED",
    };
    new_jstr(&mut env, status)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeGetDownloadsJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let c = core();
    c.engine.pump_scheduler(c.rt.handle());
    let json = c
        .engine
        .tasks
        .lock()
        .ok()
        .and_then(|list| serde_json::to_string(&*list).ok())
        .unwrap_or_else(|| "[]".into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativePauseDownload(
    _env: JNIEnv,
    _class: JClass,
    id: jlong,
) {
    core().engine.pause(id.max(0) as u64);
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeResumeDownload(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    new_url: JString,
) {
    let nu = get_str(&mut env, new_url);
    let c = core();
    c.engine.resume_or_restart(
        c.rt.handle(),
        id.max(0) as u64,
        if nu.is_empty() { None } else { Some(nu) },
    );
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeDeleteDownload(
    _env: JNIEnv,
    _class: JClass,
    id: jlong,
) {
    core().engine.delete(id.max(0) as u64);
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeClearFinishedDownloads(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    core().engine.clear_finished() as jint
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeListUserScriptsJson(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let hashes = core()
        .settings
        .lock()
        .map(|s| s.approved_script_hashes.clone())
        .unwrap_or_default();
    let list = userscripts::list_user_scripts(&hashes);
    let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    new_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeLoadApprovedUserScripts(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
) -> jstring {
    let h = get_str(&mut env, host);
    let hashes = core()
        .settings
        .lock()
        .map(|s| s.approved_script_hashes.clone())
        .unwrap_or_default();
    let js = userscripts::load_approved_user_scripts(&h, &hashes);
    new_jstr(&mut env, &js)
}

#[no_mangle]
pub extern "system" fn Java_com_shunnobrowser_app_core_RustCoreBridge_nativeApproveUserScript(
    mut env: JNIEnv,
    _class: JClass,
    sha256: JString,
    approve: jboolean,
) {
    let sha = get_str(&mut env, sha256);
    if let Ok(mut s) = core().settings.lock() {
        s.approved_script_hashes.retain(|x| x != &sha);
        if approve == JNI_TRUE && !sha.is_empty() {
            s.approved_script_hashes.push(sha);
        }
        BrowserStorage::save_global_settings(&s);
    }
}
