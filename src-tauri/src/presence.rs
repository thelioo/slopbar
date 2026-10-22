//! Which accounts are in use on the user's other machines.
//!
//! Neither Anthropic nor OpenAI tells a token where else its account is signed in, so SlopBar
//! learns it two ways:
//!
//! - **Presence records** (opt-in). Every SlopBar keeps one record in a `PresenceStore`: today a
//!   folder the user already syncs (OneDrive, Dropbox, Google Drive, iCloud Drive, Box), as
//!   `<folder>/SlopBar/presence/<machine id>.json`; later, possibly a cloud API (`CloudApi`). The
//!   record lists which accounts the machine has signed in. Accounts appear there only as a unsalted SHA-256 prefix of
//!   `provider|slot id`, which every machine computes the same way; no emails, tokens or
//!   organization names are written.
//! - **Inference**, for monitored accounts nobody claims: usage that keeps rising while the
//!   account is signed in nowhere SlopBar can see means it is used outside this PC (another
//!   machine, the web or the desktop app).
//!
//! `AccountUsage::active` and `sources` keep meaning *this machine*: switching, the remove guard
//! and the balancer depend on that. Other machines are reported in `remote` only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::accounts::Provider;
use crate::model::{AccountUsage, RemoteSeat};

/// Presence files live here, under the folder the user picked.
const SUBDIR: [&str; 2] = ["SlopBar", "presence"];
/// A machine counts as live for at least this long after it last wrote.
const MIN_LIVE: i64 = 15 * 60;
/// Machines that stopped writing are shown dimmed ("seen 3h ago") this long.
const SHOW_FOR: i64 = 24 * 3600;
/// Files older than this are ignored altogether.
const IGNORE_AFTER: i64 = 7 * 24 * 3600;
/// A clock this far ahead of ours is not trusted; the file's modification time is used instead.
const SKEW: i64 = 5 * 60;
/// Machine names longer than this are cut (they come from other machines).
const NAME_MAX: usize = 63;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seat {
    pub p: Provider,
    /// `hash(provider, slot id)`.
    pub h: String,
    #[serde(rename = "where", default)]
    pub where_: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    pub v: u32,
    pub machine_id: String,
    pub machine: String,
    pub written_at: DateTime<Utc>,
    pub interval_s: u64,
    #[serde(default)]
    pub seats: Vec<Seat>,
}

