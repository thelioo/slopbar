//! User preferences, persisted as JSON in the app config directory.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::accounts::Provider;

use serde::{Deserialize, Serialize};

/// Must match `identifier` in tauri.conf.json.
const IDENTIFIER: &str = "dev.thelio.slopbar";
/// The data folder of releases published as Usage Bar, migrated on first run.
pub const LEGACY_IDENTIFIER: &str = "dev.thelio.usagebar";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Providers {
    pub claude: bool,
    pub codex: bool,
}

impl Default for Providers {
    fn default() -> Self {
        Self { claude: true, codex: true }
    }
}

/// Automatic switching for one provider.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub auto: bool,
    /// Switch when any usage window of the signed-in account reaches this percentage.
    pub threshold: u8,
}

impl Default for Rule {
    fn default() -> Self {
        Self { auto: false, threshold: 90 }
    }
}

/// Which Windows notifications to show.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// A limit at 75% and 90%, and reached.
    pub usage: bool,
    /// A limit that was reached is back, and new usage-limit resets.
    pub resets: bool,
    /// Automatic account switches.
    pub switches: bool,
    /// A monitored account used where none of the user's computers is (see `unexpected`). It
    /// can only fire while in-use sharing is on, so "on" here means on whenever sharing is.
    pub unexpected_use: bool,
    /// A plan that looks unneeded renews within three days; any paid plan within a day.
    pub renewals: bool,
    /// Monday mornings: last week's spend, savings, renewals and accounts that ran out.
    pub digest: bool,
    /// Claude extra usage reaching 75% and 90% of its monthly limit.
    pub extra_usage: bool,
    /// An account burning through its week abnormally fast (see `burn`), at most every few hours.
    pub burn: bool,
    /// Codex reset credits: spend one or switch when an account hits its limit, and one about to
    /// expire unused (see `reset_credits`).
    pub reset_credits: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self { usage: true, resets: true, switches: true, renewals: true, unexpected_use: true, digest: true, extra_usage: true, burn: true, reset_credits: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Balancer {
    pub claude: Rule,
    pub codex: Rule,
}

impl Balancer {
    pub fn rule(&self, provider: Provider) -> Rule {
        match provider {
            Provider::Claude => self.claude,
            Provider::Codex => self.codex,
        }
    }
}

/// The plan-renewal calendar file (see `calendar`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calendar {
    /// Rewrite it after each refresh.
    pub enabled: bool,
    /// Where it goes; Documents\SlopBar\renewals.ics when unset.
    pub path: Option<String>,
}

