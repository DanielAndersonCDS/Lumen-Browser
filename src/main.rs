//! Lumen: a very light, bright, single-window browser. WebView2 on Windows 11.
//! Design rules: one idle event loop (zero CPU), lazy tab webviews, sleeping
//! background tabs, no favicons, no extensions, no background services.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod scripts;
mod store;

use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
    borrow::Cow,
    collections::{hash_map::RandomState, HashMap},
    hash::{BuildHasher, Hasher},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Theme, Window, WindowBuilder},
};
use wry::{
    dpi::{LogicalPosition, LogicalSize as LS},
    http::Response,
    NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse, Rect, WebContext, WebView, WebViewBuilder,
};

// ---- embedded assets (the whole browser is one .exe) -----------------------
static UI_HTML: &str = include_str!("../assets/ui.html");
static F_REG: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.woff2");
static F_MED: &[u8] = include_bytes!("../assets/JetBrainsMono-Medium.woff2");
static F_BOLD: &[u8] = include_bytes!("../assets/JetBrainsMono-Bold.woff2");
static KEYS_JS: &str = include_str!("../assets/keys.js");
static HIDE_JS: &str = include_str!("../assets/hide.js");
static ADBLOCK_JS: &str = include_str!("../assets/adblock.js");
static READER_JS: &str = include_str!("../assets/reader.js");
static PICKER_JS: &str = include_str!("../assets/picker.js");
static HOSTS: &str = include_str!("../assets/hosts.txt");
static ICON_PNG: &[u8] = include_bytes!("../assets/icon-96.png");
static ICON_RGBA: &[u8] = include_bytes!("../assets/icon-64.rgba");

#[cfg(windows)]
const UI_ORIGIN: &str = "http://lumen.localhost/";
#[cfg(not(windows))]
const UI_ORIGIN: &str = "lumen://localhost/";
#[cfg(windows)]
const UI_URL: &str = "http://lumen.localhost/ui.html";
#[cfg(not(windows))]
const UI_URL_NIX: &str = "lumen://localhost/ui.html";

// Strict policy for the browser's own UI page: nothing external, no network.
const UI_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src 'self'; connect-src 'none'; img-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

// Files that run code when opened. Blocked from auto-saving unless settings.json allows it.
const RISKY_EXT: &[&str] = &[
    "exe", "msi", "msix", "msixbundle", "appx", "bat", "cmd", "com", "scr", "pif", "ps1", "psm1", "vbs", "vbe",
    "js", "jse", "wsf", "wsh", "hta", "lnk", "jar", "reg", "dll", "cpl", "msp", "gadget", "inf", "url", "vhd", "vhdx",
];

const TAB_KEYS: &[&str] = &[
    "address", "new", "private", "close", "reopen", "next", "prev", "bookmark", "history", "bookmarks", "fold",
    "reader", "hide", "unhide", "settings", "tab1", "tab2", "tab3", "tab4", "tab5", "tab6", "tab7", "tab8", "tab9",
];

// Same flags for every webview (WebView2 requires that when sharing a profile).
// Low-end-device mode is NOT on by default: it caps the JS heap at about 256 MB and slows heavy sites.
static LOW_MEM: AtomicBool = AtomicBool::new(false);
// 0 = light, 1 = dark, 2 = follow system
static THEME: AtomicU8 = AtomicU8::new(0);
// Custom UI colors as JSON ("null" when none), so the UI page can paint them from its first frame.
static COLORS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

const COLOR_KEYS: [&str; 4] = ["accent", "bg", "side", "text"];

fn lum(h: &str) -> Option<f64> {
    let h = h.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| -> Option<f64> {
        let v = u8::from_str_radix(h.get(i..i + 2)?, 16).ok()? as f64 / 255.0;
        Some(if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) })
    };
    Some(0.2126 * c(0)? + 0.7152 * c(2)? + 0.0722 * c(4)?)
}

fn contrast(a: &str, b: &str) -> Option<f64> {
    let (x, y) = (lum(a)?, lum(b)?);
    Some((x.max(y) + 0.05) / (x.min(y) + 0.05))
}

/// Accepts a full set of four #rrggbb colors that stay readable. Returns None otherwise.
fn valid_colors(v: &Value) -> Option<Value> {
    let o = v.as_object()?;
    let mut out = serde_json::Map::new();
    for k in COLOR_KEYS {
        let c = o.get(k)?.as_str()?.to_ascii_lowercase();
        lum(&c)?;
        out.insert(k.to_string(), json!(c));
    }
    let g = |k: &str| out[k].as_str().unwrap_or("#000000").to_string();
    let (a, b, sd, t) = (g("accent"), g("bg"), g("side"), g("text"));
    (contrast(&t, &b)? >= 4.0 && contrast(&t, &sd)? >= 3.5 && contrast(&a, &b)? >= 2.5).then(|| Value::Object(out))
}

#[cfg(windows)]
fn wv2_args() -> String {
    let mut a = String::from("--disable-features=msWebOOUI,msPdfOOUI,msEdgeShoppingUi --no-pings");
    if LOW_MEM.load(Ordering::Relaxed) {
        a.push_str(" --enable-low-end-device-mode");
    }
    a
}

fn theme_id(name: &str) -> u8 {
    match name {
        "dark" => 1,
        "system" => 2,
        _ => 0,
    }
}

fn theme_name() -> &'static str {
    match THEME.load(Ordering::Relaxed) {
        1 => "dark",
        2 => "system",
        _ => "light",
    }
}

fn bg_color() -> (u8, u8, u8, u8) {
    if THEME.load(Ordering::Relaxed) == 1 {
        (20, 22, 27, 255)
    } else {
        (255, 255, 255, 255)
    }
}

const SIDEBAR_W: f64 = 240.0;

enum Ev {
    Ui(Value),
    Tab(u64, Value),
    Title(u64, String),
    Load(u64, bool, String),
    Open(String),
    Toast(String),
    Tick,
}

struct Tab {
    id: u64,
    url: String,
    title: String,
    pinned: bool,
    private: bool,
    loading: bool,
    used: Instant,
    wv: Option<WebView>,
}

