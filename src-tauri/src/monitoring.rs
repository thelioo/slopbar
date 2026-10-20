//! Renewable sessions owned only by the monitor. Never rotate a copied CLI refresh token.
use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::accounts::{Provider, Slot};
use crate::private_store;
use crate::diagnostics::{self, Decision, Event, RefreshOutcome, RefreshReason, RevokeOutcome, RevokeReason};

const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

pub fn key(slot: &Slot) -> String {
    format!("{}:{}", slot.provider.name(), slot.id)
}

/// Reserve connect/remove/disconnect for an account through their final disk mutation.
/// The guard also releases reservations when a task fails or is cancelled.
#[derive(Default)]
pub struct Operations(std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>);
pub struct Operation {
    key: String,
    operations: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}
impl Operations {
    pub fn begin(&self, provider: Provider, id: &str) -> Result<Operation, String> {
        let key = format!("{}:{id}", provider.name());
        if !self.0.lock().unwrap().insert(key.clone()) {
            return Err("monitor_busy".into());
        }
        Ok(Operation {
            key,
            operations: self.0.clone(),
        })
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        self.operations.lock().unwrap().remove(&self.key);
    }
}

// Deliberately no Debug: token values must not enter logs or UI errors.
#[derive(Clone, Serialize, Deserialize)]
struct Session {
    version: u8,
    provider: Provider,
    id: String,
    access_token: String,
    refresh_token: String,
    account_id: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    refresh_expires_at: Option<DateTime<Utc>>,
    // Persist the replay budget and backoff, so restarting cannot replay indefinitely.
    #[serde(default)]
    uncertain_attempts: u8,
    #[serde(default)]
    uncertain_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    reconnect: bool,
    // A usage 401 must remain a rejection during backoff and across restart.
    #[serde(default)]
    access_rejected: bool,
}

struct State {
    session: Session,
    // Keep a rotated token in memory if its disk write fails. Retry the write before any
    // subsequent network request, rather than refreshing the old on-disk token again.
    dirty: bool,
    retry_at: Option<DateTime<Utc>>,
    error: Option<String>,
}

pub struct Monitor {
    dir: PathBuf,
    // Serializes refresh, reconnect and removal. The desktop is a single-instance app.
    sessions: Mutex<HashMap<String, State>>,
    diagnostics: Option<std::sync::Arc<diagnostics::Recorder>>,
}

