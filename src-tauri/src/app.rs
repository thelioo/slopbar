use crate::{accounts, balancer, codex_local, diagnostics, dock, door, headroom, inspect, migrate, model, monitoring, notify, panel, plan_alerts, presence, providers, settings, sources, subscriptions, updates};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;

use accounts::Provider;
use model::Snapshot;
use settings::Settings;

pub(crate) const SETTINGS_LABEL: &str = "settings";
const MENU_IDS: [&str; 3] = ["refresh", "settings", "quit"];

pub struct AppState {
    monitor: monitoring::Monitor,
    diagnostics: std::sync::Arc<diagnostics::Recorder>,
    refresh_lock: tokio::sync::Mutex<()>,
    /// Held from reading settings to storing them, so the window and agents never lose a change.
    settings_lock: tokio::sync::Mutex<()>,
    account_operations: monitoring::Operations,
    pub(crate) snapshot: Mutex<Snapshot>,
    pub(crate) settings: Mutex<Settings>,
    /// The widget's right-click menu and its items (relabeled when the language changes).
    pub(crate) menu: Mutex<Option<Menu<Wry>>>,
    pub(crate) menu_items: Mutex<Vec<MenuItem<Wry>>>,
    /// Physical center of the widget when it was last clicked; the flyout opens above it.
    pub(crate) panel_anchor: Mutex<Option<(f64, f64)>>,
    pub(crate) panel_height: Mutex<f64>,
    pub(crate) panel_hidden_at: Mutex<Option<Instant>>,
    /// Content width of the taskbar widget, in CSS px.
    pub(crate) dock_width: Mutex<f64>,
    /// Last known usage per account, for accounts whose saved token can't be used right now.
    pub(crate) usage_cache: Mutex<providers::Cache>,
    /// Every account's completed weekly windows, for the Subscriptions page.
    pub(crate) usage_history: Mutex<subscriptions::History>,
    /// The Codex CLI's own history on this PC, read in the background after each refresh.
    pub(crate) codex_local: Mutex<Option<std::sync::Arc<codex_local::Local>>>,
    /// When each provider was last switched automatically (the balancer's cooldown).
    pub(crate) last_switch: Mutex<HashMap<Provider, Instant>>,
    /// The past few hours of readings, for the agents' pace forecasts.
    pub(crate) recent: Mutex<headroom::Recent>,
    /// Sources found by the last refresh, where switches are written.
    pub(crate) sources: Mutex<Vec<sources::Source>>,
    /// Recent readings of unclaimed accounts, to notice use outside this PC.
    presence_tracks: Mutex<presence::Tracks>,
    /// Plan notifications already shown (renewals, running out), so each is shown once.
    plan_alerts: Mutex<plan_alerts::Sent>,
}

#[tauri::command]
fn get_usage(state: tauri::State<AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
}

fn subscription_report(state: &AppState) -> subscriptions::Report {
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    let local = state.codex_local.lock().unwrap().clone();
    subscriptions::report(&snapshot, &state.usage_history.lock().unwrap(), &settings, local.as_deref(), chrono::Utc::now())
}

#[tauri::command]
fn get_subscriptions(state: tauri::State<AppState>) -> subscriptions::Report {
    subscription_report(&state)
}

/// The plan notifications due now (see `plan_alerts`), each returned once. The taskbar widget
/// asks after every refresh and words them in the user's language.
#[tauri::command]
fn take_plan_alerts(state: tauri::State<AppState>) -> Vec<plan_alerts::Alert> {
    let snapshot = state.snapshot.lock().unwrap().clone();
    if snapshot.updated_at.is_none() { return vec![]; }
    let report = subscription_report(&state);
    let prefs = state.settings.lock().unwrap().notifications.clone();
    let mut sent = state.plan_alerts.lock().unwrap();
    let due = plan_alerts::due(&report, &snapshot, &prefs, &mut sent, chrono::Utc::now());
    // Unsaved, they are still remembered until the app restarts.
    if !due.is_empty() { let _ = plan_alerts::save(&sent); }
    due
}

/// Opens the provider's billing page in the browser. The address is fixed per provider, so the
/// window can only choose which of the two opens.
#[tauri::command]
async fn open_billing_page(provider: Provider) -> Result<(), String> {
    let url = subscriptions::billing_page(provider);
    tauri::async_runtime::spawn_blocking(move || std::process::Command::new("explorer.exe").arg(url).spawn().map(|_| ()))
        .await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Writes a diagnostics export; shared by Settings and the agents' door.
async fn write_export(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let state = app.state::<AppState>();
    let recorder = state.diagnostics.clone();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || recorder.export(&snapshot, &settings))
        .await.map_err(|_| "diagnostic_storage")?.map_err(String::from)
}

