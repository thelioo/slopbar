//! What the Codex CLI on this PC remembers about each account: threads it ran, tokens they used,
//! and the weekly limit readings its rollouts recorded. It lets the Subscriptions page judge a
//! Codex plan today instead of after weeks of SlopBar's own polling.
//!
//! Everything is read-only and local. `state_5.sqlite` names the account that started a thread
//! (`creator_account_id`, the ChatGPT account id a saved login carries) only for threads since
//! Codex began recording it, so anything earlier is unattributed and never counts as idle. Usage
//! from WSL, other machines or the cloud never reaches these files: the report says so.
//! Nothing here reads prompts, titles or paths beyond the rollout file names.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::subscriptions::WindowRecord;

/// Local activity and rollout readings older than this are not read.
const LOOKBACK_DAYS: i64 = 63;
const WEEK_MINUTES: i64 = 7 * 24 * 60;
/// Two readings of one window can report a reset a little apart.
const SAME_WINDOW_SECONDS: i64 = 3600;
/// Lines this long are content (a pasted file, an image), never a limit reading.
const MAX_LINE: usize = 1 << 16;

/// One account's threads on this PC, by the day each was last active (UTC).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Activity {
    pub threads: u32,
    pub tokens: u64,
    pub last: Option<DateTime<Utc>>,
    pub days: BTreeMap<NaiveDate, (u32, u64)>,
}

#[derive(Debug, Clone, Default)]
pub struct Local {
    /// The earliest thread that names its account: before it, nothing here can say who ran what.
    pub attributed_since: Option<DateTime<Utc>>,
    /// Keyed by ChatGPT account id.
    pub accounts: HashMap<String, Activity>,
    /// Completed weekly windows read from rollouts, per account id, marked `local`.
    pub weeks: HashMap<String, Vec<WindowRecord>>,
}

pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

/// Reads everything fresh enough to matter. Blocking: call from `spawn_blocking`.
pub fn scan(now: DateTime<Utc>) -> Local {
    let Some(home) = codex_home() else { return Local::default() };
    let (attributed_since, accounts) = threads_cached(&home.join("state_5.sqlite"), now).unwrap_or_default();
    let store = crate::settings::app_dir().map(|d| d.join("codex-rollouts.json"));
    let weeks = rollout_weeks(&[home.join("sessions"), home.join("archived_sessions")], store.as_deref(), now);
    Local { attributed_since, accounts, weeks }
}

// ---------------------------------------------------------------------------------------------
// state_5.sqlite

type Threads = (Option<DateTime<Utc>>, HashMap<String, Activity>);

/// The database changes only through its write-ahead log until Codex checkpoints, so the cache
/// keys on both files' times.
fn threads_cached(db: &Path, now: DateTime<Utc>) -> Option<Threads> {
    static CACHE: OnceLock<Mutex<Option<((Option<SystemTime>, Option<SystemTime>), Threads)>>> = OnceLock::new();
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let key = (mtime(db), mtime(&db.with_extension("sqlite-wal")));
    key.0?;
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((k, v)) = cache.lock().unwrap().as_ref() {
        if *k == key { return Some(v.clone()); }
    }
    let fresh = read_threads(db, now).ok()?;
    *cache.lock().unwrap() = Some((key, fresh.clone()));
    Some(fresh)
}