impl Monitor {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            sessions: Mutex::new(HashMap::new()),
            diagnostics: None,
        }
    }

    pub fn with_diagnostics(mut self, recorder: std::sync::Arc<diagnostics::Recorder>) -> Self {
        self.diagnostics = Some(recorder);
        self
    }

    pub(crate) fn record(&self, slot: &Slot, event: Event) {
        if let Some(r) = &self.diagnostics { r.record(Some(slot.provider), Some(&slot.id), event); }
    }

    fn decision(&self, slot: &Slot, state: &State, decision: Decision, usable: bool) {
        self.record(slot, Event::Token { decision, access_expires_at: state.session.expires_at,
            retry_at: state.session.uncertain_retry_at.or(state.retry_at),
            uncertain_attempts: state.session.uncertain_attempts, access_rejected: state.session.access_rejected, usable });
    }

    fn path(&self, slot: &Slot) -> PathBuf {
        let name = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key(slot));
        self.dir.join(format!("{name}.session"))
    }

    pub fn connected(&self, slot: &Slot) -> bool {
        self.path(slot).is_file()
    }

    /// Cleanup must not revoke any login we own. An unreadable store disables
    /// cleanup revocation instead of assuming its tokens are unprotected.
    pub async fn protected_tokens(&self, slots: &[Slot]) -> Option<crate::login::ProtectedTokens> {
        let sessions = self.sessions.lock().await;
        self.protected_tokens_locked(slots, &sessions)
    }

    fn protected_tokens_locked(
        &self,
        slots: &[Slot],
        sessions: &HashMap<String, State>,
    ) -> Option<crate::login::ProtectedTokens> {
        let mut tokens: crate::login::ProtectedTokens = slots
            .iter()
            .filter_map(|s| refresh_token(s).map(|t| (s.provider, t.to_owned())))
            .collect();
        tokens.extend(
            sessions
                .values()
                .map(|s| (s.session.provider, s.session.refresh_token.clone())),
        );
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(tokens),
            Err(_) => return None,
        };
        for entry in entries {
            let path = entry.ok()?.path();
            if path.extension().is_some_and(|ext| ext == "session") {
                let session: Session =
                    serde_json::from_slice(&private_store::load_secret(&path).ok()?).ok()?;
                tokens.insert((session.provider, session.refresh_token));
            }
        }
        Some(tokens)
    }

    /// A fresh, isolated CLI sign-in is required. Imported/shared CLI sessions are never
    /// promoted to renewable monitoring sessions, even if their access token still works.
    pub async fn connect_replacing(
        &self,
        expected: &Slot,
        login: &Slot,
        cli_slots: &[Slot],
        client: &reqwest::Client,
    ) -> Result<(), String> {
        self.connect_or_discard_with(expected, login, cli_slots, &Api(client, self.diagnostics.as_deref()))
            .await
    }

    async fn connect_or_discard_with(
        &self,
        expected: &Slot,
        login: &Slot,
        cli_slots: &[Slot],
        revoke: &impl Revoke,
    ) -> Result<(), String> {
        let result = self.connect_with(expected, login, cli_slots, revoke).await;
        if matches!(
            result.as_ref().err().map(String::as_str),
            Some("monitor_wrong_account" | "monitor_storage")
        ) {
            let known: Vec<Slot> = cli_slots
                .iter()
                .cloned()
                .chain(std::iter::once(expected.clone()))
                .collect();
            self.discard_unstored_login_with(login, &known, revoke)
                .await;
        }
        result
    }

    /// A handed-off login that was never installed must not become an orphaned
    /// provider session. Never revoke a token used by any CLI or stored monitor.
    pub async fn discard_unstored_login(
        &self,
        login: &Slot,
        cli_slots: &[Slot],
        client: &reqwest::Client,
    ) -> bool {
        self.discard_unstored_login_with(login, cli_slots, &Api(client, self.diagnostics.as_deref()))
            .await
    }

    async fn discard_unstored_login_with(
        &self,
        login: &Slot,
        cli_slots: &[Slot],
        revoke: &impl Revoke,
    ) -> bool {
        let Some(refresh) = refresh_token(login).filter(|t| !t.is_empty()) else {
            self.record(login, Event::Revoke { reason: RevokeReason::UninstalledLogin, outcome: RevokeOutcome::NoToken, elapsed_ms: 0 });
            return false;
        };
        // Hold the same lock through the ownership check and revocation. Another
        // connect cannot adopt this token between the check and the network call.
        let sessions = self.sessions.lock().await;
        let Some(protected) = self.protected_tokens_locked(cli_slots, &sessions) else {
            self.record(login, Event::Revoke { reason: RevokeReason::UninstalledLogin, outcome: RevokeOutcome::UnknownOwnership, elapsed_ms: 0 });
            return false;
        };
        if protected.contains(&(login.provider, refresh.to_owned())) {
            self.record(login, Event::Revoke { reason: RevokeReason::UninstalledLogin, outcome: RevokeOutcome::Protected, elapsed_ms: 0 });
            return false;
        }
        let started = std::time::Instant::now();
        let revoked = revoke
            .revoke(&Session {
                version: 1,
                provider: login.provider,
                id: login.id.clone(),
                access_token: String::new(),
                refresh_token: refresh.to_owned(),
                account_id: None,
                expires_at: None,
                scopes: vec![],
                refresh_expires_at: None,
                uncertain_attempts: 0,
                uncertain_retry_at: None,
                reconnect: false,
                access_rejected: false,
            })
            .await;
        self.record(login, Event::Revoke { reason: RevokeReason::UninstalledLogin,
            outcome: if revoked { RevokeOutcome::Confirmed } else { RevokeOutcome::Unconfirmed }, elapsed_ms: diagnostics::millis(started) });
        revoked
    }

    #[cfg(test)]
    pub(crate) async fn connect(
        &self,
        expected: &Slot,
        login: &Slot,
        cli_slots: &[Slot],
    ) -> Result<(), String> {
        self.connect_with(expected, login, cli_slots, &()).await
    }

    async fn connect_with(
        &self,
        expected: &Slot,
        login: &Slot,
        cli_slots: &[Slot],
        revoke: &impl Revoke,
    ) -> Result<(), String> {
        if expected.provider != login.provider
            || expected.id != login.id
            || login.id.starts_with("token:")
        {
            return Err("monitor_wrong_account".into());
        }
        let refresh = refresh_token(login)
            .filter(|t| !t.is_empty())
            .ok_or("monitor_reconnect")?;
        if cli_slots
            .iter()
            .chain(std::iter::once(expected))
            .any(|s| s.provider == login.provider && refresh_token(s) == Some(refresh))
        {
            return Err("monitor_shared_login".into());
        }
        let (access, account_id) = login.access().ok_or("monitor_reconnect")?;
        if access.is_empty() {
            return Err("monitor_reconnect".into());
        }
        let session = Session {
            version: 1,
            provider: login.provider,
            id: login.id.clone(),
            access_token: access,
            refresh_token: refresh.to_string(),
            account_id,
            expires_at: expiry(login),
            scopes: login.data["credentials"]["claudeAiOauth"]["scopes"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            refresh_expires_at: login.data["credentials"]["claudeAiOauth"]["refreshTokenExpiresAt"]
                .as_i64()
                .and_then(DateTime::from_timestamp_millis),
            uncertain_attempts: 0,
            uncertain_retry_at: None,
            reconnect: false,
            access_rejected: false,
        };
        let mut sessions = self.sessions.lock().await;
        let previous = sessions
            .get(&key(expected))
            .map(|s| s.session.clone())
            .or_else(|| {
                private_store::load_secret(&self.path(expected))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Session>(&b).ok())
            });
        self.save(expected, &session)?;
        sessions.insert(
            key(expected),
            State {
                session,
                dirty: false,
                retry_at: None,
                error: None,
            },
        );
        // Replace successfully before revoking the old chain. A failed write must
        // leave the previously connected login usable.
        if let Some(previous) = previous.filter(|s| {
            s.version == 1
                && s.provider == expected.provider
                && s.id == expected.id
                && s.refresh_token != refresh
        }) {
            let started = std::time::Instant::now();
            let revoked = revoke.revoke(&previous).await;
            self.record(expected, Event::Revoke { reason: RevokeReason::Replacement,
                outcome: if revoked { RevokeOutcome::Confirmed } else { RevokeOutcome::Unconfirmed }, elapsed_ms: diagnostics::millis(started) });
        }
        Ok(())
    }

    fn save(&self, slot: &Slot, session: &Session) -> Result<(), String> {
        let bytes = serde_json::to_vec(session).map_err(|_| "monitor_storage")?;
        let result = private_store::save_secret(&self.path(slot), &bytes).map_err(|_| "monitor_storage".into());
        if result.is_err() { self.record(slot, Event::Action { action: diagnostics::Action::MonitorSave, code: diagnostics::Code::Storage, elapsed_ms: 0 }); }
        result
    }

    #[cfg(test)]
    async fn remove(&self, slot: &Slot) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        match std::fs::remove_file(self.path(slot)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("monitor_storage".into()),
        }
        sessions.remove(&key(slot));
        Ok(())
    }

    /// Delete locally even if the provider is offline. Never revoke the imported CLI token.
    pub async fn disconnect(&self, slot: &Slot, client: &reqwest::Client) -> Result<bool, String> {
        self.disconnect_with(slot, &Api(client, self.diagnostics.as_deref())).await
    }

    async fn disconnect_with(&self, slot: &Slot, transport: &impl Revoke) -> Result<bool, String> {
        let mut sessions = self.sessions.lock().await;
        let session = if let Some(state) = sessions.get(&key(slot)) {
            Some(state.session.clone())
        } else {
            match private_store::load_secret(&self.path(slot)) {
                Ok(bytes) => serde_json::from_slice::<Session>(&bytes).ok(),
                Err(_) => None,
            }
        };
        match std::fs::remove_file(self.path(slot)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("monitor_storage".into()),
        }
        sessions.remove(&key(slot));
        let Some(session) =
            session.filter(|s| s.version == 1 && s.provider == slot.provider && s.id == slot.id)
        else {
            self.record(slot, Event::Revoke { reason: RevokeReason::Disconnect, outcome: RevokeOutcome::NoToken, elapsed_ms: 0 });
            return Ok(false);
        };
        // Hold the same lock through revocation so an in-flight refresh cannot recreate it.
        let started = std::time::Instant::now();
        let revoked = transport.revoke(&session).await;
        self.record(slot, Event::Revoke { reason: RevokeReason::Disconnect,
            outcome: if revoked { RevokeOutcome::Confirmed } else { RevokeOutcome::Unconfirmed }, elapsed_ms: diagnostics::millis(started) });
        Ok(revoked)
    }

    pub async fn token(
        &self,
        slot: &Slot,
        client: &reqwest::Client,
        rejected: Option<&str>,
    ) -> Result<(String, Option<String>), String> {
        let started = std::time::Instant::now();
        let result = self.token_with(slot, &Api(client, self.diagnostics.as_deref()), rejected, Utc::now()).await;
        if result.is_err() { self.record(slot, Event::Action { action: diagnostics::Action::TokenRead,
            code: diagnostics::Code::from_error(result.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) }); }
        result
    }

    /// A freshly renewed token was also rejected. Persist this terminal state so every
    /// poll (and restart) does not rotate another token. Do not invalidate a newer login.
    pub async fn reject(&self, slot: &Slot, token: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get_mut(&key(slot)) {
            if state.session.access_token == token {
                state.session.reconnect = true;
                state.dirty = true;
                self.save(slot, &state.session)?;
                state.dirty = false;
            }
        }
        Ok(())
    }

    async fn token_with(
        &self,
        slot: &Slot,
        transport: &impl Refresh,
        rejected: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(String, Option<String>), String> {
        let mut sessions = self.sessions.lock().await;
        let key = key(slot);
        if !sessions.contains_key(&key) {
            let bytes = private_store::load_secret(&self.path(slot)).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "monitor_setup"
                } else {
                    "monitor_storage"
                }
            })?;
            let session: Session = serde_json::from_slice(&bytes).map_err(|_| "monitor_storage")?;
            if session.version != 1 || session.provider != slot.provider || session.id != slot.id {
                return Err("monitor_wrong_account".into());
            }
            sessions.insert(
                key.clone(),
                State {
                    session,
                    dirty: false,
                    retry_at: None,
                    error: None,
                },
            );
        }
        let state = sessions.get_mut(&key).unwrap();
        if state.dirty {
            self.save(slot, &state.session)?;
            state.dirty = false;
        }
        if state.session.reconnect {
            self.decision(slot, state, Decision::Reconnect, false);
            return Err("monitor_reconnect".into());
        }
        let expired = state
            .session
            .expires_at
            .is_some_and(|t| t <= now + chrono::Duration::seconds(60));
        // A late rejection of an old token cannot invalidate a newer rotation.
        let rejected_current = rejected.is_some_and(|t| t == state.session.access_token);
        if rejected_current && !state.session.access_rejected {
            state.session.access_rejected = true;
            state.dirty = true;
            self.save(slot, &state.session)?;
            state.dirty = false;
            self.decision(slot, state, Decision::UsageRejected, false);
        }
        if rejected.is_some() && !rejected_current { self.decision(slot, state, Decision::LateRejectionIgnored, !expired && !state.session.access_rejected); }
        // A renewal hint must not suppress still-usable access credentials during
        // token-endpoint backoff. Never reuse an expired or usage-rejected token.
        let usable_access =
            (!expired && !state.session.access_rejected && !state.session.access_token.is_empty())
                .then(|| {
                    (
                        state.session.access_token.clone(),
                        state.session.account_id.clone(),
                    )
                });
        if state.session.uncertain_attempts >= 2 {
            self.decision(slot, state, Decision::RetryExhausted, usable_access.is_some());
            return usable_access.ok_or_else(|| "monitor_uncertain".into());
        }
        if state.session.uncertain_retry_at.is_some_and(|t| t > now) {
            self.decision(slot, state, Decision::Backoff, usable_access.is_some());
            return usable_access.ok_or_else(|| "monitor_network".into());
        }
        if state.retry_at.is_some_and(|t| t > now) {
            self.decision(slot, state, Decision::Backoff, usable_access.is_some());
            return usable_access.ok_or_else(|| {
                state
                    .error
                    .clone()
                    .unwrap_or_else(|| "monitor_network".into())
            });
        }
        // A CLI can synthesize a refresh-token lifetime. Treat the saved deadline
        // as a renewal hint and let the provider decide whether the token is valid.
        let refresh_due = state.session.refresh_expires_at.is_some_and(|t| t <= now);
        if expired || state.session.access_rejected || refresh_due {
            let reason = if state.session.access_rejected { RefreshReason::UsageRejection }
                else if expired { RefreshReason::AccessExpiry } else { RefreshReason::ExpiryHint };
            self.record(slot, Event::Refresh { outcome: RefreshOutcome::Started, reason, elapsed_ms: 0 });
            let started = std::time::Instant::now();
            let result = transport.refresh(&state.session).await;
            let outcome = match &result { Ok(_) => RefreshOutcome::Success, Err(Failure::Revoked) => RefreshOutcome::Revoked,
                Err(Failure::RotatedUnreadable) => RefreshOutcome::RotatedUnreadable, Err(Failure::Transient) => RefreshOutcome::Transient,
                Err(Failure::RateLimited(_)) => RefreshOutcome::RateLimited, Err(Failure::Ambiguous) => RefreshOutcome::Ambiguous };
            self.record(slot, Event::Refresh { outcome, reason, elapsed_ms: diagnostics::millis(started) });
            match result {
                Ok(reply) => {
                    // Once the server may have rotated, preserve the returned refresh token
                    // even when the rest of the response is invalid. Never retry the old one.
                    if let Some(refresh) = reply.refresh_token.filter(|s| !s.is_empty()) {
                        state.session.refresh_token = refresh;
                    }
                    state.session.access_token = reply.access_token;
                    state.session.access_rejected = false;
                    if let Some(scope) = reply.scope {
                        state.session.scopes =
                            scope.split_whitespace().map(str::to_owned).collect();
                    }
                    state.session.refresh_expires_at = reply
                        .refresh_token_expires_in
                        .and_then(|seconds| after_seconds(now, seconds));
                    state.session.uncertain_attempts = 0;
                    state.session.uncertain_retry_at = None;
                    state.session.expires_at = reply
                        .expires_in
                        .and_then(|n| i64::try_from(n).ok())
                        .and_then(chrono::Duration::try_seconds)
                        .and_then(|d| now.checked_add_signed(d))
                        .or_else(|| jwt_expiry(&state.session.access_token));
                    state.session.reconnect = state.session.access_token.is_empty()
                        || reply
                            .token_type
                            .as_deref()
                            .is_some_and(|t| !t.eq_ignore_ascii_case("bearer"))
                        || state.session.expires_at.is_some_and(|t| t <= now);
                    state.dirty = true;
                    self.save(slot, &state.session)?;
                    state.dirty = false;
                    state.retry_at = None;
                    state.error = None;
                    if state.session.reconnect {
                        return Err("monitor_reconnect".into());
                    }
                }
                Err(Failure::Revoked | Failure::RotatedUnreadable) => {
                    state.session.reconnect = true;
                    state.dirty = true;
                    self.save(slot, &state.session)?;
                    state.dirty = false;
                    return Err("monitor_reconnect".into());
                }
                Err(failure) => {
                    let (code, delay) = match failure {
                        Failure::RateLimited(n) => ("rate_limited", n.clamp(300, 86400)),
                        // The monitor owns this chain. Allow one delayed replay after an
                        // uncertain send; an explicit rejection is terminal. A second
                        // uncertain result stops automatic replay and offers reconnect.
                        Failure::Ambiguous => {
                            state.session.uncertain_attempts += 1;
                            state.session.uncertain_retry_at =
                                Some(now + chrono::Duration::minutes(5));
                            state.dirty = true;
                            self.save(slot, &state.session)?;
                            state.dirty = false;
                            self.decision(slot, state, Decision::Grace, usable_access.is_some());
                            return usable_access.ok_or_else(|| {
                                if state.session.uncertain_attempts >= 2 {
                                    "monitor_uncertain"
                                } else {
                                    "monitor_network"
                                }
                                .into()
                            });
                        }
                        _ => ("monitor_network", 300),
                    };
                    state.retry_at = Some(now + chrono::Duration::seconds(delay));
                    state.error = Some(code.into());
                    self.decision(slot, state, Decision::Grace, usable_access.is_some());
                    return usable_access.ok_or_else(|| code.into());
                }
            }
        }
        self.decision(slot, state, Decision::Reuse, true);
        Ok((
            state.session.access_token.clone(),
            state.session.account_id.clone(),
        ))
    }
}

