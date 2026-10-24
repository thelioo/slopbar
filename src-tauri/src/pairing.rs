//! Pairing: an agent on one of the user's computers brings another onto the account roster over
//! SSH (OpenSSH or Tailscale SSH), so nobody types a roster passphrase on each machine.
//!
//! The source computer's roster passphrase (or, when it has none, a new random one) is the
//! pairing key. It goes from this process straight into `ssh`'s standard input and from there into
//! the target's `slopbar-mcp --pair-receive`, which hands it to the target app's door; each app
//! keeps it as it keeps every roster passphrase (DPAPI, see `roster::store_passphrase`). It is
//! never on a command line, in a result, in an error or in a log, so the agent driving this never
//! sees it either.
//!
//! The same SSH channel answers readiness questions (`readiness`: build, desktop session, browser,
//! roster, accounts waiting) and runs the target's own tools (`--call`), whose sign-ins the
//! target's running app opens on its desktop, never in the SSH session.

use std::io::{Read, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backup::Passphrase;
use crate::door::Request;

pub const OFFER_FORMAT: &str = "slopbar-pairing";
const OFFER_VERSION: u32 = 1;
/// An offer is one JSON line; a roster passphrase is at most 1024 characters of up to 4 bytes.
const MAX_OFFER: u64 = 16 * 1024;
/// A remote tool's answer (a usage snapshot is the largest).
const MAX_ANSWER: u64 = 4 * 1024 * 1024;
/// Random bytes in a generated key: 256 bits, far past any guessing.
const KEY_BYTES: usize = 32;
const KEY_PREFIX: &str = "slopbar-pair-";
/// Connecting, and the remote tool, together. Sign-ins return at once, so this is generous.
const SSH_TIMEOUT: Duration = Duration::from_secs(120);
/// Tools another computer may be asked to run through `run_on_machine`: what bringing it up
/// needs, and nothing that takes a secret.
pub const REMOTE_TOOLS: [&str; 14] = ["get_machine_readiness", "get_app_info", "get_health", "get_usage", "get_settings", "list_sharing_locations",
    "get_roster", "sync_roster", "update_settings", "sign_in_all", "stop_sign_ins", "connect_monitoring_all", "refresh_usage", "restart_app"];

/// What the source sends the target, on `ssh`'s standard input only.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub format: String,
    pub v: u32,
    pub key: Passphrase,
    /// The rclone remote the source shares through; None when it uses a synced folder, whose
    /// path differs per computer.
    pub presence_remote: Option<String>,
    pub from: String,
    /// Replace a different roster passphrase the target already has (which its other peers
    /// share): only when the user said so.
    #[serde(default)]
    pub replace: bool,
}

/// Settings `run_on_machine` may change on another computer: where it shares and whether, and
/// its sign-in window. Never phone alerts, aliases or anything else.
pub const REMOTE_SETTINGS: [&str; 4] = ["presence_remote", "presence_folder", "account_roster", "signin_window"];

