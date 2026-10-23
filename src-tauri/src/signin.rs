//! Where a CLI sign-in's web page opens. By default each email gets a browser profile of its own
//! (`signin-profiles\<hash of the email>` in the app's local data folder) so its provider and
//! Google cookies persist: the first sign-in for an email asks for Google once, later ones for the
//! same email (the other provider, a monitoring sign-in, a re-sign-in on this machine) take one
//! click. The user completes every page; SlopBar only chooses the window.
//!
//! The CLIs open their page themselves, so SlopBar steers them: Claude Code runs `BROWSER` with
//! the URL on Windows, so `BROWSER` points at `slopbar.exe` with [`TARGET_ENV`] saying where to
//! open it ([`launch_from_env`] handles that start before anything else). Codex ignores `BROWSER`
//! on Windows; its app server hands the URL back instead (see `accounts`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Set on a CLI sign-in that `BROWSER` points at SlopBar: `profile:<folder>` or `private`.
pub const TARGET_ENV: &str = "SLOPBAR_SIGNIN_TARGET";

/// The user's choice of window (Settings > Accounts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Window {
    /// A browser profile per email, kept between sign-ins.
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
    /// The window for a sign-in as `email`. A profile needs the email (it is per email); without
    /// one, or with `System`, the CLI opens its page as it always did.
    pub fn choose(window: Window, email: Option<&str>) -> Option<Self> {
        match window {
            Window::System => None,
            Window::Private => Some(Self::Private),
            Window::Profile => Some(Self::Profile(profile_dir(&profiles_root()?, email?)?)),
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

/// All sign-in profiles: they hold provider and Google cookies, so they live outside the roaming
/// profile and never in a backup. "Forget sign-in profiles" deletes this folder.
pub fn profiles_root() -> Option<PathBuf> {
    Some(crate::settings::local_dir()?.join("signin-profiles"))
}

/// The profile folder for an email. The folder is named by a hash, so the email itself is not
/// written to the disk in the folder's name; the same email always finds the same profile.
pub fn profile_dir(root: &Path, email: &str) -> Option<PathBuf> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return None;
    }
    let hash = Sha256::digest(email.as_bytes());
    let name: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    Some(root.join(name))
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

/// The browser's arguments for one page. The URL goes last and is an https URL, so it can never
/// be read as a switch.
pub fn browser_args(kind: Kind, target: &Target, url: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    match target {
        Target::Profile(dir) => {
            let mut a = OsString::from("--user-data-dir=");
            a.push(dir);
            args.push(a);
            args.push("--no-first-run".into());
            args.push("--no-default-browser-check".into());
        }
        Target::Private => args.push(match kind { Kind::Chrome => "--incognito", Kind::Edge => "--inprivate" }.into()),
    }
    args.push("--new-window".into());
    args.push(url.into());
    args
}

/// Opens a provider's sign-in page in `target`. A profile folder must be one of ours.
pub fn open(url: &str, target: &Target) -> Result<(), String> {
    if !allowed_url(url) {
        return Err("signin_url_rejected".into());
    }
    if let Target::Profile(dir) = target {
        let root = profiles_root().ok_or("signin_profile_storage")?;
        if dir.parent() != Some(root.as_path()) {
            return Err("signin_profile_storage".into());
        }
        std::fs::create_dir_all(dir).map_err(|_| "signin_profile_storage")?;
    }
    let (kind, exe) = find_browser().ok_or("signin_browser_missing")?;
    let spawn = |breakaway: bool| {
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
    spawn(true).or_else(|_| spawn(false)).map_err(|_| "signin_browser_missing".into())
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

/// Deletes every sign-in profile. A profile whose browser is still open cannot be deleted; the
/// error then asks to close those windows first. Returns how many were removed.
pub fn forget() -> Result<usize, String> {
    let Some(root) = profiles_root() else { return Ok(0) };
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err("signin_profiles_busy".into()),
    };
    let (mut removed, mut failed) = (0, 0);
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        let result = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        if result.is_ok() { removed += 1 } else { failed += 1 }
    }
    if failed > 0 {
        return Err("signin_profiles_busy".into());
    }
    let _ = std::fs::remove_dir(&root);
    Ok(removed)
}

/// How many sign-in profiles exist, for Settings.
pub fn count() -> usize {
    profiles_root().and_then(|r| std::fs::read_dir(r).ok()).map(|e| e.flatten().filter(|e| e.path().is_dir()).count()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_profile_per_email_named_by_a_hash() {
        let root = Path::new(r"C:\data\signin-profiles");
        let a = profile_dir(root, "Person.One@Example.Invalid").unwrap();
        assert_eq!(a, profile_dir(root, " person.one@example.invalid ").unwrap(), "case and spaces do not split a profile");
        assert_ne!(a, profile_dir(root, "person.two@example.invalid").unwrap());
        assert_eq!(a.parent(), Some(root));
        let name = a.file_name().unwrap().to_string_lossy();
        assert_eq!(name.len(), 16);
        assert!(!name.contains("person") && name.bytes().all(|c| c.is_ascii_hexdigit()));
        assert!(profile_dir(root, "  ").is_none());
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
        let dir = PathBuf::from(r"C:\data\signin-profiles\0011223344556677");
        let args = browser_args(Kind::Edge, &Target::Profile(dir), "https://claude.ai/oauth");
        assert_eq!(args.first().unwrap(), r"--user-data-dir=C:\data\signin-profiles\0011223344556677");
        assert_eq!(args.last().unwrap(), "https://claude.ai/oauth");
        assert!(browser_args(Kind::Edge, &Target::Private, "https://claude.ai/").contains(&"--inprivate".into()));
        assert!(browser_args(Kind::Chrome, &Target::Private, "https://claude.ai/").contains(&"--incognito".into()));
    }

    #[test]
    fn the_target_survives_the_environment() {
        let dir = PathBuf::from(r"C:\Users\Some One\AppData\Local\dev.thelio.slopbar\signin-profiles\0011223344556677");
        for t in [Target::Private, Target::Profile(dir)] {
            assert_eq!(Target::decode(&t.encode()), Some(t));
        }
        assert_eq!(Target::decode("C:\\elsewhere".as_ref()), None);
        assert_eq!(Target::choose(Window::System, Some("a@example.invalid")), None);
        assert_eq!(Target::choose(Window::Private, None), Some(Target::Private));
    }

    #[test]
    fn a_profile_outside_the_root_is_refused() {
        let outside = Target::Profile(std::env::temp_dir().join("not-a-signin-profile"));
        assert_eq!(open("https://claude.ai/oauth", &outside), Err("signin_profile_storage".into()));
        assert_eq!(open("https://example.invalid/", &Target::Private), Err("signin_url_rejected".into()));
    }
}
