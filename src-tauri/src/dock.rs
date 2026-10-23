//! The taskbar widget: laid over the Windows taskbar, just left of the notification area, so it
//! looks and behaves like a built-in tray item.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

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
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, IsWindow,
        IsWindowVisible, SetWindowLongPtrW, SetWindowPos, GWLP_HWNDPARENT, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE,
    };

    /// Makes the taskbar the widget's owner. Windows always keeps an owned window above its
    /// owner, so when the taskbar is activated the widget rises with it in the same instant,
    /// instead of disappearing until we notice and raise it again. Flyouts (tray overflow,
    /// calendar, Start) are separate windows and still cover it, as they should.
    pub fn own_by_taskbar(hwnd: HWND) -> bool {
        unsafe {
            let bar = bar();
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

    /// The window's outer rect, whether it is visible, and its scale (DPI / 96), straight from
    /// Windows: no round trip through the event loop, unlike Tauri's getters.
    pub fn window_state(hwnd: HWND) -> Option<(Rect, bool, f64)> {
        use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
        let rect = rect_of(hwnd)?;
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        Some((rect, unsafe { IsWindowVisible(hwnd) } != 0, if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }))
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

    /// The primary taskbar and its notification area, found once and kept while they exist
    /// (Explorer restarting makes new ones): finding them is the costly part of each look.
    static BAR: AtomicIsize = AtomicIsize::new(0);
    static TRAY: AtomicIsize = AtomicIsize::new(0);

    /// `cache`'s window if it still exists, else `find()`'s (kept for next time).
    fn kept(cache: &AtomicIsize, find: impl FnOnce() -> HWND) -> HWND {
        let known = cache.load(Ordering::Relaxed) as HWND;
        if !known.is_null() && unsafe { IsWindow(known) } != 0 {
            return known;
        }
        let found = find();
        cache.store(found as isize, Ordering::Relaxed);
        found
    }

    fn bar() -> HWND {
        kept(&BAR, || unsafe { FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null()) })
    }

    /// (taskbar, notification area) of the primary taskbar, if it is showing.
    pub fn taskbar() -> Option<(Rect, Rect)> {
        unsafe {
            let bar = bar();
            if bar.is_null() || IsWindowVisible(bar) == 0 {
                return None;
            }
            let tray = kept(&TRAY, || FindWindowExW(bar, std::ptr::null_mut(), wide("TrayNotifyWnd").as_ptr(), std::ptr::null()));
            Some((rect_of(bar)?, rect_of(tray)?))
        }
    }

    /// A taskbar button UI Automation sees, and for one whose width changes on its own (the
    /// Widgets/weather button, whose text changes), the element, to read its place again cheaply.
    pub struct Seen {
        pub rect: Rect,
        live: Option<windows::Win32::UI::Accessibility::IUIAutomationElement>,
    }

    thread_local! {
        static UIA: Option<windows::Win32::UI::Accessibility::IUIAutomation> = unsafe {
            use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
            CoCreateInstance(&windows::Win32::UI::Accessibility::CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };
    }

    /// Every button on the taskbar (Start, search, task buttons, the Widgets/weather button, the
    /// tray's icons), whatever draws it. The weather and Windows 11's buttons are drawn inside the
    /// taskbar and have no window of their own (on some taskbars one XAML window spans all of it),
    /// so only UI Automation can see them. Our own windows are skipped: a UI Automation client
    /// walking a WebView2 makes it build its whole accessibility tree. None when nothing is on
    /// screen (the taskbar slid away) or UI Automation can't answer: keep what was seen before.
    /// The caller's thread must have initialized COM.
    pub fn scan() -> Option<Vec<Seen>> {
        use windows::Win32::Foundation::HWND as Hwnd;
        use windows::Win32::System::Variant::VARIANT;
        use windows::Win32::UI::Accessibility::{
            TreeScope_Children, TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId,
            UIA_ButtonControlTypeId, UIA_ControlTypePropertyId, UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
            UIA_ProcessIdPropertyId,
        };
        let bar = bar();
        if bar.is_null() {
            return None;
        }
        let me = std::process::id() as i32;
        UIA.with(|uia| unsafe {
            let uia = uia.as_ref()?;
            let root = uia.ElementFromHandle(Hwnd(bar)).ok()?;
            // One round trip to Explorer per window instead of one per property per button.
            let cache = uia.CreateCacheRequest().ok()?;
            for p in [
                UIA_BoundingRectanglePropertyId,
                UIA_ProcessIdPropertyId,
                UIA_AutomationIdPropertyId,
                UIA_NamePropertyId,
                UIA_IsOffscreenPropertyId,
            ] {
                cache.AddProperty(p).ok()?;
            }
            let kids = root.FindAllBuildCache(TreeScope_Children, &uia.CreateTrueCondition().ok()?, &cache).ok()?;
            let buttons = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0)).ok()?;
            let mut out = Vec::new();
            for i in 0..kids.Length().ok()? {
                let Ok(kid) = kids.GetElement(i) else { continue };
                if kid.CachedProcessId().ok()? == me {
                    continue;
                }
                let Ok(found) = kid.FindAllBuildCache(TreeScope_Descendants, &buttons, &cache) else { continue };
                for j in 0..found.Length().unwrap_or(0) {
                    let Ok(e) = found.GetElement(j) else { continue };
                    if e.CachedIsOffscreen().map_or(true, |off| off.as_bool()) {
                        continue;
                    }
                    let Ok(r) = e.CachedBoundingRectangle() else { continue };
                    if r.right <= r.left {
                        continue;
                    }
                    let id = e.CachedAutomationId().map(|s| s.to_string()).unwrap_or_default();
                    let name = e.CachedName().map(|s| s.to_string()).unwrap_or_default();
                    // Windows 11 names it; elsewhere it is the button showing a temperature.
                    let weather = id == "WidgetsButton" || name.contains('\u{b0}');
                    out.push(Seen {
                        rect: Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
                        live: weather.then_some(e),
                    });
                }
            }
            (!out.is_empty()).then_some(out)
        })
    }

    /// Reads the places of the buttons whose width changes on their own again. False when one
    /// can't be read (Explorer restarted, the weather was turned off): scan again. A button off
    /// screen (the taskbar slid away) keeps its last place, so the widget doesn't land on the
    /// weather as the taskbar slides back in.
    pub fn reread(seen: &mut [Seen]) -> bool {
        for s in seen {
            let Some(e) = &s.live else { continue };
            unsafe {
                match e.CurrentIsOffscreen() {
                    Ok(off) if off.as_bool() => continue,
                    Ok(_) => {}
                    Err(_) => return false,
                }
                let Ok(r) = e.CurrentBoundingRectangle() else { return false };
                if r.right <= r.left {
                    return false;
                }
                s.rect = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
            }
        }
        true
    }

    /// The taskbar's visible child windows: (class, rect).
    pub fn child_windows() -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        unsafe {
            let bar = bar();
            if bar.is_null() {
                return out;
            }
            let mut child = FindWindowExW(bar, std::ptr::null_mut(), std::ptr::null(), std::ptr::null());
            while !child.is_null() {
                if IsWindowVisible(child) != 0 {
                    let mut buf = [0u16; 128];
                    let n = GetClassNameW(child, buf.as_mut_ptr(), buf.len() as i32);
                    if let Some(r) = rect_of(child) {
                        out.push((String::from_utf16_lossy(&buf[..n.max(0) as usize]), r));
                    }
                }
                child = FindWindowExW(bar, child, std::ptr::null(), std::ptr::null());
            }
        }
        out
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
            let bar = bar();
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
    pub struct Seen {
        pub rect: Rect,
    }
    pub fn scan() -> Option<Vec<Seen>> {
        None
    }
    pub fn reread(_: &mut [Seen]) -> bool {
        true
    }
    pub fn child_windows() -> Vec<(String, Rect)> {
        Vec::new()
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

/// What is on the taskbar (its buttons, the weather), as last seen by `watch_taskbar`.
static OCCUPIED: Mutex<Vec<Rect>> = Mutex::new(Vec::new());

/// How often the weather button's place is read again (cheap: the button is already found).
const REREAD_EVERY: Duration = Duration::from_secs(2);
/// How often the taskbar's buttons are looked for afresh when the taskbar has not changed (task
/// buttons come and go as apps open and close).
const SCAN_EVERY: Duration = Duration::from_secs(10);
/// How often the docking loop looks at the widget and the taskbar.
const DOCK_TICK: Duration = Duration::from_millis(100);
/// How often (in docking ticks) the taskbar's theme is read: every two seconds.
const THEME_TICKS: u32 = 20;

/// Looks for the taskbar's buttons as soon as the taskbar or the notification area moves or
/// resizes (`changed`), and every `SCAN_EVERY` otherwise; in between it rereads the weather
/// button's place every `REREAD_EVERY`, since it moves when the weather text changes length. A
/// thread of its own, because UI Automation asks Explorer, which can be slow to answer (and
/// costs it work), and docking must never wait on it.
fn watch_taskbar(changed: std::sync::mpsc::Receiver<()>) {
    win::init_com();
    let mut seen = Vec::new();
    let mut search = true;
    let mut searched = std::time::Instant::now();
    loop {
        if search || !win::reread(&mut seen) {
            searched = std::time::Instant::now();
            if let Some(found) = win::scan() {
                seen = found;
            }
        }
        let mut occupied: Vec<Rect> = seen.iter().map(|s| s.rect).collect();
        if let Some((bar, _)) = win::taskbar() {
            occupied.extend(occupying(bar, &win::child_windows()));
        }
        *OCCUPIED.lock().unwrap() = occupied;
        search = match changed.recv_timeout(REREAD_EVERY) {
            // A burst of changes (the taskbar sliding in) is one search.
            Ok(()) => {
                while changed.try_recv().is_ok() {}
                true
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => searched.elapsed() >= SCAN_EVERY,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
    }
}

/// Where on the taskbar the widget docks, as (right edge, room): the first free span, walking
/// left from the notification area, at least `need` wide with a `GAP` on each side, and the
/// widest the widget may be there (`room`) without touching what is to its left. Usually that
/// is right by the tray; with the weather (or a long row of task buttons) there, it is left of
/// them. Only what is on the taskbar's row counts (another monitor's taskbar doesn't), and
/// nothing right of the tray's left edge (the tray's own icons). Where nothing is that wide it
/// tries without the gaps, then for `least` (the rings without their padding), and failing all
/// of those takes the widest free span there is, so it covers as little as it can.
pub fn dock_span(bar: Rect, tray: Rect, occupied: &[Rect], need: i32, least: i32) -> (i32, i32) {
    let row: Vec<Rect> = occupied
        .iter()
        .filter(|o| o.top < bar.bottom && o.bottom > bar.top && o.right > o.left && o.left < tray.left)
        .copied()
        .collect();
    for (need, margin) in [(need, GAP), (need, 0), (least, GAP), (least, 0)] {
        if let Some(span) = fit(bar, tray, &row, need, margin) {
            return span;
        }
    }
    widest(bar, tray, &row).unwrap_or((tray.left - GAP, need))
}

/// The first span, walking left from the tray, `need` wide with `margin` on each side.
fn fit(bar: Rect, tray: Rect, row: &[Rect], need: i32, margin: i32) -> Option<(i32, i32)> {
    let mut right = tray.left;
    loop {
        let left = right - margin - need - margin;
        if left < bar.left {
            return None;
        }
        match row.iter().filter(|o| o.left < right && o.right > left).map(|o| o.left).min() {
            Some(l) => right = l,
            None => break,
        }
    }
    let wall = row.iter().filter(|o| o.left < right).map(|o| o.right).max().unwrap_or(bar.left).max(bar.left);
    Some((right - margin, right - margin - (wall + margin)))
}

/// The widest free span between the taskbar's left edge and the tray, as (right edge, width).
fn widest(bar: Rect, tray: Rect, row: &[Rect]) -> Option<(i32, i32)> {
    let mut sorted = row.to_vec();
    sorted.sort_by_key(|o| o.left);
    let mut best: Option<(i32, i32)> = None;
    let mut wall = bar.left;
    for o in sorted.iter().map(|o| (o.left, o.right)).chain([(tray.left, tray.left)]) {
        let free = o.0.min(tray.left) - wall;
        if free > 0 && best.is_none_or(|b| free > b.1) {
            best = Some((o.0.min(tray.left), free));
        }
        wall = wall.max(o.1);
    }
    best
}

/// The taskbar's own windows that show something (on Windows 10 the weather, `DynamicContent2`,
/// and search highlights, `DynamicContent1`, are windows of their own, not UI Automation
/// buttons). Containers don't count: the task list and its rebar span all the space the task
/// buttons may use (the buttons themselves are counted by UI Automation), the tray is docked
/// against, and the XAML bridges of newer taskbars span the whole bar. Nor does anything as wide
/// as half the taskbar, whatever it is called.
pub fn occupying(bar: Rect, windows: &[(String, Rect)]) -> Vec<Rect> {
    const CONTAINERS: &[&str] = &[
        "ReBarWindow32",
        "MSTaskSwWClass",
        "MSTaskListWClass",
        "TrayNotifyWnd",
        "Windows.UI.Composition.DesktopWindowContentBridge",
        "Windows.UI.Input.InputSite.WindowClass",
    ];
    windows
        .iter()
        .filter(|(class, r)| {
            !CONTAINERS.contains(&class.as_str()) && r.right > r.left && 2 * (r.right - r.left) < bar.right - bar.left
        })
        .map(|(_, r)| *r)
        .collect()
}

/// Left edge of a widget `width` wide in the span `dock_span` found for it, and its width there
/// (no wider than the room).
pub fn dock_left((right, room): (i32, i32), width: i32) -> (i32, i32) {
    let w = width.min(room);
    (right - w, w)
}

/// What docking needs to know of the web widget's window.
struct Frame {
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    visible: bool,
    scale: f64,
}

/// The widget window's handle.
#[cfg(windows)]
fn handle(win: &WebviewWindow) -> Option<isize> {
    win.hwnd().ok().map(|h| h.0 as isize)
}
#[cfg(not(windows))]
fn handle(_: &WebviewWindow) -> Option<isize> {
    Some(0)
}

/// Whether a handle read earlier is still the web widget's window (it is destroyed when the
/// native widget takes over, and created again when it is turned off).
#[cfg(windows)]
fn alive(hwnd: isize) -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as _) != 0 }
}
#[cfg(not(windows))]
fn alive(_: isize) -> bool {
    true
}

