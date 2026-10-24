//! Which account each provider's desktop app on this machine is signed in to, beside the CLI's.
//!
//! The CLIs and the desktop apps keep separate sign-ins on some providers, so "in use" can mean
//! two accounts on one machine. SlopBar reads only what each app publishes about itself, never a
//! token, cookie or session store:
//!
//! - **Claude Desktop** writes `lastKnownAccountUuid` into its `config.json`
//!   (`%APPDATA%\Claude`, or the packaged app's redirected copy). Only that key is read; the rest
//!   of the file (its encrypted token caches among it) is never looked at. A Claude slot id is
//!   `<account uuid>:<organization uuid>`, so the uuid names the slot.
//! - **The Codex app** runs the Codex CLI's own app server on `~/.codex`, so it shares the CLI's
//!   `auth.json`: it is signed in to whatever the CLI on Windows is. Switching the CLI switches
//!   it too, once the app reloads its sign-in (restarting it does).
//!
//! Neither app offers a supported way for another program to sign it in or out (Claude's
//! `claude://` links open chats and sessions only), so switching the desktop app is a guided
//! step: SlopBar opens the app, names the account to sign in with, and watches for it to change
//! (see `Guide`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::accounts::Provider;
use crate::model::{AccountUsage, SeatDrift};

/// How remote machines (through presence) and the UI name the desktop app as a place in use.
pub const PLACE: &str = "Desktop app";

/// What one provider's desktop app on this machine is signed in to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signed {
    /// A Claude account, by its account uuid (several organizations can share one).
    ClaudeAccount(String),
    /// The Codex app shares the CLI's sign-in on Windows.
    SharesCli,
}

/// Each provider's desktop app on this machine, when installed and readable.
#[derive(Debug, Clone, Default)]
pub struct Apps {
    pub claude: Option<Signed>,
    pub codex: Option<Signed>,
}

impl Apps {
    pub fn get(&self, provider: Provider) -> Option<&Signed> {
        match provider {
            Provider::Claude => self.claude.as_ref(),
            Provider::Codex => self.codex.as_ref(),
        }
    }
}

/// The desktop app's display name, for notices and the API.
pub fn app_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude Desktop",
        Provider::Codex => "Codex app",
    }
}

/// Where Claude Desktop keeps `config.json`: the classic install's folder and the packaged
/// (MSIX) app's redirected copy. The newest existing one is the live one.
fn claude_configs() -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(roaming) = std::env::var_os("APPDATA") {
        out.push(PathBuf::from(roaming).join("Claude").join("config.json"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let packages = PathBuf::from(local).join("Packages");
        if let Ok(entries) = std::fs::read_dir(&packages) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("Claude_") {
                    out.push(e.path().join("LocalCache").join("Roaming").join("Claude").join("config.json"));
                }
            }
        }
    }
    out
}

/// The account uuid Claude Desktop last signed in with, from its `config.json` text. Only the
/// one key is taken: every other value (the encrypted token caches among them) is skipped by the
/// parser, never kept.
pub fn claude_uuid_in(text: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Config {
        #[serde(rename = "lastKnownAccountUuid")]
        uuid: Option<String>,
    }
    let config: Config = serde_json::from_str(text).ok()?;
    let uuid = config.uuid?;
    let uuid = uuid.trim();
    (!uuid.is_empty() && uuid.len() <= 64 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| uuid.to_string())
}

fn read_claude() -> Option<Signed> {
    let newest = claude_configs()
        .into_iter()
        .filter_map(|p| Some((std::fs::metadata(&p).and_then(|m| m.modified()).ok()?, p)))
        .max_by_key(|(at, _)| *at)?;
    let text = std::fs::read_to_string(newest.1).ok()?;
    claude_uuid_in(&text).map(Signed::ClaudeAccount)
}

/// Whether the Codex desktop app is installed for this user (it unpacks its CLI here).
fn codex_installed() -> bool {
    let Some(local) = std::env::var_os("LOCALAPPDATA") else { return false };
    let local = PathBuf::from(local);
    if local.join("OpenAI").join("Codex").join("bin").is_dir() {
        return true;
    }
    std::fs::read_dir(local.join("Packages"))
        .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("OpenAI.Codex_")))
        .unwrap_or(false)
}

