//! Discovers home directories where CLI credentials may live:
//! the Windows user profile and the default user's home in each WSL distro.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Source {
    pub label: String,
    pub home: PathBuf,
    /// Explicit config dirs (only honored for the native source, from env vars).
    pub claude_dir: Option<PathBuf>,
    pub codex_dir: Option<PathBuf>,
    /// The WSL distro this source lives in, if any.
    pub distro: Option<String>,
    /// The home directory as seen from inside the source (e.g. /home/me in WSL).
    pub native_home: String,
}

impl Source {
    pub fn claude_dir(&self) -> PathBuf {
        self.claude_dir.clone().unwrap_or_else(|| self.home.join(".claude"))
    }
    pub fn codex_dir(&self) -> PathBuf {
        self.codex_dir.clone().unwrap_or_else(|| self.home.join(".codex"))
    }
    /// Claude Code keeps account details in `.claude.json`: in the home directory by default,
    /// inside the config dir when CLAUDE_CONFIG_DIR is set.
    pub fn claude_json(&self) -> PathBuf {
        match &self.claude_dir {
            Some(dir) => dir.join(".claude.json"),
            None => self.home.join(".claude.json"),
        }
    }
}

pub fn discover() -> Vec<Source> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(Source {
            label: if cfg!(windows) { "Windows".into() } else { "Local".into() },
            claude_dir: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            codex_dir: std::env::var_os("CODEX_HOME").map(PathBuf::from),
            distro: None,
            native_home: home.to_string_lossy().into_owned(),
            home,
        });
    }
    #[cfg(windows)]
    out.extend(wsl_sources());
    out
}

pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

// ---------------------------------------------------------------------------------------------
// WSL
//
// Reading a distro (running a command in it, or opening \\wsl.localhost\<distro>) boots it, and
// with it the WSL VM. Most PCs never sign in to a CLI from WSL, and a wedged WSL used to stall
// every refresh. So the list of distros and which are running come from `wsl.exe --list`, which
// asks the WSL service without booting anything; a stopped distro is read only when it had CLI
// sign-ins before (and then at most every `WAKE_EVERY`), when the setting says always, or once
// after the user asks for a rescan. Every wsl.exe call has a deadline: a hung WSL is skipped.

/// The setting "Look for accounts in WSL".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WslScan {
    /// Running distros, and stopped ones known to hold CLI sign-ins.
    #[default]
    Auto,
    /// Every distro, starting stopped ones (what SlopBar always did before).
    On,
    /// Never runs wsl.exe.
    Off,
}

/// What a refresh does with one distro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WslPlan {
    /// Already running: reading it costs nothing.
    Read,
    /// Stopped, but read anyway, which starts it.
    Wake,
    /// Stopped and left alone.
    Skip,
}

/// Whether to read a distro. `known` is that it held CLI sign-ins when last read; `wake_due`
/// that a stopped one was not woken within `WAKE_EVERY`; `rescan` that the user asked to look
/// at every distro once.
pub fn wsl_plan(mode: WslScan, running: bool, known: bool, wake_due: bool, rescan: bool) -> WslPlan {
    match mode {
        WslScan::Off => WslPlan::Skip,
        _ if running => WslPlan::Read,
        WslScan::On => WslPlan::Wake,
        WslScan::Auto if rescan || (known && wake_due) => WslPlan::Wake,
        WslScan::Auto => WslPlan::Skip,
    }
}

/// Listing distros asks the WSL service only and never starts a distro. It usually answers in
/// well under a second, but a WSL service that went idle can take several seconds to come back.
const LIST_TIMEOUT: Duration = Duration::from_secs(12);
/// Listings in a row that must miss their deadline before Health calls WSL stuck: one slow
/// listing is a service waking up, not a wedged WSL.
const MISSES_BEFORE_WARNING: u32 = 2;
/// Reading a running distro's home directory.
const READ_TIMEOUT: Duration = Duration::from_secs(6);
/// Starting a stopped distro and reading its home directory.
const WAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// After wsl.exe did not answer in time, WSL is left alone this long.
const BACKOFF: Duration = Duration::from_secs(10 * 60);
/// A stopped distro with sign-ins is woken at most this often: nothing in it changes while it
/// is stopped, so its first reading this session stays good until it runs again.
const WAKE_EVERY: Duration = Duration::from_secs(6 * 3600);

