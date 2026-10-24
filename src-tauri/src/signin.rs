//! Where a CLI sign-in's web page opens. By default each sign-in, or each email's run of sign-ins
//! in "Sign in all accounts", gets a brand-new empty browser profile (a [`Session`]: a folder under
//! `signin-sessions` in the app's local data folder): no Google account from an earlier sign-in is
//! offered, Google is asked once for all of an email's sign-ins, and once they end the browser is
//! closed and the folder deleted. The user completes every page; SlopBar only chooses the window.
//!
//! Profiles used to be kept per email (`signin-profiles`), and they collected stale Google
//! sessions; [`cleanup`] deletes them at startup.
//!
//! The CLIs open their page themselves, so SlopBar steers them: Claude Code runs `BROWSER` with
//! the URL on Windows, so `BROWSER` points at `slopbar.exe` with [`TARGET_ENV`] saying where to
//! open it ([`launch_from_env`] handles that start before anything else). Codex ignores `BROWSER`
//! on Windows; its app server hands the URL back instead (see `accounts`).

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Set on a CLI sign-in that `BROWSER` points at SlopBar: `profile:<folder>` or `private`.
pub const TARGET_ENV: &str = "SLOPBAR_SIGNIN_TARGET";

/// The user's choice of window (Settings > Accounts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Window {
    /// A fresh browser profile for each email's sign-ins, deleted once they end. (The name is
    /// kept from when profiles were kept per email, so saved settings still read.)
    #[default]
    Profile,
    /// A private (InPrivate / Incognito) window: nothing is kept, so Google asks every time the
    /// private session was closed.
    Private,
    /// Whatever the CLI opens: the default browser, as before.
    System,
}

/// Where one sign-in's page opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Profile(PathBuf),
    Private,
}

impl Target {
    /// The window for a sign-in. A profile is `session`'s; without one (no data folder), or with
    /// `System`, the CLI opens its page as it always did.
    pub fn choose(window: Window, session: Option<&Session>) -> Option<Self> {
        match window {
            Window::System => None,
            Window::Private => Some(Self::Private),
            Window::Profile => session.map(Session::target),
        }
    }

    fn encode(&self) -> OsString {
        match self {
            Self::Private => "private".into(),
            Self::Profile(dir) => {
                let mut s = OsString::from("profile:");
                s.push(dir);
                s
            }
        }
    }

    fn decode(value: &std::ffi::OsStr) -> Option<Self> {
        let value = value.to_str()?;
        if value == "private" {
            return Some(Self::Private);
        }
        Some(Self::Profile(PathBuf::from(value.strip_prefix("profile:")?)))
    }
}

/// Where the fresh profiles live while their sign-ins run: they hold provider and Google
/// cookies, so outside the roaming profile and never in a backup.
pub fn sessions_root() -> Option<PathBuf> {
    Some(crate::settings::local_dir()?.join("signin-sessions"))
}

/// The profiles kept per email by earlier versions; [`cleanup`] deletes them.
fn legacy_root() -> Option<PathBuf> {
    Some(crate::settings::local_dir()?.join("signin-profiles"))
}

/// Profiles whose sign-ins are running: never deleted from under them by [`cleanup`].
static ACTIVE: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn set_active(dir: &Path, active: bool) {
    let mut set = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let set = set.get_or_insert_with(HashSet::new);
    if active { set.insert(dir.to_owned()); } else { set.remove(dir); }
}

fn is_active(dir: &Path) -> bool {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.contains(dir))
}

/// A brand-new empty browser profile for one email's sign-ins (or one sign-in). Every sign-in
/// given its [`target`](Session::target) opens in the same browser, so Google is asked once; when
/// the session is dropped that browser is closed and the folder overwritten and deleted.
#[derive(Debug)]
pub struct Session {
    dir: PathBuf,
    /// Its browser has been seen running (see [`Session::browser_closed`]).
    opened: std::sync::atomic::AtomicBool,
    /// The account and language its page was labelled with, and how many times the page's
    /// progress was written (see [`Session::show`]).
    page: Mutex<Option<(String, String)>>,
    revision: std::sync::atomic::AtomicU64,
}

impl Session {
    /// A new profile folder under `root`, named at random so no two sessions ever share one.
    pub fn create_in(root: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let dir = root.join(uuid::Uuid::new_v4().simple().to_string());
        // Active before it exists, so a cleanup running now never takes it.
        set_active(&dir, true);
        if let Err(e) = std::fs::create_dir(&dir) {
            set_active(&dir, false);
            return Err(e);
        }
        Ok(Self { dir, opened: Default::default(), page: Default::default(), revision: Default::default() })
    }

    /// Names the window after the email it signs in, before its browser first starts: the
    /// profile is named after it (the profile button, where the browser honours that) and the
    /// window's first tab is a page in `lang` saying whose sign-ins these are (see [`open`]), since
    /// Codex's page cannot be prefilled and nothing else on it names the account. Best effort.
    pub fn label(&self, email: &str, lang: &str) {
        let name: String = email.chars().filter(|c| !c.is_control()).take(64).collect();
        if name.is_empty() {
            return;
        }
        let _ = std::fs::write(self.dir.join(LABEL_PAGE), label_page(&name, lang, None, 0));
        *self.page.lock().unwrap_or_else(|e| e.into_inner()) = Some((name.clone(), lang.to_owned()));
        let state = self.dir.join("Local State");
        if state.exists() {
            return;
        }
        let info = serde_json::json!({ "name": name, "is_using_default_name": false, "shortcut_name": name });
        let local = serde_json::json!({ "profile": { "info_cache": { "Default": info } } });
        let _ = std::fs::write(&state, local.to_string());
    }

