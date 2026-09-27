mod accounts;
mod balancer;
mod dock;
mod island;
mod model;
mod panel;
mod providers;
mod settings;
mod sources;
mod updates;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;

use accounts::Provider;
use model::Snapshot;
use settings::{DisplayMode, Settings};

const SETTINGS_LABEL: &str = "settings";
const MENU_IDS: [&str; 4] = ["show", "refresh", "settings", "quit"];

pub struct AppState {
    snapshot: Mutex<Snapshot>,
    settings: Mutex<Settings>,
    hit: Mutex<island::Shape>,
    tray_items: Mutex<Vec<MenuItem<Wry>>>,
    /// Physical center of the tray icon from the last click.
    tray_anchor: Mutex<Option<(f64, f64)>>,
    panel_height: Mutex<f64>,
    /// When opened from the tray overflow flyout: the flyout's rect, which the panel must not cover.
    panel_avoid: Mutex<Option<dock::Rect>>,
    panel_hidden_at: Mutex<Option<Instant>>,
    /// Content width of the taskbar widget, in CSS px.
    dock_width: Mutex<f64>,
    dragging: Mutex<bool>,
    /// Last known usage per account, for accounts whose saved token can't be used right now.
    usage_cache: Mutex<providers::Cache>,
    /// When each provider was last switched automatically (the balancer's cooldown).
    last_switch: Mutex<HashMap<Provider, Instant>>,
    /// Sources found by the last refresh, where switches are written.
    sources: Mutex<Vec<sources::Source>>,
}

#[tauri::command]
fn get_usage(state: tauri::State<AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
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
    tauri::async_runtime::spawn_blocking(move || accounts::switch(&found, provider, &target))
        .await
        .map_err(|e| e.to_string())??;
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
        let added = job.wait(&http_client()).await;
        if let Some(slot) = added {
            let _ = app.emit("account-added", (slot.provider, slot.email));
            do_refresh(&app).await;
        }
    });
    Ok(())
}

#[tauri::command]
async fn remove_account(app: AppHandle, provider: Provider, id: String) -> Result<Snapshot, String> {
    let active = {
        let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
        snapshot.accounts.iter().any(|a| a.provider == provider && a.id == id && a.active)
    };
    if active {
        // It would be captured again from the CLI on the next refresh.
        return Err("active".into());
    }
    accounts::remove_slot(provider, &id).map_err(|e| e.to_string())?;
    app.state::<AppState>().usage_cache.lock().unwrap().remove(&id);
    Ok(do_refresh(&app).await)
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
    app.package_info().version.to_string()
}

/// The OS display language (WebView2's navigator.languages doesn't follow it reliably).
#[tauri::command]
fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    settings::save(&settings)?;
    let previous = std::mem::replace(&mut *app.state::<AppState>().settings.lock().unwrap(), settings.clone());
    apply_settings(&app);
    let providers_changed = previous.providers.claude != settings.providers.claude
        || previous.providers.codex != settings.providers.codex;
    if providers_changed {
        do_refresh(&app).await;
    }
    Ok(())
}

#[derive(Serialize)]
struct MonitorInfo {
    name: String,
    width: u32,
    height: u32,
    primary: bool,
}

#[tauri::command]
fn list_monitors(app: AppHandle) -> Vec<MonitorInfo> {
    let primary = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
    app.available_monitors()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let name = m.name()?.clone();
            Some(MonitorInfo {
                primary: primary.as_ref() == Some(&name),
                width: m.size().width,
                height: m.size().height,
                name,
            })
        })
        .collect()
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
fn start_drag(app: AppHandle, width: f64) {
    dock::start_drag(&app, width);
}

#[tauri::command]
fn set_dock_width(app: AppHandle, width: f64) {
    *app.state::<AppState>().dock_width.lock().unwrap() = width;
}

#[tauri::command]
fn dock_clicked(app: AppHandle) {
    *app.state::<AppState>().panel_avoid.lock().unwrap() = None;
    if let Some(w) = app.get_webview_window(dock::LABEL) {
        if let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) {
            *app.state::<AppState>().tray_anchor.lock().unwrap() = Some((
                pos.x as f64 + size.width as f64 / 2.0,
                pos.y as f64 + size.height as f64 / 2.0,
            ));
        }
    }
    panel::toggle(&app);
}

