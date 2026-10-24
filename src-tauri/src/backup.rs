//! Backup and restore: settings (aliases, plan details, pins and every preference), the weekly
//! usage history and the spend ledger in one file, encrypted with a passphrase the user chooses.
//!
//! Sign-ins never go in: no saved logins from the vault, no monitoring sessions, no phone-alert
//! token. After restoring, each account is reconnected (Add account, then Connect monitoring);
//! its restored names, plans and history apply again as soon as it is, because accounts are
//! matched by provider and account id, which a new sign-in of the same account reproduces.
//!
//! The file is JSON: a header naming the format, the KDF (Argon2id) with its parameters and salt,
//! and the cipher (XChaCha20-Poly1305), then the ciphertext. The header is the AEAD's associated
//! data, so changing any byte of it or of the ciphertext makes the file refuse to open.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::ledger::Ledger;
use crate::settings::{ProjectPin, Settings};
use crate::subscriptions::History;

pub const FORMAT: &str = "slopbar-backup";
pub const EXTENSION: &str = "slopbar-backup";
const VERSION: u32 = 1;
const CIPHER: &str = "xchacha20poly1305";
const KDF: &str = "argon2id";
pub const MIN_PASSPHRASE: usize = 8;
const MAX_PASSPHRASE: usize = 1024;
/// Argon2id at OWASP's first recommended setting: 64 MiB, three passes, one lane.
const MEMORY_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const PARALLELISM: u32 = 1;
/// What a file may ask for: enough to open one from a future, costlier version, never so much
/// that a crafted file can exhaust memory.
const MAX_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_ITERATIONS: u32 = 16;
const MAX_FILE: u64 = 32 << 20;
pub const NOTE: &str = "Backups never contain sign-ins: reconnect each account (Add account, then Connect monitoring) after restoring. Restored names, plans and history apply to an account as soon as it is reconnected.";

/// A passphrase the user typed. Never printed, even in debug output.
#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Passphrase(pub String);

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Passphrase(..)") }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    kdf: Kdf,
    cipher: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kdf {
    algorithm: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
}

/// What a backup holds once opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contents {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub app_version: String,
    pub settings: Settings,
    pub usage_history: History,
    pub spend_ledger: Ledger,
    /// The accounts known when it was made, to name them in a preview. No sign-in data.
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
}

impl Contents {
    /// Collects a backup. `accounts` is built field by field from the vault's slots so their
    /// login data can never reach the file.
    pub fn new(settings: &Settings, history: &History, ledger: &Ledger, accounts: impl IntoIterator<Item = Account>, app_version: &str, now: DateTime<Utc>) -> Self {
        Self {
            version: VERSION,
            created_at: now,
            app_version: app_version.into(),
            settings: Settings { last_version: None, ..settings.clone() },
            usage_history: history.clone(),
            spend_ledger: ledger.clone(),
            accounts: accounts.into_iter().collect(),
        }
    }
}

fn check_passphrase(passphrase: &Passphrase) -> Result<(), String> {
    let n = passphrase.0.chars().count();
    if n < MIN_PASSPHRASE || n > MAX_PASSPHRASE { return Err("backup_passphrase_too_short".into()); }
    Ok(())
}

fn b64() -> base64::engine::GeneralPurpose { base64::engine::general_purpose::STANDARD }

fn key(passphrase: &Passphrase, kdf: &Kdf, salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(32)).map_err(|_| "backup_unreadable")?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.0.as_bytes(), salt, &mut key)
        .map_err(|_| "backup_unreadable")?;
    Ok(key)
}

/// Everything in the header, in a fixed order: the associated data.
fn associated(e: &Envelope) -> Vec<u8> {
    format!("{}|{}|{}|{}|{}|{}|{}|{}|{}", e.format, e.version, e.kdf.algorithm, e.kdf.memory_kib, e.kdf.iterations, e.kdf.parallelism, e.kdf.salt, e.cipher, e.nonce).into_bytes()
}

/// A key derived from a passphrase with a fresh random salt, at this version's Argon2id cost.
/// The roster (see `roster`) keeps one per run, so it derives once rather than at every write.
#[derive(Clone)]
pub struct SealKey {
    salt: [u8; 16],
    key: [u8; 32],
}

impl std::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("SealKey(..)") }
}