/// What the user told SlopBar about one account's paid plan (Settings > Subscriptions).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Subscription {
    /// Day of the month the plan renews, when the user knows it better than the token does.
    pub billing_day: Option<u8>,
    /// A billing date the user corrected; later renewals count on from it by the plan's period.
    pub next_billing: Option<chrono::NaiveDate>,
    /// What the user pays per month, overriding the list price.
    pub monthly_cost: Option<f64>,
    /// The user is letting the plan lapse: its last paid day. Afterwards automatic switching
    /// stops moving onto the account.
    pub cancel_after: Option<chrono::NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub refresh_minutes: u32,
    pub providers: Providers,
    /// "auto" follows the system language.
    pub language: String,
    pub launch_at_login: bool,
    /// Download and install new releases on their own.
    pub auto_update: bool,
    /// The version that last ran, to tell the user after a silent update.
    pub last_version: Option<String>,
    /// Display names for accounts, keyed by `provider:slot id`.
    pub aliases: HashMap<String, String>,
    pub balancer: Balancer,
    pub notifications: Notifications,
    /// Per-account plan details, keyed like `aliases`.
    pub subscriptions: HashMap<String, Subscription>,
    /// A folder the user syncs between machines (OneDrive, Dropbox…) where SlopBar shares which
    /// accounts are in use; off when unset (see `presence`).
    pub presence_folder: Option<String>,
    /// An rclone remote (name without the colon) to share in-use status through instead of a
    /// folder, for clouds without a sync app here; it takes precedence over `presence_folder`.
    pub presence_remote: Option<String>,
    /// Accounts the user also uses on the web or a phone, keyed like `aliases`: their use away
    /// from every computer is expected, so it is never reported as unexpected.
    pub web_use: HashMap<String, bool>,
    /// Claude accounts rotated off gracefully at their own usage percentage, keyed like `aliases`:
    /// running Claude Code sessions finish their task and save their state first (see
    /// `wind_down`), whether or not automatic switching is on for Claude.
    pub rotate_at: HashMap<String, u8>,
    /// How long a graceful rotation waits for sessions at most.
    pub rotate_grace_minutes: u16,
    /// Points the weekly window must rise between two readings to count as unexpected use.
    pub unexpected_min_rise: u8,
    /// Keep a redacted record of crashes (where in SlopBar's code, never the message) in the local
    /// diagnostics journal, so exports include them. Off unless the user turns it on.
    pub crash_records: bool,
    /// Folders whose CLI uses a given account instead of the default login (see `pins`).
    /// Changed through their own commands, which check the folder and the account.
    pub project_pins: Vec<ProjectPin>,
    /// The system-wide shortcut that opens the quick switcher, e.g. "Ctrl+Alt+S"; empty is off.
    pub switcher_hotkey: String,
    /// The system-wide shortcut that turns `compact_flyout` on and off, e.g. "Ctrl+Alt+C"; empty is off.
    pub compact_hotkey: String,
    /// Start the monitoring sign-in on its own for an account without one once it has been in use
    /// for a few minutes (see `auto_monitor`). On unless the user turns it off.
    pub auto_connect_monitoring: bool,
    pub calendar: Calendar,
    /// How spend totals are shown (see `ledger`).
    pub spend: SpendDisplay,
    /// What counts as abnormal burn (see `burn`).
    pub burn: Burn,
    /// Forwarding a few notifications to the user's phone (see `phone`). Off by default.
    pub phone: Phone,
    /// The flyout lists one line per account (provider mark, name, %, sparkline).
    pub compact_flyout: bool,
    /// The flyout offered the compact layout (it does above 8 accounts) and the user answered;
    /// it is not offered again.
    pub compact_suggested: bool,
    /// The first-run guide in Settings was closed; General opens it again.
    pub onboarding_dismissed: bool,
    /// The taskbar widget is drawn natively (Direct2D) instead of in a WebView2, which saves most
    /// of the app's idle memory. Off brings back the web widget, kept for one release.
    pub native_widget: bool,
    /// Where to look for CLI sign-ins in WSL (see `sources::WslScan`): by default only distros
    /// already running, plus stopped ones that held sign-ins before.
    pub wsl_accounts: crate::sources::WslScan,
    /// Where CLI sign-ins open their web page (see `signin`): a fresh browser profile per email,
    /// deleted after, unless the user picks a private window or the default browser.
    pub signin_window: crate::signin::Window,
    /// Share the list of accounts (provider, account id, email and plan; never sign-ins) with the
    /// user's other machines through the in-use sharing location, encrypted with a roster
    /// passphrase set on each machine (see `roster`), so a new machine knows every account.
    pub account_roster: bool,
}

/// Phone alerts through a push service (ntfy). The access token is not here: it is kept with
/// DPAPI in its own file (see `phone::store_token`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Phone {
    pub enabled: bool,
    /// An ntfy topic URL (see `phone::normalize_url`).
    pub url: Option<String>,
    /// Name the account (alias or email) in the text; otherwise only the provider.
    pub include_names: bool,
    pub kinds: PhoneKinds,
}

/// Which notifications go to the phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneKinds {
    /// A paid plan renewing within a day.
    pub renewal: bool,
    pub unexpected_use: bool,
    /// An agent burning usage unusually fast.
    pub runaway: bool,
    pub limit_reached: bool,
}

impl Default for PhoneKinds {
    fn default() -> Self {
        Self { renewal: true, unexpected_use: true, runaway: true, limit_reached: true }
    }
}

/// Thresholds for the runaway-agent alert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Burn {
    /// Weekly percentage points that, used within `minutes`, are abnormal.
    pub points: u8,
    pub minutes: u16,
    /// A repository and model on this PC using this many times its usual hourly tokens.
    pub token_factor: u8,
}

impl Default for Burn {
    fn default() -> Self { Self { points: 15, minutes: 60, token_factor: 3 } }
}

/// The values each `Burn` field accepts. Minutes stop at the three hours of readings kept in memory.
pub const BURN_POINTS: std::ops::RangeInclusive<u8> = 2..=50;
pub const BURN_MINUTES: std::ops::RangeInclusive<u16> = 15..=180;
pub const BURN_TOKEN_FACTOR: std::ops::RangeInclusive<u8> = 2..=20;

/// Display-only choices for the spend ledger: amounts stay US list prices unless the user
/// entered costs, whatever these say.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpendDisplay {
    /// Sales tax or VAT in percent, added to the totals shown.
    pub tax_percent: Option<f64>,
    /// A label for the amounts ("USD", "EUR").
    pub currency: Option<String>,
}

/// One folder pinned to one account of a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPin {
    pub folder: String,
    pub provider: Provider,
    pub id: String,
}

