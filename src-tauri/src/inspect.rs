//! Lets an agent see a SlopBar window without the user's screen: the window's visible text and a
//! PNG drawn by WebView2 itself. A hidden window is shown off-screen without activation and
//! without a taskbar button, then hidden again, so nothing moves under the user and focus stays
//! where it was. A Settings page chosen for the look is switched back afterwards.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::door::SETTINGS_PAGES as PAGES;
/// Longest text returned; the pages are far shorter.
const MAX_TEXT: usize = 20_000;
/// Captures kept on disk; older ones are removed.
const KEEP: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Settings,
    /// The flyout that opens from the taskbar widget.
    Flyout,
    /// The quick switcher.
    Switcher,
}

impl Target {
    fn label(self) -> &'static str {
        match self {
            Target::Settings => "settings",
            Target::Flyout => crate::panel::LABEL,
            Target::Switcher => crate::app::SWITCHER_LABEL,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub window: Target,
    pub page: Option<String>,
    /// Whether the user had the window open; if not, it was drawn off-screen and hidden again.
    pub was_visible: bool,
    pub text: String,
    pub screenshot: Option<PathBuf>,
}

/// Where captures are written, beside diagnostics exports.
pub fn captures_dir() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("diagnostics").join("captures"))
}

#[cfg(windows)]
pub async fn inspect(app: &AppHandle, target: Target, page: Option<String>, screenshot: bool) -> Result<Inspection, String> {
    if let Some(p) = page.as_deref() {
        if target != Target::Settings || !PAGES.contains(&p) {
            return Err(format!("page is only for the settings window, one of {PAGES:?}"));
        }
    }
    // Settings and the switcher exist only while open: one that is closed is created for the
    // look, given a moment to load, and destroyed again afterwards.
    let (window, created) = match app.get_webview_window(target.label()) {
        Some(w) => (w, false),
        None if crate::app::ON_DEMAND.contains(&target.label()) => {
            let w = crate::app::on_demand_window(app, target.label()).await.ok_or("window not available")?;
            (w, true)
        }
        None => return Err("window not available".into()),
    };
    let result = look(&window, target, page, screenshot, created).await;
    // Unless the user opened it meanwhile.
    if created && !window.is_visible().unwrap_or(false) {
        let _ = window.destroy();
    }
    result
}

