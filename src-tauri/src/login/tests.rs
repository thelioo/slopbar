use super::*;
use crate::test_support::TempDir;

#[test]
fn authentication_overrides_are_removed_case_insensitively() {
    for name in [
        "CLAUDE_CODE_OAUTH_CLIENT_ID",
        "CLAUDE_CODE_CUSTOM_OAUTH_URL",
        "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
        "CLAUDE_CODE_OAUTH_REFRESH_TOKEN_SCOPES",
        "CLAUDE_SECURESTORAGE_CONFIG_DIR",
        "CLAUDE_CODE_FORCE_WINDOWS_CREDMAN",
        "ANTHROPIC_API_KEY",
        "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
        "OPENAI_API_KEY",
        "codex_app_server_login_client_id",
        "CLAUDECODE",
        "USE_STAGING_OAUTH",
    ] {
        assert!(blocked_environment(name), "{name}");
    }
    assert!(!blocked_environment("PATH"));
    assert!(!blocked_environment("HTTPS_PROXY"));
}

#[cfg(unix)]
#[test]
fn shell_profile_overrides_do_not_reach_isolated_cli() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    // A quoted directory name exercises the actual WSL script quoting.
    let config = dir.0.join("sign-in's config");
    std::fs::create_dir(&config).unwrap();
    let fake = dir.0.join("claude");
    std::fs::write(&fake, "#!/bin/sh\npwd\nenv\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let script = format!("export CLAUDE_CODE_OAUTH_CLIENT_ID=wrong CLAUDE_SECURESTORAGE_CONFIG_DIR=/shared CLAUDE_CODE_FORCE_WINDOWS_CREDMAN=1 ANTHROPIC_API_KEY=secret OPENAI_API_KEY=secret CODEX_HOME=/shared; {}",
        shell_script(Provider::Claude, config.to_str().unwrap(), "claude", false));
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &script])
        .env("PATH", format!("{}:/usr/bin:/bin", dir.0.display()))
        .output()
        .unwrap();
    assert!(output.status.success());
    let env = String::from_utf8(output.stdout).unwrap();
    assert_eq!(env.lines().next().unwrap(), config.to_str().unwrap());
    assert!(env.contains(&format!("CLAUDE_CONFIG_DIR={}\n", config.display())));
    assert!(env.contains(&format!(
        "CLAUDE_SECURESTORAGE_CONFIG_DIR={}\n",
        config.display()
    )));
    for name in [
        "CLAUDE_CODE_OAUTH_CLIENT_ID=",
        "CLAUDE_CODE_FORCE_WINDOWS_CREDMAN=",
        "ANTHROPIC_API_KEY=",
        "OPENAI_API_KEY=",
        "CODEX_HOME=",
    ] {
        assert!(!env.contains(name), "{name}");
    }
    assert_eq!(
        std::fs::metadata(config).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn cleanup_eligibility_is_strict_and_failure_pause_preserves_exit_status() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1800);
    assert!(abandoned(".slopbar-login-123-1000-1", now));
    assert!(!abandoned(".slopbar-login-123-1700000-1", now));
    assert!(!abandoned(".slopbar-login-not-ours", now));
    assert!(!abandoned(".slopbar-login-123-1000-1-extra", now));
    assert!(!abandoned(
        ".slopbar-login-123-9999999999999999999999-1",
        now
    ));
    let dir = TempDir::new();
    prepare(&dir.0, Provider::Claude, "claude auth login").unwrap();
    let script = std::fs::read_to_string(dir.0.join("login.cmd")).unwrap();
    assert!(script.contains("call claude auth login"));
    assert!(script.contains("set \"slopbar_login_status=%errorlevel%\"\r\nif not"));
    assert!(script.ends_with("exit /b %slopbar_login_status%\r\n"));
}