/// Reads both apps. Cheap: one small file and a few directory checks.
pub fn read() -> Apps {
    if !cfg!(windows) {
        return Apps::default();
    }
    // With CODEX_HOME set, the CLI SlopBar sees may not use the home the app does.
    let shares = std::env::var_os("CODEX_HOME").is_none_or(|h| h.is_empty());
    Apps { claude: read_claude(), codex: (shares && codex_installed()).then_some(Signed::SharesCli) }
}

/// The CLI on Windows itself (not WSL): the source the Codex app shares.
fn on_windows(a: &AccountUsage) -> bool {
    a.active && a.sources.iter().any(|s| !s.starts_with("WSL"))
}

/// Whether account `a` is the one `signed` names.
fn matches(a: &AccountUsage, signed: &Signed) -> bool {
    match signed {
        Signed::ClaudeAccount(uuid) => a.provider == Provider::Claude && a.id.split(':').next() == Some(uuid.as_str()),
        Signed::SharesCli => on_windows(a),
    }
}

fn label(a: &AccountUsage) -> String {
    a.alias.clone().or_else(|| a.email.clone()).unwrap_or_else(|| a.id.chars().take(8).collect())
}

/// Marks the account each desktop app is signed in to (`desktop`) and, when that is not the
/// account the CLI uses, the drift on both (`seats`). A desktop app signed in to an account
/// SlopBar does not have still flags the CLI's account, with no desktop label.
pub fn apply(list: &mut [AccountUsage], apps: &Apps) {
    for a in list.iter_mut() {
        a.desktop = false;
        a.seats = None;
    }
    for provider in Provider::ALL {
        let Some(signed) = apps.get(provider) else { continue };
        // Several organizations of one Claude account: the Windows CLI's one if it is among them,
        // else one in use in WSL, else any.
        let desktop = list.iter().position(|a| a.provider == provider && matches(a, signed) && on_windows(a))
            .or_else(|| list.iter().position(|a| a.provider == provider && matches(a, signed) && a.active))
            .or_else(|| list.iter().position(|a| a.provider == provider && matches(a, signed)));
        let cli = list.iter().position(|a| a.provider == provider && on_windows(a));
        if let Some(d) = desktop {
            list[d].desktop = true;
        }
        if desktop == cli || cli.is_none() {
            continue;
        }
        let drift = SeatDrift {
            app: app_name(provider).into(),
            cli: cli.map(|i| label(&list[i])),
            desktop: desktop.map(|i| label(&list[i])),
        };
        for i in [cli, desktop].into_iter().flatten() {
            list[i].seats = Some(drift.clone());
        }
    }
}

/// What "Use" did about the desktop app (in the switch result and the API).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum DesktopSwitch {
    /// "Use also switches the desktop app" is off.
    Off,
    /// No desktop app for this provider here.
    NotInstalled,
    /// It is already on that account.
    Already,
    /// It shares the CLI's sign-in, now switched; it uses the account once it reloads (restart).
    Shared,
    /// SlopBar opened it for the user to sign in with `expected`, and watches for that.
    Guided { expected: String },
}

/// Opens Claude Desktop through its documented `claude://` link (a new chat), the one supported
/// way another program brings it up. It cannot open its sign-in page: the user signs out from
/// its account menu and signs in again.
pub fn open_claude() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", "claude://claude.ai/new"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// How long a guided switch is watched for before SlopBar stops waiting.
pub const GUIDE_FOR: Duration = Duration::from_secs(15 * 60);
/// How often the watch rereads the app's config.
pub const GUIDE_EVERY: Duration = Duration::from_secs(5);

