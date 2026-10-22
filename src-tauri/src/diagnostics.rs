//! Explicitly allowlisted diagnostics. Never serialize a Slot, Snapshot, error message,
//! request, environment, filesystem path, or provider response into an export.
use crate::{accounts::Provider, model::Snapshot, private_store, settings::Settings};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const MAX_EVENTS: usize = 10_000;
const MAX_ACCOUNTS: usize = 1_024;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const DAYS: i64 = 7;
const HEARTBEAT_SECONDS: i64 = 3_600;
const FLUSH_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Ok,
    Setup,
    Reconnect,
    Storage,
    Network,
    WrongAccount,
    SharedLogin,
    LoginFailed,
    Busy,
    Uncertain,
    RateLimited,
    TokenInvalid,
    TokenExpired,
    BadResponse,
    Http(u16),
    Other,
}
impl Code {
    pub fn from_error(error: Option<&str>) -> Self {
        match error {
            None => Self::Ok,
            Some("monitor_setup") => Self::Setup,
            Some("monitor_reconnect") => Self::Reconnect,
            Some("monitor_storage" | "cache_storage") => Self::Storage,
            Some("monitor_network") => Self::Network,
            Some("monitor_wrong_account") => Self::WrongAccount,
            Some("monitor_shared_login") => Self::SharedLogin,
            Some("monitor_login_failed") => Self::LoginFailed,
            Some("monitor_busy") => Self::Busy,
            Some("monitor_uncertain") => Self::Uncertain,
            Some("rate_limited") => Self::RateLimited,
            Some("token_invalid") => Self::TokenInvalid,
            Some("token_expired") => Self::TokenExpired,
            Some("bad_response") => Self::BadResponse,
            Some(e)
                if e.strip_prefix("rate_limited:")
                    .is_some_and(|n| n.parse::<i64>().is_ok()) =>
            {
                Self::RateLimited
            }
            Some(e) => e
                .strip_prefix("http_")
                .and_then(|n| n.parse::<u16>().ok())
                .filter(|n| (100..=599).contains(n))
                .map(Self::Http)
                .unwrap_or(Self::Other),
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ConnectStarted,
    ConnectFinished,
    Disconnect,
    Remove,
    SwitchManual,
    SwitchAuto,
    Cleanup,
    CacheSave,
    HistorySave,
    MonitorSave,
    TokenRead,
    Revoke,
    Export,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Settings,
    Panel,
    Dock,
    Switcher,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FrontendKind {
    Error,
    TypeError,
    ReferenceError,
    SyntaxError,
    RangeError,
    Rejection,
    Other,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Monitoring,
    Cli,
    ActiveCliFallback,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Reuse,
    Grace,
    Backoff,
    RetryExhausted,
    Reconnect,
    StorageFailure,
    UsageRejected,
    LateRejectionIgnored,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RefreshReason {
    AccessExpiry,
    UsageRejection,
    ExpiryHint,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RefreshOutcome {
    Started,
    Success,
    Revoked,
    RotatedUnreadable,
    Transient,
    RateLimited,
    Ambiguous,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LoginStage {
    CliDiscovery,
    Directory,
    Prepare,
    ProcessStart,
    ProcessExit,
    Timeout,
    Credentials,
    Profile,
    Handoff,
    FolderCleanup,
    Cancelled,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RevokeReason {
    Disconnect,
    Replacement,
    UninstalledLogin,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RevokeOutcome {
    Confirmed,
    Unconfirmed,
    Protected,
    UnknownOwnership,
    NoToken,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ProviderEvidence {
        origin: Origin,
        evidence: crate::providers::evidence::Evidence,
    },
    Http {
        endpoint: Endpoint,
        status: Option<u16>,
        network: Option<Network>,
        elapsed_ms: u64,
    },
    Startup {
        previous_clean_shutdown: Option<bool>,
        history_unreadable: bool,
    },
    Shutdown,
    PollStarted {
        gap_seconds: Option<i64>,
    },
    PollFinished {
        elapsed_ms: u64,
        accounts: usize,
        native_sources: usize,
        wsl_sources: usize,
    },
    Action {
        action: Action,
        code: Code,
        elapsed_ms: u64,
    },
    Refresh {
        outcome: RefreshOutcome,
        reason: RefreshReason,
        elapsed_ms: u64,
    },
    Token {
        decision: Decision,
        access_expires_at: Option<DateTime<Utc>>,
        retry_at: Option<DateTime<Utc>>,
        uncertain_attempts: u8,
        access_rejected: bool,
        usable: bool,
    },
    Usage {
        origin: Origin,
        code: Code,
        elapsed_ms: u64,
    },
    Frontend {
        surface: Surface,
        kind: FrontendKind,
        line: Option<u32>,
        column: Option<u32>,
    },
    Login {
        stage: LoginStage,
        ok: bool,
        wsl: bool,
        exit_code: Option<i32>,
        elapsed_ms: u64,
    },
    Revoke {
        reason: RevokeReason,
        outcome: RevokeOutcome,
        elapsed_ms: u64,
    },
    /// Encrypting the plaintext account vault of earlier releases (see `accounts::migrate_vault`):
    /// counts only. Recorded when anything was found.
    Vault {
        migrated: u32,
        failed: u32,
        unreadable: u32,
    },
    /// A panic, recorded only when the user turned crash records on (see `crash_event`): where in
    /// SlopBar's own source it happened and on which kind of thread, never its message.
    Crash {
        file: String,
        line: u32,
        thread: CrashThread,
    },
    Sample {
        active: bool,
        monitoring: bool,
        stale: bool,
        last_confirmed_at: Option<DateTime<Utc>>,
        code: Code,
        monitor_code: Code,
        windows: Vec<Window>,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Window {
    kind: WindowKind,
    used_percent: f64,
    resets_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    Token,
    Revoke,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Connect,
    Timeout,
    Transport,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum WindowKind {
    Session,
    Weekly,
    WeeklyOpus,
    WeeklySonnet,
    Window,
    Other,
}
fn sample(a: &crate::model::AccountUsage) -> Event {
    Event::Sample {
        active: a.active,
        monitoring: a.monitor_connected,
        stale: a.stale,
        last_confirmed_at: a.fetched_at,
        code: Code::from_error(a.error.as_deref()),
        monitor_code: Code::from_error(a.monitor_error.as_deref()),
        windows: a
            .windows
            .iter()
            .take(8)
            .filter(|w| w.used_percent.is_finite())
            .map(|w| Window {
                kind: match w.kind.as_str() {
                    "session" => WindowKind::Session,
                    "weekly" => WindowKind::Weekly,
                    "weekly_opus" => WindowKind::WeeklyOpus,
                    "weekly_sonnet" => WindowKind::WeeklySonnet,
                    "window" => WindowKind::Window,
                    _ => WindowKind::Other,
                },
                used_percent: w.used_percent.clamp(0.0, 100.0),
                resets_at: w.resets_at,
            })
            .collect(),
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Record {
    at: DateTime<Utc>,
    run: u64,
    provider: Option<Provider>,
    account: Option<u32>,
    event: Event,
}
#[derive(Default, Clone, Serialize, Deserialize)]
struct History {
    version: u8,
    run: u64,
    next_account: u32,
    // Only in the encrypted local journal. The report NEVER includes this mapping.
    accounts: HashMap<String, u32>,
    events: VecDeque<Record>,
    dropped: u64,
    #[serde(default)]
    suppressed: u64,
    clean_shutdown: bool,
}
type ObservationKey = (Option<Provider>, Option<u32>, Family);
type ObservationValue = (Event, DateTime<Utc>);
#[derive(Default)]
struct State {
    history: History,
    started: bool,
    history_unreadable: bool,
    storage_errors: u64,
    last_poll: Option<DateTime<Utc>>,
    observations: HashMap<ObservationKey, ObservationValue>,
    generation: u64,
    urgent_through: u64,
    completed: u64,
    write_attempts: u64,
    last_write_ok: Option<bool>,
    stop: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    ProviderEvidence(Origin, crate::providers::evidence::Endpoint),
    Token,
    Usage(Origin),
    Sample,
    PollStarted,
    PollFinished,
    Action(Action),
}
// Ignore measurement timestamps/timing jitter when deciding whether state changed.
// The actual record keeps those values; a fresh heartbeat is kept every hour.
fn observation(event: &Event) -> Option<(Family, Event)> {
    let mut signature = event.clone();
    let family = match &mut signature {
        Event::ProviderEvidence { origin, evidence } => Family::ProviderEvidence(*origin, evidence.endpoint),
        Event::Token { .. } => Family::Token,
        Event::Usage {
            origin, elapsed_ms, ..
        } => {
            *elapsed_ms = 0;
            // A failed monitor and a successful CLI fallback are simultaneous
            // streams, not alternating changes in one observation.
            Family::Usage(*origin)
        }
        Event::Sample {
            stale,
            last_confirmed_at,
            ..
        } => {
            if !*stale {
                *last_confirmed_at = None;
            }
            Family::Sample
        }
        Event::PollStarted { .. } => Family::PollStarted,
        Event::PollFinished { elapsed_ms, .. } => {
            *elapsed_ms = if *elapsed_ms >= 60_000 { 60_000 } else { 0 };
            Family::PollFinished
        }
        Event::Action {
            action, elapsed_ms, ..
        } if matches!(
            action,
            Action::TokenRead | Action::CacheSave | Action::HistorySave | Action::MonitorSave
        ) =>
        {
            *elapsed_ms = 0;
            Family::Action(*action)
        }
        _ => return None,
    };
    Some((family, signature))
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}
pub struct Recorder {
    path: PathBuf,
    shared: Arc<Shared>,
    worker: Option<std::thread::JoinHandle<()>>,
}
pub struct Context {
    pub recorder: std::sync::Arc<Recorder>,
    pub provider: Provider,
    pub id: String,
}
impl Context {
    pub fn login(
        &self,
        stage: LoginStage,
        ok: bool,
        wsl: bool,
        exit_code: Option<i32>,
        elapsed_ms: u64,
    ) {
        self.recorder.record(
            Some(self.provider),
            Some(&self.id),
            Event::Login {
                stage,
                ok,
                wsl,
                exit_code,
                elapsed_ms,
            },
        );
    }
}
impl Recorder {
    pub fn new(path: PathBuf) -> Self {
        Self::with_writer(path, |path, bytes| {
            private_store::save_secret(path, bytes).is_ok()
        })
    }
    fn with_writer(
        path: PathBuf,
        save: impl Fn(&std::path::Path, &[u8]) -> bool + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let writer = shared.clone();
        let writer_path = path.clone();
        let worker = std::thread::Builder::new()
            .name("diagnostic-writer".into())
            .spawn(move || Self::write_loop(writer_path, writer, save))
            .ok();
        if worker.is_none() {
            shared.state.lock().unwrap().storage_errors = 1;
        }
        Self {
            path,
            shared,
            worker,
        }
    }
    // Start only in Tauri setup, AFTER single-instance handling. A second launch
    // must not load/save a competing journal or falsely mark the app as stopped.
    pub fn start(&self) {
        let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.started {
            return;
        }
        if self.path.exists() {
            let loaded = std::fs::metadata(&self.path)
                .ok()
                .filter(|m| m.len() <= MAX_BYTES)
                .and_then(|_| private_store::load_secret(&self.path).ok())
                .and_then(|b| serde_json::from_slice::<History>(&b).ok())
                .filter(|h| {
                    h.version == 1
                        && h.accounts.len() <= MAX_ACCOUNTS
                        && h.events.len() <= MAX_EVENTS
                });
            if let Some(h) = loaded {
                s.history = h;
            } else {
                s.history_unreadable = true;
            }
        }
        let clean = (s.history.version == 1).then_some(s.history.clean_shutdown);
        s.history.version = 1;
        s.history.run = s.history.run.saturating_add(1);
        s.history.clean_shutdown = false;
        s.started = true;
        let unreadable = s.history_unreadable;
        Self::push(
            &mut s,
            None,
            None,
            Event::Startup {
                previous_clean_shutdown: clean,
                history_unreadable: unreadable,
            },
        );
        s.urgent_through = s.generation;
        self.shared.wake.notify_all();
    }
    fn account(s: &mut State, provider: Option<Provider>, id: Option<&str>) -> Option<u32> {
        let (provider, id) = (provider?, id?);
        if id.len() > 512 {
            return None;
        }
        let key = format!("{}:{id}", provider.name());
        if let Some(n) = s.history.accounts.get(&key) {
            return Some(*n);
        }
        if s.history.accounts.len() >= MAX_ACCOUNTS {
            return None;
        }
        s.history.next_account = s.history.next_account.saturating_add(1);
        let n = s.history.next_account;
        s.history.accounts.insert(key, n);
        Some(n)
    }
    fn push(s: &mut State, provider: Option<Provider>, id: Option<&str>, event: Event) {
        Self::push_at(s, provider, id, event, Utc::now());
    }
    fn push_at(
        s: &mut State,
        provider: Option<Provider>,
        id: Option<&str>,
        event: Event,
        now: DateTime<Utc>,
    ) {
        let account = Self::account(s, provider, id);
        if let Some((family, signature)) = observation(&event) {
            let key = (provider, account, family);
            if s.observations.get(&key).is_some_and(|(previous, at)| {
                *previous == signature
                    && now >= *at
                    && (now - *at).num_seconds() < HEARTBEAT_SECONDS
            }) {
                s.history.suppressed = s.history.suppressed.saturating_add(1);
                return;
            }
            s.observations.insert(key, (signature, now));
        }
        s.generation = s.generation.saturating_add(1);
        s.history.events.push_back(Record {
            at: now,
            run: s.history.run,
            provider,
            account,
            event,
        });
        while s.history.events.len() > MAX_EVENTS
            || s.history
                .events
                .front()
                .is_some_and(|r| r.at < now - chrono::Duration::days(DAYS))
        {
            s.history.events.pop_front();
            s.history.dropped = s.history.dropped.saturating_add(1);
        }
    }
    fn write_loop(
        path: PathBuf,
        shared: Arc<Shared>,
        save: impl Fn(&std::path::Path, &[u8]) -> bool,
    ) {
        let mut next_flush = Instant::now() + FLUSH_INTERVAL;
        loop {
            let (history, generation, stopping) = {
                let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if s.stop && (!s.started || s.generation == s.completed) {
                        return;
                    }
                    let dirty = s.started && s.generation > s.completed;
                    if dirty
                        && (s.stop
                            || s.urgent_through > s.completed
                            || Instant::now() >= next_flush)
                    {
                        break;
                    }
                    let wait = if dirty {
                        next_flush.saturating_duration_since(Instant::now())
                    } else {
                        FLUSH_INTERVAL
                    };
                    s = shared
                        .wake
                        .wait_timeout(s, wait)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                (s.history.clone(), s.generation, s.stop)
            };
            // Serialization, DPAPI and disk I/O never hold the recorder or monitor lock.
            let ok = serde_json::to_vec(&history)
                .ok()
                .filter(|b| b.len() as u64 <= MAX_BYTES)
                .is_some_and(|b| save(&path, &b));
            let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            s.write_attempts = s.write_attempts.saturating_add(1);
            s.last_write_ok = Some(ok);
            if !ok {
                s.storage_errors = s.storage_errors.saturating_add(1);
            }
            // Complete barriers even on disk failure. Diagnostics cannot gate authentication.
            s.completed = generation;
            next_flush = Instant::now() + FLUSH_INTERVAL;
            shared.wake.notify_all();
            if stopping && s.generation == s.completed {
                return;
            }
        }
    }
    // Only export/shutdown wait for the background writer. Auth callers merely notify it.
    pub(crate) fn flush(&self) {
        if self.worker.is_none() {
            return;
        }
        let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let target = s.generation;
        s.urgent_through = target;
        self.shared.wake.notify_all();
        while s.started && s.completed < target {
            s = self.shared.wake.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub fn record(&self, provider: Option<Provider>, id: Option<&str>, event: Event) {
        let shutdown = matches!(event, Event::Shutdown);
        self.record_at(provider, id, event, Utc::now());
        if shutdown {
            self.flush();
        }
    }
    fn record_at(
        &self,
        provider: Option<Provider>,
        id: Option<&str>,
        event: Event,
        now: DateTime<Utc>,
    ) {
        let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.started {
            return;
        }
        if matches!(event, Event::Shutdown) {
            s.history.clean_shutdown = true;
        }
        let urgent = match &event {
            Event::Token { decision, .. } => !matches!(decision, Decision::Reuse),
            Event::PollStarted { gap_seconds } => gap_seconds.is_some(),
            Event::Usage { .. }
            | Event::Sample { .. }
            | Event::Frontend { .. }
            | Event::Http { .. }
            | Event::PollFinished { .. } => false,
            _ => true,
        };
        let previous = s.generation;
        Self::push_at(&mut s, provider, id, event, now);
        if s.generation > previous && urgent {
            s.urgent_through = s.generation;
        }
        self.shared.wake.notify_all();
    }
    pub fn poll_started(&self, refresh_minutes: u32) {
        let now = Utc::now();
        let gap = {
            let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            s.last_poll.replace(now).map(|t| (now - t).num_seconds())
        };
        // Ordinary interval jitter is not a transition. Record unusual gaps exactly;
        // their cause (sleep, hang, slow requests) remains an inference.
        let abnormal = gap.filter(|g| *g < 0 || *g > i64::from(refresh_minutes) * 120 + 60);
        self.record(
            None,
            None,
            Event::PollStarted {
                gap_seconds: abnormal,
            },
        );
    }
    /// Every journaled reading of whether an account was the one signed in to its CLI: time,
    /// provider, slot id and active. Local only, to seed `active_log` with the past week.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    pub fn active_marks(&self) -> Vec<(DateTime<Utc>, Provider, String, bool)> {
        let s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let ids: HashMap<u32, &str> = s.history.accounts.iter()
            .filter_map(|(key, n)| key.split_once(':').map(|(_, id)| (*n, id))).collect();
        s.history.events.iter().filter_map(|r| match (&r.event, r.provider, r.account) {
            (Event::Sample { active, .. }, Some(p), Some(n)) => ids.get(&n).map(|id| (r.at, p, id.to_string(), *active)),
            _ => None,
        }).collect()
    }
    pub fn samples(&self, snapshot: &Snapshot) {
        for a in &snapshot.accounts {
            self.record(Some(a.provider), Some(&a.id), sample(a));
        }
    }
    pub fn report(
        &self,
        snapshot: &Snapshot,
        settings: &Settings,
    ) -> Result<Vec<u8>, &'static str> {
        let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.started {
            return Err("diagnostic_storage");
        }
        let current: Vec<Record> = snapshot
            .accounts
            .iter()
            .map(|a| Record {
                at: Utc::now(),
                run: s.history.run,
                provider: Some(a.provider),
                account: Self::account(&mut s, Some(a.provider), Some(&a.id)),
                event: sample(a),
            })
            .collect();
        #[derive(Serialize)]
        struct Report {
            schema_version: u8,
            exported_at: DateTime<Utc>,
            build: Build,
            retention_days: i64,
            event_limit: usize,
            dropped_events: u64,
            suppressed_observations: u64,
            heartbeat_seconds: i64,
            routine_flush_seconds: u64,
            journal_write_attempts: u64,
            history_from: Option<DateTime<Utc>>,
            history_unreadable: bool,
            storage_errors: u64,
            refresh_minutes: u32,
            claude_enabled: bool,
            codex_enabled: bool,
            crash_records: bool,
            current: Vec<Record>,
            events: VecDeque<Record>,
        }
        let report = Report {
            schema_version: 1,
            exported_at: Utc::now(),
            build: Build::current(),
            retention_days: DAYS,
            event_limit: MAX_EVENTS,
            dropped_events: s.history.dropped,
            suppressed_observations: s.history.suppressed,
            heartbeat_seconds: HEARTBEAT_SECONDS,
            routine_flush_seconds: FLUSH_INTERVAL.as_secs(),
            journal_write_attempts: s.write_attempts,
            history_from: s.history.events.front().map(|r| r.at),
            history_unreadable: s.history_unreadable,
            storage_errors: s.storage_errors,
            refresh_minutes: settings.refresh_minutes,
            claude_enabled: settings.providers.claude,
            codex_enabled: settings.providers.codex,
            crash_records: settings.crash_records,
            current,
            events: s.history.events.clone(),
        };
        drop(s);
        serde_json::to_vec_pretty(&report).map_err(|_| "diagnostic_storage")
    }
    pub fn export(
        &self,
        snapshot: &Snapshot,
        settings: &Settings,
    ) -> Result<PathBuf, &'static str> {
        self.record(
            None,
            None,
            Event::Action {
                action: Action::Export,
                code: Code::Ok,
                elapsed_ms: 0,
            },
        );
        self.flush();
        let bytes = self.report(snapshot, settings)?;
        let dir = self
            .path
            .parent()
            .ok_or("diagnostic_storage")?
            .join("exports");
        let path = dir.join(format!(
            "SlopBar-Diagnostics-{}.json",
            Utc::now().format("%Y%m%d-%H%M%S-%f")
        ));
        private_store::write(&path, &bytes).map_err(|_| "diagnostic_storage")?;
        Ok(path)
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        {
            let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            s.stop = true;
            self.shared.wake.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Serialize)]
struct Build {
    version: &'static str,
    commit: Option<&'static str>,
    source_tree: Option<&'static str>,
    dirty: bool,
    os: &'static str,
    arch: &'static str,
    upstream_updates: bool,
    tauri_dev: Option<bool>,
}
impl Build {
    fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            commit: option_env!("SLOPBAR_BUILD_COMMIT"),
            source_tree: option_env!("SLOPBAR_BUILD_TREE"),
            dirty: option_env!("SLOPBAR_BUILD_DIRTY") != Some("false"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            upstream_updates: cfg!(feature = "upstream-updates"),
            tauri_dev: option_env!("SLOPBAR_TAURI_DEV").and_then(|s| s.parse().ok()),
        }
    }
}
/// The executable's own build metadata (as `--build-info-to` writes it).
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub fn build_info() -> serde_json::Value {
    serde_json::to_value(Build::current()).unwrap_or_default()
}
pub fn write_build_info(path: &std::path::Path) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&Build::current()).map_err(std::io::Error::other)?;
    std::fs::write(path, bytes)
}
pub fn millis(start: std::time::Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}

// ---------------------------------------------------------------------------------------------
// Crash records: off unless the user turns them on (General > Diagnostics, `crash_records`).

/// The kind of thread a panic happened on; thread names are never recorded as such.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrashThread {
    Main,
    Runtime,
    Diagnostics,
    Other,
    Unnamed,
}

static CRASH_RECORDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Follows the setting; the hook stays installed and does nothing while it is off.
pub fn set_crash_records(on: bool) {
    CRASH_RECORDS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Where a panic happened, reduced to what cannot identify the user or carry data: a path inside
/// SlopBar's own sources ("src/app.rs"), else only "dependency", "std" or "other". Panic messages
/// can hold anything (tokens, emails, paths), so they are never passed in.
pub fn crash_event(location: Option<(&str, u32)>, thread: Option<&str>) -> Event {
    fn file(path: &str) -> String {
        let path = path.replace('\\', "/");
        let own = path.rsplit_once("/src-tauri/src/").map(|(_, rest)| rest).or_else(|| path.strip_prefix("src/"))
            .filter(|rest| rest.len() <= 64 && rest.ends_with(".rs") && !rest.contains("..")
                && rest.chars().all(|c| c.is_ascii_alphanumeric() || "_/.".contains(c)));
        match own {
            Some(rest) => format!("src/{rest}"),
            None if path.contains("/.cargo/") || path.contains("/registry/") => "dependency".into(),
            None if path.contains("/rustc/") || path.starts_with("library/") || path.contains("/rustlib/") => "std".into(),
            None => "other".into(),
        }
    }
    let thread = match thread {
        None => CrashThread::Unnamed,
        Some("main") => CrashThread::Main,
        Some("diagnostic-writer") => CrashThread::Diagnostics,
        Some(n) if n.starts_with("tokio-runtime-worker") || n.starts_with("tokio-") => CrashThread::Runtime,
        Some(_) => CrashThread::Other,
    };
    match location {
        Some((path, line)) => Event::Crash { file: file(path), line, thread },
        None => Event::Crash { file: "other".into(), line: 0, thread },
    }
}

/// Chains a panic hook that, while crash records are on, puts a `crash_event` into `recorder`'s
/// journal (and so into exports) before the default hook runs.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub fn install_crash_hook(recorder: Arc<Recorder>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if CRASH_RECORDS.load(std::sync::atomic::Ordering::Relaxed) {
            let current = std::thread::current();
            recorder.crash(crash_event(info.location().map(|l| (l.file(), l.line())), current.name()));
        }
        previous(info);
    }));
}

impl Recorder {
    /// Records a crash from inside a panic. The panic may have happened in the recorder itself, so
    /// this never waits for its lock, and waits for the writer only briefly and never from it.
    pub(crate) fn crash(&self, event: Event) {
        // Another thread (the writer) holds the lock only briefly; this thread holding it means
        // the panic came from inside the recorder, and then the record is given up.
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut s = loop {
            match self.shared.state.try_lock() {
                Ok(s) => break s,
                Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
                Err(std::sync::TryLockError::WouldBlock) => return,
            }
        };
        if !s.started {
            return;
        }
        Self::push(&mut s, None, None, event);
        let target = s.generation;
        s.urgent_through = target;
        self.shared.wake.notify_all();
        if self.worker.is_none() || std::thread::current().name() == Some("diagnostic-writer") {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.completed < target {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            s = self.shared.wake.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The journal's counters, for Settings > Health.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    pub fn health(&self) -> crate::health::Journal {
        let s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        crate::health::Journal { started: s.started, write_attempts: s.write_attempts, storage_errors: s.storage_errors,
            last_write_ok: s.last_write_ok.or((self.worker.is_none()).then_some(false)), history_unreadable: s.history_unreadable }
    }
}

#[cfg(test)]
mod crash_tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn crash_locations_keep_only_slopbars_own_source_files() {
        for (path, expected) in [
            ("src\\app.rs", "src/app.rs"),
            ("src/providers/claude.rs", "src/providers/claude.rs"),
            ("C:\\Users\\alice.example\\code\\slopbar\\src-tauri\\src\\door.rs", "src/door.rs"),
            ("C:\\Users\\alice.example\\.cargo\\registry\\src\\index.crates.io-6f17d22bba15001f\\tokio-1.40.0\\src\\runtime\\task.rs", "dependency"),
            ("/rustc/90b35a6239c3d8bdabc530a6a0816f7ff89a0aaf/library/core/src/option.rs", "std"),
            ("library/std/src/thread/mod.rs", "std"),
            ("D:\\secret-client-name\\build.rs", "other"),
            ("src/../../Users/alice/x.rs", "other"),
            ("src/token=abc123\nleak.rs", "other"),
        ] {
            let Event::Crash { file, .. } = crash_event(Some((path, 7)), None) else { panic!() };
            assert_eq!(file, expected, "{path}");
        }
        assert!(matches!(crash_event(None, Some("main")), Event::Crash { line: 0, thread: CrashThread::Main, .. }));
        for (name, kind) in [(Some("tokio-runtime-worker"), CrashThread::Runtime), (Some("alice@example.invalid"), CrashThread::Other), (None, CrashThread::Unnamed)] {
            let Event::Crash { thread, .. } = crash_event(Some(("src/app.rs", 1)), name) else { panic!() };
            assert_eq!(thread, kind);
        }
    }

    #[test]
    fn a_crash_record_reaches_the_export_without_its_message_or_paths() {
        let dir = TempDir::new();
        let r = Recorder::with_writer(dir.0.join("history"), |_, _| true);
        r.start();
        let canary = "PRIVATE-panic-message-token-sk-ant-oat01-alice@example.invalid";
        // What the hook does with a real panic: only the location and thread are passed on.
        let user_path = "C:\\Users\\alice.example\\.cargo\\registry\\src\\x\\serde-1.0.0\\src\\de.rs";
        r.crash(crash_event(Some(("src\\app.rs", 42)), Some("main")));
        r.crash(crash_event(Some((user_path, 9)), Some(canary)));
        let settings = Settings { crash_records: true, ..Settings::default() };
        let path = r.export(&Snapshot::default(), &settings).unwrap();
        let report = std::fs::read_to_string(path).unwrap();
        assert!(!report.contains(canary) && !report.contains("alice") && !report.contains("Users"), "{report}");
        let json: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(json["crash_records"], true);
        let crashes: Vec<_> = json["events"].as_array().unwrap().iter().filter(|e| e["event"]["type"] == "crash").map(|e| e["event"].clone()).collect();
        assert_eq!(crashes, [
            serde_json::json!({ "type": "crash", "file": "src/app.rs", "line": 42, "thread": "main" }),
            serde_json::json!({ "type": "crash", "file": "dependency", "line": 9, "thread": "other" }),
        ]);
    }

    #[test]
    fn crash_never_blocks_on_a_held_recorder_lock() {
        let dir = TempDir::new();
        let r = Recorder::with_writer(dir.0.join("history"), |_, _| true);
        r.start();
        let held = r.shared.state.lock().unwrap();
        r.crash(crash_event(Some(("src/app.rs", 1)), None));
        drop(held);
        assert!(r.health().started);
    }

    #[test]
    fn crash_records_are_off_by_default() {
        assert!(!Settings::default().crash_records);
        assert!(!CRASH_RECORDS.load(std::sync::atomic::Ordering::Relaxed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    #[test]
    fn export_allowlist_excludes_identities_credentials_paths_and_raw_errors() {
        let dir = TempDir::new();
        let r = Recorder::new(dir.0.join("history"));
        r.start();
        let canary = "PRIVATE-canary-access-refresh-email-org-path-url";
        let a = crate::model::AccountUsage {
            id: canary.into(),
            provider: Provider::Claude,
            email: Some(canary.into()),
            alias: Some(canary.into()),
            plan: Some(canary.into()),
            org: Some(canary.into()),
            active: false,
            sources: vec![canary.into()],
            windows: vec![crate::model::UsageWindow {
                kind: canary.into(),
                used_percent: 87.0,
                window_seconds: None,
                resets_at: None,
            }],
            credits: Some(canary.into()),
            resets: vec![],
            error: Some(canary.into()),
            monitor_error: Some(canary.into()),
            stale: true,
            fetched_at: None,
            monitor_connected: true,
            billing: crate::model::BillingHints { tier: Some(canary.into()), billing_type: Some(canary.into()), ..Default::default() },
            remote: vec![crate::model::RemoteSeat { machine: canary.into(), where_: vec![canary.into()], seen_at: chrono::Utc::now(), live: true }],
            used_outside: true,
            runs_out_at: None, unexpected: None, cli_rejected: false, extra_usage: None,
        };
        let snapshot = Snapshot {
            accounts: vec![a],
            updated_at: None,
        };
        let mut settings = Settings::default();
        settings.aliases.insert(canary.into(), canary.into());
        r.samples(&snapshot);
        let raw = serde_json::json!({"identity":canary,"authorization":canary,
            "credits":[{"title":canary,"reset_type":canary,"status":"available","expires_at":canary}]});
        r.record(Some(Provider::Claude), Some(canary), Event::ProviderEvidence {
            origin: Origin::Monitoring,
            evidence: crate::providers::evidence::capture(crate::providers::evidence::Endpoint::CodexResetDetails, 200, &raw),
        });
        r.flush();
        #[cfg(windows)]
        assert!(!std::fs::read(dir.0.join("history"))
            .unwrap()
            .windows(canary.len())
            .any(|b| b == canary.as_bytes()));
        let path = r.export(&snapshot, &settings).unwrap();
        let report = std::fs::read_to_string(path).unwrap();
        assert!(!report.contains(canary));
        assert!(!report.contains(&dir.0.to_string_lossy().to_string()));
        assert!(report.contains("87.0"));
        assert!(report.contains("\"account\": 1"));
        let json: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(json["current"][0]["event"]["code"], "other");
        assert_eq!(json["current"][0]["event"]["windows"][0]["kind"], "other");
        assert!(json["events"].as_array().unwrap().iter().any(|r| r["event"]["type"] == "provider_evidence"));
    }
    #[test]
    fn provider_evidence_deduplicates_each_endpoint_and_origin_separately() {
        use crate::providers::evidence::{capture, Endpoint};
        let mut s = State::default();
        let now = Utc::now();
        for _ in 0..100 {
            for origin in [Origin::Monitoring, Origin::ActiveCliFallback] {
                for endpoint in [Endpoint::CodexUsage, Endpoint::CodexResetDetails] {
                    Recorder::push_at(&mut s, Some(Provider::Codex), Some("synthetic"), Event::ProviderEvidence {
                        origin, evidence: capture(endpoint, 200, &serde_json::json!({})),
                    }, now);
                }
            }
        }
        assert_eq!(s.history.events.len(), 4);
    }
    #[test]
    fn pseudonyms_history_and_unclean_exit_survive_restart() {
        let dir = TempDir::new();
        let path = dir.0.join("history");
        let r = Recorder::new(path.clone());
        r.start();
        r.record(
            Some(Provider::Claude),
            Some("private-id"),
            Event::Action {
                action: Action::ConnectStarted,
                code: Code::Ok,
                elapsed_ms: 0,
            },
        );
        drop(r);
        let r = Recorder::new(path);
        r.start();
        r.record(
            Some(Provider::Claude),
            Some("private-id"),
            Event::Action {
                action: Action::ConnectFinished,
                code: Code::Ok,
                elapsed_ms: 1,
            },
        );
        let report: serde_json::Value = serde_json::from_slice(
            &r.report(&Snapshot::default(), &Settings::default())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(report["events"][1]["account"], 1);
        assert_eq!(report["events"][3]["account"], 1);
        assert_eq!(
            report["events"][2]["event"]["previous_clean_shutdown"],
            false
        );
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("private-id"));
    }
    #[test]
    fn rotation_corruption_and_disk_failure_are_visible_and_nonfatal() {
        let dir = TempDir::new();
        let path = dir.0.join("history");
        std::fs::write(&path, b"broken").unwrap();
        let r = Recorder::new(path.clone());
        r.start();
        {
            let mut s = r.shared.state.lock().unwrap();
            let old = Record {
                at: Utc::now() - chrono::Duration::days(8),
                run: 1,
                provider: None,
                account: None,
                event: Event::Shutdown,
            };
            s.history.events = VecDeque::from(vec![old; MAX_EVENTS + 1]);
        }
        r.record(None, None, Event::Shutdown);
        assert_eq!(r.shared.state.lock().unwrap().history.events.len(), 1);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        r.record(
            None,
            None,
            Event::PollStarted {
                gap_seconds: Some(900),
            },
        );
        r.flush();
        let report: serde_json::Value = serde_json::from_slice(
            &r.report(&Snapshot::default(), &Settings::default())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(report["history_unreadable"], true);
        assert!(report["storage_errors"].as_u64().unwrap() > 0);
        assert!(report["dropped_events"].as_u64().unwrap() > MAX_EVENTS as u64);
    }
    fn stable_sample(at: DateTime<Utc>) -> Event {
        Event::Sample {
            active: false,
            monitoring: true,
            stale: false,
            last_confirmed_at: Some(at),
            code: Code::Ok,
            monitor_code: Code::Ok,
            windows: vec![Window {
                kind: WindowKind::Session,
                used_percent: 24.0,
                resets_at: None,
            }],
        }
    }
    #[test]
    fn eleven_idle_accounts_retain_a_week_even_with_one_minute_polls() {
        for interval in [1, 5, 15] {
            let mut s = State::default();
            s.history.run = 1;
            let begin = Utc::now();
            for minute in (0..=7 * 24 * 60).step_by(interval) {
                let now = begin + chrono::Duration::minutes(minute as i64);
                idle_poll(&mut s, now, minute);
            }
            assert!(
                s.history.events.len() < 6_000,
                "interval {interval}: {}",
                s.history.events.len()
            );
            assert_eq!(s.history.dropped, 0);
            assert!(
                (s.history.events.back().unwrap().at - s.history.events.front().unwrap().at)
                    .num_days()
                    >= 7
            );
            assert!(s.history.suppressed > 10_000);
        }
    }
    #[test]
    fn eleven_accounts_retain_a_week_with_repeated_active_cli_fallback() {
        for interval in [1, 5, 15] {
            let mut s = State::default();
            let begin = Utc::now();
            for minute in (0..=7 * 24 * 60).step_by(interval) {
                let now = begin + chrono::Duration::minutes(minute as i64);
                // Ten independent monitors remain healthy; the active account's
                // monitor usage fails while its captured CLI access token works.
                Recorder::push_at(
                    &mut s,
                    None,
                    None,
                    Event::PollStarted { gap_seconds: None },
                    now,
                );
                for account in 0..11 {
                    let id = format!("synthetic-{account}");
                    let active = account == 0;
                    Recorder::push_at(
                        &mut s,
                        Some(Provider::Claude),
                        Some(&id),
                        Event::Token {
                            decision: Decision::Reuse,
                            access_expires_at: None,
                            retry_at: None,
                            uncertain_attempts: 0,
                            access_rejected: false,
                            usable: true,
                        },
                        now,
                    );
                    Recorder::push_at(
                        &mut s,
                        Some(Provider::Claude),
                        Some(&id),
                        Event::Usage {
                            origin: Origin::Monitoring,
                            code: if active { Code::BadResponse } else { Code::Ok },
                            elapsed_ms: minute as u64,
                        },
                        now,
                    );
                    if active {
                        Recorder::push_at(
                            &mut s,
                            Some(Provider::Claude),
                            Some(&id),
                            Event::Usage {
                                origin: Origin::ActiveCliFallback,
                                code: Code::Ok,
                                elapsed_ms: minute as u64 + 1,
                            },
                            now,
                        );
                    }
                    let mut reading = stable_sample(now);
                    if let Event::Sample {
                        active: is_active,
                        monitor_code,
                        ..
                    } = &mut reading
                    {
                        *is_active = active;
                        *monitor_code = if active { Code::BadResponse } else { Code::Ok };
                    }
                    Recorder::push_at(&mut s, Some(Provider::Claude), Some(&id), reading, now);
                }
                Recorder::push_at(
                    &mut s,
                    None,
                    None,
                    Event::PollFinished {
                        elapsed_ms: (minute % 1000) as u64,
                        accounts: 11,
                        native_sources: 1,
                        wsl_sources: 0,
                    },
                    now,
                );
            }
            assert!(
                s.history.events.len() < 6_500,
                "interval {interval}: {} events",
                s.history.events.len()
            );
            assert_eq!(s.history.dropped, 0, "interval {interval}");
            assert_eq!(
                (s.history.events.back().unwrap().at - s.history.events.front().unwrap().at)
                    .num_days(),
                7
            );
            for origin in [Origin::Monitoring, Origin::ActiveCliFallback] {
                let usage = s
                    .history
                    .events
                    .iter()
                    .filter(|r| {
                        r.account == Some(1)
                            && matches!(r.event, Event::Usage { origin: o, .. } if o == origin)
                    })
                    .count();
                assert_eq!(usage, 169, "each stream keeps its own hourly heartbeat");
            }
        }
    }
    fn idle_poll(s: &mut State, now: DateTime<Utc>, minute: usize) {
        Recorder::push_at(s, None, None, Event::PollStarted { gap_seconds: None }, now);
        for account in 0..11 {
            let id = format!("synthetic-{account}");
            for event in [
                Event::Token {
                    decision: Decision::Reuse,
                    access_expires_at: None,
                    retry_at: None,
                    uncertain_attempts: 0,
                    access_rejected: false,
                    usable: true,
                },
                Event::Usage {
                    origin: Origin::Monitoring,
                    code: Code::Ok,
                    elapsed_ms: minute as u64,
                },
                stable_sample(now),
            ] {
                Recorder::push_at(s, Some(Provider::Claude), Some(&id), event, now);
            }
        }
        Recorder::push_at(
            s,
            None,
            None,
            Event::PollFinished {
                elapsed_ms: (minute % 1000) as u64,
                accounts: 11,
                native_sources: 1,
                wsl_sources: 0,
            },
            now,
        );
    }
    #[test]
    fn transitions_and_hourly_heartbeat_keep_real_readings() {
        let mut s = State::default();
        let now = Utc::now();
        Recorder::push_at(
            &mut s,
            Some(Provider::Claude),
            Some("account"),
            stable_sample(now),
            now,
        );
        let later = now + chrono::Duration::minutes(5);
        Recorder::push_at(
            &mut s,
            Some(Provider::Claude),
            Some("account"),
            stable_sample(later),
            later,
        );
        assert_eq!(s.history.events.len(), 1);
        let mut failed = stable_sample(later);
        if let Event::Sample { stale, code, .. } = &mut failed {
            *stale = true;
            *code = Code::Network;
        }
        Recorder::push_at(
            &mut s,
            Some(Provider::Claude),
            Some("account"),
            failed,
            later,
        );
        let recovered = later + chrono::Duration::minutes(5);
        Recorder::push_at(
            &mut s,
            Some(Provider::Claude),
            Some("account"),
            stable_sample(recovered),
            recovered,
        );
        let heartbeat = recovered + chrono::Duration::hours(1);
        Recorder::push_at(
            &mut s,
            Some(Provider::Claude),
            Some("account"),
            stable_sample(heartbeat),
            heartbeat,
        );
        assert_eq!(s.history.events.len(), 4);
        assert!(matches!(s.history.events.back().unwrap().event,
            Event::Sample { last_confirmed_at: Some(at), .. } if at == heartbeat));
    }
    #[test]
    fn ordinary_polls_do_not_rewrite_the_journal_and_export_drains_the_writer() {
        let dir = TempDir::new();
        let path = dir.0.join("history");
        let r = Recorder::new(path.clone());
        r.start();
        r.flush();
        let writes = r.shared.state.lock().unwrap().write_attempts;
        let on_disk = std::fs::read(&path).unwrap();
        for _ in 0..100 {
            r.poll_started(5);
            r.record(
                Some(Provider::Claude),
                Some("account"),
                stable_sample(Utc::now()),
            );
            r.record(
                None,
                None,
                Event::PollFinished {
                    elapsed_ms: 2,
                    accounts: 1,
                    native_sources: 1,
                    wsl_sources: 0,
                },
            );
        }
        assert_eq!(r.shared.state.lock().unwrap().write_attempts, writes);
        assert_eq!(std::fs::read(&path).unwrap(), on_disk);
        let export = r
            .export(&Snapshot::default(), &Settings::default())
            .unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
        assert_eq!(report["journal_write_attempts"], writes + 1);
        assert!(report["suppressed_observations"].as_u64().unwrap() > 200);
        let history: History =
            serde_json::from_slice(&private_store::load_secret(&path).unwrap()).unwrap();
        assert!(history
            .events
            .iter()
            .any(|r| matches!(r.event, Event::Sample { .. })));
    }
    #[test]
    fn auth_records_return_while_the_background_disk_writer_is_stalled() {
        let dir = TempDir::new();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = std::cell::Cell::new(true);
        let r = Arc::new(Recorder::with_writer(
            dir.0.join("history"),
            move |path, bytes| {
                if first.replace(false) {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                private_store::save_secret(path, bytes).is_ok()
            },
        ));
        r.start();
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let observer = r.clone();
        let auth = std::thread::spawn(move || {
            observer.record(
                Some(Provider::Claude),
                Some("account"),
                Event::Refresh {
                    outcome: RefreshOutcome::Started,
                    reason: RefreshReason::AccessExpiry,
                    elapsed_ms: 0,
                },
            );
            done_tx.send(()).unwrap();
        });
        let result = done_rx.recv_timeout(Duration::from_secs(3));
        release_tx.send(()).unwrap();
        auth.join().unwrap();
        assert!(
            result.is_ok(),
            "authentication waited for diagnostic disk I/O"
        );
        r.flush();
        let s = r.shared.state.lock().unwrap();
        assert_eq!(s.completed, s.generation);
        assert!(s
            .history
            .events
            .iter()
            .any(|r| matches!(r.event, Event::Refresh { .. })));
    }
    #[test]
    fn error_codes_never_passthrough_messages() {
        assert_eq!(
            Code::from_error(Some("rate_limited:900")),
            Code::RateLimited
        );
        assert_eq!(Code::from_error(Some("http_503")), Code::Http(503));
        for e in [
            "http_503?token=secret",
            "http_999",
            "monitor_network:secret",
            "https://private/path",
            "rate_limited:secret",
        ] {
            assert_eq!(Code::from_error(Some(e)), Code::Other);
        }
    }
}