#[test]
fn startup_cleanup_keeps_fresh_unmarked_and_linked_directories() {
    let dir = TempDir::new();
    let fresh = dir.0.join(format!(
        ".slopbar-login-1-{}-1",
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::create_dir(&fresh).unwrap();
    prepare(&fresh, Provider::Claude, "claude auth login").unwrap();
    let unknown = dir.0.join(".slopbar-login-1-1000-2");
    std::fs::create_dir(&unknown).unwrap();
    let old = dir.0.join(".slopbar-login-1-1000-4");
    std::fs::create_dir(&old).unwrap();
    prepare(&old, Provider::Codex, "codex login").unwrap();
    let legacy = dir.0.join(".slopbar-login-1-1000-5");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(
        legacy.join(".credentials.json"),
        br#"{"claudeAiOauth":{"accessToken":"synthetic"}}"#,
    )
    .unwrap();
    let source = Source {
        label: "test".into(),
        home: dir.0.clone(),
        native_home: dir.0.to_string_lossy().into_owned(),
        distro: None,
        claude_dir: None,
        codex_dir: None,
    };
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&fresh, dir.0.join(".slopbar-login-1-1000-3")).unwrap();
    }
    let mut logouts = vec![];
    cleanup_with(
        &[source],
        Some(&ProtectedTokens::new()),
        |provider, _, path, _| logouts.push((provider, path.to_owned())),
    );
    assert_eq!(logouts, [(Provider::Codex, old.clone())]);
    assert!(!old.exists());
    assert!(!legacy.exists());
    assert!(fresh.exists());
    assert!(unknown.exists());
    #[cfg(unix)]
    assert!(dir.0.join(".slopbar-login-1-1000-3").is_symlink());
}

fn source(home: &Path) -> Source {
    Source {
        label: "test".into(),
        home: home.to_owned(),
        native_home: home.to_string_lossy().into_owned(),
        distro: None,
        claude_dir: None,
        codex_dir: None,
    }
}

#[test]
fn exported_credentials_are_never_logged_out_even_if_folder_deletion_failed() {
    for provider in Provider::ALL {
        let home = TempDir::new();
        let dir = home.0.join(".slopbar-login-1-1000-1");
        std::fs::create_dir(&dir).unwrap();
        prepare(&dir, provider, action(provider, false)).unwrap();
        let (file, data) = match provider {
            Provider::Claude => (
                ".credentials.json",
                br#"{"claudeAiOauth":{"refreshToken":"exported"}}"#.as_slice(),
            ),
            Provider::Codex => (
                "auth.json",
                br#"{"tokens":{"refresh_token":"exported"}}"#.as_slice(),
            ),
        };
        std::fs::write(dir.join(file), data).unwrap();
        // Windows cannot delete an exclusively opened file, matching antivirus or
        // indexer contention. Linux recreates the leftover after handoff instead.
        #[cfg(windows)]
        let locked = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(dir.join(file))
                .unwrap()
        };
        handoff(&dir, provider).unwrap();
        #[cfg(windows)]
        {
            assert!(dir.join(file).exists());
            drop(locked);
        }
        #[cfg(not(windows))]
        std::fs::write(dir.join(file), data).unwrap();
        let owner: Owner =
            serde_json::from_slice(&std::fs::read(dir.join("slopbar-owner.json")).unwrap())
                .unwrap();
        assert!(owner.imported);
        let mut logouts = 0;
        cleanup_with(
            &[source(&home.0)],
            Some(&ProtectedTokens::new()),
            |_, _, _, _| logouts += 1,
        );
        assert_eq!(logouts, 0);
        assert!(!dir.exists());
    }
}

