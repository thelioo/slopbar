//! The account roster: which accounts the user has on their other machines, so "Sign in all
//! accounts" on a new machine queues every one of them, with no backup to copy over.
//!
//! Opt-in (`Settings::account_roster`). Each SlopBar writes one file to the in-use sharing
//! location, `<folder>/SlopBar/roster/<machine id>.roster` (or the same path on the rclone
//! remote), listing the accounts signed in on it: provider, account id, email and plan. Never
//! sign-ins, tokens or anything else from the vault.
//!
//! Unlike presence records it names accounts, so it is encrypted as backups are (Argon2id and
//! XChaCha20-Poly1305, see `backup::seal_bytes`) with a roster passphrase the user sets once on
//! each machine and that is kept on it with DPAPI. Nothing the machines already share could be
//! the key instead: the folder is all they have in common, and whoever can read the folder
//! would have the key with it. A roster that does not open with this machine's passphrase is
//! counted (Health says so), never guessed at.
//!
//! Accounts another machine's roster names that are not signed in here are pending (see
//! `bulk::sign_in_plan`): one stops being pending once it is signed in here, and an account
//! removed everywhere else drops out with the last roster naming it.
//!
//! While "Sign in all accounts" runs on a machine, its roster also carries the run's progress
//! (`Run`), rewritten at every step, so the user's other computers can follow it and offer Retry
//! (Settings > Accounts, "Signing in on <machine>"). It names accounts too, so it goes inside the
//! same encrypted file, never beside it in the clear.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::backup::{OpenError, Passphrase, SealKey};
use crate::bulk::Pending;
use crate::model::AccountUsage;

pub const FORMAT: &str = "slopbar-roster";
const VERSION: u32 = 1;
/// Rosters live here, beside the presence records.
const SUBDIR: [&str; 2] = ["SlopBar", "roster"];
const REMOTE_DIR: &str = "SlopBar/roster";
const EXTENSION: &str = "roster";
/// Rosters are small; anything larger is not one.
const MAX_FILE: u64 = 256 * 1024;
const MAX_ACCOUNTS: usize = 500;
/// Emails, ids and plans longer than this are not kept (they come from other machines).
const FIELD_MAX: usize = 320;
/// A roster not written for this long is from a machine no longer in use and is ignored.
const IGNORE_AFTER_DAYS: i64 = 60;
/// This machine's roster is written when its accounts change, and at least this often.
const REWRITE_AFTER_HOURS: i64 = 24;
/// Other machines' rosters change only when their accounts do: read them at most this often.
const READ_EVERY_MINUTES: i64 = 10;
/// While a run goes on, this machine's roster is rewritten at least this often, so the others can
/// tell a run waiting on the person from a machine that went quiet.
const RUN_HEARTBEAT_MINUTES: i64 = 3;
/// While another machine's run goes on, rosters are read this often; with Settings watching
/// (`soon`), at least this often.
const READ_LIVE_SECONDS: i64 = 15;
const READ_SOON_SECONDS: i64 = 30;
/// After a Retry sent to another machine, its roster is read at the live pace for this long, or
/// until its run changes (see `Syncer::expect_news`).
const EXPECT_MINUTES: i64 = 2;
/// A run's steps beyond this many are not kept (they come from other machines).
const MAX_STEPS: usize = 200;
const PASSPHRASE_FILE: &str = "roster-passphrase.bin";
const MAX_PASSPHRASE: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub plan: Option<String>,
}

/// What one roster file holds once opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roster {
    pub v: u32,
    pub machine_id: String,
    pub machine: String,
    pub written_at: DateTime<Utc>,
    pub accounts: Vec<Entry>,
    /// "Sign in all accounts" as it runs there, or last ran since SlopBar started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Run>,
}

/// "Sign in all accounts" on the machine that wrote the roster, as Settings > Accounts shows it
/// there (see `app::BulkConnect`), for the user's other computers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Bumped at every change there; a lower one is older news.
    pub revision: u64,
    /// When it last changed, by that machine's clock.
    pub at: DateTime<Utc>,
    pub total: usize,
    pub done: usize,
    /// How many emails, and which one (1-based) is signing in now.
    pub emails: usize,
    pub email: usize,
    pub finished: bool,
    /// The open sign-in has waited long enough that the person there was asked whether to retry.
    pub still_waiting: bool,
    pub monitoring: bool,
    pub error: Option<String>,
    pub steps: Vec<RunStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    pub provider: Provider,
    /// Alias, else email: whom to sign in as.
    pub account: Option<String>,
    /// add or connect.
    pub kind: String,
    /// pending, running, done, skipped or failed.
    pub state: String,
    pub error: Option<String>,
}

/// This machine's roster: every account signed in here, field by field so nothing else from an
/// account can reach the file.
pub fn own(machine_id: &str, machine: &str, accounts: &[AccountUsage], now: DateTime<Utc>) -> Roster {
    let mut list: Vec<Entry> = accounts.iter()
        .map(|a| Entry { provider: a.provider, id: a.id.clone(), email: a.email.clone(), plan: a.plan.clone() })
        .collect();
    list.sort_by(|a, b| (a.provider.key(), &a.id).cmp(&(b.provider.key(), &b.id)));
    list.dedup_by(|a, b| a.provider == b.provider && a.id == b.id);
    Roster { v: VERSION, machine_id: machine_id.into(), machine: machine.into(), written_at: now, accounts: list, run: None }
}

