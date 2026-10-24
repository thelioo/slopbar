//! The native taskbar widget's window: a layered Win32 popup drawn by `widget_draw`, on a thread
//! of its own with its own message loop. dock.rs still docks it (position, z-order, fullscreen);
//! this module paints, follows the taskbar's theme, shows the tooltips and the menu, and opens
//! the flyout on click, as the web widget did (src/main.js, `IS_DOCK`).
//!
//! It costs no WebView2: its idle footprint is the window, a software Direct2D target and a DIB.

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Listener, Manager};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi as gdi;
use windows_sys::Win32::UI::Controls as cc;
use windows_sys::Win32::UI::Input::KeyboardAndMouse as km;
use windows_sys::Win32::UI::WindowsAndMessaging as wm;

use crate::accounts::Provider;
use crate::widget::{self, Lang, Texts, Theme, View};
use crate::widget_draw::{Look, Renderer};
use crate::AppState;

const WM_VIEW: u32 = wm::WM_APP + 1;
const WM_PLACE: u32 = wm::WM_APP + 2;
const WM_REDRAW: u32 = wm::WM_APP + 3;
const TIMER: usize = 1;
/// How long a notice replaces the rings, as `showToast` in main.js.
const TOAST: Duration = Duration::from_secs(4);
/// Two rings' width in DIPs (the widget's usual width).
const RINGS: f32 = 66.0;
const MENU_REFRESH: usize = 1;
const MENU_SETTINGS: usize = 2;
const MENU_QUIT: usize = 3;

/// Where dock.rs puts the widget, in physical pixels, and the DPI of that monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub dpi: u32,
    /// How wide it may be there without reaching over the taskbar's other buttons.
    pub room: i32,
}

/// What the app side tells the widget thread.
struct Shared {
    view: View,
    theme: Theme,
    /// The flyout is open: the widget keeps its highlight, like `#widget.open`.
    open: bool,
    toast: Option<(String, Instant)>,
    health_problems: usize,
    /// Where dock.rs wants it.
    want: Option<Place>,
    /// Where it was last drawn (its width follows what it shows).
    shown: Option<Place>,
    /// The content's width in DIPs, from the last layout.
    width: f32,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static HWND_: AtomicIsize = AtomicIsize::new(0);
static APP: OnceLock<AppHandle> = OnceLock::new();

fn shared<T>(f: impl FnOnce(&mut Shared) -> T) -> Option<T> {
    SHARED.lock().unwrap().as_mut().map(f)
}

pub fn hwnd() -> Option<HWND> {
    let h = HWND_.load(Ordering::Acquire);
    (h != 0).then_some(h as HWND)
}

pub fn running() -> bool {
    hwnd().is_some()
}

fn post(msg: u32) {
    if let Some(h) = hwnd() {
        unsafe { wm::PostMessageW(h, msg, 0, 0) };
    }
}

/// The widget's width at `dpi`, in physical pixels: what dock.rs docks.
pub fn width_px(dpi: u32) -> i32 {
    widget::physical(shared(|s| s.width).unwrap_or(RINGS), dpi)
}

/// Two rings' width at `dpi`: what dock.rs finds room for, however wide a notice is.
pub fn rings_px(dpi: u32) -> i32 {
    widget::physical(RINGS, dpi)
}

/// Moves (and redraws) the widget; a no-op when it is already there.
pub fn place(p: Place) {
    let changed = shared(|s| {
        let changed = s.want != Some(p);
        s.want = Some(p);
        changed
    });
    if changed == Some(true) {
        post(WM_PLACE);
    }
}

/// The widget's center, where the flyout opens above.
pub fn anchor() -> Option<(f64, f64)> {
    let p = shared(|s| s.shown.or(s.want)).flatten()?;
    Some((p.x as f64 + p.w as f64 / 2.0, p.y as f64 + p.h as f64 / 2.0))
}

pub fn visible() -> bool {
    hwnd().is_some_and(|h| unsafe { wm::IsWindowVisible(h) } != 0)
}

pub fn show() {
    if let Some(h) = hwnd() {
        unsafe { wm::ShowWindowAsync(h, wm::SW_SHOWNOACTIVATE) };
    }
}

pub fn hide() {
    if let Some(h) = hwnd() {
        unsafe { wm::ShowWindowAsync(h, wm::SW_HIDE) };
    }
}

/// The monitor DPI where `r` is (the taskbar), so the widget is drawn for the screen it is on.
pub fn dpi_at(r: crate::dock::Rect) -> u32 {
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let rect = RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        let mon = gdi::MonitorFromRect(&rect, gdi::MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y) != 0 {
            return 96;
        }
        x
    }
}