struct App {
    window: Window,
    ui: WebView,
    ctx: WebContext,
    proxy: EventLoopProxy<Ev>,
    tabs: Vec<Tab>,
    active: u64,
    next: u64,
    closed: Vec<(String, String)>,
    bm: Vec<(String, String)>,
    hist: Vec<(String, String)>,
    hidden: HashMap<String, Vec<String>>,
    search: String,
    sleep_after: Duration,
    adblock: Option<String>,
    sleep_min: u64,
    sw: f64,
    downloads: PathBuf,
    dirty_session: bool,
    dirty_hist: bool,
    nonce: String,
    picking: Option<u64>,
    popups: Vec<Instant>,
    allow_risky: bool,
    scripts_on: bool,
    script_prompt: bool,
    pw_set: bool,
    unlocked_until: Option<Instant>,
    fails: u32,
    lockout_until: Option<Instant>,
    pending: Vec<(u64, String, String)>,
    editor: bool,
    colors: Value,
}

fn builder<'a>(ctx: &'a mut WebContext) -> WebViewBuilder<'a> {
    let b = WebViewBuilder::new_with_web_context(ctx);
    #[cfg(windows)]
    let b = {
        use wry::WebViewBuilderExtWindows;
        let t = match THEME.load(Ordering::Relaxed) {
            1 => wry::Theme::Dark,
            2 => wry::Theme::Auto,
            _ => wry::Theme::Light,
        };
        b.with_additional_browser_args(wv2_args()).with_theme(t)
    };
    b
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LS::new(w.max(1.0), h.max(1.0)).into(),
    }
}

fn urlencode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{:02X}", b)),
        }
    }
    o
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").split(':').next().unwrap_or("").to_string()
}