    /// Shows on the labelled page how the email's turn is going: the page rewritten (for a
    /// reload) and its sidecar script, which the open page loads every second and redraws from,
    /// so the person sees each sign-in finish and the next one starting. Best effort; nothing
    /// before [`label`](Session::label).
    pub fn show(&self, progress: &Progress) {
        let Some((account, lang)) = self.page.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        let revision = self.revision.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let (title, html) = (page_title(&account, &lang, Some(progress)), progress_html(&account, &lang, progress));
        let script = format!("slopbarProgress({});\n", serde_json::json!({ "revision": revision, "title": title, "html": html }));
        write_replacing(&self.dir.join(PROGRESS_SCRIPT), &script);
        write_replacing(&self.dir.join(LABEL_PAGE), &label_page(&account, &lang, Some(progress), revision));
    }

    /// Whether a sign-in page was opened in this profile at or after `since` (see [`open`]).
    pub fn page_opened_since(&self, since: std::time::SystemTime) -> bool {
        // The time is written in the file: a file's own time on Windows is coarser than the clock.
        let opened = std::fs::read_to_string(self.dir.join(OPENED)).ok().and_then(|s| s.trim().parse::<u128>().ok());
        opened.is_some_and(|ms| ms >= millis(since))
    }

    /// A new profile in [`sessions_root`].
    pub fn create() -> Option<Self> {
        Self::create_in(&sessions_root()?).ok()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn target(&self) -> Target {
        Target::Profile(self.dir.clone())
    }

    /// Whether the browser, having opened for the sign-in running now, is gone: the user closed
    /// it, so that sign-in cannot finish there.
    pub fn browser_closed(&self) -> bool {
        use std::sync::atomic::Ordering;
        if browser_open(&self.dir) {
            self.opened.store(true, Ordering::Relaxed);
            return false;
        }
        self.opened.load(Ordering::Relaxed)
    }

    /// A new sign-in of the email's starts: a browser closed after the last one finished (the
    /// user closing a "you're signed in" page) is no reason to stop; this one opens it again.
    pub fn rearm(&self) {
        self.opened.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for Session {
    /// Closing and deleting can wait seconds on a browser letting go of its files, so on an async
    /// runtime it runs on a thread of its own.
    fn drop(&mut self) {
        let dir = std::mem::take(&mut self.dir);
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::spawn(move || end(&dir));
        } else {
            end(&dir);
        }
    }
}

fn end(dir: &Path) {
    close_browser(dir);
    let _ = remove_profile(dir);
    // A sign-in page whose browser was still starting (Claude's, through `BROWSER`) gets its
    // window, and the folder again, only after the first pass: a second one takes those too.
    if browser_open(dir) || dir.exists() {
        std::thread::sleep(Duration::from_secs(1));
        close_browser(dir);
        let _ = remove_profile(dir);
    }
    set_active(dir, false);
}

/// Closes every running session's browser and deletes its profile, as SlopBar exits.
pub fn close_all() {
    let dirs: Vec<PathBuf> = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.iter().cloned().collect()).unwrap_or_default();
    for dir in dirs {
        close_browser(&dir);
        let _ = remove_profile(&dir);
    }
}

/// Closes the browser running `dir` as its profile, if one is: its main process (see
/// [`browser_pid`]) and the ones it started are ended. The profile is about to be deleted, so
/// there is nothing to save.
#[cfg(windows)]
pub fn close_browser(dir: &Path) {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
    let Some(pid) = browser_pid(dir) else { return };
    // The browser process and its children (renderers, GPU), found before it ends.
    let mut pids = vec![pid];
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap != INVALID_HANDLE_VALUE {
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                if entry.th32ParentProcessID == pid && entry.th32ProcessID != pid {
                    pids.push(entry.th32ProcessID);
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
    }
    // Every handle is held before anything ends: a process ID is only that process's while it
    // runs, and one freed by a child that just exited could name another program.
    let handles: Vec<_> = pids.into_iter().map(|p| unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, p) })
        .filter(|h| !h.is_null()).collect();
    for h in &handles {
        unsafe { TerminateProcess(*h, 0) };
    }
    for h in handles {
        unsafe {
            WaitForSingleObject(h, 3000);
            CloseHandle(h);
        }
    }
}

#[cfg(not(windows))]
pub fn close_browser(_: &Path) {}

/// Whether a browser runs `dir` as its profile (see [`close_browser`]).
#[cfg(windows)]
pub fn browser_open(dir: &Path) -> bool {
    browser_pid(dir).is_some()
}

#[cfg(not(windows))]
pub fn browser_open(_: &Path) -> bool {
    false
}

/// The main process of the Chromium browser whose profile is `dir`: it names its hidden
/// `Chrome_MessageWindow` after the folder.
#[cfg(windows)]
fn browser_pid(dir: &Path) -> Option<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE};
    let class: Vec<u16> = "Chrome_MessageWindow".encode_utf16().chain([0]).collect();
    let title: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut pid = 0u32;
    unsafe {
        let hwnd = FindWindowExW(HWND_MESSAGE, std::ptr::null_mut(), class.as_ptr(), title.as_ptr());
        if hwnd.is_null() {
            return None;
        }
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    (pid != 0).then_some(pid)
}

/// Deletes a profile folder, first overwriting each file with zeros so its cookies do not linger
/// in freed disk space. Files a closing browser still holds are retried for a few seconds.
pub fn remove_profile(dir: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        // A link where a profile should be: only the link goes, never what it points at.
        Ok(m) if m.file_type().is_symlink() => return std::fs::remove_dir(dir).or_else(|_| std::fs::remove_file(dir)),
        _ => {}
    }
    let mut last = Ok(());
    for attempt in 0..20 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(250));
        }
        // Again now and then for files that were held: what was deleted needs no second pass.
        if attempt % 5 == 0 {
            scrub(dir);
        }
        last = match std::fs::remove_dir_all(dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        };
        if last.is_ok() {
            break;
        }
    }
    last
}