fn lang(app: &AppHandle) -> Lang {
    Lang(crate::i18n::resolve(&app.state::<AppState>().settings.lock().unwrap().language))
}

/// The view for the app's current state: its snapshot, language, health and any notice.
fn current_view(app: &AppHandle) -> View {
    let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
    let (toast, problems) = shared(|s| {
        if s.toast.as_ref().is_some_and(|(_, at)| at.elapsed() >= TOAST) {
            s.toast = None;
        }
        (s.toast.as_ref().map(|(t, _)| t.clone()), s.health_problems)
    })
    .unwrap_or((None, 0));
    widget::view(&snapshot, toast.as_deref(), problems, &lang(app))
}

/// Rebuilds what the widget shows (after a snapshot, a settings change or a notice).
pub fn refresh(app: &AppHandle) {
    let view = current_view(app);
    let theme = Theme::of_taskbar(crate::dock::taskbar_is_light());
    let changed = shared(|s| {
        let changed = s.view != view || s.theme != theme;
        s.view = view;
        s.theme = theme;
        changed
    });
    if changed == Some(true) {
        post(WM_VIEW);
    }
}

fn toast(app: &AppHandle, text: String) {
    shared(|s| s.toast = Some((text, Instant::now())));
    refresh(app);
}

/// Settings > Health's problem count, for the warning dot.
fn check_health(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let report = crate::app::health_report(&app).await;
        shared(|s| s.health_problems = report.problems);
        refresh(&app);
    });
}

/// Agents' view of the widget (MCP `inspect_window`): its text and a PNG drawn by the same code,
/// at the size and DPI it has on the taskbar (100% and 48 px when it isn't docked).
pub fn inspect(app: &AppHandle) -> Result<(String, Vec<u8>, bool), String> {
    let view = current_view(app);
    let texts = lang(app);
    let text = widget::text(&view, &texts);
    let (place, theme, open) = shared(|s| (s.shown.or(s.want), s.theme, s.open)).unwrap_or((None, Theme::Dark, false));
    let (dpi, h) = place.map_or((96, 48), |p| (p.dpi, p.h));
    let renderer = Renderer::new().map_err(|e| e.to_string())?;
    let look = Look { theme, hot: open, pressed: false };
    let (surface, _) = renderer.draw_within(&view, look, dpi, h, place.map(|p| p.room)).ok_or("drawing failed")?;
    Ok((text, surface.png(), visible()))
}