/// Waits (blocking) for Claude Desktop to report account `uuid`. True once it does, false when
/// `GUIDE_FOR` passes or `cancelled` says a newer switch took over.
pub fn wait_for_claude(uuid: &str, cancelled: &dyn Fn() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < GUIDE_FOR {
        if cancelled() {
            return false;
        }
        if read_claude() == Some(Signed::ClaudeAccount(uuid.to_string())) {
            return true;
        }
        std::thread::sleep(GUIDE_EVERY);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(provider: Provider, id: &str, email: &str, sources: &[&str]) -> AccountUsage {
        let mut a: AccountUsage = serde_json::from_value(serde_json::json!({
            "id": id, "provider": provider, "email": email, "alias": null, "plan": null, "org": null,
            "active": !sources.is_empty(), "sources": sources, "windows": [], "credits": null, "resets": [],
            "error": null, "stale": false, "fetched_at": null, "monitor_connected": false,
        })).unwrap();
        a.active = !sources.is_empty();
        a
    }

    #[test]
    fn reads_only_the_account_uuid() {
        assert_eq!(claude_uuid_in(r#"{"oauth:tokenCache":"x","lastKnownAccountUuid":"ab-12"}"#).as_deref(), Some("ab-12"));
        assert_eq!(claude_uuid_in(r#"{"lastKnownAccountUuid":""}"#), None);
        assert_eq!(claude_uuid_in(r#"{"lastKnownAccountUuid":"not a uuid!"}"#), None);
        assert_eq!(claude_uuid_in("{broken"), None);
    }

    #[test]
    fn same_account_marks_desktop_without_drift() {
        let mut list = vec![account(Provider::Claude, "u1:o1", "a@example.invalid", &["Windows"]), account(Provider::Claude, "u2:o2", "b@example.invalid", &[])];
        apply(&mut list, &Apps { claude: Some(Signed::ClaudeAccount("u1".into())), codex: None });
        assert!(list[0].desktop && list[0].seats.is_none());
        assert!(!list[1].desktop);
    }

    #[test]
    fn different_accounts_flag_drift_on_both() {
        let mut list = vec![account(Provider::Claude, "u1:o1", "a@example.invalid", &["Windows"]), account(Provider::Claude, "u2:o2", "b@example.invalid", &[])];
        apply(&mut list, &Apps { claude: Some(Signed::ClaudeAccount("u2".into())), codex: None });
        assert!(list[1].desktop && !list[0].desktop);
        let d = list[0].seats.clone().unwrap();
        assert_eq!((d.cli.as_deref(), d.desktop.as_deref()), (Some("a@example.invalid"), Some("b@example.invalid")));
        assert_eq!(list[1].seats, list[0].seats);
    }

    #[test]
    fn unknown_desktop_account_flags_the_cli_account() {
        let mut list = vec![account(Provider::Claude, "u1:o1", "a@example.invalid", &["Windows"])];
        apply(&mut list, &Apps { claude: Some(Signed::ClaudeAccount("zz".into())), codex: None });
        assert_eq!(list[0].seats.as_ref().unwrap().desktop, None);
    }

    #[test]
    fn codex_app_follows_the_windows_cli() {
        let mut list = vec![account(Provider::Codex, "c1", "a@example.invalid", &["Windows"]), account(Provider::Codex, "c2", "b@example.invalid", &["WSL: Ubuntu"])];
        apply(&mut list, &Apps { claude: None, codex: Some(Signed::SharesCli) });
        assert!(list[0].desktop && !list[1].desktop);
        assert!(list.iter().all(|a| a.seats.is_none()));
    }

    #[test]
    fn a_wsl_organization_does_not_hide_the_windows_one() {
        let mut list = vec![account(Provider::Claude, "u1:o1", "a@example.invalid", &["WSL: Ubuntu"]), account(Provider::Claude, "u1:o2", "a@example.invalid", &["Windows"])];
        apply(&mut list, &Apps { claude: Some(Signed::ClaudeAccount("u1".into())), codex: None });
        assert!(list[1].desktop && !list[0].desktop);
        assert!(list.iter().all(|a| a.seats.is_none()));
    }

    #[test]
    fn prefers_the_cli_organization_of_a_shared_account() {
        let mut list = vec![account(Provider::Claude, "u1:o1", "a@example.invalid", &[]), account(Provider::Claude, "u1:o2", "a@example.invalid", &["Windows"])];
        apply(&mut list, &Apps { claude: Some(Signed::ClaudeAccount("u1".into())), codex: None });
        assert!(list[1].desktop && !list[0].desktop && list[1].seats.is_none());
    }
}
