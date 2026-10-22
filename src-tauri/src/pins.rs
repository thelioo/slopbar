//! An account per project: a folder pinned to one account of a provider, whose CLI then runs with
//! that account's project profile as its config dir (see "Project profiles" in `accounts`).
//!
//! SlopBar never edits the user's repositories. It keeps a launcher in the app data folder,
//! `slopbar-run <claude|codex> [-- command…]`, that finds the pin for the current folder and sets
//! CLAUDE_CONFIG_DIR or CODEX_HOME for that one command; and it shows a PowerShell line and an
//! `.envrc` line per pin for people who prefer setting the variable themselves.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::accounts::{self, ProfileState, Provider};
use crate::settings::ProjectPin;

pub const MAX_PINS: usize = 100;

/// The environment variable that points each CLI at a config dir.
pub fn env_var(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "CLAUDE_CONFIG_DIR",
        Provider::Codex => "CODEX_HOME",
    }
}

/// A folder as pins store it: trimmed, absolute, existing, without a trailing separator.
pub fn normalize_folder(folder: &str) -> Result<String, String> {
    let trimmed = folder.trim();
    let refuse = || format!("folder must be an existing absolute folder (got {folder:?})");
    if trimmed.is_empty() || trimmed.len() > 1024 || trimmed.chars().any(char::is_control) {
        return Err(refuse());
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() || !path.is_dir() {
        return Err(refuse());
    }
    // Keep a drive root ("C:\") whole; strip the separator from anything longer.
    // One spelling per folder: Windows paths with backslashes only.
    let spelled = if cfg!(windows) { trimmed.replace('/', "\\") } else { trimmed.to_string() };
    let stripped = spelled.trim_end_matches(['\\', '/']);
    Ok(if stripped.ends_with(':') || stripped.is_empty() { spelled.clone() } else { stripped.to_string() })
}

fn same_folder(a: &str, b: &str) -> bool {
    if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// `pins` with `provider`'s pin on `folder` set to account `id`, or removed when `id` is None.
/// `known` are the accounts SlopBar has saved; only those can be pinned.
pub fn set(pins: &[ProjectPin], folder: &str, provider: Provider, id: Option<&str>, known: &[(Provider, String)]) -> Result<Vec<ProjectPin>, String> {
    let mut next: Vec<ProjectPin> = pins.to_vec();
    match id {
        None => {
            let before = next.len();
            let folder = folder.trim().trim_end_matches(['\\', '/']);
            next.retain(|p| !(p.provider == provider && same_folder(p.folder.trim_end_matches(['\\', '/']), folder)));
            if next.len() == before {
                return Err(format!("no {} pin on {folder:?}", provider.name()));
            }
        }
        Some(id) => {
            if !known.iter().any(|(p, k)| *p == provider && k == id) {
                return Err(format!("no {} account {id:?}; ids come from get_usage", provider.name()));
            }
            let folder = normalize_folder(folder)?;
            let pin = ProjectPin { folder: folder.clone(), provider, id: id.to_string() };
            match next.iter().position(|p| p.provider == provider && same_folder(&p.folder, &folder)) {
                Some(i) => next[i] = pin,
                None if next.len() >= MAX_PINS => return Err(format!("at most {MAX_PINS} pins")),
                None => next.push(pin),
            }
        }
    }
    next.sort_by(|a, b| (a.folder.to_lowercase(), a.provider.key()).cmp(&(b.folder.to_lowercase(), b.provider.key())));
    Ok(next)
}

/// Every pinned account once, for `accounts::sync_profiles`.
pub fn wanted(pins: &[ProjectPin]) -> Vec<(Provider, String)> {
    let mut out: Vec<(Provider, String)> = Vec::new();
    for p in pins {
        if !out.iter().any(|(q, id)| *q == p.provider && *id == p.id) {
            out.push((p.provider, p.id.clone()));
        }
    }
    out
}

/// Where the launcher lives.
pub fn bin_dir() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("bin"))
}

pub fn launcher_path() -> Option<PathBuf> {
    bin_dir().map(|d| d.join(if cfg!(windows) { "slopbar-run.cmd" } else { "slopbar-run.ps1" }))
}

fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A Windows path as WSL sees it ("C:\Users" is "/mnt/c/Users"), for `.envrc` lines.
pub fn wsl_path(path: &str) -> String {
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            format!("/mnt/{}{}", drive.to_ascii_lowercase(), chars.as_str().replace('\\', "/"))
        }
        _ => path.replace('\\', "/"),
    }
}

