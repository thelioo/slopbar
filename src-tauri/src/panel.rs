//! The flyout: opens above the taskbar widget when it is clicked, like the flyouts of the
//! built-in tray items (quick settings, calendar), and closes when it loses focus.
//!
//! A click must open it at once, like the shell's own flyouts. Building a WebView2 takes
//! seconds, so the flyout is built hidden shortly after startup (`prepare`) and is only ever
//! hidden, never destroyed. (WebView2's low memory target while hidden was tried: the flyout then
//! showed blank, so it is not used.) Once it has been on screen, hiding suspends its webview.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Theme, WindowEvent};

use crate::AppState;

pub const LABEL: &str = "panel";
const WIDTH: f64 = 380.0;
/// Windows 11 flyouts keep 12 px from the taskbar and the screen edge.
const MARGIN: f64 = 12.0;
/// Longest wait for a new flyout's page to lay itself out before it is shown anyway.
const FIRST_PAINT: Duration = Duration::from_millis(1500);
/// How long after startup the hidden flyout is built, so it does not compete with the first
/// refresh and the widget's first paint.
const PREPARE_AFTER: Duration = Duration::from_secs(2);

/// The page has reported its height since the window was built (see `page_ready`).
static READY: AtomicBool = AtomicBool::new(false);
/// A click arrived while the flyout was still being built: show it once it is.
static SHOW_WHEN_BUILT: AtomicBool = AtomicBool::new(false);
/// The flyout is being built (by `prepare` or a click), so a second build is not started.
static BUILDING: AtomicBool = AtomicBool::new(false);
/// The flyout's webview has been on screen at least once, so hiding may suspend it.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// The flyout's webview was suspended when it was last hidden.
static SUSPENDED: AtomicBool = AtomicBool::new(false);

/// The flyout's page has laid itself out (it reports its height through `set_panel_height`).
pub fn page_ready() {
    READY.store(true, Ordering::Release);
}

/// Builds the flyout hidden, so the first click opens it at once.
pub fn prepare(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PREPARE_AFTER).await;
        build(&app).await;
    });
}

/// Builds the flyout if it is not there, then shows it if a click asked for it meanwhile.
async fn build(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_some() || BUILDING.swap(true, Ordering::AcqRel) {
        return;
    }
    READY.store(false, Ordering::Release);
    SHOWN.store(false, Ordering::Release);
    SUSPENDED.store(false, Ordering::Release);
    let built = crate::app::on_demand_window(app, LABEL).await;
    if built.is_some() {
        let started = Instant::now();
        while !READY.load(Ordering::Acquire) && started.elapsed() < FIRST_PAINT {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    BUILDING.store(false, Ordering::Release);
    let Some(win) = built else { return };
    let _ = win;
    // Not suspended here: a webview suspended before it ever drew opens as an empty panel.
    if SHOW_WHEN_BUILT.swap(false, Ordering::AcqRel) {
        show(app);
    }
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
        // Not built yet (a click within moments of startup, or the webview went away): build it
        // and show it once its page has laid out.
        SHOW_WHEN_BUILT.store(true, Ordering::Release);
        let app = app.clone();
        tauri::async_runtime::spawn(async move { build(&app).await });
        return;
    };
    if BUILDING.load(Ordering::Acquire) {
        SHOW_WHEN_BUILT.store(true, Ordering::Release);
        return;
    }
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
    set_webview_visible(&win, true);
    let _ = win.set_focus();
    let _ = app.emit("panel-visible", true);
}

fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        set_webview_visible(&win, false);
        let _ = win.hide();
    }
    *app.state::<AppState>().panel_hidden_at.lock().unwrap() = Some(Instant::now());
    let _ = app.emit("panel-visible", false);
}

/// Esc in the flyout, like the shell's own flyouts.
pub fn close(app: &AppHandle) {
    hide(app);
}

pub fn on_blur(app: &AppHandle) {
    hide(app);
}

/// Tells the flyout's webview it is (or is no longer) on screen. Showing the window alone can leave
/// a webview that was built hidden, and stayed hidden, not drawing: the flyout then opens as an
/// empty panel. `inspect` does the same when it shows a hidden window off-screen.
///
/// Once it has been shown, hiding also suspends the webview, which lets WebView2 give back most of
/// its working set (about 450 MB down to 120 MB across the app's processes); showing resumes it first.
#[cfg(windows)]
fn set_webview_visible(win: &tauri::WebviewWindow, visible: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use webview2_com::TrySuspendCompletedHandler;
    use windows::core::Interface;
    let _ = win.with_webview(move |w| unsafe {
        let controller = w.controller();
        let webview = controller.CoreWebView2().ok().and_then(|c| c.cast::<ICoreWebView2_3>().ok());
        if visible {
            if SUSPENDED.swap(false, Ordering::AcqRel) {
                if let Some(webview) = &webview {
                    let _ = webview.Resume();
                }
            }
            let _ = controller.SetIsVisible(true);
            let _ = controller.NotifyParentWindowPositionChanged();
            SHOWN.store(true, Ordering::Release);
        } else {
            let _ = controller.SetIsVisible(false);
            // Never before it has been on screen once: suspended before it ever drew, it opens empty.
            // A close hides more than once (blur, then the click); suspend once.
            if SHOWN.load(Ordering::Acquire) && !SUSPENDED.swap(true, Ordering::AcqRel) {
                if let Some(webview) = &webview {
                    let _ = webview.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(()))));
                }
            }
        }
    });
}

#[cfg(not(windows))]
fn set_webview_visible(_win: &tauri::WebviewWindow, _visible: bool) {}

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