#[tauri::command]
async fn export_diagnostics(app: AppHandle) -> Result<String, String> {
    let path = write_export(&app).await?;
    #[cfg(windows)]
    if let Some(parent) = path.parent() {
        let folder = parent.to_owned();
        // No shell or user-supplied command. Open the dedicated export folder.
        let _ = tauri::async_runtime::spawn_blocking(move || std::process::Command::new("explorer.exe").arg(folder).spawn()).await;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// An agent's switch: the same as Use in Settings, then the balancer's cooldown so the next poll
/// does not switch straight back. Returns the new snapshot and whether the account held.
async fn agent_switch(app: &AppHandle, provider: Provider, id: String) -> Result<serde_json::Value, String> {
    switch_to(app, provider, id.clone(), false).await?;
    app.state::<AppState>().last_switch.lock().unwrap().insert(provider, Instant::now());
    let snapshot = do_refresh(app).await;
    let active = snapshot.accounts.iter().find(|a| a.provider == provider && a.active).map(|a| a.id.clone());
    let mut result = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    result["switched_to_requested"] = (active.as_deref() == Some(id.as_str())).into();
    Ok(result)
}

/// One operation for an agent, through the door (see `door` and `mcp`). Each runs the same code as
/// the matching Settings action, so the windows and journal see it the same way.
async fn door_call(app: AppHandle, request: door::Request) -> Result<serde_json::Value, String> {
    use door::Request as R;
    fn value(x: impl Serialize) -> Result<serde_json::Value, String> {
        serde_json::to_value(x).map_err(|e| e.to_string())
    }
    match request.clone() {
        R::ExportDiagnostics => Ok(serde_json::json!({ "path": write_export(&app).await? })),
        R::GetUsage => value(app.state::<AppState>().snapshot.lock().unwrap().clone()),
        R::Refresh => value(do_refresh(&app).await),
        R::GetSettings => value(app.state::<AppState>().settings.lock().unwrap().clone()),
        R::GetSubscriptions => value(subscription_report(&app.state::<AppState>())),
        R::UpdateSettings { patch } => {
            let state = app.state::<AppState>();
            let _held = state.settings_lock.lock().await;
            let current = app.state::<AppState>().settings.lock().unwrap().clone();
            let next = settings::patched(&current, &patch, updates_enabled())?;
            let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
            for key in next.aliases.keys().filter(|k| !current.aliases.contains_key(*k)) {
                if !snapshot.accounts.iter().any(|a| format!("{}:{}", a.provider.name(), a.id) == *key) {
                    return Err(format!("no account {key:?}; alias keys are \"provider:id\" from get_usage"));
                }
            }
            store_settings(&app, next.clone()).await?;
            value(next)
        }
        R::SwitchAccount { provider, id } => agent_switch(&app, provider, id).await,
        R::GetHeadroom { provider, id } => {
            let state = app.state::<AppState>();
            let snapshot = state.snapshot.lock().unwrap().clone();
            let settings = state.settings.lock().unwrap().clone();
            let now = chrono::Utc::now();
            let picked: Vec<_> = snapshot.accounts.iter().filter(|a| match (&provider, &id) {
                (_, Some(id)) => a.id == *id && provider.map_or(true, |p| a.provider == p),
                (Some(p), None) => a.provider == *p && a.active,
                (None, None) => a.active,
            }).collect();
            if picked.is_empty() {
                return Err(if id.is_some() { "monitor_wrong_account".into() } else { "no_active_account".into() });
            }
            let accounts: Vec<_> = {
                let recent = state.recent.lock().unwrap();
                picked.into_iter().map(|a| headroom::headroom(a, &settings, &recent, now)).collect()
            };
            Ok(serde_json::json!({ "accounts": value(accounts)?, "updated_at": snapshot.updated_at }))
        }
        R::RecommendAccount { provider, expected_percent, hours, switch } => {
            if expected_percent.is_some_and(|p| !(0.0..=100.0).contains(&p)) || hours.is_some_and(|h| !(0.0..=168.0).contains(&h)) {
                return Err("expected_percent must be 0-100 and hours 0-168".into());
            }
            let rec = {
                let state = app.state::<AppState>();
                let snapshot = state.snapshot.lock().unwrap().clone();
                let settings = state.settings.lock().unwrap().clone();
                headroom::recommend(&snapshot.accounts, &settings, provider, expected_percent, hours, chrono::Utc::now())
            };
            let mut result = value(&rec)?;
            result["switched"] = false.into();
            if let (true, headroom::Action::Switch, Some(id)) = (switch, rec.action, rec.recommended.clone()) {
                let done = agent_switch(&app, provider, id).await?;
                result["switched"] = true.into();
                result["switched_to_requested"] = done["switched_to_requested"].clone();
            }
            Ok(result)
        }
        R::AddAccount { provider } => {
            // One pending sign-in per provider, so a retrying agent cannot flood the desktop.
            let pending = app.state::<AppState>().account_operations.begin(provider, "add-account")?;
            start_add_account(app.clone(), provider, Some(pending)).await?;
            Ok(serde_json::json!({ "started": true, "next": "The user completes the sign-in in the terminal that opened; the account then appears in get_usage." }))
        }
        R::ConnectMonitoring { provider, id } => {
            connect_monitoring(app.clone(), provider, id).await?;
            Ok(serde_json::json!({ "started": true, "next": "The user completes the monitoring sign-in that opened; get_usage then shows monitor_connected." }))
        }
        R::DisconnectMonitoring { provider, id } => value(disconnect_monitoring(app.clone(), provider, id).await?),
        R::RemoveAccount { provider, id } => value(remove_account(app.clone(), provider, id).await?),
        R::CheckForUpdates => value(check_for_updates(app.clone()).await?),
        R::InstallUpdate => {
            let latest = check_for_updates(app.clone()).await?.latest;
            if latest.is_none() {
                return Ok(serde_json::json!({ "installing": false, "reason": "SlopBar is up to date" }));
            }
            // Installing restarts SlopBar: reply first, then install.
            let h = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = updates::install(&h).await;
            });
            Ok(serde_json::json!({ "installing": true, "version": latest, "next": "SlopBar restarts into the new version shortly." }))
        }
        R::GetAppInfo => Ok(serde_json::json!({
            "version": app_version(app.clone()),
            "updates_enabled": updates_enabled(),
            "build": diagnostics::build_info(),
        })),
        R::InspectWindow { window, page, screenshot } => {
            let target = match window { door::InspectTarget::Settings => inspect::Target::Settings, door::InspectTarget::Flyout => inspect::Target::Flyout };
            value(inspect::inspect(&app, target, page, screenshot).await?)
        }
        R::ListSharingLocations => {
            let (folder, remote) = {
                let s = app.state::<AppState>().settings.lock().unwrap().clone();
                (s.presence_folder, s.presence_remote)
            };
            Ok(serde_json::json!({
                "folders": sync_folders().await,
                "rclone": rclone_remotes().await,
                "current": { "presence_folder": folder, "presence_remote": remote },
                "next": "Set presence_remote (an rclone remote name, no colon) or presence_folder (a folder path) with update_settings.",
            }))
        }
        R::OpenBillingPage { provider } => {
            open_billing_page(provider).await?;
            Ok(serde_json::json!({ "opened": subscriptions::billing_page(provider) }))
        }
        R::OpenSettings => {
            show_settings(&app);
            Ok(serde_json::json!({ "opened": true }))
        }
        R::QuitApp | R::RestartApp => {
            let restart = matches!(request, R::RestartApp);
            if restart {
                // The installed executable, even if this one was renamed aside for an update.
                let exe = std::env::current_exe().map_err(|e| e.to_string())?
                    .with_file_name(if cfg!(windows) { "slopbar.exe" } else { "slopbar" });
                std::process::Command::new(&exe).arg("--after").arg(std::process::id().to_string())
                    .spawn().map_err(|e| format!("restart failed: {e}"))?;
            }
            // Reply first, then exit exactly as the tray's Quit does (journal flushed on Exit).
            let h = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                h.exit(0);
            });
            Ok(serde_json::json!({ "quitting": true, "restarting": restart }))
        }
    }
}

