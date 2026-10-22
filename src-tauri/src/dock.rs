//! The taskbar widget: laid over the Windows taskbar, just left of the notification area, so it
//! looks and behaves like a built-in tray item.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::AppState;

pub const LABEL: &str = "dock";
const GAP: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.left as f64 && x < self.right as f64 && y >= self.top as f64 && y < self.bottom as f64
    }
}

#[cfg(windows)]
mod win {
    use super::Rect;
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
        IsWindowVisible, SetWindowLongPtrW, SetWindowPos, GWLP_HWNDPARENT, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE,
    };

    /// Makes the taskbar the widget's owner. Windows always keeps an owned window above its
    /// owner, so when the taskbar is activated the widget rises with it in the same instant,
    /// instead of disappearing until we notice and raise it again. Flyouts (tray overflow,
    /// calendar, Start) are separate windows and still cover it, as they should.
    pub fn own_by_taskbar(hwnd: HWND) -> bool {
        unsafe {
            let bar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            if bar.is_null() {
                return false;
            }
            if GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) != bar as isize {
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, bar as isize);
            }
            GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) == bar as isize
        }
    }

    /// Whether the taskbar itself is the active window: the only time it covers the widget.
    /// Its flyouts (tray overflow, calendar, Start) are other windows, and the widget must
    /// stay below those instead of fighting them for the top.
    pub fn taskbar_focused() -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return false;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(fg, buf.as_mut_ptr(), buf.len() as i32);
            let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            class == "Shell_TrayWnd" || class == "Shell_SecondaryTrayWnd"
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn rect_of(hwnd: HWND) -> Option<Rect> {
        if hwnd.is_null() {
            return None;
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then_some(Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        })
    }

    /// The monitor rect when the foreground window covers a whole monitor (a fullscreen video,
    /// game or presentation). The desktop, the taskbar and our own windows don't count.
    pub fn fullscreen_monitor() -> Option<Rect> {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, &mut pid);
            if pid == std::process::id() {
                return None;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(fg, buf.as_mut_ptr(), buf.len() as i32);
            let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
                return None;
            }
            let win = rect_of(fg)?;
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut info) == 0 {
                return None;
            }
            let m = info.rcMonitor;
            let covers = win.left <= m.left && win.top <= m.top && win.right >= m.right && win.bottom >= m.bottom;
            covers.then_some(Rect { left: m.left, top: m.top, right: m.right, bottom: m.bottom })
        }
    }

    /// (taskbar, notification area) of the primary taskbar, if it is showing.
    pub fn taskbar() -> Option<(Rect, Rect)> {
        unsafe {
            let bar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            if bar.is_null() || IsWindowVisible(bar) == 0 {
                return None;
            }
            let tray = FindWindowExW(bar, std::ptr::null_mut(), wide("TrayNotifyWnd").as_ptr(), std::ptr::null());
            Some((rect_of(bar)?, rect_of(tray)?))
        }
    }

    pub enum Widgets {
        /// Turned off, or the taskbar isn't there to ask.
        Off,
        /// There, but the taskbar is slid away (auto-hide); it comes back where it was.
        Hidden,
        At(Rect),
    }

    /// Where the taskbar's Widgets (weather) button is. It is drawn inside the taskbar and has
    /// no window of its own, so only UI Automation can see it. The caller's thread must have
    /// initialized COM.
    pub fn widgets_button() -> Widgets {
        use std::mem::ManuallyDrop;
        use windows::core::BSTR;
        use windows::Win32::Foundation::HWND as Hwnd;
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
        use windows::Win32::System::Variant::{VariantClear, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR};
        use windows::Win32::UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Children, UIA_AutomationIdPropertyId,
            UIA_ClassNamePropertyId, UIA_PROPERTY_ID,
        };
        thread_local! {
            static UIA: Option<IUIAutomation> =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        }
        /// The child of `parent` whose `property` is `value`. Children only: a deeper search
        /// from the taskbar would also walk our own widget, and a UI Automation client walking a
        /// WebView2 makes it build its whole accessibility tree.
        unsafe fn child(
            uia: &IUIAutomation,
            parent: &IUIAutomationElement,
            property: UIA_PROPERTY_ID,
            value: &str,
        ) -> Option<IUIAutomationElement> {
            let mut v = VARIANT {
                Anonymous: VARIANT_0 {
                    Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                        vt: VT_BSTR,
                        Anonymous: VARIANT_0_0_0 { bstrVal: ManuallyDrop::new(BSTR::from(value)) },
                        ..Default::default()
                    }),
                },
            };
            let cond = uia.CreatePropertyCondition(property, &v);
            let _ = VariantClear(&mut v);
            parent.FindFirst(TreeScope_Children, &cond.ok()?).ok()
        }
        let bar = unsafe { FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null()) };
        if bar.is_null() {
            return Widgets::Off;
        }
        let found = UIA.with(|uia| unsafe {
            let uia = uia.as_ref()?;
            let root = uia.ElementFromHandle(Hwnd(bar)).ok()?;
            let site = child(uia, &root, UIA_ClassNamePropertyId, "Windows.UI.Input.InputSite.WindowClass")?;
            let frame = child(uia, &site, UIA_AutomationIdPropertyId, "TaskbarFrame")?;
            let button = child(uia, &frame, UIA_AutomationIdPropertyId, "WidgetsButton")?;
            if button.CurrentIsOffscreen().is_ok_and(|off| off.as_bool()) {
                return Some(Widgets::Hidden);
            }
            let r = button.CurrentBoundingRectangle().ok()?;
            (r.right > r.left).then_some(Widgets::At(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }))
        });
        found.unwrap_or(Widgets::Off)
    }

    pub fn init_com() {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    /// If the taskbar sits above `hwnd` in the z-order, moves `hwnd` to just above the taskbar
    /// (not to the very top, so tray flyouts, the calendar and Start stay above it). Returns
    /// whether it had to move. Windows 11 re-asserts the taskbar on top at times without
    /// activating it, which would otherwise hide the widget until the taskbar is clicked.
    pub fn stay_above_taskbar(hwnd: HWND) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindow, GW_HWNDPREV};
        unsafe {
            let bar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            if bar.is_null() {
                return false;
            }
            // Walk up from our window: meeting the taskbar means it covers us.
            let mut w = GetWindow(hwnd, GW_HWNDPREV);
            let mut covered = false;
            for _ in 0..512 {
                if w.is_null() {
                    break;
                }
                if w == bar {
                    covered = true;
                    break;
                }
                w = GetWindow(w, GW_HWNDPREV);
            }
            if !covered {
                return false;
            }
            // SetWindowPos puts us below `after`: the window right above the taskbar.
            let above_bar = GetWindow(bar, GW_HWNDPREV);
            let after = if above_bar.is_null() { HWND_TOPMOST } else { above_bar };
            SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            true
        }
    }

    /// The taskbar is topmost too; whoever asserted it last wins.
    pub fn raise(hwnd: HWND) {
        unsafe {
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    pub fn taskbar_is_light() -> bool {
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value = wide("SystemUsesLightTheme");
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut _,
                &mut size,
            )
        };
        ok == 0 && data == 1
    }

    pub fn is_windows_11() -> bool {
        use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
        let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
        let value = wide("CurrentBuildNumber");
        let mut data = [0u16; 16];
        let mut size = (data.len() * 2) as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                data.as_mut_ptr() as *mut _,
                &mut size,
            )
        };
        let text = String::from_utf16_lossy(&data[..(size as usize / 2).saturating_sub(1)]);
        ok == 0 && text.trim().parse::<u32>().is_ok_and(|build| build >= 22000)
    }

    pub fn accent_palette() -> Option<[u8; 32]> {
        use windows_sys::Win32::System::Registry::RRF_RT_REG_BINARY;
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent");
        let value = wide("AccentPalette");
        let mut data = [0u8; 32];
        let mut size = data.len() as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_BINARY,
                std::ptr::null_mut(),
                data.as_mut_ptr() as *mut _,
                &mut size,
            )
        };
        (ok == 0 && size == 32).then_some(data)
    }
}

