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

    fn key(self) -> &'static str {
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
    for token in sources.iter().filter_map(claude_token) {
        let key = stable_hash(&token);
        let known = CLAUDE_IDENTITY.lock().unwrap().as_ref().is_some_and(|m| m.contains_key(&key));
        if known {
            continue;
        }
        if let Some(account) = claude_profile(client, &token).await {
            CLAUDE_IDENTITY.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, account);
        }
    }
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

fn vault_dir() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("accounts"))
}

fn slot_path(provider: Provider, id: &str) -> Option<PathBuf> {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    vault_dir().map(|d| d.join(format!("{}-{safe}.json", provider.key())))
}

pub fn load_slots() -> Vec<Slot> {
    let Some(dir) = vault_dir() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut slots: Vec<Slot> = entries
        .flatten()
        .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    slots.sort_by(|a, b| (a.provider.key(), &a.email).cmp(&(b.provider.key(), &b.email)));
    slots
}

fn save_slot(slot: &Slot) -> std::io::Result<()> {
    let path = slot_path(slot.provider, &slot.id).ok_or_else(|| std::io::Error::other("no config dir"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_json(&path, &serde_json::to_value(slot)?)
}

pub fn remove_slot(provider: Provider, id: &str) -> std::io::Result<()> {
    match slot_path(provider, id) {
        Some(p) => std::fs::remove_file(p),
        None => Ok(()),
    }
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
    let target = load_slots()
        .into_iter()
        .find(|s| s.provider == provider && s.id == id)
        .ok_or("account not found")?;
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
    let source = login_source(provider, sources).ok_or("CLI not found")?.clone();
    let name = format!(".slopbar-login-{}", chrono::Utc::now().timestamp_millis());
    let (var, cmd) = match provider {
        Provider::Claude => ("CLAUDE_CONFIG_DIR", "claude auth login"),
        Provider::Codex => ("CODEX_HOME", "codex login"),
    };
    let dir = source.home.join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // A new console window runs the sign-in; it closes on success and stays open on failure.
    let mut term = match &source.distro {
        Some(distro) => {
            let native = format!("{}/{name}", source.native_home);
            // -l loads the login profile, where ~/.local/bin (the CLIs' usual home) joins PATH.
            let script = format!(
                "{var}='{native}' {cmd} && sleep 1 || {{ echo; read -rp 'Press Enter to close'; }}"
            );
            let mut c = std::process::Command::new("wsl.exe");
            c.args(["-d", distro, "--", "bash", "-lic", &script]);
            c
        }
        None => {
            let mut c = std::process::Command::new("cmd.exe");
            c.env(var, &dir).args(["/c", &format!("{cmd} || pause")]);
            // The app may have been started from a \\wsl.localhost path, which cmd can't use.
            if let Some(home) = dirs::home_dir() {
                c.current_dir(home);
            }
            c
        }
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        term.creation_flags(CREATE_NEW_CONSOLE);
    }
    term.spawn().map_err(|e| e.to_string())?;

    let login = Source {
        label: source.label.clone(),
        home: dir.clone(),
        claude_dir: Some(dir.clone()),
        codex_dir: Some(dir.clone()),
        distro: source.distro.clone(),
        native_home: source.native_home.clone(),
    };
    Ok(LoginJob { provider, source: login, dir, started: Instant::now() })
}

pub struct LoginJob {
    provider: Provider,
    source: Source,
    dir: PathBuf,
    started: Instant,
}

impl LoginJob {
    /// Polls until the sign-in has produced a login, then imports it and cleans up.
    /// Gives up after 15 minutes (the temp dir is removed either way).
    pub async fn wait(self, client: &reqwest::Client) -> Option<Slot> {
        let result = loop {
            if self.started.elapsed() > Duration::from_secs(15 * 60) {
                break None;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(mut slot) = read_login(self.provider, &self.source, &[]) else { continue };
            // Let the CLI finish writing (Claude adds account details right after the tokens).
            tokio::time::sleep(Duration::from_secs(3)).await;
            slot = read_login(self.provider, &self.source, &[]).unwrap_or(slot);
            if slot.provider == Provider::Claude {
                slot = with_claude_profile(client, slot).await;
            }
            let _guard = VAULT.lock().unwrap();
            let _ = save_slot(&slot);
            break Some(slot);
        };
        let _ = std::fs::remove_dir_all(&self.dir);
        result
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
async fn with_claude_profile(client: &reqwest::Client, slot: Slot) -> Slot {
    let Some((token, _)) = slot.access() else { return slot };
    match claude_profile(client, &token).await {
        Some(account) => claude_slot(slot.data["credentials"].clone(), Some(account)).unwrap_or(slot),
        None => slot,
    }
}
