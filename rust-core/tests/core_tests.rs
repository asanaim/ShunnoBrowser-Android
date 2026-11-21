use shunno_core::blocker::{build_engine, cosmetic_css_for};
use shunno_core::downloader::{
    detect_download_category, format_human_size, parse_content_disposition_filename,
    parse_content_range, parse_hls_playlist, sanitize_download_filename,
};
use shunno_core::https_guard::{Decision, HttpsGuard};
use shunno_core::sanitizer::{extract_host, sanitize_url, search_url, selection_query};
use shunno_core::state::{
    clean_group, group_after_drop, group_contiguous_order, new_profile_id, sanitize_profiles,
    GlobalSettings, Profile,
};
use shunno_core::userscripts::{compute_sha256, host_matches_pattern, parse_match_header};

#[test]
fn test_sanitizer_and_amp_stripping() {
    let g = GlobalSettings::default();
    assert_eq!(sanitize_url("", &g), "shunno://newtab");
    assert_eq!(
        sanitize_url("https://example.com/news?utm_source=x&fbclid=123&id=42", &g),
        "https://example.com/news?id=42"
    );
    assert_eq!(
        sanitize_url("https://www.google.com/amp/s/example.com/article", &g),
        "https://example.com/article"
    );
    assert!(search_url("rust jni android", &g).starts_with("https://duckduckgo.com/?q="));
    assert_eq!(selection_query("  hello   world \n "), "hello world");
    assert_eq!(extract_host("https://Sub.Example.ORG:8443/path"), "sub.example.org");
}

#[test]
fn test_https_guard_upgrade_and_loop_detection() {
    let guard = HttpsGuard::default();
    guard.set_enabled(true);
    assert_eq!(
        guard.decide(1, "http://example.com/page"),
        Decision::Upgrade("https://example.com/page".to_string())
    );
    // Immediate redirect back to http triggers loop interstitial
    assert_eq!(
        guard.decide(1, "http://example.com/page"),
        Decision::Interstitial("http://example.com/page".to_string())
    );
    // LAN IP is never upgraded
    assert_eq!(guard.decide(1, "http://192.168.1.1/admin"), Decision::Allow);
}

#[test]
fn test_adblock_engine_compilation_and_cosmetic_rules() {
    let lists = vec![
        "||doubleclick.net^\n||ads.example.com^\nexample.org##.ad-banner\n".to_string()
    ];
    let (engine, rules) = build_engine(&lists);
    assert_eq!(rules, 3);
    let css = cosmetic_css_for(&engine, "https://example.org/post", &[], &[]);
    assert!(css.contains(".ad-banner{display:none !important}"));
}

#[test]
fn test_downloader_helpers() {
    assert_eq!(sanitize_download_filename("../evil/CON.exe?token=1"), "_CON.exe");
    assert_eq!(detect_download_category("album.flac"), "Lossless Audio");
    assert_eq!(detect_download_category("movie.mp4"), "Videos");
    assert_eq!(format_human_size(1024 * 1024 * 5), "5.0 MB");
    assert_eq!(
        parse_content_disposition_filename("attachment; filename*=UTF-8''report%202026.pdf"),
        Some("report 2026.pdf".to_string())
    );
    assert_eq!(parse_content_range("bytes 0-1023/4096"), Some((0, 1023, Some(4096))));

    let base = url::Url::parse("https://cdn.example.com/live/master.m3u8").unwrap();
    let pl = parse_hls_playlist("#EXTM3U\n#EXTINF:6.0,\nseg01.ts\n#EXTINF:6.0,\nseg02.ts\n", &base);
    assert_eq!(pl.segments.len(), 2);
}

#[test]
fn test_tab_groups_and_profiles() {
    let (name, color) = clean_group("  Research  ", "#invalid");
    assert_eq!(name, "Research");
    assert_eq!(color, "#0A84FF");

    let order = group_contiguous_order(&[Some(1), None, Some(1), Some(2)]);
    assert_eq!(order, vec![0, 2, 1, 3]);
    assert_eq!(group_after_drop(&[Some(5), None, Some(5)], 1), Some(5));

    let profiles = sanitize_profiles(vec![Profile {
        id: "work".into(),
        name: "Work".into(),
        color: "#30D158".into(),
    }]);
    assert_eq!(profiles[0].id, "default");
    assert_eq!(new_profile_id("Work", &profiles), "work-2");
}

#[test]
fn test_userscripts_sha256_and_match() {
    let script = b"// @match example.com\nconsole.log('ok');";
    let hash = compute_sha256(script);
    assert_eq!(hash.len(), 64);
    assert_eq!(parse_match_header(std::str::from_utf8(script).unwrap()), "example.com");
    assert!(host_matches_pattern("sub.example.com", "example.com"));
    assert!(!host_matches_pattern("other.org", "example.com"));
}
