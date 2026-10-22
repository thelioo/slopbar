//! Presence records on any cloud storage rclone can reach, with no app registration: the user
//! connects their cloud once with `rclone config`, and rclone keeps its own config and tokens.
//! SlopBar only runs rclone (no window, a hard timeout per call) on `<remote>:SlopBar/presence`,
//! never passes or reads credentials, and never prints rclone's output.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::presence::{Presence, PresenceStore};

/// No rclone call may take longer than this.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Records live here on the remote.
const DIR: &str = "SlopBar/presence";
/// Presence records are small; anything larger is not one.
const MAX_RECORD: u64 = 256 * 1024;
/// rclone's exit code for "directory not found".
const DIR_NOT_FOUND: i32 = 3;

/// What one rclone run produced.
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
}

/// Runs rclone with these arguments (and optional stdin). Tests substitute a fake.
pub trait Runner: Send + Sync {
    fn run(&self, args: &[&str], input: Option<&[u8]>) -> std::io::Result<Ran>;
}

/// The real rclone executable.
pub struct Cli(pub PathBuf);

impl Runner for Cli {
    fn run(&self, args: &[&str], input: Option<&[u8]>) -> std::io::Result<Ran> {
        use std::process::Stdio;
        let mut cmd = crate::sources::command(&self.0.to_string_lossy());
        // Never wait on a prompt for an encrypted config's password, and give up on dead networks
        // well inside the timeout.
        cmd.args(args).args(["--ask-password=false", "--contimeout", "10s", "--timeout", "15s"]);
        cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
        // rclone's log can name paths and remotes; it is never kept or shown.
        cmd.stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            let bytes = bytes.to_vec();
            std::thread::spawn(move || {
                let _ = stdin.write_all(&bytes);
            });
        }
        let mut stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let reader = std::thread::spawn(move || {
            let mut out = vec![];
            let _ = stdout.read_to_end(&mut out);
            out
        });
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(Ran { code: status.code(), stdout: reader.join().unwrap_or_default() });
            }
            if started.elapsed() > TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "rclone timed out"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// rclone on PATH, or where WinGet links it.
pub fn find() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "rclone.exe" } else { "rclone" };
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect::<Vec<_>>())
        .unwrap_or_default();
    let winget = dirs::data_local_dir().map(|d| d.join("Microsoft").join("WinGet").join("Links").join(exe));
    on_path.into_iter().chain(winget).find(|p| p.is_file())
}

/// A remote the user configured in rclone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Remote {
    /// The remote's name, without the colon.
    pub name: String,
    /// rclone's backend type ("dropbox", "drive", "onedrive"…).
    pub kind: String,
    /// The cloud's name, as Settings shows it.
    pub provider: String,
}

/// The user's rclone remotes. `listremotes --long` prints only names and types, so no
/// credential is ever read (unlike `config dump`).
pub fn remotes(runner: &dyn Runner) -> Vec<Remote> {
    let Ok(ran) = runner.run(&["listremotes", "--long"], None) else { return vec![] };
    if ran.code != Some(0) {
        return vec![];
    }
    String::from_utf8_lossy(&ran.stdout)
        .lines()
        .filter_map(|line| {
            let (name, kind) = line.trim().split_once(':')?;
            let kind = kind.trim().to_string();
            valid_remote(name).then(|| Remote { name: name.into(), provider: provider_name(&kind), kind })
        })
        .collect()
}

fn provider_name(kind: &str) -> String {
    match kind {
        "dropbox" => "Dropbox",
        "drive" => "Google Drive",
        "onedrive" => "OneDrive",
        "box" => "Box",
        "pcloud" => "pCloud",
        "mega" => "MEGA",
        "s3" => "S3",
        "b2" => "Backblaze B2",
        "webdav" => "WebDAV",
        "sftp" => "SFTP",
        "protondrive" => "Proton Drive",
        "koofr" => "Koofr",
        "yandex" => "Yandex Disk",
        "jottacloud" => "Jottacloud",
        "" => "rclone",
        other => other,
    }
    .to_string()
}

/// A remote name SlopBar will pass to rclone: rclone's own characters, and never one that could
/// read as a flag.
pub fn valid_remote(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.starts_with(' ')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.+@ ".contains(c))
}

/// Presence records on an rclone remote.
pub struct Rclone<R: Runner = Cli> {
    remote: String,
    runner: R,
}

impl Rclone<Cli> {
    /// The store for `remote`, if rclone is installed and the name is safe to pass.
    pub fn open(remote: &str) -> Option<Self> {
        if !valid_remote(remote) {
            return None;
        }
        Some(Rclone { remote: remote.into(), runner: Cli(find()?) })
    }
}

impl<R: Runner> Rclone<R> {
    #[cfg(test)]
    pub fn with(remote: &str, runner: R) -> Self {
        Rclone { remote: remote.into(), runner }
    }

