<div align="center">

<img src="design/icon.png" width="112" alt="SlopBar icon" />

# SlopBar

Your Claude Code and Codex plan limits, right in the Windows taskbar, next to the clock.

<!-- TODO: screenshot of the taskbar widget and its flyout -->

</div>

## Features

- **Account discovery**: reads the credentials Claude Code and Codex already saved.
- **Independent monitoring**: connect each account once to keep checking its usage while it is inactive, without opening a conversation or switching the CLI.
- **Windows + WSL**: finds accounts in your Windows profile and in every WSL distro.
- **Lives in your taskbar**: a small widget next to the clock shows each provider's usage as a ring around its logo; hover for the exact number.
- **One click for the details**: click the widget for a flyout with every account, its session and weekly limits, reset times, limit resets and credits. Right-click it to refresh, open Settings or quit.
- **Multiple accounts**: add as many Claude and Codex accounts as you like and switch with one click, or let it switch for you when one hits its limit.
- **Usage by project and model**: Settings > Usage shows each account's tokens by repository and model over the last 7 or 28 days, its busiest hours of the week, and each provider's model mix, read from Claude Code's and the Codex CLI's own files on this PC (token counts, models, times and folders only, never what was written). Claude use is credited to the account SlopBar saw signed in at the time. Export the weekly history, this breakdown and the subscription report as CSV files.
- **Feels native**: follows the taskbar's light or dark theme, and steps aside while a fullscreen app is running.
- **Speaks your language**: follows the system language (10 languages, English fallback).

> [!NOTE]
> SlopBar was formerly called Usage Bar. Existing Usage Bar installs update into SlopBar automatically.

## Multiple accounts

Add accounts in **Settings → Accounts**: SlopBar opens the CLI's own sign-in (`claude auth login` / `codex login`) in an isolated folder, so your current login is untouched. Switching rewrites the CLI's login files in place; the official CLI keeps doing all the work. Switch from the flyout with **Use**, or turn on **Switch automatically** to move to the account with the most headroom when the active one reaches your threshold.

> [!NOTE]
> A running Codex session keeps its account until you restart it.

### A new computer

Turn on **Share account list with my other computers** (Settings → Data → Other computers) on each computer, with the same roster passphrase on each. Every SlopBar then adds its accounts' provider, id, email and plan (never sign-ins) to the sharing folder or rclone remote, encrypted. A computer that reads them shows "N accounts from your other machines are waiting to be signed in" in the flyout and Health, and **Sign in all accounts** (Settings → Accounts) queues every one, one email at a time.

### Monitoring accounts you aren't using

While an account is the one signed in to the CLI, SlopBar reads its usage with the CLI's own access token. That stops when you switch away: the CLI's refresh token rotates every time it is used, so SlopBar can't keep renewing a copy of it in the background without signing the CLI out. To keep reading an account you aren't using, SlopBar needs its own sign-in for it, a separate session it alone renews.

SlopBar starts that sign-in for you. Once an account without monitoring has been the one in use for 3 minutes, it opens the sign-in and shows a notification, **Finish connecting monitoring for** that account; your browser is usually already signed in to it, so one approval is enough. It runs one sign-in at a time, waits while a fullscreen app is in front, never retries a cancelled or failed sign-in for that account within 24 hours, and leaves alone an account whose monitoring you disconnected. Turn it off with **Settings → Data → Connect monitoring automatically**.

You can also choose **Connect monitoring** next to each account in Settings or the flyout. Complete the official CLI's sign-in for that same account and organization. This creates a separate monitoring session in a temporary configuration folder; it is never imported into the switching vault or written to the active CLI's files. The temporary folder is removed when sign-in completes, fails, or times out.

SlopBar renews this separate session automatically when its access token is about to expire, and polls the provider's usage endpoint. No prompt or model request is sent. Existing accounts need this one-time sign-in because their copied CLI refresh tokens cannot safely be shared with an independent background refresher.