/// A new pairing key: 256 random bits as lowercase base32.
pub fn generate_key() -> Passphrase {
    use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
    let mut bytes = [0u8; KEY_BYTES];
    OsRng.fill_bytes(&mut bytes);
    Passphrase(format!("{KEY_PREFIX}{}", base32(&bytes)))
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut out, mut buffer, mut bits) = (String::new(), 0u32, 0u32);
    for &b in bytes {
        buffer = (buffer << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// An SSH destination as `ssh` takes it: a ~/.ssh/config alias, host name, Tailscale name or
/// address, optionally `user@`. Never something `ssh` could read as an option.
pub fn valid_host(host: &str) -> bool {
    let ok_chars = host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@' | ':'));
    (1..=253).contains(&host.len()) && ok_chars && !host.starts_with('-') && host.matches('@').count() <= 1 && !host.ends_with('@')
}

/// The command `ssh` runs on the target: its `slopbar-mcp.exe` with `args`, the same from cmd,
/// Windows PowerShell or PowerShell 7 as the remote shell (an encoded command needs no quoting).
/// `args` never holds a secret: a command line is visible to every process on the target.
pub fn remote_command(args: &[&str]) -> String {
    let quoted: Vec<String> = args.iter().map(|a| format!("'{}'", a.replace('\'', "''"))).collect();
    let script = format!(concat!(
        "$ErrorActionPreference = 'Stop'\n",
        "$exe = Join-Path $env:LOCALAPPDATA 'SlopBar\\slopbar-mcp.exe'\n",
        "if (-not (Test-Path -LiteralPath $exe)) {{ '{{\"error\":\"slopbar_not_installed\"}}'; exit 3 }}\n",
        // A pipeline stage, not just capturing it, makes Windows PowerShell wait for this windowless
        // program; it still inherits standard input.
        "$out = & $exe {} | ForEach-Object {{ $_ }}\n",
        "$code = $LASTEXITCODE\n",
        "$out\n",
        "exit $code\n"), quoted.join(" "));
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD,
        script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    format!("powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded}")
}

/// JSON with every non-ASCII character escaped, so no console code page on the way through SSH
/// can garble an email or machine name.
pub fn ascii_json(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "{\"error\":\"unserializable\"}".into());
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The SSH host each paired computer answered on, by its machine id (the one its roster carries),
// so this computer's app can reach it too: Retry on Settings > Accounts' "Signing in on <machine>".

const HOSTS_FILE: &str = "paired-machines.json";
const MAX_HOSTS: usize = 64;

fn hosts(dir: &std::path::Path) -> serde_json::Map<String, Value> {
    std::fs::read(dir.join(HOSTS_FILE)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// Notes that the computer `machine_id` answered on `host` (from its readiness, which carries the id).
pub fn remember_host(dir: &std::path::Path, machine_id: &str, host: &str) {
    if !valid_host(host) || crate::roster::file_name(machine_id).as_deref() != Some(&format!("{machine_id}.roster")) {
        return;
    }
    let mut map = hosts(dir);
    if map.get(machine_id).and_then(Value::as_str) == Some(host) {
        return;
    }
    map.remove(machine_id);
    while map.len() >= MAX_HOSTS {
        let Some(first) = map.keys().next().cloned() else { break };
        map.remove(&first);
    }
    map.insert(machine_id.into(), host.into());
    let _ = crate::private_store::write(&dir.join(HOSTS_FILE), &serde_json::to_vec_pretty(&map).unwrap_or_default());
}

/// The SSH host `machine_id` last answered on here, if it was ever checked or paired from here.
pub fn host_for(dir: &std::path::Path, machine_id: &str) -> Option<String> {
    hosts(dir).get(machine_id).and_then(Value::as_str).filter(|h| valid_host(h)).map(str::to_owned)
}

fn remember_answer(host: &str, readiness: &Value) {
    if let (Some(dir), Some(id)) = (crate::settings::app_dir(), readiness.pointer("/machine/id").and_then(Value::as_str)) {
        remember_host(&dir, id, host);
    }
}

// ---------------------------------------------------------------------------------------------
// What pairing talks to, so tests can stand in for the app and the other computer.

pub trait Door {
    fn call(&self, request: &Request) -> Result<Value, String>;
}

/// The app running here, through its door.
pub struct LocalDoor(pub String);

impl Door for LocalDoor {
    fn call(&self, request: &Request) -> Result<Value, String> {
        crate::door::call(&self.0, request)
    }
}

pub trait Remote {
    /// Runs `command` on `host` with `input` as its standard input; returns its standard output.
    fn run(&self, host: &str, command: &str, input: &[u8]) -> Result<Vec<u8>, String>;
}

/// The system's OpenSSH client, with the user's own keys and ~/.ssh/config (Tailscale SSH works
/// through it too). It never asks for a password or accepts an unknown host key.
pub struct Ssh;

impl Ssh {
    fn exe() -> std::path::PathBuf {
        let system = std::env::var_os("SystemRoot").map(|r| std::path::PathBuf::from(r).join(r"System32\OpenSSH\ssh.exe"));
        system.filter(|p| p.is_file()).unwrap_or_else(|| "ssh".into())
    }
}

impl Remote for Ssh {
    fn run(&self, host: &str, command: &str, input: &[u8]) -> Result<Vec<u8>, String> {
        use std::process::{Command, Stdio};
        if !valid_host(host) {
            return Err("bad_ssh_host".into());
        }
        let mut cmd = Command::new(Self::exe());
        cmd.args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=15", "-T", host, command])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: this server has no console.
        }
        let mut child = cmd.spawn().map_err(|_| "ssh_missing".to_string())?;
        let mut stdin = child.stdin.take().ok_or("ssh_failed")?;
        let input = input.to_vec();
        let writer = std::thread::spawn(move || { let _ = stdin.write_all(&input); });
        let mut stdout = child.stdout.take().ok_or("ssh_failed")?;
        let mut stderr = child.stderr.take().ok_or("ssh_failed")?;
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = (&mut stdout).take(MAX_ANSWER).read_to_end(&mut out);
            let _ = send.send(out);
        });
        let errors = std::thread::spawn(move || {
            let mut err = Vec::new();
            let _ = (&mut stderr).take(16 * 1024).read_to_end(&mut err);
            err
        });
        let Ok(out) = receive.recv_timeout(SSH_TIMEOUT) else {
            let _ = child.kill();
            return Err("ssh_timeout".into());
        };
        let _ = writer.join();
        // Standard output closed (or hit its bound): the exit follows at once, or never.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    return Err("ssh_timeout".into());
                }
            }
        };
        let err = errors.join().unwrap_or_default();
        // 255 is ssh's own failure (no connection, key refused, unknown host key); "**" lines are
        // its banners (a key exchange warning), not the reason.
        if status.code() == Some(255) {
            let first = String::from_utf8_lossy(&err).lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("**")).unwrap_or("").chars().take(200).collect::<String>();
            return Err(format!("ssh_failed: {first}"));
        }
        Ok(out)
    }
}

/// Runs one of the target's tools through its `slopbar-mcp --call`; its answer or its error.
pub fn remote_call(remote: &dyn Remote, host: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
    let ask = json!({ "tool": tool, "arguments": arguments });
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, ask.to_string());
    answer(&remote.run(host, &remote_command(&["--call-b64", &encoded]), b"")?)
}