#[tauri::command]
fn record_frontend_error(state: tauri::State<AppState>, surface: diagnostics::Surface, kind: diagnostics::FrontendKind, line: Option<u32>, column: Option<u32>) {
    state.diagnostics.record(None, None, diagnostics::Event::Frontend { surface, kind, line, column });
}

#[tauri::command]
async fn refresh(app: AppHandle) -> Snapshot {
    do_refresh(&app).await
}

#[derive(Clone, Serialize)]
struct Switched {
    provider: Provider,
    from: Option<String>,
    to: Option<String>,
    auto: bool,
}

/// Display name of an account: its alias, else its email.
fn account_label(app: &AppHandle, provider: Provider, id: &str) -> Option<String> {
    let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
    snapshot
        .accounts
        .iter()
        .find(|a| a.provider == provider && a.id == id)
        .and_then(|a| a.alias.clone().or(a.email.clone()))
}

async fn switch_to(app: &AppHandle, provider: Provider, id: String, auto: bool) -> Result<(), String> {
    let found = app.state::<AppState>().sources.lock().unwrap().clone();
    let from = {
        let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
        snapshot.accounts.iter().find(|a| a.provider == provider && a.active).map(|a| a.id.clone())
    };
    let target = id.clone();
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(move || accounts::switch(&found, provider, &target))
        .await
        .map_err(|e| e.to_string()).and_then(|r| r);
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action {
        action: if auto { diagnostics::Action::SwitchAuto } else { diagnostics::Action::SwitchManual },
        code: diagnostics::Code::from_error(result.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
    result?;
    let event = Switched {
        provider,
        from: from.and_then(|f| account_label(app, provider, &f)),
        to: account_label(app, provider, &id),
        auto,
    };
    let _ = app.emit("account-switched", event);
    Ok(())
}

#[tauri::command]
async fn switch_account(app: AppHandle, provider: Provider, id: String) -> Result<Snapshot, String> {
    switch_to(&app, provider, id, false).await?;
    Ok(do_refresh(&app).await)
}

/// Opens a terminal running the CLI's own sign-in; the account appears once it finishes.
#[tauri::command]
async fn add_account(app: AppHandle, provider: Provider) -> Result<(), String> {
    start_add_account(app, provider, None).await
}

/// Starts the CLI sign-in; `pending` (held until it finishes) keeps a second one from starting.
async fn start_add_account(app: AppHandle, provider: Provider, pending: Option<monitoring::Operation>) -> Result<(), String> {
    let found = {
        let known = app.state::<AppState>().sources.lock().unwrap().clone();
        if known.is_empty() {
            tauri::async_runtime::spawn_blocking(sources::discover).await.unwrap_or_default()
        } else {
            known
        }
    };
    // Probing for the CLI runs wsl.exe / where.exe; keep it off the async runtime.
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login(provider, &found))
        .await
        .map_err(|e| e.to_string())??;
    tauri::async_runtime::spawn(async move {
        let _pending = pending;
        let added = job.wait(&http_client()).await;
        if let Some(slot) = added {
            let _ = app.emit("account-added", (slot.provider, slot.email));
            do_refresh(&app).await;
        }
    });
    Ok(())
}

/// A one-time login owns its own refresh-token chain; it never changes the active CLI.
#[tauri::command]
async fn connect_monitoring(app: AppHandle, provider: Provider, id: String) -> Result<(), String> {
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let expected = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id)
        .ok_or("monitor_wrong_account")?;
    let key = monitoring::key(&expected);
    let started = Instant::now();
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectStarted, code: diagnostics::Code::Ok, elapsed_ms: 0 });
    let found = app.state::<AppState>().sources.lock().unwrap().clone();
    let context = diagnostics::Context { recorder: app.state::<AppState>().diagnostics.clone(), provider, id: id.clone() };
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login_observed(provider, &found, Some(context))).await;
    let job = match job {
        Ok(Ok(job)) => job,
        _ => {
            app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectFinished, code: diagnostics::Code::LoginFailed, elapsed_ms: diagnostics::millis(started) });
            return Err("monitor_login_failed".into());
        }
    };
    tauri::async_runtime::spawn(async move {
        let operation = operation;
        let outcome = match job.wait_isolated(&http_client()).await {
            Some(login) => {
                let slots = accounts::load_slots();
                if slots.iter().any(|s| s.provider == provider && s.id == id) {
                    app.state::<AppState>().monitor.connect_replacing(&expected, &login, &slots, &http_client()).await
                } else {
                    let mut known = slots;
                    known.push(expected.clone());
                    app.state::<AppState>().monitor.discard_unstored_login(&login, &known, &http_client()).await;
                    Err("monitor_wrong_account".into())
                }
            }
            None => Err("monitor_login_failed".into()),
        };
        drop(operation);
        app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectFinished,
            code: diagnostics::Code::from_error(outcome.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
        let _ = app.emit("monitoring-login", serde_json::json!({ "provider": provider, "id": id, "error": outcome.err() }));
        // Preserve last-known data, but allow the new login to bypass an earlier error's backoff.
        {
            let state = app.state::<AppState>();
            let _refresh = state.refresh_lock.lock().await;
            providers::retry_now(&mut state.usage_cache.lock().unwrap(), &key);
        }
        do_refresh(&app).await;
    });
    Ok(())
}