**Disconnect monitoring** works even for the active account. It removes the local monitoring credentials and attempts to revoke that separate session at the provider. If server revocation cannot be confirmed, the UI asks you to revoke it in your provider's account settings. Reconnecting saves the new login before attempting to revoke the previous monitoring session.

If a completed monitoring login is rejected for the wrong account or cannot be saved, SlopBar attempts to revoke that uninstalled login. It first protects every known CLI/monitor token; shared tokens or unreadable ownership stores disable this cleanup. Failed cleanup can leave a provider session to revoke manually in account settings.

Last successful readings survive app restarts. If the provider cannot be reached, the UI shows **Last confirmed … ago** and retains the observed values. A passed reset time does not imply a measured 0%. Automatic switching considers only fresh, successfully measured accounts. A revoked session, an unusable refresh response, or a lost credential file may still require **Reconnect monitoring**; recent account use is never the recovery instruction.

A refresh timeout before success headers does not immediately disconnect an account. SlopBar waits five minutes and allows one retry of its own monitoring token. Both the backoff and retry limit survive restarts. An explicit token rejection requires reconnecting; after a second uncertain response, automatic replay stops and the UI offers reconnecting. A successful HTTP response with an unreadable body requires reconnecting immediately, because replay could reuse an already consumed token. The providers can reject a retry if the timed-out request already rotated its token.

If the active account's monitor fails, SlopBar can still read fresh usage with the CLI's current access token. It never refreshes that CLI token, and keeps the monitoring error and Reconnect control visible alongside the fresh reading. This fallback is restricted to active accounts and respects usage rate limits. A saved refresh-token expiry is a renewal hint: the server decides whether the session is still valid, so a CLI-synthesized 30-day deadline does not itself disconnect monitoring.

When only that hint triggers renewal, a temporary failure, rate limit, or uncertain send does not suppress a still-valid monitoring access token. Usage polling continues with that token while refresh backoff and the replay budget remain enforced. Expired or usage-rejected access tokens are never returned this way; explicit refresh rejection and unreadable successful rotations still require reconnecting.

### Why the original version lost inactive accounts

At upstream commit `60a7c8e0835b4cd320dba3e0b1352d7a8161c9e1`, `providers::fetch_one` stopped before requesting usage when a copied Claude access token had expired. SlopBar deliberately never renewed tokens, and `accounts::capture` only obtained replacements when an official CLI had already refreshed its login. Inactive accounts therefore lost live telemetry. The UI translated an inactive expired token into “Not used recently,” hiding the authentication cause. The last-success cache lived only in memory, and its reset prediction could display 0% without a provider response.

Refreshing those existing copied refresh tokens would race the CLI's token rotation. This branch instead owns a separate refresh-token chain per monitored account, serializes refreshes, saves rotated tokens atomically, backs off after failures, and uses Windows DPAPI for monitoring credentials. The account-switching vault is encrypted with DPAPI too: plaintext slots from earlier releases are encrypted on the first start, and a slot is never deleted before its encrypted copy reads back correctly; see [SECURITY.md](SECURITY.md).

## Diagnostics for real-machine testing

This build records a local rolling diagnostic history automatically. After testing, use **Settings → General → Diagnostics → Export diagnostics**. It creates a `SlopBar-Diagnostics-*.json` report and opens its folder on Windows. Upload that JSON report for analysis; do not upload the account vault, monitoring files, or the encrypted journal.

The report includes anonymous account numbers stable across restarts, usage percentages and last-confirmed times, monitoring/CLI/fallback usage outcomes, refresh HTTP statuses and timeout categories, renewal reasons, backoff/replay decisions, isolated login stages, revocation outcomes, poll durations/gaps, frontend error categories, and the executable's embedded build metadata. It excludes tokens, emails, aliases, account/organization IDs, home paths, distro names, request URLs/headers/bodies, provider response bodies, raw exception messages, and stacks. Collection is local; there is no automatic upload.

