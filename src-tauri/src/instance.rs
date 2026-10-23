//! One SlopBar per signed-in user, enforced before Tauri starts.
//!
//! `tauri_plugin_single_instance` is not enough on its own: when its named mutex already exists
//! but the first instance's message window does not (yet, or any more), it carries on starting a
//! second app instead of exiting. Two restarts waiting on the same old instance (`--after <pid>`)
//! both pass that check the moment the old one exits, and two instances then renew the same
//! monitoring sessions. So startup takes its own lock and waits for it, and also refuses to run
//! beside another instance that answers on the agent door (an older build that never took it).

use std::time::Duration;

use crate::door;

/// How this process was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// By the user, autostart or an installer.
    Normal,
    /// By `restart_app`: `--after <pid>` of the instance being replaced.
    Restart { after: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Run,
    /// Another instance is running: ask it to show Settings, then exit.
    HandOver,
    /// Another restart already took over: exit quietly.
    Exit,
}

/// Startup's choice, from whether this process holds the instance lock and whether another
/// instance answers on the door.
pub fn decide(launch: Launch, locked: bool, door_live: bool) -> Decision {
    if locked && !door_live {
        return Decision::Run;
    }
    match launch {
        Launch::Normal => Decision::HandOver,
        // A racing restart of the same instance won: it runs the same installed executable.
        Launch::Restart { .. } => Decision::Exit,
    }
}

/// How long startup waits for the lock: briefly for a launch (an instance that is quitting lets
/// go within a second), longer after a restart (the instance replaced exits with its journal).
pub fn lock_wait(launch: Launch) -> Duration {
    match launch {
        Launch::Normal => Duration::from_secs(3),
        Launch::Restart { .. } => Duration::from_secs(15),
    }
}

/// Whether a door probe found another instance. Only "nobody serves the pipe" (or a pipe served
/// by another user, which the door ignores too) is no instance: a busy or slow door is one.
pub fn door_live(probe: &Result<serde_json::Value, String>) -> bool {
    !matches!(probe, Err(e) if e == "not_running" || e == "door_untrusted" || e == "unsupported_platform")
}

const PROBE: Duration = Duration::from_secs(3);

/// The instance lock's name: per user and app data folder, like the door's pipe.
fn lock_name() -> String {
    let pipe = door::pipe_name();
    format!(r"Local\slopbar-instance-{}", pipe.rsplit('-').next().unwrap_or_default())
}

/// Takes this user's instance lock for the life of the process, or hands over to (or leaves to)
/// the instance already running. Returns whether this process should start the app.
pub fn claim(after: Option<u32>) -> bool {
    let launch = after.map_or(Launch::Normal, |after| Launch::Restart { after });
    if let Launch::Restart { after } = launch {
        // Let the previous instance finish its orderly exit first.
        door::wait_for_exit(after, Duration::from_secs(60));
    }
    let locked = imp::lock(&lock_name(), lock_wait(launch));
    let pipe = door::pipe_name();
    let live = door_live(&door::call_within(&pipe, &door::Request::GetAppInfo, PROBE));
    match decide(launch, locked, live) {
        Decision::Run => true,
        Decision::HandOver => {
            let _ = door::call_within(&pipe, &door::Request::OpenSettings, PROBE);
            false
        }
        Decision::Exit => false,
    }
}

/// SlopBar's processes in this Windows session: the app and agents' MCP servers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Processes {
    pub instances: usize,
    pub mcp_servers: usize,
}

pub const APP_EXE: &str = if cfg!(windows) { "slopbar.exe" } else { "slopbar" };
pub const MCP_EXE: &str = if cfg!(windows) { "slopbar-mcp.exe" } else { "slopbar-mcp" };

/// Counts `(executable file name, session)` pairs in `session`. By name, not path: an install
/// moves the running executable into a rollback folder, and a duplicate may run from there.
pub fn count(list: &[(String, u32)], session: u32) -> Processes {
    let mut found = Processes::default();
    for (name, _) in list.iter().filter(|(_, s)| *s == session) {
        if name.eq_ignore_ascii_case(APP_EXE) {
            found.instances += 1;
        } else if name.eq_ignore_ascii_case(MCP_EXE) {
            found.mcp_servers += 1;
        }
    }
    found
}