#[test]
fn cleanup_does_not_revoke_known_tokens_legacy_exports_or_unreadable_stores() {
    for mode in 0..4 {
        let home = TempDir::new();
        let dir = home.0.join(".slopbar-login-1-1000-1");
        std::fs::create_dir(&dir).unwrap();
        prepare(&dir, Provider::Claude, action(Provider::Claude, false)).unwrap();
        std::fs::write(
            dir.join(".credentials.json"),
            br#"{"claudeAiOauth":{"refreshToken":"known"}}"#,
        )
        .unwrap();
        let mut protected = ProtectedTokens::new();
        if mode == 0 {
            protected.insert((Provider::Claude, "known".into()));
        }
        if mode == 1 {
            std::fs::write(dir.join("slopbar-owner.json"), br#""claude""#).unwrap();
        }
        if mode == 2 {
            std::fs::write(dir.join(".credentials.json"), b"unreadable").unwrap();
        }
        let mut logouts = 0;
        cleanup_with(
            &[source(&home.0)],
            (mode != 3).then_some(&protected),
            |_, _, _, _| logouts += 1,
        );
        assert_eq!(logouts, 0, "mode {mode}");
        assert!(!dir.exists());
    }
}

#[test]
fn failed_handoff_keeps_credentials_private_and_native_login_uses_isolated_cwd() {
    let dir = TempDir::new();
    let marker = dir.0.join("slopbar-owner.json");
    std::fs::create_dir(&marker).unwrap();
    std::fs::write(dir.0.join(".credentials.json"), b"not-exported").unwrap();
    assert!(handoff(&dir.0, Provider::Claude).is_err());
    assert_eq!(
        std::fs::read(dir.0.join(".credentials.json")).unwrap(),
        b"not-exported"
    );
    let command = command(
        Provider::Claude,
        &source(&dir.0),
        &dir.0,
        dir.0.to_str().unwrap(),
        action(Provider::Claude, false),
        true,
    );
    assert_eq!(command.get_current_dir(), Some(dir.0.as_path()));
    assert_eq!(
        action(Provider::Claude, false),
        "claude --setting-sources user auth login"
    );
    assert_eq!(
        action(Provider::Claude, true),
        "claude --setting-sources user auth logout"
    );
}

#[cfg(windows)]
#[test]
fn closing_windows_job_terminates_the_login_process() {
    use std::os::windows::process::CommandExt;
    let mut child = Command::new("cmd.exe")
        .args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"])
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let tree = ProcessTree::attach(&child).unwrap();
    assert!(child.try_wait().unwrap().is_none());
    drop(tree);
    let start = std::time::Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            // Kill-on-close can report exit code zero. The process was still
            // running before close, and must terminate long before its 30s wait.
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            panic!("job did not terminate login process")
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
#[test]
fn job_fixture_process() {
    let Ok(role) = std::env::var("SLOPBAR_JOB_TEST_ROLE") else {
        return;
    };
    let file = std::env::var_os("SLOPBAR_JOB_TEST_PID").unwrap();
    if role == "parent" {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "login::tests::job_fixture_process",
                "--nocapture",
            ])
            .env("SLOPBAR_JOB_TEST_ROLE", "leaf")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(file, child.id().to_string()).unwrap();
        let _ = child.wait();
    } else {
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[cfg(windows)]
#[test]
fn native_login_script_handles_profile_path_shell_metacharacters() {
    let home = TempDir::new();
    let dir = home.0.join("profile & ^ %SLOPBAR_PATH_TEST%");
    std::fs::create_dir(&dir).unwrap();
    prepare(&dir, Provider::Codex, "echo login-ok>result.txt").unwrap();
    let mut cmd = command(
        Provider::Codex,
        &source(&home.0),
        &dir,
        dir.to_str().unwrap(),
        "unused",
        true,
    );
    cmd.env("SLOPBAR_PATH_TEST", "wrong-path")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let (mut child, _tree) = ProcessTree::spawn(&mut cmd, false).unwrap();
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "login script did not exit"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("result.txt"))
            .unwrap()
            .trim(),
        "login-ok"
    );
}

#[cfg(windows)]
#[test]
fn suspended_login_is_assigned_before_running_and_job_close_kills_descendants() {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    };
    let dir = TempDir::new();
    let file = dir.0.join("leaf-pid");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "login::tests::job_fixture_process",
            "--nocapture",
        ])
        .env("SLOPBAR_JOB_TEST_ROLE", "parent")
        .env("SLOPBAR_JOB_TEST_PID", &file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !file.exists(),
        "suspended launcher ran before job assignment"
    );
    let tree = ProcessTree::attach(&child).unwrap();
    resume(&child).unwrap();
    let start = std::time::Instant::now();
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Ok(pid) = text.parse::<u32>() {
                break pid;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "descendant did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    // Keep a handle so process-ID reuse cannot make the termination check pass.
    let descendant = unsafe { OpenProcess(0x0010_0000, 0, pid) }; // SYNCHRONIZE
    assert!(!descendant.is_null());
    assert_eq!(unsafe { WaitForSingleObject(descendant, 0) }, WAIT_TIMEOUT);
    drop(tree);
    let result = unsafe { WaitForSingleObject(descendant, 3000) };
    unsafe {
        CloseHandle(descendant);
    }
    assert_eq!(result, WAIT_OBJECT_0, "descendant escaped the job");
    child.wait().unwrap();

    // Exercise the production suspended-spawn wrapper as well.
    let (mut child, tree) = ProcessTree::spawn(
        Command::new("cmd.exe").args(["/d", "/c", "exit /b 0"]),
        false,
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    drop(tree);
}