fn refresh_token(slot: &Slot) -> Option<&str> {
    match slot.provider {
        Provider::Claude => slot.data["credentials"]["claudeAiOauth"]["refreshToken"].as_str(),
        Provider::Codex => slot.data["auth"]["tokens"]["refresh_token"].as_str(),
    }
}

fn expiry(slot: &Slot) -> Option<DateTime<Utc>> {
    match slot.provider {
        Provider::Claude => slot.data["credentials"]["claudeAiOauth"]["expiresAt"]
            .as_i64()
            .and_then(DateTime::from_timestamp_millis),
        Provider::Codex => slot.access().and_then(|(token, _)| jwt_expiry(&token)),
    }
}

fn jwt_expiry(token: &str) -> Option<DateTime<Utc>> {
    let data = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.split('.').nth(1)?.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&data).ok()?;
    DateTime::from_timestamp(claims["exp"].as_i64()?, 0)
}

fn after_seconds(now: DateTime<Utc>, seconds: u64) -> Option<DateTime<Utc>> {
    i64::try_from(seconds)
        .ok()
        .and_then(chrono::Duration::try_seconds)
        .and_then(|d| now.checked_add_signed(d))
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    scope: Option<String>,
    refresh_token_expires_in: Option<u64>,
}

#[derive(Debug, PartialEq)]
enum Failure {
    Revoked,
    RateLimited(i64),
    Transient,
    Ambiguous,
    RotatedUnreadable,
}