#[cfg(not(windows))]
mod win {
    use super::Rect;
    pub fn taskbar() -> Option<(Rect, Rect)> {
        None
    }
    pub fn taskbar_focused() -> bool {
        false
    }
    pub fn fullscreen_monitor() -> Option<Rect> {
        None
    }
    pub enum Widgets {
        Off,
        #[allow(dead_code)]
        Hidden,
        #[allow(dead_code)]
        At(Rect),
    }
    pub fn widgets_button() -> Widgets {
        Widgets::Off
    }
    pub fn init_com() {}
    pub fn own_by_taskbar(_: ()) -> bool {
        false
    }
    pub fn taskbar_is_light() -> bool {
        false
    }
    pub fn accent_palette() -> Option<[u8; 32]> {
        None
    }
    pub fn is_windows_11() -> bool {
        false
    }
}

pub fn taskbar_is_light() -> bool {
    win::taskbar_is_light()
}

/// Windows 11 has the Mica and rounded-corner materials the settings window relies on.
pub fn is_windows_11() -> bool {
    win::is_windows_11()
}

/// Whether a fullscreen app (video, game, presentation) is in front.
pub fn fullscreen() -> bool {
    win::fullscreen_monitor().is_some()
}

/// The system accent palette: eight RGBA entries from lightest to darkest.
pub fn accent_palette() -> Option<[u8; 32]> {
    win::accent_palette()
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// The taskbar's Widgets button, as last seen by `watch_widgets`.
static WIDGETS: Mutex<Option<Rect>> = Mutex::new(None);

/// Looks for the Widgets button every second. It moves when the weather text changes length
/// or the user switches the taskbar alignment. A thread of its own, because UI Automation asks
/// Explorer, which can be slow to answer, and docking must never wait on it.
fn watch_widgets() {
    win::init_com();
    loop {
        match win::widgets_button() {
            win::Widgets::Off => *WIDGETS.lock().unwrap() = None,
            win::Widgets::At(rect) => *WIDGETS.lock().unwrap() = Some(rect),
            // Keep the last place, so the widget doesn't land on the weather as the taskbar
            // slides back in.
            win::Widgets::Hidden => {}
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Left edge of a widget `width` wide, docked against the notification area. With the taskbar
/// left-aligned, Windows 11 puts the Widgets (weather) button there too; the widget then sits
/// left of it instead of on top of it. Centered, that button is at the far left, out of the way.
pub fn dock_left(bar: Rect, tray: Rect, widgets: Option<Rect>, width: i32) -> i32 {
    let mut right = tray.left;
    if let Some(w) = widgets {
        let on_bar = w.top < bar.bottom && w.bottom > bar.top;
        if on_bar && w.left < right && w.right > right - width - GAP {
            right = w.left;
        }
    }
    right - width - GAP
}

/// Docks the widget over the taskbar, left of the notification area. `raise` brings it back
/// above the taskbar (only needed when the taskbar has just been activated).
fn place_docked(app: &AppHandle, win: &WebviewWindow, raise: bool) -> bool {
    let Some((bar, tray)) = win::taskbar() else { return false };
    let scale = win.scale_factor().unwrap_or(1.0);
    let width = (*app.state::<AppState>().dock_width.lock().unwrap() * scale).ceil() as i32;
    let height = bar.bottom - bar.top;
    let widgets = *WIDGETS.lock().unwrap();
    let pos = PhysicalPosition::new(dock_left(bar, tray, widgets, width), bar.top);
    let size = PhysicalSize::new(width as u32, height as u32);
    if win.outer_position().ok() != Some(pos) {
        let _ = win.set_position(pos);
    }
    if win.outer_size().ok() != Some(size) {
        let _ = win.set_size(size);
    }
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        // Owned by the taskbar, the widget stays above it on its own; raising by hand is only
        // a fallback (it lags a tick behind and flickers when the tray is toggled quickly).
        let owned = win::own_by_taskbar(hwnd.0 as _);
        if raise && !owned {
            win::raise(hwnd.0 as _);
        }
        win::stay_above_taskbar(hwnd.0 as _);
    }
    #[cfg(not(windows))]
    let _ = raise;
    true
}

/// Keeps the docked widget glued to the taskbar (it moves, resizes, and fights for z-order).
pub fn keep_docked(app: AppHandle) {
    std::thread::spawn(watch_widgets);
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let Some(win) = window(&app) else { continue };
        let covered = match (win::fullscreen_monitor(), win::taskbar()) {
            (Some(fs), Some((bar, _))) => fs.contains(((bar.left + bar.right) / 2) as f64, bar.top as f64 - 1.0),
            _ => false,
        };
        if covered {
            if win.is_visible().unwrap_or(false) {
                let _ = win.hide();
            }
            continue;
        }
        if place_docked(&app, &win, win::taskbar_focused()) {
            if !win.is_visible().unwrap_or(false) {
                let _ = win.show();
            }
        } else if win.is_visible().unwrap_or(false) {
            // Taskbar hidden (auto-hide or a fullscreen app).
            let _ = win.hide();
        }
    }
}

pub fn show_docked(app: &AppHandle) {
    if let Some(win) = window(app) {
        if place_docked(app, &win, true) {
            let _ = win.show();
        }
    }
}

/// Keeps the widget visible while Aero Peek previews a window from the taskbar.
#[cfg(windows)]
pub fn exclude_from_peek(window: &WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK};
    let Ok(hwnd) = window.hwnd() else { return };
    let value: i32 = 1;
    unsafe {
        DwmSetWindowAttribute(
            hwnd.0 as _,
            DWMWA_EXCLUDED_FROM_PEEK as u32,
            &value as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Measured on a 3840-wide taskbar with the weather showing next to the tray.
    const BAR: Rect = Rect { left: 0, top: 2328, right: 3840, bottom: 2400 };
    const TRAY: Rect = Rect { left: 3279, top: 2328, right: 3840, bottom: 2400 };

    #[test]
    fn docks_against_the_tray_without_widgets() {
        assert_eq!(dock_left(BAR, TRAY, None, 98), 3279 - 98 - GAP);
    }

    #[test]
    fn steps_left_of_the_weather_next_to_the_tray() {
        let weather = Rect { left: 3033, top: 2328, right: 3261, bottom: 2400 };
        assert_eq!(dock_left(BAR, TRAY, Some(weather), 98), 3033 - 98 - GAP);
    }

    #[test]
    fn ignores_widgets_at_the_far_left_of_a_centered_taskbar() {
        let weather = Rect { left: 0, top: 2328, right: 228, bottom: 2400 };
        assert_eq!(dock_left(BAR, TRAY, Some(weather), 98), 3279 - 98 - GAP);
    }

    #[test]
    fn steps_left_of_widgets_that_only_reach_into_its_spot() {
        let weather = Rect { left: 3000, top: 2328, right: 3180, bottom: 2400 };
        assert_eq!(dock_left(BAR, TRAY, Some(weather), 98), 3000 - 98 - GAP);
    }

    #[test]
    fn stays_by_the_tray_when_widgets_leave_room() {
        let weather = Rect { left: 2900, top: 2328, right: 3177, bottom: 2400 };
        assert_eq!(dock_left(BAR, TRAY, Some(weather), 98), 3279 - 98 - GAP);
    }

    #[test]
    fn ignores_widgets_on_another_taskbar() {
        let elsewhere = Rect { left: 3033, top: 1000, right: 3261, bottom: 1072 };
        assert_eq!(dock_left(BAR, TRAY, Some(elsewhere), 98), 3279 - 98 - GAP);
    }
}