impl SealKey {
    pub fn new(passphrase: &Passphrase) -> Result<Self, String> {
        check_passphrase(passphrase)?;
        let mut salt = [0u8; 16];
        chacha20poly1305::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut salt);
        let kdf = Kdf { algorithm: KDF.into(), memory_kib: MEMORY_KIB, iterations: ITERATIONS, parallelism: PARALLELISM, salt: String::new() };
        Ok(SealKey { salt, key: key(passphrase, &kdf, &salt)? })
    }
}

/// The key a sealed file asks for: its KDF settings and salt.
pub(crate) struct KeyRequest<'a> {
    kdf: &'a Kdf,
    salt: &'a [u8],
}

impl KeyRequest<'_> {
    /// Names the settings and salt, so a caller can keep keys it already derived.
    pub fn id(&self) -> String {
        format!("{}|{}|{}|{}", self.kdf.memory_kib, self.kdf.iterations, self.kdf.parallelism, self.kdf.salt)
    }

    pub fn derive(&self, passphrase: &Passphrase) -> Result<[u8; 32], String> {
        key(passphrase, self.kdf, self.salt)
    }
}

/// Why a sealed file did not open. Each caller names these in its own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// Not this envelope, or another format.
    NotOurs,
    /// A version, cipher, KDF or KDF cost this build does not open.
    Unsupported,
    /// The passphrase is wrong, or the file changed after it was written.
    WrongOrDamaged,
}

/// Encrypts `plain` into an envelope of `format`.
pub(crate) fn seal_bytes(format: &str, plain: &[u8], key: &SealKey) -> Result<Vec<u8>, String> {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut envelope = Envelope {
        format: format.into(),
        version: VERSION,
        kdf: Kdf { algorithm: KDF.into(), memory_kib: MEMORY_KIB, iterations: ITERATIONS, parallelism: PARALLELISM, salt: b64().encode(key.salt) },
        cipher: CIPHER.into(),
        nonce: b64().encode(nonce),
        ciphertext: String::new(),
    };
    let aad = associated(&envelope);
    let sealed = XChaCha20Poly1305::new((&key.key).into())
        .encrypt(&nonce, Payload { msg: plain, aad: &aad })
        .map_err(|_| "backup_unwritable")?;
    envelope.ciphertext = b64().encode(sealed);
    serde_json::to_vec_pretty(&envelope).map_err(|_| "backup_unwritable".into())
}

/// Opens an envelope of `format` at most `max` bytes long; `key` supplies the key it asks for.
pub(crate) fn open_bytes(format: &str, bytes: &[u8], max: u64, key: &mut dyn FnMut(&KeyRequest) -> Result<[u8; 32], String>) -> Result<Vec<u8>, OpenError> {
    if bytes.len() as u64 > max { return Err(OpenError::NotOurs); }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| OpenError::NotOurs)?;
    if envelope.format != format { return Err(OpenError::NotOurs); }
    if envelope.version != VERSION || envelope.cipher != CIPHER || envelope.kdf.algorithm != KDF {
        return Err(OpenError::Unsupported);
    }
    let k = &envelope.kdf;
    if k.memory_kib > MAX_MEMORY_KIB || k.iterations > MAX_ITERATIONS || !(1..=8).contains(&k.parallelism) {
        return Err(OpenError::Unsupported);
    }
    let salt = b64().decode(&k.salt).map_err(|_| OpenError::NotOurs)?;
    let nonce = b64().decode(&envelope.nonce).ok().filter(|n| n.len() == 24).ok_or(OpenError::NotOurs)?;
    let sealed = b64().decode(&envelope.ciphertext).map_err(|_| OpenError::NotOurs)?;
    let key = key(&KeyRequest { kdf: k, salt: &salt }).map_err(|_| OpenError::WrongOrDamaged)?;
    XChaCha20Poly1305::new((&key).into())
        .decrypt(XNonce::from_slice(&nonce), Payload { msg: &sealed, aad: &associated(&envelope) })
        .map_err(|_| OpenError::WrongOrDamaged)
}

/// Encrypts a backup into the file's bytes.
pub fn seal(contents: &Contents, passphrase: &Passphrase) -> Result<Vec<u8>, String> {
    let key = SealKey::new(passphrase)?;
    let plain = serde_json::to_vec(contents).map_err(|_| "backup_unwritable")?;
    seal_bytes(FORMAT, &plain, &key)
}