/// How another machine's account appears on every machine: the first 32 hex digits of
/// SHA-256(`provider|slot id`). Stable across versions and platforms (unlike `DefaultHasher`).
pub fn hash(provider: Provider, id: &str) -> String {
    let digest = Sha256::digest(format!("{}|{id}", provider.key()).as_bytes());
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

pub fn dir(folder: &Path) -> PathBuf {
    SUBDIR.iter().fold(folder.to_path_buf(), |p, s| p.join(s))
}

/// This installation's id: random, kept in the app's data folder, never derived from the user.
pub fn machine_id() -> Option<String> {
    let path = crate::settings::app_dir()?.join("machine-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if uuid::Uuid::parse_str(id).is_ok() {
            return Some(id.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    crate::private_store::write(&path, id.as_bytes()).ok()?;
    Some(id)
}

/// This machine's id and name, read once.
pub fn identity() -> Option<&'static (String, String)> {
    static IDENTITY: std::sync::OnceLock<Option<(String, String)>> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(|| Some((machine_id()?, machine_name()))).as_ref()
}

/// Says this machine has nothing signed in any more: on quit, or when sharing moves elsewhere.
pub fn withdraw(store: &dyn PresenceStore, interval_s: u64) {
    if let Some((id, name)) = identity() {
        let _ = store.clear(id, name, interval_s);
    }
}

/// The computer's name, as other machines will show it.
pub fn machine_name() -> String {
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    clean_name(&name).unwrap_or_else(|| "?".into())
}

fn clean_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
    (!name.is_empty()).then_some(name)
}

// ---------------------------------------------------------------------------------------------
// Storage

/// Where presence records are kept. The reader, staleness rules and UI only see this trait, so a
/// cloud API backend (see `CloudApi` and docs/presence-cloud-backends.md) plugs in beside the
/// synced folder without touching them.
pub trait PresenceStore: Send + Sync {
    /// Every record there, with when the storage last modified it (when it knows).
    fn list(&self) -> std::io::Result<Vec<(Presence, Option<DateTime<Utc>>)>>;
    /// Replaces this machine's record.
    fn write(&self, own: &Presence) -> std::io::Result<()>;
    /// Says this machine has nothing signed in.
    fn clear(&self, machine_id: &str, machine: &str, interval_s: u64) -> std::io::Result<()> {
        self.write(&own(machine_id, machine, interval_s, &[], Utc::now()))
    }
}

/// The store the settings choose, if sharing is on.
pub fn store(settings: &crate::settings::Settings) -> Option<Box<dyn PresenceStore>> {
    if let Some(remote) = &settings.presence_remote {
        // A chosen remote never falls back to a folder: rclone missing means no sharing.
        return crate::rclone::Rclone::open(remote).map(|s| Box::new(s) as Box<dyn PresenceStore>);
    }
    settings.presence_folder.as_ref().map(|f| Box::new(SyncedFolder(PathBuf::from(f))) as Box<dyn PresenceStore>)
}

/// A folder a sync client (OneDrive, Dropbox, Google Drive…) mirrors between machines: records are
/// plain files in `<folder>/SlopBar/presence/`, one per machine.
pub struct SyncedFolder(pub PathBuf);

impl PresenceStore for SyncedFolder {
    fn list(&self) -> std::io::Result<Vec<(Presence, Option<DateTime<Utc>>)>> {
        let entries = match std::fs::read_dir(dir(&self.0)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        Ok(entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some("json") {
                    return None;
                }
                let meta = e.metadata().ok()?;
                // Presence files are small; anything large is not one.
                if meta.len() > 256 * 1024 {
                    return None;
                }
                let presence: Presence = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
                Some((presence, meta.modified().ok().map(DateTime::<Utc>::from)))
            })
            .collect())
    }

    /// Atomic (temp file and rename), so a sync client never uploads half a file.
    fn write(&self, presence: &Presence) -> std::io::Result<()> {
        if !self.0.is_dir() {
            return Err(std::io::Error::other("presence folder missing"));
        }
        let safe: String = presence.machine_id.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        let bytes = serde_json::to_vec_pretty(presence)?;
        crate::private_store::write(&dir(&self.0).join(format!("{safe}.json")), &bytes)
    }
}

/// A cloud provider's own API, for machines without its sync client. Each would keep the records
/// in an app-private folder only (Dropbox App folder, Google Drive `appDataFolder`, OneDrive
/// `approot`), with its OAuth tokens in DPAPI storage (`private_store::save_secret`), never in
/// settings.json. None is configured yet: each needs an app registered by the owner (see
/// docs/presence-cloud-backends.md), so for now every call reports `not_configured`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum CloudProvider {
    Dropbox,
    GoogleDrive,
    OneDrive,
}

#[allow(dead_code)] // Selectable once an app is registered for the provider.
pub struct CloudApi(pub CloudProvider);

#[allow(dead_code)]
impl CloudApi {
    /// The OAuth client id registered for this provider; none is, yet.
    fn client_id(&self) -> Option<&'static str> {
        None
    }

    fn not_configured(&self) -> std::io::Error {
        std::io::Error::other("not_configured")
    }
}

impl PresenceStore for CloudApi {
    fn list(&self) -> std::io::Result<Vec<(Presence, Option<DateTime<Utc>>)>> {
        self.client_id().map(|_| vec![]).ok_or_else(|| self.not_configured())
    }

    fn write(&self, _own: &Presence) -> std::io::Result<()> {
        self.client_id().map(|_| ()).ok_or_else(|| self.not_configured())
    }
}

/// A folder a sync client keeps the same on every machine.
#[derive(Debug, Clone, Serialize)]
pub struct SyncFolder {
    /// The provider, as Settings shows it.
    pub name: &'static str,
    pub path: String,
}

