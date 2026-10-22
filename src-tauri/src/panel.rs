//! The flyout: opens above the taskbar widget when it is clicked, like the flyouts of the
//! built-in tray items (quick settings, calendar), and closes when it loses focus.
//!
//! It is built on the first click (`"create": false`, one of `app::ON_DEMAND`) and destroyed once
//! it has stayed closed for `LINGER`, so with the native widget nothing keeps a WebView2 alive
//! while only the widget shows; opening it again within `LINGER` is instant.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Theme, WindowEvent};

use crate::AppState;

pub const LABEL: &str = "panel";
const WIDTH: f64 = 380.0;
/// Windows 11 flyouts keep 12 px from the taskbar and the screen edge.
const MARGIN: f64 = 12.0;
/// How long a closed flyout is kept for a quick reopen before its webview is released.
const LINGER: Duration = Duration::from_secs(60);
/// Longest wait for a new flyout's page to lay itself out before it is shown anyway.
const FIRST_PAINT: Duration = Duration::from_millis(1500);

/// The page has reported its height since the window was built (see `page_ready`).
static READY: AtomicBool = AtomicBool::new(false);

/// The flyout's page has laid itself out (it reports its height through `set_panel_height`).
pub fn page_ready() {
    READY.store(true, Ordering::Release);
}

/// Set up a newly built flyout window: rounded, kept visible under Aero Peek, closed on blur.
pub fn watch(app: &AppHandle, win: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        crate::dock::exclude_from_peek(win);
        round_corners(win);
    }
    let handle = app.clone();
    win.on_window_event(move |e| {
        if let WindowEvent::Focused(false) = e {
            on_blur(&handle);
        }
    });
}

/// Positions the flyout above the widget, clamped to the monitor's work area.
pub fn place(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else { return };
    let state = app.state::<AppState>();
    let height = *state.panel_height.lock().unwrap();
    let anchor = *state.panel_anchor.lock().unwrap();

    let monitor = anchor
        .and_then(|(x, y)| app.monitor_from_point(x, y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let area = m.work_area();
    let (ax, ay, aw, ah) = (
        area.position.x as f64,
        area.position.y as f64,
        area.size.width as f64,
        area.size.height as f64,
    );
    // Never taller than the work area: with many accounts the flyout's top (its header and the
    // first provider) would otherwise sit off-screen. The account list scrolls instead.
    let height = height.min((ah / scale - 2.0 * MARGIN).max(200.0));
    let (w, h, margin) = (WIDTH * scale, height * scale, MARGIN * scale);
    let (anchor_x, anchor_y) = anchor.unwrap_or((ax + aw, ay + ah));

    let x = (anchor_x - w / 2.0).clamp(ax + margin, ax + aw - w - margin);
    // Taskbar at the bottom (the usual case) puts the widget below the work area's middle.
    let y = if anchor_y >= ay + ah / 2.0 { ay + ah - h - margin } else { ay + margin };

    let _ = win.set_size(LogicalSize::new(WIDTH, height));
    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub fn toggle(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else {
        // Not built yet (or released): build it, and show it once its page has laid out.
        READY.store(false, Ordering::Release);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if crate::app::on_demand_window(&app, LABEL).await.is_none() {
                return;
            }
            let started = Instant::now();
            while !READY.load(Ordering::Acquire) && started.elapsed() < FIRST_PAINT {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            show(&app);
        });
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide(app);
        return;
    }
    // Clicking the widget blurs (and hides) the flyout just before the click arrives.
    let just_hidden = app
        .state::<AppState>()
        .panel_hidden_at
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(250));
    if just_hidden {
        return;
    }
    show(app);
}

fn show(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else { return };
    // Flyouts follow the taskbar's theme ("Windows mode"), not the apps' one.
    let theme = if crate::dock::taskbar_is_light() { Theme::Light } else { Theme::Dark };
    let _ = win.set_theme(Some(theme));
    place(app);
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("panel-visible", true);
}

fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
    }
    *app.state::<AppState>().panel_hidden_at.lock().unwrap() = Some(Instant::now());
    let _ = app.emit("panel-visible", false);
    // Released unless it is opened again in the meantime.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LINGER).await;
        let state = app.state::<AppState>();
        let quiet = state.panel_hidden_at.lock().unwrap().is_some_and(|t| t.elapsed() >= LINGER);
        if let Some(win) = app.get_webview_window(LABEL) {
            if quiet && !win.is_visible().unwrap_or(true) {
                let _ = win.destroy();
            }
        }
    });
}

/// Esc in the flyout, like the shell's own flyouts.
pub fn close(app: &AppHandle) {
    hide(app);
}

pub fn on_blur(app: &AppHandle) {
    hide(app);
}

/// Rounded corners like every Windows 11 flyout (undecorated windows are square by default).
#[cfg(windows)]
pub fn round_corners(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    let Ok(hwnd) = window.hwnd() else { return };
    let value = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            hwnd.0 as _,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &value as *const _ as *const _,
            std::mem::size_of_val(&value) as u32,
        );
    }
}