/// Opens the database read-only (Codex may be writing it: waits briefly for a lock, then gives up)
/// and folds attributed threads by account.
pub fn read_threads(db: &Path, now: DateTime<Utc>) -> rusqlite::Result<Threads> {
    use rusqlite::OpenFlags;
    let conn = rusqlite::Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    let since: Option<i64> = conn.query_row(
        "SELECT MIN(created_at) FROM threads WHERE creator_account_id IS NOT NULL AND creator_account_id != ''", [], |r| r.get(0))?;
    let cutoff = (now - Duration::days(LOOKBACK_DAYS)).timestamp();
    let mut stmt = conn.prepare(
        "SELECT creator_account_id, COUNT(*), SUM(tokens_used), MAX(updated_at) FROM threads \
         WHERE creator_account_id IS NOT NULL AND creator_account_id != '' GROUP BY creator_account_id")?;
    let mut accounts: HashMap<String, Activity> = HashMap::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, i64>(3)?)))? {
        let (id, n, tokens, last) = row?;
        let a = accounts.entry(id).or_default();
        a.threads = n.max(0) as u32;
        a.tokens = tokens.unwrap_or(0).max(0) as u64;
        a.last = Utc.timestamp_opt(last, 0).single();
    }
    let mut stmt = conn.prepare(
        "SELECT creator_account_id, updated_at, tokens_used FROM threads \
         WHERE creator_account_id IS NOT NULL AND creator_account_id != '' AND updated_at >= ?1")?;
    for row in stmt.query_map([cutoff], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (id, at, tokens) = row?;
        let Some(day) = Utc.timestamp_opt(at, 0).single().map(|t| t.date_naive()) else { continue };
        let d = accounts.entry(id).or_default().days.entry(day).or_default();
        d.0 += 1;
        d.1 += tokens.max(0) as u64;
    }
    Ok((since.and_then(|s| Utc.timestamp_opt(s, 0).single()), accounts))
}

// ---------------------------------------------------------------------------------------------
// Rollouts

/// One weekly limit reading: when, how far used, and when that window resets (unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Reading { at: i64, used: f64, resets: i64 }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FileSummary {
    len: u64,
    mtime: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    readings: Vec<Reading>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store { version: u8, files: HashMap<String, FileSummary> }

const STORE_VERSION: u8 = 1;
const MAX_STORE: u64 = 32 << 20;

/// Rollouts are written once and kept, and the first read of weeks of them takes a while, so
/// each file's readings are kept (in memory and in `store`) until its size or time changes.
fn rollout_weeks(roots: &[PathBuf], store: Option<&Path>, now: DateTime<Utc>) -> HashMap<String, Vec<WindowRecord>> {
    static CACHE: OnceLock<Mutex<Option<Store>>> = OnceLock::new();
    let mut guard = CACHE.get_or_init(Mutex::default).lock().unwrap();
    let cache = guard.get_or_insert_with(|| store.map(load_store).unwrap_or_default());
    let cutoff = SystemTime::now() - std::time::Duration::from_secs(LOOKBACK_DAYS as u64 * 86400);
    let mut files = vec![];
    for root in roots { collect(root, 4, cutoff, &mut files); }
    let mut changed = false;
    let mut by_account: HashMap<String, Vec<Reading>> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for (path, len, mtime) in files {
        let key = path.to_string_lossy().into_owned();
        let fresh = cache.files.get(&key).is_some_and(|s| s.len == len && s.mtime == mtime);
        if !fresh {
            let mut s = read_rollout(&path).unwrap_or_default();
            (s.len, s.mtime) = (len, mtime);
            cache.files.insert(key.clone(), s);
            changed = true;
        }
        let s = &cache.files[&key];
        if let Some(account) = &s.account {
            by_account.entry(account.clone()).or_default().extend(&s.readings);
        }
        seen.insert(key);
    }
    let before = cache.files.len();
    cache.files.retain(|k, _| seen.contains(k));
    if let Some(path) = store.filter(|_| changed || cache.files.len() != before) {
        cache.version = STORE_VERSION;
        if let Ok(bytes) = serde_json::to_vec(&*cache) { let _ = crate::private_store::write(path, &bytes); }
    }
    by_account.into_iter().map(|(id, readings)| (id, weeks_from(readings, now))).filter(|(_, w)| !w.is_empty()).collect()
}

fn load_store(path: &Path) -> Store {
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_STORE) { return Store::default(); }
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Store>(&b).ok())
        .filter(|s| s.version == STORE_VERSION).unwrap_or_default()
}

