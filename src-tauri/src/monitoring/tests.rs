use super::*;
use crate::test_support::{slot, TempDir};
use std::collections::VecDeque;
use std::sync::Mutex as SyncMutex;

struct Scripted {
    replies: SyncMutex<VecDeque<Result<Reply, Failure>>>,
    refreshes: SyncMutex<Vec<String>>,
}

impl Scripted {
    fn new(replies: impl IntoIterator<Item = Result<Reply, Failure>>) -> Self {
        Self {
            replies: SyncMutex::new(replies.into_iter().collect()),
            refreshes: SyncMutex::new(vec![]),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.refreshes.lock().unwrap().clone()
    }
}

impl Refresh for Scripted {
    async fn refresh(&self, session: &Session) -> Result<Reply, Failure> {
        self.refreshes
            .lock()
            .unwrap()
            .push(session.refresh_token.clone());
        // Allow a competing poll to enter while this simulated network call is in flight.
        tokio::task::yield_now().await;
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected refresh")
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}
fn reply(access: &str, refresh: Option<&str>) -> Result<Reply, Failure> {
    Ok(Reply {
        access_token: access.into(),
        refresh_token: refresh.map(String::from),
        expires_in: Some(3600),
        token_type: Some("Bearer".into()),
        scope: None,
        refresh_token_expires_in: None,
    })
}
fn cli() -> Slot {
    slot("account:org", "cli-access", "cli-refresh", 0)
}
fn login() -> Slot {
    slot("account:org", "monitor-access-1", "monitor-refresh-1", 0)
}

#[tokio::test]
async fn diagnostic_refresh_trace_is_secret_free_and_recording_failure_cannot_break_renewal() {
    let dir = TempDir::new();
    let journal = dir.0.join("journal");
    // A directory at the journal-file path simulates storage failure.
    std::fs::create_dir(&journal).unwrap();
    let recorder = std::sync::Arc::new(crate::diagnostics::Recorder::new(journal));
    recorder.start();
    let monitor = Monitor::new(dir.0.join("sessions")).with_diagnostics(recorder.clone());
    let expected = cli();
    let transport = Scripted::new([Err(Failure::Transient), reply("PRIVATE-new-access", Some("PRIVATE-new-refresh"))]);
    monitor.connect(&expected, &login(), &[]).await.unwrap();
    assert_eq!(monitor.token_with(&expected, &transport, None, now()).await.err().as_deref(), Some("monitor_network"));
    assert_eq!(monitor.token_with(&expected, &transport, None, now() + chrono::Duration::minutes(6)).await.unwrap().0, "PRIVATE-new-access");
    // Storage failures are recorded asynchronously; wait for this write before
    // asserting its result rather than racing the diagnostic worker.
    recorder.flush();
    let report = String::from_utf8(recorder.report(&crate::model::Snapshot::default(), &crate::settings::Settings::default()).unwrap()).unwrap();
    for private in ["PRIVATE", "monitor-access-1", "monitor-refresh-1", "cli-refresh", "account:org", "test@example.invalid"] { assert!(!report.contains(private)); }
    let data: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert!(data["storage_errors"].as_u64().unwrap() > 0);
    let outcomes: Vec<_> = data["events"].as_array().unwrap().iter().filter_map(|r| r["event"]["outcome"].as_str()).collect();
    assert_eq!(outcomes, ["started", "transient", "started", "success"]);
}

#[tokio::test]
async fn inactive_account_renews_repeatedly_across_restart_without_using_cli_session() {
    let dir = TempDir::new();
    let expected = cli();
    let original = serde_json::to_vec(&expected).unwrap();
    let transport = Scripted::new([
        reply("access-2", Some("refresh-2")),
        reply("access-3", Some("refresh-3")),
    ]);
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&expected, &login(), &[]).await.unwrap();
    assert_eq!(
        monitor
            .token_with(&expected, &transport, None, now())
            .await
            .unwrap()
            .0,
        "access-2"
    );
    drop(monitor);
    // Neither a recent CLI login nor a running CLI exists. Use the saved rotated token.
    let restarted = Monitor::new(dir.0.clone());
    assert_eq!(
        restarted
            .token_with(
                &expected,
                &transport,
                None,
                now() + chrono::Duration::days(30)
            )
            .await
            .unwrap()
            .0,
        "access-3"
    );
    assert_eq!(transport.calls(), ["monitor-refresh-1", "refresh-2"]);
    assert_eq!(serde_json::to_vec(&expected).unwrap(), original);
    let saved: Session =
        serde_json::from_slice(&private_store::load_secret(&restarted.path(&expected)).unwrap())
            .unwrap();
    assert_eq!(saved.refresh_token, "refresh-3");
}

#[tokio::test]
async fn concurrent_unauthorized_polls_rotate_only_once() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    let mut fresh = login();
    fresh.data["credentials"]["claudeAiOauth"]["expiresAt"] =
        json!((now() + chrono::Duration::hours(1)).timestamp_millis());
    let expected = cli();
    monitor.connect(&expected, &fresh, &[]).await.unwrap();
    let transport = Scripted::new([reply("access-2", Some("refresh-2"))]);
    let (a, b) = tokio::join!(
        monitor.token_with(&expected, &transport, Some("monitor-access-1"), now()),
        monitor.token_with(&expected, &transport, Some("monitor-access-1"), now()),
    );
    assert_eq!(a.unwrap().0, "access-2");
    assert_eq!(b.unwrap().0, "access-2");
    assert_eq!(transport.calls().len(), 1);
}