#[tauri::command]
async fn remove_account(app: AppHandle, provider: Provider, id: String) -> Result<Snapshot, String> {
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let active = {
        let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
        snapshot.accounts.iter().any(|a| a.provider == provider && a.id == id && a.active)
    };
    if active {
        // It would be captured again from the CLI on the next refresh.
        return Err("active".into());
    }
    if let Some(slot) = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id) {
        app.state::<AppState>().monitor.disconnect(&slot, &http_client()).await?;
        app.state::<AppState>().usage_cache.lock().unwrap().remove(&monitoring::key(&slot));
    }
    accounts::remove_slot(provider, &id).map_err(|e| e.to_string())?;
    drop(operation);
    Ok(do_refresh(&app).await)
}

#[derive(Serialize)]
struct Disconnected { snapshot: Snapshot, revoked: bool }

#[tauri::command]
async fn disconnect_monitoring(app: AppHandle, provider: Provider, id: String) -> Result<Disconnected, String> {
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let slot = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id)
        .ok_or("monitor_wrong_account")?;
    let started = Instant::now();
    let result = app.state::<AppState>().monitor.disconnect(&slot, &http_client()).await;
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::Disconnect,
        code: diagnostics::Code::from_error(result.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
    let revoked = result?;
    {
        let state = app.state::<AppState>();
        let _refresh = state.refresh_lock.lock().await;
        providers::retry_now(&mut state.usage_cache.lock().unwrap(), &monitoring::key(&slot));
    }
    drop(operation);
    Ok(Disconnected { snapshot: do_refresh(&app).await, revoked })
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<updates::UpdateInfo, String> {
    updates::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    updates::install(&app).await
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    let version = app.package_info().version.to_string();
    if cfg!(feature = "upstream-updates") { version } else { format!("{version} (independent monitoring)") }
}

#[tauri::command]
fn updates_enabled() -> bool { cfg!(feature = "upstream-updates") }

/// The OS display language (WebView2's navigator.languages doesn't follow it reliably).
#[tauri::command]
fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

/// Sync folders (OneDrive, Dropbox, iCloud Drive) Settings can offer for sharing in-use status.
#[tauri::command]
async fn sync_folders() -> Vec<presence::SyncFolder> {
    tauri::async_runtime::spawn_blocking(presence::sync_folders).await.unwrap_or_default()
}

#[derive(Serialize)]
struct RcloneRemotes { installed: bool, remotes: Vec<crate::rclone::Remote> }

/// The user's rclone remotes (names and types only), for sharing in-use status without a sync app.
#[tauri::command]
async fn rclone_remotes() -> RcloneRemotes {
    tauri::async_runtime::spawn_blocking(|| match crate::rclone::find() {
        Some(exe) => RcloneRemotes { installed: true, remotes: crate::rclone::remotes(&crate::rclone::Cli(exe)) },
        None => RcloneRemotes { installed: false, remotes: vec![] },
    }).await.unwrap_or(RcloneRemotes { installed: false, remotes: vec![] })
}

/// Whether a typed folder can hold presence files.
#[tauri::command]
fn check_presence_folder(folder: String) -> bool {
    settings::valid_presence_folder(folder.trim())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _held = state.settings_lock.lock().await;
    store_settings(&app, settings).await
}

/// Saves and applies settings; callers hold `settings_lock`.
async fn store_settings(app: &AppHandle, settings: Settings) -> Result<(), String> {
    settings::save(&settings)?;
    let previous = std::mem::replace(&mut *app.state::<AppState>().settings.lock().unwrap(), settings.clone());
    apply_settings(app);
    let providers_changed = previous.providers.claude != settings.providers.claude
        || previous.providers.codex != settings.providers.codex;
    let presence_changed = previous.presence_folder != settings.presence_folder || previous.presence_remote != settings.presence_remote;
    if presence_changed {
        // The old folder's other machines should not keep seeing this one as signed in.
        if let Some(old) = presence::store(&previous) {
            let interval = settings.refresh_minutes.max(1) as u64 * 60;
            let _ = tauri::async_runtime::spawn_blocking(move || presence::withdraw(old.as_ref(), interval)).await;
        }
    }
    if providers_changed || presence_changed {
        do_refresh(app).await;
    }
    Ok(())
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

#[tauri::command]
fn set_panel_height(app: AppHandle, height: f64) {
    *app.state::<AppState>().panel_height.lock().unwrap() = height;
    panel::place(&app);
}

#[tauri::command]
fn set_dock_width(app: AppHandle, width: f64) {
    *app.state::<AppState>().dock_width.lock().unwrap() = width;
}

#[tauri::command]
fn dock_clicked(app: AppHandle) {
    if let Some(w) = app.get_webview_window(dock::LABEL) {
        if let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) {
            *app.state::<AppState>().panel_anchor.lock().unwrap() = Some((
                pos.x as f64 + size.width as f64 / 2.0,
                pos.y as f64 + size.height as f64 / 2.0,
            ));
        }
    }
    panel::toggle(&app);
}

/// The Windows accent color as WinUI uses it on light and dark backgrounds
/// (SystemAccentColorDark1 and SystemAccentColorLight2), from the system's accent palette.
#[tauri::command]
fn accent_colors() -> Option<(String, String)> {
    let p = dock::accent_palette()?;
    let hex = |i: usize| format!("#{:02x}{:02x}{:02x}", p[i * 4], p[i * 4 + 1], p[i * 4 + 2]);
    // Palette entries: Light3, Light2, Light1, Accent, Dark1, Dark2, Dark3, (unused).
    Some((hex(4), hex(1)))
}

/// Whether the settings window is on Mica (Windows 11); elsewhere it paints its own background.
#[tauri::command]
fn has_mica() -> bool {
    dock::is_windows_11()
}

#[tauri::command]
fn taskbar_theme() -> &'static str {
    if dock::taskbar_is_light() { "light" } else { "dark" }
}