    fn path(&self, name: &str) -> String {
        format!("{}:{DIR}/{name}", self.remote)
    }

    fn run(&self, args: &[&str], input: Option<&[u8]>) -> std::io::Result<Ran> {
        let ran = self.runner.run(args, input)?;
        match ran.code {
            Some(0) => Ok(ran),
            code => Err(std::io::Error::other(format!("rclone {} failed ({code:?})", args[0]))),
        }
    }
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Size", default)]
    size: i64,
    #[serde(rename = "ModTime")]
    mod_time: Option<DateTime<Utc>>,
    #[serde(rename = "IsDir", default)]
    is_dir: bool,
}

impl<R: Runner> PresenceStore for Rclone<R> {
    fn list(&self) -> std::io::Result<Vec<(Presence, Option<DateTime<Utc>>)>> {
        let dir = format!("{}:{DIR}", self.remote);
        let ran = self.runner.run(&["lsjson", "--files-only", "--no-mimetype", &dir], None)?;
        match ran.code {
            Some(0) => {}
            // Nobody has written yet.
            Some(DIR_NOT_FOUND) => return Ok(vec![]),
            code => return Err(std::io::Error::other(format!("rclone lsjson failed ({code:?})"))),
        }
        let entries: Vec<Entry> = serde_json::from_slice(&ran.stdout).map_err(std::io::Error::other)?;
        Ok(entries
            .into_iter()
            .filter(|e| !e.is_dir && e.name.ends_with(".json") && (0..=MAX_RECORD as i64).contains(&e.size) && !e.name.contains('/'))
            .filter_map(|e| {
                let ran = self.run(&["cat", &self.path(&e.name)], None).ok()?;
                let presence: Presence = serde_json::from_slice(&ran.stdout).ok()?;
                Some((presence, e.mod_time))
            })
            .collect())
    }