#[tokio::test]
async fn still_valid_monitor_token_needs_no_refresh() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    let fresh = slot(
        "account:org",
        "monitor-access",
        "monitor-refresh",
        (now() + chrono::Duration::hours(1)).timestamp_millis(),
    );
    monitor.connect(&cli(), &fresh, &[]).await.unwrap();
    let transport = Scripted::new([]);
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now())
            .await
            .unwrap()
            .0,
        "monitor-access"
    );
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn rejects_shared_cli_tokens_and_wrong_account_or_provider() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    assert_eq!(
        monitor.connect(&cli(), &cli(), &[]).await,
        Err("monitor_shared_login".into())
    );
    assert_eq!(
        monitor.connect(&cli(), &login(), &[login()]).await,
        Err("monitor_shared_login".into())
    );
    let mut wrong = login();
    wrong.id = "different:org".into();
    assert_eq!(
        monitor.connect(&cli(), &wrong, &[]).await,
        Err("monitor_wrong_account".into())
    );
    wrong.id = cli().id;
    wrong.provider = Provider::Codex;
    assert_eq!(
        monitor.connect(&cli(), &wrong, &[]).await,
        Err("monitor_wrong_account".into())
    );
    assert!(!monitor.connected(&cli()));
}

#[tokio::test]
async fn revoked_refresh_requires_reconnect_even_after_restart() {
    for failure in [Failure::Revoked] {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        monitor.connect(&cli(), &login(), &[]).await.unwrap();
        let transport = Scripted::new([Err(failure)]);
        assert_eq!(
            monitor.token_with(&cli(), &transport, None, now()).await,
            Err("monitor_reconnect".into())
        );
        let restarted = Monitor::new(dir.0.clone());
        assert_eq!(
            restarted
                .token_with(&cli(), &transport, None, now() + chrono::Duration::days(1))
                .await,
            Err("monitor_reconnect".into())
        );
        assert_eq!(transport.calls().len(), 1);
    }
}

#[tokio::test]
async fn rate_limit_and_network_failure_back_off_then_recover() {
    for (failure, delay, code) in [
        (Failure::RateLimited(900), 900, "rate_limited"),
        (Failure::Transient, 300, "monitor_network"),
    ] {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        monitor.connect(&cli(), &login(), &[]).await.unwrap();
        let transport = Scripted::new([Err(failure), reply("access-2", None)]);
        assert_eq!(
            monitor.token_with(&cli(), &transport, None, now()).await,
            Err(code.into())
        );
        assert_eq!(
            monitor
                .token_with(
                    &cli(),
                    &transport,
                    None,
                    now() + chrono::Duration::seconds(delay - 1)
                )
                .await,
            Err(code.into())
        );
        assert_eq!(transport.calls().len(), 1);
        assert_eq!(
            monitor
                .token_with(
                    &cli(),
                    &transport,
                    None,
                    now() + chrono::Duration::seconds(delay)
                )
                .await
                .unwrap()
                .0,
            "access-2"
        );
        assert_eq!(transport.calls().len(), 2);
    }
}

#[tokio::test]
async fn failed_disk_write_keeps_rotated_refresh_token_and_never_replays_old_one() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    let path = monitor.path(&cli());
    // Force the atomic rename to fail even when tests run as an administrator.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let transport = Scripted::new([
        reply("access-2", Some("refresh-2")),
        reply("access-3", Some("refresh-3")),
    ]);
    assert_eq!(
        monitor.token_with(&cli(), &transport, None, now()).await,
        Err("monitor_storage".into())
    );
    assert_eq!(
        monitor.token_with(&cli(), &transport, None, now()).await,
        Err("monitor_storage".into())
    );
    assert_eq!(transport.calls(), ["monitor-refresh-1"]);
    std::fs::remove_dir(&path).unwrap();
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now())
            .await
            .unwrap()
            .0,
        "access-2"
    );
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now() + chrono::Duration::hours(2))
            .await
            .unwrap()
            .0,
        "access-3"
    );
    assert_eq!(transport.calls(), ["monitor-refresh-1", "refresh-2"]);
}