trait Refresh {
    async fn refresh(&self, session: &Session) -> Result<Reply, Failure>;
}
trait Revoke {
    async fn revoke(&self, session: &Session) -> bool;
}
#[cfg(test)]
impl Revoke for () {
    async fn revoke(&self, _: &Session) -> bool {
        false
    }
}
struct Api<'a>(&'a reqwest::Client, Option<&'a diagnostics::Recorder>);

impl Refresh for Api<'_> {
    async fn refresh(&self, session: &Session) -> Result<Reply, Failure> {
        let (url, client_id) = match session.provider {
            Provider::Claude => (CLAUDE_TOKEN_URL, CLAUDE_CLIENT_ID),
            Provider::Codex => (CODEX_TOKEN_URL, CODEX_CLIENT_ID),
        };
        self.refresh_at(session, url, client_id).await
    }
}

impl Api<'_> {
    fn http_event(&self, session: &Session, endpoint: diagnostics::Endpoint, status: Option<u16>, network: Option<diagnostics::Network>, started: std::time::Instant) {
        if let Some(r) = self.1 { r.record(Some(session.provider), Some(&session.id), Event::Http { endpoint, status, network, elapsed_ms: diagnostics::millis(started) }); }
    }
    async fn refresh_at(
        &self,
        session: &Session,
        url: &str,
        client_id: &str,
    ) -> Result<Reply, Failure> {
        let mut fields = json!({ "grant_type": "refresh_token", "refresh_token": session.refresh_token, "client_id": client_id });
        if session.provider == Provider::Claude {
            // Compatibility for monitoring files from the first version of this patch.
            // These are a subset of the scopes granted by the official CLI login.
            fields["scope"] = json!(if session.scopes.is_empty() {
                "user:profile user:inference".into()
            } else {
                session.scopes.join(" ")
            });
        }
        let request = self
            .0
            .post(url)
            .header("Accept", "application/json")
            .json(&fields);
        let started = std::time::Instant::now();
        let response = request.send().await.map_err(|e| {
            self.http_event(session, diagnostics::Endpoint::Token, None, Some(network_kind(&e)), started);
            if e.is_connect() {
                Failure::Transient
            } else {
                Failure::Ambiguous
            }
        })?;
        let status = response.status().as_u16();
        self.http_event(session, diagnostics::Endpoint::Token, Some(status), None, started);
        if status == 429 {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(300);
            return Err(Failure::RateLimited(seconds));
        }
        if status >= 500 {
            return Err(Failure::Transient);
        }
        if !response.status().is_success() {
            // Only explicit OAuth rejection is terminal. Do not expose provider response bodies.
            let body: Value = response.json().await.unwrap_or(Value::Null);
            return Err(if revoked(status, &body) {
                Failure::Revoked
            } else {
                Failure::Transient
            });
        }
        // Success headers confirm that the refresh was accepted. If the body is
        // truncated, times out, or is invalid JSON, never replay the spent token.
        response
            .json()
            .await
            .map_err(|_| Failure::RotatedUnreadable)
    }
}