/// Right-click on the widget: the native context menu, like the built-in tray items have.
#[tauri::command]
fn dock_menu(app: AppHandle) {
    let menu = app.state::<AppState>().menu.lock().unwrap().clone();
    if let (Some(menu), Some(w)) = (menu, app.get_webview_window(dock::LABEL)) {
        let _ = w.popup_menu(&menu);
    }
}

/// Menu labels come from the frontend, which owns the translations.
#[tauri::command]
fn set_menu_labels(state: tauri::State<AppState>, labels: HashMap<String, String>) {
    for item in state.menu_items.lock().unwrap().iter() {
        if let Some(text) = labels.get(item.id().as_ref()) {
            let _ = item.set_text(text);
        }
    }
}

pub(crate) fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Applies the settings that live outside the webviews.
pub(crate) fn apply_settings(app: &AppHandle) {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(false) != settings.launch_at_login {
        let _ = if settings.launch_at_login { autolaunch.enable() } else { autolaunch.disable() };
    }
    let _ = app.emit("settings-changed", &settings);
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client")
}

/// Captures live logins into the vault, fetches every account's usage, then lets the balancer
/// switch accounts that reached their threshold.
async fn do_refresh(app: &AppHandle) -> Snapshot {
    let state = app.state::<AppState>();
    let _refresh = state.refresh_lock.lock().await;
    let started = Instant::now();
    let settings = state.settings.lock().unwrap().clone();
    state.diagnostics.poll_started(settings.refresh_minutes);
    let found = tauri::async_runtime::spawn_blocking(sources::discover)
        .await
        .unwrap_or_default();
    *state.sources.lock().unwrap() = found.clone();
    static CLEANUP_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    accounts::resolve_identities(&http_client(), &found).await;
    let scan = found.clone();
    let (active, slots) = tauri::async_runtime::spawn_blocking(move || (accounts::capture(&scan), accounts::load_slots()))
        .await
        .unwrap_or_default();
    if !CLEANUP_STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let homes = found.clone();
        let protected = state.monitor.protected_tokens(&slots).await;
        let started = Instant::now();
        let result = tauri::async_runtime::spawn_blocking(move || crate::login::cleanup(&homes, protected.as_ref())).await;
        state.diagnostics.record(None, None, diagnostics::Event::Action { action: diagnostics::Action::Cleanup,
            code: if result.is_ok() { diagnostics::Code::Ok } else { diagnostics::Code::Other }, elapsed_ms: diagnostics::millis(started) });
    }

    let mut cache = state.usage_cache.lock().unwrap().clone();
    let mut list = providers::fetch_all(&http_client(), &slots, &active, &mut cache, &settings, &state.monitor).await;
    let valid: HashSet<_> = slots.iter().map(monitoring::key).collect();
    cache.retain(|key, _| valid.contains(key));
    let saved = providers::save_cache(&cache);
    if saved.is_err() { state.diagnostics.record(None, None, diagnostics::Event::Action { action: diagnostics::Action::CacheSave, code: diagnostics::Code::Storage, elapsed_ms: 0 }); }
    *state.usage_cache.lock().unwrap() = cache;

    let decisions = {
        let last = state.last_switch.lock().unwrap().clone();
        balancer::decide(&list, &settings, &last)
    };
    for (provider, id) in decisions {
        // Publish the current numbers first so the switch notice can name both accounts.
        *state.snapshot.lock().unwrap() = Snapshot { accounts: list.clone(), updated_at: Some(chrono::Utc::now()) };
        if switch_to(app, provider, id.clone(), true).await.is_ok() {
            state.last_switch.lock().unwrap().insert(provider, Instant::now());
            for a in list.iter_mut().filter(|a| a.provider == provider) {
                let now_active = a.id == id;
                if now_active != a.active {
                    a.sources = if now_active {
                        found.iter().map(|s| s.label.clone()).collect()
                    } else {
                        vec![]
                    };
                    a.active = now_active;
                }
            }
        }
    }

    // Other machines: what they share through the presence folder, then what rising usage implies.
    let now = chrono::Utc::now();
    let interval = settings.refresh_minutes.max(1) as u64 * 60;
    if let Some(store) = presence::store(&settings) {
        let mine = list.clone();
        let peers = tauri::async_runtime::spawn_blocking(move || {
            let Some((id, name)) = presence::identity() else { return vec![] };
            let _ = store.write(&presence::own(id, name, interval, &mine, now));
            presence::read(store.as_ref(), id, now)
        }).await.unwrap_or_default();
        presence::attach(&mut list, &peers, now);
    }
    // Two refresh intervals without a rise (readings are at least a minute apart), plus slack.
    let hold = chrono::Duration::seconds(2 * interval.max(60) as i64 + 30);
    presence::infer(&mut state.presence_tracks.lock().unwrap(), &mut list, hold, now);

    let mut snapshot = Snapshot { accounts: list, updated_at: Some(chrono::Utc::now()) };
    state.diagnostics.samples(&snapshot);
    let history = {
        let mut history = state.usage_history.lock().unwrap();
        subscriptions::record(&mut history, &snapshot, &valid, chrono::Utc::now());
        history.clone()
    };
    // A reading older than two refresh intervals is not the current rate.
    let max_age = chrono::Duration::seconds(2 * interval as i64 + 300);
    subscriptions::attach_forecasts(&mut snapshot, &history, max_age, chrono::Utc::now());
    if subscriptions::save(&history).is_err() { state.diagnostics.record(None, None, diagnostics::Event::Action { action: diagnostics::Action::HistorySave, code: diagnostics::Code::Storage, elapsed_ms: 0 }); }
    state.diagnostics.record(None, None, diagnostics::Event::PollFinished { elapsed_ms: diagnostics::millis(started), accounts: snapshot.accounts.len(),
        native_sources: found.iter().filter(|s| s.distro.is_none()).count(), wsl_sources: found.iter().filter(|s| s.distro.is_some()).count() });
    state.recent.lock().unwrap().record(&snapshot.accounts, chrono::Utc::now());
    *state.snapshot.lock().unwrap() = snapshot.clone();
    let _ = app.emit("usage-updated", &snapshot);
    if settings.providers.codex { scan_codex_local(app); }
    snapshot
}