#[tokio::test]
async fn rejection_of_renewed_token_is_terminal_but_old_poll_cannot_invalidate_new_login() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    monitor.reject(&cli(), "monitor-access-1").await.unwrap();
    let restarted = Monitor::new(dir.0.clone());
    let transport = Scripted::new([reply("access-2", None)]);
    assert_eq!(
        restarted.token_with(&cli(), &transport, None, now()).await,
        Err("monitor_reconnect".into())
    );
    let fresh = slot("account:org", "new-login", "new-login-refresh", 0);
    restarted.connect(&cli(), &fresh, &[]).await.unwrap();
    restarted.reject(&cli(), "monitor-access-1").await.unwrap();
    assert_eq!(
        restarted
            .token_with(&cli(), &transport, None, now())
            .await
            .unwrap()
            .0,
        "access-2"
    );
    assert_eq!(transport.calls(), ["new-login-refresh"]);
}

#[tokio::test]
async fn codex_renews_from_jwt_expiry_and_preserves_account_routing() {
    let dir = TempDir::new();
    let mut expected = cli();
    expected.provider = Provider::Codex;
    expected.data = json!({"auth":{"tokens":{"access_token":"cli", "refresh_token":"cli-refresh", "account_id":"workspace"}}});
    let mut fresh = expected.clone();
    let jwt = format!(
        "header.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1}"#)
    );
    fresh.data["auth"]["tokens"]["access_token"] = json!(jwt);
    fresh.data["auth"]["tokens"]["refresh_token"] = json!("codex-monitor-refresh");
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&expected, &fresh, &[]).await.unwrap();
    let transport = Scripted::new([reply("codex-access", None)]);
    assert_eq!(
        monitor
            .token_with(&expected, &transport, None, now())
            .await
            .unwrap(),
        ("codex-access".into(), Some("workspace".into()))
    );
    assert_eq!(transport.calls(), ["codex-monitor-refresh"]);
}

#[tokio::test]
async fn mismatched_or_corrupt_persisted_session_is_never_sent() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    let mut wrong = cli();
    wrong.id = "other:org".into();
    std::fs::copy(monitor.path(&cli()), monitor.path(&wrong)).unwrap();
    let transport = Scripted::new([]);
    assert_eq!(
        monitor.token_with(&wrong, &transport, None, now()).await,
        Err("monitor_wrong_account".into())
    );
    std::fs::write(monitor.path(&wrong), b"corrupt").unwrap();
    assert_eq!(
        monitor.token_with(&wrong, &transport, None, now()).await,
        Err("monitor_storage".into())
    );
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn removal_deletes_persisted_session_and_in_memory_refresh_token() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    monitor.remove(&cli()).await.unwrap();
    assert!(!monitor.connected(&cli()));
    assert_eq!(
        monitor
            .token_with(&cli(), &Scripted::new([]), None, now())
            .await,
        Err("monitor_setup".into())
    );
}

#[test]
fn connect_remove_and_disconnect_share_a_reservation_until_mutations_finish() {
    let operations = Operations::default();
    let connect = operations.begin(Provider::Claude, "one").unwrap();
    assert!(operations.begin(Provider::Claude, "one").is_err());
    let other = operations.begin(Provider::Claude, "two").unwrap();
    drop(connect);
    let remove = operations.begin(Provider::Claude, "one").unwrap();
    assert!(operations.begin(Provider::Claude, "one").is_err());
    drop(remove);
    drop(other);
    assert!(operations.begin(Provider::Claude, "one").is_ok());
}

#[tokio::test]
async fn reconnect_saves_new_login_before_revoking_old_chain_and_never_revokes_after_failed_save() {
    struct Inspect {
        path: PathBuf,
        revoked: SyncMutex<Vec<String>>,
    }
    impl Revoke for Inspect {
        async fn revoke(&self, old: &Session) -> bool {
            let saved: Session =
                serde_json::from_slice(&private_store::load_secret(&self.path).unwrap()).unwrap();
            assert_eq!(saved.refresh_token, "new-monitor-refresh");
            self.revoked.lock().unwrap().push(old.refresh_token.clone());
            true
        }
    }
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    let inspector = Inspect {
        path: monitor.path(&cli()),
        revoked: SyncMutex::new(vec![]),
    };
    let fresh = slot(
        "account:org",
        "new-monitor-access",
        "new-monitor-refresh",
        0,
    );
    std::fs::remove_file(&inspector.path).unwrap();
    std::fs::create_dir(&inspector.path).unwrap();
    assert_eq!(
        monitor.connect_with(&cli(), &fresh, &[], &inspector).await,
        Err("monitor_storage".into())
    );
    assert!(inspector.revoked.lock().unwrap().is_empty());
    std::fs::remove_dir(&inspector.path).unwrap();
    monitor
        .connect_with(&cli(), &fresh, &[], &inspector)
        .await
        .unwrap();
    assert_eq!(*inspector.revoked.lock().unwrap(), ["monitor-refresh-1"]);
}

