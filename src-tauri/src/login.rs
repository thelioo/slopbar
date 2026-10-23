//! Isolated CLI process environment and recovery of abandoned sign-in directories.
use crate::{accounts::Provider, sources::Source};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, SystemTime};

#[derive(serde::Serialize, serde::Deserialize)]
struct Owner {
    provider: Provider,
    imported: bool,
}

pub type ProtectedTokens = std::collections::HashSet<(Provider, String)>;

pub fn action(provider: Provider, logout: bool) -> &'static str {
    match (provider, logout) {
        // Only the user settings in our empty, isolated config may load. In
        // particular, do not load project settings from ancestors of the cwd.
        (Provider::Claude, false) => "claude --setting-sources user auth login",
        (Provider::Claude, true) => "claude --setting-sources user auth logout",
        (Provider::Codex, false) => "codex login",
        (Provider::Codex, true) => "codex logout",
    }
}

pub fn blocked_environment(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    ["CLAUDE_", "ANTHROPIC_", "CODEX_", "OPENAI_"]
        .iter()
        .any(|p| name.starts_with(p))
        || matches!(
            name.as_str(),
            "CLAUDECODE" | "USE_LOCAL_OAUTH" | "USE_STAGING_OAUTH"
        )
}

pub fn shell_script(provider: Provider, native: &str, action: &str, visible: bool) -> String {
    let native = native.replace('\'', "'\\''");
    // Expand this allowlist AFTER the login profile runs. env -i also drops exported
    // functions, BASH_ENV, provider overrides, and settings re-exported by that profile.
    let keep = [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "TERM",
        "LANG",
        "LC_ALL",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "WSL_INTEROP",
        "WSL_DISTRO_NAME",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
    ];
    let env = keep
        .iter()
        .map(|n| format!("{n}=\"${{{n}-}}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let config = match provider {
        Provider::Claude => {
            format!("CLAUDE_CONFIG_DIR='{native}' CLAUDE_SECURESTORAGE_CONFIG_DIR='{native}'")
        }
        Provider::Codex => format!("CODEX_HOME='{native}'"),
    };
    let pause = if visible {
        "if [ \"$slopbar_login_status\" -ne 0 ]; then read -r -p 'Sign-in failed. Press Enter to close.'; fi; "
    } else {
        ""
    };
    format!("chmod 700 -- '{native}' && cd -- '{native}' && env -i {env} {config} {action}; slopbar_login_status=$?; {pause}exit \"$slopbar_login_status\"")
}

pub fn command(
    provider: Provider,
    source: &Source,
    dir: &Path,
    native: &str,
    action: &str,
    visible: bool,
) -> Command {
    #[allow(unused_mut)]
    let mut command = match &source.distro {
        Some(distro) => {
            let mut c = crate::sources::command("wsl.exe");
            c.args([
                "-d",
                distro,
                "--",
                "bash",
                "-lic",
                &shell_script(provider, native, action, visible),
            ]);
            c
        }
        None => {
            let mut c = crate::sources::command("cmd.exe");
            // /d disables cmd AutoRun, which can restore authentication overrides.
            c.arg("/d").arg("/c");
            if visible {
                // The cwd is set by CreateProcess, not parsed by cmd. Keeping the
                // script argument constant also protects profile paths containing
                // cmd metacharacters or paired %environment-variable% text. The explicit
                // `.\` matters: with NoDefaultCurrentDirectoryInExePath set (Claude Code sets
                // it, and SlopBar inherits it when an agent starts it), cmd does not look for a
                // bare `login.cmd` in the current directory and the sign-in never starts.
                c.arg(r".\login.cmd");
            } else {
                c.arg(action);
            }
            for (name, _) in std::env::vars_os() {
                if blocked_environment(&name.to_string_lossy()) {
                    c.env_remove(name);
                }
            }
            match provider {
                Provider::Claude => {
                    c.env("CLAUDE_CONFIG_DIR", dir)
                        .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", dir);
                }
                Provider::Codex => {
                    c.env("CODEX_HOME", dir);
                }
            }
            c.current_dir(dir);
            c
        }
    };
    if visible {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
        }
    }
    command
}

