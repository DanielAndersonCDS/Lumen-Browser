# Lumen

A very light, single-window browser for Windows 11 (icon: neon orbit "L"). Light by default, with a dark theme. Rust + WebView2 (the Edge engine already on Windows 11). One small .exe, no bundled Chromium. Light theme only. JetBrains Mono is embedded in the exe and used for all browser UI.

## Build (Windows 11)

1. Install Rust: https://rustup.rs  (choose the default MSVC toolchain; it will ask you to install the VS Build Tools)
2. In this folder:

```
cargo run            # debug build, F12 opens devtools
cargo build --release
```

The binary is `target\release\lumen.exe`. Lumen opens no console window in any build (double-click the exe to launch it with nothing else on screen; `cargo run` still keeps the terminal you ran it from). If it ever closes unexpectedly, the reason is in `%LOCALAPPDATA%\Lumen\crash.log`. WebView2 Runtime ships with Windows 11, nothing else to install.

## Keys

| Key | Action |
|---|---|
| Ctrl+L | address / search (focus the field) |
| Ctrl+T / Ctrl+W | new / close tab |
| Ctrl+Shift+T | reopen closed tab |
| Ctrl+Shift+N | private tab |
| Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+1..9 | switch tabs |
| Ctrl+D | bookmark page |
| Ctrl+Y / Ctrl+Shift+B | history / bookmarks panel |
| Ctrl+B | fold the sidebar away |
| Ctrl+, | settings |
| Ctrl+Shift+R | reading mode (press again to leave) |
| Ctrl+Shift+H | hide an element on this site for good (click it; Esc cancels) |
| Ctrl+Shift+U | restore everything hidden on this site |
| Ctrl+F, Ctrl+P, Ctrl +/-, F5, Alt+Left/Right | handled natively by WebView2 |

Right-click a pinned tab to unpin it. Middle-click closes a tab.

## How it stays light

- One event loop that sleeps until something happens: ~0% CPU idle.
- A tab gets a webview (renderer process) only when you first select it. Restored tabs cost nothing.
- Background tabs unused for 15 minutes are put to sleep (webview dropped, title and URL kept).
- While a page is showing, the browser's own UI layer shrinks to just the sidebar, so the page isn't drawn over a second full-window layer.
- Hidden tabs also ask WebView2 to trim their memory (Windows).
- No favicons, no extensions, no sync, no telemetry, no update service, no background threads except one 60 s timer.
- Release profile: size-optimised, LTO, stripped, panic=abort.

## Files (in %LOCALAPPDATA%\Lumen)

`settings.json` (all of these are also in the Settings panel; search URL with `%s`, default Google; Bing is `https://www.bing.com/search?q=%s`, Yahoo is `https://search.yahoo.com/search?p=%s`. Also `sleep_minutes` (0 = never), `adblock` (off by default), `allow_risky_downloads`), `session.json`, `history.json`, `bookmarks.json`, `hidden.json`, and `WebView2\` (cookies and site data). Private tabs write nothing.

## Colors (Settings > Colors)

For people who don't want to touch files: pick a preset (Paper, Ocean, Forest, Rose, Graphite, Neon) or set four colors yourself with the color pickers: accent, background, sidebar, text. Everything else (borders, hover, muted text) is worked out from those. Changes show live. Lumen refuses color combinations that are hard to read (so you can't lock yourself out of the UI), and **Reset colors** goes back to the default. Websites keep their own colors; use Theme for the light/dark preference sites see. The choice is saved in `settings.json` under `colors` (delete that line to reset by hand).

## Dark mode

Settings > Theme: Light (default), Dark, or Follow Windows. It switches the sidebar and also tells web pages your preference, so sites with a dark style use it.

## Memory saver mode (off by default)

Settings > Memory saver mode passes WebView2's low-end-device flag after a restart. It cuts memory but caps the JS heap near 256 MB, which slows or breaks heavy sites and hurts benchmark scores. Leave it off unless you are short on RAM.

## Scripts (off by default)

Settings > Run my own scripts, then the Scripts button. A script is a plain .js file saved in `%LOCALAPPDATA%\Lumen\scripts`. Write it in the built-in editor (Ctrl+S saves).

```
// @name My script
// @match https://*.example.com/*
document.title = "[Lumen] " + document.title;
```

Rules:
- Each script must have `@match` lines naming real sites. `*://*/*`, `https://*/*` and `*.com` are refused.
- It runs after the page loads, in that page only. It cannot reach the browser, history, bookmarks or other sites' data.
- By default the first time a script wants to run on a site you get a prompt in the sidebar: Allow once, Always here, or Block. Click a site chip in the Scripts panel to forget a choice. This prompt can be turned off in Settings.
- Optional unlock password (Settings): when set, it is required to open the Scripts panel, change script settings, or clear/erase data. It unlocks for 10 minutes, locks out for 30 s after 5 wrong tries, and is stored only as a salted PBKDF2-SHA256 hash (600,000 rounds). It cannot be recovered; to reset it delete `lock.json`, which anyone with access to your Windows account can do, so it protects against casual use and other people at your keyboard, not against someone who controls your account.
- A script you approve can still read or change whatever is on the page it runs on. Only run scripts you wrote or fully understand.