pub const DEFAULT_SWITCHER_HOTKEY: &str = "Ctrl+Alt+S";
pub const DEFAULT_COMPACT_HOTKEY: &str = "Ctrl+Alt+C";

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_minutes: 5,
            providers: Providers::default(),
            language: "auto".into(),
            launch_at_login: false,
            auto_update: cfg!(feature = "upstream-updates"),
            last_version: None,
            aliases: HashMap::new(),
            balancer: Balancer::default(),
            notifications: Notifications::default(),
            subscriptions: HashMap::new(),
            presence_folder: None,
            presence_remote: None,
            web_use: HashMap::new(),
            rotate_at: HashMap::new(),
            rotate_grace_minutes: 20,
            unexpected_min_rise: 2,
            crash_records: false,
            project_pins: Vec::new(),
            switcher_hotkey: DEFAULT_SWITCHER_HOTKEY.into(),
            compact_hotkey: DEFAULT_COMPACT_HOTKEY.into(),
            auto_connect_monitoring: true,
            calendar: Calendar::default(),
            spend: SpendDisplay::default(),
            burn: Burn::default(),
            phone: Phone::default(),
            compact_flyout: false,
            compact_suggested: false,
            onboarding_dismissed: false,
            native_widget: true,
            wsl_accounts: crate::sources::WslScan::Auto,
            signin_window: crate::signin::Window::Profile,
            account_roster: false,
        }
    }
}

/// The app's data folder (settings and the account vault): the same place as Tauri's app
/// config dir, but usable before the app is built, so settings are ready before any window
/// can ask for them.
pub fn app_dir() -> Option<PathBuf> {
    isolated_dir().or_else(|| dirs::config_dir().map(|d| d.join(IDENTIFIER)))
}

/// The app's machine-local data folder (`%LOCALAPPDATA%\dev.thelio.slopbar`), for what must not
/// roam with the Windows profile, such as the sign-in browser profiles.
pub fn local_dir() -> Option<PathBuf> {
    isolated_dir().map(|d| d.join("local")).or_else(|| dirs::data_local_dir().map(|d| d.join(IDENTIFIER)))
}

pub fn legacy_app_dir() -> Option<PathBuf> {
    if isolated() {
        return None;
    }
    dirs::config_dir().map(|d| d.join(LEGACY_IDENTIFIER))
}

/// Where `SLOPBAR_DATA_DIR` points, for running a build beside the installed app (to measure
/// it, say): app data of its own, and so its own door, instance lock and WebView2 folder.
fn isolated_dir() -> Option<PathBuf> {
    std::env::var_os("SLOPBAR_DATA_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Whether this process runs on `SLOPBAR_DATA_DIR`. It then leaves what belongs to the
/// installed app alone: launch at login and updates.
pub fn isolated() -> bool {
    isolated_dir().is_some()
}

fn path() -> Option<PathBuf> {
    app_dir().map(|d| d.join("settings.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let p = path().ok_or("no config dir")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
}

/// The refresh intervals the Settings window offers.
pub const REFRESH_CHOICES: [u32; 4] = [1, 5, 15, 30];
/// The interface languages (see `LANGUAGES` in i18n.js).
pub const LANGUAGES: [&str; 11] = ["auto", "en", "pt", "es", "fr", "de", "it", "ja", "zh", "ko", "ru"];
/// Settings an agent may change; `last_version` is the app's own bookkeeping.
/// `project_pins` has its own operations (see `pins`).
const PATCHABLE: [&str; 29] = ["rotate_at", "rotate_grace_minutes", "refresh_minutes", "providers", "language", "launch_at_login", "auto_update", "aliases", "balancer", "notifications", "subscriptions", "presence_folder", "presence_remote", "web_use", "unexpected_min_rise", "crash_records", "auto_connect_monitoring", "calendar", "switcher_hotkey", "compact_hotkey", "spend", "burn", "phone", "compact_flyout", "compact_suggested", "onboarding_dismissed", "native_widget", "wsl_accounts", "account_roster"];

/// Modifier names a shortcut may use, as the Settings window writes them.
const HOTKEY_MODIFIERS: [&str; 4] = ["Ctrl", "Alt", "Shift", "Win"];

/// A shortcut's canonical form ("Ctrl+Alt+S"): one or more modifiers, at least one of them Ctrl,
/// Alt or Win so typing never triggers it, then one key (a letter, digit, F1–F24 or Space).
/// Empty means off.
pub fn normalize_hotkey(text: &str) -> Result<String, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    let refuse = || format!("a shortcut must be modifiers ({}) and one key, like \"{DEFAULT_SWITCHER_HOTKEY}\", or empty for off (got {text:?})", HOTKEY_MODIFIERS.join(", "));
    let (key, modifiers) = parts.split_last().ok_or_else(refuse)?;
    let mut held = Vec::new();
    for m in modifiers {
        let name = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            "win" | "super" | "meta" => "Win",
            _ => return Err(refuse()),
        };
        if held.contains(&name) {
            return Err(refuse());
        }
        held.push(name);
    }
    if !held.iter().any(|m| *m != "Shift") {
        return Err(refuse());
    }
    let key = key.to_ascii_uppercase();
    let valid = match key.as_str() {
        "SPACE" => true,
        k if k.len() == 1 => k.chars().all(|c| c.is_ascii_alphanumeric()),
        k => k.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n)),
    };
    if !valid {
        return Err(refuse());
    }
    held.sort_by_key(|m| HOTKEY_MODIFIERS.iter().position(|x| x == m));
    let key = if key == "SPACE" { "Space".to_string() } else { key };
    Ok(format!("{}+{key}", held.join("+")))
}
/// The switcher's shortcut and the compact toggle's cannot be the same keys.
pub fn distinct_hotkeys(s: &Settings) -> Result<(), String> {
    if !s.compact_hotkey.is_empty() && s.compact_hotkey == s.switcher_hotkey {
        return Err(format!("compact_hotkey and switcher_hotkey are both {:?}; choose different shortcuts", s.compact_hotkey));
    }
    Ok(())
}