/// Opens a backup: checks the format and version, then decrypts. A wrong passphrase and a file
/// changed after it was written fail alike (the AEAD cannot tell them apart).
pub fn open(bytes: &[u8], passphrase: &Passphrase) -> Result<Contents, String> {
    let plain = open_bytes(FORMAT, bytes, MAX_FILE, &mut |request| request.derive(passphrase)).map_err(|e| match e {
        OpenError::NotOurs => "backup_not_a_backup",
        OpenError::Unsupported => "backup_version_unsupported",
        OpenError::WrongOrDamaged => "backup_wrong_passphrase_or_damaged",
    })?;
    let contents: Contents = serde_json::from_slice(&plain).map_err(|_| "backup_version_unsupported")?;
    if contents.version != VERSION
        || contents.usage_history.version != crate::subscriptions::VERSION
        || contents.spend_ledger.version != crate::ledger::VERSION {
        return Err("backup_version_unsupported".into());
    }
    Ok(contents)
}

/// Where an export goes: `path` (a file, or an existing folder to put a dated file in), else
/// Documents\SlopBar Exports. Always a `.slopbar-backup` file, so an export never replaces
/// anything else.
pub fn target(path: Option<&str>, default_folder: Option<PathBuf>, stamp: &str) -> Result<PathBuf, String> {
    let name = format!("SlopBar-Backup-{stamp}.{EXTENSION}");
    let path = match path.map(str::trim).filter(|p| !p.is_empty()) {
        None => default_folder.ok_or("backup_unwritable")?.join(name),
        Some(p) if p.chars().any(char::is_control) || p.len() > 1024 => return Err("backup_path_invalid".into()),
        Some(p) => {
            let p = PathBuf::from(p);
            if !p.is_absolute() { return Err("backup_path_invalid".into()); }
            if p.is_dir() { p.join(name) } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) { p } else {
                let mut s = p.into_os_string();
                s.push(format!(".{EXTENSION}"));
                PathBuf::from(s)
            }
        }
    };
    Ok(path)
}

/// Writes a sealed backup, creating the default folder when needed.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::private_store::write(path, bytes).map_err(|_| "backup_unwritable".into())
}

