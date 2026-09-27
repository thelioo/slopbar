//! One-time move from releases published as Usage Bar: settings and saved accounts come along,
//! and once SlopBar is installed, the old install and its startup entry are removed.

use std::path::Path;

/// Copies the old data folder (never moves it, so a local test build can't break the installed
/// app), then retires the old install. Must run before settings are loaded.
pub fn run() {
    if let (Some(old), Some(new)) = (crate::settings::legacy_app_dir(), crate::settings::app_dir()) {
        if old.is_dir() && !new.exists() {
            let _ = copy_dir(&old, &new);
        }
    }
    #[cfg(windows)]
    retire_legacy_install();
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Only an installed SlopBar replaces Usage Bar: removes its launch-at-login entry (ours is
/// written by the autostart plugin from the same setting) and runs its uninstaller silently.
/// Installed as an update of Usage Bar, the installer skips the Start menu shortcut (and Usage
/// Bar's own goes with its uninstaller), so SlopBar adds one when it's missing.
#[cfg(windows)]
fn retire_legacy_install() {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER};
    const LEGACY_NAME: &str = "Usage Bar";
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Some(local) = dirs::data_local_dir() else { return };
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.starts_with(local.join("SlopBar"))))
        .unwrap_or(false);
    if !installed {
        return;
    }
    if let (Some(programs), Ok(exe)) = (dirs::data_dir(), std::env::current_exe()) {
        let link = programs.join(r"Microsoft\Windows\Start Menu\Programs\SlopBar.lnk");
        if !link.exists() {
            let script = format!(
                "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath = '{}'; $s.Save()",
                link.display().to_string().replace('\'', "''"),
                exe.display().to_string().replace('\'', "''"),
            );
            let _ = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
    }

    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let run = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    unsafe {
        RegDeleteKeyValueW(HKEY_CURRENT_USER, run.as_ptr(), wide(LEGACY_NAME).as_ptr());
    }
    let uninstaller = local.join(LEGACY_NAME).join("uninstall.exe");
    if uninstaller.is_file() {
        let _ = std::process::Command::new(uninstaller)
            .arg("/S")
            .creation_flags(DETACHED_PROCESS)
            .spawn();
    }
}