/// The JSON object a remote `slopbar-mcp` printed last: `{"ok": …}` or `{"error": …}`.
fn answer(out: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8_lossy(out);
    let last = text.lines().map(str::trim).rfind(|l| l.starts_with('{')).ok_or("remote_no_answer")?;
    let mut value: Value = serde_json::from_str(last).map_err(|_| "remote_no_answer".to_string())?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    value.get_mut("ok").map(Value::take).ok_or_else(|| "remote_no_answer".into())
}

// ---------------------------------------------------------------------------------------------
// Pairing

/// The source's half: checks the target is up, makes sure this computer shares its roster with a
/// key (its own passphrase, or a new random one), sends the key to `host` and syncs both.
/// `saved` is this computer's saved roster passphrase, if any.
pub fn pair(door: &dyn Door, remote: &dyn Remote, host: &str, saved: Option<Passphrase>, machine: &str, replace: bool) -> Result<Value, String> {
    if !valid_host(host) {
        return Err("bad_ssh_host".into());
    }
    let settings = door.call(&Request::GetSettings)?;
    let presence_remote = settings.get("presence_remote").and_then(Value::as_str).map(str::to_owned);
    let folder = settings.get("presence_folder").and_then(Value::as_str).is_some();
    if presence_remote.is_none() && !folder {
        return Err("no_sharing_location".into());
    }
    // The target first: nothing changes here when it cannot take the key.
    let target = remote_call(remote, host, "get_machine_readiness", &json!({})).map_err(|e| format!("target: {e}"))?;
    if target.pointer("/app/running") != Some(&Value::Bool(true)) {
        return Err("target_not_running".into());
    }
    // And that it can share where this computer does, before anything changes here.
    match &presence_remote {
        Some(name) => {
            let places = remote_call(remote, host, "list_sharing_locations", &json!({})).map_err(|e| format!("target: {e}"))?;
            // rclone itself must list the remote there: the setting alone shares nothing.
            if !lists_remote(&places, name) {
                return Err(format!("target: remote_missing_here: {name}"));
            }
        }
        None if target.pointer("/roster/location") != Some(&Value::Bool(true)) => return Err("no_sharing_location_here".into()),
        None => {}
    }
    if settings.get("account_roster") != Some(&Value::Bool(true)) {
        door.call(&Request::UpdateSettings { patch: json!({ "account_roster": true }) })?;
    }
    let key = match saved {
        Some(key) => (key, false),
        None => {
            let key = generate_key();
            door.call(&Request::SetRosterPassphrase { passphrase: Some(key.clone()) })?;
            (key, true)
        }
    };
    let shown = key.0 .0.clone();
    let offer = Offer { format: OFFER_FORMAT.into(), v: OFFER_VERSION, key: key.0, presence_remote, from: machine.into(), replace };
    let mut line = serde_json::to_vec(&offer).map_err(|_| "pairing_failed")?;
    line.push(b'\n');
    drop(offer);
    // Whatever the target sends back is scrubbed of the key, in case it echoes it.
    let received = match remote.run(host, &remote_command(&["--pair-receive"]), &line) {
        Ok(out) => answer(&out).map(|v| redact(v, &shown)).map_err(|e| format!("target: {}", e.replace(&shown, "[key]"))),
        Err(e) if e.starts_with("ssh_failed") => Err("ssh_failed".into()),
        Err(e) => Err(e),
    };
    line.iter_mut().for_each(|b| *b = 0);
    let received = received?;
    let here = door.call(&Request::SyncRoster)?;
    Ok(json!({
        "paired": true,
        "machine": target.get("machine").cloned().unwrap_or(Value::Null),
        "key": if key.1 { "generated" } else { "reused" },
        "here": roster_summary(&here),
        "target": received,
        "next": "Both computers share the roster with the same key, kept by each SlopBar and never shown. Check the target with get_machine_readiness (ssh_host), then ask the user to sit at the target's screen before run_on_machine sign_in_all with confirm: true.",
    }))
}

/// The target's half: reads the offer from `input`, points sharing at the source's rclone remote
/// when it has one, turns the roster on and saves the key. Never echoes the key. `saved` is the
/// target's own roster passphrase: a different one is replaced only when the offer says so, since
/// the target's other computers share it.
pub fn receive(door: &dyn Door, input: &mut dyn Read, saved: Option<Passphrase>) -> Result<Value, String> {
    let mut bytes = Vec::new();
    input.take(MAX_OFFER + 1).read_to_end(&mut bytes).map_err(|_| "pairing_unreadable")?;
    if bytes.len() as u64 > MAX_OFFER {
        bytes.iter_mut().for_each(|b| *b = 0);
        return Err("pairing_unreadable".into());
    }
    let offer: Result<Offer, _> = serde_json::from_slice(&bytes);
    bytes.iter_mut().for_each(|b| *b = 0);
    let offer = offer.map_err(|_| "pairing_unreadable")?;
    if offer.format != OFFER_FORMAT || offer.v != OFFER_VERSION {
        return Err("pairing_unsupported".into());
    }
    let replaced = saved.as_ref().is_some_and(|s| s.0 != offer.key.0);
    if replaced && !offer.replace {
        return Err("target_has_other_key".into());
    }
    let settings = door.call(&Request::GetSettings)?;
    let mut patch = serde_json::Map::new();
    match &offer.presence_remote {
        Some(name) => {
            if !lists_remote(&door.call(&Request::ListSharingLocations)?, name) {
                return Err(format!("remote_missing_here: {name}"));
            }
            if settings.get("presence_remote").and_then(Value::as_str) != Some(name) {
                patch.insert("presence_remote".into(), name.clone().into());
            }
        }
        None if settings.get("presence_remote").is_some_and(Value::is_string) || settings.get("presence_folder").is_some_and(Value::is_string) => {}
        None => return Err("no_sharing_location_here".into()),
    }
    if settings.get("account_roster") != Some(&Value::Bool(true)) {
        patch.insert("account_roster".into(), true.into());
    }
    if !patch.is_empty() {
        door.call(&Request::UpdateSettings { patch: Value::Object(patch) })?;
    }
    let roster = door.call(&Request::SetRosterPassphrase { passphrase: Some(offer.key) })?;
    Ok(json!({ "paired": true, "replaced_key": replaced, "from": offer.from.chars().filter(|c| !c.is_control()).take(63).collect::<String>(), "roster": roster_summary(&roster) }))
}

