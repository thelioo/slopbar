//! Multiple accounts per provider.
//!
//! Each CLI signs in one account at a time, in its default config location. SlopBar keeps a
//! vault with a saved login ("slot") for every account it has seen there, and switches accounts
//! by writing a slot back into those default files — the official CLI keeps doing all the work,
//! nothing is proxied.
//!
//! Tokens rotate when a CLI refreshes them, so the live login is always synced back into its
//! slot before anything else is written. New accounts are added by running the CLI's own login
//! in an isolated, temporary config dir and importing the result.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::sources::{self, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

/// A saved login: everything a CLI needs to be signed in as one account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub id: String,
    pub provider: Provider,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub org: Option<String>,
    /// Claude: `{ credentials: {claudeAiOauth, organizationUuid}, oauth_account }`.
    /// Codex: `{ auth: <auth.json> }`.
    pub data: Value,
}

impl Slot {
    /// Access token and, for Codex, the ChatGPT account id used for usage requests.
    pub fn access(&self) -> Option<(String, Option<String>)> {
        match self.provider {
            Provider::Claude => {
                let oauth = &self.data["credentials"]["claudeAiOauth"];
                Some((oauth["accessToken"].as_str()?.to_string(), None))
            }
            Provider::Codex => {
                let tokens = &self.data["auth"]["tokens"];
                Some((
                    tokens["access_token"].as_str()?.to_string(),
                    tokens["account_id"].as_str().map(String::from),
                ))
            }
        }
    }

