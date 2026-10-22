//! Phone alerts: a few high-value notifications (a plan renewing tomorrow, unexpected use, a
//! runaway agent, a limit reached) also sent to a push service the user chose. Off by default.
//!
//! The first provider is ntfy: a topic URL on ntfy.sh or a self-hosted server. Messages are
//! published as JSON to the server's root, so titles in any language arrive intact. An optional
//! access token is kept with Windows DPAPI next to the vault, never in settings.json. Unless the
//! user ticks "include account names", the text names only the provider ("A Codex plan renews
//! tomorrow"): the push service and the phone's lock screen never see an alias or email.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::settings::Phone;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TOKEN: usize = 256;
const MAX_TEXT: usize = 1024;
const TOKEN_FILE: &str = "phone-token.bin";

/// What a phone alert is about; the user picks which kinds are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A paid plan renews within a day.
    Renewal,
    /// An account is used where none of the user's computers is.
    UnexpectedUse,
    /// An agent is burning usage unusually fast.
    Runaway,
    /// A usage limit was reached.
    LimitReached,
    /// The "Send test" button.
    Test,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Renewal => "calendar",
            Kind::UnexpectedUse => "warning",
            Kind::Runaway => "rotating_light",
            Kind::LimitReached => "stop_sign",
            Kind::Test => "white_check_mark",
        }
    }

    fn priority(self) -> u8 {
        match self {
            Kind::UnexpectedUse | Kind::Runaway => 4,
            _ => 3,
        }
    }

    fn selected(self, phone: &Phone) -> bool {
        match self {
            Kind::Renewal => phone.kinds.renewal,
            Kind::UnexpectedUse => phone.kinds.unexpected_use,
            Kind::Runaway => phone.kinds.runaway,
            Kind::LimitReached => phone.kinds.limit_reached,
            Kind::Test => true,
        }
    }
}

/// A topic URL as stored: `https://ntfy.sh/my-topic` or a self-hosted server, possibly under a
/// path. Plain http only for this machine or the local network, so a token never crosses the
/// internet unencrypted. No user name, query or fragment.
pub fn normalize_url(text: &str) -> Result<String, String> {
    let refuse = || "phone.url must be an ntfy topic URL such as https://ntfy.sh/your-topic (http only for a server on this machine or the local network)".to_string();
    let text = text.trim();
    if text.len() > 512 || text.chars().any(char::is_control) { return Err(refuse()); }
    let url = Url::parse(text).map_err(|_| refuse())?;
    let local = match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost" || d.ends_with(".local") || d.ends_with(".lan") || d.ends_with(".home.arpa"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => return Err(refuse()),
    };
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
    if !scheme_ok || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(refuse());
    }
    let topic = url.path_segments().and_then(|s| s.filter(|p| !p.is_empty()).last()).ok_or_else(refuse)?;
    if topic.len() > 64 || !topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(refuse());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Where to publish and the topic: ntfy takes JSON at the server root (its path, for a server
/// under one).
fn endpoint(url: &str) -> Result<(Url, String), String> {
    let url = Url::parse(&normalize_url(url)?).map_err(|_| "phone_url_invalid")?;
    let mut segments: Vec<String> = url.path_segments().into_iter().flatten().filter(|s| !s.is_empty()).map(String::from).collect();
    let topic = segments.pop().ok_or("phone_url_invalid")?;
    let mut base = url.clone();
    base.set_path(&format!("/{}", segments.iter().map(|s| format!("{s}/")).collect::<String>()));
    Ok((base, topic))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub kind: Kind,
    pub title: String,
    pub body: String,
}

fn clip(s: &str) -> String { s.chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_TEXT).collect() }