/// The values `unexpected_min_rise` accepts.
pub const UNEXPECTED_MIN_RISE: std::ops::RangeInclusive<u8> = 1..=20;

/// Whether `folder` can hold presence files: an existing absolute folder.
pub fn valid_presence_folder(folder: &str) -> bool {
    let path = std::path::Path::new(folder);
    folder.len() <= 1024 && !folder.chars().any(char::is_control) && path.is_absolute() && path.is_dir()
}

const SUBSCRIPTION_FIELDS: [&str; 4] = ["billing_day", "next_billing", "monthly_cost", "cancel_after"];

/// Applies an RFC 7386 merge patch from an agent to `current`. Only produces settings the
/// Settings window could: unknown or internal fields, wrong types and out-of-range values are
/// refused rather than ignored, so a typo never silently does nothing.
pub fn patched(current: &Settings, patch: &serde_json::Value, updates_enabled: bool) -> Result<Settings, String> {
    use serde_json::Value;
    let patch = patch.as_object().ok_or("the patch must be a JSON object")?;
    let mut merged = serde_json::to_value(current).map_err(|e| e.to_string())?;
    // Every patched key must already exist (aliases excepted, which are keyed by account).
    fn merge(target: &mut Value, patch: &serde_json::Map<String, Value>, path: &str, open: bool) -> Result<(), String> {
        let target = target.as_object_mut().ok_or_else(|| format!("{path} is not an object"))?;
        for (key, value) in patch {
            let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
            if !open && !target.contains_key(key) {
                return Err(format!("unknown setting: {here}"));
            }
            match value {
                Value::Null => { target.remove(key); }
                // An account's existing plan details merge field by field, like everything else.
                Value::Object(inner) if !open || target.get(key).is_some_and(Value::is_object) => merge(target.get_mut(key).unwrap(), inner, &here, ["aliases", "subscriptions", "web_use", "rotate_at"].contains(&here.as_str()))?,
                _ => { target.insert(key.clone(), value.clone()); }
            }
        }
        Ok(())
    }
    for key in patch.keys() {
        if !PATCHABLE.contains(&key.as_str()) {
            return Err(format!("unknown or read-only setting: {key}"));
        }
    }
    // A new account's plan details arrive whole, so check their fields here (the stored type
    // tolerates unknown fields, so that one stray field never resets every setting).
    for (key, plan) in patch.get("subscriptions").and_then(Value::as_object).into_iter().flatten() {
        if let Some(field) = plan.as_object().into_iter().flat_map(|o| o.keys()).find(|f| !SUBSCRIPTION_FIELDS.contains(&f.as_str())) {
            return Err(format!("unknown setting: subscriptions.{key}.{field}"));
        }
    }
    merge(&mut merged, patch, "", false)?;
    let mut next: Settings = serde_json::from_value(merged).map_err(|e| format!("invalid setting: {e}"))?;
    // Only values this patch changes are checked, so a stored value the window can no longer
    // produce (an old auto_update, a long alias) never blocks unrelated changes.
    if next.refresh_minutes != current.refresh_minutes && !REFRESH_CHOICES.contains(&next.refresh_minutes) {
        return Err(format!("refresh_minutes must be one of {REFRESH_CHOICES:?}"));
    }
    if next.language != current.language && !LANGUAGES.contains(&next.language.as_str()) {
        return Err(format!("language must be one of {LANGUAGES:?}"));
    }
    for (rule, before) in [(next.balancer.claude, current.balancer.claude), (next.balancer.codex, current.balancer.codex)] {
        if rule.threshold != before.threshold && (!(50..=95).contains(&rule.threshold) || rule.threshold % 5 != 0) {
            return Err("balancer thresholds must be 50–95 in steps of 5".into());
        }
    }
    if next.auto_update && !current.auto_update && !updates_enabled {
        return Err("this build uses manual updates; auto_update cannot be enabled".into());
    }
    for (key, alias) in next.aliases.iter_mut() {
        if current.aliases.get(key) == Some(alias) {
            continue;
        }
        *alias = alias.trim().to_string();
        let valid_key = key.split_once(':').is_some_and(|(p, id)| ["claude", "codex"].contains(&p) && !id.is_empty());
        let printable = !alias.chars().any(char::is_control);
        if !valid_key || alias.is_empty() || !printable || alias.chars().count() > 64 {
            return Err(format!("aliases are keyed \"provider:account id\" with a printable name of 1–64 characters (got {key:?})"));
        }
    }
    for (key, plan) in &next.subscriptions {
        if current.subscriptions.get(key) == Some(plan) {
            continue;
        }
        let valid_key = key.split_once(':').is_some_and(|(p, id)| ["claude", "codex"].contains(&p) && !id.is_empty());
        let day = plan.billing_day.map_or(true, |d| (1..=31).contains(&d));
        let cost = plan.monthly_cost.map_or(true, |c| c.is_finite() && (0.0..=10_000.0).contains(&c));
        if !valid_key || !day || !cost {
            return Err(format!("subscriptions are keyed \"provider:account id\" with billing_day 1–31, monthly_cost 0–10000 and cancel_after a YYYY-MM-DD date (got {key:?})"));
        }
    }
    if next.presence_folder != current.presence_folder {
        if let Some(folder) = &mut next.presence_folder {
            *folder = folder.trim().to_string();
            if !valid_presence_folder(folder) {
                return Err("presence_folder must be an existing absolute folder (null turns sharing off)".into());
            }
        }
    }
    if next.presence_remote != current.presence_remote {
        if let Some(remote) = &mut next.presence_remote {
            *remote = remote.trim().trim_end_matches(':').to_string();
            if !crate::rclone::valid_remote(remote) {
                return Err("presence_remote must be an rclone remote name from `rclone listremotes`, without the colon (null turns it off)".into());
            }
        }
    }
    for key in next.web_use.keys().filter(|k| !current.web_use.contains_key(*k)) {
        if !key.split_once(':').is_some_and(|(p, id)| ["claude", "codex"].contains(&p) && !id.is_empty()) {
            return Err(format!("web_use is keyed \"provider:account id\" with true or false (got {key:?})"));
        }
    }
    for (key, at) in next.rotate_at.iter().filter(|(k, v)| current.rotate_at.get(*k) != Some(*v)) {
        if !key.strip_prefix("claude:").is_some_and(|id| !id.is_empty()) || !(50..=95).contains(at) || at % 5 != 0 {
            return Err(format!("rotate_at is keyed \"claude:account id\" with 50–95 in steps of 5 (got {key:?})"));
        }
    }
    if next.rotate_grace_minutes != current.rotate_grace_minutes && !(5..=120).contains(&next.rotate_grace_minutes) {
        return Err("rotate_grace_minutes must be 5–120".into());
    }
    if next.unexpected_min_rise != current.unexpected_min_rise && !UNEXPECTED_MIN_RISE.contains(&next.unexpected_min_rise) {
        return Err(format!("unexpected_min_rise must be {}–{} points", UNEXPECTED_MIN_RISE.start(), UNEXPECTED_MIN_RISE.end()));
    }
    if next.burn != current.burn {
        let b = &next.burn;
        if !BURN_POINTS.contains(&b.points) || !BURN_MINUTES.contains(&b.minutes) || !BURN_TOKEN_FACTOR.contains(&b.token_factor) {
            return Err(format!("burn needs points {}–{}, minutes {}–{} and token_factor {}–{}", BURN_POINTS.start(), BURN_POINTS.end(),
                BURN_MINUTES.start(), BURN_MINUTES.end(), BURN_TOKEN_FACTOR.start(), BURN_TOKEN_FACTOR.end()));
        }
    }
    if next.calendar.path != current.calendar.path {
        if let Some(path) = &mut next.calendar.path {
            *path = path.trim().to_string();
            if !crate::calendar::valid_path(path) {
                return Err("calendar.path must be an absolute .ics file path in an existing folder (null uses Documents\\SlopBar\\renewals.ics)".into());
            }
        }
    }
    if next.phone.url != current.phone.url {
        if let Some(url) = &mut next.phone.url {
            *url = crate::phone::normalize_url(url)?;
        }
    }
    if next.phone.enabled && !current.phone.enabled && next.phone.url.is_none() {
        return Err("phone.enabled needs phone.url, an ntfy topic URL".into());
    }
    next.subscriptions.retain(|_, plan| *plan != Subscription::default());
    if next.switcher_hotkey != current.switcher_hotkey {
        next.switcher_hotkey = normalize_hotkey(&next.switcher_hotkey)?;
    }
    if next.compact_hotkey != current.compact_hotkey {
        next.compact_hotkey = normalize_hotkey(&next.compact_hotkey)?;
    }
    distinct_hotkeys(&next)?;
    next.web_use.retain(|_, on| *on);
    Ok(Settings { last_version: current.last_version.clone(), ..next })
}