## Settings and clearing data (Ctrl+,)

Theme, ad blocker on/off, memory saver, scripts, unlock password, search engine (Google, Bing, Yahoo), tab sleep time, risky downloads, and three clear buttons, each needing a second click to confirm:

- **Clear cache, cookies and site data** (signs you out of sites)
- **Clear history**
- **Erase everything** (the above plus bookmarks, hidden elements and open tabs; settings stay)

Lumen has no built-in password manager. After clearing cookies, sign back in with whatever password manager you already use (a desktop app or your phone).

## Security

- **Pages can't drive the browser.** Web pages can post messages to the browser, so every tab message must carry a per-launch random secret that only the real key handler knows, come from the visible tab, and be a known shortcut. Synthetic (script-made) key and click events are ignored.
- **Only http and https.** Typed addresses, restored sessions, history and page navigations are limited to http/https (plus about:blank and blob: for downloads). `file:`, `javascript:`, `data:` and app-launching schemes are refused.
- **Browser UI is locked down.** It is served from inside the exe with a strict Content-Security-Policy (no network, no external anything), can't be navigated away, and gets no permissions.
- **Downloads** land in your Downloads folder without overwriting existing files. Executable and script types (.exe, .msi, .bat, .ps1, .js, .lnk ...) are blocked by default; set `allow_risky_downloads` to true in settings.json to allow them. WebView2's SmartScreen stays on.
- **Pop-ups** are limited to 3 per 10 seconds. MIDI, motion sensors, local-font enumeration and window-management permissions are always denied; camera, microphone, location and notifications keep WebView2's own per-site prompt.
- **Plain HTTP pages** turn the address field amber.
- **Build hardening:** Control Flow Guard and overflow checks on, devtools only in debug builds, four small dependencies, `Cargo.lock` included (run `cargo audit` now and then).
- **Patches:** the web engine is the Evergreen WebView2 runtime, which Windows updates by itself. Keep Windows Update on.

## Measuring performance honestly

- Task Manager > Processes: expand the **Lumen** group. The Lumen process alone is only the Rust shell; the real cost is the `Microsoft Edge WebView2` processes under it. Add them up.
- Benchmarks (Speedometer etc.): use the same power mode (plugged in, "Best performance"), close other apps, let the PC cool, run 3 to 5 times and compare the median. A swing from 17 to 30 usually means CPU throttling or background load, not the browser.
- If a benchmark page reports a JS heap limit near 256 MB, memory saver mode is on.

## Known limits

- The ad/tracker blocker (opt-in) is script-level (it intercepts fetch, XHR, beacons and script/img/iframe sources against a host list, plus a little cosmetic CSS). It is not network-level, so some scripts that run before it will get through. A real WebView2 request filter is the next step.
- Web pages keep their own fonts; JetBrains Mono is the browser's UI font.
- No tab drag-reordering yet.