/// Starts the widget (once): its window, its thread and its event listeners. Hidden until
/// dock.rs places and shows it.
pub fn start(app: &AppHandle) {
    if running() {
        refresh(app);
        return;
    }
    let _ = APP.set(app.clone());
    let texts = lang(app);
    *SHARED.lock().unwrap() = Some(Shared {
        view: widget::view(&Default::default(), None, 0, &texts),
        theme: Theme::of_taskbar(crate::dock::taskbar_is_light()),
        open: false,
        toast: None,
        health_problems: 0,
        want: None,
        shown: None,
        width: 66.0,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("taskbar-widget".into())
        .spawn(move || run(tx))
        .expect("widget thread");
    // The window exists before dock.rs looks for it.
    let _ = rx.recv_timeout(Duration::from_secs(5));
    listen(app);
    refresh(app);
    check_health(app);
}

#[derive(Deserialize)]
struct Switched {
    provider: Provider,
    to: Option<String>,
    #[serde(default)]
    auto: bool,
}

#[derive(Deserialize)]
struct Login {
    error: Option<String>,
}

/// The events the web widget listened to (main.js), handled here instead.
fn listen(app: &AppHandle) {
    let a = app.clone();
    app.listen_any("usage-updated", move |_| {
        refresh(&a);
        check_health(&a);
    });
    let a = app.clone();
    app.listen_any("settings-changed", move |_| refresh(&a));
    let a = app.clone();
    app.listen_any("update-installed", move |e| {
        let v: String = serde_json::from_str(e.payload()).unwrap_or_default();
        toast(&a, lang(&a).t("updatedTo", &[("v", &v)]));
    });
    let a = app.clone();
    app.listen_any("account-switched", move |e| {
        let Ok(s) = serde_json::from_str::<Switched>(e.payload()) else { return };
        let key = if s.auto { "switchedAuto" } else { "switched" };
        let text = lang(&a).t(key, &[("provider", widget::provider_name(s.provider)), ("to", s.to.as_deref().unwrap_or(""))]);
        toast(&a, text);
    });
    let a = app.clone();
    app.listen_any("monitoring-login", move |e| {
        let texts = lang(&a);
        let text = match serde_json::from_str::<Login>(e.payload()).ok().and_then(|l| l.error) {
            Some(err) => widget::error_text(&texts, "", &err),
            None => texts.t("monitorConnected", &[]),
        };
        toast(&a, text);
        check_health(&a);
    });
    app.listen_any("panel-visible", move |e| {
        let open = e.payload() == "true";
        if shared(|s| std::mem::replace(&mut s.open, open) != open) == Some(true) {
            post(WM_REDRAW);
        }
    });
}

/// State only the widget thread touches.
struct Local {
    renderer: Option<Renderer>,
    tooltip: HWND,
    tools: usize,
    hot: bool,
    pressed: bool,
    light: bool,
}

thread_local! {
    static LOCAL: RefCell<Local> = RefCell::new(Local {
        renderer: None,
        tooltip: std::ptr::null_mut(),
        tools: 0,
        hot: false,
        pressed: false,
        light: false,
    });
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn run(ready: std::sync::mpsc::Sender<()>) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("SlopBarTaskbarWidget");
        let wc = wm::WNDCLASSEXW {
            cbSize: std::mem::size_of::<wm::WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: std::ptr::null_mut(),
            hCursor: wm::LoadCursorW(std::ptr::null_mut(), wm::IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        wm::RegisterClassExW(&wc);
        let title = wide("SlopBar");
        let hwnd = wm::CreateWindowExW(
            wm::WS_EX_LAYERED | wm::WS_EX_TOOLWINDOW | wm::WS_EX_NOACTIVATE | wm::WS_EX_TOPMOST,
            class.as_ptr(),
            title.as_ptr(),
            wm::WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            let _ = ready.send(());
            return;
        }
        exclude_from_peek(hwnd);
        let tooltip = tooltip(hwnd, instance);
        LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            l.renderer = Renderer::new().ok();
            l.tooltip = tooltip;
            l.light = crate::dock::taskbar_is_light();
        });
        wm::SetTimer(hwnd, TIMER, 1000, None);
        HWND_.store(hwnd as isize, Ordering::Release);
        let _ = ready.send(());
        let mut msg: wm::MSG = std::mem::zeroed();
        while wm::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            wm::TranslateMessage(&msg);
            wm::DispatchMessageW(&msg);
        }
    }
}

/// Keeps the widget visible while Aero Peek previews a window from the taskbar.
unsafe fn exclude_from_peek(hwnd: HWND) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK};
    let value: i32 = 1;
    DwmSetWindowAttribute(hwnd, DWMWA_EXCLUDED_FROM_PEEK as u32, &value as *const i32 as *const _, 4);
}

unsafe fn tooltip(owner: HWND, instance: windows_sys::Win32::Foundation::HINSTANCE) -> HWND {
    let icc = cc::INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<cc::INITCOMMONCONTROLSEX>() as u32, dwICC: cc::ICC_WIN95_CLASSES };
    cc::InitCommonControlsEx(&icc);
    // Always: the widget never becomes the active window.
    let tip = wm::CreateWindowExW(
        wm::WS_EX_TOPMOST | wm::WS_EX_TOOLWINDOW,
        cc::TOOLTIPS_CLASSW,
        std::ptr::null(),
        wm::WS_POPUP | cc::TTS_ALWAYSTIP | cc::TTS_NOPREFIX,
        wm::CW_USEDEFAULT,
        wm::CW_USEDEFAULT,
        wm::CW_USEDEFAULT,
        wm::CW_USEDEFAULT,
        owner,
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );
    if !tip.is_null() {
        wm::SendMessageW(tip, cc::TTM_SETMAXTIPWIDTH, 0, 400);
        // The tooltips run to a dozen lines: leave them up long enough to read (30 s).
        wm::SendMessageW(tip, cc::TTM_SETDELAYTIME, cc::TTDT_AUTOPOP as usize, 30_000);
    }
    tip
}