/// One distro as the last refresh saw it, for Settings > Health.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DistroStatus {
    pub name: String,
    pub running: bool,
    pub plan: WslPlan,
    /// Read, and CLI sign-in files were there.
    pub has_logins: bool,
    /// Known (from earlier reads) to hold CLI sign-ins.
    pub known: bool,
    /// Reading it did not finish in time.
    pub timed_out: bool,
}

/// How the last discovery went with WSL.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WslStatus {
    pub mode: WslScan,
    pub checked_at: Option<DateTime<Utc>>,
    /// wsl.exe answered with at least one distro.
    pub installed: bool,
    /// Listing distros did not finish in time; WSL is skipped for a while.
    pub timed_out: bool,
    /// Skipped without asking, still waiting out an earlier timeout.
    pub backing_off: bool,
    pub distros: Vec<DistroStatus>,
}

struct WslState {
    mode: WslScan,
    rescan: bool,
    backoff_until: Option<Instant>,
    /// Listings in a row that did not answer in time.
    listing_misses: u32,
    woken: BTreeMap<String, Instant>,
    status: WslStatus,
}

static WSL: Mutex<WslState> = Mutex::new(WslState {
    mode: WslScan::Auto,
    rescan: false,
    backoff_until: None,
    listing_misses: 0,
    woken: BTreeMap::new(),
    status: WslStatus { mode: WslScan::Auto, checked_at: None, installed: false, timed_out: false, backing_off: false, distros: Vec::new() },
});

fn wsl_state() -> std::sync::MutexGuard<'static, WslState> {
    WSL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Follows the setting; the app calls it on start and whenever settings change.
pub fn set_wsl_mode(mode: WslScan) {
    wsl_state().mode = mode;
}

/// The next discovery reads every distro once, starting stopped ones (Settings > Data, "Rescan
/// WSL"), and forgets an earlier timeout.
pub fn request_wsl_rescan() {
    let mut s = wsl_state();
    s.rescan = true;
    s.backoff_until = None;
}

/// How the last discovery went with WSL (Settings > Health).
pub fn wsl_status() -> WslStatus {
    wsl_state().status.clone()
}

/// Whether `misses` listings in a row without an answer mean WSL is stuck.
pub fn listing_stuck(misses: u32) -> bool {
    misses >= MISSES_BEFORE_WARNING
}

/// Runs a command, giving up (and ending it) after `limit`. None when it could not start or did
/// not finish in time.
pub fn output_within(cmd: &mut Command, limit: Duration) -> Option<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    // Read on another thread so a full pipe can't hold the command up.
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let stdout = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    Some(Output { status, stdout, stderr: Vec::new() })
}

/// Runs wsl.exe with `args`, within `limit`.
pub fn wsl(args: &[&str], limit: Duration) -> Option<Output> {
    output_within(command("wsl.exe").args(args), limit)
}

/// Distro names from `wsl.exe --list --quiet` output, without Docker Desktop's own.
pub fn parse_distros(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().trim_matches('\0').trim())
        .filter(|d| !d.is_empty() && !d.starts_with("docker-desktop"))
        .map(str::to_string)
        .collect()
}

/// Distros known to hold CLI sign-ins, remembered across restarts.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Known {
    with_logins: Vec<String>,
}

fn known_path() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("wsl-distros.json"))
}

fn load_known() -> Vec<String> {
    known_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Known>(&b).ok())
        .map(|k| k.with_logins).unwrap_or_default()
}

fn save_known(with_logins: Vec<String>) {
    if let (Some(path), Ok(bytes)) = (known_path(), serde_json::to_vec(&Known { with_logins })) {
        let _ = crate::private_store::write(&path, &bytes);
    }
}