#[cfg(test)]
mod patch_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_like_the_settings_window() {
        let mut current = Settings { last_version: Some("0.1.0".into()), ..Settings::default() };
        current.aliases.insert("claude:a".into(), "Work".into());
        let next = patched(&current, &json!({
            "refresh_minutes": 15,
            "balancer": { "claude": { "auto": true } },
            "notifications": { "usage": false },
            "aliases": { "claude:a": null, "codex:b": "Spare" },
        }), false).unwrap();
        assert_eq!(next.refresh_minutes, 15);
        assert!(next.balancer.claude.auto);
        assert_eq!(next.balancer.claude.threshold, 90, "untouched fields keep their values");
        assert!(!next.notifications.usage && next.notifications.resets && next.notifications.renewals);
        assert!(!patched(&next, &json!({ "notifications": { "renewals": false } }), false).unwrap().notifications.renewals);
        assert_eq!(next.aliases.get("codex:b").map(String::as_str), Some("Spare"));
        assert!(!next.aliases.contains_key("claude:a"), "null removes an alias");
        assert_eq!(next.last_version.as_deref(), Some("0.1.0"));
        let web = patched(&next, &json!({ "web_use": { "claude:a": true, "codex:b": false }, "unexpected_min_rise": 5, "notifications": { "unexpected_use": false } }), false).unwrap();
        assert_eq!(web.web_use.keys().collect::<Vec<_>>(), ["claude:a"], "false entries are dropped");
        assert_eq!(web.unexpected_min_rise, 5);
        assert!(!next.crash_records, "crash records are off by default");
        assert!(next.notifications.digest && !next.calendar.enabled, "the digest is on and the calendar off by default");
        let dir = crate::test_support::TempDir::new();
        let ics = dir.0.join("renewals.ics").display().to_string();
        let cal = patched(&next, &json!({ "calendar": { "enabled": true, "path": ics }, "notifications": { "digest": false } }), false).unwrap();
        assert!(cal.calendar.enabled && cal.calendar.path.as_deref() == Some(ics.as_str()) && !cal.notifications.digest);
        assert!(patched(&cal, &json!({ "calendar": { "path": null } }), false).unwrap().calendar.path.is_none(), "null goes back to the default");
        let crash = patched(&next, &json!({ "crash_records": true }), false).unwrap();
        assert!(crash.crash_records && crash.refresh_minutes == 15);
        assert!(patched(&crash, &json!({ "crash_records": "yes" }), false).is_err());
        assert!(next.auto_connect_monitoring, "monitoring connects automatically by default");
        assert!(!patched(&next, &json!({ "auto_connect_monitoring": false }), false).unwrap().auto_connect_monitoring);
        assert!(serde_json::from_str::<Settings>("{}").unwrap().auto_connect_monitoring, "older settings files turn it on");
        assert!(!web.notifications.unexpected_use && web.notifications.switches);
        assert!(patched(&web, &json!({ "web_use": { "claude:a": null } }), false).unwrap().web_use.is_empty());
        let planned = patched(&next, &json!({ "subscriptions": { "claude:a": { "billing_day": 3, "monthly_cost": 100, "cancel_after": "2026-11-02" } } }), false).unwrap();
        assert_eq!(planned.subscriptions["claude:a"], Subscription { billing_day: Some(3), monthly_cost: Some(100.0), cancel_after: chrono::NaiveDate::from_ymd_opt(2026, 11, 2), ..Default::default() });
        let corrected = patched(&planned, &json!({ "subscriptions": { "claude:a": { "next_billing": "2026-11-07" } } }), false).unwrap();
        assert_eq!(corrected.subscriptions["claude:a"].next_billing, chrono::NaiveDate::from_ymd_opt(2026, 11, 7));
        let cleared = patched(&planned, &json!({ "subscriptions": { "claude:a": { "billing_day": null, "monthly_cost": null, "cancel_after": null } } }), false).unwrap();
        assert!(cleared.subscriptions.is_empty(), "an emptied plan is dropped");
        let partial = patched(&planned, &json!({ "subscriptions": { "claude:a": { "billing_day": 9 } } }), false).unwrap();
        assert_eq!((partial.subscriptions["claude:a"].billing_day, partial.subscriptions["claude:a"].monthly_cost), (Some(9), Some(100.0)));
    }

    #[test]
    fn refuses_what_the_settings_window_cannot_produce() {
        let current = Settings::default();
        for (patch, why) in [
            (json!({ "refresh_minute": 5 }), "typo"),
            (json!({ "last_version": "9.9.9" }), "internal"),
            (json!({ "balancer": { "claude": { "treshold": 80 } } }), "nested typo"),
            (json!({ "refresh_minutes": 7 }), "interval not offered"),
            (json!({ "refresh_minutes": "5" }), "wrong type"),
            (json!({ "notifications": { "renewal": false } }), "notification typo"),
            (json!({ "notifications": { "renewals": "no" } }), "notification type"),
            (json!({ "language": "xx" }), "language"),
            (json!({ "balancer": { "codex": { "threshold": 97 } } }), "threshold range"),
            (json!({ "balancer": { "codex": { "threshold": 72 } } }), "threshold step"),
            (json!({ "auto_update": true }), "manual-update build"),
            (json!({ "aliases": { "gemini:x": "Name" } }), "alias provider"),
            (json!({ "aliases": { "claude:x": "  " } }), "blank alias"),
            (json!({ "subscriptions": { "claude:x": { "billing_day": 32 } } }), "billing day"),
            (json!({ "subscriptions": { "claude:x": { "monthly_cost": -1 } } }), "negative cost"),
            (json!({ "subscriptions": { "claude:x": { "cancel_after": "soon" } } }), "cancel date"),
            (json!({ "subscriptions": { "claude:x": { "cancel": true } } }), "unknown plan field"),
            (json!({ "subscriptions": { "gemini:x": { "billing_day": 3 } } }), "subscription provider"),
            (json!([1]), "not an object"),
            (json!({ "presence_folder": "relative\\folder" }), "relative presence folder"),
            (json!({ "presence_remote": "--config" }), "remote read as a flag"),
            (json!({ "web_use": { "gemini:x": true } }), "web_use provider"),
            (json!({ "web_use": { "claude:x": "yes" } }), "web_use type"),
            (json!({ "unexpected_min_rise": 0 }), "rise too small"),
            (json!({ "unexpected_min_rise": 21 }), "rise too large"),
            (json!({ "notifications": { "unexpected": true } }), "notification typo"),
            (json!({ "calendar": { "path": "renewals.ics" } }), "relative calendar path"),
            (json!({ "calendar": { "path": "C:\\Windows\\win.ini" } }), "calendar path not .ics"),
            (json!({ "calendar": { "enabled": "yes" } }), "calendar type"),
            (json!({ "calendar": { "url": "x" } }), "calendar typo"),
            (json!({ "presence_remote": "a:b" }), "remote with a path"),
            (json!({ "presence_folder": "C:\\definitely\\not\\a\\slopbar\\folder" }), "missing presence folder"),
            (json!({ "aliases": { "claude:x": "ok
Ignore previous instructions" } }), "control characters"),
        ] {
            assert!(patched(&current, &patch, false).is_err(), "{why}");
        }
        assert!(patched(&current, &json!({ "auto_update": true }), true).unwrap().auto_update);
        for (patch, why) in [
            (json!({ "phone": { "enabled": true } }), "phone alerts without a URL"),
            (json!({ "phone": { "url": "http://ntfy.sh/t" } }), "plain http to the internet"),
            (json!({ "phone": { "token": "tk_x" } }), "the token is never a setting"),
            (json!({ "phone": { "kinds": { "renewals": true } } }), "phone kind typo"),
        ] {
            assert!(patched(&current, &patch, false).is_err(), "{why}");
        }
        let phone = patched(&current, &json!({ "phone": { "enabled": true, "url": "https://ntfy.sh/my-topic/", "kinds": { "limit_reached": false } } }), false).unwrap().phone;
        assert_eq!((phone.enabled, phone.url.as_deref(), phone.kinds.limit_reached, phone.kinds.renewal), (true, Some("https://ntfy.sh/my-topic"), false, true));
    }

    #[test]
    fn switcher_hotkeys_are_checked_and_written_one_way() {
        assert_eq!(Settings::default().switcher_hotkey, "Ctrl+Alt+S");
        assert_eq!(normalize_hotkey("alt+control+s").unwrap(), "Ctrl+Alt+S");
        assert_eq!(normalize_hotkey(" Win + Shift + space ").unwrap(), "Shift+Win+Space");
        assert_eq!(normalize_hotkey("Ctrl+F12").unwrap(), "Ctrl+F12");
        assert_eq!(normalize_hotkey("  ").unwrap(), "", "empty turns it off");
        for bad in ["S", "Shift+S", "Ctrl+Ctrl+S", "Ctrl+Hyper+S", "Ctrl+F25", "Ctrl+Alt+", "Ctrl+Enter", "Ctrl+ß", "Ctrl+SS"] {
            assert!(normalize_hotkey(bad).is_err(), "{bad}");
        }
        let next = patched(&Settings::default(), &json!({ "switcher_hotkey": "ctrl+shift+k" }), false).unwrap();
        assert_eq!(next.switcher_hotkey, "Ctrl+Shift+K");
        assert_eq!(patched(&next, &json!({ "switcher_hotkey": "" }), false).unwrap().switcher_hotkey, "");
        assert!(patched(&Settings::default(), &json!({ "switcher_hotkey": "K" }), false).is_err());
        assert!(patched(&Settings::default(), &json!({ "project_pins": [] }), false).is_err(), "pins have their own operations");
        let stored: Settings = serde_json::from_value(json!({ "refresh_minutes": 15 })).unwrap();
        assert!(stored.project_pins.is_empty() && stored.switcher_hotkey == "Ctrl+Alt+S", "older settings get the defaults");
    }

    #[test]
    fn the_compact_toggle_has_its_own_shortcut() {
        assert_eq!(Settings::default().compact_hotkey, "Ctrl+Alt+C");
        let stored: Settings = serde_json::from_value(json!({ "refresh_minutes": 15 })).unwrap();
        assert_eq!(stored.compact_hotkey, "Ctrl+Alt+C", "older settings get the default");
        let next = patched(&Settings::default(), &json!({ "compact_hotkey": "shift+alt+f9" }), false).unwrap();
        assert_eq!(next.compact_hotkey, "Alt+Shift+F9");
        assert_eq!(patched(&next, &json!({ "compact_hotkey": "" }), false).unwrap().compact_hotkey, "", "empty turns it off");
        assert!(patched(&Settings::default(), &json!({ "compact_hotkey": "C" }), false).is_err());
        assert!(patched(&Settings::default(), &json!({ "compact_hotkey": "alt+ctrl+s" }), false).is_err(), "the switcher's keys");
        assert!(patched(&Settings::default(), &json!({ "switcher_hotkey": "Ctrl+Alt+C" }), false).is_err(), "the toggle's keys");
        let both_off = patched(&Settings::default(), &json!({ "switcher_hotkey": "", "compact_hotkey": "" }), false).unwrap();
        assert!(both_off.switcher_hotkey.is_empty() && both_off.compact_hotkey.is_empty());
    }

    #[test]
    fn a_plan_field_from_a_newer_version_does_not_reset_every_setting() {
        let stored: Settings = serde_json::from_value(json!({ "refresh_minutes": 15,
            "subscriptions": { "claude:a": { "billing_day": 3, "from_the_future": true } } })).unwrap();
        assert_eq!((stored.refresh_minutes, stored.subscriptions["claude:a"].billing_day), (15, Some(3)));
        let older: Settings = serde_json::from_value(json!({ "notifications": { "usage": false } })).unwrap();
        assert!(older.notifications.renewals, "settings saved before plan renewals existed turn them on");
    }

    #[test]
    fn stored_values_the_window_no_longer_offers_do_not_block_other_changes() {
        let mut current = Settings { auto_update: true, ..Settings::default() };
        current.aliases.insert("claude:a".into(), "x".repeat(80));
        let next = patched(&current, &json!({ "language": "en", "aliases": { "codex:b": "  Spare  " } }), false).unwrap();
        assert_eq!(next.language, "en");
        assert!(next.auto_update, "left as stored");
        assert_eq!(next.aliases["codex:b"], "Spare", "trimmed like the window");
    }
}