/// Replaces the tooltip regions: one per ring, and the health dot.
unsafe fn set_tools(l: &mut Local, owner: HWND, tips: &[(widget::Box2, String)], dpi: u32) {
    if l.tooltip.is_null() {
        return;
    }
    let mut info: cc::TTTOOLINFOW = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<cc::TTTOOLINFOW>() as u32;
    info.hwnd = owner;
    for id in 0..l.tools {
        info.uId = id;
        wm::SendMessageW(l.tooltip, cc::TTM_DELTOOLW, 0, &info as *const _ as LPARAM);
    }
    let k = widget::scale(dpi);
    for (id, (b, text)) in tips.iter().enumerate() {
        // A tooltip breaks lines at CR LF (with a maximum width set).
        let mut text = wide(&text.replace('\n', "\r\n"));
        info.uFlags = cc::TTF_SUBCLASS;
        info.uId = id;
        info.rect = RECT {
            left: (b.x * k).floor() as i32,
            top: (b.y * k).floor() as i32,
            right: ((b.x + b.w) * k).ceil() as i32,
            bottom: ((b.y + b.h) * k).ceil() as i32,
        };
        info.lpszText = text.as_mut_ptr();
        // The control copies the text.
        wm::SendMessageW(l.tooltip, cc::TTM_ADDTOOLW, 0, &info as *const _ as LPARAM);
    }
    l.tools = tips.len();
}

/// Draws the current view where dock.rs wants it and shows it there. The right edge stays put
/// when the width changes (the widget is docked by its right edge, next to the tray).
fn redraw(hwnd: HWND) {
    let Some((view, theme, open, want)) = shared(|s| (s.view.clone(), s.theme, s.open, s.want)) else { return };
    LOCAL.with(|l| {
        let mut l = l.borrow_mut();
        let Some(renderer) = l.renderer.as_ref() else { return };
        let (dpi, h) = want.map_or((96, 48), |p| (p.dpi, p.h));
        let look = Look { theme, hot: l.hot || open, pressed: l.pressed };
        let Some((surface, layout)) = renderer.draw_within(&view, look, dpi, h, want.map(|p| p.room)) else { return };
        let width = layout.width;
        let tips = widget::tips(&view, &layout, &lang_of_app());
        let shown = want.map(|p| Place { x: p.x + p.w - surface.width, y: p.y, w: surface.width, h: surface.height, dpi, room: p.room });
        unsafe {
            let dst = shown.map(|p| POINT { x: p.x, y: p.y });
            let size = SIZE { cx: surface.width, cy: surface.height };
            let src = POINT { x: 0, y: 0 };
            let blend = gdi::BLENDFUNCTION {
                BlendOp: gdi::AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: gdi::AC_SRC_ALPHA as u8,
            };
            let screen = gdi::GetDC(std::ptr::null_mut());
            wm::UpdateLayeredWindow(
                hwnd,
                screen,
                dst.as_ref().map_or(std::ptr::null(), |p| p as *const POINT),
                &size,
                surface.dc,
                &src,
                0,
                &blend,
                wm::ULW_ALPHA,
            );
            gdi::ReleaseDC(std::ptr::null_mut(), screen);
            set_tools(&mut l, hwnd, &tips, dpi);
            let label = wide(&view.label);
            wm::SetWindowTextW(hwnd, label.as_ptr());
        }
        shared(|s| {
            s.width = width;
            if shown.is_some() {
                s.shown = shown;
            }
        });
    });
}

fn lang_of_app() -> Lang {
    APP.get().map(lang).unwrap_or(Lang("en".into()))
}

fn on_app(f: impl FnOnce(&AppHandle) + Send + 'static) {
    if let Some(app) = APP.get().cloned() {
        // Off this thread: the app may need the main thread, which may be waiting on this one.
        std::thread::spawn(move || f(&app));
    }
}