/// Whether a home holds a CLI sign-in (Claude Code's or Codex's).
fn has_logins(source: &Source) -> bool {
    source.claude_dir().join(".credentials.json").exists() || source.codex_dir().join("auth.json").exists()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn wsl_sources() -> Vec<Source> {
    let (mode, rescan, backoff, woken) = {
        let mut s = wsl_state();
        let rescan = std::mem::take(&mut s.rescan);
        (s.mode, rescan, s.backoff_until.is_some_and(|t| Instant::now() < t), s.woken.clone())
    };
    let mut status = WslStatus { mode, checked_at: Some(Utc::now()), backing_off: backoff, ..Default::default() };
    let finish = |status: WslStatus| wsl_state().status = status;
    // A listing that missed its deadline: back off, and call WSL stuck only once it keeps missing.
    let listing_missed = |mut status: WslStatus| {
        let mut s = wsl_state();
        s.backoff_until = Some(Instant::now() + BACKOFF);
        s.listing_misses += 1;
        let previous = std::mem::take(&mut s.status);
        status.timed_out = listing_stuck(s.listing_misses);
        status.installed = previous.installed;
        status.distros = previous.distros;
        s.status = status;
    };
    if mode == WslScan::Off || backoff {
        // Keep what the last look found, so Health can still say which distros there are.
        let previous = wsl_status();
        status.installed = previous.installed;
        status.timed_out = backoff && previous.timed_out;
        if backoff { status.distros = previous.distros; }
        finish(status);
        return vec![];
    }
    let Some(all) = wsl(&["--list", "--quiet"], LIST_TIMEOUT) else {
        listing_missed(status);
        return vec![];
    };
    if !all.status.success() {
        finish(status);
        return vec![];
    }
    let distros = parse_distros(&decode_wsl_output(&all.stdout));
    status.installed = !distros.is_empty();
    // No distro running makes this fail with a message; that is an empty list.
    let running: Vec<String> = match wsl(&["--list", "--running", "--quiet"], LIST_TIMEOUT) {
        Some(o) if o.status.success() => parse_distros(&decode_wsl_output(&o.stdout)),
        Some(_) => vec![],
        None => {
            listing_missed(status);
            return vec![];
        }
    };
    wsl_state().listing_misses = 0;
    let known_before = load_known();
    let mut known = known_before.clone();
    let mut found = vec![];
    let mut timed_out = false;
    for name in distros {
        let is_running = running.contains(&name);
        let is_known = known.contains(&name);
        let wake_due = woken.get(&name).map_or(true, |t| t.elapsed() >= WAKE_EVERY);
        let plan = wsl_plan(mode, is_running, is_known, wake_due, rescan);
        let mut entry = DistroStatus { name: name.clone(), running: is_running, plan, has_logins: false, known: is_known, timed_out: false };
        if plan != WslPlan::Skip {
            if plan == WslPlan::Wake { wsl_state().woken.insert(name.clone(), Instant::now()); }
            match wsl_source(&name, if plan == WslPlan::Wake { WAKE_TIMEOUT } else { READ_TIMEOUT }) {
                Ok(source) => {
                    entry.has_logins = source.as_ref().is_some_and(has_logins);
                    // Remember (or forget) that it holds sign-ins.
                    known.retain(|k| k != &name);
                    if entry.has_logins { known.push(name.clone()); }
                    found.extend(source);
                }
                Err(()) => { entry.timed_out = true; timed_out = true; }
            }
        }
        status.distros.push(entry);
    }
    if known != known_before { save_known(known); }
    // A distro that did not answer is not a wedged WSL: only the listing backs off.
    status.timed_out = timed_out;
    finish(status);
    found
}

/// A distro's home as a source. Err when wsl.exe did not answer within `limit`; Ok(None) when
/// it answered without a usable home.
fn wsl_source(distro: &str, limit: Duration) -> Result<Option<Source>, ()> {
    let out = wsl(&["-d", distro, "-e", "sh", "-c", "printf %s \"$HOME\""], limit).ok_or(())?;
    let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !home.starts_with('/') {
        return Ok(None);
    }
    let unc = [r"\\wsl.localhost\", r"\\wsl$\"]
        .iter()
        .map(|p| PathBuf::from(format!("{p}{distro}{}", home.replace('/', "\\"))))
        .find(|p| p.exists());
    Ok(unc.map(|unc| Source {
        label: format!("WSL: {distro}"),
        home: unc,
        claude_dir: None,
        codex_dir: None,
        distro: Some(distro.to_string()),
        native_home: home,
    }))
}

/// `wsl.exe --list` writes UTF-16LE; fall back to UTF-8 if WSL_UTF8 is set.
fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) || (bytes.len() >= 2 && bytes[1] == 0) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_string()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_off_never_reads_a_distro() {
        for running in [true, false] {
            for known in [true, false] {
                assert_eq!(wsl_plan(WslScan::Off, running, known, true, true), WslPlan::Skip);
            }
        }
    }

    #[test]
    fn one_slow_listing_is_not_a_stuck_wsl() {
        assert!(!listing_stuck(0));
        assert!(!listing_stuck(1));
        assert!(listing_stuck(2));
        assert!(LIST_TIMEOUT >= Duration::from_secs(10), "an idle WSL service can take seconds to answer");
    }

    #[test]
    fn running_distros_are_read_without_waking_anything() {
        assert_eq!(wsl_plan(WslScan::Auto, true, false, false, false), WslPlan::Read);
        assert_eq!(wsl_plan(WslScan::On, true, false, false, false), WslPlan::Read);
    }

    #[test]
    fn auto_leaves_stopped_distros_alone_unless_they_had_sign_ins() {
        // A stopped distro that never held a CLI sign-in stays stopped.
        assert_eq!(wsl_plan(WslScan::Auto, false, false, true, false), WslPlan::Skip);
        // One that did is woken, but at most every WAKE_EVERY.
        assert_eq!(wsl_plan(WslScan::Auto, false, true, true, false), WslPlan::Wake);
        assert_eq!(wsl_plan(WslScan::Auto, false, true, false, false), WslPlan::Skip);
        // A rescan looks at every distro once.
        assert_eq!(wsl_plan(WslScan::Auto, false, false, false, true), WslPlan::Wake);
    }

    #[test]
    fn on_wakes_every_stopped_distro() {
        assert_eq!(wsl_plan(WslScan::On, false, false, false, false), WslPlan::Wake);
    }

    #[test]
    fn the_setting_reads_as_lowercase_words_and_defaults_to_auto() {
        assert_eq!(serde_json::from_str::<WslScan>("\"off\"").unwrap(), WslScan::Off);
        assert_eq!(serde_json::to_string(&WslScan::On).unwrap(), "\"on\"");
        assert_eq!(WslScan::default(), WslScan::Auto);
    }

    #[test]
    fn distro_lists_decode_from_utf16_and_drop_docker() {
        let text = "Ubuntu-22.04\r\nDebian\r\ndocker-desktop\r\ndocker-desktop-data\r\n\r\n";
        let utf16: Vec<u8> = std::iter::once(0xfeffu16).chain(text.encode_utf16()).flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_distros(&decode_wsl_output(&utf16)), ["Ubuntu-22.04", "Debian"]);
        assert_eq!(parse_distros(&decode_wsl_output(text.as_bytes())), ["Ubuntu-22.04", "Debian"]);
        assert!(parse_distros("").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_command_that_outlives_its_deadline_is_ended() {
        let started = Instant::now();
        let mut cmd = command("powershell.exe");
        cmd.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 120"]);
        assert!(output_within(&mut cmd, Duration::from_millis(300)).is_none());
        // Far sooner than the command would end, with slack for a machine busy running tests.
        assert!(started.elapsed() < Duration::from_secs(60));
        let mut quick = command("cmd.exe");
        quick.args(["/C", "echo hi"]);
        let out = output_within(&mut quick, Duration::from_secs(60)).unwrap();
        assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("hi"));
    }
}
