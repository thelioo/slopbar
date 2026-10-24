//! Account leases: an agent (or the `slopbar-run` launcher) reserves an account for a job, so
//! nobody else is steered onto it meanwhile. recommend_account leaves accounts leased by someone
//! else out of its ranking, automatic switching never moves to a leased account, and the flyout
//! shows "leased: <purpose>" on the account's row.
//!
//! Leases live in memory only: they end when SlopBar quits, when they expire, when released, or
//! for a launcher's lease, when the process that took it exits. A leased account that is not the
//! CLI's current sign-in keeps its project profile signed in (see `accounts::sync_profiles`), so
//! a command started with that profile keeps working.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::accounts::Provider;

/// A lease taken without `minutes`.
pub const DEFAULT_MINUTES: u32 = 120;
/// The longest lease an agent can take.
pub const MAX_MINUTES: u32 = 24 * 60;
/// A process's lease ends when the process does, and after this at the latest.
pub const PROCESS_DAYS: i64 = 7;
pub const MAX_LEASES: usize = 64;
pub const MAX_PURPOSE: usize = 80;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Lease {
    pub lease_id: String,
    pub provider: Provider,
    /// The leased account's id (as in get_usage).
    pub id: String,
    pub purpose: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// The process whose life the lease lasts (the launcher's).
    pub pid: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Leases {
    list: Vec<Lease>,
    next: u64,
}

/// A purpose as leases keep it: trimmed, plain text, bounded.
pub fn clean_purpose(purpose: &str) -> Result<String, String> {
    let text: String = purpose.trim().chars().filter(|c| !c.is_control()).take(MAX_PURPOSE).collect();
    if text.is_empty() { Err("purpose must name the job".into()) } else { Ok(text) }
}

impl Leases {
    /// Drops expired leases and those whose process has exited. Returns whether any ended.
    pub fn prune(&mut self, now: DateTime<Utc>, alive: impl Fn(u32) -> bool) -> bool {
        let before = self.list.len();
        self.list.retain(|l| l.expires_at > now && l.pid.map_or(true, &alive));
        self.list.len() != before
    }

    /// Leases account `id` of `provider` for `purpose`: for `minutes` (default two hours, at most
    /// a day), or while process `pid` runs (at most a week).
    pub fn take(&mut self, provider: Provider, id: &str, purpose: &str, minutes: Option<u32>, pid: Option<u32>, now: DateTime<Utc>) -> Result<Lease, String> {
        let purpose = clean_purpose(purpose)?;
        if minutes.is_some_and(|m| !(1..=MAX_MINUTES).contains(&m)) {
            return Err(format!("minutes must be 1-{MAX_MINUTES}"));
        }
        if let Some(held) = self.holder(provider, id) {
            return Err(format!("account_leased: {}", held.purpose));
        }
        if self.list.len() >= MAX_LEASES {
            return Err(format!("at most {MAX_LEASES} leases at once"));
        }
        self.next += 1;
        let expires_at = match (pid, minutes) {
            (_, Some(m)) => now + Duration::minutes(m as i64),
            (Some(_), None) => now + Duration::days(PROCESS_DAYS),
            (None, None) => now + Duration::minutes(DEFAULT_MINUTES as i64),
        };
        // Unique within the run; nothing about the account in it.
        let lease_id = format!("lease-{:x}-{}", now.timestamp_millis(), self.next);
        let lease = Lease { lease_id, provider, id: id.to_string(), purpose, created_at: now, expires_at, pid };
        self.list.push(lease.clone());
        Ok(lease)
    }

    pub fn release(&mut self, lease_id: &str) -> Option<Lease> {
        let i = self.list.iter().position(|l| l.lease_id == lease_id)?;
        Some(self.list.remove(i))
    }

    /// The lease on an account, if any.
    pub fn holder(&self, provider: Provider, id: &str) -> Option<&Lease> {
        self.list.iter().find(|l| l.provider == provider && l.id == id)
    }

    /// The provider's leased accounts with their purposes, leaving out lease `mine`.
    pub fn others(&self, provider: Provider, mine: Option<&str>) -> Vec<(String, String)> {
        self.list.iter()
            .filter(|l| l.provider == provider && mine != Some(l.lease_id.as_str()))
            .map(|l| (l.id.clone(), l.purpose.clone()))
            .collect()
    }

    /// Every leased account, for automatic switching and profile syncing.
    pub fn accounts(&self) -> Vec<(Provider, String)> {
        self.list.iter().map(|l| (l.provider, l.id.clone())).collect()
    }

    pub fn list(&self) -> Vec<Lease> {
        self.list.clone()
    }
}

/// Whether process `pid` is still running.
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: the handle, if any, is queried and closed once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(process, &mut code) != 0;
        CloseHandle(process);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
pub fn process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    #[test]
    fn a_lease_reserves_one_account_until_it_expires_or_is_released() {
        let now = at("2026-10-22T10:00:00Z");
        let mut leases = Leases::default();
        let a = leases.take(Provider::Claude, "user-1:org-1", "  nightly refactor\n", Some(30), None, now).unwrap();
        assert_eq!((a.purpose.as_str(), a.expires_at), ("nightly refactor", at("2026-10-22T10:30:00Z")));
        assert_eq!(leases.take(Provider::Claude, "user-1:org-1", "other job", None, None, now).unwrap_err(), "account_leased: nightly refactor");
        // The same id under the other provider is another account.
        let b = leases.take(Provider::Codex, "user-1:org-1", "review", None, None, now).unwrap();
        assert_eq!(b.expires_at, now + Duration::minutes(DEFAULT_MINUTES as i64));
        assert_ne!(a.lease_id, b.lease_id);
        assert_eq!(leases.others(Provider::Claude, None), [("user-1:org-1".to_string(), "nightly refactor".to_string())]);
        assert!(leases.others(Provider::Claude, Some(&a.lease_id)).is_empty(), "the holder's own lease does not exclude it");
        assert!(!leases.prune(at("2026-10-22T10:29:00Z"), |_| true));
        assert!(leases.prune(at("2026-10-22T10:30:00Z"), |_| true), "expired");
        assert!(leases.holder(Provider::Claude, "user-1:org-1").is_none());
        assert_eq!(leases.release(&b.lease_id).map(|l| l.id), Some("user-1:org-1".into()));
        assert!(leases.release(&b.lease_id).is_none());
        assert!(leases.list().is_empty());
    }

    #[test]
    fn a_process_lease_ends_with_its_process() {
        let now = at("2026-10-22T10:00:00Z");
        let mut leases = Leases::default();
        let l = leases.take(Provider::Codex, "user-2:acct-2", "slopbar-run", None, Some(4242), now).unwrap();
        assert_eq!(l.expires_at, now + Duration::days(PROCESS_DAYS));
        assert!(!leases.prune(now, |pid| pid == 4242));
        assert!(leases.prune(now, |_| false));
        assert_eq!(leases.accounts(), []);
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn refuses_bad_leases() {
        let now = at("2026-10-22T10:00:00Z");
        let mut leases = Leases::default();
        assert!(leases.take(Provider::Claude, "a", " \n", None, None, now).is_err(), "a purpose is required");
        assert!(leases.take(Provider::Claude, "a", "job", Some(0), None, now).is_err());
        assert!(leases.take(Provider::Claude, "a", "job", Some(MAX_MINUTES + 1), None, now).is_err());
        assert_eq!(clean_purpose(&"x".repeat(200)).unwrap().len(), MAX_PURPOSE);
        for i in 0..MAX_LEASES { leases.take(Provider::Claude, &format!("acct-{i}"), "job", None, None, now).unwrap(); }
        assert!(leases.take(Provider::Claude, "one-more", "job", None, None, now).is_err());
    }
}