/// This session's SlopBar processes, if they can be listed.
pub fn processes() -> Option<Processes> {
    let (list, session) = imp::list()?;
    let named: Vec<_> = list.iter().map(|p| (p.name.clone(), p.session)).collect();
    Some(count(&named, session))
}

/// Ends every other SlopBar app process in this session (duplicates of an older build, which
/// ignore the instance lock) that have run for at least `DUPLICATE_AGE`, so a moment-long
/// `--build-info-to` or a restart still deciding is left alone. Returns how many were ended.
pub fn end_duplicates() -> usize {
    let Some((list, session)) = imp::list() else { return 0 };
    let me = std::process::id();
    list.iter()
        .filter(|p| p.session == session && p.pid != me && p.name.eq_ignore_ascii_case(APP_EXE))
        .filter(|p| imp::age(p.pid).is_some_and(|age| age >= DUPLICATE_AGE))
        .filter(|p| imp::terminate(p.pid))
        .count()
}

const DUPLICATE_AGE: Duration = Duration::from_secs(90);

/// The process that started `pid`, while it is still that process (started before `pid`, so its
/// id was not reused by a later one).
pub fn parent_of(pid: u32) -> Option<u32> {
    imp::parent_of(pid)
}

/// Blocks until process `pid` exits (immediately if it is gone).
pub fn wait_gone(pid: u32) {
    imp::wait_gone(pid)
}

#[derive(Debug, Clone)]
struct Entry {
    pid: u32,
    parent: u32,
    session: u32,
    name: String,
}

