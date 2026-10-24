//! Windows notifications (toasts). What to say and when is decided by the frontend, which owns
//! the translations (see src/alerts.js); this only shows them under SlopBar's own name and icon.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// The same news for the phone (see `phone`), worded without account names; sent instead of the
/// toast's text unless the user chose to include names.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PhoneForward {
    pub kind: crate::phone::Kind,
    pub title: String,
    pub body: String,
}

/// Sends a notification to the phone when phone alerts are on and its kind is selected. In the
/// background: a slow or unreachable push server never delays the toast.
pub fn forward(app: &AppHandle, title: &str, body: &str, phone: Option<PhoneForward>) {
    let Some(phone) = phone.filter(|p| p.kind != crate::phone::Kind::Test) else { return };
    let settings = app.state::<crate::AppState>().settings.lock().unwrap().phone.clone();
    let Some(message) = crate::phone::compose(&settings, phone.kind, (title, body), (&phone.title, &phone.body)) else { return };
    let Some(url) = settings.url else { return };
    tauri::async_runtime::spawn(async move {
        let token = crate::settings::app_dir().and_then(|d| crate::phone::load_token(&d));
        let _ = crate::phone::send(&crate::phone::client(), &url, token.as_deref(), &message).await;
    });
}

/// Shows a notification. With `page` (a Settings page, e.g. "subscriptions"), clicking it opens
/// Settings on that page.
#[tauri::command]
pub fn notify(app: AppHandle, title: String, body: String, page: Option<String>, phone: Option<PhoneForward>) -> Result<(), String> {
    forward(&app, &title, &body, phone);
    #[cfg(windows)]
    if let Some(page) = page.filter(|p| crate::door::SETTINGS_PAGES.contains(&p.as_str())) {
        return clickable(app, &title, &body, page);
    }
    app.notification().builder().title(title).body(body).show().map_err(|e| e.to_string())
}

/// The unexpected-use notice (see `unexpected`), with two buttons: review the account in
/// Settings > Accounts, or open the provider's own security page. Clicking the toast itself
/// opens Settings too. Signing anything out is left to the user on that page.
#[tauri::command]
pub fn notify_unexpected(app: AppHandle, provider: crate::accounts::Provider, title: String, body: String, review: String, security: String, phone: Option<PhoneForward>) -> Result<(), String> {
    forward(&app, &title, &body, phone);
    #[cfg(windows)]
    {
        let identifier = app.config().identifier.clone();
        let url = crate::unexpected::security_page(provider);
        tauri_winrt_notification::Toast::new(&identifier)
            .title(&title)
            .text1(&body)
            .add_button(&review, "review")
            .add_button(&security, "security")
            .on_activated(move |action| {
                if action.as_deref() == Some("security") {
                    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
                } else {
                    crate::app::show_settings_page(&app, Some("accounts".into()));
                }
                Ok(())
            })
            .show()
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (provider, review, security);
        notify(app, title, body, Some("accounts".into()), None)
    }
}

/// A notification with buttons (label, action): `on` gets the button's action, or None for a
/// click on the notification itself.
pub fn notify_choice(app: AppHandle, title: String, body: String, buttons: Vec<(String, &'static str)>, on: impl Fn(Option<&str>) + Send + Sync + 'static) -> Result<(), String> {
    #[cfg(windows)]
    {
        let identifier = app.config().identifier.clone();
        let mut toast = tauri_winrt_notification::Toast::new(&identifier).title(&title).text1(&body);
        for (label, action) in &buttons {
            toast = toast.add_button(label, action);
        }
        toast.on_activated(move |action| {
            on(action.as_deref().filter(|a| !a.is_empty()));
            Ok(())
        }).show().map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (buttons, on);
        notify(app, title, body, Some("accounts".into()), None)
    }
}

/// The notification plugin cannot react to a click on Windows, so this one is shown directly,
/// under the same app id. The click reaches SlopBar while it runs, which it always does.
#[cfg(windows)]
fn clickable(app: AppHandle, title: &str, body: &str, page: String) -> Result<(), String> {
    let id = app.config().identifier.clone();
    let handle = app.clone();
    tauri_winrt_notification::Toast::new(&id)
        .title(title)
        .text1(body)
        .on_activated(move |_| {
            crate::app::show_settings_page(&handle, Some(page.clone()));
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())
}

/// Toasts are attributed to an app user model id (the plugin uses the bundle identifier). The
/// installer's Start menu shortcut declares it; registering it here too gives toasts SlopBar's
/// name and icon whatever way the app was started. Returns whether every value was written.
#[cfg(windows)]
pub fn register_app_id(identifier: &str) -> bool {
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let Some(dir) = crate::settings::app_dir() else { return false };
    let icon = dir.join("icon.png");
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&icon, include_bytes!("../icons/128x128@2x.png")).is_err() {
        return false;
    }
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let key = wide(&format!(r"Software\Classes\AppUserModelId\{identifier}"));
    let mut ok = true;
    for (name, value) in [("DisplayName", "SlopBar".to_string()), ("IconUri", icon.display().to_string())] {
        let data = wide(&value);
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                wide(name).as_ptr(),
                REG_SZ,
                data.as_ptr() as *const _,
                (data.len() * 2) as u32,
            )
        };
        ok &= status == 0;
    }
    ok
}

/// Whether Windows lets SlopBar show toasts, and whether it lets any app: the switches in Windows
/// Settings > System > Notifications. `None` when Windows never stored a choice (the default is on).
#[cfg(windows)]
pub fn toast_switches(identifier: &str) -> (Option<bool>, Option<bool>) {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let read = |key: &str, value: &str| -> Option<bool> {
        let (key, value) = (wide(key), wide(value));
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the buffers outlive the call and `size` matches `data`.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut data as *mut u32 as *mut _, &mut size)
        };
        (status == 0).then_some(data != 0)
    };
    let app = read(&format!(r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{identifier}"), "Enabled");
    let system = read(r"Software\Microsoft\Windows\CurrentVersion\PushNotifications", "ToastEnabled");
    (app, system)
}