/// Read straight from Windows: each of Tauri's getters is a round trip through the event loop,
/// which ten times a second kept the main thread busy.
#[cfg(windows)]
fn frame(_: &WebviewWindow, hwnd: isize) -> Option<Frame> {
    let (r, visible, scale) = win::window_state(hwnd as _)?;
    Some(Frame {
        pos: PhysicalPosition::new(r.left, r.top),
        size: PhysicalSize::new((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32),
        visible,
        scale,
    })
}
#[cfg(not(windows))]
fn frame(win: &WebviewWindow, _: isize) -> Option<Frame> {
    Some(Frame { pos: win.outer_position().ok()?, size: win.outer_size().ok()?, visible: win.is_visible().ok()?, scale: win.scale_factor().ok()? })
}

/// Which widget is up: the native one (setting `native_widget`, the default) or the web one.
enum Host {
    Web { win: WebviewWindow, hwnd: isize, frame: Frame },
    #[cfg(windows)]
    Native,
}

impl Host {
    fn visible(&self) -> bool {
        match self {
            Host::Web { frame, .. } => frame.visible,
            #[cfg(windows)]
            Host::Native => crate::widget_win::visible(),
        }
    }
    fn show(&self) {
        match self {
            Host::Web { win, .. } => {
                let _ = win.show();
            }
            #[cfg(windows)]
            Host::Native => crate::widget_win::show(),
        }
    }
    fn hide(&self) {
        match self {
            Host::Web { win, .. } => {
                let _ = win.hide();
            }
            #[cfg(windows)]
            Host::Native => crate::widget_win::hide(),
        }
    }
}

/// Whether the native widget is the one wanted (Windows only; elsewhere the web one).
pub fn native_wanted(app: &AppHandle) -> bool {
    cfg!(windows) && app.state::<AppState>().settings.lock().unwrap().native_widget
}

/// The widget that is up, if any. `cache` keeps the web window's handle between docking ticks.
fn host(app: &AppHandle, cache: &mut Option<isize>) -> Option<Host> {
    #[cfg(windows)]
    if native_wanted(app) {
        return crate::widget_win::running().then_some(Host::Native);
    }
    let win = window(app)?;
    let hwnd = match *cache {
        Some(h) if alive(h) => h,
        _ => handle(&win)?,
    };
    *cache = Some(hwnd);
    let frame = frame(&win, hwnd)?;
    Some(Host::Web { win, hwnd, frame })
}

/// The widget's center on screen, where the flyout opens above it.
pub fn anchor(app: &AppHandle) -> Option<(f64, f64)> {
    match host(app, &mut None)? {
        Host::Web { frame, .. } => {
            Some((frame.pos.x as f64 + frame.size.width as f64 / 2.0, frame.pos.y as f64 + frame.size.height as f64 / 2.0))
        }
        #[cfg(windows)]
        Host::Native => crate::widget_win::anchor(),
    }
}

/// Puts up the widget the settings ask for, and takes the other one down: switching needs no
/// restart. The web widget's window is built (from its configuration) only when it is wanted,
/// so the native one runs without a WebView2 behind it.
pub fn apply_mode(app: &AppHandle, native: bool) {
    #[cfg(windows)]
    if native {
        if let Some(w) = window(app) {
            let _ = w.destroy();
        }
        crate::widget_win::start(app);
        return;
    }
    #[cfg(windows)]
    crate::widget_win::hide();
    let _ = native;
    if window(app).is_some() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if crate::app::on_demand_window(&app, LABEL).await.is_some() {
            show_docked(&app);
        }
    });
}