fn collect(dir: &Path, depth: u8, cutoff: SystemTime, out: &mut Vec<(PathBuf, u64, i64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let path = e.path();
        if meta.is_dir() {
            if depth > 0 { collect(&path, depth - 1, cutoff, out); }
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("rollout-") || !(name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) { continue; }
        let Ok(mtime) = meta.modified() else { continue };
        if mtime < cutoff { continue; }
        let secs = mtime.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        out.push((path, meta.len(), secs));
    }
}

fn read_rollout(path: &Path) -> std::io::Result<FileSummary> {
    let file = std::fs::File::open(path)?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zst") {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    Ok(summarize(BufReader::with_capacity(1 << 16, reader)))
}

/// The account from the session header, then the weekly readings of the main Codex limit while
/// they can still be that account's. A rollout from before Codex named the account is skipped
/// after its first line.
///
/// The header names who started the thread, not who runs it later: when the CLI's login changes
/// (SlopBar switches accounts), a resumed thread reports another account's window. A new window
/// is the same account's only once the previous one has reset and the new one began after it;
/// any other jump ends the file's readings.
fn summarize(mut reader: impl BufRead) -> FileSummary {
    let mut summary = FileSummary::default();
    let mut line = Vec::new();
    let mut first = true;
    let mut window: Option<i64> = None;
    'lines: loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) { Ok(0) | Err(_) => break, Ok(_) => {} }
        if first {
            first = false;
            let header: serde_json::Value = serde_json::from_slice(&line).unwrap_or_default();
            let account = header["payload"]["creator_account_id"].as_str().filter(|s| !s.is_empty());
            if header["type"] != "session_meta" || account.is_none() { return summary; }
            summary.account = account.map(String::from);
            continue;
        }
        if line.len() > MAX_LINE || !contains(&line, b"\"rate_limits\":{") { continue; }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else { continue };
        let p = &v["payload"];
        if p["type"] != "token_count" { continue; }
        let limits = &p["rate_limits"];
        // Other ids are separate per-model limits, not the plan's weekly allowance.
        if !(limits["limit_id"].is_null() || limits["limit_id"] == "codex") { continue; }
        let Some(at) = v["timestamp"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.timestamp()) else { continue };
        for w in [&limits["primary"], &limits["secondary"]] {
            if w["window_minutes"].as_i64() != Some(WEEK_MINUTES) { continue; }
            let (Some(used), Some(resets)) = (w["used_percent"].as_f64(), w["resets_at"].as_i64()) else { continue };
            match window {
                Some(cur) if (resets - cur).abs() < SAME_WINDOW_SECONDS => {}
                Some(cur) if at >= cur && resets - WEEK_MINUTES * 60 >= cur - SAME_WINDOW_SECONDS => window = Some(resets),
                Some(_) => break 'lines,
                None => window = Some(resets),
            }
            summary.readings.push(Reading { at, used, resets });
        }
    }
    // One reading per hour and window is all a week record uses: keep the highest.
    summary.readings.sort_by(|a, b| (a.resets, a.at / 3600).cmp(&(b.resets, b.at / 3600)).then(b.used.total_cmp(&a.used)));
    summary.readings.dedup_by(|later, kept| later.resets == kept.resets && later.at / 3600 == kept.at / 3600);
    summary
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Folds readings into completed weekly windows, the same records SlopBar's own polling makes.
/// A reading taken after its window reset, or of a window that has not ended, is not a record.
/// Two windows of one account cannot overlap; if they seem to, the better watched one stays.
fn weeks_from(mut readings: Vec<Reading>, now: DateTime<Utc>) -> Vec<WindowRecord> {
    readings.retain(|r| r.at < r.resets && r.resets <= now.timestamp());
    readings.sort_by_key(|r| (r.resets, r.at));
    let mut groups: Vec<Vec<Reading>> = vec![];
    for r in readings {
        match groups.last_mut().filter(|g| r.resets - g[0].resets < SAME_WINDOW_SECONDS) {
            Some(g) => g.push(r),
            None => groups.push(vec![r]),
        }
    }
    let mut out: Vec<WindowRecord> = vec![];
    for g in groups {
        let time = |s: i64| Utc.timestamp_opt(s, 0).single();
        let (Some(resets_at), Some(first_obs), Some(last_obs)) = (time(g[0].resets),
            time(g.iter().map(|r| r.at).min().unwrap_or(0)), time(g.iter().map(|r| r.at).max().unwrap_or(0))) else { continue };
        let hours: std::collections::HashSet<i64> = g.iter().map(|r| r.at / 3600).collect();
        let days: std::collections::HashSet<NaiveDate> = g.iter().filter_map(|r| time(r.at)).map(|t| t.date_naive()).collect();
        let w = WindowRecord { kind: "weekly".into(), resets_at, peak: g.iter().map(|r| r.used).fold(0.0, f64::max), first_obs, last_obs,
            hours: hours.len().min(u16::MAX as usize) as u16, days: days.len().min(u8::MAX as usize) as u8, idle: false, local: true };
        match out.last_mut().filter(|p| (w.resets_at - p.resets_at).num_seconds() < WEEK_MINUTES * 60 - 86400) {
            Some(p) if w.hours > p.hours => *p = w,
            Some(_) => {}
            None => out.push(w),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Per account, for the report

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WeekCount {
    /// Monday of the ISO week (UTC).
    pub start: NaiveDate,
    pub threads: u32,
    pub tokens: u64,
}

/// What this PC's Codex shows for one account. Only threads Codex attributed count.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalActivity {
    pub last_local_activity: Option<DateTime<Utc>>,
    /// Days since that last thread; with none, days since attribution began (at least that long).
    pub days_idle: Option<i64>,
    /// No attributed thread at all: `days_idle` is a lower bound.
    pub never_seen: bool,
    pub attributed_since: Option<DateTime<Utc>>,
    pub threads_7d: u32,
    pub tokens_7d: u64,
    pub threads_28d: u32,
    pub tokens_28d: u64,
    pub threads_total: u32,
    pub tokens_total: u64,
    /// The last eight ISO weeks, oldest first, including the one in progress.
    pub weeks: Vec<WeekCount>,
    /// Completed weekly windows read from rollouts and added to the history.
    pub backfilled_weeks: usize,
}

impl Local {
    /// `id` is the slot id (`user:account`); its ChatGPT account id is the part after the last colon.
    pub fn activity(&self, slot_id: &str, now: DateTime<Utc>) -> Option<LocalActivity> {
        let account = account_id(slot_id)?;
        let since = self.attributed_since?;
        let empty = Activity::default();
        let a = self.accounts.get(account).unwrap_or(&empty);
        let today = now.date_naive();
        let window = |days: i64| a.days.range(today - Duration::days(days - 1)..).fold((0, 0), |(n, t), (_, (dn, dt))| (n + dn, t + dt));
        let (threads_7d, tokens_7d) = window(7);
        let (threads_28d, tokens_28d) = window(28);
        let monday = today - Duration::days(chrono::Datelike::weekday(&today).num_days_from_monday() as i64);
        let weeks = (0..8).rev().map(|i| {
            let start = monday - Duration::weeks(i);
            let (threads, tokens) = a.days.range(start..start + Duration::days(7)).fold((0, 0), |(n, t), (_, (dn, dt))| (n + dn, t + dt));
            WeekCount { start, threads, tokens }
        }).collect();
        let idle_from = a.last.unwrap_or(since);
        Some(LocalActivity {
            last_local_activity: a.last,
            days_idle: Some((now - idle_from).num_days().max(0)),
            never_seen: a.last.is_none(),
            attributed_since: Some(since),
            threads_7d, tokens_7d, threads_28d, tokens_28d,
            threads_total: a.threads,
            tokens_total: a.tokens,
            weeks,
            backfilled_weeks: self.weeks.get(account).map_or(0, Vec::len),
        })
    }

    pub fn weeks_for(&self, slot_id: &str) -> &[WindowRecord] {
        account_id(slot_id).and_then(|a| self.weeks.get(a)).map_or(&[], Vec::as_slice)
    }
}

fn account_id(slot_id: &str) -> Option<&str> {
    if slot_id.starts_with("token:") { return None; }
    slot_id.rsplit_once(':').map(|(_, a)| a).filter(|a| !a.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    /// A threads table shaped like Codex's, with synthetic accounts only.
    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("state_5.sqlite");
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
            tokens_used INTEGER NOT NULL DEFAULT 0, title TEXT NOT NULL DEFAULT 'secret prompt', creator_user_id TEXT, creator_account_id TEXT);").unwrap();
        let rows: [(&str, Option<&str>, &str, i64); 6] = [
            ("t1", None, "2026-08-01T00:00:00Z", 500),                  // before attribution began
            ("t2", Some("acct-busy"), "2026-09-25T10:00:00Z", 1000),
            ("t3", Some("acct-busy"), "2026-10-19T10:00:00Z", 2000),
            ("t4", Some("acct-busy"), "2026-10-20T23:00:00Z", 3000),
            ("t5", Some("acct-old"), "2026-09-28T10:00:00Z", 400),
            ("t6", Some(""), "2026-10-20T10:00:00Z", 9),
        ];
        for (id, acct, when, tokens) in rows {
            let t = at(when).timestamp();
            c.execute("INSERT INTO threads (id, created_at, updated_at, tokens_used, creator_account_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, t, t, tokens, acct]).unwrap();
        }
        path
    }

    fn local_from(dir: &Path, now: DateTime<Utc>) -> Local {
        let (attributed_since, accounts) = read_threads(&fixture(dir), now).unwrap();
        Local { attributed_since, accounts, weeks: HashMap::new() }
    }

    #[test]
    fn threads_fold_by_account_and_unattributed_ones_count_for_nobody() {
        let dir = TempDir::new();
        let now = at("2026-10-21T12:00:00Z");
        let local = local_from(&dir.0, now);
        assert_eq!(local.attributed_since, Some(at("2026-09-25T10:00:00Z")));
        assert_eq!(local.accounts.len(), 2);
        let busy = local.activity("user-1:acct-busy", now).unwrap();
        assert_eq!((busy.threads_total, busy.tokens_total), (3, 6000));
        assert_eq!((busy.threads_7d, busy.tokens_7d, busy.threads_28d), (2, 5000, 3));
        assert_eq!((busy.last_local_activity, busy.days_idle, busy.never_seen), (Some(at("2026-10-20T23:00:00Z")), Some(0), false));
        assert_eq!(busy.weeks.len(), 8);
        assert_eq!(busy.weeks.last().unwrap(), &WeekCount { start: "2026-10-19".parse().unwrap(), threads: 2, tokens: 5000 });

        let old = local.activity("user-2:acct-old", now).unwrap();
        assert_eq!((old.threads_28d, old.days_idle), (1, Some(23)));
        assert_eq!((old.threads_7d, old.weeks.iter().map(|w| w.threads).sum::<u32>()), (0, 1));

        // An account this PC never ran is idle since attribution began, and says it is a lower bound.
        let never = local.activity("user-3:acct-never", now).unwrap();
        assert_eq!((never.never_seen, never.days_idle, never.threads_28d), (true, Some(26), 0));
        assert!(local.activity("token:abc", now).is_none());
    }

    #[test]
    fn a_missing_or_foreign_database_reads_as_nothing() {
        let dir = TempDir::new();
        assert!(threads_cached(&dir.0.join("state_5.sqlite"), Utc::now()).is_none());
        std::fs::write(dir.0.join("junk.sqlite"), b"not a database").unwrap();
        assert!(read_threads(&dir.0.join("junk.sqlite"), Utc::now()).is_err());
        assert!(Local::default().activity("u:a", Utc::now()).is_none(), "no attribution, no claim of idleness");
    }

    #[test]
    fn the_database_is_opened_read_only() {
        let dir = TempDir::new();
        let path = fixture(&dir.0);
        let before = std::fs::read(&path).unwrap();
        read_threads(&path, at("2026-10-21T12:00:00Z")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    fn rollout(account: Option<&str>, readings: &[(&str, f64, &str)]) -> String {
        let mut s = match account {
            Some(a) => format!("{{\"timestamp\":\"2026-10-01T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"x\",\"creator_account_id\":\"{a}\"}}}}\n"),
            None => "{\"timestamp\":\"2026-10-01T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"x\"}}\n".into(),
        };
        s.push_str("{\"timestamp\":\"2026-10-01T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"content\":\"secret\"}}\n");
        for (when, used, resets) in readings {
            let resets = at(resets).timestamp();
            s.push_str(&format!("{{\"timestamp\":\"{when}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":null,\"rate_limits\":{{\"limit_id\":\"codex\",\"limit_name\":null,\"primary\":{{\"used_percent\":{used},\"window_minutes\":10080,\"resets_at\":{resets}}},\"secondary\":null,\"plan_type\":\"pro\"}}}}}}\n"));
            // A per-model limit in the same file is not the plan's allowance.
            s.push_str(&format!("{{\"timestamp\":\"{when}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex_other\",\"primary\":{{\"used_percent\":99,\"window_minutes\":10080,\"resets_at\":{resets}}}}}}}}}\n"));
        }
        s
    }

    #[test]
    fn rollouts_give_completed_weeks_per_account() {
        let dir = TempDir::new();
        let day = dir.0.join("sessions/2026/10/01");
        std::fs::create_dir_all(&day).unwrap();
        let mut readings = vec![];
        // A week watched over four days, then the window in progress.
        for (d, h, used) in [(13, 9, 5.0), (13, 9, 6.0), (13, 15, 10.0), (14, 10, 20.0), (15, 10, 30.0), (16, 10, 42.0)] {
            readings.push((format!("2026-10-{d:02}T{h:02}:30:00Z"), used, "2026-10-17T00:00:00Z"));
        }
        readings.push(("2026-10-20T10:00:00Z".into(), 3.0, "2026-10-24T00:00:00Z"));
        let readings: Vec<(&str, f64, &str)> = readings.iter().map(|(a, b, c)| (a.as_str(), *b, *c)).collect();
        std::fs::write(day.join("rollout-a.jsonl"), rollout(Some("acct-busy"), &readings)).unwrap();
        let zst = zstd::encode_all(rollout(Some("acct-busy"), &[("2026-10-16T20:00:00Z", 44.0, "2026-10-17T00:20:00Z")]).as_bytes(), 3).unwrap();
        std::fs::write(day.join("rollout-b.jsonl.zst"), zst).unwrap();
        std::fs::write(day.join("rollout-c.jsonl"), rollout(None, &[("2026-10-14T10:00:00Z", 80.0, "2026-10-17T00:00:00Z")])).unwrap();
        std::fs::write(day.join("notes.jsonl"), rollout(Some("acct-x"), &[("2026-10-14T10:00:00Z", 80.0, "2026-10-17T00:00:00Z")])).unwrap();

        let store = dir.0.join("codex-rollouts.json");
        let weeks = rollout_weeks(&[dir.0.join("sessions")], Some(&store), at("2026-10-21T12:00:00Z"));
        assert_eq!(weeks.keys().collect::<Vec<_>>(), ["acct-busy"]);
        let w = &weeks["acct-busy"];
        assert_eq!(w.len(), 1, "only the completed window; reset jitter does not split it");
        assert_eq!((w[0].peak, w[0].hours, w[0].days, w[0].local), (44.0, 6, 4, true));
        assert_eq!(w[0].resets_at, at("2026-10-17T00:00:00Z"));
        let kept = load_store(&store);
        assert_eq!(kept.files.len(), 3, "every rollout is remembered, attributed or not; other files are not");
        assert!(!std::fs::read_to_string(&store).unwrap().contains("secret"), "no content is kept");
    }

    #[test]
    fn a_thread_resumed_under_another_login_stops_counting_for_its_creator() {
        let text = rollout(Some("acct-a"), &[
            ("2026-10-10T10:00:00Z", 20.0, "2026-10-12T00:00:00Z"),
            // The window resets and a new one begins: still the same account.
            ("2026-10-12T05:00:00Z", 1.0, "2026-10-19T05:00:00Z"),
            // Mid-window jump to a window that began earlier: another account's.
            ("2026-10-13T05:00:00Z", 80.0, "2026-10-15T00:00:00Z"),
            ("2026-10-13T06:00:00Z", 2.0, "2026-10-19T05:00:00Z"),
        ]);
        let s = summarize(std::io::Cursor::new(text));
        assert_eq!(s.account.as_deref(), Some("acct-a"));
        assert_eq!(s.readings.iter().map(|r| r.used).collect::<Vec<_>>(), [20.0, 1.0]);
    }

    #[test]
    fn a_reading_after_its_reset_is_not_part_of_that_week() {
        let r = |when: &str, used, resets: &str| Reading { at: at(when).timestamp(), used, resets: at(resets).timestamp() };
        let w = weeks_from(vec![r("2026-10-10T00:00:00Z", 10.0, "2026-10-12T00:00:00Z"), r("2026-10-13T00:00:00Z", 90.0, "2026-10-12T00:00:00Z")],
            at("2026-10-21T00:00:00Z"));
        assert_eq!(w[0].peak, 10.0);
    }
}