/// The message to send for a notification, or None when phone alerts are off, have no URL, or
/// this kind is not selected. `named` is the notification as shown on this PC; `generic` the same
/// news without account names, sent unless the user chose to include them.
pub fn compose(phone: &Phone, kind: Kind, named: (&str, &str), generic: (&str, &str)) -> Option<Message> {
    if phone.url.is_none() || (kind != Kind::Test && !phone.enabled) || !kind.selected(phone) { return None; }
    let (title, body) = if phone.include_names { named } else { generic };
    Some(Message { kind, title: clip(title), body: clip(body) })
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        // A redirect could carry the token to another host.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

/// Publishes one message. Errors are codes for the Settings window, never response bodies.
pub async fn send(client: &reqwest::Client, url: &str, token: Option<&str>, message: &Message) -> Result<(), String> {
    let (base, topic) = endpoint(url)?;
    let mut body = serde_json::json!({
        "topic": topic,
        "title": message.title,
        "message": message.body,
        "tags": [message.kind.tag()],
        "priority": message.kind.priority(),
    });
    if message.body.is_empty() { body["message"] = message.title.clone().into(); }
    let mut request = client.post(base).json(&body);
    if let Some(token) = token { request = request.bearer_auth(token); }
    let response = request.send().await.map_err(|e| if e.is_timeout() { "phone_timeout" } else { "phone_unreachable" })?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err("phone_unauthorized".into()),
        429 => Err("phone_rate_limited".into()),
        _ => Err("phone_rejected".into()),
    }
}

pub fn token_path(dir: &Path) -> PathBuf { dir.join(TOKEN_FILE) }

pub fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN && token.chars().all(|c| c.is_ascii_graphic())
}

pub fn load_token(dir: &Path) -> Option<String> {
    let bytes = crate::private_store::load_secret(&token_path(dir)).ok()?;
    String::from_utf8(bytes).ok().filter(|t| valid_token(t))
}