pub fn prepare(dir: &Path, provider: Provider, action: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    write_owner(dir, provider, false)?;
    // `call` supports CLIs installed as .cmd launchers. Keep the original exit code
    // after pause so a failed login can never be imported as a successful one.
    std::fs::write(dir.join("login.cmd"), format!("@echo off\r\ncall {action}\r\nset \"slopbar_login_status=%errorlevel%\"\r\nif not \"%slopbar_login_status%\"==\"0\" pause\r\nexit /b %slopbar_login_status%\r\n"))
}

fn write_owner(dir: &Path, provider: Provider, imported: bool) -> std::io::Result<()> {
    crate::private_store::write(
        &dir.join("slopbar-owner.json"),
        &serde_json::to_vec(&Owner { provider, imported })?,
    )
}

/// Persist the handoff BEFORE returning credentials to any caller. If this fails,
/// the login must not be exported. A crash or failed delete after handoff can then
/// never make startup cleanup revoke the same login now held by a CLI or monitor.
pub fn handoff(dir: &Path, provider: Provider) -> std::io::Result<()> {
    write_owner(dir, provider, true)?;
    let file = dir.join(match provider {
        Provider::Claude => ".credentials.json",
        Provider::Codex => "auth.json",
    });
    // Deletion is best effort: the durable imported marker is the revocation guard.
    for _ in 0..3 {
        match std::fs::remove_file(&file) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Ok(())
}

/// A hidden logout is needed if the CLI wrote to its isolated OS credential store.
/// Always use the same sanitized environment and config directory as the login.
pub fn logout(provider: Provider, source: &Source, dir: &Path, native: &str) {
    let Ok((mut child, _tree)) = ProcessTree::spawn(
        command(provider, source, dir, native, action(provider, true), false)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
        false,
    ) else {
        return;
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() < Duration::from_secs(6) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

impl ProcessTree {
    pub fn spawn(command: &mut Command, visible: bool) -> std::io::Result<(Child, Self)> {
        Self::spawn_with(command, visible, false)
    }

    /// `breakaway`: a process in the tree may leave it by asking to (`CREATE_BREAKAWAY_FROM_JOB`).
    /// Only SlopBar's own sign-in page launcher asks (see `signin::open`), so the browser window
    /// it opens outlives the sign-in; the CLIs never ask, so they still die with the tree.
    pub fn spawn_with(command: &mut Command, visible: bool, breakaway: bool) -> std::io::Result<(Child, Self)> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_SUSPENDED,
            };
            command.creation_flags(
                CREATE_SUSPENDED
                    | if visible {
                        CREATE_NEW_CONSOLE
                    } else {
                        CREATE_NO_WINDOW
                    },
            );
        }
        #[cfg(not(windows))]
        let _ = visible;
        let mut child = command.spawn()?;
        let result = Self::attach_with(&child, breakaway);
        #[cfg(windows)]
        let result = result.and_then(|tree| {
            resume(&child)?;
            Ok(tree)
        });
        match result {
            Ok(tree) => Ok((child, tree)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
}

/// std::process::Child closes the primary thread handle. Recover it while the
/// process is still suspended, assign the job first, then allow any user code to run.
#[cfg(windows)]
fn resume(child: &Child) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut found = Thread32First(snapshot, &mut entry) != 0;
        let result = loop {
            if !found {
                break Err(std::io::Error::other("suspended login thread not found"));
            }
            if entry.th32OwnerProcessID == child.id() {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if thread.is_null() {
                    break Err(std::io::Error::last_os_error());
                }
                let previous = ResumeThread(thread);
                let result = if previous == u32::MAX {
                    Err(std::io::Error::last_os_error())
                } else if previous != 1 {
                    Err(std::io::Error::other("unexpected login suspension count"))
                } else {
                    Ok(())
                };
                CloseHandle(thread);
                break result;
            }
            found = Thread32Next(snapshot, &mut entry) != 0;
        };
        CloseHandle(snapshot);
        result
    }
}

fn abandoned(name: &str, now: SystemTime) -> bool {
    let Some(rest) = name.strip_prefix(".slopbar-login-") else {
        return false;
    };
    let fields: Vec<_> = rest.split('-').collect();
    if fields.len() != 3
        || fields
            .iter()
            .any(|v| v.is_empty() || !v.bytes().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    let Ok(millis) = fields[1].parse::<u64>() else {
        return false;
    };
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(millis))
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age >= Duration::from_secs(15 * 60))
}

