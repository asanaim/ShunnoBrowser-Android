use crate::state::GlobalSettings;
use url::Url;

pub fn resolve_amp_url(parsed: &Url) -> Option<String> {
    let host = parsed.host_str()?.to_ascii_lowercase();
    let path = parsed.path();

    // Google AMP viewer: https://www.google.com/amp/s/example.com/article
    if host.contains("google.") && path.starts_with("/amp/") {
        let rest = path.trim_start_matches("/amp/");
        let stripped = rest
            .strip_prefix("s/")
            .map(|s| format!("https://{}", s))
            .unwrap_or_else(|| format!("http://{}", rest));
        if Url::parse(&stripped).is_ok() {
            return Some(stripped);
        }
    }

    // AMP subdomain: https://amp.example.com/news -> https://example.com/news
    if let Some(root_domain) = host.strip_prefix("amp.") {
        if root_domain.contains('.') {
            let mut canonical = parsed.clone();
            if canonical.set_host(Some(root_domain)).is_ok() {
                return Some(canonical.to_string());
            }
        }
    }

    None
}

/// Search URL for `query` with the active search engine (always a search, even if `query` looks like a domain).
pub fn search_url(query: &str, global: &GlobalSettings) -> String {
    let encoded: String = query
        .trim()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect();
    let template = global
        .search_engines
        .get(global.active_search_engine)
        .map(|e| e.query_url.as_str())
        .unwrap_or("https://duckduckgo.com/?q=%s");
    if template.contains("%s") {
        template.replace("%s", &encoded)
    } else {
        format!("{}{}", template, encoded)
    }
}

/// Selected page text as a search query: whitespace collapsed, at most 300 characters.
pub fn selection_query(sel: &str) -> String {
    sel.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect()
}

pub fn sanitize_url(raw: &str, global: &GlobalSettings) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("shunno://newtab") || trimmed.eq_ignore_ascii_case("about:newtab") {
        return "shunno://newtab".to_string();
    }
    let normalized = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.contains('.') && !trimmed.contains(' ') {
        format!("https://{}", trimmed)
    } else {
        search_url(trimmed, global)
    };

    if !global.strip_tracking_params {
        return normalized;
    }

    let Ok(mut parsed) = Url::parse(&normalized) else {
        return normalized;
    };

    if let Some(amp_resolved) = resolve_amp_url(&parsed) {
        if let Ok(new_parsed) = Url::parse(&amp_resolved) {
            parsed = new_parsed;
        }
    }

    let tracking_params = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "fbclid",
        "gclid",
        "si",
        "igshid",
        "mc_cid",
        "mc_eid",
        "ref_src",
        "ref_url",
    ];

    let filtered: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !tracking_params.iter().any(|tp| k.eq_ignore_ascii_case(tp)))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if filtered.len() != parsed.query_pairs().count() {
        if filtered.is_empty() {
            parsed.set_query(None);
        } else {
            parsed.query_pairs_mut().clear().extend_pairs(filtered);
        }
    }

    parsed.to_string()
}

pub fn extract_host(url_str: &str) -> String {
    Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// True only for http(s) URLs whose host is a public name or public IP literal.
/// Used to stop web pages from making the native download engine reach
/// loopback, LAN, link-local or cloud-metadata targets (the engine bypasses CORS,
/// and probe results are reported back to the page).
pub fn is_public_http_url(url_str: &str) -> bool {
    let Ok(parsed) = Url::parse(url_str) else {
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            !(d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
                || d.ends_with(".lan")
                || d.ends_with(".home.arpa")
                || !d.contains('.'))
        }
        Some(url::Host::Ipv4(ip)) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || ip.octets()[0] == 0
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64)) // CGNAT 100.64/10
        }
        Some(url::Host::Ipv6(ip)) => {
            let seg0 = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (seg0 & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg0 & 0xffc0) == 0xfe80 // link local fe80::/10
                || ip.to_ipv4_mapped().map(|v4| v4.is_private() || v4.is_loopback() || v4.is_link_local()).unwrap_or(false))
        }
        None => false,
    }
}

/// Hosts and paths blocked by the built-in ad filter (replaced by full filter lists later).
pub fn is_blocked_ad_url(url_str: &str) -> bool {
    let Ok(u) = Url::parse(url_str) else {
        return false;
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let on = |d: &str| host == d || host.ends_with(&format!(".{}", d));
    const AD_HOSTS: [&str; 11] = [
        "doubleclick.net",
        "googlesyndication.com",
        "googleadservices.com",
        "adnxs.com",
        "adsrvr.org",
        "taboola.com",
        "outbrain.com",
        "popads.net",
        "popcash.net",
        "propellerads.com",
        "exoclick.com",
    ];
    if AD_HOSTS.iter().any(|d| on(d)) {
        return true;
    }
    let path = u.path();
    if on("youtube.com")
        && (path.starts_with("/pagead/")
            || path.starts_with("/api/stats/ads")
            || path.starts_with("/ptracking")
            || path.starts_with("/get_midroll_"))
    {
        return true;
    }
    if on("googlevideo.com") {
        if let Some(i) = url_str.find("/initplayback") {
            return url_str[i..].contains("oad");
        }
    }
    false
}