/// The file name for a machine id: its hex digits and dashes only.
pub fn file_name(machine_id: &str) -> Option<String> {
    let safe: String = machine_id.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect();
    (!safe.is_empty()).then(|| format!("{safe}.{EXTENSION}"))
}

pub fn seal(roster: &Roster, key: &SealKey) -> Result<Vec<u8>, String> {
    let plain = serde_json::to_vec(roster).map_err(|_| "roster_unwritable")?;
    crate::backup::seal_bytes(FORMAT, &plain, key)
}

/// Opens a roster. `keys` keeps every key derived so far by salt, so each machine's roster costs
/// one Argon2id derivation per run, not one per read; it must be emptied when the passphrase
/// changes.
pub fn open(bytes: &[u8], passphrase: &Passphrase, keys: &mut HashMap<String, [u8; 32]>) -> Result<Roster, OpenError> {
    let plain = crate::backup::open_bytes(FORMAT, bytes, MAX_FILE, &mut |request| {
        let id = request.id();
        if let Some(key) = keys.get(&id) {
            return Ok(*key);
        }
        let key = request.derive(passphrase)?;
        keys.insert(id, key);
        Ok(key)
    })?;
    let mut roster: Roster = serde_json::from_slice(&plain).map_err(|_| OpenError::Unsupported)?;
    if roster.v != VERSION {
        return Err(OpenError::Unsupported);
    }
    let fits = |s: &String| s.len() <= FIELD_MAX && !s.chars().any(char::is_control);
    roster.accounts.retain(|e| fits(&e.id) && !e.id.is_empty() && e.email.as_ref().is_none_or(fits) && e.plan.as_ref().is_none_or(fits));
    roster.accounts.truncate(MAX_ACCOUNTS);
    roster.machine = roster.machine.chars().filter(|c| !c.is_control()).take(63).collect();
    if let Some(run) = roster.run.as_mut() {
        let one_of = |s: &String, words: &[&str]| words.contains(&s.as_str());
        let code = |s: &String| s.len() <= 200 && !s.chars().any(char::is_control);
        run.steps.retain(|s| one_of(&s.kind, &["add", "connect"]) && one_of(&s.state, &["pending", "running", "done", "skipped", "failed"])
            && s.account.as_ref().is_none_or(fits) && s.error.as_ref().is_none_or(code));
        run.steps.truncate(MAX_STEPS);
        if run.error.as_ref().is_some_and(|e| !code(e)) {
            run.error = Some("unknown".into());
        }
    }
    Ok(roster)
}

// ---------------------------------------------------------------------------------------------
// Storage: the same place presence records go (see `presence::store`).

pub trait RosterStore: Send + Sync {
    /// Every roster file there: its name, bytes and modification time (when known).
    fn list(&self) -> std::io::Result<Vec<(String, Vec<u8>, Option<DateTime<Utc>>)>>;
    /// Replaces the file `name`, atomically.
    fn put(&self, name: &str, bytes: &[u8]) -> std::io::Result<()>;
    /// Deletes the file `name`; one already gone is fine.
    fn remove(&self, name: &str) -> std::io::Result<()>;
}

/// Where the settings share in-use status, whether or not the roster is on (turning it off
/// removes this machine's roster from there).
pub fn store(settings: &crate::settings::Settings) -> Option<Box<dyn RosterStore>> {
    if let Some(remote) = &settings.presence_remote {
        return crate::rclone::Rclone::open(remote).map(|s| Box::new(s) as Box<dyn RosterStore>);
    }
    settings.presence_folder.as_ref().map(|f| Box::new(Folder(PathBuf::from(f))) as Box<dyn RosterStore>)
}

/// The synced folder the user chose for in-use sharing.
pub struct Folder(pub PathBuf);

impl Folder {
    fn dir(&self) -> PathBuf {
        SUBDIR.iter().fold(self.0.clone(), |p, s| p.join(s))
    }
}