pub fn cleanup(sources: &[Source], protected: Option<&ProtectedTokens>) {
    cleanup_with(sources, protected, logout);
}

fn cleanup_with(
    sources: &[Source],
    protected: Option<&ProtectedTokens>,
    mut logout: impl FnMut(Provider, &Source, &Path, &str),
) {
    for source in sources {
        let Ok(entries) = std::fs::read_dir(&source.home) else {
            continue;
        };
        for entry in entries.flatten() {
            if !abandoned(&entry.file_name().to_string_lossy(), SystemTime::now()) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_dir() || meta.file_type().is_symlink() {
                continue;
            }
            let marker = path.join("slopbar-owner.json");
            // Only our marked directories are eligible. Never follow a marker symlink.
            let owner = if std::fs::symlink_metadata(&marker)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            {
                std::fs::read(&marker).ok().and_then(|b| {
                    serde_json::from_slice::<Owner>(&b).ok().or_else(|| {
                        // Old markers cannot prove that no handoff occurred.
                        serde_json::from_slice::<Provider>(&b)
                            .ok()
                            .map(|provider| Owner {
                                provider,
                                imported: true,
                            })
                    })
                })
            } else if !marker.exists() {
                // Recover folders from the first version, which had no ownership marker.
                // Require the exact generated name AND a regular CLI credentials file.
                [
                    (".credentials.json", "claudeAiOauth", Provider::Claude),
                    ("auth.json", "tokens", Provider::Codex),
                ]
                .into_iter()
                .find_map(|(file, field, provider)| {
                    let file = path.join(file);
                    if !std::fs::symlink_metadata(&file)
                        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                    {
                        return None;
                    }
                    let data: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
                    data[field].is_object().then_some(Owner {
                        provider,
                        imported: true,
                    })
                })
            } else {
                None
            };
            if let Some(owner) = owner {
                let provider = owner.provider;
                let native = format!(
                    "{}/{}",
                    source.native_home,
                    entry.file_name().to_string_lossy()
                );
                // Fail closed for old/exported folders or an unreadable protected
                // store. Also defend against a pending folder holding a known token.
                let credential = path.join(match provider {
                    Provider::Claude => ".credentials.json",
                    Provider::Codex => "auth.json",
                });
                let safe = protected.is_some_and(|tokens| {
                    if !credential.exists() {
                        return std::fs::symlink_metadata(&credential)
                            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
                    }
                    if !std::fs::symlink_metadata(&credential)
                        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                    {
                        return false;
                    }
                    let Ok(bytes) = std::fs::read(&credential) else {
                        return false;
                    };
                    let Ok(data) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                        return false;
                    };
                    let refresh = match provider {
                        Provider::Claude => data["claudeAiOauth"]["refreshToken"].as_str(),
                        Provider::Codex => data["tokens"]["refresh_token"].as_str(),
                    };
                    refresh.is_some_and(|t| !tokens.contains(&(provider, t.to_owned())))
                });
                if !owner.imported && safe {
                    logout(provider, source, &path, &native);
                }
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}

// Closing the Windows process closes this handle and kills the entire sign-in
// process tree, even on an updater restart or an abrupt app exit.
#[cfg(windows)]
pub struct ProcessTree(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
impl ProcessTree {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn attach(child: &Child) -> std::io::Result<Self> {
        Self::attach_with(child, false)
    }

    fn attach_with(child: &Child, breakaway: bool) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        let tree = Self(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
        if tree.0.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | if breakaway { JOB_OBJECT_LIMIT_BREAKAWAY_OK } else { 0 };
        let ok = unsafe {
            SetInformationJobObject(
                tree.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            ) != 0
                && AssignProcessToJobObject(tree.0, child.as_raw_handle() as _) != 0
        };
        if !ok {
            return Err(std::io::Error::last_os_error());
        }
        Ok(tree)
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(not(windows))]
pub struct ProcessTree;
#[cfg(not(windows))]
impl Drop for ProcessTree {
    fn drop(&mut self) {}
}
#[cfg(not(windows))]
impl ProcessTree {
    pub fn attach(_: &Child) -> std::io::Result<Self> {
        Ok(Self)
    }

    fn attach_with(child: &Child, _: bool) -> std::io::Result<Self> {
        Self::attach(child)
    }
}

#[cfg(test)]
mod tests;