    /// Uploads under a temporary name, then renames over the record, so a reader never sees half
    /// of one.
    fn write(&self, presence: &Presence) -> std::io::Result<()> {
        let safe: String = presence.machine_id.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        if safe.is_empty() {
            return Err(std::io::Error::other("no machine id"));
        }
        let bytes = serde_json::to_vec_pretty(presence)?;
        let tmp = self.path(&format!("{safe}.tmp"));
        self.run(&["rcat", &tmp], Some(&bytes))?;
        self.run(&["moveto", &tmp, &self.path(&format!("{safe}.json"))], None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presence::{own, read};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// A fake rclone: an in-memory remote that understands the commands SlopBar uses.
    #[derive(Default)]
    struct Fake {
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        calls: Mutex<Vec<String>>,
        remotes: String,
        fail: Option<i32>,
    }

    impl Runner for Fake {
        fn run(&self, args: &[&str], input: Option<&[u8]>) -> std::io::Result<Ran> {
            self.calls.lock().unwrap().push(args.join(" "));
            let ok = |stdout: Vec<u8>| Ok(Ran { code: Some(0), stdout });
            if let Some(code) = self.fail {
                return Ok(Ran { code: Some(code), stdout: vec![] });
            }
            let mut files = self.files.lock().unwrap();
            match args {
                ["listremotes", "--long"] => ok(self.remotes.clone().into_bytes()),
                ["lsjson", "--files-only", "--no-mimetype", dir] => {
                    let prefix = format!("{dir}/");
                    let list: Vec<_> = files
                        .iter()
                        .filter_map(|(k, v)| k.strip_prefix(&prefix).map(|n| serde_json::json!({
                            "Path": n, "Name": n, "Size": v.len(), "ModTime": "2026-10-21T07:00:00Z", "IsDir": false })))
                        .collect();
                    if list.is_empty() {
                        return Ok(Ran { code: Some(DIR_NOT_FOUND), stdout: vec![] });
                    }
                    ok(serde_json::to_vec(&list).unwrap())
                }
                ["cat", path] => ok(files.get(*path).cloned().unwrap_or_default()),
                ["rcat", path] => {
                    files.insert(path.to_string(), input.unwrap_or_default().to_vec());
                    ok(vec![])
                }
                ["moveto", from, to] => {
                    let bytes = files.remove(*from).ok_or_else(|| std::io::Error::other("missing"))?;
                    files.insert(to.to_string(), bytes);
                    ok(vec![])
                }
                other => panic!("unexpected rclone call: {other:?}"),
            }
        }
    }

    fn account(id: &str) -> crate::model::AccountUsage {
        let mut a: crate::model::AccountUsage = serde_json::from_value(serde_json::json!({
            "id": id, "provider": "claude", "email": format!("{id}@example.invalid"), "alias": null, "plan": "max",
            "org": "Synthetic Org", "active": true, "sources": ["Windows"], "windows": [], "credits": null, "resets": [],
            "error": null, "stale": false, "fetched_at": null, "monitor_connected": false })).unwrap();
        a.active = true;
        a
    }

    #[test]
    fn writes_atomically_and_reads_back_through_the_remote() {
        let store = Rclone::with("cloud", Fake::default());
        let now = Utc::now();
        assert!(store.list().unwrap().is_empty(), "a missing folder is an empty one");
        let mine = own("11111111-1111-4111-8111-111111111111", "BACHELOR", 300, &[account("synthetic-a:org")], now);
        let other = own("22222222-2222-4222-8222-222222222222", "LAPTOP-2", 300, &[account("synthetic-a:org")], now);
        store.write(&mine).unwrap();
        store.write(&other).unwrap();
        let calls = store.runner.calls.lock().unwrap().clone();
        assert!(calls.contains(&"rcat cloud:SlopBar/presence/22222222-2222-4222-8222-222222222222.tmp".to_string()));
        assert!(calls.contains(&"moveto cloud:SlopBar/presence/22222222-2222-4222-8222-222222222222.tmp cloud:SlopBar/presence/22222222-2222-4222-8222-222222222222.json".to_string()));
        assert!(calls.iter().all(|c| c.contains("cloud:SlopBar/presence") || c.starts_with("listremotes")), "only the presence folder: {calls:?}");
        for bytes in store.runner.files.lock().unwrap().values() {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains('@') && !text.contains("synthetic-a") && !text.contains("Synthetic Org"), "{text}");
        }
        let peers = read(&store, &mine.machine_id, now);
        assert_eq!(peers.iter().map(|p| p.machine.as_str()).collect::<Vec<_>>(), ["LAPTOP-2"]);
        assert_eq!(peers[0].seats.len(), 1);
        store.clear(&other.machine_id, "LAPTOP-2", 300).unwrap();
        assert!(read(&store, &mine.machine_id, Utc::now())[0].seats.is_empty(), "a clean quit empties the record");
    }

    #[test]
    fn failures_show_nothing_rather_than_guessing() {
        let store = Rclone::with("cloud", Fake { fail: Some(1), ..Default::default() });
        assert!(store.list().is_err());
        assert!(store.write(&own("11111111-1111-4111-8111-111111111111", "B", 300, &[], Utc::now())).is_err());
        assert!(read(&store, "me", Utc::now()).is_empty());
    }

    #[test]
    fn lists_remotes_by_type_only() {
        let fake = Fake { remotes: "dropbox: dropbox\nwork drive:   drive\n-evil: s3\nodd:\n".into(), ..Default::default() };
        let found = remotes(&fake);
        assert_eq!(found, [
            Remote { name: "dropbox".into(), kind: "dropbox".into(), provider: "Dropbox".into() },
            Remote { name: "work drive".into(), kind: "drive".into(), provider: "Google Drive".into() },
            Remote { name: "odd".into(), kind: "".into(), provider: "rclone".into() },
        ]);
        assert_eq!(*fake.calls.lock().unwrap(), ["listremotes --long"], "never `config dump`");
        assert!(remotes(&Fake { fail: Some(1), ..Default::default() }).is_empty());
    }

    #[test]
    fn remote_names_cannot_become_flags() {
        for bad in ["", "-x", "--config", " lead", "a/b", "a:b", "a\nb", &"x".repeat(65)] {
            assert!(!valid_remote(bad), "{bad:?}");
        }
        for good in ["dropbox", "work drive", "my.remote+2", "a@b_c-d"] {
            assert!(valid_remote(good), "{good:?}");
        }
    }

    /// Against a real remote, touching only `SlopBar/presence`:
    /// `SLOPBAR_RCLONE_LIVE=dropbox cargo test --lib rclone_live -- --ignored`
    #[test]
    #[ignore]
    fn rclone_live() {
        let remote = std::env::var("SLOPBAR_RCLONE_LIVE").expect("SLOPBAR_RCLONE_LIVE names a remote");
        let store = Rclone::open(&remote).expect("rclone installed and a valid remote");
        let id = "00000000-0000-4000-8000-00000000c0de";
        let record = own(id, "SLOPBAR-LIVE-TEST", 300, &[account("synthetic-live:org")], Utc::now());
        store.write(&record).unwrap();
        let listed = store.list().unwrap();
        let back = listed.iter().find(|(p, _)| p.machine_id == id).expect("written record listed");
        assert_eq!(back.0.seats, record.seats);
        assert!(back.1.is_some(), "rclone reports a modification time");
        store.clear(id, "SLOPBAR-LIVE-TEST", 300).unwrap();
        let cleared = store.list().unwrap();
        assert!(cleared.iter().find(|(p, _)| p.machine_id == id).unwrap().0.seats.is_empty());
        // Leave nothing behind.
        store.run(&["deletefile", &store.path(&format!("{id}.json"))], None).unwrap();
        assert!(store.list().unwrap().iter().all(|(p, _)| p.machine_id != id));
    }
}