/// The right-click menu: the same items as the web widget's (refresh, settings, quit), drawn in
/// the taskbar's theme like the shell's own menus.
unsafe fn menu(hwnd: HWND) {
    let texts = lang_of_app();
    let menu = wm::CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, key) in [(MENU_REFRESH, "refresh"), (MENU_SETTINGS, "settings")] {
        let text = wide(&texts.t(key, &[]));
        wm::AppendMenuW(menu, wm::MF_STRING, id, text.as_ptr());
    }
    wm::AppendMenuW(menu, wm::MF_SEPARATOR, 0, std::ptr::null());
    let quit = wide(&texts.t("quit", &[]));
    wm::AppendMenuW(menu, wm::MF_STRING, MENU_QUIT, quit.as_ptr());
    menu_theme(crate::dock::taskbar_is_light());
    let mut at = POINT { x: 0, y: 0 };
    wm::GetCursorPos(&mut at);
    // Without being foreground, the menu would not close when clicking elsewhere.
    wm::SetForegroundWindow(hwnd);
    let chosen = wm::TrackPopupMenuEx(menu, wm::TPM_RETURNCMD | wm::TPM_NONOTIFY | wm::TPM_RIGHTBUTTON | wm::TPM_BOTTOMALIGN, at.x, at.y, hwnd, std::ptr::null());
    wm::PostMessageW(hwnd, wm::WM_NULL, 0, 0);
    wm::DestroyMenu(menu);
    let id = match chosen as usize {
        MENU_REFRESH => "refresh",
        MENU_SETTINGS => "settings",
        MENU_QUIT => "quit",
        _ => return,
    };
    on_app(move |app| crate::app::menu_event(app, id));
}

/// Dark or light Win32 menus for this process, as Explorer's follow the taskbar. These are
/// uxtheme's unnamed exports (SetPreferredAppMode = 135, FlushMenuThemes = 136) that the shell and
/// the menu library Tauri uses also rely on; missing ones (older Windows) leave the default.
unsafe fn menu_theme(light: bool) {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let lib = LoadLibraryW(wide("uxtheme.dll").as_ptr());
    if lib.is_null() {
        return;
    }
    if let Some(set) = GetProcAddress(lib, 135 as *const u8) {
        let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set);
        // 2 = force dark, 3 = force light.
        set(if light { 3 } else { 2 });
    }
    if let Some(flush) = GetProcAddress(lib, 136 as *const u8) {
        let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
        flush();
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_VIEW | WM_PLACE | WM_REDRAW => {
            redraw(hwnd);
            0
        }
        wm::WM_MOUSEACTIVATE => wm::MA_NOACTIVATE as LRESULT,
        wm::WM_MOUSEMOVE => {
            let start = LOCAL.with(|l| !std::mem::replace(&mut l.borrow_mut().hot, true));
            if start {
                let mut t = km::TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<km::TRACKMOUSEEVENT>() as u32,
                    dwFlags: km::TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                km::TrackMouseEvent(&mut t);
                redraw(hwnd);
            }
            0
        }
        cc::WM_MOUSELEAVE => {
            let tip = LOCAL.with(|l| {
                let mut l = l.borrow_mut();
                l.hot = false;
                l.pressed = false;
                l.tooltip
            });
            // A click turned the tooltips off; the next hover may show them again.
            if !tip.is_null() {
                wm::SendMessageW(tip, cc::TTM_ACTIVATE, 1, 0);
            }
            redraw(hwnd);
            0
        }
        wm::WM_LBUTTONDOWN => {
            let tip = LOCAL.with(|l| {
                let mut l = l.borrow_mut();
                l.pressed = true;
                l.tooltip
            });
            // Clicking opens the flyout: the tooltip goes at once and stays away until the
            // pointer leaves the widget, so it never sits over the flyout.
            if !tip.is_null() {
                wm::SendMessageW(tip, cc::TTM_POP, 0, 0);
                wm::SendMessageW(tip, cc::TTM_ACTIVATE, 0, 0);
            }
            km::SetCapture(hwnd);
            redraw(hwnd);
            0
        }
        wm::WM_LBUTTONUP => {
            km::ReleaseCapture();
            let (x, y) = ((lparam & 0xffff) as i16 as i32, ((lparam >> 16) & 0xffff) as i16 as i32);
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            wm::GetClientRect(hwnd, &mut r);
            let inside = x >= r.left && x < r.right && y >= r.top && y < r.bottom;
            let was = LOCAL.with(|l| std::mem::replace(&mut l.borrow_mut().pressed, false));
            redraw(hwnd);
            if was && inside {
                on_app(|app| crate::app::dock_clicked(app.clone()));
            }
            0
        }
        wm::WM_RBUTTONUP => {
            menu(hwnd);
            0
        }
        wm::WM_TIMER if wparam == TIMER => {
            // The taskbar's theme, and a notice that has run its time.
            let light = crate::dock::taskbar_is_light();
            let flipped = LOCAL.with(|l| std::mem::replace(&mut l.borrow_mut().light, light) != light);
            let expired = shared(|s| s.toast.as_ref().is_some_and(|(_, at)| at.elapsed() >= TOAST)).unwrap_or(false);
            if flipped || expired {
                on_app(refresh);
            }
            0
        }
        _ => wm::DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