fn pairs(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| Some((e.get(0)?.as_str()?.to_string(), e.get(1)?.as_str().unwrap_or("").to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Hidden tabs ask WebView2 to trim their memory (Windows only).
fn set_mem(v: &WebView, low: bool) {
    #[cfg(windows)]
    {
        use wry::{MemoryUsageLevel, WebViewExtWindows};
        let _ = v.set_memory_usage_level(if low { MemoryUsageLevel::Low } else { MemoryUsageLevel::Normal });
    }
    #[cfg(not(windows))]
    {
        let _ = (v, low);
    }
}

fn adblock_script() -> String {
    let hosts: Vec<&str> = HOSTS.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    ADBLOCK_JS.replace("__HOSTS__", &serde_json::to_string(&hosts).unwrap_or_else(|_| "[]".into()))
}

fn window_theme() -> Option<Theme> {
    match THEME.load(Ordering::Relaxed) {
        1 => Some(Theme::Dark),
        2 => None,
        _ => Some(Theme::Light),
    }
}

fn ui_url() -> &'static str {
    #[cfg(windows)]
    {
        UI_URL
    }
    #[cfg(not(windows))]
    {
        UI_URL_NIX
    }
}

fn safe_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

fn nav_allowed(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l == "about:blank" || l.starts_with("blob:")
}

fn safe_selector(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && !s.contains(['{', '}', ';', '<', '\\', '@', '\n'])
}

fn unique_path(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let base = Path::new(name);
    let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ext = base.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1..1000 {
        let c = dir.join(format!("{stem} ({n}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    p
}

fn random_nonce() -> String {
    let a = RandomState::new().build_hasher().finish();
    let b = RandomState::new().build_hasher().finish();
    format!("{a:016x}{b:016x}")
}

fn make_view(
    window: &Window,
    ctx: &mut WebContext,
    proxy: &EventLoopProxy<Ev>,
    id: u64,
    url: &str,
    private: bool,
    init: String,
    bounds: Rect,
    dl: PathBuf,
    allow_risky: bool,
) -> Option<WebView> {
    let (p_ipc, p_title, p_load, p_new, p_dl, p_dlb) =
        (proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone());
    builder(ctx)
        .with_url(url)
        .with_bounds(bounds)
        .with_initialization_script(init)
        .with_incognito(private)
        .with_clipboard(true)
        .with_hotkeys_zoom(true)
        .with_back_forward_navigation_gestures(true)
        .with_background_color(bg_color())
        .with_devtools(cfg!(debug_assertions))
        .with_ipc_handler(move |req| {
            if let Ok(v) = serde_json::from_str::<Value>(req.body()) {
                let _ = p_ipc.send_event(Ev::Tab(id, v));
            }
        })
        .with_navigation_handler(|u| nav_allowed(&u))
        .with_permission_handler(|k| match k {
            // Powerful APIs almost no site needs: refuse outright. The rest keep WebView2's own per-site prompt.
            PermissionKind::Midi | PermissionKind::Sensors | PermissionKind::LocalFonts | PermissionKind::WindowManagement => {
                PermissionResponse::Deny
            }
            _ => PermissionResponse::Default,
        })
        .with_document_title_changed_handler(move |t| {
            let _ = p_title.send_event(Ev::Title(id, t));
        })
        .with_on_page_load_handler(move |e, u| {
            let _ = p_load.send_event(Ev::Load(id, matches!(e, PageLoadEvent::Started), u));
        })
        .with_new_window_req_handler(move |u, _| {
            let _ = p_new.send_event(Ev::Open(u));
            NewWindowResponse::Deny
        })
        .with_download_started_handler(move |_url, path| {
            let Some(name) = path.file_name().map(|n| n.to_owned()) else { return false };
            let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if !allow_risky && RISKY_EXT.contains(&ext.as_str()) {
                let _ = p_dlb.send_event(Ev::Toast(format!("Blocked .{ext} download. Set allow_risky_downloads in settings.json to permit")));
                return false;
            }
            *path = unique_path(&dl, &name);
            true
        })
        .with_download_completed_handler(move |_url, path, ok| {
            let name = path
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "file".into());
            let _ = p_dl.send_event(Ev::Toast(if ok { format!("Saved {name} to Downloads") } else { "Download failed".into() }));
        })
        .build_as_child(window)
        .ok()
}

impl App {
    fn size(&self) -> (f64, f64) {
        let s = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        (s.width, s.height)
    }

    fn content_rect(&self) -> Rect {
        let (w, h) = self.size();
        rect(self.sw, 0.0, w - self.sw, h)
    }

    fn layout(&self) {
        let (w, h) = self.size();
        let page = self.tabs.iter().find(|t| t.id == self.active).map(|t| t.wv.is_some()).unwrap_or(false) && !self.editor;
        let ui_rect = if !page {
            rect(0.0, 0.0, w, h)
        } else if self.sw > 0.0 {
            rect(0.0, 0.0, self.sw, h)
        } else {
            rect(0.0, 0.0, 1.0, 1.0)
        };
        let _ = self.ui.set_bounds(ui_rect);
        if let Some(t) = self.tabs.iter().find(|t| t.id == self.active) {
            if let Some(v) = &t.wv {
                let _ = v.set_bounds(self.content_rect());
                let _ = v.set_visible(!self.editor);
            }
        }
    }

    fn unlocked(&self) -> bool {
        self.unlocked_until.map(|t| Instant::now() < t).unwrap_or(false)
    }

    /// True when a password is set and hasn't been entered in the last 10 minutes.
    fn locked(&self) -> bool {
        self.pw_set && !self.unlocked()
    }

    fn deny_locked(&self) -> bool {
        if self.locked() {
            self.eval_ui("toast('Locked. Enter your password first')");
            true
        } else {
            false
        }
    }

    fn apply_theme(&mut self, name: &str) {
        THEME.store(theme_id(name), Ordering::Relaxed);
        self.window.set_theme(window_theme());
        let bg = bg_color();
        let mut all: Vec<&WebView> = self.tabs.iter().filter_map(|t| t.wv.as_ref()).collect();
        all.push(&self.ui);
        for v in all {
            let _ = v.set_background_color(bg);
            #[cfg(windows)]
            {
                use wry::WebViewExtWindows;
                let t = match THEME.load(Ordering::Relaxed) {
                    1 => wry::Theme::Dark,
                    2 => wry::Theme::Auto,
                    _ => wry::Theme::Light,
                };
                let _ = v.set_theme(t);
            }
        }
    }

    // ---- scripts ----------------------------------------------------------

    fn exec_script(&self, tab_id: u64, name: &str) {
        let Some(code) = scripts::read(name) else { return };
        let Some(v) = self.tabs.iter().find(|t| t.id == tab_id).and_then(|t| t.wv.as_ref()) else { return };
        let n = json!(name);
        let _ = v.evaluate_script(&format!("(()=>{{try{{\n{code}\n}}catch(e){{console.error('[Lumen script]',{n},e)}}}})();"));
    }

    fn run_scripts(&mut self, id: u64, url: &str) {
        if !self.scripts_on || !safe_url(url) {
            return;
        }
        let host = host_of(url).to_ascii_lowercase();
        let meta = scripts::load_meta();
        for name in scripts::names() {
            let entry = meta.get(&name);
            if entry.and_then(|m| m.get("enabled")).and_then(|e| e.as_bool()) != Some(true) {
                continue;
            }
            let Some(code) = scripts::read(&name) else { continue };
            if !scripts::matches_of(&code).iter().any(|p| scripts::pattern_ok(p) && scripts::url_matches(p, url)) {
                continue;
            }
            match entry.and_then(|m| m.get("sites")).and_then(|x| x.get(&host)).and_then(|x| x.as_str()) {
                Some("allow") => self.exec_script(id, &name),
                Some("block") => {}
                _ => {
                    if self.script_prompt {
                        let key = (id, name.clone(), host.clone());
                        if !self.pending.contains(&key) {
                            self.pending.push(key);
                        }
                    } else {
                        self.exec_script(id, &name);
                    }
                }
            }
        }
    }

    fn set_consent(&self, name: &str, host: &str, state: &str) {
        let mut meta = scripts::load_meta();
        let e = meta.entry(name.to_string()).or_insert_with(|| json!({"enabled": true, "sites": {}}));
        if let Some(o) = e.as_object_mut() {
            let sites = o.entry("sites").or_insert_with(|| json!({}));
            if let Some(m) = sites.as_object_mut() {
                if state.is_empty() {
                    m.remove(host);
                } else {
                    m.insert(host.to_string(), json!(state));
                }
            }
        }
        scripts::save_meta(&meta);
    }

    fn push_scripts(&self) {
        let meta = scripts::load_meta();
        let list: Vec<Value> = scripts::names()
            .into_iter()
            .map(|n| {
                let m = meta.get(&n);
                let enabled = m.and_then(|m| m.get("enabled")).and_then(|e| e.as_bool()).unwrap_or(false);
                let sites: Vec<Value> = m
                    .and_then(|m| m.get("sites"))
                    .and_then(|x| x.as_object())
                    .map(|o| o.iter().map(|(h, st)| json!([h, st])).collect())
                    .unwrap_or_default();
                let matches = scripts::read(&n).map(|c| scripts::matches_of(&c)).unwrap_or_default();
                json!({"name": n, "enabled": enabled, "sites": sites, "matches": matches})
            })
            .collect();
        self.eval_ui(&format!("S.scripts={};render()", json!(list)));
    }

    fn unlock(&mut self, pw: &str) {
        if let Some(t) = self.lockout_until {
            if Instant::now() < t {
                self.eval_ui("toast('Too many tries. Wait 30 seconds')");
                return;
            }
            self.lockout_until = None;
            self.fails = 0;
        }
        if scripts::check_password(pw) {
            self.fails = 0;
            self.unlocked_until = Some(Instant::now() + Duration::from_secs(600));
            self.eval_ui("toast('Unlocked for 10 minutes')");
        } else {
            self.fails += 1;
            if self.fails >= 5 {
                self.lockout_until = Some(Instant::now() + Duration::from_secs(30));
            }
            self.eval_ui("toast('Wrong password')");
        }
        self.push();
    }

    fn script_cmd(&mut self, cmd: &str, v: &Value) {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        if self.deny_locked() {
            return;
        }
        match cmd {
            "script-list" => self.push_scripts(),
            "script-new" => {
                self.editor = true;
                self.layout();
                self.eval_ui("openEditor('', null)");
            }
            "script-open" => {
                if let Some(code) = scripts::read(&s("name")) {
                    self.editor = true;
                    self.layout();
                    self.eval_ui(&format!("openEditor({},{})", json!(s("name")), json!(code)));
                }
            }
            "script-save" => {
                let name = s("name");
                let code = s("code");
                let pats = scripts::matches_of(&code);
                if scripts::clean_name(&name).is_none() {
                    self.eval_ui("toast('Name: letters, numbers, space, - and _ (max 40)')");
                } else if pats.is_empty() || !pats.iter().all(|p| scripts::pattern_ok(p)) {
                    self.eval_ui("toast('Add // @match lines naming sites, e.g. https://*.example.com/*. No all-sites patterns')");
                } else if !scripts::write(&name, &code) {
                    self.eval_ui("toast('Could not save (max 200 KB)')");
                } else {
                    let mut meta = scripts::load_meta();
                    meta.entry(name.clone()).or_insert_with(|| json!({"enabled": true, "sites": {}}));
                    scripts::save_meta(&meta);
                    self.eval_ui("toast('Saved')");
                    self.push_scripts();
                }
            }
            "script-delete" => {
                let name = s("name");
                scripts::delete(&name);
                let mut meta = scripts::load_meta();
                meta.remove(&name);
                scripts::save_meta(&meta);
                self.push_scripts();
            }
            "script-toggle" => {
                let name = s("name");
                let on = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
                let mut meta = scripts::load_meta();
                let e = meta.entry(name).or_insert_with(|| json!({"enabled": false, "sites": {}}));
                if let Some(o) = e.as_object_mut() {
                    o.insert("enabled".into(), json!(on));
                }
                scripts::save_meta(&meta);
                self.push_scripts();
            }
            "script-forget" => {
                self.set_consent(&s("name"), &s("host"), "");
                self.push_scripts();
            }
            _ => {}
        }
    }

    fn script_answer(&mut self, v: &Value) {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let (name, host, choice) = (s("name"), s("host"), s("choice"));
        let id = self.active;
        let Some(i) = self.pending.iter().position(|p| p.0 == id && p.1 == name && p.2 == host) else { return };
        self.pending.remove(i);
        let same_site = host_of(&self.active_url()).eq_ignore_ascii_case(&host);
        match choice.as_str() {
            "block" => self.set_consent(&name, &host, "block"),
            "always" => {
                self.set_consent(&name, &host, "allow");
                if same_site {
                    self.exec_script(id, &name);
                }
            }
            "once" => {
                if same_site {
                    self.exec_script(id, &name);
                }
            }
            _ => {}
        }
        self.push();
    }

    fn tab(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn init_script(&self) -> String {
        let hidden = serde_json::to_string(&self.hidden).unwrap_or_else(|_| "{}".into());
        let mut s = KEYS_JS.replace("__NONCE__", &self.nonce);
        s.push('\n');
        s.push_str(&HIDE_JS.replace("__HIDE__", &hidden));
        if let Some(a) = &self.adblock {
            s.push('\n');
            s.push_str(a);
        }
        s
    }

    fn ensure(&mut self, id: u64) {
        let (url, private, has) = match self.tabs.iter().find(|t| t.id == id) {
            Some(t) => (t.url.clone(), t.private, t.wv.is_some()),
            None => return,
        };
        if has || url.is_empty() || !safe_url(&url) {
            return;
        }
        let init = self.init_script();
        let bounds = self.content_rect();
        let wv = make_view(&self.window, &mut self.ctx, &self.proxy, id, &url, private, init, bounds, self.downloads.clone(), self.allow_risky);
        if let Some(t) = self.tab(id) {
            t.loading = wv.is_some();
            t.wv = wv;
        }
    }

    fn eval_ui(&self, js: &str) {
        let _ = self.ui.evaluate_script(js);
    }

    fn focus_address(&mut self) {
        if self.sw == 0.0 {
            self.sw = SIDEBAR_W;
            self.layout();
            self.push();
        }
        let _ = self.ui.focus();
        self.eval_ui("focusAddr()");
    }

    fn select(&mut self, id: u64) {
        if !self.tabs.iter().any(|t| t.id == id) {
            return;
        }
        self.active = id;
        self.picking = None;
        self.editor = false;
        self.ensure(id);
        for t in &self.tabs {
            if let Some(v) = &t.wv {
                let _ = v.set_visible(t.id == id);
                set_mem(v, t.id != id);
            }
        }
        self.layout();
        let mut blank = false;
        if let Some(t) = self.tab(id) {
            t.used = Instant::now();
            match &t.wv {
                Some(v) => {
                    let _ = v.focus();
                }
                None => blank = t.url.is_empty(),
            }
        }
        if blank {
            self.focus_address();
        }
        self.dirty_session = true;
        self.update_title();
        self.push();
    }

    fn new_tab(&mut self, url: &str, private: bool) {
        if !url.is_empty() && !safe_url(url) {
            return;
        }
        let id = self.next;
        self.next += 1;
        self.tabs.push(Tab { id, url: url.into(), title: String::new(), pinned: false, private, loading: false, used: Instant::now(), wv: None });
        self.select(id);
    }

    fn close(&mut self, id: u64) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return };
        let t = self.tabs.remove(i);
        if !t.private && !t.url.is_empty() {
            self.closed.push((t.url, t.title));
            if self.closed.len() > 20 {
                self.closed.remove(0);
            }
        }
        drop(t.wv);
        self.pending.retain(|p| p.0 != id);
        if self.tabs.is_empty() {
            self.new_tab("", false);
        } else if self.active == id {
            let n = self.tabs[i.min(self.tabs.len() - 1)].id;
            self.select(n);
        } else {
            self.dirty_session = true;
            self.push();
        }
    }

    fn resolve(&self, text: &str) -> String {
        let t = text.trim();
        if t.is_empty() {
            return String::new();
        }
        if safe_url(t) {
            return t.into();
        }
        let l = t.to_ascii_lowercase();
        let blocked = ["javascript:", "data:", "file:", "about:", "blob:", "vbscript:", "ms-", "edge:", "chrome:"];
        if t.contains("://") || blocked.iter().any(|b| l.starts_with(b)) {
            self.eval_ui("toast('Only http and https addresses are allowed')");
            return String::new();
        }
        if !t.contains(' ') && (t.contains('.') || t.starts_with("localhost")) {
            let local = t.starts_with("localhost") || t.starts_with("127.") || t.starts_with("192.168.") || t.starts_with("10.");
            return format!("{}://{}", if local { "http" } else { "https" }, t);
        }
        self.search.replace("%s", &urlencode(t))
    }

    fn go(&mut self, text: &str) {
        let url = self.resolve(text);
        if url.is_empty() {
            return;
        }
        self.go_url(&url);
    }

    fn go_url(&mut self, url: &str) {
        if !safe_url(url) {
            return;
        }
        if !self.tabs.iter().any(|t| t.id == self.active) {
            self.new_tab(url, false);
            return;
        }
        let id = self.active;
        let mut load = None;
        if let Some(t) = self.tab(id) {
            t.url = url.into();
            t.title.clear();
            t.loading = true;
            load = Some(t.wv.is_some());
        }
        if load == Some(true) {
            let _ = self.tabs.iter().find(|t| t.id == id).and_then(|t| t.wv.as_ref()).map(|v| v.load_url(url));
        }
        self.select(id);
    }

    fn settings_json(&self) -> Value {
        json!({"search": self.search, "sleep_minutes": self.sleep_min, "adblock": self.adblock.is_some(), "allow_risky_downloads": self.allow_risky, "theme": theme_name(), "low_memory": LOW_MEM.load(Ordering::Relaxed), "scripts_enabled": self.scripts_on, "script_prompt": self.script_prompt, "colors": self.colors})
    }

    fn save_settings(&self) {
        store::save("settings.json", &self.settings_json());
    }

    /// Drop every webview and rebuild the active one so new settings take effect.
    fn rebuild_views(&mut self) {
        for t in self.tabs.iter_mut() {
            t.wv = None;
            t.loading = false;
        }
        let id = self.active;
        self.select(id);
    }

    fn set_option(&mut self, key: &str, val: Option<&Value>) {
        let mut rebuild = false;
        match key {
            "adblock" => {
                if let Some(b) = val.and_then(|x| x.as_bool()) {
                    self.adblock = b.then(adblock_script);
                    rebuild = true;
                }
            }
            "risky" => {
                if let Some(b) = val.and_then(|x| x.as_bool()) {
                    self.allow_risky = b;
                    rebuild = true;
                }
            }
            "search" => {
                if let Some(u) = val.and_then(|x| x.as_str()) {
                    if u.starts_with("https://") && u.contains("%s") && u.len() < 200 {
                        self.search = u.to_string();
                    }
                }
            }
            "colors" => match val {
                None | Some(Value::Null) => {
                    self.colors = Value::Null;
                    *COLORS.lock().unwrap() = "null".into();
                }
                Some(v) => match valid_colors(v) {
                    Some(c) => {
                        *COLORS.lock().unwrap() = c.to_string();
                        self.colors = c;
                    }
                    None => {
                        self.eval_ui("toast('Those colors are too hard to read together')");
                        self.push();
                        return;
                    }
                },
            },
            "theme" => {
                if let Some(t) = val.and_then(|x| x.as_str()) {
                    self.apply_theme(t);
                }
            }
            "low_memory" => {
                if let Some(b) = val.and_then(|x| x.as_bool()) {
                    LOW_MEM.store(b, Ordering::Relaxed);
                    self.eval_ui("toast('Takes effect after you restart Lumen')");
                }
            }
            "scripts" => {
                if self.deny_locked() {
                    return;
                }
                if let Some(b) = val.and_then(|x| x.as_bool()) {
                    self.scripts_on = b;
                }
            }
            "script_prompt" => {
                if self.deny_locked() {
                    return;
                }
                if let Some(b) = val.and_then(|x| x.as_bool()) {
                    self.script_prompt = b;
                }
            }
            "sleep" => {
                if let Some(m) = val.and_then(|x| x.as_u64()) {
                    self.sleep_min = m.min(1440);
                    self.sleep_after = if m == 0 { Duration::MAX } else { Duration::from_secs(self.sleep_min * 60) };
                }
            }
            _ => return,
        }
        self.save_settings();
        if rebuild {
            self.rebuild_views();
            self.eval_ui("toast('Applied. This tab was reloaded')");
        }
        self.push();
    }

    fn clear_data(&mut self, what: &str) {
        if (what == "all" || what == "site") && self.deny_locked() {
            return;
        }
        match what {
            "site" => {
                let _ = self.ui.clear_all_browsing_data();
                self.eval_ui("toast('Cache, cookies and site data cleared')");
            }
            "all" => {
                let _ = self.ui.clear_all_browsing_data();
                self.hist.clear();
                self.bm.clear();
                self.hidden.clear();
                self.closed.clear();
                scripts::erase_all();
                self.pending.clear();
                self.eval_ui("S.scripts=[];render()");
                for f in ["history.json", "bookmarks.json", "hidden.json", "session.json"] {
                    let _ = std::fs::remove_file(store::dir().join(f));
                }
                self.tabs.clear();
                self.dirty_hist = false;
                self.new_tab("", false);
                self.push_hist();
                self.eval_ui("toast('Everything erased')");
            }
            _ => {}
        }
    }

    fn push(&self) {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|t| json!({"id": t.id, "title": t.title, "url": t.url, "pinned": t.pinned, "private": t.private, "loading": t.loading, "sleeping": t.wv.is_none() && !t.url.is_empty()}))
            .collect();
        let s = json!({"tabs": tabs, "active": self.active, "bm": self.bm, "sw": self.sw, "settings": self.settings_json(), "editor": self.editor, "locked": self.locked(), "pw_set": self.pw_set, "ask": self.pending.iter().find(|p| p.0 == self.active).map(|p| json!({"name": p.1, "host": p.2}))});
        self.eval_ui(&format!("window.__s({})", s));
    }

    fn push_hist(&self) {
        let h: Vec<&(String, String)> = self.hist.iter().take(300).collect();
        self.eval_ui(&format!("S.hist={};render()", json!(h)));
    }

    fn update_title(&self) {
        let t = self.tabs.iter().find(|t| t.id == self.active).map(|t| t.title.clone()).unwrap_or_default();
        self.window.set_title(&if t.is_empty() { "Lumen".to_string() } else { format!("{t} — Lumen") });
    }

    fn active_view(&self) -> Option<&WebView> {
        self.tabs.iter().find(|t| t.id == self.active).and_then(|t| t.wv.as_ref())
    }

    fn active_url(&self) -> String {
        self.tabs.iter().find(|t| t.id == self.active).map(|t| t.url.clone()).unwrap_or_default()
    }

    fn cycle(&mut self, dir: i64) {
        let n = self.tabs.len() as i64;
        if n < 2 {
            return;
        }
        let i = self.tabs.iter().position(|t| t.id == self.active).unwrap_or(0) as i64;
        let id = self.tabs[((i + dir + n) % n) as usize].id;
        self.select(id);
    }

    fn panel(&mut self, name: &str) {
        if self.sw == 0.0 {
            self.sw = SIDEBAR_W;
            self.layout();
            self.push();
        }
        if name == "history" {
            self.push_hist();
        }
        let _ = self.ui.focus();
        self.eval_ui(&format!("showPanel('{name}')"));
    }

    fn toggle_bookmark(&mut self) {
        let url = self.active_url();
        if url.is_empty() {
            return;
        }
        if let Some(i) = self.bm.iter().position(|b| b.0 == url) {
            self.bm.remove(i);
        } else {
            let title = self.tabs.iter().find(|t| t.id == self.active).map(|t| t.title.clone()).unwrap_or_default();
            self.bm.push((url, title));
        }
        store::save("bookmarks.json", &json!(self.bm));
        self.push();
    }

    fn key(&mut self, combo: &str) {
        match combo {
            "address" => self.focus_address(),
            "new" => self.new_tab("", false),
            "private" => self.new_tab("", true),
            "close" => self.close(self.active),
            "reopen" => {
                if let Some((u, _)) = self.closed.pop() {
                    self.new_tab(&u, false);
                }
            }
            "next" => self.cycle(1),
            "prev" => self.cycle(-1),
            "bookmark" => self.toggle_bookmark(),
            "history" => self.panel("history"),
            "bookmarks" => self.panel("bookmarks"),
            "settings" => self.panel("settings"),
            "fold" => {
                self.sw = if self.sw == 0.0 { SIDEBAR_W } else { 0.0 };
                self.layout();
                self.push();
            }
            "reader" => {
                if let Some(v) = self.active_view() {
                    let _ = v.evaluate_script(READER_JS);
                }
            }
            "hide" => {
                if let Some(v) = self.active_view() {
                    let _ = v.evaluate_script(PICKER_JS);
                    let _ = v.focus();
                    self.picking = Some(self.active);
                }
            }
            "unhide" => {
                let h = host_of(&self.active_url());
                if self.hidden.remove(&h).is_some() {
                    store::save("hidden.json", &json!(self.hidden));
                    if let Some(v) = self.active_view() {
                        let _ = v.reload();
                    }
                    self.eval_ui("toast('Hidden elements restored for this site')");
                }
            }
            c if c.starts_with("tab") => {
                if let Ok(n) = c[3..].parse::<usize>() {
                    let id = if n == 9 { self.tabs.last().map(|t| t.id) } else { self.tabs.get(n - 1).map(|t| t.id) };
                    if let Some(id) = id {
                        self.select(id);
                    }
                }
            }
            _ => {}
        }
    }

    /// Messages from web pages. A page can post anything to window.ipc, so only a
    /// secret-nonce key message from the visible tab, or a hide-save while the picker is armed, is honoured.
    fn tab_ipc(&mut self, id: u64, v: &Value) {
        if id != self.active {
            return;
        }
        match v.get("cmd").and_then(|c| c.as_str()).unwrap_or("") {
            "key" => {
                let ok = v.get("n").and_then(|n| n.as_str()) == Some(self.nonce.as_str());
                let combo = v.get("combo").and_then(|c| c.as_str()).unwrap_or("");
                if ok && TAB_KEYS.contains(&combo) {
                    self.key(combo);
                }
            }
            "hide-save" if self.picking == Some(id) => {
                self.picking = None;
                let sel = v.get("sel").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let host = host_of(&self.active_url());
                if !host.is_empty() && safe_selector(&sel) {
                    let e = self.hidden.entry(host).or_default();
                    if !e.contains(&sel) && e.len() < 50 {
                        e.push(sel);
                    }
                    store::save("hidden.json", &json!(self.hidden));
                    self.eval_ui("toast('Hidden for good on this site. Ctrl+Shift+U brings it back')");
                }
            }
            _ => {}
        }
    }

    fn ipc(&mut self, v: &Value) {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let id = v.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
        match v.get("cmd").and_then(|c| c.as_str()).unwrap_or("") {
            "ready" => {
                self.push();
                self.push_hist();
                self.select(self.active);
            }
            "go" => self.go(&s("text")),
            "open" => {
                if v.get("newtab").and_then(|x| x.as_bool()).unwrap_or(false) {
                    self.new_tab(&s("url"), false)
                } else {
                    self.go_url(&s("url"))
                }
            }
            "select" => self.select(id),
            "close" => self.close(id),
            "new" => self.new_tab("", false),
            "private" => self.new_tab("", true),
            "pin" => {
                if let Some(t) = self.tab(id) {
                    t.pinned = !t.pinned;
                }
                self.dirty_session = true;
                self.push();
            }
            "back" => {
                if let Some(w) = self.active_view() {
                    let _ = w.evaluate_script("history.back()");
                }
            }
            "fwd" => {
                if let Some(w) = self.active_view() {
                    let _ = w.evaluate_script("history.forward()");
                }
            }
            "reload" => {
                if let Some(w) = self.active_view() {
                    let _ = w.reload();
                }
            }
            "bookmark" => self.toggle_bookmark(),
            "delbm" => {
                let u = s("url");
                self.bm.retain(|b| b.0 != u);
                store::save("bookmarks.json", &json!(self.bm));
                self.push();
            }
            "fold" => self.key("fold"),
            "clearhist" => {
                self.hist.clear();
                self.dirty_hist = true;
                self.push_hist();
            }
            "key" => self.key(&s("combo")),
            "set" => self.set_option(&s("key"), v.get("value")),
            "unlock" => self.unlock(&s("pw")),
            "lock" => {
                self.unlocked_until = None;
                self.push();
            }
            "setpw" => {
                let pw = s("pw");
                if self.locked() {
                    self.eval_ui("toast('Unlock first')");
                } else if pw.chars().count() < 6 {
                    self.eval_ui("toast('Use at least 6 characters')");
                } else if scripts::set_password(&pw) {
                    self.pw_set = true;
                    self.unlocked_until = Some(Instant::now() + Duration::from_secs(600));
                    self.eval_ui("toast('Password set')");
                    self.push();
                }
            }
            "clearpw" => {
                if !self.deny_locked() {
                    scripts::clear_password();
                    self.pw_set = false;
                    self.unlocked_until = None;
                    self.eval_ui("toast('Password removed')");
                    self.push();
                }
            }
            "script-answer" => self.script_answer(v),
            "editor-close" => {
                self.editor = false;
                self.layout();
                if let Some(w) = self.active_view() {
                    let _ = w.focus();
                }
            }
            c if c.starts_with("script-") => self.script_cmd(c, v),
            "clear" => self.clear_data(&s("what")),
            _ => {}
        }
    }

    fn on_event(&mut self, ev: Ev) {
        match ev {
            Ev::Ui(v) => self.ipc(&v),
            Ev::Tab(id, v) => self.tab_ipc(id, &v),
            Ev::Open(u) => {
                // Popup limiter: at most 3 pop-up tabs per 10 seconds.
                self.popups.retain(|t| t.elapsed() < Duration::from_secs(10));
                if self.popups.len() >= 3 {
                    self.eval_ui("toast('Pop-up blocked')");
                } else {
                    self.popups.push(Instant::now());
                    self.new_tab(&u, false);
                }
            }
            Ev::Toast(m) => self.eval_ui(&format!("toast({})", json!(m))),
            Ev::Title(id, title) => {
                let mut url = None;
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.title = title.clone();
                    if let Some(v) = &t.wv {
                        if let Ok(u) = v.url() {
                            t.url = u;
                        }
                    }
                    if !t.private {
                        url = Some(t.url.clone());
                    }
                }
                if let Some(u) = url {
                    if let Some(h) = self.hist.iter_mut().find(|h| h.0 == u) {
                        h.1 = title;
                        self.dirty_hist = true;
                    }
                }
                if id == self.active {
                    self.update_title();
                }
                self.push();
            }
            Ev::Load(id, started, url) => {
                let mut record = None;
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.loading = started;
                    if !url.is_empty() && url != "about:blank" {
                        t.url = url.clone();
                    }
                    if !started && !t.private && url.starts_with("http") {
                        record = Some(t.title.clone());
                    }
                }
                if !started {
                    self.run_scripts(id, &url);
                }
                if let Some(title) = record {
                    self.hist.retain(|h| h.0 != url);
                    self.hist.insert(0, (url, title));
                    self.hist.truncate(3000);
                    self.dirty_hist = true;
                }
                self.dirty_session = true;
                self.push();
            }
            Ev::Tick => {
                let active = self.active;
                let after = self.sleep_after;
                let mut slept = false;
                for t in self.tabs.iter_mut() {
                    if t.id != active && t.wv.is_some() && t.used.elapsed() > after {
                        t.wv = None; // frees the renderer process for this tab
                        t.loading = false;
                        slept = true;
                    }
                }
                if slept {
                    self.push();
                }
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        if self.dirty_session {
            let list: Vec<&Tab> = self.tabs.iter().filter(|t| !t.private && !t.url.is_empty()).collect();
            let active = list.iter().position(|t| t.id == self.active).unwrap_or(0);
            let tabs: Vec<Value> = list.iter().map(|t| json!({"url": t.url, "title": t.title, "pinned": t.pinned})).collect();
            store::save("session.json", &json!({"active": active, "tabs": tabs}));
            self.dirty_session = false;
        }
        if self.dirty_hist {
            store::save("history.json", &json!(self.hist));
            self.dirty_hist = false;
        }
    }
}

fn main() {
    // No console window in any build, so write panics to a file instead.
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::write(store::dir().join("crash.log"), info.to_string());
    }));
    let settings = store::load("settings.json");
    let colors = settings.get("colors").and_then(valid_colors).unwrap_or(Value::Null);
    *COLORS.lock().unwrap() = colors.to_string();
    let search = settings.get("search").and_then(|v| v.as_str()).unwrap_or("https://www.google.com/search?q=%s").to_string();
    let sleep_min = settings.get("sleep_minutes").and_then(|v| v.as_u64()).unwrap_or(15).min(1440);
    let adblock_on = settings.get("adblock").and_then(|v| v.as_bool()).unwrap_or(false);
    let allow_risky = settings.get("allow_risky_downloads").and_then(|v| v.as_bool()).unwrap_or(false);
    let scripts_on = settings.get("scripts_enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let script_prompt = settings.get("script_prompt").and_then(|v| v.as_bool()).unwrap_or(true);
    LOW_MEM.store(settings.get("low_memory").and_then(|v| v.as_bool()).unwrap_or(false), Ordering::Relaxed);
    THEME.store(theme_id(settings.get("theme").and_then(|v| v.as_str()).unwrap_or("light")), Ordering::Relaxed);

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Lumen")
        .with_inner_size(LogicalSize::new(1200.0, 780.0))
        .with_theme(window_theme())
        .with_window_icon(tao::window::Icon::from_rgba(ICON_RGBA.to_vec(), 64, 64).ok())
        .build(&event_loop)
        .expect("window");

    let mut ctx = WebContext::new(Some(store::dir().join("WebView2")));
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let p_ipc = proxy.clone();
    let ui = builder(&mut ctx)
        .with_custom_protocol("lumen".into(), |_id, req| {
            let (body, mime): (Cow<'static, [u8]>, &str) = match req.uri().path() {
                "/ui.html" => (Cow::Owned(UI_HTML.replace("__THEME__", theme_name()).replace("__COLORS__", &COLORS.lock().map(|c| if c.is_empty() { "null".to_string() } else { c.clone() }).unwrap_or_else(|_| "null".into())).into_bytes()), "text/html; charset=utf-8"),
                "/f/JetBrainsMono-Regular.woff2" => (Cow::Borrowed(F_REG), "font/woff2"),
                "/f/JetBrainsMono-Medium.woff2" => (Cow::Borrowed(F_MED), "font/woff2"),
                "/f/JetBrainsMono-Bold.woff2" => (Cow::Borrowed(F_BOLD), "font/woff2"),
                "/icon.png" => (Cow::Borrowed(ICON_PNG), "image/png"),
                _ => (Cow::Borrowed(&b""[..]), "text/plain"),
            };
            Response::builder()
                .header("Content-Type", mime)
                .header("Content-Security-Policy", UI_CSP)
                .header("X-Content-Type-Options", "nosniff")
                .body(body)
                .unwrap()
        })
        .with_url(ui_url())
        .with_bounds(rect(0.0, 0.0, size.width, size.height))
        .with_initialization_script(KEYS_JS.replace("__NONCE__", "ui"))
        .with_background_color(bg_color())
        .with_focused(true)
        .with_ipc_handler(move |req| {
            if let Ok(v) = serde_json::from_str::<Value>(req.body()) {
                let _ = p_ipc.send_event(Ev::Ui(v));
            }
        })
        .with_navigation_handler(|u| u.starts_with(UI_ORIGIN))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_permission_handler(|_| PermissionResponse::Deny)
        .build_as_child(&window)
        .expect("ui webview");

    let adblock = adblock_on.then(adblock_script);

    let hidden: HashMap<String, Vec<String>> = store::load("hidden.json")
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();

    let mut app = App {
        window,
        ui,
        ctx,
        proxy: proxy.clone(),
        tabs: Vec::new(),
        active: 0,
        next: 1,
        closed: Vec::new(),
        bm: pairs(&store::load("bookmarks.json")),
        hist: pairs(&store::load("history.json")),
        hidden,
        search,
        sleep_after: if sleep_min == 0 { Duration::MAX } else { Duration::from_secs(sleep_min * 60) },
        adblock,
        sleep_min,
        sw: SIDEBAR_W,
        downloads: dirs::download_dir().unwrap_or_else(std::env::temp_dir),
        dirty_session: false,
        dirty_hist: false,
        nonce: random_nonce(),
        picking: None,
        popups: Vec::new(),
        allow_risky,
        scripts_on,
        script_prompt,
        pw_set: scripts::has_password(),
        unlocked_until: None,
        fails: 0,
        lockout_until: None,
        pending: Vec::new(),
        editor: false,
        colors,
    };

    if settings.is_null() {
        app.save_settings();
    }

    // Restore last session. Tabs come back asleep: no webview until selected.
    let session = store::load("session.json");
    let mut restored_active = 0usize;
    if let Some(list) = session.get("tabs").and_then(|t| t.as_array()) {
        restored_active = session.get("active").and_then(|a| a.as_u64()).unwrap_or(0) as usize;
        for t in list {
            if !t.get("url").and_then(|x| x.as_str()).map(safe_url).unwrap_or(false) {
                continue;
            }
            let id = app.next;
            app.next += 1;
            app.tabs.push(Tab {
                id,
                url: t.get("url").and_then(|x| x.as_str()).unwrap_or("").into(),
                title: t.get("title").and_then(|x| x.as_str()).unwrap_or("").into(),
                pinned: t.get("pinned").and_then(|x| x.as_bool()).unwrap_or(false),
                private: false,
                loading: false,
                used: Instant::now(),
                wv: None,
            });
        }
    }
    if app.tabs.is_empty() {
        let id = app.next;
        app.next += 1;
        app.tabs.push(Tab { id, url: String::new(), title: String::new(), pinned: false, private: false, loading: false, used: Instant::now(), wv: None });
        restored_active = 0;
    }
    app.active = app.tabs[restored_active.min(app.tabs.len() - 1)].id;

    // One timer thread: wakes once a minute to sleep idle tabs and flush to disk.
    let tick = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        if tick.send_event(Ev::Tick).is_err() {
            break;
        }
    });

    event_loop.run(move |event, _, cf| {
        *cf = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.dirty_session = true;
                app.flush();
                *cf = ControlFlow::Exit;
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.layout(),
            Event::UserEvent(ev) => app.on_event(ev),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(a: &str, b: &str, sd: &str, t: &str) -> Value {
        json!({"accent": a, "bg": b, "side": sd, "text": t})
    }

    #[test]
    fn presets_are_readable() {
        let presets = [
            set("#b45309", "#fbf6ec", "#f1e9d8", "#3b3226"),
            set("#0284c7", "#f4f9fc", "#e6f1f7", "#12303f"),
            set("#15803d", "#f6faf5", "#e8f1e6", "#1c2b1f"),
            set("#e11d48", "#fff7f8", "#fdeaed", "#3a1f26"),
            set("#f59e0b", "#1a1a1a", "#242424", "#eaeaea"),
            set("#8b7cff", "#0b1020", "#121935", "#e8ecff"),
        ];
        for p in presets {
            assert!(valid_colors(&p).is_some(), "{p}");
        }
    }

    #[test]
    fn unreadable_or_malformed_colors_are_refused() {
        assert!(valid_colors(&set("#2563eb", "#ffffff", "#ffffff", "#ffffff")).is_none());
        assert!(valid_colors(&set("#ffffff", "#ffffff", "#f5f6f8", "#1f2430")).is_none());
        assert!(valid_colors(&set("red", "#ffffff", "#f5f6f8", "#1f2430")).is_none());
        assert!(valid_colors(&json!({"accent": "#2563eb"})).is_none());
        assert!(valid_colors(&set("#2563eb", "#ffffff", "#f5f6f8", "#1f2430")).is_some());
    }

    #[test]
    fn url_schemes() {
        assert!(safe_url("https://a.com") && safe_url("http://a.com"));
        assert!(!safe_url("file:///c:/x") && !safe_url("javascript:alert(1)") && !safe_url("data:text/html,x"));
        assert!(!safe_selector("a{color:red}") && safe_selector("div.ad>p:nth-of-type(2)"));
    }
}