impl Revoke for Api<'_> {
    async fn revoke(&self, session: &Session) -> bool {
        let (url, client_id) = match session.provider {
            Provider::Claude => (
                "https://platform.claude.com/v1/oauth/token/revoke",
                CLAUDE_CLIENT_ID,
            ),
            Provider::Codex => ("https://auth.openai.com/oauth/revoke", CODEX_CLIENT_ID),
        };
        self.revoke_at(session, url, client_id).await
    }
}
impl Api<'_> {
    async fn revoke_at(&self, session: &Session, url: &str, client_id: &str) -> bool {
        let started = std::time::Instant::now();
        let result = self.0.post(url).timeout(std::time::Duration::from_secs(5))
            .json(&json!({"token": session.refresh_token, "token_type_hint": "refresh_token", "client_id": client_id}))
            .send().await;
        match result {
            Ok(r) => { self.http_event(session, diagnostics::Endpoint::Revoke, Some(r.status().as_u16()), None, started); r.status().is_success() },
            Err(e) => { self.http_event(session, diagnostics::Endpoint::Revoke, None, Some(network_kind(&e)), started); false },
        }
    }
}

fn network_kind(error: &reqwest::Error) -> diagnostics::Network {
    if error.is_timeout() { diagnostics::Network::Timeout } else if error.is_connect() { diagnostics::Network::Connect } else { diagnostics::Network::Transport }
}

fn revoked(status: u16, body: &Value) -> bool {
    if status == 401 {
        return true;
    }
    // Check all shapes rather than letting a descriptive string hide a nested code.
    [
        body["error"].as_str(),
        body["error"]["type"].as_str(),
        body["error"]["code"].as_str(),
        body["code"].as_str(),
    ]
    .into_iter()
    .flatten()
    .any(|code| {
        (status == 400 && code.eq_ignore_ascii_case("invalid_grant"))
            || [
                "refresh_token_expired",
                "refresh_token_reused",
                "refresh_token_invalidated",
            ]
            .iter()
            .any(|known| code.eq_ignore_ascii_case(known))
    })
}

#[cfg(test)]
mod tests;