/// Every sync-client folder on this machine that SlopBar can offer.
pub fn sync_folders() -> Vec<SyncFolder> {
    let home = dirs::home_dir();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut candidates: Vec<(&'static str, PathBuf)> = vec![];
    // OneDrive: personal and work accounts each have their own folder.
    for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        candidates.extend(env(var).map(|p| ("OneDrive", p)));
    }
    // Dropbox records where its folders are; ~/Dropbox is only the default.
    for base in [dirs::config_dir(), dirs::data_local_dir()].into_iter().flatten() {
        let info = std::fs::read_to_string(base.join("Dropbox").join("info.json")).unwrap_or_default();
        candidates.extend(dropbox_paths(&info).into_iter().map(|p| ("Dropbox", p)));
    }
    candidates.extend(home.as_ref().map(|h| ("Dropbox", h.join("Dropbox"))));
    // Google Drive for desktop: "My Drive" on its virtual drive (often G:), and folders it mirrors.
    for letter in 'D'..='Z' {
        candidates.push(("Google Drive", PathBuf::from(format!("{letter}:\\My Drive"))));
    }
    candidates.extend(home.as_ref().map(|h| ("Google Drive", h.join("My Drive"))));
    if let Some(local) = dirs::data_local_dir() {
        let config = std::fs::read(local.join("Google").join("DriveFS").join("root_preference_sqlite.db")).unwrap_or_default();
        candidates.extend(paths_in(&config).into_iter().map(|p| ("Google Drive", p)));
    }
    candidates.extend(home.as_ref().map(|h| ("iCloud Drive", h.join("iCloudDrive"))));
    candidates.extend(home.as_ref().map(|h| ("Box", h.join("Box"))));

    let mut out: Vec<SyncFolder> = vec![];
    for (name, path) in candidates {
        if !path.is_dir() {
            continue;
        }
        let path = path.to_string_lossy().trim_end_matches('\\').to_string();
        if !out.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
            out.push(SyncFolder { name, path });
        }
    }
    out
}

/// The folders Dropbox's info.json names (`{"personal": {"path": …}, "business": {…}}`).
fn dropbox_paths(info: &str) -> Vec<PathBuf> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(info) else { return vec![] };
    v.as_object()
        .into_iter()
        .flat_map(|o| o.values())
        .filter_map(|account| account["path"].as_str().map(PathBuf::from))
        .collect()
}

