//! User scripts and the optional unlock password.
//! Scripts live as plain .js files in %LOCALAPPDATA%\Lumen\scripts. A script runs only on the
//! sites named by its `// @match` lines (never "all sites"), only after the page has loaded,
//! and (by default) only after the user approves it for that site.
use crate::store;
use serde_json::{json, Map, Value};
use std::{
    collections::hash_map::RandomState,
    fs,
    hash::{BuildHasher, Hasher},
    path::PathBuf,
};

pub fn dir() -> PathBuf {
    let d = store::dir().join("scripts");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn clean_name(n: &str) -> Option<String> {
    let n = n.trim();
    if n.is_empty() || n.len() > 40 {
        return None;
    }
    n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ').then(|| n.to_string())
}

pub fn names() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()?.to_str()? == "js").then(|| p.file_stem()?.to_str().map(String::from))?
        })
        .filter(|n| clean_name(n).is_some())
        .collect();
    v.sort();
    v
}

pub fn read(name: &str) -> Option<String> {
    clean_name(name)?;
    fs::read_to_string(dir().join(format!("{name}.js"))).ok()
}

pub fn write(name: &str, code: &str) -> bool {
    clean_name(name).is_some() && code.len() <= 200_000 && fs::write(dir().join(format!("{name}.js")), code).is_ok()
}

pub fn delete(name: &str) {
    if clean_name(name).is_some() {
        let _ = fs::remove_file(dir().join(format!("{name}.js")));
    }
}

pub fn erase_all() {
    let _ = fs::remove_dir_all(dir());
    let _ = fs::remove_file(store::dir().join("scripts.json"));
}

pub fn load_meta() -> Map<String, Value> {
    store::load("scripts.json").as_object().cloned().unwrap_or_default()
}

pub fn save_meta(m: &Map<String, Value>) {
    store::save("scripts.json", &Value::Object(m.clone()));
}

// ---- @match handling -------------------------------------------------------

pub fn matches_of(code: &str) -> Vec<String> {
    code.lines()
        .take(40)
        .filter_map(|l| {
            let r = l.trim().strip_prefix("//")?.trim().strip_prefix("@match")?.trim();
            (!r.is_empty()).then(|| r.to_string())
        })
        .collect()
}

fn parse_pattern(p: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = p.split_once("://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase(), path.to_string()))
}

/// A pattern must name a real site: `example.com` or `*.example.com`. No bare `*`, no `*.com`.
pub fn pattern_ok(p: &str) -> bool {
    let Some((s, h, _)) = parse_pattern(p) else { return false };
    if !(s == "*" || s == "http" || s == "https") || h.is_empty() {
        return false;
    }
    match h.strip_prefix("*.") {
        Some(base) => !base.contains('*') && base.contains('.'),
        None => !h.contains('*'),
    }
}

fn glob(p: &str, s: &str) -> bool {
    let p: Vec<char> = p.chars().collect();
    let s: Vec<char> = s.chars().collect();
    let (mut i, mut j, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while j < s.len() {
        if i < p.len() && p[i] == s[j] {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            i += 1;
            mark = j;
        } else if let Some(st) = star {
            i = st + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

pub fn url_matches(pat: &str, url: &str) -> bool {
    let Some((ps, ph, pp)) = parse_pattern(pat) else { return false };
    let Some((us, rest)) = url.split_once("://") else { return false };
    let us = us.to_ascii_lowercase();
    if !((ps == "*" && (us == "http" || us == "https")) || ps == us) {
        return false;
    }
    let (hostport, path) = match rest.find(['/', '?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = hostport.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("").to_ascii_lowercase();
    let host_ok = match ph.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == ph,
    };
    if !host_ok {
        return false;
    }
    let mut path = path.split('#').next().unwrap_or("/").to_string();
    if path.starts_with('?') {
        path.insert(0, '/');
    }
    glob(&pp, &path)
}

// ---- optional unlock password (PBKDF2-HMAC-SHA256, 600k iterations, random salt) ----

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn derive(pw: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(pw.as_bytes(), salt, 600_000, &mut out);
    out
}

pub fn has_password() -> bool {
    store::dir().join("lock.json").exists()
}

pub fn set_password(pw: &str) -> bool {
    let mut salt = [0u8; 16];
    salt[..8].copy_from_slice(&RandomState::new().build_hasher().finish().to_le_bytes());
    salt[8..].copy_from_slice(&RandomState::new().build_hasher().finish().to_le_bytes());
    let h = derive(pw, &salt);
    store::save("lock.json", &json!({"salt": hex(&salt), "hash": hex(&h)}));
    has_password()
}

pub fn check_password(pw: &str) -> bool {
    let v = store::load("lock.json");
    let (Some(salt), Some(want)) = (
        v.get("salt").and_then(|x| x.as_str()).and_then(unhex),
        v.get("hash").and_then(|x| x.as_str()).and_then(unhex),
    ) else {
        return false;
    };
    let got = derive(pw, &salt);
    want.len() == got.len() && want.iter().zip(got.iter()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

pub fn clear_password() {
    let _ = fs::remove_file(store::dir().join("lock.json"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_must_name_a_site() {
        assert!(pattern_ok("https://example.com/*"));
        assert!(pattern_ok("*://*.example.com/*"));
        assert!(pattern_ok("http://localhost/*"));
        assert!(!pattern_ok("*://*/*"));
        assert!(!pattern_ok("https://*/*"));
        assert!(!pattern_ok("https://*.com/*"));
        assert!(!pattern_ok("https://ex*mple.com/*"));
        assert!(!pattern_ok("file:///etc/*"));
        assert!(!pattern_ok("example.com"));
    }

    #[test]
    fn url_matching() {
        assert!(url_matches("https://example.com/*", "https://example.com/a/b?x=1"));
        assert!(url_matches("*://*.example.com/*", "http://www.example.com/"));
        assert!(url_matches("*://*.example.com/*", "https://example.com"));
        assert!(!url_matches("https://example.com/*", "http://example.com/"));
        assert!(!url_matches("*://*.example.com/*", "https://evilexample.com/"));
        assert!(!url_matches("*://*.example.com/*", "https://example.com.evil.net/"));
        assert!(!url_matches("https://example.com/*", "https://user@evil.com:80/"));
        assert!(url_matches("https://example.com/docs/*", "https://example.com/docs/x"));
        assert!(!url_matches("https://example.com/docs/*", "https://example.com/other"));
    }

    #[test]
    fn names_and_headers() {
        assert!(clean_name("My script_1").is_some());
        assert!(clean_name("../evil").is_none());
        assert!(clean_name("a\\b").is_none());
        assert!(clean_name("").is_none());
        let m = matches_of("// @name x\n// @match https://a.com/*\n//@match https://b.com/*\nlet x;");
        assert_eq!(m, vec!["https://a.com/*", "https://b.com/*"]);
    }

    #[test]
    fn password_roundtrip() {
        assert!(set_password("correct horse"));
        assert!(check_password("correct horse"));
        assert!(!check_password("wrong"));
        assert!(!check_password(""));
        clear_password();
        assert!(!has_password());
    }
}