/// The PowerShell launcher: picks the pin with the longest folder containing the current one.
/// A profile without a login means the account is the CLI's default login right now (see
/// `accounts::sync_profiles`), so the command runs on that, which is the same account.
pub fn launcher_script(entries: &[(ProjectPin, PathBuf)]) -> String {
    let table = entries.iter().map(|(pin, dir)| format!(
        "  [pscustomobject]@{{ Folder = {}; Provider = {}; Dir = {} }}",
        ps_quote(&pin.folder), ps_quote(pin.provider.key()), ps_quote(&dir.to_string_lossy()),
    )).collect::<Vec<_>>().join("\n");
    format!(r#"# Generated by SlopBar whenever project pins change; edits are overwritten.
# Runs a CLI with the account pinned to the current folder (Settings > Accounts > Projects).
# Usage: slopbar-run <claude|codex> [-- <command> [args...]]   The command defaults to the CLI.
$pins = @(
{table}
)
if ($args.Count -lt 1 -or @('claude', 'codex') -notcontains $args[0]) {{
  [Console]::Error.WriteLine('usage: slopbar-run <claude|codex> [-- <command> [args...]]')
  exit 2
}}
$provider = [string]$args[0]
$rest = @($args | Select-Object -Skip 1)
if ($rest.Count -gt 0 -and $rest[0] -eq '--') {{ $rest = @($rest | Select-Object -Skip 1) }}
if ($rest.Count -eq 0) {{ $rest = @($provider) }}
$here = (Get-Location).ProviderPath
$best = $null
$bestLength = -1
foreach ($pin in $pins) {{
  if ($pin.Provider -ne $provider) {{ continue }}
  $folder = $pin.Folder.Replace('/', '\').TrimEnd('\')
  $inside = $here -eq $folder -or $here.StartsWith($folder + '\', [StringComparison]::OrdinalIgnoreCase)
  if ($inside -and $folder.Length -gt $bestLength) {{ $best = $pin; $bestLength = $folder.Length }}
}}
if ($null -ne $best) {{
  $var = if ($provider -eq 'claude') {{ 'CLAUDE_CONFIG_DIR' }} else {{ 'CODEX_HOME' }}
  $file = if ($provider -eq 'claude') {{ '.credentials.json' }} else {{ 'auth.json' }}
  if (Test-Path -LiteralPath (Join-Path $best.Dir $file)) {{
    Set-Item -Path "Env:$var" -Value $best.Dir
  }} else {{
    [Console]::Error.WriteLine("slopbar-run: this folder's account is the $provider CLI's current sign-in, so it runs on that.")
  }}
}}
$command = $rest[0]
$commandArgs = @($rest | Select-Object -Skip 1)
& $command @commandArgs
exit $LASTEXITCODE
"#)
}

const LAUNCHER_CMD: &str = "@echo off\r\nrem Generated by SlopBar: see slopbar-run.ps1 next to this file.\r\npowershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"%~dp0slopbar-run.ps1\" %*\r\nexit /b %ERRORLEVEL%\r\n";

/// Writes the launcher for `pins` into `dir` (through temp files, so a running launcher never
/// reads half a script).
pub fn write_launcher_in(dir: &Path, root: &Path, pins: &[ProjectPin]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let entries: Vec<_> = pins.iter().map(|p| (p.clone(), root.join(accounts::profile_name(p.provider, &p.id)))).collect();
    for (name, text) in [("slopbar-run.ps1", launcher_script(&entries)), ("slopbar-run.cmd", LAUNCHER_CMD.to_string())] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            continue;
        }
        let tmp = path.with_extension("slopbar-tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(())
}

pub fn write_launcher(pins: &[ProjectPin]) -> std::io::Result<()> {
    match (bin_dir(), accounts::profiles_root()) {
        (Some(dir), Some(root)) => write_launcher_in(&dir, &root, pins),
        _ => Err(std::io::Error::other("no config dir")),
    }
}

/// One pin as Settings and agents see it.
#[derive(Debug, Clone, Serialize)]
pub struct PinView {
    pub folder: String,
    pub provider: Provider,
    pub id: String,
    /// The account's alias or email, when SlopBar knows the account.
    pub account: Option<String>,
    /// The project profile (the config dir the CLI uses here).
    pub profile: String,
    /// ready, at_default (the account is the CLI's current sign-in, so the folder uses that) or
    /// no_login (SlopBar has no saved login for the account).
    pub state: ProfileState,
    /// The variable to set, with lines that set it for PowerShell and for direnv in WSL.
    pub env_var: &'static str,
    pub powershell: String,
    pub envrc: String,
    /// Runs the CLI through the launcher from inside the folder.
    pub launch: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub pins: Vec<PinView>,
    /// The launcher; `slopbar-run <claude|codex> [-- command…]` from inside a pinned folder.
    pub launcher: Option<String>,
    pub how: &'static str,
}

pub const HOW: &str = "Run `slopbar-run claude` (or codex, optionally `-- <command> args`) from inside a pinned folder: it sets the config dir for that command only. A pinned account is signed in at one place at a time: while it is the CLI's current sign-in its profile is empty and the folder runs on that sign-in. The PowerShell and .envrc lines point at the profile permanently and only work while the account is not the CLI's current sign-in.";

/// What Settings and agents show: each pin with its state and the lines that use it.
pub fn listing(
    pins: &[ProjectPin],
    states: &std::collections::HashMap<(Provider, String), ProfileState>,
    label: impl Fn(Provider, &str) -> Option<String>,
    root: &Path,
    launcher: Option<&Path>,
) -> Listing {
    let launcher_text = launcher.map(|p| p.to_string_lossy().into_owned());
    Listing {
        pins: pins.iter().map(|p| {
            let profile = root.join(accounts::profile_name(p.provider, &p.id)).to_string_lossy().into_owned();
            let var = env_var(p.provider);
            PinView {
                folder: p.folder.clone(),
                provider: p.provider,
                id: p.id.clone(),
                account: label(p.provider, &p.id),
                state: states.get(&(p.provider, p.id.clone())).copied().unwrap_or(ProfileState::NoLogin),
                env_var: var,
                powershell: format!("$env:{var} = {}", ps_quote(&profile)),
                envrc: format!("export {var}={}", sh_quote(&wsl_path(&profile))),
                launch: match &launcher_text {
                    Some(l) => format!("& {} {}", ps_quote(l), p.provider.key()),
                    None => String::new(),
                },
                profile,
            }
        }).collect(),
        launcher: launcher_text,
        how: HOW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn known() -> Vec<(Provider, String)> {
        vec![(Provider::Claude, "user-1:org-1".into()), (Provider::Claude, "user-3:org-1".into()), (Provider::Codex, "user-2:acct-2".into())]
    }

    #[test]
    fn pins_one_account_per_provider_per_folder() {
        let repo = TempDir::new();
        let folder = repo.0.to_string_lossy().into_owned();
        let pins = set(&[], &format!("  {folder}\\ "), Provider::Claude, Some("user-1:org-1"), &known()).unwrap();
        assert_eq!(pins, [ProjectPin { folder: folder.clone(), provider: Provider::Claude, id: "user-1:org-1".into() }], "trimmed, no trailing separator");
        let pins = set(&pins, &folder, Provider::Codex, Some("user-2:acct-2"), &known()).unwrap();
        assert_eq!(pins.len(), 2, "each provider has its own pin");
        let pins = set(&pins, &folder.to_uppercase(), Provider::Claude, Some("user-3:org-1"), &known()).unwrap();
        assert_eq!(pins.len(), 2, "the same folder is re-pinned, not pinned twice");
        assert_eq!(pins.iter().find(|p| p.provider == Provider::Claude).unwrap().id, "user-3:org-1");
        let pins = set(&pins, &folder, Provider::Claude, None, &known()).unwrap();
        assert_eq!(pins.iter().map(|p| p.provider).collect::<Vec<_>>(), [Provider::Codex]);
        assert!(set(&pins, &folder, Provider::Claude, None, &known()).is_err(), "nothing to remove");
        assert_eq!(wanted(&[pins[0].clone(), ProjectPin { folder: "D:\\other".into(), ..pins[0].clone() }]), [(Provider::Codex, "user-2:acct-2".to_string())]);
    }

    #[test]
    fn refuses_unknown_accounts_and_folders_that_are_not_there() {
        let repo = TempDir::new();
        let folder = repo.0.to_string_lossy().into_owned();
        assert!(set(&[], &folder, Provider::Claude, Some("user-2:acct-2"), &known()).is_err(), "a Codex id is not a Claude account");
        assert!(set(&[], &folder, Provider::Claude, Some("nobody"), &known()).is_err());
        for bad in ["relative\\repo", "", "C:\\definitely\\not\\a\\slopbar\\repo", "C:\\repo\nrm"] {
            assert!(set(&[], bad, Provider::Claude, Some("user-1:org-1"), &known()).is_err(), "{bad:?}");
        }
        let full: Vec<ProjectPin> = (0..MAX_PINS).map(|i| ProjectPin { folder: format!("{folder}\\{i}"), provider: Provider::Claude, id: "user-1:org-1".into() }).collect();
        assert!(set(&full, &folder, Provider::Claude, Some("user-1:org-1"), &known()).is_err());
    }

    #[test]
    fn the_launcher_names_every_pin_and_quotes_paths() {
        let pin = ProjectPin { folder: "C:\\code\\o'brien".into(), provider: Provider::Claude, id: "user-1:org-1".into() };
        let script = launcher_script(&[(pin, PathBuf::from("C:\\Users\\test\\AppData\\Roaming\\dev.thelio.slopbar\\profiles\\claude-1"))]);
        assert!(script.contains("Folder = 'C:\\code\\o''brien'; Provider = 'claude'; Dir = 'C:\\Users\\test\\AppData\\Roaming\\dev.thelio.slopbar\\profiles\\claude-1'"), "{script}");
        assert!(script.contains("CLAUDE_CONFIG_DIR") && script.contains("CODEX_HOME") && script.contains(".credentials.json") && script.contains("auth.json"));
        assert!(launcher_script(&[]).contains("$pins = @(\n\n)"), "no pins is an empty table");

        let (bin, root) = (TempDir::new(), TempDir::new());
        let pins = [ProjectPin { folder: "C:\\code\\a".into(), provider: Provider::Codex, id: "user-2:acct-2".into() }];
        write_launcher_in(&bin.0, &root.0, &pins).unwrap();
        let written = std::fs::read_to_string(bin.0.join("slopbar-run.ps1")).unwrap();
        assert!(written.contains(&*root.0.join(accounts::profile_name(Provider::Codex, "user-2:acct-2")).to_string_lossy()));
        assert!(std::fs::read_to_string(bin.0.join("slopbar-run.cmd")).unwrap().contains("slopbar-run.ps1"));
    }

    /// Runs the generated launcher for real: the profile is set inside the pinned folder only,
    /// and not while the account is the CLI's own sign-in (its profile has no login).
    #[cfg(windows)]
    #[test]
    fn the_launcher_sets_the_profile_only_inside_pinned_folders() {
        let out = TempDir::new();
        let repo = out.0.join("repo");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        let pin = ProjectPin { folder: normalize_folder(&repo.to_string_lossy().replace('\\', "/")).unwrap(), provider: Provider::Claude, id: "user-1:org-1".into() };
        assert!(!pin.folder.contains('/'), "one spelling: {}", pin.folder);
        let profiles = out.0.join("profiles");
        let profile = profiles.join(accounts::profile_name(Provider::Claude, "user-1:org-1"));
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join(".credentials.json"), "{}").unwrap();
        write_launcher_in(&out.0.join("bin"), &profiles, &[pin]).unwrap();
        let run = |cwd: &Path, provider: &str| {
            let output = std::process::Command::new("cmd.exe")
                .arg("/c").arg(out.0.join("bin").join("slopbar-run.cmd")).args([provider, "--", "cmd", "/c", "set", "CLAUDE_CONFIG_DIR"])
                .current_dir(cwd).env_remove("CLAUDE_CONFIG_DIR").output().unwrap();
            (String::from_utf8_lossy(&output.stdout).trim().to_string(), output.status.code())
        };
        let expected = format!("CLAUDE_CONFIG_DIR={}", profile.to_string_lossy());
        assert_eq!(run(&repo.join("sub"), "claude").0, expected, "inside a pinned folder");
        assert_eq!(run(&repo, "claude").0, expected);
        assert_eq!(run(&out.0, "claude"), (String::new(), Some(1)), "outside: untouched, and the command's exit code comes back");
        assert_eq!(run(&repo, "codex").0, "", "a pin is per provider");
        std::fs::remove_file(profile.join(".credentials.json")).unwrap();
        assert_eq!(run(&repo, "claude").0, "", "while the account is the CLI's sign-in the folder runs on that");
    }

    #[test]
    fn listing_gives_each_pin_its_state_and_lines() {
        let pins = [ProjectPin { folder: "C:\\code\\a".into(), provider: Provider::Codex, id: "user-2:acct-2".into() }];
        let mut states = std::collections::HashMap::new();
        states.insert((Provider::Codex, "user-2:acct-2".to_string()), ProfileState::AtDefault);
        let root = PathBuf::from("C:\\Users\\test\\AppData\\Roaming\\dev.thelio.slopbar\\profiles");
        let launcher = PathBuf::from("C:\\Users\\test\\AppData\\Roaming\\dev.thelio.slopbar\\bin\\slopbar-run.cmd");
        let listed = listing(&pins, &states, |_, _| Some("Spare".into()), &root, Some(&launcher));
        let pin = &listed.pins[0];
        assert_eq!((pin.state, pin.account.as_deref(), pin.env_var), (ProfileState::AtDefault, Some("Spare"), "CODEX_HOME"));
        assert!(pin.powershell.starts_with("$env:CODEX_HOME = 'C:\\Users\\test\\"), "{}", pin.powershell);
        assert!(pin.envrc.starts_with("export CODEX_HOME='/mnt/c/Users/test/AppData/Roaming/dev.thelio.slopbar/profiles/codex-"), "{}", pin.envrc);
        assert!(pin.launch.ends_with("slopbar-run.cmd' codex"));
        assert_eq!(listing(&pins, &Default::default(), |_, _| None, &root, None).pins[0].state, ProfileState::NoLogin);
        assert_eq!(wsl_path("D:\\x y\\z"), "/mnt/d/x y/z");
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
    }
}