/// Docks the widget over `taskbar` (bar, notification area), left of the notification area.
/// `raise` brings it back above the taskbar (only needed when the taskbar has just been
/// activated). Moves or resizes the window only when it is not where it belongs already.
fn place_docked(app: &AppHandle, host: &Host, taskbar: Option<(Rect, Rect)>, raise: bool) -> bool {
    let Some((bar, tray)) = taskbar else { return false };
    let height = bar.bottom - bar.top;
    let occupied = OCCUPIED.lock().unwrap().clone();
    let hwnd: Option<isize> = match host {
        Host::Web { win, hwnd, frame } => {
            // The web widget sizes itself; it can only be put where it fits.
            let width = (*app.state::<AppState>().dock_width.lock().unwrap() * frame.scale).ceil() as i32;
            let (right, _) = dock_span(bar, tray, &occupied, width, width);
            let pos = PhysicalPosition::new(right - width, bar.top);
            let size = PhysicalSize::new(width as u32, height as u32);
            if frame.pos != pos {
                let _ = win.set_position(pos);
            }
            if frame.size != size {
                let _ = win.set_size(size);
            }
            Some(*hwnd)
        }
        #[cfg(windows)]
        Host::Native => {
            use crate::widget_win as native;
            let dpi = native::dpi_at(bar);
            let width = native::width_px(dpi);
            // A notice is wider than the rings: the span is picked for the rings (so a notice
            // doesn't move the widget), and the notice is cut short to fit it.
            let span = dock_span(bar, tray, &occupied, width.min(native::rings_px(dpi)), native::least_px(dpi));
            let (x, w) = dock_left(span, width);
            native::place(native::Place { x, y: bar.top, w, h: height, dpi, room: span.1 });
            native::hwnd().map(|h| h as isize)
        }
    };
    #[cfg(windows)]
    if let Some(hwnd) = hwnd {
        // Owned by the taskbar, the widget stays above it on its own; raising by hand is only
        // a fallback (it lags a tick behind and flickers when the tray is toggled quickly).
        let owned = win::own_by_taskbar(hwnd as _);
        if raise && !owned {
            win::raise(hwnd as _);
        }
        win::stay_above_taskbar(hwnd as _);
    }
    #[cfg(not(windows))]
    let _ = (raise, hwnd);
    true
}

