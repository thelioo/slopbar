//! User preferences, persisted as JSON in the app config directory.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::accounts::Provider;

use serde::{Deserialize, Serialize};

/// Must match `identifier` in tauri.conf.json.
const IDENTIFIER: &str = "dev.thelio.usagebar";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// Dynamic Island hanging from the top edge of one monitor.
    Island,
    /// A widget laid over the taskbar, next to the notification area.
    Taskbar,
    /// Tray icon only; clicking it opens a panel above the taskbar.
    Tray,
}

/// Where the island sits on its monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Hanging from the top edge, like a notch.
    Top,
    Left,
    Right,
}

impl Anchor {
    pub const ALL: [Anchor; 3] = [Anchor::Top, Anchor::Left, Anchor::Right];
}

/// How providers are told apart in the collapsed island and the taskbar widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactStyle {
    /// "Claude 30%".
    Names,
    /// Just the usage ring with a small provider mark inside; no name, no percentage.
    #[serde(alias = "icons")]
    Minimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandOn {
    Hover,
    Click,
}

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
    pub mode: DisplayMode,
    /// Monitor name as reported by the OS; `None` follows the primary monitor.
    pub monitor: Option<String>,
    #[serde(deserialize_with = "lenient_anchor")]
    pub anchor: Anchor,
    pub expand_on: ExpandOn,
    pub compact_style: CompactStyle,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: DisplayMode::Island,
            monitor: None,
            anchor: Anchor::Top,
            expand_on: ExpandOn::Hover,
            compact_style: CompactStyle::Names,
            refresh_minutes: 5,
            providers: Providers::default(),
            language: "auto".into(),
            launch_at_login: false,
            auto_update: true,
            last_version: None,
            aliases: HashMap::new(),
            balancer: Balancer::default(),
        }
    }
}

/// Same place as Tauri's app config dir, but usable before the app is built,
/// so settings are ready before any window can ask for them.
/// Anchors that no longer exist (older versions had corners and a bottom) fall back to the top.
fn lenient_anchor<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Anchor, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or(Anchor::Top))
}

/// The app's data folder (settings and the account vault).
pub fn app_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(IDENTIFIER))
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