/// Absolute Windows folder paths spelled out in a binary config file (Google Drive keeps its
/// mirrored folders in SQLite); only ones that exist are offered later.
fn paths_in(bytes: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = vec![];
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i + 3 < chars.len() {
        if chars[i].is_ascii_alphabetic() && chars[i + 1] == ':' && chars[i + 2] == '\\' && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let end = (i..chars.len()).find(|&j| chars[j].is_control() || chars[j] == '\u{fffd}' || chars[j] == '"').unwrap_or(chars.len());
            let path: String = chars[i..end].iter().collect();
            if path.len() > 3 && path.len() < 260 {
                out.push(PathBuf::from(path.trim_end()));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// What this machine says about itself. Logins SlopBar could only identify by token
/// (`token:` slot ids) are left out: their id would change with the token.
pub fn own(machine_id: &str, machine: &str, interval_s: u64, accounts: &[AccountUsage], now: DateTime<Utc>) -> Presence {
    let mut seats: Vec<Seat> = accounts
        .iter()
        .filter(|a| a.active && !a.id.starts_with("token:"))
        .map(|a| Seat { p: a.provider, h: hash(a.provider, &a.id), where_: a.sources.clone() })
        .collect();
    seats.sort_by(|a, b| (a.p.key(), &a.h).cmp(&(b.p.key(), &b.h)));
    Presence { v: 1, machine_id: machine_id.into(), machine: machine.into(), written_at: now, interval_s, seats }
}

/// Another machine, as of its newest file.
#[derive(Debug, Clone)]
pub struct Peer {
    pub machine: String,
    pub seen_at: DateTime<Utc>,
    pub live: bool,
    pub seats: Vec<Seat>,
}

/// Every other machine in `store` that wrote within the last week.
pub fn read(store: &dyn PresenceStore, own_id: &str, now: DateTime<Utc>) -> Vec<Peer> {
    peers(store.list().unwrap_or_default(), own_id, now)
}

/// Sync clients leave conflict copies ("<id> (1).json", "<id>-LAPTOP.json"), so records are
/// grouped by the machine id inside them and the newest wins.
pub fn peers(files: impl IntoIterator<Item = (Presence, Option<DateTime<Utc>>)>, own_id: &str, now: DateTime<Utc>) -> Vec<Peer> {
    let mut newest: HashMap<String, (Presence, DateTime<Utc>)> = HashMap::new();
    for (presence, modified) in files {
        if presence.v != 1 || presence.machine_id.is_empty() || presence.machine_id == own_id {
            continue;
        }
        // A clock running ahead would keep a dead machine "live" forever: trust the file's own
        // modification time then, and never a moment after now.
        let seen = if presence.written_at > now + Duration::seconds(SKEW) {
            modified.unwrap_or(now - Duration::seconds(IGNORE_AFTER + 1)).min(now)
        } else {
            presence.written_at.min(now)
        };
        match newest.get(&presence.machine_id) {
            Some((_, at)) if *at >= seen => {}
            _ => {
                newest.insert(presence.machine_id.clone(), (presence, seen));
            }
        }
    }
    let mut out: Vec<Peer> = newest
        .into_values()
        .filter_map(|(p, seen)| {
            let age = (now - seen).num_seconds();
            if age > IGNORE_AFTER {
                return None;
            }
            let live_for = (p.interval_s.min(24 * 3600) as i64 * 3).max(MIN_LIVE);
            Some(Peer {
                machine: clean_name(&p.machine).unwrap_or_else(|| "?".into()),
                seen_at: seen,
                live: age <= live_for,
                seats: p.seats,
            })
        })
        .collect();
    out.sort_by(|a, b| a.machine.cmp(&b.machine));
    out
}

/// Fills each account's `remote` from the other machines' files: live ones, and ones that
/// stopped writing within the last day (dimmed).
pub fn attach(accounts: &mut [AccountUsage], peers: &[Peer], now: DateTime<Utc>) {
    for a in accounts.iter_mut() {
        let h = hash(a.provider, &a.id);
        a.remote = peers
            .iter()
            .filter(|p| p.live || (now - p.seen_at).num_seconds() <= SHOW_FOR)
            .filter_map(|p| {
                let seat = p.seats.iter().find(|s| s.p == a.provider && s.h == h)?;
                Some(RemoteSeat {
                    machine: p.machine.clone(),
                    where_: seat.where_.iter().filter_map(|w| clean_name(w)).take(8).collect(),
                    seen_at: p.seen_at,
                    live: p.live,
                })
            })
            .collect();
    }
}

// ---------------------------------------------------------------------------------------------
// Inference: used outside this PC

/// What the last fresh reading of one account looked like.
#[derive(Debug, Clone, Default)]
pub struct Track {
    fetched_at: Option<DateTime<Utc>>,
    /// (window kind, reset time, percent used)
    windows: Vec<(String, Option<DateTime<Utc>>, f64)>,
    /// Whether the account was unclaimed (not in use here or on a live peer) when read.
    unclaimed: bool,
    last_rise: Option<DateTime<Utc>>,
}

pub type Tracks = HashMap<(Provider, String), Track>;

/// What counts as a surge: a weekly window rising by `min_rise` points or more between two
/// fresh readings at most `max_gap` apart. Weekly windows move slowly, so a few points in one
/// refresh is real work being done somewhere, not rounding.
#[derive(Debug, Clone, Copy)]
pub struct Surge {
    pub min_rise: f64,
    pub max_gap: Duration,
}

/// Flags monitored accounts that nobody SlopBar can see is using, yet whose usage rose between
/// two consecutive fresh readings of the same window (same reset time) by at least one point.
/// The flag clears once `hold` passes without another rise. Returns the accounts whose weekly
/// window surged (see `Surge`) in this reading, for `unexpected`.
pub fn infer(tracks: &mut Tracks, accounts: &mut [AccountUsage], hold: Duration, surge: Surge, now: DateTime<Utc>) -> Vec<(Provider, String)> {
    tracks.retain(|(p, id), _| accounts.iter().any(|a| a.provider == *p && a.id == *id));
    let mut surged = vec![];
    for a in accounts.iter_mut() {
        let unclaimed = a.monitor_connected && !a.active && !a.remote.iter().any(|r| r.live);
        let track = tracks.entry((a.provider, a.id.clone())).or_default();
        let fresh = !a.stale && a.error.is_none() && a.monitor_error.is_none() && a.fetched_at.is_some();
        if fresh && a.fetched_at != track.fetched_at {
            let rise = |w: &crate::model::UsageWindow| {
                track.windows.iter().find_map(|(kind, resets, before)| {
                    (*kind == w.kind && resets.is_some() && *resets == w.resets_at).then(|| w.used_percent - before)
                })
            };
            let comparable = unclaimed && track.unclaimed;
            let rose = comparable && a.windows.iter().any(|w| rise(w).is_some_and(|r| r >= 1.0));
            let close = match (track.fetched_at, a.fetched_at) {
                (Some(before), Some(after)) => after - before <= surge.max_gap,
                _ => false,
            };
            if comparable && close && a.windows.iter().any(|w| w.kind.starts_with("weekly") && rise(w).is_some_and(|r| r >= surge.min_rise)) {
                surged.push((a.provider, a.id.clone()));
            }
            if rose {
                track.last_rise = a.fetched_at;
            }
            track.fetched_at = a.fetched_at;
            track.windows = a.windows.iter().map(|w| (w.kind.clone(), w.resets_at, w.used_percent)).collect();
            track.unclaimed = unclaimed;
        } else if !fresh {
            // A failed reading breaks the chain: the next fresh one has nothing to compare with.
            track.windows.clear();
        } else if !unclaimed {
            track.unclaimed = false;
        }
        if !unclaimed {
            track.last_rise = None;
        }
        a.used_outside = track.last_rise.is_some_and(|t| now - t <= hold);
    }
    surged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageWindow;
    use crate::test_support::TempDir;

    fn at(minutes: i64) -> DateTime<Utc> {
        // Real time: `PresenceStore::clear` stamps records with the clock.
        static BASE: std::sync::OnceLock<DateTime<Utc>> = std::sync::OnceLock::new();
        *BASE.get_or_init(Utc::now) + Duration::minutes(minutes)
    }

    fn account(id: &str, active: bool) -> AccountUsage {
        AccountUsage {
            id: id.into(),
            provider: Provider::Claude,
            email: Some(format!("{id}@example.invalid")),
            alias: None,
            plan: Some("max".into()),
            org: Some("Synthetic Org".into()),
            active,
            sources: if active { vec!["Windows".into(), "WSL: Ubuntu".into()] } else { vec![] },
            windows: vec![],
            credits: None,
            resets: vec![],
            error: None,
            monitor_error: None,
            stale: false,
            fetched_at: None,
            monitor_connected: true,
            billing: Default::default(),
            remote: vec![],
            used_outside: false,
            runs_out_at: None, unexpected: None,
        }
    }

    fn file(id: &str, machine: &str, written: DateTime<Utc>, seats: Vec<Seat>) -> Presence {
        Presence { v: 1, machine_id: id.into(), machine: machine.into(), written_at: written, interval_s: 300, seats }
    }

    fn seat(id: &str) -> Seat {
        Seat { p: Provider::Claude, h: hash(Provider::Claude, id), where_: vec!["Windows".into()] }
    }

    #[test]
    fn hashing_is_stable_and_provider_specific() {
        // Fixed vector (printf "claude|synthetic-account:synthetic-org" | sha256sum): every machine
        // and every future version must compute the same value.
        assert_eq!(hash(Provider::Claude, "synthetic-account:synthetic-org"), "95ebf9717011b553004c8c1c6a6241f1");
        assert_eq!(hash(Provider::Claude, "a").len(), 32);
        assert_ne!(hash(Provider::Claude, "a"), hash(Provider::Codex, "a"));
    }

    #[test]
    fn conflict_copies_resolve_to_the_newest_and_own_file_is_ignored() {
        let now = at(0);
        let peers = peers(
            [
                (file("peer", "LAPTOP-2", at(-30), vec![seat("old")]), None),
                (file("peer", "LAPTOP-2", at(-2), vec![seat("new")]), None),
                (file("peer", "LAPTOP-2", at(-10), vec![seat("mid")]), None),
                (file("me", "BACHELOR", at(-1), vec![seat("new")]), None),
            ],
            "me",
            now,
        );
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].seats, vec![seat("new")]);
        assert!(peers[0].live);
    }

    #[test]
    fn staleness_windows() {
        let now = at(0);
        let list = peers(
            [
                (file("a", "A", at(-14), vec![]), None),          // within 15 min floor
                (file("b", "B", at(-16), vec![]), None),          // past 3 × 5 min and 15 min
                (Presence { interval_s: 1800, ..file("c", "C", at(-80), vec![]) }, None), // 3 × 30 min
                (file("d", "D", at(-6 * 24 * 60), vec![]), None), // kept, not live
                (file("e", "E", at(-8 * 24 * 60), vec![]), None), // ignored
            ],
            "me",
            now,
        );
        let live: HashMap<_, _> = list.iter().map(|p| (p.machine.as_str(), p.live)).collect();
        assert_eq!(live, HashMap::from([("A", true), ("B", false), ("C", true), ("D", false)]));

        let mut accounts = vec![account("x", false)];
        let shown = |minutes: i64| {
            let mut accounts = accounts.clone();
            let p = peers([(file("p", "P", at(-minutes), vec![seat("x")]), None)], "me", now);
            attach(&mut accounts, &p, now);
            accounts[0].remote.first().map(|r| r.live)
        };
        assert_eq!(shown(5), Some(true));
        assert_eq!(shown(3 * 60), Some(false), "dimmed: seen 3h ago");
        assert_eq!(shown(25 * 60), None, "older than a day is not shown");
        attach(&mut accounts, &[], now);
        assert!(accounts[0].remote.is_empty());
    }

    #[test]
    fn future_timestamps_are_clamped() {
        let now = at(0);
        // Slightly ahead: treated as now.
        let p = peers([(file("a", "A", at(3), vec![]), None)], "me", now);
        assert_eq!(p[0].seen_at, now);
        // Far ahead: the file's modification time decides, so a dead machine goes stale.
        let p = peers([(file("a", "A", at(600), vec![]), Some(at(-120)))], "me", now);
        assert_eq!(p[0].seen_at, at(-120));
        assert!(!p[0].live);
        let p = peers([(file("a", "A", at(600), vec![]), None)], "me", now);
        assert!(p.is_empty(), "no trustworthy time at all");
    }

    #[test]
    fn writes_only_hashes_and_round_trips_through_the_folder() {
        let dir_ = TempDir::new();
        let store = SyncedFolder(dir_.0.clone());
        let now = at(0);
        let mut accounts = vec![account("synthetic-user:synthetic-org", true), account("token:abcdef", true), account("idle", false)];
        accounts[0].email = Some("someone@example.invalid".into());
        let mine = own("11111111-1111-4111-8111-111111111111", "BACHELOR", 300, &accounts, now);
        assert_eq!(mine.seats.len(), 1, "token: logins and idle accounts are left out");
        store.write(&mine).unwrap();
        let other = own("22222222-2222-4222-8222-222222222222", "LAPTOP-2", 300, &accounts, now);
        store.write(&other).unwrap();
        // A sync client's conflict copy of the other machine, older.
        let stale = Presence { written_at: at(-60), seats: vec![], ..other.clone() };
        std::fs::write(dir(&dir_.0).join("22222222-2222-4222-8222-222222222222 (1).json"), serde_json::to_vec(&stale).unwrap()).unwrap();

        for entry in std::fs::read_dir(dir(&dir_.0)).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(!text.contains('@'), "no emails: {text}");
            assert!(!text.contains("synthetic-user") && !text.contains("Synthetic Org") && !text.contains("abcdef"), "{text}");
        }

        let peers = read(&store, &mine.machine_id, now);
        assert_eq!(peers.len(), 1);
        let mut idle = vec![account("synthetic-user:synthetic-org", false)];
        attach(&mut idle, &peers, now);
        assert_eq!(idle[0].remote.len(), 1);
        assert_eq!(idle[0].remote[0].machine, "LAPTOP-2");
        assert_eq!(idle[0].remote[0].where_, vec!["Windows", "WSL: Ubuntu"]);
        assert!(idle[0].remote[0].live);
        assert!(!idle[0].active, "another machine never makes an account active here");

        // A clean quit empties the seats.
        store.clear(&other.machine_id, "LAPTOP-2", 300).unwrap();
        let peers = read(&store, &mine.machine_id, now + Duration::minutes(1));
        attach(&mut idle, &peers, now);
        assert!(idle[0].remote.is_empty());
    }

    /// A backend other than a folder, as a cloud API store would be.
    struct Memory(std::sync::Mutex<HashMap<String, Presence>>);
    impl PresenceStore for Memory {
        fn list(&self) -> std::io::Result<Vec<(Presence, Option<DateTime<Utc>>)>> {
            Ok(self.0.lock().unwrap().values().map(|p| (p.clone(), None)).collect())
        }
        fn write(&self, own: &Presence) -> std::io::Result<()> {
            self.0.lock().unwrap().insert(own.machine_id.clone(), own.clone());
            Ok(())
        }
    }

    #[test]
    fn any_store_backs_the_same_reader() {
        let store = Memory(Default::default());
        let now = at(0);
        store.write(&file("peer", "LAPTOP-2", now, vec![seat("x")])).unwrap();
        store.write(&file("me", "BACHELOR", now, vec![seat("x")])).unwrap();
        let mut list = vec![account("x", false)];
        attach(&mut list, &read(&store, "me", now), now);
        assert_eq!(list[0].remote.iter().map(|r| r.machine.as_str()).collect::<Vec<_>>(), ["LAPTOP-2"]);
        store.clear("peer", "LAPTOP-2", 300).unwrap();
        attach(&mut list, &read(&store, "me", now), now);
        assert!(list[0].remote.is_empty());
    }

    #[test]
    fn cloud_backends_report_not_configured() {
        for provider in [CloudProvider::Dropbox, CloudProvider::GoogleDrive, CloudProvider::OneDrive] {
            let store = CloudApi(provider);
            assert_eq!(store.list().unwrap_err().to_string(), "not_configured");
            assert_eq!(store.write(&file("me", "BACHELOR", at(0), vec![])).unwrap_err().to_string(), "not_configured");
            assert!(read(&store, "me", at(0)).is_empty(), "an unconfigured backend shows nothing");
        }
    }

    #[test]
    fn sync_client_configs_name_their_folders() {
        let info = r#"{"personal":{"path":"C:\\Users\\Someone\\Dropbox","host":1},"business":{"path":"D:\\Dropbox (Synthetic Co)"}}"#;
        assert_eq!(dropbox_paths(info), [PathBuf::from("C:\\Users\\Someone\\Dropbox"), PathBuf::from("D:\\Dropbox (Synthetic Co)")]);
        assert!(dropbox_paths("not json").is_empty());
        // As Google Drive's SQLite config holds them: text among binary bytes.
        let mut db = b"SQLite format 3\x00\x01\x02mirror_root\x05E:\\Synced\\Work\x00\x07".to_vec();
        db.extend_from_slice(b"\x03x C:\\Users\\Someone\\My Drive\x01tail");
        assert_eq!(paths_in(&db), [PathBuf::from("E:\\Synced\\Work"), PathBuf::from("C:\\Users\\Someone\\My Drive")]);
    }

    fn reading(a: &mut AccountUsage, minute: i64, percent: f64, resets: i64) {
        a.fetched_at = Some(at(minute));
        a.windows = vec![UsageWindow { kind: "session".into(), window_seconds: Some(18000), used_percent: percent, resets_at: Some(at(resets)) }];
    }

    const SURGE: Surge = Surge { min_rise: 2.0, max_gap: Duration::minutes(16) };

    fn weekly(a: &mut AccountUsage, minute: i64, percent: f64, resets: i64) {
        a.fetched_at = Some(at(minute));
        a.windows = vec![
            UsageWindow { kind: "session".into(), window_seconds: Some(18000), used_percent: 50.0, resets_at: Some(at(300)) },
            UsageWindow { kind: "weekly".into(), window_seconds: Some(604800), used_percent: percent, resets_at: Some(at(resets)) },
        ];
    }

    #[test]
    fn surges_are_weekly_rises_between_close_fresh_readings() {
        let hold = Duration::minutes(11);
        let mut tracks = Tracks::new();
        let mut list = vec![account("x", false)];
        weekly(&mut list[0], 0, 10.0, 9000);
        assert!(infer(&mut tracks, &mut list, hold, SURGE, at(0)).is_empty(), "one reading");
        weekly(&mut list[0], 5, 11.0, 9000);
        assert!(infer(&mut tracks, &mut list, hold, SURGE, at(5)).is_empty(), "under two points");
        weekly(&mut list[0], 10, 13.0, 9000);
        assert_eq!(infer(&mut tracks, &mut list, hold, SURGE, at(10)), [(Provider::Claude, "x".to_string())]);
        // Only the session window rising is not a surge.
        reading(&mut list[0], 15, 90.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(15));
        reading(&mut list[0], 20, 99.0, 300);
        assert!(infer(&mut tracks, &mut list, hold, SURGE, at(20)).is_empty(), "session window only");

        // The weekly window resetting is not a rise, even when the new one starts higher.
        let mut tracks = Tracks::new();
        weekly(&mut list[0], 0, 1.0, 9000);
        infer(&mut tracks, &mut list, hold, SURGE, at(0));
        weekly(&mut list[0], 5, 6.0, 19000);
        assert!(infer(&mut tracks, &mut list, hold, SURGE, at(5)).is_empty(), "new window");

        // Readings too far apart (the PC slept) say nothing about how fast it was used.
        let mut tracks = Tracks::new();
        weekly(&mut list[0], 0, 10.0, 9000);
        infer(&mut tracks, &mut list, hold, SURGE, at(0));
        weekly(&mut list[0], 60, 30.0, 9000);
        assert!(infer(&mut tracks, &mut list, hold, SURGE, at(60)).is_empty(), "gap");

        // In use here or claimed by a live peer: not a surge. A stale peer claims nothing.
        for (active, live) in [(true, false), (false, true), (false, false)] {
            let mut tracks = Tracks::new();
            let mut list = vec![account("x", active)];
            list[0].remote = vec![RemoteSeat { machine: "P".into(), where_: vec![], seen_at: at(0), live }];
            weekly(&mut list[0], 0, 10.0, 9000);
            infer(&mut tracks, &mut list, hold, SURGE, at(0));
            weekly(&mut list[0], 5, 20.0, 9000);
            let surged = !infer(&mut tracks, &mut list, hold, SURGE, at(5)).is_empty();
            assert_eq!(surged, !active && !live, "active {active}, live peer {live}");
        }
    }

    #[test]
    fn inference_flags_a_rise_and_clears_after_two_intervals() {
        let hold = Duration::minutes(10) + Duration::seconds(30);
        let mut tracks = Tracks::new();
        let mut list = vec![account("x", false)];
        reading(&mut list[0], 0, 10.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(0));
        assert!(!list[0].used_outside, "one reading is not a rise");
        reading(&mut list[0], 5, 10.5, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(5));
        assert!(!list[0].used_outside, "under one point");
        reading(&mut list[0], 10, 12.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(10));
        assert!(list[0].used_outside);
        reading(&mut list[0], 15, 12.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(15));
        assert!(list[0].used_outside, "still within two intervals");
        reading(&mut list[0], 21, 12.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(21));
        assert!(!list[0].used_outside, "cleared after two intervals without a rise");
    }

    #[test]
    fn inference_ignores_what_it_cannot_attribute() {
        let hold = Duration::minutes(11);
        // A new window (different reset time) is not a rise.
        let mut tracks = Tracks::new();
        let mut list = vec![account("x", false)];
        reading(&mut list[0], 0, 80.0, 60);
        infer(&mut tracks, &mut list, hold, SURGE, at(0));
        reading(&mut list[0], 5, 90.0, 360);
        infer(&mut tracks, &mut list, hold, SURGE, at(5));
        assert!(!list[0].used_outside);

        // A stale or failed reading in between does not count as consecutive fresh readings.
        reading(&mut list[0], 10, 95.0, 360);
        list[0].stale = true;
        list[0].error = Some("http_500".into());
        infer(&mut tracks, &mut list, hold, SURGE, at(10));
        assert!(!list[0].used_outside);
        list[0].stale = false;
        list[0].error = None;
        reading(&mut list[0], 15, 99.0, 360);
        infer(&mut tracks, &mut list, hold, SURGE, at(15));
        assert!(!list[0].used_outside, "the reading before the failure is not compared");
        reading(&mut list[0], 20, 100.0, 360);
        infer(&mut tracks, &mut list, hold, SURGE, at(20));
        assert!(list[0].used_outside, "two fresh readings in a row again");

        // In use here, or on a live machine that says so: not "outside".
        for claimed in [true, false] {
            let mut tracks = Tracks::new();
            let mut list = vec![account("x", claimed)];
            if !claimed {
                list[0].remote = vec![RemoteSeat { machine: "P".into(), where_: vec![], seen_at: at(0), live: true }];
            }
            reading(&mut list[0], 0, 10.0, 300);
            infer(&mut tracks, &mut list, hold, SURGE, at(0));
            reading(&mut list[0], 5, 20.0, 300);
            infer(&mut tracks, &mut list, hold, SURGE, at(5));
            assert!(!list[0].used_outside);
        }

        // Usage from this PC just before switching away is not counted against the next reading.
        let mut tracks = Tracks::new();
        let mut list = vec![account("x", true)];
        reading(&mut list[0], 0, 10.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(0));
        list[0].active = false;
        list[0].sources.clear();
        reading(&mut list[0], 5, 20.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(5));
        assert!(!list[0].used_outside);

        // Not monitored: SlopBar only reads it when it is in use, so nothing to infer.
        let mut tracks = Tracks::new();
        let mut list = vec![account("x", false)];
        list[0].monitor_connected = false;
        reading(&mut list[0], 0, 10.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(0));
        reading(&mut list[0], 5, 20.0, 300);
        infer(&mut tracks, &mut list, hold, SURGE, at(5));
        assert!(!list[0].used_outside);
    }
}