#[cfg(windows)]
async fn look(window: &tauri::WebviewWindow, target: Target, page: Option<String>, screenshot: bool, created: bool) -> Result<Inspection, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging as wm;
    let window = window.clone();
    // Kept as a number: a raw window handle cannot be held across an await.
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let controller = {
        let (tx, rx) = tokio::sync::oneshot::channel();
        window.with_webview(move |w| { let _ = tx.send(SendController(w.controller())); }).map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "webview not available")?
    };
    if created {
        // The page loads its data as it starts (Settings fetches several reports).
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while script(&window, &controller, "document.readyState").await.as_deref() != Ok("\"complete\"") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    let was_visible = unsafe { wm::IsWindowVisible(hwnd as windows_sys::Win32::Foundation::HWND) } != 0;
    let mut restore = None;
    if !was_visible {
        restore = Some(on_main(&window, {
            let controller = controller.clone();
            move || unsafe { let controller = controller; show_off_screen(hwnd as windows_sys::Win32::Foundation::HWND, &controller.0) }
        }).await?);
        // A webview that was hidden needs a moment to lay out and paint again.
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    let result = async {
        let previous = match &page {
            Some(p) => {
                let before = script(&window, &controller, "document.querySelector('.nav.active')?.dataset.page ?? null").await?;
                let found = script(&window, &controller, &format!("(() => {{ const b = document.querySelector('.nav[data-page={}]'); b?.click(); return !!b; }})()", serde_json::to_string(p).unwrap())).await?;
                if found != "true" { return Err("that page is not in this build's Settings window".to_string()); }
                // Pages load their data on open (the Subscriptions report is fetched then).
                tokio::time::sleep(Duration::from_millis(800)).await;
                serde_json::from_str::<Option<String>>(&before).ok().flatten()
            }
            None => None,
        };
        let text = script(&window, &controller, "(document.querySelector('main .page.active') ?? document.body).innerText").await?;
        let mut text: String = serde_json::from_str(&text).unwrap_or_default();
        if text.chars().count() > MAX_TEXT { text = text.chars().take(MAX_TEXT).collect(); }
        let shot = if screenshot { Some(capture(&window, &controller, target, page.as_deref()).await?) } else { None };
        if let (Some(before), Some(now)) = (previous, page.as_deref()) {
            if before != now {
                let _ = script(&window, &controller, &format!("document.querySelector('.nav[data-page={}]')?.click()", serde_json::to_string(&before).unwrap())).await;
            }
        }
        Ok((text, shot))
    }.await;
    if let Some(saved) = restore {
        let controller = controller.clone();
        let _ = on_main(&window, move || unsafe { let controller = controller; hide_again(hwnd as windows_sys::Win32::Foundation::HWND, &controller.0, saved) }).await;
    }
    let (text, screenshot) = result?;
    Ok(Inspection { window: target, page, was_visible, text, screenshot })
}

#[cfg(not(windows))]
pub async fn inspect(_: &AppHandle, _: Target, _: Option<String>, _: bool) -> Result<Inspection, String> {
    Err("window inspection is only available on Windows".into())
}

/// The webview's controller. It is only ever called on the main thread (`on_main`); other threads
/// just carry it there.
#[cfg(windows)]
#[derive(Clone)]
struct SendController(webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller);
#[cfg(windows)]
unsafe impl Send for SendController {}
#[cfg(windows)]
unsafe impl Sync for SendController {}

#[cfg(windows)]
struct Saved { x: i32, y: i32, ex_style: isize }

/// Runs `f` on the main thread, where the window and its webview live.
#[cfg(windows)]
async fn on_main<T: Send + 'static>(window: &tauri::WebviewWindow, f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window.run_on_main_thread(move || { let _ = tx.send(f()); }).map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "main thread unavailable".to_string())
}

#[cfg(windows)]
unsafe fn show_off_screen(hwnd: windows_sys::Win32::Foundation::HWND, controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller) -> Saved {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging as wm;
    let mut rect: RECT = std::mem::zeroed();
    wm::GetWindowRect(hwnd, &mut rect);
    let ex_style = wm::GetWindowLongPtrW(hwnd, wm::GWL_EXSTYLE);
    // A tool window gets no taskbar button, so the taskbar does not flicker either.
    wm::SetWindowLongPtrW(hwnd, wm::GWL_EXSTYLE, (ex_style | wm::WS_EX_TOOLWINDOW as isize | wm::WS_EX_NOACTIVATE as isize) & !(wm::WS_EX_APPWINDOW as isize));
    // Left of every monitor.
    let left = wm::GetSystemMetrics(wm::SM_XVIRTUALSCREEN) - (rect.right - rect.left) - 100;
    wm::SetWindowPos(hwnd, std::ptr::null_mut(), left, rect.top, 0, 0, wm::SWP_NOSIZE | wm::SWP_NOZORDER | wm::SWP_NOACTIVATE);
    wm::ShowWindow(hwnd, wm::SW_SHOWNOACTIVATE);
    let _ = controller.SetIsVisible(true);
    Saved { x: rect.left, y: rect.top, ex_style }
}