/// Overwrites every file under `dir` with zeros, never following a link out of it.
fn scrub(dir: &Path) {
    use std::io::Write;
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let zeros = [0u8; 64 * 1024];
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_symlink() {
            continue;
        } else if kind.is_dir() {
            scrub(&path);
        } else if let (Ok(len), Ok(mut f)) = (entry.metadata().map(|m| m.len()), std::fs::OpenOptions::new().write(true).open(&path)) {
            // A hard link shares its bytes with a file elsewhere: deleting the link is enough.
            if linked_elsewhere(&f) {
                continue;
            }
            let mut left = len;
            while left > 0 {
                let n = left.min(zeros.len() as u64) as usize;
                if f.write_all(&zeros[..n]).is_err() {
                    break;
                }
                left -= n as u64;
            }
            let _ = f.sync_all();
        }
    }
}

#[cfg(windows)]
fn linked_elsewhere(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 || info.nNumberOfLinks > 1 }
}

#[cfg(not(windows))]
fn linked_elsewhere(file: &std::fs::File) -> bool {
    use std::os::unix::fs::MetadataExt;
    file.metadata().map_or(true, |m| m.nlink() > 1)
}

/// A session's own folder: named as [`Session::create_in`] names them, directly under
/// [`sessions_root`], and a real folder rather than a link to one.
fn session_folder(dir: &Path) -> bool {
    let named = dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit()));
    named && sessions_root().is_some_and(|root| dir.parent() == Some(root.as_path()))
        && std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// An email safe to hand to a CLI on its command line (Claude's `--email`). The sign-in runs
/// through a `cmd` script, so anything beyond the plain address characters is not prefilled.
pub fn prefill_email(email: Option<&str>) -> Option<&str> {
    let email = email?.trim();
    let (local, domain) = email.split_once('@')?;
    let plain = |s: &str, extra: &[u8]| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c));
    (email.len() <= 254 && plain(local, b"._+-") && plain(domain, b".-") && domain.contains('.')
        && !domain.starts_with(['.', '-']) && !local.starts_with('-'))
        .then_some(email)
}

/// The page in a labelled profile (see [`Session::label`]) that names its account.
const LABEL_PAGE: &str = "slopbar-account.html";
/// Beside it, the progress the open page redraws from (see [`Session::show`]).
const PROGRESS_SCRIPT: &str = "slopbar-progress.js";

/// How one sign-in of an email's turn is going, as its page shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageStep {
    Pending,
    /// Its CLI is starting; its page opens in a new tab once it is up.
    Opening,
    /// Its page is open: the person signs in there.
    Open,
    Done,
    /// Already done when its turn came.
    Skipped,
}

/// An email's turn, for its page (see [`Session::show`]).
pub struct Progress {
    /// Each of the email's sign-ins: what it is ("Claude", "Codex monitoring") and how it is going.
    pub steps: Vec<(String, PageStep)>,
    /// The sign-in of the whole run that is open now (1-based), of how many.
    pub at: usize,
    pub total: usize,
}