impl RosterStore for Folder {
    fn list(&self) -> std::io::Result<Vec<(String, Vec<u8>, Option<DateTime<Utc>>)>> {
        let entries = match std::fs::read_dir(self.dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        Ok(entries.flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some(EXTENSION) {
                    return None;
                }
                let meta = e.metadata().ok().filter(|m| m.is_file() && m.len() <= MAX_FILE)?;
                Some((e.file_name().to_string_lossy().into_owned(), std::fs::read(&path).ok()?, meta.modified().ok().map(DateTime::<Utc>::from)))
            })
            .collect())
    }

    fn put(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        if !self.0.is_dir() {
            return Err(std::io::Error::other("sharing folder missing"));
        }
        crate::private_store::write(&self.dir().join(name), bytes)
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.dir().join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

impl<R: crate::rclone::Runner> RosterStore for crate::rclone::Rclone<R> {
    fn list(&self) -> std::io::Result<Vec<(String, Vec<u8>, Option<DateTime<Utc>>)>> {
        self.files(REMOTE_DIR, EXTENSION, MAX_FILE)
    }

    fn put(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        crate::rclone::Rclone::put(self, REMOTE_DIR, name, bytes)
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        self.delete(REMOTE_DIR, name)
    }
}

/// Removes this machine's roster: the roster was turned off, or sharing moved elsewhere.
pub fn withdraw(store: &dyn RosterStore) -> std::io::Result<()> {
    match crate::presence::identity().and_then(|(id, _)| file_name(id)) {
        Some(name) => store.remove(&name),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------
// The passphrase, kept on this machine with DPAPI.

pub fn passphrase_path(dir: &Path) -> PathBuf { dir.join(PASSPHRASE_FILE) }

pub fn load_passphrase(dir: &Path) -> Option<Passphrase> {
    let bytes = crate::private_store::load_secret(&passphrase_path(dir)).ok()?;
    String::from_utf8(bytes).ok().map(Passphrase).filter(valid_passphrase)
}

fn valid_passphrase(p: &Passphrase) -> bool {
    (crate::backup::MIN_PASSPHRASE..=MAX_PASSPHRASE).contains(&p.0.chars().count())
}

/// Saves the passphrase with DPAPI, or deletes it for None.
pub fn store_passphrase(dir: &Path, passphrase: Option<&Passphrase>) -> Result<(), String> {
    match passphrase {
        None => match std::fs::remove_file(passphrase_path(dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err("roster_passphrase_storage".into()),
            _ => Ok(()),
        },
        Some(p) if !valid_passphrase(p) => Err("roster_passphrase_too_short".into()),
        Some(p) => crate::private_store::save_secret(&passphrase_path(dir), p.0.as_bytes()).map_err(|_| "roster_passphrase_storage".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Writing and reading, once per refresh

/// Another machine's roster.
#[derive(Debug, Clone, Serialize)]
pub struct Peer {
    pub machine_id: String,
    pub machine: String,
    pub written_at: DateTime<Utc>,
    pub accounts: Vec<Entry>,
    pub run: Option<Run>,
}

impl Peer {
    /// Its run is still going, by its last word.
    pub fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.finished)
    }
}

/// What the last write and read found, for Settings, Health and the door.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Seen {
    /// Whether the last write of this machine's roster worked, and when one last did.
    pub wrote: Option<bool>,
    pub written_at: Option<DateTime<Utc>>,
    /// Whether the last read worked, and when it was.
    pub read: Option<bool>,
    pub read_at: Option<DateTime<Utc>>,
    /// This machine's roster as read back from the sharing location: how many accounts it names.
    pub own_accounts: Option<usize>,
    /// Newest first.
    pub peers: Vec<Peer>,
    /// Rosters that did not open with this machine's passphrase (or were changed).
    pub unreadable: usize,
    /// Rosters from a newer SlopBar.
    pub unsupported: usize,
}

/// Kept between refreshes: derived keys, and what was written last.
#[derive(Default)]
pub struct Syncer {
    keys: HashMap<String, [u8; 32]>,
    seal: Option<SealKey>,
    written: Option<(Vec<Entry>, DateTime<Utc>)>,
    /// This machine's run, newest revision (see `offer_run`), and the one last written.
    run: Option<Run>,
    written_run: Option<Run>,
    /// `run` was taken back from this machine's own roster after a restart; any new run replaces
    /// it, whatever its revision (revisions start again with the app).
    adopted: bool,
    /// Whether the first sync since the app started has run.
    started: bool,
    /// Machines sent a Retry: until when their news is expected, and their run's time then.
    expecting: HashMap<String, (DateTime<Utc>, Option<DateTime<Utc>>)>,
    pub seen: Seen,
}

impl Syncer {
    /// Forgets everything that depends on the passphrase or the location: after either changes,
    /// or the roster is turned off.
    pub fn reset(&mut self) {
        *self = Syncer::default();
    }

    /// Takes this machine's run as it is now, unless a newer revision is already in hand (syncs
    /// started one after another can finish in any order).
    pub fn offer_run(&mut self, run: Run) {
        if self.adopted || self.run.as_ref().is_none_or(|r| r.revision < run.revision) {
            self.run = Some(run);
            self.adopted = false;
        }
    }

    /// A Retry was sent to `machine_id`: its roster is read at the live pace until its run changes
    /// or `EXPECT_MINUTES` pass, so the new run shows within seconds of its upload.
    pub fn expect_news(&mut self, machine_id: &str, now: DateTime<Utc>) {
        let at = self.seen.peers.iter().find(|p| p.machine_id == machine_id).and_then(|p| p.run.as_ref()).map(|r| r.at);
        self.expecting.insert(machine_id.into(), (now + Duration::minutes(EXPECT_MINUTES), at));
    }

    /// One refresh's turn: writes this machine's roster when its accounts or its run changed (or a
    /// day passed, or a few minutes while its run goes on), then reads every roster when due, or
    /// at once with `force`. With `soon`, Settings is watching, so rosters are read more often.
    #[allow(clippy::too_many_arguments)]
    pub fn sync(&mut self, store: &dyn RosterStore, passphrase: &Passphrase, me: (&str, &str), accounts: &[AccountUsage], now: DateTime<Utc>, force: bool, soon: bool) {
        let Some(name) = file_name(me.0) else { return };
        // After a restart, this machine's last failed run is taken back from its own roster before
        // the first write would replace it, so the other computers keep its Retry for the hour.
        let read_first = !self.started && self.run.is_none();
        self.started = true;
        if read_first {
            let recent = |r: &Run| r.finished && r.error.is_some() && now - r.at.min(now) < Duration::minutes(SHOW_FINISHED_MINUTES);
            if let Some(run) = self.read_all(store, passphrase, me.0, &name, now).filter(recent) {
                self.written_run = Some(run.clone());
                self.run = Some(run);
                self.adopted = true;
            }
        }
        let mut mine = own(me.0, me.1, accounts, now);
        mine.run = self.run.clone();
        let accounts_due = force || self.written.as_ref().is_none_or(|(list, at)| *list != mine.accounts || now - *at >= Duration::hours(REWRITE_AFTER_HOURS));
        let run_due = mine.run != self.written_run
            || mine.run.as_ref().is_some_and(|r| !r.finished) && self.seen.written_at.is_none_or(|at| now - at >= Duration::minutes(RUN_HEARTBEAT_MINUTES));
        let mut wrote = false;
        if accounts_due || run_due {
            if self.seal.is_none() {
                self.seal = SealKey::new(passphrase).ok();
            }
            let ok = self.seal.as_ref().and_then(|key| seal(&mine, key).ok()).is_some_and(|bytes| store.put(&name, &bytes).is_ok());
            self.seen.wrote = Some(ok);
            if ok {
                self.written = Some((mine.accounts.clone(), now));
                self.written_run = mine.run.clone();
                self.seen.written_at = Some(now);
                // A change of accounts is worth reading back; a step of a run is not.
                wrote = accounts_due;
            }
        }
        // A machine sent a Retry stays live until its run changes or the wait runs out.
        let peers = &self.seen.peers;
        self.expecting.retain(|id, (until, at)| {
            now < *until && peers.iter().find(|p| &p.machine_id == id).and_then(|p| p.run.as_ref()).map(|r| r.at) == *at
        });
        let every = if self.seen.peers.iter().any(Peer::running) || !self.expecting.is_empty() {
            Duration::seconds(READ_LIVE_SECONDS)
        } else if soon {
            Duration::seconds(READ_SOON_SECONDS)
        } else {
            Duration::minutes(READ_EVERY_MINUTES)
        };
        let read_due = self.seen.read_at.is_none_or(|t| now - t >= every);
        // Just read before the write: not again (what was written is what a read-back would find).
        if read_first {
            if wrote {
                self.seen.own_accounts = self.written.as_ref().map(|(list, _)| list.len());
            }
            return;
        }
        if !(force || wrote || read_due) {
            return;
        }
        self.read_all(store, passphrase, me.0, &name, now);
    }

    /// Reads every roster; returns the run this machine's own roster carries.
    fn read_all(&mut self, store: &dyn RosterStore, passphrase: &Passphrase, own_id: &str, own_name: &str, now: DateTime<Utc>) -> Option<Run> {
        self.seen.read_at = Some(now);
        match store.list() {
            Err(_) => { self.seen.read = Some(false); None }
            Ok(files) => {
                self.seen.read = Some(true);
                self.read(files, passphrase, own_id, own_name, now)
            }
        }
    }

    fn read(&mut self, files: Vec<(String, Vec<u8>, Option<DateTime<Utc>>)>, passphrase: &Passphrase, own_id: &str, own_name: &str, now: DateTime<Utc>) -> Option<Run> {
        let (mut unreadable, mut unsupported, mut own_accounts, mut own_run) = (0, 0, None, None);
        let mut peers: HashMap<String, Peer> = HashMap::new();
        for (name, bytes, modified) in files {
            let roster = match open(&bytes, passphrase, &mut self.keys) {
                Ok(r) => r,
                Err(OpenError::WrongOrDamaged) => { unreadable += 1; continue; }
                Err(_) => { unsupported += 1; continue; }
            };
            // A clock running ahead is not trusted: the storage's time is used instead.
            let written_at = if roster.written_at > now + Duration::minutes(5) { modified.unwrap_or(now).min(now) } else { roster.written_at };
            if roster.machine_id == own_id || name == own_name {
                own_accounts = Some(roster.accounts.len());
                own_run = roster.run;
                continue;
            }
            if now - written_at > Duration::days(IGNORE_AFTER_DAYS) {
                continue;
            }
            let peer = Peer { machine_id: roster.machine_id.clone(), machine: roster.machine, written_at, accounts: roster.accounts, run: roster.run };
            match peers.get(&roster.machine_id) {
                Some(p) if p.written_at >= peer.written_at => {}
                _ => { peers.insert(roster.machine_id, peer); }
            }
        }
        let mut peers: Vec<Peer> = peers.into_values().collect();
        peers.sort_by(|a, b| b.written_at.cmp(&a.written_at).then_with(|| a.machine.cmp(&b.machine)));
        self.seen = Seen { own_accounts, peers, unreadable, unsupported, ..self.seen.clone() };
        own_run
    }
}

/// A run going on whose machine has not rewritten its roster for this long has gone quiet (its
/// SlopBar stopped, or lost the sharing location); one rewrites every few refreshes at most.
const QUIET_MINUTES: i64 = 20;
/// A finished run is shown for this long after its last change.
const SHOW_FINISHED_MINUTES: i64 = 60;

/// Another machine's run as Settings > Accounts shows it: "Signing in on <machine>".
#[derive(Debug, Clone, Serialize)]
pub struct PeerRun {
    pub machine_id: String,
    pub machine: String,
    pub written_at: DateTime<Utc>,
    pub run: Run,
    /// Going on, but nothing heard from its machine for a while.
    pub quiet: bool,
}

/// The other machines' runs worth showing: going on, or finished within the hour.
pub fn peer_runs(seen: &Seen, now: DateTime<Utc>) -> Vec<PeerRun> {
    seen.peers.iter()
        .filter_map(|p| {
            let run = p.run.clone()?;
            // A clock running ahead counts as now.
            let at = run.at.min(now);
            (!run.finished || now - at < Duration::minutes(SHOW_FINISHED_MINUTES)).then(|| PeerRun {
                machine_id: p.machine_id.clone(), machine: p.machine.clone(), written_at: p.written_at,
                quiet: !run.finished && now - p.written_at > Duration::minutes(QUIET_MINUTES), run,
            })
        })
        .collect()
}

/// The accounts other machines' rosters name that are not signed in here: once each, newest
/// roster first, with the first email any of them gives.
pub fn waiting(seen: &Seen, here: &[AccountUsage]) -> Vec<Pending> {
    let mut out: Vec<Pending> = Vec::new();
    for e in seen.peers.iter().flat_map(|p| &p.accounts) {
        if here.iter().any(|a| a.provider == e.provider && a.id == e.id) {
            continue;
        }
        match out.iter_mut().find(|p| p.provider == e.provider && p.id == e.id) {
            Some(p) => { if p.email.is_none() { p.email = e.email.clone(); } }
            None => out.push(Pending { provider: e.provider, id: e.id.clone(), email: e.email.clone() }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    const MACHINE_A: &str = "11111111-1111-4111-8111-111111111111";
    const MACHINE_B: &str = "22222222-2222-4222-8222-222222222222";

    fn pass(s: &str) -> Passphrase { Passphrase(s.into()) }

    fn account(provider: &str, id: &str) -> AccountUsage {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": provider, "email": format!("{id}@example.invalid"), "alias": "Synthetic Alias", "plan": "max",
            "org": "Synthetic Org", "active": false, "sources": [], "windows": [], "credits": null, "resets": [], "error": null,
            "stale": false, "fetched_at": null, "monitor_connected": true,
        })).unwrap()
    }

    fn now() -> DateTime<Utc> { "2026-10-23T12:00:00Z".parse().unwrap() }

    #[test]
    fn a_roster_holds_only_provider_id_email_and_plan_encrypted() {
        let roster = own(MACHINE_A, "DESK-1", &[account("codex", "cx-one"), account("claude", "cl-one"), account("claude", "cl-one")], now());
        assert_eq!(roster.accounts.iter().map(|e| (e.provider.key(), e.id.as_str())).collect::<Vec<_>>(), [("claude", "cl-one"), ("codex", "cx-one")]);
        let json = serde_json::to_value(&roster.accounts[0]).unwrap();
        assert_eq!(json, serde_json::json!({ "provider": "claude", "id": "cl-one", "email": "cl-one@example.invalid", "plan": "max" }));
        let key = SealKey::new(&pass("correct horse battery")).unwrap();
        let bytes = seal(&roster, &key).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for secret in ["cl-one", "example.invalid", "DESK-1", "Synthetic", "max"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
        let mut keys = HashMap::new();
        let back = open(&bytes, &pass("correct horse battery"), &mut keys).unwrap();
        assert_eq!(back.accounts, roster.accounts);
        assert_eq!(back.machine, "DESK-1");
        assert_eq!(keys.len(), 1, "the key is kept for the next read");
        assert!(matches!(open(&bytes, &pass("wrong horse battery"), &mut HashMap::new()), Err(OpenError::WrongOrDamaged)));
        let mut changed = bytes.clone();
        let at = text.find("\"ciphertext\": \"").unwrap() + 20;
        changed[at] = if changed[at] == b'A' { b'B' } else { b'A' };
        assert!(matches!(open(&changed, &pass("correct horse battery"), &mut HashMap::new()), Err(OpenError::WrongOrDamaged)));
        assert!(matches!(open(b"{\"format\":\"slopbar-backup\"}", &pass("correct horse battery"), &mut HashMap::new()), Err(OpenError::NotOurs)));
    }

    #[test]
    fn a_new_machine_finds_every_account_its_other_machines_have() {
        let shared = TempDir::new();
        let store = Folder(shared.0.clone());
        let p = pass("synthetic roster passphrase");
        // The old machine has three accounts; the new one has signed in one of them.
        let mut old = Syncer::default();
        old.sync(&store, &p, (MACHINE_A, "OLD-PC"), &[account("claude", "cl-one"), account("codex", "cx-one"), account("claude", "cl-two")], now(), false, false);
        assert_eq!(old.seen.wrote, Some(true));
        assert_eq!(old.seen.own_accounts, Some(3), "its own roster reads back");
        assert!(shared.0.join("SlopBar").join("roster").join(format!("{MACHINE_A}.roster")).is_file());
        let here = [account("claude", "cl-two")];
        let mut new = Syncer::default();
        new.sync(&store, &p, (MACHINE_B, "NEW-PC"), &here, now(), false, false);
        assert_eq!(new.seen.peers.iter().map(|p| p.machine.as_str()).collect::<Vec<_>>(), ["OLD-PC"]);
        let pending = waiting(&new.seen, &here);
        assert_eq!(pending.iter().map(|p| (p.provider.key(), p.id.as_str(), p.email.as_deref())).collect::<Vec<_>>(),
            [("claude", "cl-one", Some("cl-one@example.invalid")), ("codex", "cx-one", Some("cx-one@example.invalid"))],
            "signed-in accounts are not pending");
        // Its plan shows them as Add steps, one email at a time.
        let snapshot = crate::model::Snapshot { accounts: here.to_vec(), updated_at: None };
        let plan = crate::bulk::sign_in_plan(&snapshot, &pending, false);
        assert_eq!(plan.iter().map(|g| g.steps.len()).sum::<usize>(), 2);
        // Once signed in here, nothing is waiting.
        let all = [account("claude", "cl-one"), account("codex", "cx-one"), account("claude", "cl-two")];
        assert!(waiting(&new.seen, &all).is_empty());
    }

    #[test]
    fn another_passphrase_is_counted_and_old_rosters_are_ignored() {
        let shared = TempDir::new();
        let store = Folder(shared.0.clone());
        let mut a = Syncer::default();
        a.sync(&store, &pass("synthetic passphrase one"), (MACHINE_A, "A"), &[account("claude", "cl-one")], now(), false, false);
        let mut b = Syncer::default();
        b.sync(&store, &pass("synthetic passphrase two"), (MACHINE_B, "B"), &[], now(), false, false);
        assert!(b.seen.peers.is_empty());
        assert_eq!(b.seen.unreadable, 1, "a roster sealed with another passphrase is counted, not read");
        let mut late = Syncer::default();
        late.sync(&store, &pass("synthetic passphrase one"), (MACHINE_B, "B"), &[], now() + Duration::days(IGNORE_AFTER_DAYS + 1), false, false);
        assert!(late.seen.peers.is_empty(), "a roster unwritten for two months is from a machine no longer used");
    }

    /// Counts the writes, to show the roster is written only when it changes.
    struct Counting(Folder, std::sync::atomic::AtomicUsize);

    impl RosterStore for Counting {
        fn list(&self) -> std::io::Result<Vec<(String, Vec<u8>, Option<DateTime<Utc>>)>> { self.0.list() }
        fn put(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.0.put(name, bytes)
        }
        fn remove(&self, name: &str) -> std::io::Result<()> { self.0.remove(name) }
    }

    #[test]
    fn writes_when_accounts_change_or_a_day_passes_and_withdraws() {
        let shared = TempDir::new();
        let store = Counting(Folder(shared.0.clone()), Default::default());
        let writes = || store.1.load(std::sync::atomic::Ordering::SeqCst);
        let p = pass("synthetic roster passphrase");
        let mut sync = Syncer::default();
        let one = [account("claude", "cl-one")];
        sync.sync(&store, &p, (MACHINE_A, "A"), &one, now(), false, false);
        sync.sync(&store, &p, (MACHINE_A, "A"), &one, now() + Duration::minutes(5), false, false);
        assert_eq!(writes(), 1, "unchanged: not written again");
        sync.sync(&store, &p, (MACHINE_A, "A"), &[account("claude", "cl-one"), account("codex", "cx-one")], now() + Duration::minutes(10), false, false);
        assert_eq!(writes(), 2);
        assert_eq!(sync.seen.own_accounts, Some(2), "read again after a write");
        sync.sync(&store, &p, (MACHINE_A, "A"), &[account("claude", "cl-one"), account("codex", "cx-one")], now() + Duration::hours(25), false, false);
        assert_eq!(writes(), 3, "rewritten daily, so it never looks abandoned");
        store.remove(&file_name(MACHINE_A).unwrap()).unwrap();
        store.remove(&file_name(MACHINE_A).unwrap()).unwrap();
        assert!(store.list().unwrap().is_empty(), "withdrawn, and withdrawing twice is fine");
    }

    #[test]
    fn the_passphrase_is_kept_here_and_never_too_short() {
        let dir = TempDir::new();
        assert!(load_passphrase(&dir.0).is_none());
        assert_eq!(store_passphrase(&dir.0, Some(&pass("short"))), Err("roster_passphrase_too_short".to_string()));
        store_passphrase(&dir.0, Some(&pass("synthetic roster passphrase"))).unwrap();
        assert_eq!(load_passphrase(&dir.0).map(|p| p.0), Some("synthetic roster passphrase".into()));
        #[cfg(windows)]
        assert!(!String::from_utf8_lossy(&std::fs::read(passphrase_path(&dir.0)).unwrap()).contains("synthetic"), "DPAPI, not plaintext");
        store_passphrase(&dir.0, None).unwrap();
        store_passphrase(&dir.0, None).unwrap();
        assert!(load_passphrase(&dir.0).is_none());
    }

    fn run(revision: u64, finished: bool, state: &str) -> Run {
        Run { revision, at: now(), total: 2, done: 0, emails: 1, email: 1, finished, still_waiting: false, monitoring: true,
            error: finished.then(|| "signin_stopped".into()),
            steps: vec![
                RunStep { provider: Provider::Claude, account: Some("cl-one@example.invalid".into()), kind: "add".into(), state: state.into(), error: None },
                RunStep { provider: Provider::Codex, account: Some("cl-one@example.invalid".into()), kind: "connect".into(), state: "pending".into(), error: None },
            ] }
    }

    #[test]
    fn a_run_reaches_the_other_machines_encrypted_newest_first() {
        let shared = TempDir::new();
        let store = Counting(Folder(shared.0.clone()), Default::default());
        let writes = || store.1.load(std::sync::atomic::Ordering::SeqCst);
        let p = pass("synthetic roster passphrase");
        let one = [account("claude", "cl-two")];
        let mut target = Syncer::default();
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, now(), false, false);
        assert_eq!(writes(), 1);
        // A step changes: written at once, though the accounts did not change.
        target.offer_run(run(3, false, "running"));
        target.offer_run(run(2, false, "pending"));
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, now() + Duration::seconds(5), false, false);
        assert_eq!(writes(), 2, "an older revision offered later is ignored, the newer one written");
        let bytes = std::fs::read(shared.0.join("SlopBar").join("roster").join(format!("{MACHINE_A}.roster"))).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("example.invalid"), "the run is inside the encrypted roster");
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, now() + Duration::seconds(10), false, false);
        assert_eq!(writes(), 2, "unchanged: not written again");
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, now() + Duration::minutes(RUN_HEARTBEAT_MINUTES) + Duration::seconds(6), false, false);
        assert_eq!(writes(), 3, "rewritten every few minutes while it runs, so it never looks quiet");
        // The pilot sees it, and reads again soon while it runs.
        let mut pilot = Syncer::default();
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, now() + Duration::minutes(4), false, false);
        let peer = &pilot.seen.peers[0];
        assert_eq!((peer.machine_id.as_str(), peer.machine.as_str()), (MACHINE_A, "TARGET"));
        let seen = peer.run.clone().unwrap();
        assert_eq!((seen.revision, seen.steps[0].state.as_str(), seen.steps[0].account.as_deref()), (3, "running", Some("cl-one@example.invalid")));
        target.offer_run(run(4, true, "failed"));
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, now() + Duration::minutes(4), false, false);
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, now() + Duration::minutes(4) + Duration::seconds(READ_LIVE_SECONDS), false, false);
        let seen = pilot.seen.peers[0].run.clone().unwrap();
        assert_eq!((seen.finished, seen.error.as_deref()), (true, Some("signin_stopped")), "read again within seconds while a run went on");
        // Finished: back to the slow pace, unless Settings is watching.
        let read_at = pilot.seen.read_at;
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, now() + Duration::minutes(5), false, false);
        assert_eq!(pilot.seen.read_at, read_at);
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, now() + Duration::minutes(5), false, true);
        assert_ne!(pilot.seen.read_at, read_at);
        // Shown for an hour after it finished, then no more.
        assert_eq!(peer_runs(&pilot.seen, now() + Duration::minutes(5)).len(), 1);
        assert!(peer_runs(&pilot.seen, now() + Duration::minutes(SHOW_FINISHED_MINUTES)).is_empty());
    }

    #[test]
    fn after_a_retry_the_machine_is_read_at_the_live_pace_until_its_run_changes() {
        let shared = TempDir::new();
        let store = Folder(shared.0.clone());
        let p = pass("synthetic roster passphrase");
        let one = [account("claude", "cl-two")];
        let at = |s: i64| now() + Duration::seconds(s);
        let mut target = Syncer::default();
        target.offer_run(run(4, true, "failed"));
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, at(0), false, false);
        let mut pilot = Syncer::default();
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(60), false, false);
        // Nothing going on: the next read waits, even with Settings watching.
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(60 + READ_LIVE_SECONDS), false, true);
        assert_eq!(pilot.seen.read_at, Some(at(60)));
        // Retry sent (the target has not uploaded its new run yet): read again within seconds.
        pilot.expect_news(MACHINE_A, at(62));
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(60 + READ_LIVE_SECONDS), false, false);
        assert_eq!(pilot.seen.read_at, Some(at(60 + READ_LIVE_SECONDS)), "live pace, Settings closed");
        target.offer_run(Run { at: at(80), ..run(5, false, "running") });
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, at(80), false, false);
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(60 + 2 * READ_LIVE_SECONDS), false, false);
        assert_eq!(pilot.seen.peers[0].run.as_ref().map(|r| (r.revision, r.finished)), Some((5, false)));
        target.offer_run(Run { at: at(95), ..run(6, true, "failed") });
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, at(95), false, false);
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(60 + 3 * READ_LIVE_SECONDS), false, false);
        // The news came: back to the slow pace.
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(60 + 4 * READ_LIVE_SECONDS), false, false);
        assert_eq!(pilot.seen.read_at, Some(at(60 + 3 * READ_LIVE_SECONDS)));
        // No news at all: the wait runs out.
        let mut other = Syncer::default();
        other.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(200), false, false);
        other.expect_news(MACHINE_A, at(200));
        other.sync(&store, &p, (MACHINE_B, "PILOT"), &one, at(200 + EXPECT_MINUTES * 60), false, false);
        assert_eq!(other.seen.read_at, Some(at(200)));
    }

    #[test]
    fn a_failed_run_outlives_a_restart_of_its_app_for_the_hour() {
        let p = pass("synthetic roster passphrase");
        let one = [account("claude", "cl-two")];
        let after = |m: i64| now() + Duration::minutes(m);
        let shared = TempDir::new();
        let store = Folder(shared.0.clone());
        let mut target = Syncer::default();
        target.offer_run(run(4, true, "failed"));
        target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, after(0), false, false);
        // Restarted: its first write keeps the run, so the pilot keeps its Retry.
        let mut restarted = Syncer::default();
        restarted.sync(&store, &p, (MACHINE_A, "TARGET"), &one, after(10), false, false);
        assert_eq!(restarted.seen.own_accounts, Some(1));
        let mut pilot = Syncer::default();
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, after(11), false, false);
        let shown = peer_runs(&pilot.seen, after(11));
        assert_eq!(shown.iter().map(|r| (r.run.revision, r.run.error.as_deref())).collect::<Vec<_>>(), [(4, Some("signin_stopped"))]);
        // Its next run starts again at revision 1, and replaces it.
        restarted.offer_run(run(1, false, "running"));
        restarted.sync(&store, &p, (MACHINE_A, "TARGET"), &one, after(12), false, false);
        pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, after(12), true, false);
        assert_eq!(pilot.seen.peers[0].run.as_ref().map(|r| r.revision), Some(1));
        // A run older than the hour, or one that went well, is not taken back.
        for (old, minutes) in [(run(4, true, "failed"), SHOW_FINISHED_MINUTES), (Run { error: None, ..run(4, true, "done") }, 10)] {
            let shared = TempDir::new();
            let store = Folder(shared.0.clone());
            let mut target = Syncer::default();
            target.offer_run(old);
            target.sync(&store, &p, (MACHINE_A, "TARGET"), &one, after(0), false, false);
            let mut restarted = Syncer::default();
            restarted.sync(&store, &p, (MACHINE_A, "TARGET"), &one, after(minutes), false, false);
            let mut pilot = Syncer::default();
            pilot.sync(&store, &p, (MACHINE_B, "PILOT"), &one, after(minutes), false, false);
            assert!(pilot.seen.peers[0].run.is_none(), "{minutes}");
        }
    }

    #[test]
    fn a_run_whose_machine_went_quiet_says_so() {
        let peer = |written: i64, finished: bool| Peer { machine_id: MACHINE_A.into(), machine: "TARGET".into(),
            written_at: now() - Duration::minutes(written), accounts: vec![], run: Some(run(1, finished, "running")) };
        let seen = |p: Peer| Seen { peers: vec![p], ..Seen::default() };
        assert!(!peer_runs(&seen(peer(1, false)), now())[0].quiet);
        assert!(peer_runs(&seen(peer(QUIET_MINUTES + 1, false)), now())[0].quiet, "no word for a while");
        assert!(peer_runs(&seen(Peer { run: None, ..peer(1, false) }), now()).is_empty(), "no run, no card");
    }

    #[test]
    fn a_run_from_another_machine_is_trimmed_to_known_words() {
        let mut roster = own(MACHINE_A, "A", &[], now());
        let mut r = run(1, false, "running");
        r.steps.push(RunStep { provider: Provider::Claude, account: None, kind: "delete".into(), state: "running".into(), error: None });
        r.steps.push(RunStep { provider: Provider::Claude, account: None, kind: "add".into(), state: "hacked".into(), error: None });
        r.error = Some("bad\nerror".into());
        roster.run = Some(r);
        let key = SealKey::new(&pass("synthetic roster passphrase")).unwrap();
        let back = open(&seal(&roster, &key).unwrap(), &pass("synthetic roster passphrase"), &mut HashMap::new()).unwrap().run.unwrap();
        assert_eq!(back.steps.len(), 2);
        assert_eq!(back.error.as_deref(), Some("unknown"));
        // A roster from a SlopBar without runs still opens.
        let mut plain: serde_json::Value = serde_json::to_value(own(MACHINE_A, "A", &[], now())).unwrap();
        plain.as_object_mut().unwrap().remove("run");
        let bytes = crate::backup::seal_bytes(FORMAT, &serde_json::to_vec(&plain).unwrap(), &key).unwrap();
        assert!(open(&bytes, &pass("synthetic roster passphrase"), &mut HashMap::new()).unwrap().run.is_none());
    }

    #[test]
    fn rosters_from_others_are_trimmed_to_sane_fields() {
        let mut roster = own(MACHINE_A, "A\u{7}PC", &[account("claude", "cl-one")], now());
        roster.accounts.push(Entry { provider: Provider::Claude, id: "x".repeat(FIELD_MAX + 1), email: None, plan: None });
        roster.accounts.push(Entry { provider: Provider::Codex, id: "cx\nbad".into(), email: None, plan: None });
        let key = SealKey::new(&pass("synthetic roster passphrase")).unwrap();
        let back = open(&seal(&roster, &key).unwrap(), &pass("synthetic roster passphrase"), &mut HashMap::new()).unwrap();
        assert_eq!(back.accounts.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["cl-one"]);
        assert_eq!(back.machine, "APC");
    }
}
