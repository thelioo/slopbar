//! `slopbar-mcp --session-hook`: Claude Code's SessionStart hook. Claude Code runs it with a JSON
//! object on stdin (`session_id`, `transcript_path`, `cwd`, `source`) and the session's own
//! environment, so CLAUDE_CONFIG_DIR is the config dir that session uses (unset: the default
//! login). It tells the running app through the door, which records which account that config
//! dir is signed in to (see `active_log::Log::record_session`); the usage breakdown then
//! attributes the session's transcript to that account exactly.
//!
//! A hook must never get in the way: it prints nothing, gives the door a short deadline, and
//! exits 0 whatever happens (the app closed, bad input). Only the session id, the working folder
//! and the config dir's path cross the door; the app keeps the session id, the folder and the
//! account's slot id.
//!
//! The hook entry for `~/.claude/settings.json` is in the README ("Exact Claude attribution").

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::door::Request;

/// How long the door may take, connecting included.
pub const DOOR_LIMIT: Duration = Duration::from_millis(1500);
/// How long stdin may take to arrive and close.
const STDIN_LIMIT: Duration = Duration::from_secs(2);
/// The hook input is small; anything larger is not Claude Code's.
const MAX_INPUT: u64 = 64 * 1024;

/// A Claude Code session id: a UUID in practice; letters, digits, '-' and '_' are accepted.
pub fn valid_session_id(id: &str) -> bool {
    (8..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Deserialize)]
struct Input {
    session_id: String,
    #[serde(default)]
    cwd: Option<String>,
}

/// The door request for the hook's stdin and CLAUDE_CONFIG_DIR, or None when the input is not a
/// session start SlopBar can use.
pub fn request(input: &[u8], config_dir: Option<String>) -> Option<Request> {
    let input: Input = serde_json::from_slice(input).ok()?;
    if !valid_session_id(&input.session_id) {
        return None;
    }
    let clean = |s: String| -> Option<String> {
        let s = s.trim().to_string();
        (!s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control)).then_some(s)
    };
    Some(Request::RecordSession {
        session_id: input.session_id,
        cwd: input.cwd.and_then(clean),
        config_dir: config_dir.and_then(clean),
    })
}

/// Reads stdin, bounded in size and time: a hook whose stdin never closes still exits.
fn read_stdin(limit: Duration) -> Vec<u8> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::stdin().take(MAX_INPUT).read_to_end(&mut bytes);
        let _ = send.send(bytes);
    });
    receive.recv_timeout(limit).unwrap_or_default()
}

/// Runs the hook. Never fails and prints nothing.
pub fn run() {
    let input = read_stdin(STDIN_LIMIT);
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let Some(request) = request(&input, config_dir) else { return };
    let reply = crate::door::call_within(&crate::door::pipe_name(), &request, DOOR_LIMIT);
    // SLOPBAR_HOOK_DEBUG=1 says on stderr what happened (Claude Code shows stderr only in debug).
    if std::env::var_os("SLOPBAR_HOOK_DEBUG").is_some() {
        eprintln!("slopbar session hook: {}", reply.map_or_else(|e| e, |v| v.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "0b5e7c1a-2f3d-4e5f-8a9b-0c1d2e3f4a5b";

    #[test]
    fn builds_a_request_from_claude_codes_hook_input() {
        let input = format!(r#"{{"session_id":"{SESSION}","transcript_path":"C:\\Users\\test\\.claude\\projects\\C--code-app\\{SESSION}.jsonl","cwd":"C:\\code\\app","hook_event_name":"SessionStart","source":"startup"}}"#);
        match request(input.as_bytes(), Some("C:\\Users\\test\\AppData\\Roaming\\dev.thelio.slopbar\\profiles\\claude-1".into())) {
            Some(Request::RecordSession { session_id, cwd, config_dir }) => {
                assert_eq!(session_id, SESSION);
                assert_eq!(cwd.as_deref(), Some("C:\\code\\app"));
                assert!(config_dir.is_some_and(|d| d.ends_with("claude-1")));
            }
            other => panic!("{other:?}"),
        }
        // No config dir (the default login), an empty one and a missing cwd are fine.
        let plain = format!(r#"{{"session_id":"{SESSION}"}}"#);
        assert!(matches!(request(plain.as_bytes(), Some("  ".into())), Some(Request::RecordSession { cwd: None, config_dir: None, .. })));
    }

    #[test]
    fn refuses_input_that_is_not_a_session_start() {
        for bad in [&b""[..], b"not json", br#"{"cwd":"C:\\x"}"#, br#"{"session_id":"..\\..\\x"}"#, br#"{"session_id":"short"}"#, br#"{"session_id":42}"#] {
            assert!(request(bad, None).is_none(), "{}", String::from_utf8_lossy(bad));
        }
        assert!(valid_session_id(SESSION));
        assert!(!valid_session_id(&"a".repeat(129)));
        // A control character in the folder drops the folder, not the session.
        let input = format!("{{\"session_id\":\"{SESSION}\",\"cwd\":\"C:\\\\a\\u0007b\"}}");
        assert!(matches!(request(input.as_bytes(), None), Some(Request::RecordSession { cwd: None, .. })));
    }

    #[test]
    fn exits_quietly_when_the_app_is_not_running() {
        let started = std::time::Instant::now();
        let request = request(format!(r#"{{"session_id":"{SESSION}"}}"#).as_bytes(), None).unwrap();
        let pipe = format!(r"\\.\pipe\slopbar-door-test-hook-{}", std::process::id());
        assert!(crate::door::call_within(&pipe, &request, DOOR_LIMIT).is_err());
        assert!(started.elapsed() < Duration::from_secs(1), "a missing app answers at once");
    }
}