    /// Plan and billing facts the saved login carries: Claude's rate-limit tier and account record,
    /// Codex's subscription claims in its id token. Never a token, email or account id.
    pub fn billing(&self) -> crate::model::BillingHints {
        let date = |v: &Value| v.as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|t| t.with_timezone(&chrono::Utc));
        let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(String::from);
        match self.provider {
            Provider::Claude => {
                let account = &self.data["oauth_account"];
                crate::model::BillingHints {
                    tier: text(&self.data["credentials"]["claudeAiOauth"]["rateLimitTier"]).or_else(|| text(&account["organizationRateLimitTier"])),
                    billing_type: text(&account["billingType"]),
                    started_at: date(&account["subscriptionCreatedAt"]),
                    ..Default::default()
                }
            }
            Provider::Codex => {
                let claims = self.data["auth"]["tokens"]["id_token"].as_str().and_then(jwt_claims).unwrap_or(Value::Null);
                let openai = &claims["https://api.openai.com/auth"];
                crate::model::BillingHints {
                    started_at: date(&openai["chatgpt_subscription_active_start"]),
                    active_until: date(&openai["chatgpt_subscription_active_until"]),
                    checked_at: date(&openai["chatgpt_subscription_last_checked"]),
                    ..Default::default()
                }
            }
        }
    }

    /// Whether the saved access token has certainly expired (Claude records its expiry).
    pub fn expired(&self) -> bool {
        match self.provider {
            Provider::Claude => self.data["credentials"]["claudeAiOauth"]["expiresAt"]
                .as_i64()
                .is_some_and(|ms| ms < chrono::Utc::now().timestamp_millis()),
            Provider::Codex => false,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading and writing a CLI's login files

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes through a temp file and a rename so a CLI never reads a half-written file.
fn write_json(path: &Path, value: &Value) -> std::io::Result<()> {
    let tmp = path.with_extension("slopbar-tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)
}

fn stable_hash(s: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Claude account identities resolved from the profile API, keyed by access token hash.
/// `.claude.json` can't be trusted for identity: a running Claude Code rewrites it from memory
/// and may restore the previous account's details right after a switch.
static CLAUDE_IDENTITY: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

fn claude_token(source: &Source) -> Option<String> {
    let creds = read_json(&source.claude_dir().join(".credentials.json"))?;
    creds["claudeAiOauth"]["accessToken"].as_str().map(String::from)
}

/// Resolves (once per token) who each live Claude login belongs to.
pub async fn resolve_identities(client: &reqwest::Client, sources: &[Source]) {
    let mut unknown: Vec<_> = sources.iter().filter_map(claude_token).map(|token| (stable_hash(&token), token))
        .filter(|(key, _)| !CLAUDE_IDENTITY.lock().unwrap().as_ref().is_some_and(|m| m.contains_key(key)))
        .collect();
    unknown.sort_by(|a, b| a.0.cmp(&b.0));
    unknown.dedup_by(|a, b| a.0 == b.0);
    // Read-only profile lookups: all at once, so one slow login does not hold up the others.
    let found = futures::future::join_all(unknown.into_iter().map(|(key, token)| async move {
        claude_profile(client, &token).await.map(|account| (key, account))
    })).await;
    let mut known = CLAUDE_IDENTITY.lock().unwrap();
    known.get_or_insert_with(HashMap::new).extend(found.into_iter().flatten());
}

/// The login currently signed in at `source`, if any. `known` are the vault's slots.
///
/// Claude's identity is taken, in order of trust, from: a vault slot holding this exact refresh
/// token (right after a switch), the profile API for this access token, and only then
/// `.claude.json` (which a running Claude Code may have rewritten with a previous account).
pub fn read_login(provider: Provider, source: &Source, known: &[Slot]) -> Option<Slot> {
    match provider {
        Provider::Claude => {
            let creds = read_json(&source.claude_dir().join(".credentials.json"))?;
            let oauth = creds.get("claudeAiOauth")?;
            let token = oauth.get("accessToken")?.as_str()?;
            let refresh = oauth["refreshToken"].as_str().unwrap_or_default();
            let file = read_json(&source.claude_json())
                .and_then(|j| j.get("oauthAccount").cloned())
                .filter(|a| a.get("accountUuid").is_some());

            let same_login = known.iter().find(|s| {
                s.provider == Provider::Claude
                    && !refresh.is_empty()
                    && s.data["credentials"]["claudeAiOauth"]["refreshToken"].as_str() == Some(refresh)
            });
            let resolved = CLAUDE_IDENTITY
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|m| m.get(&stable_hash(token)).cloned());
            let identity = same_login
                .map(|s| s.data["oauth_account"].clone())
                .filter(|a| !a.is_null())
                .or(resolved);
            let account = match identity {
                // Prefer the CLI's full record when it agrees with the trusted identity.
                Some(id) => Some(match &file {
                    Some(f) if f["accountUuid"] == id["accountUuid"] => f.clone(),
                    _ => id,
                }),
                None => file,
            };
            claude_slot(creds.clone(), account)
        }
        Provider::Codex => {
            let _ = known;
            let auth = read_json(&source.codex_dir().join("auth.json"))?;
            codex_slot(auth)
        }
    }
}

fn claude_slot(creds: Value, account: Option<Value>) -> Option<Slot> {
    let oauth = creds.get("claudeAiOauth")?;
    let org = creds["organizationUuid"]
        .as_str()
        .or_else(|| account.as_ref().and_then(|a| a["organizationUuid"].as_str()))
        .unwrap_or("");
    // Identity must survive token rotation: account + organization when known.
    let id = match account.as_ref().and_then(|a| a["accountUuid"].as_str()) {
        Some(uuid) => format!("{uuid}:{org}"),
        None => format!("token:{}", stable_hash(oauth["refreshToken"].as_str().unwrap_or(""))),
    };
    Some(Slot {
        id,
        provider: Provider::Claude,
        email: account.as_ref().and_then(|a| a["emailAddress"].as_str().map(String::from)),
        plan: oauth["subscriptionType"].as_str().map(String::from),
        org: account.as_ref().and_then(|a| a["organizationName"].as_str().map(String::from)),
        data: json!({
            "credentials": {
                "claudeAiOauth": oauth,
                "organizationUuid": creds.get("organizationUuid").cloned().unwrap_or(Value::Null),
            },
            "oauth_account": account.unwrap_or(Value::Null),
        }),
    })
}

fn codex_slot(auth: Value) -> Option<Slot> {
    let tokens = auth.get("tokens")?;
    tokens.get("access_token")?;
    let claims = tokens["id_token"].as_str().and_then(jwt_claims).unwrap_or(Value::Null);
    let openai = &claims["https://api.openai.com/auth"];
    let user = openai["chatgpt_user_id"]
        .as_str()
        .or(openai["user_id"].as_str())
        .or(claims["sub"].as_str())
        .unwrap_or("");
    let account = tokens["account_id"].as_str().or(openai["chatgpt_account_id"].as_str()).unwrap_or("");
    let id = if user.is_empty() && account.is_empty() {
        format!("token:{}", stable_hash(tokens["refresh_token"].as_str().unwrap_or("")))
    } else {
        format!("{user}:{account}")
    };
    Some(Slot {
        id,
        provider: Provider::Codex,
        email: claims["email"].as_str().map(String::from),
        plan: openai["chatgpt_plan_type"].as_str().map(String::from),
        org: None,
        data: json!({ "auth": auth }),
    })
}

/// Signs `source` in as `slot`, touching only account-specific fields.
fn write_login(slot: &Slot, source: &Source) -> std::io::Result<()> {
    match slot.provider {
        Provider::Claude => {
            let path = source.claude_dir().join(".credentials.json");
            let mut creds = read_json(&path).unwrap_or_else(|| json!({}));
            creds["claudeAiOauth"] = slot.data["credentials"]["claudeAiOauth"].clone();
            creds["organizationUuid"] = slot.data["credentials"]["organizationUuid"].clone();
            write_json(&path, &creds)?;
            // The rest of .claude.json is user config and history; only the account block changes.
            let config = source.claude_json();
            if let (Some(mut cfg), false) = (read_json(&config), slot.data["oauth_account"].is_null()) {
                cfg["oauthAccount"] = slot.data["oauth_account"].clone();
                write_json(&config, &cfg)?;
            }
            Ok(())
        }
        Provider::Codex => write_json(&source.codex_dir().join("auth.json"), &slot.data["auth"]),
    }
}

/// Whether the provider's CLI has been used at `source` (its config dir exists).
fn installed(provider: Provider, source: &Source) -> bool {
    match provider {
        Provider::Claude => source.claude_dir().is_dir(),
        Provider::Codex => source.codex_dir().is_dir(),
    }
}

// ---------------------------------------------------------------------------------------------
// The vault
//
// One file per slot, encrypted with Windows DPAPI for the signed-in Windows user (see
// `private_store::save_secret`): `<provider>-<id>.slot`. Releases before the encrypted vault
// wrote the same login as plaintext `<provider>-<id>.json`; `migrate_vault` encrypts those on
// startup. For one release a plaintext slot is still read (whichever file is newer wins), so a
// slot whose migration failed is never lost, only left unencrypted until the next attempt.

fn vault_dir() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("accounts"))
}

fn slot_stem(provider: Provider, id: &str) -> String {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("{}-{safe}", provider.key())
}

fn slot_path_in(vault: &Path, provider: Provider, id: &str) -> PathBuf {
    vault.join(format!("{}.{SLOT_EXT}", slot_stem(provider, id)))
}

const SLOT_EXT: &str = "slot";
/// The plaintext slots of earlier releases, read for one release as a fallback.
const PLAIN_EXT: &str = "json";

pub fn load_slots() -> Vec<Slot> {
    vault_dir().map(|d| load_slots_in(&d)).unwrap_or_default()
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read_slot(path: &Path) -> Option<Slot> {
    let bytes = match path.extension().and_then(|e| e.to_str()) {
        Some(SLOT_EXT) => crate::private_store::load_secret(path).ok()?,
        Some(PLAIN_EXT) => std::fs::read(path).ok()?,
        _ => return None,
    };
    serde_json::from_slice(&bytes).ok()
}

fn load_slots_in(dir: &Path) -> Vec<Slot> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    // Per file stem, the newest readable copy: an encrypted slot, or a plaintext one written by an
    // earlier release (after a downgrade, the plaintext copy is the newer login).
    let mut newest: HashMap<String, (Option<std::time::SystemTime>, Slot)> = HashMap::new();
    for path in entries.flatten().map(|e| e.path()) {
        let (Some(stem), Some(slot)) = (path.file_stem().and_then(|s| s.to_str()).map(String::from), read_slot(&path)) else { continue };
        let at = modified(&path);
        if newest.get(&stem).map_or(true, |(t, _)| at > *t) {
            newest.insert(stem, (at, slot));
        }
    }
    let mut slots: Vec<Slot> = newest.into_values().map(|(_, s)| s).collect();
    slots.sort_by(|a, b| (a.provider.key(), &a.email, &a.id).cmp(&(b.provider.key(), &b.email, &b.id)));
    slots
}

fn save_slot(slot: &Slot) -> std::io::Result<()> {
    save_slot_in(&vault_dir().ok_or_else(|| std::io::Error::other("no config dir"))?, slot)
}

fn save_slot_in(vault: &Path, slot: &Slot) -> std::io::Result<()> {
    let path = slot_path_in(vault, slot.provider, &slot.id);
    crate::private_store::save_secret(&path, &serde_json::to_vec(slot)?)?;
    // The encrypted copy is now the newest, so a leftover plaintext one is only a stale secret.
    match std::fs::remove_file(path.with_extension(PLAIN_EXT)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

pub fn remove_slot(provider: Provider, id: &str) -> std::io::Result<()> {
    let Some(vault) = vault_dir() else { return Ok(()) };
    let path = slot_path_in(&vault, provider, id);
    let mut removed = false;
    for path in [path.with_extension(PLAIN_EXT), path] {
        match std::fs::remove_file(&path) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    if removed { Ok(()) } else { Err(std::io::ErrorKind::NotFound.into()) }
}

/// What encrypting the plaintext slots of earlier releases did (see `migrate_vault`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VaultMigration {
    /// Plaintext slots now stored encrypted, their plaintext file removed.
    pub migrated: usize,
    /// Slots that could not be encrypted, or did not read back; their plaintext file was kept.
    pub failed: usize,
    /// Plaintext files that are not a readable slot; left untouched.
    pub unreadable: usize,
}

impl VaultMigration {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Encrypts every plaintext slot. Each one is written encrypted, read back and compared, and only
/// then is its plaintext file removed; on any failure the plaintext file stays (and is still read)
/// and a half-written encrypted copy is removed, so no slot is ever lost.
pub fn migrate_vault() -> VaultMigration {
    let _guard = VAULT.lock().unwrap_or_else(|e| e.into_inner());
    vault_dir().map(|d| migrate_vault_in(&d, crate::private_store::save_secret, crate::private_store::load_secret)).unwrap_or_default()
}

fn migrate_vault_in(
    vault: &Path,
    save: impl Fn(&Path, &[u8]) -> std::io::Result<()>,
    load: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
) -> VaultMigration {
    let mut result = VaultMigration::default();
    let Ok(entries) = std::fs::read_dir(vault) else { return result };
    let mut plain: Vec<PathBuf> = entries.flatten().map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(PLAIN_EXT))
        .collect();
    plain.sort();
    for path in plain {
        let Some(slot) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(|v| serde_json::from_value::<Slot>(v.clone()).is_ok()) else {
            result.unreadable += 1;
            continue;
        };
        let target = path.with_extension(SLOT_EXT);
        // A newer encrypted copy already exists: the plaintext one is stale, but it is only
        // removed once that copy is known to read back.
        let newer = target.exists() && modified(&target) > modified(&path)
            && load(&target).ok().and_then(|b| serde_json::from_slice::<Slot>(&b).ok()).is_some();
        let stored = newer || (|| -> std::io::Result<bool> {
            save(&target, &serde_json::to_vec(&slot)?)?;
            Ok(load(&target).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).as_ref() == Some(&slot))
        })().unwrap_or(false);
        if stored && std::fs::remove_file(&path).is_ok() {
            result.migrated += 1;
        } else {
            if !stored {
                // Whatever encrypted copy is there now is either half-written or older than the
                // plaintext one, which stays and keeps being read.
                let _ = std::fs::remove_file(&target);
            }
            result.failed += 1;
        }
    }
    result
}

/// Plaintext slot files still in the vault (a migration that failed, or an earlier release
/// running alongside).
pub fn plaintext_slots() -> usize {
    let Some(Ok(entries)) = vault_dir().map(std::fs::read_dir) else { return 0 };
    entries.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some(PLAIN_EXT)).count()
}

/// Serializes vault reads/writes against switches.
static VAULT: Mutex<()> = Mutex::new(());

/// Where each account is currently signed in: `(provider, slot id) -> source labels`.
pub type Active = HashMap<(Provider, String), Vec<String>>;

/// Saves every live login into the vault (new accounts appear here; rotated tokens are kept
/// current) and reports which account is signed in where.
pub fn capture(sources: &[Source]) -> Active {
    let _guard = VAULT.lock().unwrap();
    let known = load_slots();
    let mut active = Active::new();
    for provider in Provider::ALL {
        for source in sources {
            if let Some(slot) = read_login(provider, source, &known) {
                let _ = save_slot(&slot);
                active.entry((provider, slot.id.clone())).or_default().push(source.label.clone());
            }
        }
    }
    active
}

/// Signs every source where the provider is installed into the account `id`.
pub fn switch(sources: &[Source], provider: Provider, id: &str) -> Result<(), String> {
    // Sync live logins first: the account being left may have refreshed its tokens.
    capture(sources);
    let _guard = VAULT.lock().unwrap();
    let mut target = load_slots()
        .into_iter()
        .find(|s| s.provider == provider && s.id == id)
        .ok_or("account not found")?;
    // A project profile holding this account hands its (possibly newer) login over first, so
    // the account is never signed in at two places at once (see "Project profiles" below).
    if let (Some(vault), Some(root)) = (vault_dir(), profiles_root()) {
        if let Some(newer) = reclaim_profile(&vault, &root, &target) {
            target = newer;
        }
    }
    let mut written = 0;
    for source in sources.iter().filter(|s| installed(provider, s)) {
        write_login(&target, source).map_err(|e| format!("{}: {e}", source.label))?;
        written += 1;
    }
    if written == 0 {
        return Err("CLI not found".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Project profiles
//
// A folder can be pinned to an account (see `pins`): its CLI then runs with CLAUDE_CONFIG_DIR or
// CODEX_HOME set to that account's profile, a config dir under the app data folder that holds the
// account's saved login.
//
// Both CLIs rotate their refresh token when they renew a login, and the old one stops working, so
// a login copied into two live places breaks whichever copy renews second. The vault avoids this
// for the default location by keeping one account signed in there at a time; profiles keep the
// same rule, an account is signed in at exactly one place:
// - while the CLI's default login is another account, the profile holds the login, and renewals
//   made there are synced back into the vault (the newer login wins, see `keeps`);
// - switching the CLI to the account first takes the profile's login back (`reclaim_profile`) and
//   removes it from the profile. The launcher then runs that folder on the default login, which is
//   the same account, until the CLI is switched away and the profile is signed in again.
// The trade-off: a CLI process that is already running when its login moves keeps the login it
// read in memory, and a renewal made there leaves the other place with a stale login, exactly as
// when switching accounts under a running CLI. Starting the CLI through the launcher picks the
// right place each time.

/// Where profiles live: one folder per pinned account, under the app data folder.
pub fn profiles_root() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("profiles"))
}

/// A profile's folder name: short, stable across builds (FNV-1a) and never the account id itself.
pub fn profile_name(provider: Provider, id: &str) -> String {
    let hash = id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{}-{hash:016x}", provider.key())
}

fn profile_source(dir: &Path) -> Source {
    Source {
        label: "Project profile".into(),
        home: dir.to_owned(),
        claude_dir: Some(dir.to_owned()),
        codex_dir: Some(dir.to_owned()),
        distro: None,
        native_home: dir.to_string_lossy().into_owned(),
    }
}

/// The profiles of these accounts as sources, so their Claude logins' identities can be resolved.
pub fn profile_sources(wanted: &[(Provider, String)]) -> Vec<Source> {
    let Some(root) = profiles_root() else { return vec![] };
    wanted.iter().map(|(p, id)| profile_source(&root.join(profile_name(*p, id)))).collect()
}

/// The file in a profile that holds its login.
pub fn profile_login_file(provider: Provider, dir: &Path) -> PathBuf {
    dir.join(match provider {
        Provider::Claude => ".credentials.json",
        Provider::Codex => "auth.json",
    })
}

/// When a login was issued, to tell two copies of one account's login apart: Claude records its
/// access token's expiry, Codex when it last renewed.
fn issued(slot: &Slot) -> Option<i64> {
    match slot.provider {
        Provider::Claude => slot.data["credentials"]["claudeAiOauth"]["expiresAt"].as_i64(),
        Provider::Codex => slot.data["auth"]["last_refresh"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.timestamp_millis()),
    }
}

fn refresh_token(slot: &Slot) -> Option<&str> {
    match slot.provider {
        Provider::Claude => slot.data["credentials"]["claudeAiOauth"]["refreshToken"].as_str(),
        Provider::Codex => slot.data["auth"]["tokens"]["refresh_token"].as_str(),
    }
}

/// Whether a profile's `live` login should replace the vault's `saved` copy of the same account:
/// it differs and the saved one is not certainly newer. The profile is where the account was in
/// use, so it wins when the order is unknown.
fn keeps(live: &Slot, saved: &Slot) -> bool {
    let saved_newer = matches!((issued(saved), issued(live)), (Some(s), Some(l)) if s > l);
    refresh_token(live) != refresh_token(saved) && !saved_newer
}

/// Signs a profile in as `slot`, creating what a fresh config dir needs.
fn write_profile(slot: &Slot, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    match slot.provider {
        Provider::Claude => {
            // `write_login` only replaces the account block of an existing .claude.json.
            let config = dir.join(".claude.json");
            if !config.exists() {
                write_json(&config, &json!({ "hasCompletedOnboarding": true }))?;
            }
        }
        Provider::Codex => {
            // Keep the profile's login in its own auth.json, out of the shared credential store.
            let config = dir.join("config.toml");
            if !config.exists() {
                std::fs::write(config, "cli_auth_credentials_store = \"file\"\n")?;
            }
        }
    }
    write_login(slot, &profile_source(dir))
}

/// Removes a profile's login, keeping the rest of the folder (the CLI's settings and history).
pub fn withdraw_profile(provider: Provider, dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(profile_login_file(provider, dir)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Takes `target`'s login back from its profile before the default location is signed in as it.
/// Returns the profile's login when that one should be used instead (saved into the vault), and
/// leaves the profile without a login. Callers hold `VAULT`.
fn reclaim_profile(vault: &Path, root: &Path, target: &Slot) -> Option<Slot> {
    let dir = root.join(profile_name(target.provider, &target.id));
    let live = read_login(target.provider, &profile_source(&dir), std::slice::from_ref(target))
        .filter(|l| l.id == target.id && keeps(l, target));
    if let Some(l) = &live {
        let _ = save_slot_in(vault, l);
    }
    let _ = withdraw_profile(target.provider, &dir);
    live
}

/// Where a pinned account's profile stands after `sync_profiles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileState {
    /// The profile is signed in as the account.
    Ready,
    /// The account is the CLI's default login, so the profile is empty and the folder uses that.
    AtDefault,
    /// SlopBar has no saved login for the account.
    NoLogin,
}

/// Keeps each pinned account's profile signed in, or empty while the account is the default
/// login somewhere (`active`), syncing renewals made in profiles back into the vault. Profiles
/// no longer pinned lose their login.
pub fn sync_profiles(wanted: &[(Provider, String)], active: &Active) -> HashMap<(Provider, String), ProfileState> {
    let (Some(vault), Some(root)) = (vault_dir(), profiles_root()) else { return HashMap::new() };
    if wanted.is_empty() && !root.exists() {
        return HashMap::new();
    }
    let _guard = VAULT.lock().unwrap();
    sync_profiles_in(&vault, &root, wanted, active)
}

fn sync_profiles_in(vault: &Path, root: &Path, wanted: &[(Provider, String)], active: &Active) -> HashMap<(Provider, String), ProfileState> {
    let slots = load_slots_in(vault);
    let saved = |provider: Provider, id: &str| slots.iter().find(|s| s.provider == provider && s.id == id);
    let mut states = HashMap::new();
    for (provider, id) in wanted {
        let key = (*provider, id.clone());
        if states.contains_key(&key) {
            continue;
        }
        let dir = root.join(profile_name(*provider, id));
        let live = read_login(*provider, &profile_source(&dir), &slots);
        let mut current = saved(*provider, id).cloned();
        match &live {
            Some(l) if l.id == *id => {
                if current.as_ref().map_or(true, |s| keeps(l, s)) && save_slot_in(vault, l).is_ok() {
                    current = Some(l.clone());
                }
            }
            // Someone signed in to another account here: keep it, like any login SlopBar sees.
            Some(l) => { let _ = save_slot_in(vault, l); }
            None => {}
        }
        let state = if active.contains_key(&key) {
            let _ = withdraw_profile(*provider, &dir);
            ProfileState::AtDefault
        } else if let Some(slot) = current {
            let signed_in = live.as_ref().is_some_and(|l| l.id == *id && refresh_token(l) == refresh_token(&slot));
            if signed_in || write_profile(&slot, &dir).is_ok() { ProfileState::Ready } else { ProfileState::NoLogin }
        } else {
            ProfileState::NoLogin
        };
        states.insert(key, state);
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(provider) = Provider::ALL.into_iter().find(|p| name.starts_with(&format!("{}-", p.key()))) else { continue };
            if wanted.iter().any(|(p, id)| profile_name(*p, id) == name) {
                continue;
            }
            let dir = entry.path();
            if let Some(l) = read_login(provider, &profile_source(&dir), &slots) {
                // Only into an account the vault still has: a removed account stays removed.
                if saved(provider, &l.id).is_some_and(|s| keeps(&l, s)) {
                    let _ = save_slot_in(vault, &l);
                }
            }
            let _ = withdraw_profile(provider, &dir);
        }
    }
    states
}

// ---------------------------------------------------------------------------------------------
// Adding an account

fn executable(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "claude",
        Provider::Codex => "codex",
    }
}

/// Whether the CLI can actually be run at `source`. A config dir alone isn't enough: editor
/// extensions and desktop apps create `~/.codex` or `~/.claude` without installing the CLI.
fn has_cli(provider: Provider, source: &Source) -> bool {
    let exe = executable(provider);
    let status = match &source.distro {
        // A login shell, so ~/.local/bin and version managers are on PATH.
        Some(distro) => sources::command("wsl.exe")
            .args(["-d", distro, "--", "bash", "-lc", &format!("command -v {exe}")])
            .output(),
        None => sources::command("where.exe").arg(exe).output(),
    };
    status.is_ok_and(|o| o.status.success())
}

/// Where to run a sign-in: a source that has the CLI, preferring one where it's signed in.
fn login_source(provider: Provider, sources: &[Source]) -> Option<&Source> {
    let with_cli: Vec<&Source> = sources.iter().filter(|s| has_cli(provider, s)).collect();
    with_cli
        .iter()
        .find(|s| read_login(provider, s, &[]).is_some())
        .or_else(|| with_cli.first())
        .copied()
}

/// Opens a terminal that runs the CLI's own sign-in in an isolated temporary config dir, then
/// imports the account once the sign-in finishes. The current login is never touched.
pub fn start_login(provider: Provider, sources: &[Source]) -> Result<LoginJob, String> {
    start_login_observed(provider, sources, None)
}

pub fn start_login_observed(provider: Provider, sources: &[Source], diagnostics: Option<crate::diagnostics::Context>) -> Result<LoginJob, String> {
    use crate::diagnostics::LoginStage;
    let source = login_source(provider, sources);
    if let Some(d) = &diagnostics { d.login(LoginStage::CliDiscovery, source.is_some(), source.is_some_and(|s| s.distro.is_some()), None, 0); }
    let source = source.ok_or("CLI not found")?.clone();
    static LOGIN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = format!(".slopbar-login-{}-{}-{}", std::process::id(), chrono::Utc::now().timestamp_millis(), LOGIN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let cmd = crate::login::action(provider, false);
    let dir = source.home.join(&name);
    // Never reuse a directory (or a symlink) left by an earlier/crashed sign-in.
    let made = std::fs::create_dir(&dir);
    if let Some(d) = &diagnostics { d.login(LoginStage::Directory, made.is_ok(), source.distro.is_some(), None, 0); }
    made.map_err(|_| "monitor_storage".to_string())?;
    // Keep an isolated Codex login out of the machine's shared credential-store entry.
    // A config file also avoids different cmd.exe / bash quoting rules for TOML strings.
    if provider == Provider::Codex
        && std::fs::write(dir.join("config.toml"), "cli_auth_credentials_store = \"file\"\n").is_err() {
        if let Some(d) = &diagnostics { d.login(LoginStage::Prepare, false, source.distro.is_some(), None, 0); }
        let _ = std::fs::remove_dir_all(&dir);
        return Err("monitor_storage".into());
    }

    let prepared = crate::login::prepare(&dir, provider, cmd);
    if let Some(d) = &diagnostics { d.login(LoginStage::Prepare, prepared.is_ok(), source.distro.is_some(), None, 0); }
    if prepared.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("monitor_storage".into());
    }
    let native = format!("{}/{name}", source.native_home);
    let mut term = crate::login::command(provider, &source, &dir, &native, cmd, true);
    let (child, tree) = match crate::login::ProcessTree::spawn(&mut term, true) {
        Ok(process) => process,
        Err(_) => {
            if let Some(d) = &diagnostics { d.login(LoginStage::ProcessStart, false, source.distro.is_some(), None, 0); }
            let _ = std::fs::remove_dir_all(&dir);
            return Err("monitor_login_failed".into());
        }
    };
    if let Some(d) = &diagnostics { d.login(LoginStage::ProcessStart, true, source.distro.is_some(), None, 0); }

    let login = Source {
        label: source.label.clone(),
        home: dir.clone(),
        claude_dir: Some(dir.clone()),
        codex_dir: Some(dir.clone()),
        distro: source.distro.clone(),
        native_home: source.native_home.clone(),
    };
    Ok(LoginJob { provider, source: login, dir, native, started: Instant::now(), child, tree: Some(tree), diagnostics, finished: false })
}

pub struct LoginJob {
    diagnostics: Option<crate::diagnostics::Context>,
    finished: bool,
    tree: Option<crate::login::ProcessTree>,
    native: String,
    child: std::process::Child,
    provider: Provider,
    source: Source,
    dir: PathBuf,
    started: Instant,
}

impl LoginJob {
    /// Polls until the sign-in has produced a login, then imports it and cleans up.
    /// Gives up after 15 minutes (the temp dir is removed either way).
    pub async fn wait(self, client: &reqwest::Client) -> Option<Slot> {
        let slot = self.wait_isolated(client).await?;
        let _guard = VAULT.lock().unwrap();
        save_slot(&slot).ok()?;
        Some(slot)
    }

    /// Complete a separate login without ever importing it into the switching vault.
    pub async fn wait_isolated(mut self, client: &reqwest::Client) -> Option<Slot> {
        use crate::diagnostics::LoginStage;
        let result = loop {
            if self.started.elapsed() > Duration::from_secs(15 * 60) {
                self.record_login(LoginStage::Timeout, false, None);
                break None;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            // Wait for successful CLI exit, not the first partially written credential file.
            match self.child.try_wait() {
                Ok(Some(status)) => { self.record_login(LoginStage::ProcessExit, status.success(), status.code()); if !status.success() { break None; } },
                Ok(None) => continue,
                _ => { self.record_login(LoginStage::ProcessExit, false, None); break None; },
            }
            let slot = read_login(self.provider, &self.source, &[]);
            self.record_login(LoginStage::Credentials, slot.is_some(), None);
            let Some(mut slot) = slot else { break None };
            if slot.provider == Provider::Claude {
                // A monitor must be bound to the server-verified account, not a config label.
                let verified = with_claude_profile(client, slot).await;
                self.record_login(LoginStage::Profile, verified.is_some(), None);
                let Some(verified) = verified else { break None };
                slot = verified;
            }
            break Some(slot);
        };
        // Close the process tree before reading/removing any remaining login state.
        drop(self.tree.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Credentials may reach the vault, main CLI, or monitor only after a
        // durable handoff marker prevents later cleanup from logging them out.
        let result = result.filter(|_| {
            let ok = crate::login::handoff(&self.dir, self.provider).is_ok();
            self.record_login(LoginStage::Handoff, ok, None); ok
        });
        if result.is_none() {
            let (provider, source, dir, native) = (self.provider, self.source.clone(), self.dir.clone(), self.native.clone());
            let _ = tokio::task::spawn_blocking(move || crate::login::logout(provider, &source, &dir, &native)).await;
        }
        let removed = std::fs::remove_dir_all(&self.dir);
        self.record_login(LoginStage::FolderCleanup, removed.is_ok(), None);
        self.finished = true;
        result
    }

    fn record_login(&self, stage: crate::diagnostics::LoginStage, ok: bool, exit_code: Option<i32>) {
        if let Some(d) = &self.diagnostics { d.login(stage, ok, self.source.distro.is_some(), exit_code, crate::diagnostics::millis(self.started)); }
    }
}

impl Drop for LoginJob {
    fn drop(&mut self) {
        if !self.finished { self.record_login(crate::diagnostics::LoginStage::Cancelled, false, None); }
        drop(self.tree.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A cancelled task leaves the marked folder for startup logout/cleanup.
    }
}

/// A Claude account's identity, in `.claude.json`'s `oauthAccount` shape.
async fn claude_profile(client: &reqwest::Client, token: &str) -> Option<Value> {
    let p: Value = client
        .get("https://api.anthropic.com/api/oauth/profile")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    p["account"]["uuid"].as_str()?;
    Some(json!({
        "accountUuid": p["account"]["uuid"],
        "emailAddress": p["account"]["email"],
        "organizationUuid": p["organization"]["uuid"],
        "organizationName": p["organization"]["name"],
        "displayName": p["account"]["display_name"],
    }))
}

/// Replaces a new Claude login's identity with the profile API's answer.
async fn with_claude_profile(client: &reqwest::Client, slot: Slot) -> Option<Slot> {
    let (token, _) = slot.access()?;
    let account = claude_profile(client, &token).await?;
    claude_slot(slot.data["credentials"].clone(), Some(account))
}

#[cfg(test)]
mod billing_tests {
    use super::*;

    #[test]
    fn billing_hints_come_from_the_saved_login_without_identities() {
        let claims = json!({ "email": "test@example.invalid", "https://api.openai.com/auth": {
            "chatgpt_plan_type": "plus", "chatgpt_account_id": "acct",
            "chatgpt_subscription_active_start": "2026-09-18T21:26:13+00:00",
            "chatgpt_subscription_active_until": "2026-10-18T21:26:13+00:00",
            "chatgpt_subscription_last_checked": "2026-10-09T16:01:36.550331+00:00",
        } });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        let codex = codex_slot(json!({ "tokens": { "access_token": "a", "id_token": format!("h.{payload}.s") } })).unwrap();
        let b = codex.billing();
        assert_eq!(b.active_until.unwrap().to_rfc3339(), "2026-10-18T21:26:13+00:00");
        assert!(b.started_at.is_some() && b.checked_at.is_some() && b.tier.is_none());

        let claude = claude_slot(json!({ "claudeAiOauth": { "accessToken": "a", "subscriptionType": "max", "rateLimitTier": "default_claude_max_5x" } }),
            Some(json!({ "accountUuid": "u", "emailAddress": "test@example.invalid", "billingType": "stripe_subscription",
                "subscriptionCreatedAt": "2026-08-03T09:00:00.000000Z" }))).unwrap();
        let b = claude.billing();
        assert_eq!((b.tier.as_deref(), b.billing_type.as_deref()), (Some("default_claude_max_5x"), Some("stripe_subscription")));
        assert_eq!(b.started_at.unwrap().to_rfc3339(), "2026-08-03T09:00:00+00:00");
        let text = serde_json::to_string(&b).unwrap();
        assert!(!text.contains("example.invalid") && !text.contains("\"a\""), "{text}");
        assert!(codex_slot(json!({ "tokens": { "access_token": "a" } })).unwrap().billing().active_until.is_none());
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::test_support::TempDir;

    fn claude(refresh: &str, expires: i64) -> Slot {
        claude_slot(
            json!({ "claudeAiOauth": { "accessToken": format!("at-{refresh}"), "refreshToken": refresh, "expiresAt": expires }, "organizationUuid": "org-1" }),
            Some(json!({ "accountUuid": "user-1", "organizationUuid": "org-1", "emailAddress": "test@example.invalid" })),
        ).unwrap()
    }

    fn codex(refresh: &str, last: &str) -> Slot {
        let claims = json!({ "email": "test@example.invalid", "https://api.openai.com/auth": { "chatgpt_user_id": "user-2", "chatgpt_account_id": "acct-2" } });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        codex_slot(json!({ "last_refresh": last, "tokens": { "access_token": format!("at-{refresh}"), "refresh_token": refresh, "id_token": format!("h.{payload}.s") } })).unwrap()
    }

    fn stored(vault: &Path, slot: &Slot) -> Option<String> {
        load_slots_in(vault).into_iter().find(|s| s.id == slot.id).and_then(|s| refresh_token(&s).map(String::from))
    }

    fn in_profile(root: &Path, slot: &Slot) -> Option<String> {
        read_login(slot.provider, &profile_source(&root.join(profile_name(slot.provider, &slot.id))), &[]).and_then(|s| refresh_token(&s).map(String::from))
    }

    #[test]
    fn a_pinned_account_not_in_use_is_signed_in_to_its_profile_and_renewals_come_back() {
        let (vault, root) = (TempDir::new(), TempDir::new());
        let slot = claude("r1", 1_000);
        save_slot_in(&vault.0, &slot).unwrap();
        let wanted = [(Provider::Claude, slot.id.clone())];
        let states = sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new());
        assert_eq!(states[&wanted[0]], ProfileState::Ready);
        assert_eq!(in_profile(&root.0, &slot).as_deref(), Some("r1"));
        let dir = root.0.join(profile_name(Provider::Claude, &slot.id));
        let config = read_json(&dir.join(".claude.json")).unwrap();
        assert_eq!(config["oauthAccount"]["accountUuid"], "user-1", "the profile knows whose login it holds");

        // The CLI in the project renews its login: the vault takes the newer one.
        write_login(&claude("r2", 2_000), &profile_source(&dir)).unwrap();
        sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new());
        assert_eq!(stored(&vault.0, &slot).as_deref(), Some("r2"));
        assert_eq!(in_profile(&root.0, &slot).as_deref(), Some("r2"));

        // A newer login in the vault (renewed while it was the default) replaces an older profile copy.
        save_slot_in(&vault.0, &claude("r3", 3_000)).unwrap();
        sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new());
        assert_eq!(in_profile(&root.0, &slot).as_deref(), Some("r3"));
    }

    #[test]
    fn an_account_in_use_by_the_cli_is_never_also_signed_in_to_its_profile() {
        let (vault, root) = (TempDir::new(), TempDir::new());
        let slot = claude("r1", 1_000);
        save_slot_in(&vault.0, &slot).unwrap();
        let wanted = [(Provider::Claude, slot.id.clone())];
        sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new());
        let dir = root.0.join(profile_name(Provider::Claude, &slot.id));
        write_login(&claude("r2", 2_000), &profile_source(&dir)).unwrap();

        let mut active = Active::new();
        active.insert((Provider::Claude, slot.id.clone()), vec!["Windows".into()]);
        let states = sync_profiles_in(&vault.0, &root.0, &wanted, &active);
        assert_eq!(states[&wanted[0]], ProfileState::AtDefault);
        assert!(!profile_login_file(Provider::Claude, &dir).exists(), "one live copy only");
        assert_eq!(stored(&vault.0, &slot).as_deref(), Some("r2"), "the profile's renewal was kept first");
        assert!(dir.join(".claude.json").exists(), "the rest of the profile stays");

        // Switched away again: the profile is signed in from the vault.
        assert_eq!(sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new())[&wanted[0]], ProfileState::Ready);
        assert_eq!(in_profile(&root.0, &slot).as_deref(), Some("r2"));
    }

    #[test]
    fn switching_to_a_pinned_account_takes_its_newer_login_back_from_the_profile() {
        let (vault, root) = (TempDir::new(), TempDir::new());
        let slot = claude("r1", 1_000);
        save_slot_in(&vault.0, &slot).unwrap();
        let dir = root.0.join(profile_name(Provider::Claude, &slot.id));
        write_profile(&claude("r2", 2_000), &dir).unwrap();
        let taken = reclaim_profile(&vault.0, &root.0, &slot).expect("the profile's login is newer");
        assert_eq!(refresh_token(&taken), Some("r2"));
        assert_eq!(stored(&vault.0, &slot).as_deref(), Some("r2"));
        assert!(!profile_login_file(Provider::Claude, &dir).exists());

        // An older copy is dropped and the vault's login is used.
        write_profile(&claude("r0", 500), &dir).unwrap();
        assert!(reclaim_profile(&vault.0, &root.0, &claude("r2", 2_000)).is_none());
        assert!(!profile_login_file(Provider::Claude, &dir).exists());
        // No profile at all is fine.
        assert!(reclaim_profile(&vault.0, &TempDir::new().0, &slot).is_none());
    }

    #[test]
    fn unpinned_profiles_lose_their_login_and_removed_accounts_stay_removed() {
        let (vault, root) = (TempDir::new(), TempDir::new());
        let kept = claude("r1", 1_000);
        save_slot_in(&vault.0, &kept).unwrap();
        let kept_dir = root.0.join(profile_name(Provider::Claude, &kept.id));
        write_profile(&claude("r2", 2_000), &kept_dir).unwrap();
        let gone = codex("c1", "2026-10-01T00:00:00Z");
        let gone_dir = root.0.join(profile_name(Provider::Codex, &gone.id));
        write_profile(&gone, &gone_dir).unwrap();

        sync_profiles_in(&vault.0, &root.0, &[], &Active::new());
        assert!(!profile_login_file(Provider::Claude, &kept_dir).exists() && !profile_login_file(Provider::Codex, &gone_dir).exists());
        assert_eq!(stored(&vault.0, &kept).as_deref(), Some("r2"), "a renewal is not lost on unpinning");
        assert!(stored(&vault.0, &gone).is_none(), "an account missing from the vault is not brought back");
        assert!(gone_dir.join("config.toml").exists());
    }

    #[test]
    fn codex_profiles_use_their_own_auth_file_and_renewal_time() {
        let (vault, root) = (TempDir::new(), TempDir::new());
        let slot = codex("c1", "2026-10-01T00:00:00Z");
        save_slot_in(&vault.0, &slot).unwrap();
        let wanted = [(Provider::Codex, slot.id.clone())];
        assert_eq!(sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new())[&wanted[0]], ProfileState::Ready);
        let dir = root.0.join(profile_name(Provider::Codex, &slot.id));
        assert!(std::fs::read_to_string(dir.join("config.toml")).unwrap().contains("cli_auth_credentials_store = \"file\""));
        assert_eq!(in_profile(&root.0, &slot).as_deref(), Some("c1"));
        // A stale profile copy (older renewal) never overwrites the vault.
        save_slot_in(&vault.0, &codex("c2", "2026-10-05T00:00:00Z")).unwrap();
        write_login(&codex("c0", "2026-09-01T00:00:00Z"), &profile_source(&dir)).unwrap();
        sync_profiles_in(&vault.0, &root.0, &wanted, &Active::new());
        assert_eq!(stored(&vault.0, &slot).as_deref(), Some("c2"));
        assert_eq!(in_profile(&root.0, &slot).as_deref(), Some("c2"));
        // No saved login: nothing to sign in with.
        let none = [(Provider::Codex, "missing".to_string())];
        assert_eq!(sync_profiles_in(&vault.0, &root.0, &none, &Active::new())[&none[0]], ProfileState::NoLogin);
    }

    #[test]
    fn profile_names_are_stable_and_hide_the_account_id() {
        assert_eq!(profile_name(Provider::Claude, "user-1:org-1"), profile_name(Provider::Claude, "user-1:org-1"));
        assert_ne!(profile_name(Provider::Claude, "a"), profile_name(Provider::Codex, "a"));
        assert!(!profile_name(Provider::Codex, "user-2:acct-2").contains("user-2"));
    }

    // -- The encrypted vault ----------------------------------------------------------------

    /// A slot as releases before the encrypted vault stored it.
    fn plain(vault: &Path, slot: &Slot) -> PathBuf {
        let path = slot_path_in(vault, slot.provider, &slot.id).with_extension(PLAIN_EXT);
        std::fs::write(&path, serde_json::to_string_pretty(slot).unwrap()).unwrap();
        path
    }

    fn files(vault: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(vault).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn real_migrate(vault: &Path) -> VaultMigration {
        migrate_vault_in(vault, crate::private_store::save_secret, crate::private_store::load_secret)
    }

    #[test]
    fn migration_encrypts_every_plaintext_slot_and_leaves_no_token_on_disk() {
        let vault = TempDir::new();
        let (a, b) = (claude("SYNTH-REFRESH-ALPHA", 1_000), codex("SYNTH-REFRESH-BRAVO", "2026-10-01T00:00:00Z"));
        plain(&vault.0, &a);
        plain(&vault.0, &b);
        assert_eq!(real_migrate(&vault.0), VaultMigration { migrated: 2, failed: 0, unreadable: 0 });
        assert!(files(&vault.0).iter().all(|f| f.ends_with(".slot")), "{:?}", files(&vault.0));
        assert_eq!(stored(&vault.0, &a).as_deref(), Some("SYNTH-REFRESH-ALPHA"));
        assert_eq!(stored(&vault.0, &b).as_deref(), Some("SYNTH-REFRESH-BRAVO"));
        // DPAPI on Windows; on other platforms save_secret is a private (0600) plaintext file.
        #[cfg(windows)]
        for entry in std::fs::read_dir(&vault.0).unwrap().flatten() {
            let bytes = std::fs::read(entry.path()).unwrap();
            for token in ["SYNTH-REFRESH-ALPHA", "SYNTH-REFRESH-BRAVO", "at-SYNTH", "test@example.invalid"] {
                assert!(!bytes.windows(token.len()).any(|w| w == token.as_bytes()), "{token} in {:?}", entry.path());
            }
        }
        // Running again finds nothing to do, and saves keep writing encrypted slots only.
        assert!(real_migrate(&vault.0).is_empty());
        save_slot_in(&vault.0, &claude("SYNTH-REFRESH-CHARLIE", 2_000)).unwrap();
        assert_eq!(stored(&vault.0, &a).as_deref(), Some("SYNTH-REFRESH-CHARLIE"));
        assert_eq!(files(&vault.0).len(), 2);
    }

    #[test]
    fn a_failed_migration_keeps_the_plaintext_slot_and_it_is_still_read() {
        let vault = TempDir::new();
        let (a, b) = (claude("SYNTH-REFRESH-ALPHA", 1_000), codex("SYNTH-REFRESH-BRAVO", "2026-10-01T00:00:00Z"));
        let (pa, pb) = (plain(&vault.0, &a), plain(&vault.0, &b));
        // Encrypting the Claude slot fails outright.
        let failing = |path: &Path, bytes: &[u8]| {
            if path.to_string_lossy().contains("claude-") { Err(std::io::Error::other("disk full")) } else { crate::private_store::save_secret(path, bytes) }
        };
        let result = migrate_vault_in(&vault.0, failing, crate::private_store::load_secret);
        assert_eq!(result, VaultMigration { migrated: 1, failed: 1, unreadable: 0 });
        assert!(pa.exists() && !pa.with_extension(SLOT_EXT).exists());
        assert!(!pb.exists() && pb.with_extension(SLOT_EXT).exists());
        assert_eq!(load_slots_in(&vault.0).len(), 2);
        assert_eq!(stored(&vault.0, &a).as_deref(), Some("SYNTH-REFRESH-ALPHA"));

        // Writing works but the copy does not read back the same: the copy goes, the plaintext stays.
        let garbled = |path: &Path| crate::private_store::load_secret(path).map(|mut b| { b.reverse(); b });
        let result = migrate_vault_in(&vault.0, crate::private_store::save_secret, garbled);
        assert_eq!(result, VaultMigration { migrated: 0, failed: 1, unreadable: 0 });
        assert!(pa.exists() && !pa.with_extension(SLOT_EXT).exists());
        assert_eq!(stored(&vault.0, &a).as_deref(), Some("SYNTH-REFRESH-ALPHA"));

        // The next start succeeds.
        assert_eq!(real_migrate(&vault.0), VaultMigration { migrated: 1, failed: 0, unreadable: 0 });
        assert_eq!(load_slots_in(&vault.0).len(), 2);
    }

    #[test]
    fn unreadable_files_are_left_alone_and_never_stop_the_others() {
        let vault = TempDir::new();
        let a = claude("SYNTH-REFRESH-ALPHA", 1_000);
        plain(&vault.0, &a);
        std::fs::write(vault.0.join("claude-broken.json"), "{ not json").unwrap();
        std::fs::write(vault.0.join("codex-other.json"), r#"{"something":"else"}"#).unwrap();
        std::fs::write(vault.0.join("codex-garbage.slot"), b"\x00\x01not dpapi").unwrap();
        assert_eq!(real_migrate(&vault.0), VaultMigration { migrated: 1, failed: 0, unreadable: 2 });
        assert_eq!(std::fs::read_to_string(vault.0.join("claude-broken.json")).unwrap(), "{ not json");
        assert!(vault.0.join("codex-garbage.slot").exists());
        let slots = load_slots_in(&vault.0);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].id, a.id);
    }

    #[test]
    fn a_newer_plaintext_slot_wins_until_the_next_save_replaces_it() {
        let vault = TempDir::new();
        save_slot_in(&vault.0, &claude("SYNTH-REFRESH-OLD", 1_000)).unwrap();
        let encrypted = slot_path_in(&vault.0, Provider::Claude, &claude("x", 0).id);
        // An earlier release, run after this one, renewed the login and wrote it as plaintext.
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&encrypted).unwrap().set_modified(old).unwrap();
        let newer = plain(&vault.0, &claude("SYNTH-REFRESH-NEW", 2_000));
        assert_eq!(load_slots_in(&vault.0).len(), 1);
        assert_eq!(stored(&vault.0, &claude("x", 0)).as_deref(), Some("SYNTH-REFRESH-NEW"));
        // Migration takes the newer plaintext login, not the older encrypted one.
        assert_eq!(real_migrate(&vault.0).migrated, 1);
        assert!(!newer.exists());
        assert_eq!(stored(&vault.0, &claude("x", 0)).as_deref(), Some("SYNTH-REFRESH-NEW"));
        // A save also removes a leftover plaintext copy.
        plain(&vault.0, &claude("SYNTH-REFRESH-STALE", 500));
        save_slot_in(&vault.0, &claude("SYNTH-REFRESH-LATEST", 3_000)).unwrap();
        assert_eq!(files(&vault.0).len(), 1);
        assert_eq!(stored(&vault.0, &claude("x", 0)).as_deref(), Some("SYNTH-REFRESH-LATEST"));
    }
}