#[tokio::test]
async fn one_uncertain_replay_is_allowed_after_backoff_and_budget_survives_restart() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    let transport = Scripted::new([Err(Failure::Ambiguous), Err(Failure::Ambiguous)]);
    assert_eq!(
        monitor.token_with(&cli(), &transport, None, now()).await,
        Err("monitor_network".into())
    );
    drop(monitor);
    let monitor = Monitor::new(dir.0.clone());
    assert_eq!(
        monitor
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::seconds(299)
            )
            .await,
        Err("monitor_network".into())
    );
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(
        monitor
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::minutes(5)
            )
            .await,
        Err("monitor_uncertain".into())
    );
    drop(monitor);
    let monitor = Monitor::new(dir.0.clone());
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now() + chrono::Duration::days(1))
            .await,
        Err("monitor_uncertain".into())
    );
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn scopes_and_refresh_lifetime_are_persisted_without_inventing_a_lifetime() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    let mut fresh = login();
    fresh.data["credentials"]["claudeAiOauth"]["scopes"] =
        json!(["user:profile", "user:inference"]);
    monitor.connect(&cli(), &fresh, &[]).await.unwrap();
    let mut result = reply("access-2", Some("refresh-2")).unwrap();
    result.scope = Some("user:profile user:inference".into());
    result.refresh_token_expires_in = Some(7200);
    monitor
        .token_with(&cli(), &Scripted::new([Ok(result)]), None, now())
        .await
        .unwrap();
    let saved: Session =
        serde_json::from_slice(&private_store::load_secret(&monitor.path(&cli())).unwrap())
            .unwrap();
    assert_eq!(saved.scopes, ["user:profile", "user:inference"]);
    assert_eq!(
        saved.refresh_expires_at,
        Some(now() + chrono::Duration::hours(2))
    );
    assert_eq!(
        Monitor::new(dir.0.clone())
            .token_with(
                &cli(),
                &Scripted::new([reply("access-3", Some("refresh-3"))]),
                None,
                now() + chrono::Duration::hours(2)
            )
            .await,
        Ok(("access-3".into(), None))
    );
    let restarted = Monitor::new(dir.0.clone());
    let saved: Session =
        serde_json::from_slice(&private_store::load_secret(&restarted.path(&cli())).unwrap())
            .unwrap();
    assert_eq!(saved.refresh_expires_at, None);
}

// These tests exercise the production reqwest encoder, response parser and state
// machine over a real socket. Only synthetic tokens are sent, to localhost.
#[tokio::test]
async fn imported_cli_expiry_hint_requests_one_refresh_instead_of_forcing_reconnect() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    let mut fresh = login();
    fresh.data["credentials"]["claudeAiOauth"]["expiresAt"] =
        json!((now() + chrono::Duration::days(40)).timestamp_millis());
    fresh.data["credentials"]["claudeAiOauth"]["refreshTokenExpiresAt"] =
        json!((now() + chrono::Duration::days(30)).timestamp_millis());
    monitor.connect(&cli(), &fresh, &[]).await.unwrap();
    let transport = Scripted::new([reply("access-after-deadline", Some("new-chain"))]);
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now() + chrono::Duration::days(31))
            .await
            .unwrap()
            .0,
        "access-after-deadline"
    );
    assert_eq!(transport.calls(), ["monitor-refresh-1"]);
    let saved: Session =
        serde_json::from_slice(&private_store::load_secret(&monitor.path(&cli())).unwrap())
            .unwrap();
    assert!(!saved.reconnect);
    assert_eq!(saved.refresh_expires_at, None);
}

fn hinted_login() -> Slot {
    let mut fresh = login();
    fresh.data["credentials"]["claudeAiOauth"]["expiresAt"] =
        json!((now() + chrono::Duration::days(9)).timestamp_millis());
    fresh.data["credentials"]["claudeAiOauth"]["refreshTokenExpiresAt"] =
        json!((now() - chrono::Duration::days(1)).timestamp_millis());
    fresh
}

#[tokio::test]
async fn hint_failures_keep_valid_access_and_backoff_but_never_return_rejected_or_expired_access() {
    for (failure, delay, code) in [
        (Failure::Transient, 300, "monitor_network"),
        (Failure::RateLimited(900), 900, "rate_limited"),
        (Failure::Ambiguous, 300, "monitor_network"),
    ] {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        monitor.connect(&cli(), &hinted_login(), &[]).await.unwrap();
        let transport = Scripted::new([Err(failure), reply("renewed", Some("rotated"))]);
        assert_eq!(
            monitor
                .token_with(&cli(), &transport, None, now())
                .await
                .unwrap()
                .0,
            "monitor-access-1"
        );
        let backoff = now() + chrono::Duration::seconds(delay - 1);
        assert_eq!(
            monitor
                .token_with(&cli(), &transport, None, backoff)
                .await
                .unwrap()
                .0,
            "monitor-access-1"
        );
        assert_eq!(
            monitor
                .token_with(&cli(), &transport, Some("monitor-access-1"), backoff)
                .await,
            Err(code.into())
        );
        assert_eq!(
            monitor.token_with(&cli(), &transport, None, backoff).await,
            Err(code.into())
        );
        // The access-token grace does not disable renewal backoff or rate limits.
        assert_eq!(transport.calls().len(), 1);
        assert_eq!(
            monitor
                .token_with(
                    &cli(),
                    &transport,
                    None,
                    now() + chrono::Duration::seconds(delay)
                )
                .await
                .unwrap()
                .0,
            "renewed"
        );
        assert_eq!(transport.calls().len(), 2);
    }
}