/// Keeps the docked widget glued to the taskbar (it moves, resizes, and fights for z-order),
/// and tells the windows when the taskbar's theme changes (`taskbar-theme`).
pub fn keep_docked(app: AppHandle) {
    let (changed, watch) = std::sync::mpsc::channel();
    std::thread::spawn(move || watch_taskbar(watch));
    let mut cache = None;
    let mut last_taskbar = None;
    let mut light = taskbar_is_light();
    let mut ticks = 0u32;
    loop {
        std::thread::sleep(DOCK_TICK);
        ticks = ticks.wrapping_add(1);
        if ticks % THEME_TICKS == 0 && taskbar_is_light() != light {
            light = !light;
            let _ = app.emit("taskbar-theme", if light { "light" } else { "dark" });
        }
        let taskbar = win::taskbar();
        if taskbar != last_taskbar {
            last_taskbar = taskbar;
            let _ = changed.send(());
        }
        let Some(host) = host(&app, &mut cache) else { continue };
        let covered = match (win::fullscreen_monitor(), taskbar) {
            (Some(fs), Some((bar, _))) => fs.contains(((bar.left + bar.right) / 2) as f64, bar.top as f64 - 1.0),
            _ => false,
        };
        if covered {
            if host.visible() {
                host.hide();
            }
            continue;
        }
        if place_docked(&app, &host, taskbar, win::taskbar_focused()) {
            if !host.visible() {
                host.show();
            }
        } else if host.visible() {
            // Taskbar hidden (auto-hide or a fullscreen app).
            host.hide();
        }
    }
}

