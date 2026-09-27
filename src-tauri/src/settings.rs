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
}

impl Default for Notifications {
    fn default() -> Self {
        Self { usage: true, resets: true, switches: true }
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_minutes: 5,
            providers: Providers::default(),
            language: "auto".into(),
            launch_at_login: false,
            auto_update: true,
            last_version: None,
            aliases: HashMap::new(),
            balancer: Balancer::default(),
            notifications: Notifications::default(),
        }
    }
}

/// The app's data folder (settings and the account vault): the same place as Tauri's app
/// config dir, but usable before the app is built, so settings are ready before any window
/// can ask for them.
pub fn app_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(IDENTIFIER))
}

pub fn legacy_app_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(LEGACY_IDENTIFIER))
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