#[tokio::test]
async fn usage_rejection_survives_restart_during_hint_backoff_and_clears_after_rotation() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &hinted_login(), &[]).await.unwrap();
    let transport = Scripted::new([Err(Failure::Ambiguous), reply("renewed", Some("rotated"))]);
    assert!(monitor
        .token_with(&cli(), &transport, None, now())
        .await
        .is_ok());
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, Some("monitor-access-1"), now())
            .await,
        Err("monitor_network".into())
    );
    drop(monitor);
    let restarted = Monitor::new(dir.0.clone());
    assert_eq!(
        restarted
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::seconds(299)
            )
            .await,
        Err("monitor_network".into())
    );
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(
        restarted
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::minutes(5)
            )
            .await
            .unwrap()
            .0,
        "renewed"
    );
    assert_eq!(
        restarted
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::minutes(6)
            )
            .await
            .unwrap()
            .0,
        "renewed"
    );
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn hint_ambiguity_keeps_access_across_restart_without_expanding_replay_budget() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &hinted_login(), &[]).await.unwrap();
    let transport = Scripted::new([Err(Failure::Ambiguous), Err(Failure::Ambiguous)]);
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now())
            .await
            .unwrap()
            .0,
        "monitor-access-1"
    );
    drop(monitor);
    let monitor = Monitor::new(dir.0.clone());
    assert_eq!(
        monitor
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::seconds(299)
            )
            .await
            .unwrap()
            .0,
        "monitor-access-1"
    );
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(
        monitor
            .token_with(
                &cli(),
                &transport,
                None,
                now() + chrono::Duration::minutes(5)
            )
            .await
            .unwrap()
            .0,
        "monitor-access-1"
    );
    drop(monitor);
    let monitor = Monitor::new(dir.0.clone());
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now() + chrono::Duration::days(1))
            .await
            .unwrap()
            .0,
        "monitor-access-1"
    );
    assert_eq!(
        monitor
            .token_with(
                &cli(),
                &transport,
                Some("monitor-access-1"),
                now() + chrono::Duration::days(1)
            )
            .await,
        Err("monitor_uncertain".into())
    );
    assert_eq!(
        monitor
            .token_with(&cli(), &transport, None, now() + chrono::Duration::days(9))
            .await,
        Err("monitor_uncertain".into())
    );
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn hint_never_grants_access_after_rejection_or_an_accepted_unreadable_rotation() {
    for failure in [Failure::Revoked, Failure::RotatedUnreadable] {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        monitor.connect(&cli(), &hinted_login(), &[]).await.unwrap();
        let transport = Scripted::new([Err(failure)]);
        assert_eq!(
            monitor.token_with(&cli(), &transport, None, now()).await,
            Err("monitor_reconnect".into())
        );
        assert_eq!(
            Monitor::new(dir.0.clone())
                .token_with(&cli(), &transport, None, now() + chrono::Duration::days(1))
                .await,
            Err("monitor_reconnect".into())
        );
        assert_eq!(transport.calls().len(), 1);
    }
}

#[derive(Default)]
struct RecordedRevocations(SyncMutex<Vec<String>>);
impl Revoke for RecordedRevocations {
    async fn revoke(&self, session: &Session) -> bool {
        self.0.lock().unwrap().push(session.refresh_token.clone());
        true
    }
}

#[tokio::test]
async fn rejected_handoff_is_revoked_only_when_no_cli_or_monitor_owns_its_token() {
    for mode in 0..6 {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        let mut wrong = login();
        wrong.id = "different:org".into();
        let mut known = vec![];
        if mode == 1 {
            known.push(wrong.clone());
        }
        if mode == 2 {
            wrong.data = cli().data;
        }
        if mode == 3 {
            monitor.connect(&cli(), &login(), &[]).await.unwrap();
            // Force the ownership check to load the stored monitor from disk.
            monitor.sessions.lock().await.clear();
        }
        if mode == 4 {
            monitor.connect(&cli(), &login(), &[]).await.unwrap();
            private_store::save_secret(&monitor.path(&cli()), b"corrupt").unwrap();
        }
        if mode == 5 {
            wrong.id = cli().id;
            wrong.data = cli().data;
        }
        let revoked = RecordedRevocations::default();
        let result = monitor
            .connect_or_discard_with(&cli(), &wrong, &known, &revoked)
            .await;
        assert_eq!(
            result,
            Err(if mode == 5 {
                "monitor_shared_login"
            } else {
                "monitor_wrong_account"
            }
            .into())
        );
        let calls = revoked.0.lock().unwrap();
        if mode == 0 {
            assert_eq!(*calls, ["monitor-refresh-1"]);
        } else {
            assert!(calls.is_empty(), "mode {mode}");
        }
    }
}