The journal retains at most **7 days or 10,000 events**, whichever limit is reached first. Unchanged token, usage and account observations are suppressed, with hourly heartbeat readings; state changes and unusual polling gaps are kept. Stable 11-account tests, including repeated monitoring errors with a working active-CLI fallback, cover a full week even at one-minute polling. Each usage origin has an independent state/heartbeat, so the monitor and fallback do not overwrite each other’s observations. Frequent real changes or repeated launches can still reach the event cap sooner; reports show the actual history start, discarded events and suppressed observations. Its account-number mapping is retained locally for continuity and never exported. Windows protects the journal/mapping with current-user DPAPI; exports are readable JSON containing usage/timing data. A background writer batches routine changes at most once every five minutes when needed. Authentication/lifecycle transitions request an immediate background write; no diagnostic serialization, DPAPI or disk I/O runs on the authentication caller. Export and orderly shutdown wait for queued writes. Killing the app can lose queued transitions and up to five minutes of routine changes, but the next startup records the missing clean shutdown. Storage failures are best effort and do not change token handling. A full crash dump and provider-side/CLI internal logs are not collected.

For automated build verification, `slopbar.exe --build-info-to <path>` writes the executable's own commit, tree, dirty-checkout flag, and Tauri/updater configuration without launching the GUI or reading credentials. CI packaging checks this output against the checkout before producing the ZIP.

### Agent access (MCP)