/// Saves the access token with DPAPI, or deletes it for None (or an empty one).
pub fn store_token(dir: &Path, token: Option<&str>) -> Result<(), String> {
    match token.map(str::trim).filter(|t| !t.is_empty()) {
        None => match std::fs::remove_file(token_path(dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err("phone_token_storage".into()),
            _ => Ok(()),
        },
        Some(t) if !valid_token(t) => Err("phone_token_invalid".into()),
        Some(t) => crate::private_store::save_secret(&token_path(dir), t.as_bytes()).map_err(|_| "phone_token_storage".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{PhoneKinds, Settings};
    use crate::test_support::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TOKEN: &str = "tk_SYNTHETICPHONETOKEN0042";

    /// A one-request HTTP server on this machine: answers with `status` and returns what it got.
    async fn mock(status: u16) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if got.len() >= end + 4 + length { break; }
                }
                if n == 0 { break; }
            }
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
            socket.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (url, served)
    }

    fn phone(url: &str) -> Phone {
        Phone { enabled: true, url: Some(url.into()), include_names: false, kinds: PhoneKinds::default() }
    }

    fn local() -> reqwest::Client { reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap() }

    #[test]
    fn off_by_default_and_generic_unless_names_are_chosen() {
        let settings = Settings::default();
        assert!(!settings.phone.enabled && settings.phone.url.is_none() && !settings.phone.include_names);
        let named = ("Codex plan renews in 20h", "work@example.invalid · $200/month");
        let generic = ("Codex plan renews tomorrow", "A Codex plan renews tomorrow.");
        assert_eq!(compose(&settings.phone, Kind::Renewal, named, generic), None, "nothing is sent while off");
        let mut on = phone("https://ntfy.sh/synthetic-topic");
        let m = compose(&on, Kind::Renewal, named, generic).unwrap();
        assert_eq!((m.title.as_str(), m.body.as_str()), generic);
        assert!(!m.body.contains("example.invalid"));
        on.include_names = true;
        assert_eq!(compose(&on, Kind::Renewal, named, generic).unwrap().body, named.1);
        on.kinds.renewal = false;
        assert_eq!(compose(&on, Kind::Renewal, named, generic), None, "an unselected kind is not sent");
        on.enabled = false;
        assert!(compose(&on, Kind::Test, ("t", "b"), ("t", "b")).is_some(), "a test works before turning alerts on");
    }

    #[test]
    fn topic_urls_are_checked() {
        assert_eq!(normalize_url(" https://ntfy.sh/my-topic/ ").unwrap(), "https://ntfy.sh/my-topic");
        assert_eq!(normalize_url("http://192.168.1.20:8080/alerts").unwrap(), "http://192.168.1.20:8080/alerts");
        assert!(normalize_url("https://push.example.invalid/ntfy/alerts_1").is_ok());
        for bad in ["http://ntfy.sh/topic", "https://ntfy.sh/", "https://ntfy.sh", "https://user:pw@ntfy.sh/t", "https://ntfy.sh/t?auth=x", "https://ntfy.sh/t#x", "ftp://ntfy.sh/t", "ntfy.sh/topic", "https://ntfy.sh/bad topic", "https://ntfy.sh/a\nb"] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
        let (base, topic) = endpoint("https://push.example.invalid/ntfy/alerts").unwrap();
        assert_eq!((base.as_str(), topic.as_str()), ("https://push.example.invalid/ntfy/", "alerts"));
    }

    #[tokio::test]
    async fn publishes_json_with_the_token_to_a_local_server() {
        let (server, served) = mock(200).await;
        let url = format!("{server}/synthetic-topic");
        let m = compose(&phone(&url), Kind::UnexpectedUse, ("x", "y"), ("Claude used outside your computers", "A Claude account is being used where none of your computers is signed in.")).unwrap();
        send(&local(), &url, Some(TOKEN), &m).await.unwrap();
        let got = served.await.unwrap();
        assert!(got.starts_with("POST / HTTP/1.1"), "{got}");
        assert!(got.to_ascii_lowercase().contains(&format!("authorization: bearer {}", TOKEN.to_ascii_lowercase())));
        let body: serde_json::Value = serde_json::from_str(&got[got.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["topic"], "synthetic-topic");
        assert_eq!(body["title"], "Claude used outside your computers");
        assert_eq!((body["priority"].as_u64(), body["tags"][0].as_str()), (Some(4), Some("warning")));
    }

    #[tokio::test]
    async fn failures_are_codes() {
        let (server, served) = mock(403).await;
        let url = format!("{server}/t");
        let m = Message { kind: Kind::Test, title: "SlopBar test".into(), body: String::new() };
        assert_eq!(send(&local(), &url, None, &m).await.unwrap_err(), "phone_unauthorized");
        let got = served.await.unwrap();
        assert!(!got.to_ascii_lowercase().contains("authorization"), "no token, no header");
        assert!(got.contains("\"message\":\"SlopBar test\""), "an empty body repeats the title");
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert_eq!(send(&local(), &format!("http://127.0.0.1:{port}/t"), None, &m).await.unwrap_err(), "phone_unreachable");
    }

    #[test]
    fn the_token_lives_in_its_own_protected_file() {
        let dir = TempDir::new();
        assert_eq!(load_token(&dir.0), None);
        store_token(&dir.0, Some(TOKEN)).unwrap();
        assert_eq!(load_token(&dir.0).as_deref(), Some(TOKEN));
        #[cfg(windows)]
        assert!(!String::from_utf8_lossy(&std::fs::read(token_path(&dir.0)).unwrap()).contains(TOKEN), "encrypted at rest");
        assert_eq!(store_token(&dir.0, Some("has space")).unwrap_err(), "phone_token_invalid");
        store_token(&dir.0, None).unwrap();
        assert_eq!(load_token(&dir.0), None);
        store_token(&dir.0, None).unwrap();
        // settings.json has no place for it: no token field under phone, and never the secret itself
        // (other settings may still mention tokens, e.g. burn's token_factor).
        let settings = serde_json::to_value(&Settings { phone: phone("https://ntfy.sh/t"), ..Settings::default() }).unwrap();
        assert!(!settings["phone"].to_string().contains("token"), "settings.json has no place for it");
        assert!(!settings.to_string().contains(TOKEN));
    }
}