#[cfg(windows)]
#[tokio::test]
async fn failed_replacement_revokes_only_new_handoff_and_preserves_old_monitor() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    let path = monitor.path(&cli());
    // Permit reading the protected old session but deny replacement/deletion.
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let fresh = slot("account:org", "new-access", "new-refresh", 0);
    let revoked = RecordedRevocations::default();
    assert_eq!(
        monitor
            .connect_or_discard_with(&cli(), &fresh, &[], &revoked)
            .await,
        Err("monitor_storage".into())
    );
    assert_eq!(*revoked.0.lock().unwrap(), ["new-refresh"]);
    let saved: Session =
        serde_json::from_slice(&private_store::load_secret(&path).unwrap()).unwrap();
    assert_eq!(saved.refresh_token, "monitor-refresh-1");
    assert_eq!(
        monitor.sessions.lock().await[&key(&cli())]
            .session
            .refresh_token,
        "monitor-refresh-1"
    );
    drop(locked);
    monitor
        .connect_or_discard_with(&cli(), &fresh, &[], &revoked)
        .await
        .unwrap();
    assert_eq!(
        *revoked.0.lock().unwrap(),
        ["new-refresh", "monitor-refresh-1"]
    );
}

#[tokio::test]
async fn cleanup_protects_vault_and_persisted_monitor_tokens_and_fails_closed_on_corruption() {
    let dir = TempDir::new();
    let monitor = Monitor::new(dir.0.clone());
    monitor.connect(&cli(), &login(), &[]).await.unwrap();
    drop(monitor);
    let monitor = Monitor::new(dir.0.clone());
    let protected = monitor.protected_tokens(&[cli()]).await.unwrap();
    assert!(protected.contains(&(Provider::Claude, "cli-refresh".into())));
    assert!(protected.contains(&(Provider::Claude, "monitor-refresh-1".into())));
    private_store::save_secret(&monitor.path(&cli()), b"corrupt-session").unwrap();
    assert!(monitor.protected_tokens(&[cli()]).await.is_none());
}