#[cfg(windows)]
unsafe fn hide_again(hwnd: windows_sys::Win32::Foundation::HWND, controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller, saved: Saved) {
    use windows_sys::Win32::UI::WindowsAndMessaging as wm;
    let _ = controller.SetIsVisible(false);
    wm::ShowWindow(hwnd, wm::SW_HIDE);
    wm::SetWindowPos(hwnd, std::ptr::null_mut(), saved.x, saved.y, 0, 0, wm::SWP_NOSIZE | wm::SWP_NOZORDER | wm::SWP_NOACTIVATE);
    wm::SetWindowLongPtrW(hwnd, wm::GWL_EXSTYLE, saved.ex_style);
}

/// Runs a script in the page and returns its result as JSON text.
#[cfg(windows)]
async fn script(window: &tauri::WebviewWindow, controller: &SendController, js: &str) -> Result<String, String> {
    use webview2_com::ExecuteScriptCompletedHandler;
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let controller = controller.clone();
    let js = js.to_string();
    on_main(window, move || unsafe {
        let controller = controller;
        let run = || -> windows::core::Result<()> {
            let webview = controller.0.CoreWebView2()?;
            webview.ExecuteScript(&windows::core::HSTRING::from(js), &ExecuteScriptCompletedHandler::create(Box::new(move |status, json| {
                let _ = tx.send(status.map(|_| json).map_err(|e| e.to_string()));
                Ok(())
            })))
        };
        run().map_err(|e| e.to_string())
    }).await??;
    tokio::time::timeout(Duration::from_secs(10), rx).await.map_err(|_| "script timed out")?.map_err(|_| "script dropped")?
}

/// Writes the webview's current picture to a new PNG under `captures_dir`.
#[cfg(windows)]
async fn capture(window: &tauri::WebviewWindow, controller: &SendController, target: Target, page: Option<&str>) -> Result<PathBuf, String> {
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
    use windows::Win32::System::Com::{STGM_CREATE, STGM_WRITE};
    let dir = captures_dir().ok_or("no app data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("SlopBar-{}{}-{}.png", target.label(), page.map(|p| format!("-{p}")).unwrap_or_default(), chrono::Utc::now().format("%Y%m%d-%H%M%S%3f"));
    let path = dir.join(name);
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let controller = controller.clone();
    let file = path.clone();
    on_main(window, move || unsafe {
        let controller = controller;
        let run = || -> windows::core::Result<()> {
            let webview = controller.0.CoreWebView2()?;
            let stream = windows::Win32::UI::Shell::SHCreateStreamOnFileEx(&windows::core::HSTRING::from(file.as_os_str()), (STGM_CREATE | STGM_WRITE).0, 0, true, None)?;
            let held = stream.clone();
            webview.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &CapturePreviewCompletedHandler::create(Box::new(move |status| {
                // Releasing the stream closes the file.
                drop(held);
                let _ = tx.send(status.map_err(|e| e.to_string()));
                Ok(())
            })))
        };
        run().map_err(|e| e.to_string())
    }).await??;
    tokio::time::timeout(Duration::from_secs(10), rx).await.map_err(|_| "capture timed out")?.map_err(|_| "capture dropped")??;
    prune(&dir);
    Ok(path)
}

/// Keeps the newest `KEEP` captures in `dir`; other files are left alone.
pub fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut shots: Vec<_> = entries.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("SlopBar-") && e.file_name().to_string_lossy().ends_with(".png"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    shots.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in shots.into_iter().skip(KEEP) { let _ = std::fs::remove_file(path); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn only_the_newest_captures_are_kept_and_other_files_are_left_alone() {
        let dir = TempDir::new();
        for i in 0..13 {
            std::fs::write(dir.0.join(format!("SlopBar-settings-{i:02}.png")), b"png").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        std::fs::write(dir.0.join("notes.txt"), b"keep").unwrap();
        prune(&dir.0);
        let mut left: Vec<String> = std::fs::read_dir(&dir.0).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left.len(), KEEP + 1);
        assert!(left.contains(&"notes.txt".to_string()) && left.contains(&"SlopBar-settings-12.png".to_string()));
        assert!(!left.contains(&"SlopBar-settings-00.png".to_string()));
    }
}
