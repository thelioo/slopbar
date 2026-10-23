//! A Codex sign-in whose page SlopBar opens itself (see `signin`). `codex login` always opens the
//! default browser on Windows (it ignores `BROWSER` there), so this runs the CLI's own app server
//! instead, the way the IDE extensions sign in: `account/login/start` returns the provider's
//! authorization URL without opening anything, the app server keeps its local callback listener,
//! and `account/login/completed` says when the user finished. The login lands in the isolated
//! `CODEX_HOME` exactly as `codex login` writes it, and is imported from there as before.
//!
//! No email is prefilled: the authorization page's `login_hint` skips straight to a password
//! form, hiding "Continue with Google".

use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

/// The action run in the isolated folder (see `login::command`).
pub const ACTION: &str = "codex app-server";

/// How long the app server may take to hand over the URL; it can be a cold start of the CLI.
const START_LIMIT: Duration = Duration::from_secs(45);

/// A started sign-in: its outcome arrives on `done` (true once the user signed in). Holds the app
/// server's stdin, whose closing ends the server.
pub struct Started {
    pub done: mpsc::Receiver<bool>,
    _stdin: ChildStdin,
}

enum Message {
    Response { id: u64, result: Option<Value>, error: bool },
    Completed { login_id: Option<String>, success: bool },
}

fn parse(line: &str) -> Option<Message> {
    let v: Value = serde_json::from_str(line).ok()?;
    if let Some(id) = v["id"].as_u64() {
        if v.get("method").is_none() {
            return Some(Message::Response { id, result: v.get("result").cloned(), error: v.get("error").is_some_and(|e| !e.is_null()) });
        }
    }
    (v["method"] == "account/login/completed").then(|| Message::Completed {
        login_id: v["params"]["loginId"].as_str().map(str::to_owned),
        success: v["params"]["success"].as_bool() == Some(true),
    })
}

/// The URL and login id from `account/login/start`'s result, when it is a ChatGPT sign-in whose
/// URL is a provider page.
fn login_started(result: &Value) -> Option<(String, String)> {
    let url = result["authUrl"].as_str()?;
    (result["type"] == "chatgpt" && crate::signin::allowed_url(url)).then(|| (url.to_owned(), result["loginId"].as_str().unwrap_or_default().to_owned()))
}

fn send(stdin: &mut ChildStdin, message: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    stdin.write_all(&line)?;
    stdin.flush()
}

/// Asks the app server for the sign-in URL and hands it to `open`. Fails when the server does not
/// answer as expected (an older Codex, say); the caller then falls back to `codex login`.
pub fn start(mut stdin: ChildStdin, stdout: ChildStdout, open: impl FnOnce(&str) -> Result<(), String>) -> Result<Started, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(m) = parse(&line) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        }
    });
    let deadline = std::time::Instant::now() + START_LIMIT;
    let response = |want: u64| -> Result<Value, String> {
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(Message::Response { id, result, error }) if id == want => {
                    return if error { Err("monitor_login_failed".into()) } else { result.ok_or_else(|| "monitor_login_failed".into()) };
                }
                Ok(_) => continue,
                Err(_) => return Err("monitor_login_failed".into()),
            }
        }
    };
    let io = |_| "monitor_login_failed".to_string();
    send(&mut stdin, &json!({ "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "slopbar", "title": "SlopBar", "version": env!("CARGO_PKG_VERSION") } } })).map_err(io)?;
    response(1)?;
    send(&mut stdin, &json!({ "method": "initialized" })).map_err(io)?;
    send(&mut stdin, &json!({ "id": 2, "method": "account/login/start", "params": { "type": "chatgpt" } })).map_err(io)?;
    let (url, login_id) = login_started(&response(2)?).ok_or("monitor_login_failed")?;
    open(&url)?;
    let (done_tx, done) = mpsc::channel();
    std::thread::spawn(move || {
        // The server exiting (the reader closes the channel) without completing is a failure.
        let success = rx.iter().find_map(|m| match m {
            Message::Completed { login_id: id, success } if id.as_deref().is_none_or(|id| id == login_id) => Some(success),
            _ => None,
        });
        let _ = done_tx.send(success == Some(true));
    });
    Ok(Started { done, _stdin: stdin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_app_server_messages_it_needs() {
        assert!(matches!(parse(r#"{"id":1,"result":{"userAgent":"x"}}"#), Some(Message::Response { id: 1, result: Some(_), error: false })));
        assert!(matches!(parse(r#"{"id":2,"error":{"code":-32600,"message":"no"}}"#), Some(Message::Response { id: 2, error: true, .. })));
        assert!(matches!(parse(r#"{"method":"account/login/completed","params":{"loginId":"L","success":true,"error":null}}"#),
            Some(Message::Completed { success: true, .. })));
        assert!(matches!(parse(r#"{"method":"account/login/completed","params":{"loginId":"L","success":false,"error":"denied"}}"#),
            Some(Message::Completed { success: false, .. })));
        // A request from the server to the client carries an id and a method: not an answer.
        assert!(parse(r#"{"id":7,"method":"item/commandExecution/requestApproval","params":{}}"#).is_none());
        assert!(parse(r#"{"method":"account/updated","params":{}}"#).is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn only_a_provider_url_starts_the_sign_in() {
        let ok = json!({ "type": "chatgpt", "loginId": "L", "authUrl": "https://auth.openai.com/oauth/authorize?state=s" });
        assert_eq!(login_started(&ok), Some(("https://auth.openai.com/oauth/authorize?state=s".into(), "L".into())));
        assert_eq!(login_started(&json!({ "type": "chatgpt", "loginId": "L", "authUrl": "https://auth.example.invalid/" })), None);
        assert_eq!(login_started(&json!({ "type": "apiKey" })), None);
    }
}