#[cfg(windows)]
mod imp {
    use super::Entry;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Waits up to `limit` to own the named mutex; the handle is kept until the process exits
    /// (ownership belongs to this thread, the one that runs the app). An abandoned mutex (its
    /// owner died) is owned too.
    pub fn lock(name: &str, limit: Duration) -> bool {
        let name = wide(name);
        // SAFETY: `name` is NUL-terminated; the handle is deliberately never closed.
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if mutex.is_null() {
                return false;
            }
            let waited = WaitForSingleObject(mutex, limit.as_millis().min(u32::MAX as u128 - 1) as u32);
            waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED
        }
    }

    pub(super) fn list() -> Option<(Vec<Entry>, u32)> {
        let mut session = 0;
        // SAFETY: plain out-parameter; the snapshot handle is closed below.
        unsafe {
            if ProcessIdToSessionId(std::process::id(), &mut session) == 0 {
                return None;
            }
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = vec![];
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                let mut s = u32::MAX;
                ProcessIdToSessionId(entry.th32ProcessID, &mut s);
                found.push(Entry { pid: entry.th32ProcessID, parent: entry.th32ParentProcessID, session: s,
                    name: String::from_utf16_lossy(&entry.szExeFile[..len]) });
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            Some((found, session))
        }
    }

    pub fn terminate(pid: u32) -> bool {
        // SAFETY: the handle, if any, is used once and closed.
        unsafe {
            let process = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return false;
            }
            let ended = TerminateProcess(process, 1) != 0;
            if ended {
                WaitForSingleObject(process, 5000);
            }
            CloseHandle(process);
            ended
        }
    }

    fn created(process: HANDLE) -> Option<u64> {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        // SAFETY: out-parameters only, for a handle the caller owns.
        (unsafe { GetProcessTimes(process, &mut c, &mut e, &mut k, &mut u) } != 0)
            .then(|| (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime))
    }

    fn started(pid: u32) -> Option<u64> {
        // SAFETY: the handle, if any, is used once and closed.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let at = created(process);
            CloseHandle(process);
            at
        }
    }

    /// How long process `pid` has run.
    pub fn age(pid: u32) -> Option<Duration> {
        // FILETIME counts 100 ns intervals from 1601; the Unix epoch is 11644473600 s later.
        let since_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
        let now = (since_unix.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(since_unix.subsec_nanos() / 100);
        Some(Duration::from_nanos(now.checked_sub(started(pid)?)? * 100))
    }

    pub fn parent_of(pid: u32) -> Option<u32> {
        let (list, _) = list()?;
        let parent = list.iter().find(|p| p.pid == pid)?.parent;
        // A parent that exited may have had its id reused by a later process.
        (parent != 0 && started(parent)? <= started(pid)?).then_some(parent)
    }

    pub fn wait_gone(pid: u32) {
        // SAFETY: the handle, if any, is waited on and closed once.
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return;
            }
            WaitForSingleObject(process, INFINITE);
            CloseHandle(process);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Entry;
    use std::time::Duration;

    pub fn lock(_name: &str, _limit: Duration) -> bool {
        true
    }
    pub(super) fn list() -> Option<(Vec<Entry>, u32)> {
        None
    }
    pub fn terminate(_pid: u32) -> bool {
        false
    }
    pub fn age(_pid: u32) -> Option<Duration> {
        None
    }
    pub fn parent_of(_pid: u32) -> Option<u32> {
        None
    }
    pub fn wait_gone(_pid: u32) {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_lone_lock_holder_runs() {
        let after = Launch::Restart { after: 4242 };
        assert_eq!(decide(Launch::Normal, true, false), Decision::Run);
        assert_eq!(decide(after, true, false), Decision::Run);
        // Someone else holds the lock: a launch hands over, a losing restart just leaves.
        assert_eq!(decide(Launch::Normal, false, false), Decision::HandOver);
        assert_eq!(decide(after, false, false), Decision::Exit);
        // An older build that never takes the lock still answers on the door.
        assert_eq!(decide(Launch::Normal, true, true), Decision::HandOver);
        assert_eq!(decide(after, true, true), Decision::Exit);
        assert_eq!(decide(after, false, true), Decision::Exit);
    }

    #[test]
    fn restarts_wait_longer_for_the_lock() {
        assert!(lock_wait(Launch::Restart { after: 1 }) > lock_wait(Launch::Normal));
    }

    #[test]
    fn only_an_empty_pipe_means_no_other_instance() {
        assert!(!door_live(&Err("not_running".into())));
        assert!(!door_live(&Err("door_untrusted".into())));
        assert!(door_live(&Ok(serde_json::json!({ "version": "1" }))));
        for busy in ["timeout", "door_unavailable", "bad_door_reply"] {
            assert!(door_live(&Err(busy.into())), "{busy}");
        }
    }

    #[test]
    fn counts_this_sessions_app_and_servers_by_name() {
        let list: Vec<(String, u32)> = [("slopbar.exe", 1), ("SlopBar.exe", 1), ("slopbar-mcp.exe", 1), ("slopbar-mcp.exe", 1),
            ("slopbar-mcp.exe", 1), ("slopbar.exe", 2), ("slopbar-mcp.exe", 2), ("explorer.exe", 1)]
            .iter().map(|(n, s)| (n.to_string(), *s)).collect();
        if cfg!(windows) {
            assert_eq!(count(&list, 1), Processes { instances: 2, mcp_servers: 3 });
            assert_eq!(count(&list, 2), Processes { instances: 1, mcp_servers: 1 });
        }
        assert_eq!(count(&list, 9), Processes::default());
    }

    #[cfg(windows)]
    #[test]
    fn a_second_holder_cannot_take_the_lock_until_the_first_ends() {
        let name = format!(r"Local\slopbar-test-instance-{}-{}", std::process::id(), line!());
        let (taken, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
        let first = {
            let name = name.clone();
            std::thread::spawn(move || {
                taken.0.send(imp::lock(&name, Duration::ZERO)).unwrap();
                release.1.recv().unwrap();
                // The thread ends owning it: the mutex is abandoned, which the next waiter owns.
            })
        };
        assert!(taken.1.recv().unwrap());
        assert!(!imp::lock(&name, Duration::from_millis(50)));
        release.0.send(()).unwrap();
        first.join().unwrap();
        assert!(imp::lock(&name, Duration::from_secs(5)));
    }

    #[cfg(windows)]
    #[test]
    fn this_process_has_a_live_parent_and_is_listed() {
        assert!(parent_of(std::process::id()).is_some());
        assert!(processes().is_some());
        assert!(imp::age(std::process::id()).is_some_and(|a| a < Duration::from_secs(3600)));
    }
}