#[tauri::command]
fn taskbar_theme() -> &'static str {
    if dock::taskbar_is_light() { "light" } else { "dark" }
}

#[tauri::command]
fn set_hit_shape(state: tauri::State<AppState>, shape: island::Shape) {
    *state.hit.lock().unwrap() = shape;
}

/// Tray menu labels come from the frontend, which owns the translations.
#[tauri::command]
fn set_tray_labels(state: tauri::State<AppState>, labels: HashMap<String, String>) {
    for item in state.tray_items.lock().unwrap().iter() {
        if let Some(text) = labels.get(item.id().as_ref()) {
            let _ = item.set_text(text);
        }
    }
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Shows the right window for the display mode, on the chosen monitor.
pub(crate) fn apply_settings(app: &AppHandle) {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    if let Some(main) = app.get_webview_window(island::LABEL) {
        if settings.mode == DisplayMode::Island {
            if let Some(m) = island::target_monitor(app, settings.monitor.as_deref()) {
                island::place(&main, &m, settings.anchor);
            }
            let _ = main.show();
            if let Some(p) = app.get_webview_window(panel::LABEL) {
                let _ = p.hide();
            }
        } else {
            let _ = main.hide();
        }
    }
    if settings.mode == DisplayMode::Taskbar {
        dock::show_docked(app);
    } else {
        dock::hide(app);
    }
    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(false) != settings.launch_at_login {
        let _ = if settings.launch_at_login { autolaunch.enable() } else { autolaunch.disable() };
    }
    let _ = app.emit("settings-changed", &settings);
}

fn show_usage(app: &AppHandle) {
    let mode = app.state::<AppState>().settings.lock().unwrap().mode;
    match mode {
        DisplayMode::Tray => panel::toggle(app),
        DisplayMode::Taskbar => dock_clicked(app.clone()),
        DisplayMode::Island => {
            if let Some(w) = app.get_webview_window(island::LABEL) {
                let visible = w.is_visible().unwrap_or(false);
                let _ = if visible { w.hide() } else { w.show() };
            }
        }
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

/// Captures live logins into the vault, fetches every account's usage, then lets the balancer
/// switch accounts that reached their threshold.
async fn do_refresh(app: &AppHandle) -> Snapshot {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let found = tauri::async_runtime::spawn_blocking(sources::discover)
        .await
        .unwrap_or_default();
    *state.sources.lock().unwrap() = found.clone();
    accounts::resolve_identities(&http_client(), &found).await;
    let scan = found.clone();
    let (active, slots) = tauri::async_runtime::spawn_blocking(move || (accounts::capture(&scan), accounts::load_slots()))
        .await
        .unwrap_or_default();

    let mut cache = state.usage_cache.lock().unwrap().clone();
    let mut list = providers::fetch_all(&http_client(), &slots, &active, &mut cache, &settings).await;
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

    let snapshot = Snapshot { accounts: list, updated_at: Some(chrono::Utc::now()) };
    *state.snapshot.lock().unwrap() = snapshot.clone();
    let _ = app.emit("usage-updated", &snapshot);
    snapshot
}

/// Hides the island while a fullscreen app (video, game, presentation) covers its monitor, and
/// brings it back afterwards. The taskbar widget handles this in `dock::keep_docked`.
fn watch_fullscreen(app: AppHandle) {
    let mut hidden = false;
    loop {
        std::thread::sleep(Duration::from_millis(400));
        let state = app.state::<AppState>();
        if *state.dragging.lock().unwrap() {
            continue;
        }
        let island_mode = state.settings.lock().unwrap().mode == DisplayMode::Island;
        let Some(main) = app.get_webview_window(island::LABEL) else { continue };
        let covered = island_mode
            && dock::fullscreen_monitor().is_some_and(|fs| {
                // The island's own monitor, judged by the center of its window.
                match (main.outer_position(), main.outer_size()) {
                    (Ok(p), Ok(s)) => fs.contains(p.x as f64 + s.width as f64 / 2.0, p.y as f64 + 1.0),
                    _ => false,
                }
            });
        if covered && !hidden {
            let _ = main.hide();
            hidden = true;
        } else if !covered && hidden {
            if island_mode {
                let _ = main.show();
            }
            hidden = false;
        }
    }
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
    // Managed before the builder runs: config windows load (and call commands) before `setup`.
    let state = AppState {
        snapshot: Mutex::default(),
        settings: Mutex::new(settings::load()),
        hit: Mutex::default(),
        tray_items: Mutex::default(),
        tray_anchor: Mutex::default(),
        panel_height: Mutex::new(320.0),
        panel_avoid: Mutex::default(),
        panel_hidden_at: Mutex::default(),
        dock_width: Mutex::new(160.0),
        dragging: Mutex::default(),
        usage_cache: Mutex::default(),
        last_switch: Mutex::default(),
        sources: Mutex::default(),
    };
    tauri::Builder::default()
        // Must come first: a second launch hands over to the running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_usage,
            refresh,
            get_settings,
            system_locale,
            check_for_updates,
            install_update,
            app_version,
            save_settings,
            list_monitors,
            open_settings,
            set_panel_height,
            set_hit_shape,
            set_tray_labels,
            switch_account,
            add_account,
            remove_account,
            start_drag,
            set_dock_width,
            dock_clicked,
            taskbar_theme
        ])
        .setup(|app| {
            let handle = app.handle();

            let defaults = ["Show usage", "Refresh", "Settings…", "Quit"];
            let items = MENU_IDS
                .iter()
                .zip(defaults)
                .map(|(id, text)| MenuItem::with_id(app, *id, text, true, None::<&str>))
                .collect::<Result<Vec<_>, _>>()?;
            let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
                items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<Wry>).collect();
            let menu = Menu::with_items(app, &refs)?;
            *app.state::<AppState>().tray_items.lock().unwrap() = items;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Usage Bar")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_usage(app),
                    "refresh" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move { do_refresh(&app).await });
                    }
                    "settings" => show_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        let pos = rect.position.to_physical::<f64>(1.0);
                        let size = rect.size.to_physical::<f64>(1.0);
                        let icon = (pos.x + size.width / 2.0, pos.y + size.height / 2.0);
                        // Clicked inside the tray overflow flyout (above the taskbar): open where
                        // Windows opens tray popovers, by the clock, instead of over the flyout.
                        let in_flyout = dock::taskbar_top().is_some_and(|top| icon.1 < top);
                        let anchor = if in_flyout { dock::tray_center().unwrap_or(icon) } else { icon };
                        *app.state::<AppState>().tray_anchor.lock().unwrap() = Some(anchor);
                        *app.state::<AppState>().panel_avoid.lock().unwrap() =
                            if in_flyout { dock::window_at(icon.0, icon.1) } else { None };
                        // Outside island mode, the tray icon opens the popover by the tray; it
                        // must not reuse the widget's position (that would open it mid-screen).
                        let mode = app.state::<AppState>().settings.lock().unwrap().mode;
                        if mode == DisplayMode::Island {
                            show_usage(app);
                        } else {
                            panel::toggle(app);
                        }
                    }
                })
                .build(app)?;

            for label in [island::LABEL, dock::LABEL, dock::OVERLAY] {
                if let Some(win) = app.get_webview_window(label) {
                    #[cfg(windows)]
                    island::exclude_from_peek(&win);
                }
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
                let w = win.clone();
                win.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            apply_settings(handle);

            // Updates install silently; after one, say so once in the island.
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
            std::thread::spawn(move || island::track_hover(h));
            let h = handle.clone();
            std::thread::spawn(move || dock::keep_docked(h));
            let h = handle.clone();
            std::thread::spawn(move || watch_fullscreen(h));
            tauri::async_runtime::spawn(refresh_loop(handle.clone()));
            tauri::async_runtime::spawn(updates::run_loop(handle.clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running usage-bar");
}