`slopbar-mcp.exe`, built next to `slopbar.exe`, is a stdio [MCP](https://modelcontextprotocol.io) server that gives coding agents what the Settings window offers, without driving the UI. Register it with your agent, for example:

```
claude mcp add --scope user slopbar -- "%LOCALAPPDATA%\SlopBar\slopbar-mcp.exe"
```

or in Codex's `~/.codex/config.toml`:

```toml
[mcp_servers.slopbar]
command = 'C:\Users\<you>\AppData\Local\SlopBar\slopbar-mcp.exe'
```

| Tool | Does |
| --- | --- |
| `get_usage`, `refresh_usage` | Every account's usage windows, resets, monitoring state and errors (as last refreshed, or polled now). |
| `get_settings`, `update_settings` | Read settings, or change them with a JSON merge patch. Only values the Settings window offers are accepted; unknown fields are refused. |
| `switch_account`, `remove_account` | Use or remove a saved account, as in Settings. |
| `add_account`, `connect_monitoring`, `disconnect_monitoring` | Start the CLI's own sign-in for the user to complete, or revoke a monitoring sign-in. |
| `revoke_account_sessions` | Revoke every sign-in SlopBar holds for an account (its monitoring session, at the provider and here), optionally removing the saved CLI login with `remove_slot`. Only reports unless `confirm: true`; the CLI's own sign-in is left to the CLI. |
| `check_for_updates`, `install_update`, `get_app_info`, `open_settings` | Updates (builds with automatic updates only), version and build, and showing the Settings window. |
| `quit_app`, `restart_app` | Quit as the tray's Quit does, or quit and start the installed executable again. |
| `get_usage_breakdown` | Tokens on this PC by account and repository, model or day, with each provider's model mix and an hour-of-week heatmap, and how each account's tokens were attributed. |
| `export_csv` | Write the weekly history, the usage breakdown and the subscription report as CSV files (default `Documents\SlopBar Exports`) and return their paths. |
| `export_diagnostics`, `list_diagnostic_exports` | Write a diagnostics export (a repeat within 30 seconds returns the same file) or list earlier ones; listing works while SlopBar is closed. |
| `export_calendar` | Write the plan-renewal calendar (below) now and return its path. |
| `lease_account`, `release_lease`, `list_leases` | Reserve an account for a job (`purpose`, `minutes`, optionally `expected_percent` or a given `id`): until the lease ends, `recommend_account` lists it as excluded ("leased: …") for everyone else, automatic switching never moves to it and the flyout shows the purpose on its row. A lease switches nothing; it returns the `config_dir` to run the job with. Leases expire (default 2 hours, at most a day) and end when SlopBar quits. |

The server never starts SlopBar, reads credentials or contacts a provider itself: each tool asks the running app, which performs the same action as the matching Settings control. They talk over a named pipe that only the signed-in user and SYSTEM can open and that rejects remote clients. The server checks that the process answering runs as the same Windows user, connects so that process cannot act as the agent's, bounds every reply in size and time, and only passes on an export path inside the export folder. Sign-ins always happen in the CLI's own login, completed by the user. Usage results include account emails, so treat them like the Settings window. `slopbar-mcp.exe` is a separate executable so that app updates, which stop `slopbar.exe`, do not cut agents off mid-session.

### Claude Code status line

`slopbar-mcp.exe --statusline` prints one line about the Claude account in use, for Claude Code's [status line](https://docs.claude.com/en/docs/claude-code/statusline), for example `work · 12% 5h · 42% wk · runs out Fri 3PM`. It asks the running app, reuses an answer for 15 seconds, gives up after 120 ms and prints nothing when SlopBar is not running. To use it, add this to `~/.claude/settings.json` (forward slashes keep the JSON simple):

```json
{
  "statusLine": {
    "type": "command",
    "command": "C:/Users/<you>/AppData/Local/SlopBar/slopbar-mcp.exe --statusline"
  }
}
```

### Exact Claude attribution

Claude Code's transcripts do not name the account they ran on, so the usage breakdown otherwise attributes Claude use to the account SlopBar saw signed in at the time (`inferred_from_active_account`), and use from times SlopBar was not watching stays unattributed. `slopbar-mcp.exe --session-hook` is a [SessionStart hook](https://docs.claude.com/en/docs/claude-code/hooks): when a session starts it tells the running app the session id, working folder and `CLAUDE_CONFIG_DIR` (unset means the default login), and the app records which account that config dir is signed in to. The breakdown then attributes that session's transcript, subagents included, to that account (`exact`) ahead of any inference. It prints nothing, waits at most 1.5 seconds for SlopBar and always exits 0, so a closed SlopBar never holds up Claude Code. Only the session id, folder and account id are kept (in `active-accounts.json`, 90 days); no emails. Add this to `~/.claude/settings.json` yourself (SlopBar never edits it):

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "C:/Users/<you>/AppData/Local/SlopBar/slopbar-mcp.exe --session-hook" }
        ]
      }
    ]
  }
}
```

Merge it into an existing `hooks` object if you have one. `SLOPBAR_HOOK_DEBUG=1` makes the hook say on stderr what happened.

### Smart launcher (`slopbar-run`)

SlopBar keeps a launcher at `%APPDATA%\dev.thelio.slopbar\bin\slopbar-run.cmd` (Settings > Accounts > Projects shows the exact path). Put that folder on your `PATH`, then:

```
slopbar-run [claude|codex] [--provider <claude|codex>] [--lease <purpose>] [--percent <n>] [-- <command> [args...]]
```

- Inside a folder pinned to an account, the command runs on that account's project profile (as before).
- Anywhere else it asks SlopBar for the account `recommend_account` would pick among those nobody else leased (`--percent`: the share of a week the job needs). When that is not the CLI's current sign-in, it leases the account for as long as the command runs, so the account's project profile stays signed in and other agents keep off it, and sets `CLAUDE_CONFIG_DIR` or `CODEX_HOME` for that one command. `--lease <purpose>` names the job and leases the account even when it is the current sign-in. The lease is released when the command exits (and ends by itself if the launcher is killed).
- The provider defaults to the command's (`codex` runs Codex, anything else Claude Code); the command defaults to the provider's CLI. Examples: `slopbar-run claude`, `slopbar-run --lease "nightly refactor" -- claude -p "…"`, `slopbar-run -- codex exec "…"`.
- Without SlopBar running, the command runs on the CLI's current sign-in. `SLOPBAR_RUN_DEBUG=1` says why on stderr.

### Renewal calendar and Monday digest

**Settings → Subscriptions → Renewal calendar** keeps an iCalendar file (`Documents\SlopBar\renewals.ics` unless you pick another path) with an all-day event for each paid plan's next renewal, and a reminder three days ahead for plans that look unused or underused. Subscribe to the file from Outlook (**Add calendar → From file**) or import it into Google Calendar; with **Keep a renewal calendar** on, SlopBar rewrites it after each refresh, and each renewal keeps the same event ID across rewrites, so subscribed calendars update rather than duplicate it. **Export now** writes it once.

The **Monday digest** (Settings → Notifications, on by default) is one notification on Monday mornings with what your paid plans cost per month, what you could save, the plans renewing that week, accounts that ran out in the past week and how many plans your usage needs. Clicking it opens Settings → Subscriptions.

To install a local release build without quitting SlopBar by hand, run `scripts/install-local-build.ps1` after `cargo build --locked --release --features tauri/custom-protocol --manifest-path src-tauri/Cargo.toml`. It checks the build matches the clean checkout, moves the running executables into a rollback folder under `%LOCALAPPDATA%\SlopBar-Rollback`, copies the new ones in and restarts SlopBar through `restart_app`.

## Build

On Windows, with Rust and the usual Tauri prerequisites installed:

```powershell
cargo test --locked --no-default-features --lib --manifest-path src-tauri/Cargo.toml
cargo build --locked --release --features tauri/custom-protocol --manifest-path src-tauri/Cargo.toml
```

The standalone executable is `src-tauri/target/release/slopbar.exe`. Exit any running SlopBar before starting it; the single-instance guard otherwise hands control to the already running version. Existing settings and accounts are read from the same data directory.

The `tauri/custom-protocol` feature selects Tauri's release configuration with embedded assets. CI checks and test builds use this feature too; a plain Cargo release build without it uses Tauri's development configuration.

Local builds disable upstream updates at compile time, including if an existing settings file enables automatic updates. This prevents an unpatched upstream release from replacing this fix. Release maintainers can explicitly enable the `upstream-updates` Cargo feature after merging the fix. The Settings page shows when updates are unavailable.

An optional frontend smoke check is available as `node scripts/test-telemetry-ui.cjs` with Playwright and Chromium installed. It uses synthetic accounts and mocked Tauri commands to check monitoring controls, stale usage, error feedback, and the minimum supported window width. Set `CHROMIUM_PATH` to use an existing Chromium binary. These checks do not authenticate with a provider or exercise a real Windows credential store; the Rust tests run DPAPI checks when executed on Windows.

The Rust suite also runs the real HTTP code against a local server with synthetic tokens: JSON request encoding, saved Claude scopes, nested OAuth errors, rate limits, delayed responses, and revocation. It covers failed credential-file deletion after handoff, non-revocation of imported sessions, and successful-response bodies that must never be replayed. Windows tests check suspended process startup and descendant termination. `Telemetry tests` runs core tests on Linux and Windows, then checks the official updater feature and builds an unsigned native Windows desktop executable. Its test-build artifact includes the exact source and SHA256 manifest; it does not publish a release or use signing secrets. Fork PR workflows can require maintainer approval before they execute.

Cross-compiled for Windows from WSL/Linux:

```bash
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
sudo apt install -y nsis lld llvm clang
pnpm install
pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle
```

The app is at `src-tauri/target/x86_64-pc-windows-msvc/release/slopbar.exe`.

> [!NOTE]
> Monitoring only renews its own separately connected sessions. Imported CLI sessions are never refreshed by SlopBar.

## Releases

The upstream release workflow publishes from `main` (`v0.2.<run>`), runs the telemetry tests on Windows, and enables signed upstream updates for those releases. Local builds use manual updates.

## License

[GPL-3.0](LICENSE). Use it, change it, share it; derivative works must stay open source under the same license.