pub fn show_docked(app: &AppHandle) {
    let Some(host) = host(app, &mut None) else { return };
    if place_docked(app, &host, win::taskbar(), true) {
        host.show();
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

    // Measured on a 3840-wide left-aligned Windows 11 taskbar with the weather next to the tray.
    const BAR: Rect = Rect { left: 0, top: 2328, right: 3840, bottom: 2400 };
    const TRAY: Rect = Rect { left: 3279, top: 2328, right: 3840, bottom: 2400 };

    fn at(left: i32, right: i32) -> Rect {
        Rect { left, top: 2328, right, bottom: 2400 }
    }

    /// Start, search, Task View and ten task buttons, as UI Automation reported them.
    fn left_side() -> Vec<Rect> {
        let mut v = vec![at(0, 83), at(86, 416), at(419, 485)];
        v.extend((0..10).map(|i| at(485 + 66 * i, 551 + 66 * i)));
        v
    }

    #[test]
    fn docks_against_the_tray_with_nothing_else_there() {
        assert_eq!(dock_span(BAR, TRAY, &[], 98, 98), (3279 - GAP, 3279 - 2 * GAP));
        assert_eq!(dock_span(BAR, TRAY, &left_side(), 98, 98), (3279 - GAP, 3279 - GAP - 1145 - GAP));
        assert_eq!(dock_left(dock_span(BAR, TRAY, &left_side(), 98, 98), 98), (3279 - GAP - 98, 98));
    }

    #[test]
    fn steps_left_of_the_weather_next_to_the_tray() {
        let mut o = left_side();
        o.push(at(3033, 3261));
        let span = dock_span(BAR, TRAY, &o, 98, 98);
        assert_eq!(span, (3033 - GAP, 3033 - GAP - 1145 - GAP));
        assert_eq!(dock_left(span, 98), (3033 - GAP - 98, 98));
    }

    #[test]
    fn steps_left_of_weather_that_only_reaches_into_its_spot() {
        assert_eq!(dock_span(BAR, TRAY, &[at(3000, 3180)], 98, 98).0, 3000 - GAP);
    }

    #[test]
    fn stays_by_the_tray_when_the_weather_leaves_room() {
        // The widget spans 3177..3275 with a gap of 4 on each side.
        assert_eq!(dock_span(BAR, TRAY, &[at(2900, 3173)], 98, 98), (3279 - GAP, 98));
        assert_eq!(dock_span(BAR, TRAY, &[at(2900, 3174)], 98, 98).0, 2900 - GAP);
    }

    #[test]
    fn ignores_the_weather_at_the_far_left_of_a_centered_taskbar() {
        // Windows 11 centered: weather far left, task buttons in the middle.
        let mut o = vec![at(0, 228)];
        o.extend((0..10).map(|i| at(1590 + 66 * i, 1656 + 66 * i)));
        assert_eq!(dock_span(BAR, TRAY, &o, 98, 98), (3279 - GAP, 3279 - GAP - 2250 - GAP));
    }

    #[test]
    fn ignores_what_is_on_another_taskbar_or_in_the_tray() {
        let elsewhere = Rect { left: 3033, top: 1000, right: 3261, bottom: 1072 };
        let tray_icon = at(3327, 3375);
        assert_eq!(dock_span(BAR, TRAY, &[elsewhere, tray_icon], 98, 98).0, 3279 - GAP);
    }

    #[test]
    fn walks_past_task_buttons_that_reach_the_weather() {
        // Windows 10 style: the weather by the tray, and task buttons running up to it.
        let o = vec![at(2700, 2766), at(2766, 2832), at(2832, 2898), at(2900, 3030), at(3033, 3261)];
        assert_eq!(dock_span(BAR, TRAY, &o, 98, 98).0, 2700 - GAP);
    }

    #[test]
    fn a_notice_is_cut_to_the_room_between_its_neighbours() {
        // Task buttons up to 2800, the weather from 3033: 225 px free between them.
        let o = vec![at(2734, 2800), at(3033, 3261)];
        let span = dock_span(BAR, TRAY, &o, 66, 66);
        assert_eq!(span, (3029, 3029 - 2804));
        // A notice 600 px wide is cut to the room; the rings fit as they are.
        assert_eq!(dock_left(span, 600), (2804, 225));
        assert_eq!(dock_left(span, 66), (3029 - 66, 66));
    }

    #[test]
    fn a_full_taskbar_takes_the_widest_free_span() {
        let mut o: Vec<Rect> = (0..60).map(|i| at(i * 60, i * 60 + 58)).collect();
        // Every gap is 2 px, but one is 10.
        o[40] = at(2400, 2450);
        assert_eq!(dock_span(BAR, TRAY, &o, 98, 81), (2460, 10));
    }

    // UltraMagnus's Windows 10 taskbar (physical px), where the weather is a window of its own.
    const BAR10: Rect = Rect { left: 0, top: 2100, right: 3840, bottom: 2160 };
    const TRAY10: Rect = Rect { left: 3440, top: 2100, right: 3840, bottom: 2160 };

    fn win10(class: &str, left: i32, right: i32) -> (String, Rect) {
        (class.to_string(), Rect { left, top: 2100, right, bottom: 2160 })
    }

    fn win10_windows() -> Vec<(String, Rect)> {
        vec![
            win10("Start", 0, 72),
            win10("DynamicContent1", 384, 504),
            win10("ReBarWindow32", 504, 3128),
            win10("DynamicContent2", 3234, 3428),
            win10("TrayNotifyWnd", 3440, 3840),
        ]
    }

    #[test]
    fn counts_windows_10s_weather_window_but_not_its_containers() {
        let o = occupying(BAR10, &win10_windows());
        assert_eq!(o, vec![win10("", 0, 72).1, win10("", 384, 504).1, win10("", 3234, 3428).1]);
        let bridge = win10("Windows.UI.Composition.DesktopWindowContentBridge", 0, 3840);
        let unknown_but_huge = win10("SomethingNew", 100, 3000);
        assert!(occupying(BAR10, &[bridge, unknown_but_huge]).is_empty());
    }

    #[test]
    fn docks_left_of_windows_10s_weather_window() {
        // The widget was at 3337-3436, over the weather; it goes left of it.
        let o = occupying(BAR10, &win10_windows());
        let span = dock_span(BAR10, TRAY10, &o, 99, 81);
        assert_eq!(span.0, 3234 - GAP);
        let (x, w) = dock_left(span, 99);
        assert_eq!((x, w), (3230 - 99, 99));
        assert!(x + w <= 3234);
    }

    #[test]
    fn squeezes_into_a_tight_gap_rather_than_overlap() {
        // The search box, and task buttons filling the task list up to 3128: 106 px to the
        // weather, too tight for 99 px with a 4 px gap each side, so it goes without the gaps.
        let mut o = occupying(BAR10, &win10_windows());
        o.push(win10("", 72, 384).1);
        o.push(win10("", 504, 3128).1);
        let span = dock_span(BAR10, TRAY10, &o, 99, 81);
        assert_eq!(span, (3234, 106));
        let (x, w) = dock_left(span, 99);
        assert!(x >= 3128 && x + w <= 3234, "{x} {w}");
        // 90 px: only the rings without their padding fit.
        o.push(win10("", 3128, 3144).1);
        assert_eq!(dock_span(BAR10, TRAY10, &o, 99, 81), (3234 - GAP, 3234 - GAP - 3144 - GAP));
    }

    /// Against this machine's real taskbar: `cargo test --lib real_taskbar -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scans_the_real_taskbar() {
        win::init_com();
        let started = std::time::Instant::now();
        let mut seen = win::scan().expect("no taskbar buttons found");
        let took = started.elapsed();
        assert!(win::reread(&mut seen));
        let (bar, tray) = win::taskbar().unwrap();
        let rects: Vec<Rect> = seen.iter().map(|s| s.rect).collect();
        for r in &rects {
            println!("{r:?}");
        }
        println!("{} buttons in {took:?}; bar {bar:?} tray {tray:?}", rects.len());
        let windows = win::child_windows();
        println!("child windows {windows:?}; occupying {:?}", occupying(bar, &windows));
        println!("span for 99 px: {:?}", dock_span(bar, tray, &rects, 99, 81));
    }

    #[test]
    fn a_window_spanning_the_whole_taskbar_counts_from_its_left_edge() {
        // Should something span the bar, nothing left of it is free: dock against the tray.
        assert_eq!(dock_span(BAR, TRAY, &[at(0, 3840)], 98, 98), (3279 - GAP, 98));
    }
}