/// Whether list_sharing_locations' result shows rclone installed with the remote `name`.
fn lists_remote(places: &Value, name: &str) -> bool {
    places.pointer("/rclone/remotes").and_then(Value::as_array).is_some_and(|l| l.iter().any(|r| r.get("name").and_then(Value::as_str) == Some(name)))
}

/// `value` with every occurrence of `secret` in its strings replaced.
fn redact(value: Value, secret: &str) -> Value {
    match value {
        Value::String(s) if !secret.is_empty() && s.contains(secret) => Value::String(s.replace(secret, "[key]")),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| redact(v, secret)).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, redact(v, secret))).collect()),
        other => other,
    }
}

/// get_roster's result without the email list: machines, counts and what is waiting.
fn roster_summary(roster: &Value) -> Value {
    let pick = |k: &str| roster.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "enabled": pick("enabled"), "location": pick("location"), "passphrase_saved": pick("passphrase_saved"),
        "wrote": pick("wrote"), "read": pick("read"), "own_accounts": pick("own_accounts"),
        "machines": pick("machines"), "unreadable": pick("unreadable"),
        "waiting": roster.get("waiting").and_then(Value::as_array).map_or(0, Vec::len),
    })
}

// ---------------------------------------------------------------------------------------------
// Readiness

/// Whether this computer is ready for "Sign in all": what is missing, as codes, and what to do.
pub fn assess(running: bool, desktop: Option<crate::desktop::Desktop>, browser: bool, roster: &Value) -> (bool, Vec<&'static str>, String) {
    let mut problems = Vec::new();
    if !running {
        problems.push("app_not_running");
    }
    match desktop {
        None => problems.push("no_desktop_session"),
        Some(d) => problems.extend(d.problem()),
    }
    if !browser {
        problems.push("no_browser");
    }
    let ready = problems.is_empty();
    let flag = |k: &str| roster.get(k) == Some(&Value::Bool(true));
    if running {
        if !flag("enabled") { problems.push("roster_off"); }
        if !flag("location") { problems.push("no_sharing_location"); }
        if !flag("passphrase_saved") { problems.push("no_roster_key"); }
        if roster.get("unreadable").and_then(Value::as_u64).unwrap_or(0) > 0 { problems.push("roster_unreadable"); }
    }
    let next = match problems.first().copied() {
        Some("no_desktop_session") => "Nobody is signed in to this computer's desktop. The user signs in at its screen (or over Remote Desktop) and starts SlopBar there; sign-ins can only open on a desktop someone sees.",
        Some("app_not_running") => "SlopBar is not running on this computer's desktop. The user starts it there (Start menu); tools reached over SSH never start it, because it would run where nobody can see it.",
        Some("desktop_locked") => "This computer's desktop is locked: the user unlocks it at the screen before any sign-in starts.",
        Some("desktop_not_shown") => "This computer's desktop is signed in but not shown on any screen (a closed Remote Desktop, or another user switched in): the user goes back to it before any sign-in starts.",
        Some("no_browser") => "No Edge or Chrome was found for the sign-in pages: install one.",
        Some(_) => "Ready for sign-ins at the desktop, but the account roster is not set up: pair this computer with pair_machine from a computer that has the accounts.",
        None => "Ready: someone can complete sign-ins at this computer's screen. Ask them to sit there before sign_in_all with confirm: true.",
    };
    (ready, problems, next.into())
}

/// Readiness when the app does not answer: who is at the desktop, whether a browser and SlopBar
/// are installed. `reason` is the door's error.
pub fn readiness_without_app(reason: &str) -> Value {
    let desktop = crate::desktop::best(&crate::desktop::of_user());
    let browser = crate::signin::find_browser().map(|(k, _)| browser_name(k));
    let installed = std::env::var_os("LOCALAPPDATA").is_some_and(|d| std::path::Path::new(&d).join(r"SlopBar\slopbar.exe").is_file());
    let (ready, mut problems, mut next) = assess(false, desktop, browser.is_some(), &Value::Null);
    if reason == "unsupported_request" {
        problems.insert(0, "app_outdated");
        next = "The SlopBar running here predates readiness checks: update it, then ask again.".into();
    }
    json!({
        "app": { "running": reason == "unsupported_request", "installed": installed, "door": reason },
        "desktop": desktop, "browser": browser, "ready_for_sign_in": ready && reason != "unsupported_request",
        "problems": problems, "next": next,
    })
}

