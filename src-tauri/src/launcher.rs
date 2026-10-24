//! The `slopbar-run` launcher's link to the app (see `pins` for the launcher itself): when no
//! project pin applies, the launcher runs `slopbar-mcp --pick <provider> --pid <its pid>` and
//! uses the account the app picks (recommend_account's choice among accounts nobody else leased),
//! then `slopbar-mcp --release <lease id>` once the command exits.
//!
//! `--pick` prints one JSON line, `{"id", "label", "dir", "env_var", "lease_id", "note"}`, where
//! `dir` is the config dir to set (null: the CLI's current sign-in is the pick) and `lease_id` the
//! lease held for the command's lifetime. On any failure it prints nothing and the launcher runs
//! the command on the CLI's current sign-in. Exit code 0 either way.

use std::time::Duration;

use serde_json::{json, Value};

use crate::accounts::Provider;
use crate::door::Request;

/// Picking can sign a project profile in; it stays well under a second in practice.
const DOOR_LIMIT: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq)]
pub enum Helper {
    Pick { provider: Provider, pid: u32, purpose: Option<String>, expected_percent: Option<f64> },
    Release { lease_id: String },
}

/// Parses `--pick <provider> --pid <n> [--purpose <text>] [--percent <n>]` or
/// `--release <lease id>`.
pub fn parse(args: &[String]) -> Result<Helper, String> {
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("--release") => {
            let lease_id = it.next().filter(|s| !s.is_empty()).ok_or("--release needs a lease id")?.clone();
            Ok(Helper::Release { lease_id })
        }
        Some("--pick") => {
            let provider = match it.next().map(String::as_str) {
                Some("claude") => Provider::Claude,
                Some("codex") => Provider::Codex,
                _ => return Err("--pick needs claude or codex".into()),
            };
            let (mut pid, mut purpose, mut expected_percent) = (None, None, None);
            while let Some(flag) = it.next() {
                let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
                match flag.as_str() {
                    "--pid" => pid = Some(value.parse::<u32>().map_err(|_| "--pid must be a process id")?),
                    "--purpose" => purpose = Some(value.clone()).filter(|p| !p.trim().is_empty()),
                    "--percent" => expected_percent = Some(value.parse::<f64>().ok().filter(|p| (0.0..=100.0).contains(p)).ok_or("--percent must be 0-100")?),
                    other => return Err(format!("unknown option {other}")),
                }
            }
            Ok(Helper::Pick { provider, pid: pid.ok_or("--pick needs --pid")?, purpose, expected_percent })
        }
        _ => Err("usage: --pick <claude|codex> --pid <n> [--purpose <text>] [--percent <n>] | --release <lease id>".into()),
    }
}

/// The line `--pick` prints for the app's PickAccount reply.
pub fn pick_line(reply: &Value) -> String {
    json!({
        "id": reply["id"], "label": reply["label"], "dir": reply["dir"], "env_var": reply["env_var"],
        "lease_id": reply["lease_id"], "note": reply["note"],
    }).to_string()
}

/// `slopbar-mcp --pick …` / `--release …`. Returns the process exit code.
pub fn helper(args: &[String]) -> i32 {
    use std::io::Write;
    let debug = std::env::var_os("SLOPBAR_RUN_DEBUG").is_some();
    let request = match parse(args) {
        Ok(Helper::Pick { provider, pid, purpose, expected_percent }) => Request::PickAccount { provider, pid, purpose, expected_percent },
        Ok(Helper::Release { lease_id }) => Request::ReleaseLease { lease_id },
        Err(e) => {
            if debug { eprintln!("slopbar-run: {e}"); }
            return 0;
        }
    };
    let pick = matches!(request, Request::PickAccount { .. });
    match crate::door::call_within(&crate::door::pipe_name(), &request, DOOR_LIMIT) {
        Ok(reply) if pick => { let _ = writeln!(std::io::stdout(), "{}", pick_line(&reply)); }
        Ok(_) => {}
        Err(e) => if debug { eprintln!("slopbar-run: {e}"); },
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> { s.iter().map(|a| a.to_string()).collect() }

    #[test]
    fn parses_the_launchers_calls() {
        assert_eq!(parse(&args(&["--pick", "codex", "--pid", "42", "--purpose", "nightly tests", "--percent", "12.5"])).unwrap(),
            Helper::Pick { provider: Provider::Codex, pid: 42, purpose: Some("nightly tests".into()), expected_percent: Some(12.5) });
        assert_eq!(parse(&args(&["--pick", "claude", "--pid", "7", "--purpose", " "])).unwrap(),
            Helper::Pick { provider: Provider::Claude, pid: 7, purpose: None, expected_percent: None });
        assert_eq!(parse(&args(&["--release", "lease-1"])).unwrap(), Helper::Release { lease_id: "lease-1".into() });
        for bad in [&["--pick", "gemini", "--pid", "1"][..], &["--pick", "claude"], &["--pick", "claude", "--pid"], &["--pick", "claude", "--pid", "x"],
                    &["--pick", "claude", "--pid", "1", "--percent", "300"], &["--pick", "claude", "--pid", "1", "--force", "1"], &["--release"], &[]] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn prints_only_what_the_launcher_reads() {
        let reply = json!({ "id": "user-1:org-1", "label": "Spare", "dir": "C:\\p\\claude-1", "env_var": "CLAUDE_CONFIG_DIR", "lease_id": "lease-1", "note": null, "ranking": [1, 2] });
        let line: Value = serde_json::from_str(&pick_line(&reply)).unwrap();
        assert_eq!(line["dir"], "C:\\p\\claude-1");
        assert_eq!(line["lease_id"], "lease-1");
        assert!(line.get("ranking").is_none());
    }

    #[test]
    fn a_helper_without_the_app_prints_nothing_and_succeeds() {
        assert_eq!(helper(&args(&["--bogus"])), 0);
    }
}
