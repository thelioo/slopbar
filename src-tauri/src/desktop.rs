//! Whether someone is at this Windows user's desktop: sign-ins open browser windows there, and an
//! agent reaching SlopBar over SSH must hear plainly when nobody can see them.
//!
//! The running app asks about its own session (`of_process`); a door client that finds no app
//! running asks about every session of its user (`of_user`), which is how an SSH session (itself
//! a non-interactive session) learns whether the user is signed in at the screen at all.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Shown on a screen (local or remote desktop).
    Active,
    /// Signed in but not shown anywhere (a closed remote desktop, or another user switched in).
    Disconnected,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Desktop {
    pub session: u32,
    pub state: State,
    /// None when Windows does not say.
    pub locked: Option<bool>,
}

impl Desktop {
    /// Why sign-in windows opened in this session would go unseen, as an error code.
    pub fn problem(&self) -> Option<&'static str> {
        match (self.state, self.locked) {
            (State::Active, Some(true)) => Some("desktop_locked"),
            (State::Active, _) => None,
            _ => Some("desktop_not_shown"),
        }
    }
}

/// The one of `sessions` someone can work in: an unlocked active one, else any active one, else
/// any. None when the user is not signed in at all.
pub fn best(sessions: &[Desktop]) -> Option<Desktop> {
    let rank = |d: &Desktop| match d.problem() { None => 0, Some("desktop_locked") => 1, _ => 2 };
    sessions.iter().copied().min_by_key(rank)
}

#[cfg(windows)]
mod imp {
    use super::{Desktop, State};
    use windows_sys::Win32::System::RemoteDesktop::{
        ProcessIdToSessionId, WTSActive, WTSDisconnected, WTSEnumerateSessionsW, WTSFreeMemory, WTSQuerySessionInformationW,
        WTSSessionInfoEx, WTSINFOEXW, WTS_CURRENT_SERVER_HANDLE, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTS_SESSION_INFOW,
    };

    fn text(chars: &[u16]) -> String {
        let n = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
        String::from_utf16_lossy(&chars[..n])
    }

    /// The session's state and lock, and who is signed in to it (user, domain).
    fn query(session: u32) -> Option<(Desktop, String, String)> {
        let mut buf = std::ptr::null_mut();
        let mut len = 0u32;
        // SAFETY: Windows allocates `buf` (freed below once read); the level-1 layout is the only
        // one WTSSessionInfoEx returns.
        unsafe {
            if WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, WTSSessionInfoEx, &mut buf, &mut len) == 0 || buf.is_null() {
                return None;
            }
            let info = &*(buf as *const WTSINFOEXW);
            let out = (info.Level == 1).then(|| {
                let l1 = &info.Data.WTSInfoExLevel1;
                let state = match l1.SessionState {
                    s if s == WTSActive => State::Active,
                    s if s == WTSDisconnected => State::Disconnected,
                    _ => State::Other,
                };
                let locked = match l1.SessionFlags as u32 {
                    WTS_SESSIONSTATE_LOCK => Some(true),
                    WTS_SESSIONSTATE_UNLOCK => Some(false),
                    _ => None,
                };
                (Desktop { session, state, locked }, text(&l1.UserName), text(&l1.DomainName))
            });
            WTSFreeMemory(buf.cast());
            out
        }
    }

    pub fn of_process() -> Option<Desktop> {
        let mut session = 0u32;
        // SAFETY: plain out-parameter.
        if unsafe { ProcessIdToSessionId(std::process::id(), &mut session) } == 0 {
            return None;
        }
        query(session).map(|(d, _, _)| d)
    }

    pub fn of_user() -> Vec<Desktop> {
        let me = std::env::var("USERNAME").unwrap_or_default();
        let domain = std::env::var("USERDOMAIN").unwrap_or_default();
        let mut list = std::ptr::null_mut::<WTS_SESSION_INFOW>();
        let mut count = 0u32;
        // SAFETY: Windows allocates `list` with `count` entries; it is freed once below.
        let ids: Vec<u32> = unsafe {
            if WTSEnumerateSessionsW(WTS_CURRENT_SERVER_HANDLE, 0, 1, &mut list, &mut count) == 0 || list.is_null() {
                return vec![];
            }
            let ids = std::slice::from_raw_parts(list, count as usize).iter().map(|s| s.SessionId).collect();
            WTSFreeMemory(list.cast());
            ids
        };
        ids.into_iter()
            .filter_map(query)
            .filter(|(_, user, dom)| !user.is_empty() && user.eq_ignore_ascii_case(&me) && (domain.is_empty() || dom.eq_ignore_ascii_case(&domain)))
            .map(|(d, _, _)| d)
            .collect()
    }
}

#[cfg(windows)]
pub use imp::{of_process, of_user};

/// Seconds since the last keyboard or mouse input in this process's session: whether someone has
/// been at the screen lately (an agent asks before starting sign-ins; nothing waits on it).
#[cfg(windows)]
pub fn idle_seconds() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: `info` is a valid, sized out-parameter.
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            return None;
        }
        Some(u64::from(GetTickCount().wrapping_sub(info.dwTime) / 1000))
    }
}

#[cfg(not(windows))]
pub fn idle_seconds() -> Option<u64> { None }

#[cfg(not(windows))]
pub fn of_process() -> Option<Desktop> { None }

#[cfg(not(windows))]
pub fn of_user() -> Vec<Desktop> { vec![] }

#[cfg(test)]
mod tests {
    use super::*;

    fn d(session: u32, state: State, locked: Option<bool>) -> Desktop { Desktop { session, state, locked } }

    #[test]
    fn only_an_unlocked_shown_desktop_can_take_sign_ins() {
        assert_eq!(d(1, State::Active, Some(false)).problem(), None);
        assert_eq!(d(1, State::Active, None).problem(), None, "Windows not saying is not a lock");
        assert_eq!(d(1, State::Active, Some(true)).problem(), Some("desktop_locked"));
        assert_eq!(d(2, State::Disconnected, Some(false)).problem(), Some("desktop_not_shown"));
        assert_eq!(d(0, State::Other, None).problem(), Some("desktop_not_shown"));
    }

    #[test]
    fn the_best_session_is_the_one_someone_can_use() {
        assert_eq!(best(&[]), None);
        let pick = best(&[d(3, State::Disconnected, None), d(1, State::Active, Some(true)), d(2, State::Active, Some(false))]);
        assert_eq!(pick.map(|d| d.session), Some(2));
        assert_eq!(best(&[d(3, State::Disconnected, None), d(1, State::Active, Some(true))]).map(|d| d.session), Some(1));
    }

    #[cfg(windows)]
    #[test]
    fn this_process_has_a_session() {
        // Tests run in some session (a desktop, a service or CI): the query must answer.
        assert!(of_process().is_some());
    }
}
