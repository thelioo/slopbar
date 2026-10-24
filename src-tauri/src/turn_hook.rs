//! `slopbar-mcp --turn-hook`: Claude Code's UserPromptSubmit and Stop hook, for graceful switching
//! (see `wind_down`). It tells the running app through the door that a session started or ended
//! a turn, and prints what the app answers: nothing at all unless the session's account is winding
//! down, then context for the prompt or a one-time block of the stop with the wrap-up instruction.
//!
//! Like the SessionStart hook it must never get in the way: a short door deadline, output only
//! when the app asks for it, exit 0 whatever happens. Only the session id, the event and the
//! config dir's path cross the door, never the prompt.
//!
//! The hook entries for `~/.claude/settings.json` are in the README ("Graceful switching").

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::door::Request;
use crate::session_hook::{valid_session_id, DOOR_LIMIT};

const STDIN_LIMIT: Duration = Duration::from_secs(2);
/// A prompt can be long (pasted files); the input is read in full so its JSON parses.
const MAX_INPUT: u64 = 32 * 1024 * 1024;

#[derive(Deserialize)]
struct Input {
    session_id: String,
    hook_event_name: String,
}

/// The door request for the hook's stdin and CLAUDE_CONFIG_DIR, or None when it is not a turn
/// event SlopBar uses.
pub fn request(input: &[u8], config_dir: Option<String>) -> Option<Request> {
    let input: Input = serde_json::from_slice(input).ok()?;
    if !valid_session_id(&input.session_id) {
        return None;
    }
    let event = match input.hook_event_name.as_str() {
        "UserPromptSubmit" => "prompt",
        "Stop" => "stop",
        _ => return None,
    };
    let config_dir = config_dir.map(|d| d.trim().to_string()).filter(|d| !d.is_empty() && d.len() <= 1024 && !d.chars().any(char::is_control));
    Some(Request::Turn { session_id: input.session_id, event: event.into(), config_dir })
}

/// What Claude Code reads from the hook's stdout for the app's reply, or None for no output.
pub fn output(event: &str, reply: &serde_json::Value) -> Option<String> {
    let text = |key: &str| reply.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let value = match (event, text("context"), text("block")) {
        ("prompt", Some(context), _) => serde_json::json!({
            "hookSpecificOutput": { "hookEventName": "UserPromptSubmit", "additionalContext": context }
        }),
        ("stop", _, Some(reason)) => serde_json::json!({ "decision": "block", "reason": reason }),
        _ => return None,
    };
    Some(value.to_string())
}

fn read_stdin(limit: Duration) -> Vec<u8> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::stdin().take(MAX_INPUT).read_to_end(&mut bytes);
        let _ = send.send(bytes);
    });
    receive.recv_timeout(limit).unwrap_or_default()
}

/// Runs the hook. Never fails; prints only what the app asks for.
pub fn run() {
    let input = read_stdin(STDIN_LIMIT);
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let Some(request) = request(&input, config_dir) else { return };
    let Request::Turn { event, .. } = &request else { return };
    let event = event.clone();
    let reply = crate::door::call_within(&crate::door::pipe_name(), &request, DOOR_LIMIT);
    if std::env::var_os("SLOPBAR_HOOK_DEBUG").is_some() {
        eprintln!("slopbar turn hook: {}", reply.as_ref().map_or_else(|e| e.clone(), |v| v.to_string()));
    }
    if let Some(out) = reply.ok().and_then(|r| output(&event, &r)) {
        println!("{out}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "0b5e7c1a-2f3d-4e5f-8a9b-0c1d2e3f4a5b";

    #[test]
    fn reads_prompt_and_stop_events_only() {
        let prompt = format!(r#"{{"session_id":"{SESSION}","hook_event_name":"UserPromptSubmit","prompt":"hi","cwd":"C:\\code"}}"#);
        assert!(matches!(request(prompt.as_bytes(), None), Some(Request::Turn { ref event, config_dir: None, .. }) if event == "prompt"));
        let stop = format!(r#"{{"session_id":"{SESSION}","hook_event_name":"Stop","stop_hook_active":true}}"#);
        assert!(matches!(request(stop.as_bytes(), Some("D:\\p".into())), Some(Request::Turn { ref event, config_dir: Some(_), .. }) if event == "stop"));
        let sub = format!(r#"{{"session_id":"{SESSION}","hook_event_name":"SubagentStop"}}"#);
        assert!(request(sub.as_bytes(), None).is_none(), "a subagent's stop is not the session's");
        assert!(request(b"{\"session_id\":\"x\",\"hook_event_name\":\"Stop\"}", None).is_none());
        assert!(request(b"not json", None).is_none());
    }

    #[test]
    fn prints_claude_codes_shapes_and_nothing_otherwise() {
        let reply = serde_json::json!({ "context": "rotating", "block": null });
        let out: serde_json::Value = serde_json::from_str(&output("prompt", &reply).unwrap()).unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert_eq!(out["hookSpecificOutput"]["additionalContext"], "rotating");
        let reply = serde_json::json!({ "block": "wrap up" });
        let out: serde_json::Value = serde_json::from_str(&output("stop", &reply).unwrap()).unwrap();
        assert_eq!(out["decision"], "block");
        assert_eq!(out["reason"], "wrap up");
        assert!(output("stop", &serde_json::json!({})).is_none());
        assert!(output("prompt", &serde_json::json!({ "block": "x" })).is_none(), "a prompt is never blocked");
    }
}
