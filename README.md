<div align="center">

<img src="design/icon.png" width="112" alt="SlopBar icon" />

# SlopBar

Your Claude Code and Codex plan limits, right in the Windows taskbar, next to the clock.

<!-- TODO: screenshot of the taskbar widget and its flyout -->

</div>

## Features

- **No sign-in**: reads the credentials Claude Code and Codex already saved.
- **Windows + WSL**: finds accounts in your Windows profile and in every WSL distro.
- **Lives in your taskbar**: a small widget next to the clock shows each provider's usage as a ring around its logo; hover for the exact number.
- **One click for the details**: click the widget for a flyout with every account, its session and weekly limits, reset times, limit resets and credits. Right-click it to refresh, open Settings or quit.
- **Multiple accounts**: add as many Claude and Codex accounts as you like and switch with one click, or let it switch for you when one hits its limit.
- **Feels native**: follows the taskbar's light or dark theme, and steps aside while a fullscreen app is running.
- **Speaks your language**: follows the system language (10 languages, English fallback).

> [!NOTE]
> SlopBar was formerly called Usage Bar. Existing Usage Bar installs update into SlopBar automatically.

## Multiple accounts

Add accounts in **Settings → Accounts**: SlopBar opens the CLI's own sign-in (`claude auth login` / `codex login`) in an isolated folder, so your current login is untouched. Switching rewrites the CLI's login files in place; the official CLI keeps doing all the work. Switch from the flyout with **Use**, or turn on **Switch automatically** to move to the account with the most headroom when the active one reaches your threshold.

> [!NOTE]
> A running Codex session keeps its account until you restart it.

## Build

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
> SlopBar never refreshes tokens, so it can't log you out of your CLI. If a token expires, open `claude` or `codex` once.

## Releases

Every push to `main` builds on GitHub Actions and publishes a release (`v0.2.<run>`) with the installer and the standalone `.exe`. Installed apps update themselves silently in the background.

## License

[GPL-3.0](LICENSE). Use it, change it, share it; derivative works must stay open source under the same license.