/// Replaces `path`'s contents so a reader never sees half of them, else just writes it.
fn write_replacing(path: &Path, contents: &str) {
    let part = path.with_extension("part");
    if std::fs::write(&part, contents).and_then(|_| std::fs::rename(&part, path)).is_err() {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::write(path, contents);
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The tab's title; with progress it starts with how many of the email's sign-ins are done.
fn page_title(account: &str, lang: &str, progress: Option<&Progress>) -> String {
    let title = crate::i18n::t(lang, "signinPageTitle", &[("account", account)]);
    match progress {
        Some(p) => {
            let done = p.steps.iter().filter(|(_, s)| matches!(s, PageStep::Done | PageStep::Skipped)).count();
            format!("{done}/{} \u{b7} {title}", p.steps.len())
        }
        None => title,
    }
}

/// The page's progress section: where the run is, then each of the email's sign-ins.
fn progress_html(account: &str, lang: &str, progress: &Progress) -> String {
    let t = |key: &str, vars: &[(&str, &str)]| esc(&crate::i18n::t(lang, key, vars));
    let (at, total) = (progress.at.to_string(), progress.total.to_string());
    let mut html = format!("<p class=\"overall\">{}</p><ol>", t("signinPageOverall", &[("i", &at), ("n", &total)]));
    for (name, state) in &progress.steps {
        let (class, mark, note) = match state {
            PageStep::Pending => ("pending", "", String::new()),
            PageStep::Opening => ("opening", "<i class=\"spin\"></i>", t("signinPageOpening", &[])),
            PageStep::Open => ("open", "&#10140;", t("signinPageOpen", &[("account", account)])),
            PageStep::Done => ("done", "&#10003;", t("signinPageDone", &[])),
            PageStep::Skipped => ("done", "&#10003;", t("signinPageSkipped", &[])),
        };
        html += &format!("<li class=\"{class}\"><span class=\"mark\">{mark}</span><span class=\"name\">{}</span><span class=\"note\">{note}</span></li>", esc(name));
    }
    html + "</ol>"
}

/// Touched in a profile each time a sign-in page is opened in it, by whichever process opened it
/// (Claude's opens through `BROWSER`, in a process of its own): see [`Session::page_opened_since`].
const OPENED: &str = "slopbar-page-opened";

fn millis(t: std::time::SystemTime) -> u128 {
    t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

/// The label page: its title (what the tab shows) and heading name the account, it asks the person
/// to keep the window open, and with `progress` (from [`Session::show`], its `revision`th) it
/// shows how the email's sign-ins are going, redrawing itself as [`PROGRESS_SCRIPT`] changes.
fn label_page(account: &str, lang: &str, progress: Option<&Progress>, revision: u64) -> String {
    let t = |key: &str| esc(&crate::i18n::t(lang, key, &[("account", account)]));
    let section = progress.map(|p| progress_html(account, lang, p)).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         :root{{color-scheme:light dark;font-family:'Segoe UI',system-ui,sans-serif;--ok:#15803d;--warn:#b45309}}\
         @media (prefers-color-scheme:dark){{:root{{--ok:#4ade80;--warn:#fbbf24}}}}\
         body{{max-width:40em;margin:10vh auto;padding:0 24px;line-height:1.5}}h1{{font-size:1.6em;overflow-wrap:anywhere}}\
         .keep{{font-weight:600;padding:10px 14px;border-left:4px solid var(--warn);background:color-mix(in srgb,var(--warn) 12%,transparent)}}\
         .overall{{margin-bottom:.3em;opacity:.75}}ol{{list-style:none;padding:0;margin-top:0}}\
         li{{display:flex;gap:.6em;align-items:baseline;padding:.3em 0}}.mark{{width:1.2em;text-align:center;flex:none}}\
         .name{{font-weight:600}}.note{{opacity:.8}}.pending{{opacity:.55}}.done .mark{{color:var(--ok)}}\
         .spin{{display:inline-block;width:.8em;height:.8em;border:2px solid currentColor;border-right-color:transparent;border-radius:50%;animation:spin .8s linear infinite;vertical-align:-.1em}}\
         @keyframes spin{{to{{transform:rotate(1turn)}}}}@media (prefers-reduced-motion:reduce){{.spin{{animation-duration:2.4s}}}}\
         </style></head><body><h1>{heading}</h1><p class=\"keep\">{keep}</p><div id=\"progress\" aria-live=\"polite\">{section}</div>\
         <p>{body}</p><p>{codex}</p><script>(function(){{var seen={revision},n=0,box=document.getElementById('progress');\
         window.slopbarProgress=function(p){{if(p.revision>seen){{seen=p.revision;box.innerHTML=p.html;document.title=p.title;}}}};\
         function poll(){{var s=document.createElement('script');s.src='{script}?'+(++n);s.onload=s.onerror=function(){{s.remove();}};document.head.appendChild(s);}}\
         setInterval(poll,1000);document.addEventListener('visibilitychange',poll);}})();</script></body></html>",
        lang = esc(lang), title = esc(&page_title(account, lang, progress)), heading = t("signinPageHeading"), keep = t("signinPageKeepOpen"),
        body = t("signinPageBody"), codex = t("signinPageCodex"), script = PROGRESS_SCRIPT,
    )
}

/// Only the providers' own sign-in pages are opened, over HTTPS.
pub fn allowed_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else { return false };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.to_ascii_lowercase();
    ["claude.ai", "claude.com", "anthropic.com", "openai.com", "chatgpt.com"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Chrome,
    Edge,
}

/// A browser that takes `--user-data-dir`: Chrome when installed (its first run is quiet with
/// `--no-first-run`), else Edge, which every Windows 11 has.
pub fn find_browser() -> Option<(Kind, PathBuf)> {
    let env = |n: &str| std::env::var_os(n).map(PathBuf::from);
    let mut candidates = Vec::new();
    for base in [env("ProgramFiles"), env("ProgramFiles(x86)"), env("LOCALAPPDATA")].into_iter().flatten() {
        candidates.push((Kind::Chrome, base.join(r"Google\Chrome\Application\chrome.exe")));
    }
    for base in [env("ProgramFiles(x86)"), env("ProgramFiles")].into_iter().flatten() {
        candidates.push((Kind::Edge, base.join(r"Microsoft\Edge\Application\msedge.exe")));
    }
    candidates.into_iter().find(|(_, p)| p.is_file())
}

/// The browser's arguments for one page. The URL goes last and is an https URL (or the label
/// page's file URL), so it can never be read as a switch.
pub fn browser_args(kind: Kind, target: &Target, url: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    match target {
        Target::Profile(dir) => {
            let mut a = OsString::from("--user-data-dir=");
            a.push(dir);
            args.push(a);
            // A new profile: no welcome pages, browser sign-in or sync offers, extensions or
            // default-browser question in the way of the provider's page.
            for flag in ["--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-extensions", "--disable-default-apps"] {
                args.push(flag.into());
            }
        }
        Target::Private => {
            args.push(match kind { Kind::Chrome => "--incognito", Kind::Edge => "--inprivate" }.into());
            args.push("--new-window".into());
        }
    }
    // A profile's later pages open as tabs in its one window (the first opens the window), so an
    // email's sign-ins stay together instead of stacking windows.
    args.push(url.into());
    args
}

/// Opens a provider's sign-in page in `target`. A profile must be a session's folder that exists.
pub fn open(url: &str, target: &Target) -> Result<(), String> {
    if !allowed_url(url) {
        return Err("signin_url_rejected".into());
    }
    if let Target::Profile(dir) = target {
        if !session_folder(dir) {
            return Err("signin_profile_storage".into());
        }
    }
    let (kind, exe) = find_browser().ok_or("signin_browser_missing")?;
    let spawn_page = |url: &str, breakaway: bool| {
        let mut c = std::process::Command::new(&exe);
        c.args(browser_args(kind, target, url))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // A Claude sign-in's processes share a kill-on-close job (see `login::ProcessTree`) that
        // lets this one break away: the browser outlives the sign-in, so finishing it does not
        // close the user's window. Started outside such a job, it just starts.
        #[cfg(windows)]
        if breakaway {
            use std::os::windows::process::CommandExt;
            c.creation_flags(windows_sys::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB);
        }
        #[cfg(not(windows))]
        let _ = breakaway;
        c.spawn().map(|_| ())
    };
    let spawn = |url: &str| spawn_page(url, true).or_else(|_| spawn_page(url, false)).map_err(|_| "signin_browser_missing".to_string());
    if let Target::Profile(dir) = target {
        // A labelled profile's browser starts on the page naming the account; the sign-in page
        // then opens as the next tab, in front, once the browser is up to take it.
        let label = dir.join(LABEL_PAGE);
        if label.is_file() && !browser_open(dir) {
            if let Ok(page) = reqwest::Url::from_file_path(&label) {
                spawn(page.as_str())?;
                let until = std::time::Instant::now() + Duration::from_secs(15);
                while !browser_open(dir) && std::time::Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(250));
                }
                std::thread::sleep(Duration::from_millis(750));
            }
        }
    }
    spawn(url)?;
    if let Target::Profile(dir) = target {
        let _ = std::fs::write(dir.join(OPENED), millis(std::time::SystemTime::now()).to_string());
    }
    Ok(())
}

/// The environment that makes a Claude sign-in open its page in `target`: `BROWSER` names this
/// executable (by its short path when the long one has spaces, in case the CLI runs it through a
/// shell), and [`TARGET_ENV`] where to open it.
pub fn claude_browser_env(target: &Target) -> Option<[(OsString, OsString); 2]> {
    let exe = std::env::current_exe().ok()?;
    let exe = short_path(&exe).unwrap_or(exe);
    Some([("BROWSER".into(), exe.into_os_string()), (TARGET_ENV.into(), target.encode())])
}

#[cfg(windows)]
fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    if !path.to_string_lossy().contains(' ') {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut buf = vec![0u16; 1024];
    let n = unsafe { windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let short = PathBuf::from(OsString::from_wide(&buf[..n]));
    (!short.to_string_lossy().contains(' ')).then_some(short)
}

#[cfg(not(windows))]
fn short_path(_: &Path) -> Option<PathBuf> {
    None
}

/// `slopbar.exe <url>` started by a CLI through `BROWSER`, with [`TARGET_ENV`] set: opens the
/// page and returns the exit code, before any of the app starts. `None`: not such a start.
pub fn launch_from_env(args: &[OsString]) -> Option<i32> {
    let target = std::env::var_os(TARGET_ENV)?;
    if args.len() != 2 {
        return Some(2);
    }
    let Some(target) = Target::decode(&target) else { return Some(2) };
    let url = args[1].to_str()?;
    Some(if open(url, &target).is_ok() { 0 } else { 1 })
}

/// Profiles nothing is using under `legacy` (the per-email ones earlier versions kept) and
/// `sessions` (ones left by a SlopBar that ended mid-sign-in).
fn stale_in(legacy: Option<&Path>, sessions: Option<&Path>) -> Vec<PathBuf> {
    let dirs = |root: &Path| -> Vec<PathBuf> {
        // A root that is a link would lead the cleanup out of the app's folder.
        if std::fs::symlink_metadata(root).map_or(true, |m| m.file_type().is_symlink()) {
            return vec![];
        }
        std::fs::read_dir(root).map(|e| e.flatten().map(|e| e.path())
            .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())).collect())
            .unwrap_or_default()
    };
    let mut found: Vec<PathBuf> = legacy.map(dirs).unwrap_or_default();
    found.extend(sessions.map(dirs).unwrap_or_default().into_iter().filter(|d| !is_active(d)));
    found
}

fn cleanup_in(legacy: Option<&Path>, sessions: Option<&Path>) -> Result<usize, String> {
    let (mut removed, mut failed) = (0, 0);
    for dir in stale_in(legacy, sessions) {
        close_browser(&dir);
        if remove_profile(&dir).is_ok() { removed += 1 } else { failed += 1 }
    }
    if failed > 0 {
        return Err("signin_profiles_busy".into());
    }
    if let Some(legacy) = legacy {
        let _ = std::fs::remove_dir(legacy);
    }
    Ok(removed)
}

/// Closes and deletes every profile nothing is using: runs at startup, so the profiles kept per
/// email go on the first start of this version, and from Settings ("Forget sign-in profiles").
/// Returns how many were removed, or `signin_profiles_busy` when one could not be.
pub fn cleanup() -> Result<usize, String> {
    cleanup_in(legacy_root().as_deref(), sessions_root().as_deref())
}

/// How many profiles [`cleanup`] would remove, for Settings.
pub fn count() -> usize {
    stale_in(legacy_root().as_deref(), sessions_root().as_deref()).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::TempDir;

    /// A profile as a browser leaves it: cookies in a nested folder.
    fn fill(dir: &Path) -> PathBuf {
        let cookies = dir.join("Default").join("Network").join("Cookies");
        std::fs::create_dir_all(cookies.parent().unwrap()).unwrap();
        std::fs::write(&cookies, b"session=synthetic-cookie-for-a-test").unwrap();
        cookies
    }

    #[test]
    fn each_session_is_a_new_empty_profile_its_sign_ins_share() {
        let root = TempDir::new();
        let a = Session::create_in(&root.0).unwrap();
        let b = Session::create_in(&root.0).unwrap();
        assert_ne!(a.dir(), b.dir(), "the next email gets a profile of its own");
        for s in [&a, &b] {
            assert_eq!(s.dir().parent(), Some(root.0.as_path()));
            assert_eq!(std::fs::read_dir(s.dir()).unwrap().count(), 0, "nothing from an earlier sign-in");
            assert!(is_active(s.dir()));
        }
        assert_eq!(a.target(), a.target(), "an email's Claude and Codex sign-ins open in the same profile");
        assert_eq!(Target::choose(Window::Profile, Some(&a)), Some(a.target()));
        assert_eq!(Target::choose(Window::Profile, None), None, "no profile: the CLI's own browser");
        assert_eq!(Target::choose(Window::Private, Some(&a)), Some(Target::Private));
    }

    #[test]
    fn a_session_is_deleted_however_its_sign_ins_end() {
        let root = TempDir::new();
        // Finished: the email's last sign-in completed.
        let done = Session::create_in(&root.0).unwrap();
        let dir = done.dir().to_owned();
        fill(&dir);
        drop(done);
        assert!(!dir.exists() && !is_active(&dir));
        // Failed: a sign-in returned an error and the group ended early.
        let failed = |root: &Path| -> Result<PathBuf, (String, PathBuf)> {
            let s = Session::create_in(root).unwrap();
            fill(s.dir());
            Err(("monitor_wrong_account".to_string(), s.dir().to_owned()))
        };
        let (_, dir) = failed(&root.0).unwrap_err();
        assert!(!dir.exists());
        // Stopped: the running sign-in's future is dropped before it finishes.
        let s = Session::create_in(&root.0).unwrap();
        let dir = s.dir().to_owned();
        fill(&dir);
        let running = async move {
            let _s = s;
            std::future::pending::<()>().await;
        };
        drop(running);
        assert!(!dir.exists());
        // Shared by an email's sign-ins: only the last one lets go of it.
        let shared = std::sync::Arc::new(Session::create_in(&root.0).unwrap());
        let dir = shared.dir().to_owned();
        let step = shared.clone();
        drop(step);
        assert!(dir.exists(), "between an email's sign-ins the profile stays");
        drop(shared);
        assert!(!dir.exists());
        assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
    }

    #[test]
    fn a_profile_is_overwritten_before_it_is_deleted() {
        let root = TempDir::new();
        let cookies = fill(&root.0);
        let len = std::fs::metadata(&cookies).unwrap().len();
        scrub(&root.0);
        let left = std::fs::read(&cookies).unwrap();
        assert_eq!(left.len() as u64, len);
        assert!(left.iter().all(|b| *b == 0));
        remove_profile(&root.0).unwrap();
        assert!(!root.0.exists());
        remove_profile(&root.0).unwrap();
    }

    #[test]
    fn cleanup_removes_kept_and_left_profiles_but_never_a_running_one() {
        let data = TempDir::new();
        let (legacy, sessions) = (data.0.join("signin-profiles"), data.0.join("signin-sessions"));
        for name in ["0011223344556677", "8899aabbccddeeff"] {
            fill(&legacy.join(name));
        }
        fill(&sessions.join("left-by-a-stopped-slopbar"));
        let running = Session::create_in(&sessions).unwrap();
        fill(running.dir());
        assert_eq!(stale_in(Some(&legacy), Some(&sessions)).len(), 3);
        assert_eq!(cleanup_in(Some(&legacy), Some(&sessions)), Ok(3));
        assert!(!legacy.exists(), "the per-email profiles are gone with their folder");
        assert!(running.dir().exists(), "a sign-in still running keeps its profile");
        assert!(stale_in(Some(&legacy), Some(&sessions)).is_empty());
        assert_eq!(cleanup_in(Some(&legacy), Some(&sessions)), Ok(0), "running again finds nothing");
    }

    #[test]
    fn a_folder_without_a_browser_has_none_to_close() {
        let root = TempDir::new();
        assert!(!browser_open(&root.0));
        close_browser(&root.0);
        let session = Session::create_in(&root.0).unwrap();
        assert!(!session.browser_closed(), "a browser that never opened was not closed");
        session.label("synthetic\n@example.invalid", "en");
        let state: serde_json::Value = serde_json::from_slice(&std::fs::read(session.dir().join("Local State")).unwrap()).unwrap();
        assert_eq!(state["profile"]["info_cache"]["Default"]["name"], "synthetic@example.invalid");
        let page = std::fs::read_to_string(session.dir().join(LABEL_PAGE)).unwrap();
        assert!(page.contains("<title>synthetic@example.invalid"), "the tab names the account: {page}");
        session.label("other@example.invalid", "en");
        let again: serde_json::Value = serde_json::from_slice(&std::fs::read(session.dir().join("Local State")).unwrap()).unwrap();
        assert_eq!(again, state, "never rewritten once there (the browser owns it)");
        // Seen open for one sign-in, then closed: closed for that one, not for the next.
        session.opened.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(session.browser_closed());
        session.rearm();
        assert!(!session.browser_closed(), "the next sign-in opens the browser again");
    }

    #[test]
    fn the_label_page_names_the_account_in_every_language_and_escapes_it() {
        for lang in crate::i18n::languages() {
            let page = label_page("a<b>&\"c\"@example.invalid", lang, None, 0);
            assert!(page.contains("a&lt;b&gt;&amp;&quot;c&quot;@example.invalid"), "{lang}: {page}");
            assert!(!page.contains("<b>") && !page.contains("{account}"), "{lang}");
            assert!(page.contains(&format!("lang=\"{lang}\"")));
        }
        let root = TempDir::new();
        let session = Session::create_in(&root.0).unwrap();
        let before = std::time::SystemTime::now();
        assert!(!session.page_opened_since(before), "no page yet");
        std::fs::write(session.dir().join(OPENED), millis(std::time::SystemTime::now()).to_string()).unwrap();
        assert!(session.page_opened_since(before));
        assert!(!session.page_opened_since(std::time::SystemTime::now() + Duration::from_secs(60)), "an earlier step's page is not this one's");
    }

    #[test]
    fn the_label_page_shows_the_email_s_turn_as_it_goes() {
        let root = TempDir::new();
        let session = Session::create_in(&root.0).unwrap();
        let mut progress = Progress { steps: vec![("Claude".into(), PageStep::Done), ("Claude <monitoring>".into(), PageStep::Opening),
            ("Codex".into(), PageStep::Pending)], at: 5, total: 27 };
        session.show(&progress);
        assert!(!session.dir().join(PROGRESS_SCRIPT).exists(), "nothing before the page is labelled");
        session.label("synthetic@example.invalid", "en");
        let page = std::fs::read_to_string(session.dir().join(LABEL_PAGE)).unwrap();
        assert!(page.contains("Keep this window open") && !page.contains("<ol>"), "labelled, before any progress: {page}");
        session.show(&progress);
        let page = std::fs::read_to_string(session.dir().join(LABEL_PAGE)).unwrap();
        let title = format!("1/3 \u{b7} {}", crate::i18n::t("en", "signinPageTitle", &[("account", "synthetic@example.invalid")]));
        assert!(page.contains(&format!("<title>{title}</title>")), "{page}");
        assert!(page.contains("Sign-in 5 of 27") && page.contains("var seen=1,"), "{page}");
        assert!(page.contains("<li class=\"done\"><span class=\"mark\">&#10003;</span><span class=\"name\">Claude</span><span class=\"note\">done</span></li>"), "{page}");
        assert!(page.contains("<i class=\"spin\"></i></span><span class=\"name\">Claude &lt;monitoring&gt;</span><span class=\"note\">opening…</span>"), "{page}");
        progress.steps[1].1 = PageStep::Open;
        session.show(&progress);
        let script = std::fs::read_to_string(session.dir().join(PROGRESS_SCRIPT)).unwrap();
        let json: serde_json::Value = serde_json::from_str(script.trim().strip_prefix("slopbarProgress(").unwrap().strip_suffix(");").unwrap()).unwrap();
        assert_eq!(json["revision"], 2);
        assert_eq!(json["title"], title.as_str());
        assert!(json["html"].as_str().unwrap().contains("sign in as synthetic@example.invalid in the tab that opened"), "{json}");
        assert!(!session.dir().join("slopbar-progress.part").exists());
        for lang in crate::i18n::languages() {
            let page = label_page("a@example.invalid", lang, Some(&progress), 3);
            assert!(!page.contains("{account}") && !page.contains("{i}") && !page.contains("signinPage"), "{lang}: {page}");
        }
    }

    /// For rendering it headless: `SLOPBAR_PAGE_OUT=<dir> cargo test --lib write_a_progress_page -- --ignored`
    /// leaves a page drawn at "Claude done, Claude monitoring opening" whose sidecar has moved on
    /// to "Claude monitoring open", so a render a few seconds in shows the page redrew itself.
    #[test]
    #[ignore]
    fn write_a_progress_page() {
        let out = PathBuf::from(std::env::var_os("SLOPBAR_PAGE_OUT").expect("SLOPBAR_PAGE_OUT"));
        let lang = std::env::var("SLOPBAR_PAGE_LANG").unwrap_or_else(|_| "en".into());
        let session = Session::create_in(&out).unwrap();
        session.label("visual.one@example.invalid", &lang);
        let mut progress = Progress { steps: vec![("Claude".into(), PageStep::Done), ("Claude monitoring".into(), PageStep::Opening),
            ("Codex".into(), PageStep::Pending), ("Codex monitoring".into(), PageStep::Pending)], at: 6, total: 27 };
        session.show(&progress);
        let first = std::fs::read(session.dir().join(LABEL_PAGE)).unwrap();
        progress.steps[1].1 = PageStep::Open;
        session.show(&progress);
        std::fs::write(session.dir().join(LABEL_PAGE), first).unwrap();
        let page = session.dir().join(LABEL_PAGE);
        std::fs::write(out.join("page-path.txt"), page.to_string_lossy().as_bytes()).unwrap();
        // Kept for the render: not deleted as the session ends.
        std::mem::forget(session);
    }

    #[test]
    fn a_hard_link_is_deleted_but_never_overwritten() {
        let (profile, elsewhere) = (TempDir::new(), TempDir::new());
        let original = elsewhere.0.join("keep.txt");
        std::fs::write(&original, b"the user's own file").unwrap();
        std::fs::hard_link(&original, profile.0.join("link.txt")).unwrap();
        remove_profile(&profile.0).unwrap();
        assert!(!profile.0.exists());
        assert_eq!(std::fs::read(&original).unwrap(), b"the user's own file");
    }

    #[test]
    fn only_a_session_folder_opens() {
        let Some(root) = sessions_root() else { return };
        for dir in [root.join(".."), root.join("not-a-session"), root.join("0123456789abcdef0123456789abcdef").join("..")] {
            assert_eq!(open("https://claude.ai/oauth", &Target::Profile(dir.clone())), Err("signin_profile_storage".into()), "{}", dir.display());
        }
    }

    #[test]
    fn only_plain_addresses_are_prefilled() {
        assert_eq!(prefill_email(Some("first.last+tag@example.invalid")), Some("first.last+tag@example.invalid"));
        for bad in ["a@b", "a&calc@example.invalid", "a%PATH%@example.invalid", "\"a\"@example.invalid", "a b@example.invalid",
            "-x@example.invalid", "a@-example.invalid", "a@example.invalid^", "no-at-sign", "@example.invalid"] {
            assert_eq!(prefill_email(Some(bad)), None, "{bad}");
        }
        assert_eq!(prefill_email(None), None);
    }

    #[test]
    fn only_provider_pages_over_https_are_opened() {
        for ok in ["https://claude.ai/oauth/authorize?x=1", "https://claude.com/cai/oauth/authorize", "https://platform.claude.com/oauth/authorize",
            "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A1455"] {
            assert!(allowed_url(ok), "{ok}");
        }
        for bad in ["http://claude.ai/oauth", "https://claude.ai.example.invalid/", "https://evilclaude.ai/", "file:///C:/x", "https://u:p@claude.ai/",
            "https://claude.ai:8443/", "--user-data-dir=C:\\x", "javascript:alert(1)"] {
            assert!(!allowed_url(bad), "{bad}");
        }
    }

    #[test]
    fn browser_arguments_keep_the_url_last() {
        let dir = PathBuf::from(r"C:\data\signin-sessions\0011223344556677");
        let args = browser_args(Kind::Edge, &Target::Profile(dir), "https://claude.ai/oauth");
        assert_eq!(args.first().unwrap(), r"--user-data-dir=C:\data\signin-sessions\0011223344556677");
        assert_eq!(args.last().unwrap(), "https://claude.ai/oauth");
        assert!(!args.contains(&"--new-window".into()), "a profile's later pages open as tabs in its window");
        assert!(browser_args(Kind::Edge, &Target::Private, "https://claude.ai/").contains(&"--inprivate".into()));
        assert!(browser_args(Kind::Edge, &Target::Private, "https://claude.ai/").contains(&"--new-window".into()));
        assert!(browser_args(Kind::Chrome, &Target::Private, "https://claude.ai/").contains(&"--incognito".into()));
    }

    #[test]
    fn the_target_survives_the_environment() {
        let dir = PathBuf::from(r"C:\Users\Some One\AppData\Local\dev.thelio.slopbar\signin-sessions\0011223344556677");
        for t in [Target::Private, Target::Profile(dir)] {
            assert_eq!(Target::decode(&t.encode()), Some(t));
        }
        assert_eq!(Target::decode("C:\\elsewhere".as_ref()), None);
        assert_eq!(Target::choose(Window::System, None), None);
        assert_eq!(Target::choose(Window::Private, None), Some(Target::Private));
    }

    #[test]
    fn a_profile_outside_the_root_or_gone_is_refused() {
        let outside = Target::Profile(std::env::temp_dir().join("not-a-signin-profile"));
        assert_eq!(open("https://claude.ai/oauth", &outside), Err("signin_profile_storage".into()));
        if let Some(root) = sessions_root() {
            let deleted = Target::Profile(root.join("a-session-that-ended"));
            assert_eq!(open("https://claude.ai/oauth", &deleted), Err("signin_profile_storage".into()), "never recreated, so never reused");
        }
        assert_eq!(open("https://example.invalid/", &Target::Private), Err("signin_url_rejected".into()));
    }

    /// The real sign-in pages in fresh profiles, for `scripts/test-signin-window.ps1`, which
    /// captures each step on screen; nothing is ever typed or approved. Claude's page opens
    /// through `SLOPBAR_EXE` as its `BROWSER`, exactly as in the app; Codex's through its app
    /// server. Each step writes `<step>.ready` in `SLOPBAR_VISUAL_DIR` and waits for `<step>.go`.
    #[cfg(windows)]
    #[test]
    #[ignore = "opens browser windows; run by scripts/test-signin-window.ps1"]
    fn visual_sign_in_windows() {
        use crate::login::ProcessTree;
        use std::process::{Command, Stdio};
        let sync = PathBuf::from(std::env::var_os("SLOPBAR_VISUAL_DIR").expect("SLOPBAR_VISUAL_DIR"));
        let exe = PathBuf::from(std::env::var_os("SLOPBAR_EXE").expect("SLOPBAR_EXE"));
        assert!(std::env::var_os("SLOPBAR_DATA_DIR").is_some(), "keep the sessions out of the installed app's folder");
        let step = |name: &str, report: String| {
            std::fs::write(sync.join(format!("{name}.ready")), &report).unwrap();
            println!("{name}: {report}");
            let go = sync.join(format!("{name}.go"));
            let until = std::time::Instant::now() + Duration::from_secs(180);
            while !go.exists() && std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        let browser = |dir: &Path| {
            let until = std::time::Instant::now() + Duration::from_secs(45);
            loop {
                if let Some(pid) = browser_pid(dir) {
                    std::thread::sleep(Duration::from_secs(4));
                    return Some(pid);
                }
                if std::time::Instant::now() > until {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        };
        // Through cmd, as the app runs them: the CLIs may be npm .cmd shims. Each runs in a
        // kill-on-close tree, as in the app; only the page launcher breaks away from it.
        let claude = |session: &Session, email: &str, home: &Path| {
            let mut c = Command::new("cmd");
            c.args(["/c", "claude", "--setting-sources", "user", "auth", "login", "--email", email])
                .env("CLAUDE_CONFIG_DIR", home).envs(claude_browser_env_for(&exe, &session.target()));
            ProcessTree::spawn_with(&mut c, true, true).expect("claude")
        };

        let first = Session::create().expect("session");
        // As Sign in all accounts does: the window starts on a tab naming the account.
        first.label("visual.one@example.invalid", "en");
        let homes = [TempDir::new(), TempDir::new(), TempDir::new()];
        let claude_one = claude(&first, "visual.one@example.invalid", &homes[0].0);
        let pid = browser(first.dir());
        step("1-claude", format!("browser={pid:?} profile={}", first.dir().display()));

        std::fs::write(homes[1].0.join("config.toml"), "cli_auth_credentials_store = \"file\"\n").unwrap();
        let mut c = Command::new("cmd");
        c.args(["/c", "codex", "app-server"]).env("CODEX_HOME", &homes[1].0).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let (mut codex, codex_tree) = ProcessTree::spawn(&mut c, false).expect("codex");
        let (stdin, stdout) = (codex.stdin.take().unwrap(), codex.stdout.take().unwrap());
        let target = first.target();
        let started = crate::codex_signin::start(stdin, stdout, |url| open(url, &target));
        std::thread::sleep(Duration::from_secs(6));
        let again = browser_pid(first.dir());
        step("2-codex", format!("started={} browser={again:?} same_browser={}", started.is_ok(), again.is_some() && again == pid));

        drop(claude_one);
        drop(started);
        drop(codex_tree);
        let _ = codex.wait();
        let dir = first.dir().to_owned();
        drop(first);
        step("3-closed", format!("browser_open={} profile_exists={}", browser_open(&dir), dir.exists()));

        let second = Session::create().expect("session");
        let claude_two = claude(&second, "visual.two@example.invalid", &homes[2].0);
        let next = browser(second.dir());
        step("4-next", format!("browser={next:?} new_profile={} new_browser={}", second.dir() != dir, next.is_some() && next != pid));

        drop(claude_two);
        let dir = second.dir().to_owned();
        drop(second);
        step("5-closed", format!("browser_open={} profile_exists={}", browser_open(&dir), dir.exists()));
    }

    /// [`claude_browser_env`] with another launcher than this test binary.
    #[cfg(windows)]
    fn claude_browser_env_for(exe: &Path, target: &Target) -> [(OsString, OsString); 2] {
        [("BROWSER".into(), short_path(exe).unwrap_or(exe.to_owned()).into_os_string()), (TARGET_ENV.into(), target.encode())]
    }
}