pub fn read(path: &str) -> Result<Vec<u8>, String> {
    let path = Path::new(path.trim());
    if !path.is_absolute() { return Err("backup_path_invalid".into()); }
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.len() <= MAX_FILE => std::fs::read(path).map_err(|_| "backup_unreadable".into()),
        Ok(_) => Err("backup_not_a_backup".into()),
        Err(_) => Err("backup_not_found".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Restore: what changes, and the merged result

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Changes {
    pub added: Vec<String>,
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skipped {
    pub what: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountMatch {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    /// Signed in here already; otherwise pending until it is reconnected.
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preview {
    pub created_at: DateTime<Utc>,
    pub app_version: String,
    /// Preferences (top-level settings) the backup changes.
    pub settings: Vec<String>,
    pub aliases: Changes,
    pub plans: Changes,
    pub pins: Changes,
    /// Accounts whose weekly history is added or merged.
    pub history: Changes,
    /// Months of the spend ledger added or merged.
    pub ledger: Changes,
    pub skipped: Vec<Skipped>,
    pub accounts: Vec<AccountMatch>,
    pub pending: usize,
    pub note: &'static str,
}

pub struct Merged {
    pub settings: Settings,
    pub history: History,
    pub ledger: Ledger,
}

/// What restoring needs to know about this machine.
pub struct Here<'a> {
    /// Accounts signed in here (provider, account id).
    pub known: &'a [(Provider, String)],
    pub updates_enabled: bool,
    /// A pinned folder as this machine spells it, or None when it does not exist here.
    pub folder: &'a dyn Fn(&str) -> Option<String>,
    /// Whether a sharing folder or calendar path from the backup can be used here.
    pub presence_folder_ok: &'a dyn Fn(&str) -> bool,
    pub calendar_path_ok: &'a dyn Fn(&str) -> bool,
}

fn account_key(key: &str) -> Option<(Provider, String)> {
    let (p, id) = key.split_once(':')?;
    let provider = match p.to_ascii_lowercase().as_str() { "claude" => Provider::Claude, "codex" => Provider::Codex, _ => return None };
    (!id.is_empty()).then(|| (provider, id.to_string()))
}

fn printable(s: &str, max: usize) -> bool { !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control) }

fn merge_map<V: Clone + PartialEq>(current: &mut HashMap<String, V>, from: &HashMap<String, V>, valid: impl Fn(&str, &V) -> bool, changes: &mut Changes, skipped: &mut Vec<Skipped>, what: &str) {
    let mut keys: Vec<_> = from.keys().collect();
    keys.sort();
    for key in keys {
        let value = &from[key];
        if !valid(key, value) {
            skipped.push(Skipped { what: format!("{what} {key}"), reason: "invalid" });
            continue;
        }
        match current.get(key) {
            None => changes.added.push(key.clone()),
            Some(v) if v != value => changes.changed.push(key.clone()),
            Some(_) => continue,
        }
        current.insert(key.clone(), value.clone());
    }
}

/// Merges a backup over what is here. Preferences take the backup's values; per-account data
/// (aliases, plan details, pins, history, ledger) is combined, the backup winning where both
/// hold the same entry. Data for accounts not signed in here is kept, pending their reconnection.
pub fn merge(settings: &Settings, history: &History, ledger: &Ledger, backup: &Contents, here: &Here, now: DateTime<Utc>) -> (Merged, Preview) {
    let mut skipped = Vec::new();
    let from = &backup.settings;

    // Preferences: the backup's, unless this machine cannot use them.
    let mut next = Settings {
        last_version: settings.last_version.clone(),
        aliases: settings.aliases.clone(),
        subscriptions: settings.subscriptions.clone(),
        web_use: settings.web_use.clone(),
        project_pins: settings.project_pins.clone(),
        ..from.clone()
    };
    if next.auto_update && !here.updates_enabled { next.auto_update = settings.auto_update; }
    if !crate::settings::LANGUAGES.contains(&next.language.as_str()) { next.language = settings.language.clone(); }
    if !crate::settings::REFRESH_CHOICES.contains(&next.refresh_minutes) { next.refresh_minutes = settings.refresh_minutes; }
    if let Some(folder) = next.presence_folder.clone().filter(|f| !(here.presence_folder_ok)(f)) {
        skipped.push(Skipped { what: format!("presence_folder {folder}"), reason: "folder_missing_here" });
        next.presence_folder = settings.presence_folder.clone();
    }
    if let Some(path) = next.calendar.path.clone().filter(|p| !(here.calendar_path_ok)(p)) {
        skipped.push(Skipped { what: format!("calendar.path {path}"), reason: "folder_missing_here" });
        next.calendar.path = settings.calendar.path.clone();
    }
    next.switcher_hotkey = crate::settings::normalize_hotkey(&next.switcher_hotkey).unwrap_or_else(|_| settings.switcher_hotkey.clone());
    if next.phone.url.as_deref().is_some_and(|u| crate::phone::normalize_url(u).is_err()) {
        skipped.push(Skipped { what: "phone.url".into(), reason: "invalid" });
        next.phone = settings.phone.clone();
    }

    let mut aliases = Changes::default();
    merge_map(&mut next.aliases, &from.aliases, |k, v: &String| account_key(k).is_some() && printable(v.trim(), 64), &mut aliases, &mut skipped, "alias");
    let mut plans = Changes::default();
    merge_map(&mut next.subscriptions, &from.subscriptions, |k, p: &crate::settings::Subscription| {
        account_key(k).is_some() && p.billing_day.map_or(true, |d| (1..=31).contains(&d)) && p.monthly_cost.map_or(true, |c| c.is_finite() && (0.0..=10_000.0).contains(&c))
    }, &mut plans, &mut skipped, "plan");
    let mut web = Changes::default();
    merge_map(&mut next.web_use, &from.web_use, |k, on: &bool| account_key(k).is_some() && *on, &mut web, &mut Vec::new(), "web_use");

    let mut pins = Changes::default();
    for pin in &from.project_pins {
        let Some(folder) = (here.folder)(&pin.folder) else {
            skipped.push(Skipped { what: format!("pin {}", pin.folder), reason: "folder_missing_here" });
            continue;
        };
        let pin = ProjectPin { folder, ..pin.clone() };
        let full = next.project_pins.len() >= crate::pins::MAX_PINS;
        match next.project_pins.iter_mut().find(|p| p.folder.eq_ignore_ascii_case(&pin.folder) && p.provider == pin.provider) {
            Some(p) if *p == pin => {}
            Some(p) => { pins.changed.push(pin.folder.clone()); *p = pin; }
            None if full => skipped.push(Skipped { what: format!("pin {}", pin.folder), reason: "too_many_pins" }),
            None => { pins.added.push(pin.folder.clone()); next.project_pins.push(pin); }
        }
    }

    let preferences: Vec<String> = {
        let (a, b) = (serde_json::to_value(settings).unwrap_or_default(), serde_json::to_value(&next).unwrap_or_default());
        let fixed = ["aliases", "subscriptions", "project_pins", "web_use", "last_version"];
        b.as_object().into_iter().flatten().filter(|(k, v)| !fixed.contains(&k.as_str()) && a.get(k.as_str()) != Some(v)).map(|(k, _)| k.clone()).collect()
    };

    // Accounts: everything the backup holds data for, matched by provider and id.
    let known: HashSet<(Provider, String)> = here.known.iter().cloned().collect();
    let mut seen: BTreeMap<String, AccountMatch> = BTreeMap::new();
    let keys = backup.accounts.iter().map(|a| (a.provider, a.id.clone()))
        .chain(from.aliases.keys().chain(from.subscriptions.keys()).chain(backup.usage_history.accounts.keys()).filter_map(|k| account_key(k)))
        .chain(from.project_pins.iter().map(|p| (p.provider, p.id.clone())));
    for (provider, id) in keys {
        let pref = crate::providers::alias_key(provider, &id);
        seen.entry(pref.clone()).or_insert_with(|| AccountMatch {
            provider,
            email: backup.accounts.iter().find(|a| a.provider == provider && a.id == id).and_then(|a| a.email.clone()),
            alias: next.aliases.get(&pref).cloned(),
            connected: known.contains(&(provider, id.clone())),
            id,
        });
    }
    let accounts: Vec<AccountMatch> = seen.into_values().collect();
    let pending = accounts.iter().filter(|a| !a.connected).count();

    // Weekly history per account: windows from both, one per reset.
    let mut merged_history = history.clone();
    merged_history.version = crate::subscriptions::VERSION;
    let mut hist = Changes::default();
    let mut keys: Vec<_> = backup.usage_history.accounts.keys().collect();
    keys.sort();
    for key in keys {
        let from = &backup.usage_history.accounts[key];
        let Some((provider, id)) = account_key(key) else { continue };
        let connected = known.contains(&(provider, id));
        let entry = merged_history.accounts.entry(key.clone());
        let mine = match entry {
            std::collections::hash_map::Entry::Vacant(v) => { hist.added.push(key.clone()); v.insert(from.clone()) }
            std::collections::hash_map::Entry::Occupied(o) => {
                let mine = o.into_mut();
                let before = serde_json::to_value(&*mine).ok();
                for w in &from.windows {
                    match mine.windows.iter_mut().find(|m| m.kind == w.kind && m.resets_at == w.resets_at && m.idle == w.idle) {
                        Some(m) if w.hours > m.hours => *m = w.clone(),
                        Some(_) => {}
                        None => mine.windows.push(w.clone()),
                    }
                }
                mine.windows.sort_by_key(|w| (w.resets_at, w.kind.clone()));
                mine.first_seen = match (mine.first_seen, from.first_seen) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
                mine.last_seen = mine.last_seen.max(from.last_seen);
                if mine.credits.is_none() { mine.credits = from.credits.clone(); }
                if before != serde_json::to_value(&*mine).ok() { hist.changed.push(key.clone()); }
                mine
            }
        };
        // History of an account not seen for two weeks is forgotten; a pending one gets those two
        // weeks from now to be reconnected.
        if !connected { mine.last_seen = mine.last_seen.max(Some(now)); }
    }

    // Spend ledger: one charge per account and day, the larger extra-usage amount.
    let mut merged_ledger = ledger.clone();
    merged_ledger.version = crate::ledger::VERSION;
    let mut months = Changes::default();
    for (key, from) in &backup.spend_ledger.months {
        match merged_ledger.months.get_mut(key) {
            None => { months.added.push(key.clone()); merged_ledger.months.insert(key.clone(), from.clone()); }
            Some(mine) => {
                let before = mine.clone();
                for c in &from.charges {
                    if !mine.charges.iter().any(|m| m.account == c.account && m.at.date_naive() == c.at.date_naive()) { mine.charges.push(c.clone()); }
                }
                mine.charges.sort_by_key(|c| c.at);
                for (account, v) in &from.extra {
                    let e = mine.extra.entry(account.clone()).or_insert(0.0);
                    if *v > *e { *e = *v; }
                }
                if *mine != before { months.changed.push(key.clone()); }
            }
        }
    }
    for (k, p) in &backup.spend_ledger.pending { merged_ledger.pending.entry(k.clone()).or_insert_with(|| p.clone()); }
    for (k, v) in &backup.spend_ledger.extra_marks { merged_ledger.extra_marks.entry(k.clone()).or_insert(*v); }

    let preview = Preview {
        created_at: backup.created_at,
        app_version: backup.app_version.clone(),
        settings: preferences,
        aliases,
        plans,
        pins,
        history: hist,
        ledger: months,
        skipped,
        accounts,
        pending,
        note: NOTE,
    };
    (Merged { settings: next, history: merged_history, ledger: merged_ledger }, preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{slot, TempDir};
    use chrono::TimeZone;

    const ACCESS: &str = "sk-ant-oat01-SYNTHETIC-ACCESS-7f3a9c";
    const REFRESH: &str = "sk-ant-ort01-SYNTHETIC-REFRESH-2b8d41";
    const PHONE_TOKEN: &str = "tk_SYNTHETICPHONETOKEN0042";

    fn at(day: u32) -> DateTime<Utc> { Utc.with_ymd_and_hms(2026, 10, day, 12, 0, 0).unwrap() }
    fn pass(s: &str) -> Passphrase { Passphrase(s.into()) }

    fn sample() -> Contents {
        let mut settings = Settings { refresh_minutes: 15, language: "de".into(), last_version: Some("1.2.3".into()), ..Settings::default() };
        settings.aliases.insert("claude:a1".into(), "Work".into());
        settings.aliases.insert("codex:gone".into(), "Old laptop".into());
        settings.subscriptions.insert("claude:a1".into(), crate::settings::Subscription { billing_day: Some(3), monthly_cost: Some(100.0), ..Default::default() });
        settings.phone.enabled = true;
        settings.phone.url = Some("https://ntfy.sh/synthetic-topic".into());
        let history: History = serde_json::from_value(serde_json::json!({ "version": 1, "accounts": {
            "Claude:a1": { "first_seen": at(1), "last_seen": at(10), "windows": [
                { "kind": "weekly", "resets_at": at(8), "peak": 40.0, "first_obs": at(1), "last_obs": at(8), "hours": 30, "days": 5 } ] },
            "Codex:gone": { "first_seen": at(1), "last_seen": at(2), "windows": [] },
        } })).unwrap();
        let ledger: Ledger = serde_json::from_value(serde_json::json!({ "version": 1, "months": {
            "2026-10": { "charges": [{ "account": "Claude:a1", "at": at(3), "amount": 100.0, "tier": "max_5x", "source": "entered", "cost_entered": true }], "extra": { "Claude:a1": 4.5 } },
        } })).unwrap();
        let s = slot("a1", ACCESS, REFRESH, 0);
        let accounts = vec![Account { provider: s.provider, id: s.id.clone(), email: s.email.clone() }];
        Contents::new(&settings, &history, &ledger, accounts, "0.9.0", at(12))
    }

    fn here_with<'a>(known: &'a [(Provider, String)], folder: &'a dyn Fn(&str) -> Option<String>) -> Here<'a> {
        Here { known, updates_enabled: false, folder, presence_folder_ok: &|_| true, calendar_path_ok: &|_| true }
    }

    #[test]
    fn round_trips_through_the_passphrase() {
        let contents = sample();
        let bytes = seal(&contents, &pass("correct horse battery")).unwrap();
        let opened = open(&bytes, &pass("correct horse battery")).unwrap();
        assert_eq!(opened.settings.aliases, contents.settings.aliases);
        assert_eq!(opened.settings.refresh_minutes, 15);
        assert_eq!(opened.settings.last_version, None, "the app's own bookkeeping stays out");
        assert_eq!(opened.usage_history.accounts["Claude:a1"].windows.len(), 1);
        assert_eq!(opened.spend_ledger.months["2026-10"].charges[0].amount, Some(100.0));
        assert_eq!(opened.accounts, contents.accounts);
        let again = seal(&contents, &pass("correct horse battery")).unwrap();
        assert_ne!(bytes, again, "a fresh salt and nonce every time");
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!((envelope["format"].as_str(), envelope["kdf"]["algorithm"].as_str(), envelope["cipher"].as_str()), (Some(FORMAT), Some("argon2id"), Some("xchacha20poly1305")));
    }

    #[test]
    fn a_wrong_passphrase_does_not_open_it() {
        let bytes = seal(&sample(), &pass("correct horse battery")).unwrap();
        assert_eq!(open(&bytes, &pass("correct horse battery!")).unwrap_err(), "backup_wrong_passphrase_or_damaged");
        assert_eq!(seal(&sample(), &pass("short")).unwrap_err(), "backup_passphrase_too_short");
    }

    #[test]
    fn a_tampered_file_does_not_open() {
        let bytes = seal(&sample(), &pass("correct horse battery")).unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        // One flipped ciphertext byte.
        let mut sealed = b64().decode(envelope["ciphertext"].as_str().unwrap()).unwrap();
        sealed[10] ^= 1;
        let mut flipped = envelope.clone();
        flipped["ciphertext"] = b64().encode(&sealed).into();
        assert_eq!(open(&serde_json::to_vec(&flipped).unwrap(), &pass("correct horse battery")).unwrap_err(), "backup_wrong_passphrase_or_damaged");
        // A header change (weaker KDF settings) breaks the associated data.
        let mut weaker = envelope.clone();
        weaker["kdf"]["iterations"] = 1.into();
        assert_eq!(open(&serde_json::to_vec(&weaker).unwrap(), &pass("correct horse battery")).unwrap_err(), "backup_wrong_passphrase_or_damaged");
        let mut newer = envelope.clone();
        newer["version"] = 2.into();
        assert_eq!(open(&serde_json::to_vec(&newer).unwrap(), &pass("correct horse battery")).unwrap_err(), "backup_version_unsupported");
        let mut greedy = envelope.clone();
        greedy["kdf"]["memory_kib"] = (8u64 << 20).into();
        assert_eq!(open(&serde_json::to_vec(&greedy).unwrap(), &pass("correct horse battery")).unwrap_err(), "backup_version_unsupported");
        assert_eq!(open(b"{\"hello\":1}", &pass("correct horse battery")).unwrap_err(), "backup_not_a_backup");
        assert_eq!(open(&bytes[..bytes.len() / 2], &pass("correct horse battery")).unwrap_err(), "backup_not_a_backup");
    }

    #[test]
    fn no_secrets_reach_the_file() {
        let contents = sample();
        let bytes = seal(&contents, &pass("correct horse battery")).unwrap();
        let plain = serde_json::to_string(&open(&bytes, &pass("correct horse battery")).unwrap()).unwrap();
        let file = String::from_utf8(bytes).unwrap();
        for secret in [ACCESS, REFRESH, PHONE_TOKEN, "accessToken", "refreshToken", "claudeAiOauth", "access_token", "refresh_token", "correct horse battery"] {
            assert!(!plain.contains(secret), "{secret} in the backup's contents");
            assert!(!file.contains(secret), "{secret} in the backup file");
        }
        assert!(plain.contains("test@example.invalid"), "the account's email names it in a preview");
        assert_eq!(format!("{:?}", pass("correct horse battery")), "Passphrase(..)");
    }

    #[test]
    fn restore_previews_and_merges_keeping_unknown_accounts_pending() {
        let backup = open(&seal(&sample(), &pass("correct horse battery")).unwrap(), &pass("correct horse battery")).unwrap();
        let mut current = Settings { last_version: Some("2.0.0".into()), ..Settings::default() };
        current.aliases.insert("claude:a1".into(), "Personal".into());
        current.aliases.insert("claude:mine".into(), "Kept".into());
        let mut history = History { version: 1, ..Default::default() };
        history.accounts.insert("Claude:a1".into(), serde_json::from_value(serde_json::json!({ "first_seen": at(5), "last_seen": at(20), "windows": [
            { "kind": "weekly", "resets_at": at(15), "peak": 70.0, "first_obs": at(8), "last_obs": at(15), "hours": 40, "days": 6 } ] })).unwrap());
        let ledger = Ledger { version: 1, ..Default::default() };
        let known = [(Provider::Claude, "a1".to_string())];
        let (merged, preview) = merge(&current, &history, &ledger, &backup, &here_with(&known, &|_| None), at(22));
        assert_eq!(merged.settings.aliases["claude:a1"], "Work", "the backup wins where both have an entry");
        assert_eq!(merged.settings.aliases["claude:mine"], "Kept", "entries only here stay");
        assert_eq!(merged.settings.aliases["codex:gone"], "Old laptop");
        assert_eq!(merged.settings.last_version.as_deref(), Some("2.0.0"));
        assert_eq!((merged.settings.refresh_minutes, merged.settings.language.as_str()), (15, "de"));
        assert_eq!(preview.aliases, Changes { added: vec!["codex:gone".into()], changed: vec!["claude:a1".into()] });
        assert_eq!(preview.plans.added, ["claude:a1"]);
        assert!(preview.settings.contains(&"refresh_minutes".to_string()) && preview.settings.contains(&"phone".to_string()), "{:?}", preview.settings);
        let a1 = &merged.history.accounts["Claude:a1"];
        assert_eq!(a1.windows.len(), 2, "both weeks kept");
        assert_eq!((a1.first_seen, a1.last_seen), (Some(at(1)), Some(at(20))));
        assert_eq!(merged.history.accounts["Codex:gone"].last_seen, Some(at(22)), "a pending account gets time to be reconnected");
        assert_eq!((preview.history.added.clone(), preview.history.changed.clone()), (vec!["Codex:gone".to_string()], vec!["Claude:a1".to_string()]));
        assert_eq!(preview.ledger.added, ["2026-10"]);
        assert_eq!(merged.ledger.months["2026-10"].extra["Claude:a1"], 4.5);
        let names: Vec<_> = preview.accounts.iter().map(|a| (a.id.as_str(), a.connected)).collect();
        assert_eq!(names, [("a1", true), ("gone", false)]);
        assert_eq!(preview.pending, 1);
        assert_eq!(preview.accounts[0].email.as_deref(), Some("test@example.invalid"));
        assert!(preview.note.contains("reconnect"));

        // Restoring the same backup again changes nothing per account.
        let (twice, again) = merge(&merged.settings, &merged.history, &merged.ledger, &backup, &here_with(&known, &|_| None), at(22));
        assert!(again.aliases.added.is_empty() && again.aliases.changed.is_empty() && again.ledger.changed.is_empty() && again.history.changed.is_empty(), "{again:?}");
        assert_eq!(twice.ledger.months["2026-10"].charges.len(), 1);
    }

    #[test]
    fn pins_restore_only_where_the_folder_exists_here() {
        let dir = TempDir::new();
        let mut contents = sample();
        let here_folder = dir.0.display().to_string();
        contents.settings.project_pins = vec![
            ProjectPin { folder: here_folder.clone(), provider: Provider::Claude, id: "a1".into() },
            ProjectPin { folder: "C:\\nowhere\\at\\all".into(), provider: Provider::Codex, id: "gone".into() },
        ];
        let folder = |f: &str| crate::pins::normalize_folder(f).ok();
        let (merged, preview) = merge(&Settings::default(), &History::default(), &Ledger::default(), &contents, &here_with(&[], &folder), at(22));
        assert_eq!(merged.settings.project_pins.len(), 1);
        assert_eq!(preview.pins.added.len(), 1);
        assert!(preview.skipped.iter().any(|s| s.what.contains("nowhere") && s.reason == "folder_missing_here"), "{:?}", preview.skipped);
    }

    #[test]
    fn exports_go_to_a_backup_file_only() {
        let dir = TempDir::new();
        let default = Some(dir.0.clone());
        assert_eq!(target(None, default.clone(), "x").unwrap(), dir.0.join("SlopBar-Backup-x.slopbar-backup"));
        assert_eq!(target(Some(&dir.0.display().to_string()), None, "x").unwrap(), dir.0.join("SlopBar-Backup-x.slopbar-backup"));
        let settings = dir.0.join("settings.json");
        assert_eq!(target(Some(&settings.display().to_string()), None, "x").unwrap(), dir.0.join("settings.json.slopbar-backup"), "never replaces another file");
        assert!(target(Some("relative.slopbar-backup"), None, "x").is_err());
        let path = target(None, default, "y").unwrap();
        write(&path, &seal(&sample(), &pass("correct horse battery")).unwrap()).unwrap();
        let opened = open(&read(&path.display().to_string()).unwrap(), &pass("correct horse battery")).unwrap();
        assert_eq!(opened.settings.refresh_minutes, 15);
        assert_eq!(read(&dir.0.join("absent").display().to_string()).unwrap_err(), "backup_not_found");
    }
}