pub fn browser_name(kind: crate::signin::Kind) -> &'static str {
    match kind {
        crate::signin::Kind::Chrome => "chrome",
        crate::signin::Kind::Edge => "edge",
    }
}

// ---------------------------------------------------------------------------------------------
// The MCP tools served by `slopbar-mcp` itself (see `mcp::step`)

/// Whether `run_on_machine` may run `tool` with `arguments` on another computer.
pub fn check_remote(tool: &str, arguments: &Value) -> Result<(), String> {
    if !REMOTE_TOOLS.contains(&tool) {
        return Err(format!("run_on_machine runs only {}", REMOTE_TOOLS.join(", ")));
    }
    if tool == "update_settings" {
        let keys = arguments.pointer("/patch").and_then(Value::as_object).ok_or("Invalid arguments: update_settings takes a patch")?;
        if let Some(k) = keys.keys().find(|k| !REMOTE_SETTINGS.contains(&k.as_str())) {
            return Err(format!("run_on_machine changes only {} on another computer, not {k}", REMOTE_SETTINGS.join(", ")));
        }
    }
    Ok(())
}

/// `get_machine_readiness`, `pair_machine` and `run_on_machine`. `remote_ok` is false inside a
/// `--call` reached over SSH, so a target never reaches on to a third computer.
pub fn local_tool(name: &str, args: &Value, pipe: &str, remote_ok: bool) -> Result<Value, String> {
    let host = args.get("ssh_host").and_then(Value::as_str);
    if host.is_some() && !remote_ok {
        return Err("not_from_a_remote_call".into());
    }
    match (name, host) {
        ("get_machine_readiness", Some(host)) => {
            // Waiting for a change is passed on: the other computer's app does the waiting.
            let wait: serde_json::Map<String, Value> = args.as_object().into_iter().flatten()
                .filter(|(k, _)| matches!(k.as_str(), "wait_seconds" | "since")).map(|(k, v)| (k.clone(), v.clone())).collect();
            let answer = remote_call(&Ssh, host, "get_machine_readiness", &Value::Object(wait))?;
            remember_answer(host, &answer);
            Ok(answer)
        }
        ("get_machine_readiness", None) => match crate::door::call(pipe, &Request::GetReadiness {
            wait_seconds: args.get("wait_seconds").and_then(Value::as_u64).map(|s| s.min(45) as u32),
            since: args.get("since").and_then(Value::as_u64),
        }) {
            Ok(value) => Ok(value),
            Err(e) if matches!(e.as_str(), "not_running" | "unsupported_request" | "door_unavailable" | "timeout") => Ok(readiness_without_app(&e)),
            Err(e) => Err(e),
        },
        ("pair_machine", Some(host)) => {
            let saved = crate::settings::app_dir().and_then(|d| crate::roster::load_passphrase(&d));
            let machine = crate::presence::identity().map(|(_, name)| name.to_string()).unwrap_or_default();
            let replace = args.get("replace_key").and_then(Value::as_bool).unwrap_or(false);
            let paired = pair(&LocalDoor(pipe.into()), &Ssh, host, saved, &machine, replace)?;
            remember_answer(host, &paired);
            Ok(paired)
        }
        ("run_on_machine", Some(host)) => {
            let tool = args.get("tool").and_then(Value::as_str).unwrap_or("");
            let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
            check_remote(tool, &arguments)?;
            let answer = remote_call(&Ssh, host, tool, &arguments)?;
            if tool == "get_machine_readiness" {
                remember_answer(host, &answer);
            }
            Ok(answer)
        }
        _ => Err("Invalid arguments: ssh_host is required".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const KEY_SHOWN: &str = "slopbar-pair-";

    #[test]
    fn keys_are_long_random_and_pass_as_roster_passphrases() {
        let (a, b) = (generate_key(), generate_key());
        assert_ne!(a.0, b.0);
        assert_eq!(a.0.len(), KEY_PREFIX.len() + 52, "256 bits in base32");
        assert!(a.0[KEY_PREFIX.len()..].chars().all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c)));
        assert!(!format!("{a:?}").contains(KEY_SHOWN), "debug output hides the key");
        let dir = crate::test_support::TempDir::new();
        crate::roster::store_passphrase(&dir.0, Some(&a)).unwrap();
        assert_eq!(crate::roster::load_passphrase(&dir.0).map(|p| p.0), Some(a.0));
        assert_eq!(base32(b"foobar"), "mzxw6ytboi", "RFC 4648, lowercase, unpadded");
    }

    #[test]
    fn hosts_are_never_read_as_ssh_options() {
        for good in ["virtualten", "ultra-magnus", "obabalola@virtual-ten", "box.tail1234.ts.net", "100.64.0.7", "fe80::1"] {
            assert!(valid_host(good), "{good}");
        }
        for bad in ["", "-oProxyCommand=calc", "a b", "a;b", "a@b@c", "user@", "a\nb", "$(x)", "a`b", &"x".repeat(254)] {
            assert!(!valid_host(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_remote_command_carries_no_secret_and_quotes_its_arguments() {
        let cmd = remote_command(&["--call-b64", "it's"]);
        assert!(cmd.starts_with("powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand "));
        let b64 = cmd.rsplit(' ').next().unwrap();
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let script = String::from_utf16(&units).unwrap();
        assert!(script.contains("$out = & $exe '--call-b64' 'it''s' | ForEach-Object { $_ }"), "{script}");
        assert!(script.contains("SlopBar\\slopbar-mcp.exe"));
    }

    #[test]
    fn answers_are_ascii_and_read_back() {
        let v = json!({ "ok": { "email": "zoë@example.invalid", "machine": "東京" } });
        let text = ascii_json(&v);
        assert!(text.is_ascii());
        assert_eq!(answer(format!("noise\r\n{text}\r\n").as_bytes()).unwrap(), v["ok"]);
        assert_eq!(answer(b"{\"error\":\"desktop_locked\"}").unwrap_err(), "desktop_locked");
        assert_eq!(answer(b"").unwrap_err(), "remote_no_answer");
    }

    /// A fake app: records requests, answers settings and roster calls.
    struct FakeApp {
        settings: Value,
        saved: Option<&'static str>,
        remotes: Vec<&'static str>,
        seen: RefCell<Vec<Request>>,
    }

    impl FakeApp {
        fn new(settings: Value) -> Self { FakeApp { settings, saved: None, remotes: vec!["dropbox"], seen: RefCell::default() } }
        fn key_set(&self) -> Option<String> {
            self.seen.borrow().iter().find_map(|r| match r { Request::SetRosterPassphrase { passphrase: Some(p) } => Some(p.0.clone()), _ => None })
        }
        fn patches(&self) -> Vec<Value> {
            self.seen.borrow().iter().filter_map(|r| match r { Request::UpdateSettings { patch } => Some(patch.clone()), _ => None }).collect()
        }
    }

    impl Door for FakeApp {
        fn call(&self, request: &Request) -> Result<Value, String> {
            self.seen.borrow_mut().push(request.clone());
            Ok(match request {
                Request::GetSettings => self.settings.clone(),
                Request::ListSharingLocations => json!({ "rclone": { "installed": true, "remotes": self.remotes.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>() } }),
                Request::SetRosterPassphrase { .. } | Request::SyncRoster => json!({ "enabled": true, "location": true, "passphrase_saved": true, "machines": [{ "machine": "OTHER", "accounts": 3 }], "waiting": [{ "provider": "claude", "id": "a", "email": "a@example.invalid" }] }),
                _ => json!({}),
            })
        }
    }

    /// A fake other computer: its app answers readiness, and `--pair-receive` runs `receive`
    /// against `target` with the bytes ssh would carry.
    struct FakeSsh<'a> {
        target: &'a FakeApp,
        running: bool,
        /// A misbehaving target that echoes what it was sent.
        echo: bool,
        commands: RefCell<Vec<String>>,
        inputs: RefCell<Vec<Vec<u8>>>,
    }

    impl Remote for FakeSsh<'_> {
        fn run(&self, _host: &str, command: &str, input: &[u8]) -> Result<Vec<u8>, String> {
            self.commands.borrow_mut().push(command.into());
            self.inputs.borrow_mut().push(input.to_vec());
            let asked = |tool: &str| command == remote_command(&["--call-b64", &base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, json!({ "tool": tool, "arguments": {} }).to_string())]);
            let out = if command == remote_command(&["--pair-receive"]) {
                match receive(self.target, &mut &input[..], self.target.saved.map(|s| Passphrase(s.into()))) {
                    Ok(v) => json!({ "ok": if self.echo { json!({ "leak": String::from_utf8_lossy(input) }) } else { v } }),
                    Err(e) => json!({ "error": e }),
                }
            } else if asked("list_sharing_locations") {
                let mut places = self.target.call(&Request::ListSharingLocations).unwrap();
                places["current"] = json!({ "presence_remote": self.target.settings["presence_remote"] });
                json!({ "ok": places })
            } else {
                let location = self.target.settings["presence_remote"].is_string() || self.target.settings["presence_folder"].is_string();
                json!({ "ok": { "app": { "running": self.running }, "roster": { "location": location } } })
            };
            Ok(ascii_json(&out).into_bytes())
        }
    }

    fn ssh(target: &FakeApp, running: bool) -> FakeSsh<'_> {
        FakeSsh { target, running, echo: false, commands: RefCell::default(), inputs: RefCell::default() }
    }

    #[test]
    fn pairing_moves_a_new_key_only_through_ssh_input() {
        let source = FakeApp::new(json!({ "presence_remote": "dropbox", "presence_folder": null, "account_roster": false }));
        let target = FakeApp::new(json!({ "presence_remote": null, "presence_folder": null, "account_roster": false }));
        let link = ssh(&target, true);
        let result = pair(&source, &link, "virtualten", None, "DESK-1", false).unwrap();
        let key = source.key_set().expect("the source saved a new key");
        assert!(key.starts_with(KEY_PREFIX));
        assert_eq!(target.key_set().as_deref(), Some(key.as_str()), "the target saved the same key");
        assert_eq!(source.patches(), [json!({ "account_roster": true })]);
        assert_eq!(target.patches(), [json!({ "presence_remote": "dropbox", "account_roster": true })]);
        // The key is in ssh's input, never in a command line or the result.
        assert!(link.commands.borrow().iter().all(|c| !c.contains(&key)));
        assert!(link.inputs.borrow().iter().filter(|i| String::from_utf8_lossy(i).contains(&key)).count() == 1, "sent once, as input");
        let shown = result.to_string();
        assert!(!shown.contains(&key) && !shown.contains(KEY_SHOWN), "{shown}");
        assert_eq!(result["key"], "generated");
        assert_eq!(result["target"]["roster"]["waiting"], 1, "a count, not the list of emails");
        assert_eq!(result["target"]["from"], "DESK-1");
        assert!(matches!(source.seen.borrow().last(), Some(Request::SyncRoster)), "the source reads the target's roster at once");
    }

    #[test]
    fn pairing_reuses_the_saved_passphrase() {
        let source = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": true }));
        let target = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": true }));
        let mut target = target;
        target.saved = Some("synthetic saved passphrase");
        let result = pair(&source, &ssh(&target, true), "virtualten", Some(Passphrase("synthetic saved passphrase".into())), "A", false).unwrap();
        assert_eq!(result["key"], "reused");
        assert!(source.key_set().is_none(), "the source keeps its passphrase");
        assert!(source.patches().is_empty() && target.patches().is_empty(), "nothing else to change");
        assert_eq!(target.key_set().as_deref(), Some("synthetic saved passphrase"));
        assert_eq!(result["target"]["replaced_key"], false, "the same key again is no change");
    }

    #[test]
    fn a_target_keeps_the_key_its_other_computers_share_unless_told() {
        let source = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": true }));
        let mut target = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": true }));
        target.saved = Some("synthetic other passphrase");
        let mine = || Some(Passphrase("synthetic saved passphrase".into()));
        assert_eq!(pair(&source, &ssh(&target, true), "virtualten", mine(), "A", false).unwrap_err(), "target: target_has_other_key");
        assert!(target.key_set().is_none() && target.patches().is_empty(), "nothing changed there");
        let result = pair(&source, &ssh(&target, true), "virtualten", mine(), "A", true).unwrap();
        assert_eq!(result["target"]["replaced_key"], true);
        assert_eq!(target.key_set().as_deref(), Some("synthetic saved passphrase"));
    }

    #[test]
    fn pairing_changes_nothing_when_either_side_cannot_take_it() {
        let nowhere = FakeApp::new(json!({ "presence_remote": null, "presence_folder": null }));
        let target = FakeApp::new(json!({}));
        assert_eq!(pair(&nowhere, &ssh(&target, true), "virtualten", None, "A", false).unwrap_err(), "no_sharing_location");
        assert_eq!(pair(&nowhere, &ssh(&target, true), "-oProxyCommand=x", None, "A", false).unwrap_err(), "bad_ssh_host");
        let source = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": false }));
        assert_eq!(pair(&source, &ssh(&target, false), "virtualten", None, "A", false).unwrap_err(), "target_not_running");
        assert!(source.key_set().is_none() && source.patches().is_empty(), "the target is checked before anything changes here");
        // A target without the source's rclone remote: nothing changes on either computer.
        let mut bare = FakeApp::new(json!({ "presence_remote": null, "presence_folder": null }));
        bare.remotes = vec![];
        let err = pair(&source, &ssh(&bare, true), "virtualten", None, "A", false).unwrap_err();
        assert_eq!(err, "target: remote_missing_here: dropbox");
        assert!(bare.key_set().is_none() && bare.patches().is_empty());
        assert!(source.key_set().is_none() && source.patches().is_empty(), "checked before anything changed here");
        // The setting names the remote but rclone there does not have it (or is not installed).
        let mut unset = FakeApp::new(json!({ "presence_remote": "dropbox", "presence_folder": null }));
        unset.remotes = vec![];
        assert_eq!(pair(&source, &ssh(&unset, true), "virtualten", None, "A", false).unwrap_err(), "target: remote_missing_here: dropbox");
        // A folder-sharing source and a target with no location at all.
        let folder = FakeApp::new(json!({ "presence_remote": null, "presence_folder": "C:/Dropbox", "account_roster": true }));
        assert_eq!(pair(&folder, &ssh(&bare, true), "virtualten", None, "A", false).unwrap_err(), "no_sharing_location_here");
    }

    #[test]
    fn a_target_echoing_the_key_never_shows_it() {
        let source = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": true }));
        let target = FakeApp::new(json!({ "presence_remote": "dropbox", "account_roster": true }));
        let mut link = ssh(&target, true);
        link.echo = true;
        let result = pair(&source, &link, "virtualten", Some(Passphrase("synthetic saved passphrase".into())), "A", false).unwrap();
        assert!(!result.to_string().contains("synthetic saved passphrase"), "{result}");
        assert!(result.to_string().contains("[key]"));
    }

    #[test]
    fn receiving_rejects_anything_but_an_offer() {
        let target = FakeApp::new(json!({ "presence_folder": "C:/Dropbox" }));
        for bad in [&b""[..], b"not json", b"{\"format\":\"slopbar-pairing\",\"v\":2,\"key\":\"synthetic key 123\",\"presence_remote\":null,\"from\":\"A\"}",
                    b"{\"format\":\"other\",\"v\":1,\"key\":\"synthetic key 123\",\"presence_remote\":null,\"from\":\"A\"}"] {
            assert!(receive(&target, &mut &bad[..], None).is_err());
        }
        assert!(target.key_set().is_none());
        // A folder-sharing source: the target keeps its own folder.
        let ok = br#"{"format":"slopbar-pairing","v":1,"key":"synthetic key 123","presence_remote":null,"from":"A"}"#;
        receive(&target, &mut &ok[..], None).unwrap();
        assert_eq!(target.patches(), [json!({ "account_roster": true })]);
        let none = FakeApp::new(json!({ "presence_remote": null, "presence_folder": null }));
        assert_eq!(receive(&none, &mut &ok[..], None).unwrap_err(), "no_sharing_location_here");
        let huge = format!(r#"{{"format":"slopbar-pairing","v":1,"key":"{}","presence_remote":null,"from":"A"}}"#, "é".repeat(1024));
        receive(&target, &mut huge.as_bytes(), None).expect("the longest passphrase fits");
        let too_big = vec![b' '; MAX_OFFER as usize + 1];
        assert_eq!(receive(&target, &mut &too_big[..], None).unwrap_err(), "pairing_unreadable");
    }

    #[test]
    fn readiness_says_what_stops_sign_ins() {
        use crate::desktop::{Desktop, State};
        let roster = json!({ "enabled": true, "location": true, "passphrase_saved": true, "unreadable": 0 });
        let at = |state, locked| Some(Desktop { session: 1, state, locked });
        assert_eq!(assess(true, at(State::Active, Some(false)), true, &roster).0, true);
        let (ready, problems, next) = assess(true, None, true, &roster);
        assert!(!ready && problems == ["no_desktop_session"] && next.contains("Nobody is signed in"));
        assert_eq!(assess(true, at(State::Active, Some(true)), true, &roster).1, ["desktop_locked"]);
        assert_eq!(assess(true, at(State::Disconnected, None), false, &roster).1, ["desktop_not_shown", "no_browser"]);
        assert_eq!(assess(false, None, true, &Value::Null).1, ["app_not_running", "no_desktop_session"]);
        let (ready, problems, _) = assess(true, at(State::Active, None), true, &json!({ "enabled": false, "unreadable": 2 }));
        assert!(ready, "sign-ins can still run without a roster");
        assert_eq!(problems, ["roster_off", "no_sharing_location", "no_roster_key", "roster_unreadable"]);
    }

    #[test]
    fn remote_tools_are_limited_and_never_chain() {
        assert_eq!(local_tool("run_on_machine", &json!({ "ssh_host": "virtualten", "tool": "set_roster_passphrase" }), "x", true).unwrap_err().split(' ').next(), Some("run_on_machine"));
        assert_eq!(local_tool("pair_machine", &json!({ "ssh_host": "virtualten" }), "x", false).unwrap_err(), "not_from_a_remote_call");
        assert!(local_tool("pair_machine", &json!({}), "x", true).unwrap_err().contains("ssh_host is required"));
        assert!(check_remote("update_settings", &json!({ "patch": { "presence_remote": "dropbox", "account_roster": true } })).is_ok());
        assert!(check_remote("update_settings", &json!({ "patch": { "phone": { "url": "https://ntfy.sh/x" } } })).unwrap_err().contains("not phone"));
        assert!(check_remote("update_settings", &json!({})).is_err());
        assert!(check_remote("sign_in_all", &json!({ "confirm": true })).is_ok());
        for t in REMOTE_TOOLS {
            assert!(!t.contains("passphrase") && !t.contains("backup") && !t.contains("remove"), "{t}");
        }
    }

    #[test]
    fn paired_hosts_are_remembered_by_machine_id_and_never_options() {
        let dir = crate::test_support::TempDir::new();
        const ID: &str = "22222222-2222-4222-8222-222222222222";
        assert_eq!(host_for(&dir.0, ID), None);
        remember_host(&dir.0, ID, "virtual-ten");
        assert_eq!(host_for(&dir.0, ID).as_deref(), Some("virtual-ten"));
        remember_host(&dir.0, ID, "user@virtual-ten.tailnet");
        assert_eq!(host_for(&dir.0, ID).as_deref(), Some("user@virtual-ten.tailnet"), "the last host it answered on");
        remember_host(&dir.0, ID, "-oProxyCommand=x");
        remember_host(&dir.0, "../../evil", "virtual-ten");
        assert_eq!(host_for(&dir.0, ID).as_deref(), Some("user@virtual-ten.tailnet"));
        assert_eq!(host_for(&dir.0, "../../evil"), None);
        std::fs::write(dir.0.join(HOSTS_FILE), format!("{{\"{ID}\": \"-oProxyCommand=x\"}}")).unwrap();
        assert_eq!(host_for(&dir.0, ID), None, "a changed file never hands ssh an option");
    }
}