mod http {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    struct Response {
        status: u16,
        body: &'static str,
        delay: Duration,
        body_delay: Duration,
        header: &'static str,
    }
    fn response(status: u16, body: &'static str) -> Response {
        Response {
            status,
            body,
            delay: Duration::ZERO,
            body_delay: Duration::ZERO,
            header: "",
        }
    }
    struct Server {
        url: String,
        requests: std::sync::mpsc::Receiver<String>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(responses: Vec<Response>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/token", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let (send, requests) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                for reply in responses {
                    let start = std::time::Instant::now();
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    && start.elapsed() < Duration::from_secs(3) =>
                            {
                                std::thread::sleep(Duration::from_millis(2))
                            }
                            _ => return,
                        }
                    };
                    // Winsock accepts inherit the listener's nonblocking mode.
                    // Read the entire request before responding on every platform.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = vec![];
                    loop {
                        let mut buf = [0; 4096];
                        let Ok(n) = stream.read(&mut buf) else { return };
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&buf[..n]);
                        if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                            let len = headers
                                .lines()
                                .find_map(|l| {
                                    l.strip_prefix("content-length: ")
                                        .and_then(|s| s.parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if request.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let _ = send.send(String::from_utf8(request).unwrap());
                    std::thread::sleep(reply.delay);
                    let data = format!("HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n", reply.status, reply.body.len(), reply.header);
                    let _ = stream.write_all(data.as_bytes());
                    std::thread::sleep(reply.body_delay);
                    let _ = stream.write_all(reply.body.as_bytes());
                }
            });
            Self {
                url,
                requests,
                thread: Some(thread),
            }
        }
        fn request(&self) -> String {
            self.requests.recv_timeout(Duration::from_secs(3)).unwrap()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }
    struct Local<'a>(&'a reqwest::Client, &'a str);
    impl Refresh for Local<'_> {
        async fn refresh(&self, session: &Session) -> Result<Reply, Failure> {
            Api(self.0, None).refresh_at(session, self.1, "test-client").await
        }
    }
    impl Revoke for Local<'_> {
        async fn revoke(&self, session: &Session) -> bool {
            Api(self.0, None).revoke_at(session, self.1, "test-client").await
        }
    }
    fn client(timeout_ms: u64) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .unwrap()
    }
    fn session(provider: Provider) -> Session {
        Session {
            version: 1,
            provider,
            id: "test".into(),
            access_token: "test-access".into(),
            refresh_token: "test-refresh".into(),
            account_id: None,
            expires_at: None,
            scopes: vec!["user:profile".into(), "user:inference".into()],
            refresh_expires_at: None,
            uncertain_attempts: 0,
            uncertain_retry_at: None,
            reconnect: false,
            access_rejected: false,
        }
    }
    #[tokio::test]
    async fn http_diagnostics_keep_status_and_timeout_but_never_requests_or_bodies() {
        let dir = TempDir::new();
        let recorder = crate::diagnostics::Recorder::new(dir.0.join("journal")); recorder.start();
        let mut slow = response(200, "PRIVATE-provider-body"); slow.delay = Duration::from_millis(60);
        let server = Server::new(vec![response(503, "PRIVATE-provider-body"), response(200, "PRIVATE-revoke-body"), slow]);
        let http = client(20);
        let api = Api(&http, Some(&recorder));
        let mut s = session(Provider::Claude); s.id = "PRIVATE-account-id".into(); s.refresh_token = "PRIVATE-refresh-token".into();
        assert!(matches!(api.refresh_at(&s, &server.url, "PRIVATE-client-id").await, Err(Failure::Transient)));
        assert!(api.revoke_at(&s, &server.url, "PRIVATE-client-id").await);
        assert!(matches!(api.refresh_at(&s, &server.url, "PRIVATE-client-id").await, Err(Failure::Ambiguous)));
        let report = String::from_utf8(recorder.report(&crate::model::Snapshot::default(), &crate::settings::Settings::default()).unwrap()).unwrap();
        assert!(!report.contains("PRIVATE")); assert!(!report.contains(&server.url));
        let data: Value = serde_json::from_str(&report).unwrap();
        let events: Vec<_> = data["events"].as_array().unwrap().iter().filter(|r| r["event"]["type"] == "http").collect();
        assert_eq!(events.len(), 3); assert_eq!(events[0]["event"]["status"], 503);
        assert_eq!(events[1]["event"]["status"], 200); assert_eq!(events[2]["event"]["network"], "timeout");
    }
    #[tokio::test]
    async fn real_hint_failures_allow_usage_without_repeating_refresh_during_backoff() {
        for (status, delay, retry_after) in [
            (503, 0, ""),
            (429, 0, "Retry-After: 900\r\n"),
            (200, 150, ""),
        ] {
            let mut failed = response(status, "{}");
            failed.delay = Duration::from_millis(delay);
            failed.header = retry_after;
            let server = Server::new(vec![failed]);
            let dir = TempDir::new();
            let monitor = Monitor::new(dir.0.clone());
            monitor.connect(&cli(), &hinted_login(), &[]).await.unwrap();
            let client = client(if delay == 0 { 1000 } else { 40 });
            let local = Local(&client, &server.url);
            assert_eq!(
                monitor
                    .token_with(&cli(), &local, None, now())
                    .await
                    .unwrap()
                    .0,
                "monitor-access-1"
            );
            assert_eq!(
                monitor
                    .token_with(&cli(), &local, None, now() + chrono::Duration::seconds(299))
                    .await
                    .unwrap()
                    .0,
                "monitor-access-1"
            );
            assert!(server.request().contains("monitor-refresh-1"));
            assert!(server.requests.try_recv().is_err());
            let code = if status == 429 {
                "rate_limited"
            } else {
                "monitor_network"
            };
            assert_eq!(
                monitor
                    .token_with(
                        &cli(),
                        &local,
                        Some("monitor-access-1"),
                        now() + chrono::Duration::seconds(299)
                    )
                    .await,
                Err(code.into())
            );
        }
    }
    #[tokio::test]
    async fn refresh_requests_use_json_and_claude_sends_saved_scopes() {
        for provider in [Provider::Claude, Provider::Codex] {
            let server = Server::new(vec![response(
                200,
                r#"{"access_token":"renewed","refresh_token":"rotated","expires_in":3600}"#,
            )]);
            let reply = Local(&client(1000), &server.url)
                .refresh(&session(provider))
                .await
                .unwrap();
            assert_eq!(reply.access_token, "renewed");
            let request = server.request();
            assert!(request
                .to_lowercase()
                .contains("content-type: application/json"));
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body["grant_type"], "refresh_token");
            assert_eq!(body["client_id"], "test-client");
            assert_eq!(body["refresh_token"], "test-refresh");
            if provider == Provider::Claude {
                assert_eq!(body["scope"], "user:profile user:inference");
            } else {
                assert!(body.get("scope").is_none());
            }
        }
    }
    #[tokio::test]
    async fn real_http_error_shapes_reject_dead_tokens_and_keep_other_failures_transient() {
        for (status, body, expected) in [
            (400, r#"{"error":"invalid_grant"}"#, Failure::Revoked),
            (
                400,
                r#"{"error":{"type":"invalid_grant"}}"#,
                Failure::Revoked,
            ),
            (
                400,
                r#"{"error":{"code":"refresh_token_reused"}}"#,
                Failure::Revoked,
            ),
            (400, r#"{"code":"refresh_token_expired"}"#, Failure::Revoked),
            (
                403,
                r#"{"error":{"code":"REFRESH_TOKEN_INVALIDATED"}}"#,
                Failure::Revoked,
            ),
            (401, "not-json", Failure::Revoked),
            (400, r#"{"error":"invalid_client"}"#, Failure::Transient),
            (400, "not-json", Failure::Transient),
            (503, r#"{"error":"unavailable"}"#, Failure::Transient),
            (200, "not-json", Failure::RotatedUnreadable),
        ] {
            let server = Server::new(vec![response(status, body)]);
            assert_eq!(
                Local(&client(1000), &server.url)
                    .refresh(&session(Provider::Claude))
                    .await
                    .err(),
                Some(expected),
                "{status} {body}"
            );
        }
        let mut limited = response(429, "{}");
        limited.header = "Retry-After: 900\r\n";
        let server = Server::new(vec![limited]);
        assert_eq!(
            Local(&client(1000), &server.url)
                .refresh(&session(Provider::Codex))
                .await
                .err(),
            Some(Failure::RateLimited(900))
        );
    }
    #[tokio::test]
    async fn accepted_but_unreadable_refresh_never_replays_even_after_restart() {
        for (body, delay) in [
            ("not-json", 0),
            (r#"{"refresh_token":"new","expires_in":"invalid-type"}"#, 0),
            (r#"{"refresh_token":"lost"}"#, 150),
        ] {
            let mut accepted = response(200, body);
            accepted.body_delay = Duration::from_millis(delay);
            let server = Server::new(vec![accepted]);
            let dir = TempDir::new();
            let monitor = Monitor::new(dir.0.clone());
            monitor.connect(&cli(), &login(), &[]).await.unwrap();
            assert_eq!(
                monitor
                    .token_with(
                        &cli(),
                        &Local(&client(if delay == 0 { 1000 } else { 40 }), &server.url),
                        None,
                        now()
                    )
                    .await,
                Err("monitor_reconnect".into())
            );
            assert!(server.request().contains("monitor-refresh-1"));
            drop(monitor);
            let replay = Scripted::new([]);
            assert_eq!(
                Monitor::new(dir.0.clone())
                    .token_with(&cli(), &replay, None, now() + chrono::Duration::days(1))
                    .await,
                Err("monitor_reconnect".into())
            );
            assert!(replay.calls().is_empty());
        }
    }
    #[tokio::test]
    async fn real_timeout_recovers_once_without_marking_account_revoked() {
        let mut late = response(200, r#"{"access_token":"lost-response"}"#);
        late.delay = Duration::from_millis(150);
        let server = Server::new(vec![
            late,
            response(
                200,
                r#"{"access_token":"recovered","refresh_token":"rotated","expires_in":3600}"#,
            ),
        ]);
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        monitor.connect(&cli(), &login(), &[]).await.unwrap();
        assert_eq!(
            monitor
                .token_with(&cli(), &Local(&client(40), &server.url), None, now())
                .await,
            Err("monitor_network".into())
        );
        drop(monitor);
        let monitor = Monitor::new(dir.0.clone());
        let recovered = monitor
            .token_with(
                &cli(),
                &Local(&client(1000), &server.url),
                None,
                now() + chrono::Duration::minutes(5),
            )
            .await
            .unwrap();
        assert_eq!(recovered.0, "recovered");
        for _ in 0..2 {
            assert!(server.request().contains("monitor-refresh-1"));
        }
        let saved: Session =
            serde_json::from_slice(&private_store::load_secret(&monitor.path(&cli())).unwrap())
                .unwrap();
        assert!(!saved.reconnect);
        assert_eq!(saved.uncertain_attempts, 0);
        assert_eq!(saved.refresh_token, "rotated");
    }
    #[tokio::test]
    async fn explicit_reuse_rejection_after_timeout_requires_reconnect_without_more_replays() {
        let mut late = response(200, "{}");
        late.delay = Duration::from_millis(150);
        let server = Server::new(vec![
            late,
            response(400, r#"{"error":{"code":"refresh_token_reused"}}"#),
        ]);
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        monitor.connect(&cli(), &login(), &[]).await.unwrap();
        assert_eq!(
            monitor
                .token_with(&cli(), &Local(&client(40), &server.url), None, now())
                .await,
            Err("monitor_network".into())
        );
        assert_eq!(
            monitor
                .token_with(
                    &cli(),
                    &Local(&client(1000), &server.url),
                    None,
                    now() + chrono::Duration::minutes(5)
                )
                .await,
            Err("monitor_reconnect".into())
        );
        assert_eq!(
            Monitor::new(dir.0.clone())
                .token_with(
                    &cli(),
                    &Scripted::new([]),
                    None,
                    now() + chrono::Duration::days(1)
                )
                .await,
            Err("monitor_reconnect".into())
        );
    }
    #[tokio::test]
    async fn disconnect_revokes_only_monitoring_chain_and_deletes_locally_when_offline() {
        for status in [200, 503] {
            let server = Server::new(vec![response(status, "{}")]);
            let dir = TempDir::new();
            let monitor = Monitor::new(dir.0.clone());
            monitor.connect(&cli(), &login(), &[]).await.unwrap();
            let revoked = monitor
                .disconnect_with(&cli(), &Local(&client(1000), &server.url))
                .await
                .unwrap();
            assert_eq!(revoked, status == 200);
            let request = server.request();
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(
                body,
                json!({"token":"monitor-refresh-1", "token_type_hint":"refresh_token", "client_id":"test-client"})
            );
            assert!(!monitor.connected(&cli()));
            assert_eq!(
                monitor
                    .token_with(&cli(), &Scripted::new([]), None, now())
                    .await,
                Err("monitor_setup".into())
            );
        }
    }
}