/// Re-reads the Codex CLI's local history off the async runtime, one scan at a time, then has
/// open windows reload their report. Its first run reads weeks of rollouts and can take a while.
fn scan_codex_local(app: &AppHandle) {
    static SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SCANNING.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let local = tauri::async_runtime::spawn_blocking(|| codex_local::scan(chrono::Utc::now())).await;
        SCANNING.store(false, std::sync::atomic::Ordering::Release);
        let Ok(local) = local else { return };
        let state = app.state::<AppState>();
        *state.codex_local.lock().unwrap() = Some(std::sync::Arc::new(local));
        let snapshot = state.snapshot.lock().unwrap().clone();
        let _ = app.emit("usage-updated", &snapshot);
    });
}

/// Refreshes on the interval from settings, re-reading it so changes apply without a restart.
async fn refresh_loop(app: AppHandle) {
    loop {
        do_refresh(&app).await;
        let started = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let minutes = app.state::<AppState>().settings.lock().unwrap().refresh_minutes.max(1);
            if started.elapsed() >= Duration::from_secs(minutes as u64 * 60) {
                break;
            }
        }
    }
}

pub fn run() {
    migrate::run();
    let diagnostics = std::sync::Arc::new(diagnostics::Recorder::new(settings::app_dir().expect("config directory").join("diagnostics/history.bin")));
    // Managed before the builder runs: config windows load (and call commands) before `setup`.
    let state = AppState {
        snapshot: Mutex::default(),
        settings: Mutex::new(settings::load()),
        menu: Mutex::default(),
        menu_items: Mutex::default(),
        panel_anchor: Mutex::default(),
        panel_height: Mutex::new(320.0),
        panel_hidden_at: Mutex::default(),
        dock_width: Mutex::new(160.0),
        usage_cache: Mutex::new(providers::load_cache()),
        usage_history: Mutex::new(subscriptions::load()),
        codex_local: Mutex::default(),
        monitor: monitoring::Monitor::new(settings::app_dir().expect("config directory").join("monitoring")).with_diagnostics(diagnostics.clone()),
        diagnostics: diagnostics.clone(),
        refresh_lock: tokio::sync::Mutex::new(()),
        settings_lock: tokio::sync::Mutex::new(()),
        account_operations: monitoring::Operations::default(),
        last_switch: Mutex::default(),
        recent: Mutex::default(),
        sources: Mutex::default(),
        presence_tracks: Mutex::default(),
        plan_alerts: Mutex::new(plan_alerts::load()),
    };
    tauri::Builder::default()
        // Must come first: a second launch hands over to the running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_subscriptions,
            take_plan_alerts,
            open_billing_page,
            export_diagnostics,
            record_frontend_error,
            refresh,
            get_settings,
            sync_folders,
            check_presence_folder,
            rclone_remotes,
            system_locale,
            check_for_updates,
            install_update,
            app_version,
            updates_enabled,
            save_settings,
            open_settings,
            set_panel_height,
            set_menu_labels,
            switch_account,
            add_account,
            connect_monitoring,
            disconnect_monitoring,
            remove_account,
            set_dock_width,
            dock_clicked,
            dock_menu,
            accent_colors,
            notify::notify,
            has_mica,
            taskbar_theme
        ])
        .setup(|app| {
            app.state::<AppState>().diagnostics.start();
            let handle = app.handle();
            #[cfg(windows)]
            notify::register_app_id(&app.config().identifier);

            let defaults = ["Refresh", "Settings…", "Quit"];
            let items = MENU_IDS
                .iter()
                .zip(defaults)
                .map(|(id, text)| MenuItem::with_id(app, *id, text, true, None::<&str>))
                .collect::<Result<Vec<_>, _>>()?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&items[0], &items[1], &separator, &items[2]])?;
            *app.state::<AppState>().menu.lock().unwrap() = Some(menu);
            *app.state::<AppState>().menu_items.lock().unwrap() = items;
            app.on_menu_event(|app, event| match event.id().as_ref() {
                "refresh" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { do_refresh(&app).await });
                }
                "settings" => show_settings(app),
                "quit" => app.exit(0),
                _ => {}
            });

            #[cfg(windows)]
            for label in [dock::LABEL, panel::LABEL] {
                if let Some(win) = app.get_webview_window(label) {
                    dock::exclude_from_peek(&win);
                }
            }
            #[cfg(windows)]
            if let Some(win) = app.get_webview_window(panel::LABEL) {
                panel::round_corners(&win);
            }
            if let Some(win) = app.get_webview_window(panel::LABEL) {
                let handle = handle.clone();
                win.on_window_event(move |e| {
                    if let WindowEvent::Focused(false) = e {
                        panel::on_blur(&handle);
                    }
                });
            }
            if let Some(win) = app.get_webview_window(SETTINGS_LABEL) {
                if dock::is_windows_11() {
                    use tauri::window::{Effect, EffectsBuilder};
                    let _ = win.set_effects(EffectsBuilder::new().effect(Effect::Mica).build());
                }
                let w = win.clone();
                win.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            apply_settings(handle);
            dock::show_docked(handle);

            // Updates install silently; after one, say so once in the widget.
            let current = app.package_info().version.to_string();
            let previous = {
                let state = app.state::<AppState>();
                let mut s = state.settings.lock().unwrap();
                let previous = s.last_version.replace(current.clone());
                let _ = settings::save(&s);
                previous
            };
            if previous.is_some_and(|p| p != current) {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    let _ = h.emit("update-installed", current);
                });
            }

            let h = handle.clone();
            std::thread::spawn(move || dock::keep_docked(h));
            tauri::async_runtime::spawn(refresh_loop(handle.clone()));
            tauri::async_runtime::spawn(updates::run_loop(handle.clone()));
            // Agents use Settings' operations through `slopbar-mcp`, which knocks here.
            #[cfg(windows)]
            {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    door::serve(door::pipe_name(), move |request| door_call(h.clone(), request)).await;
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building slopbar")
        .run(|app, event| {
            // App::run exits the process directly: code after it cannot flush a journal.
            if matches!(event, tauri::RunEvent::Exit) {
                // Other machines stop showing this one's accounts as in use.
                let settings = app.state::<AppState>().settings.lock().unwrap().clone();
                if let Some(store) = presence::store(&settings) {
                    presence::withdraw(store.as_ref(), settings.refresh_minutes.max(1) as u64 * 60);
                }
                app.state::<AppState>().diagnostics.record(None, None, diagnostics::Event::Shutdown);
            }
        });
}
