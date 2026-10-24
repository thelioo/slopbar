# Bring up another computer with an agent

A field guide for setting up SlopBar on a second Windows computer from an agent (Claude Code,
Codex or any MCP client) running on a computer that already has your accounts. Nobody types a
roster passphrase; you only sit at the new computer's screen to complete each sign-in page.

What the agent does, in order:

1. `get_machine_readiness` with `ssh_host`: is SlopBar running there, is someone signed in at its
   desktop, is there a browser?
2. `pair_machine` with `ssh_host`: both computers share the account roster with the same key.
3. `run_on_machine` → `get_roster`: the accounts waiting to be signed in there.
4. You sit at the new computer and say so; the agent runs `run_on_machine` → `sign_in_all` with
   `confirm: true`, and guides you email by email.

## 1. Reach the other computer over SSH

The agent uses the `ssh` command with your own keys. Either OpenSSH or Tailscale works.

### OpenSSH (any network you can route over)

On the **new computer** (the one being set up), in an elevated PowerShell:

```powershell
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Start-Service sshd
Set-Service sshd -StartupType Automatic
# PowerShell as the shell for SSH sessions (SlopBar's commands work from cmd too).
New-ItemProperty -Path HKLM:\SOFTWARE\OpenSSH -Name DefaultShell -PropertyType String -Force `
  -Value "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
```

On the **computer with the accounts**, create a key once and copy its public half over:

```powershell
ssh-keygen -t ed25519                     # accept the default path; a passphrase is up to you
type $env:USERPROFILE\.ssh\id_ed25519.pub
```

On the new computer, put that line in the right file:

- a standard user: `C:\Users\<you>\.ssh\authorized_keys`
- a member of Administrators: `C:\ProgramData\ssh\administrators_authorized_keys`, then
  `icacls C:\ProgramData\ssh\administrators_authorized_keys /inheritance:r /grant Administrators:F /grant SYSTEM:F`

Give it a name on the computer with the accounts, in `C:\Users\<you>\.ssh\config`:

```
Host newpc
    HostName 192.168.1.42        # or its DNS name
    User <your Windows user name there>
```

Then connect once by hand so the host key is recorded (the agent never accepts an unknown host
key) and check that key auth works without a prompt:

```powershell
ssh newpc hostname
ssh -o BatchMode=yes newpc hostname
```

### Tailscale

Install Tailscale on both computers and sign in to the same tailnet. Use the new computer's
Tailscale name (`newpc` or `newpc.tailnet-name.ts.net`) as `HostName` above, or directly as
`ssh_host`. Windows has no Tailscale SSH server, so keep OpenSSH Server on the new computer as above;
Tailscale only gives the two computers a private route to each other wherever they are.

## 2. On the new computer, once, at its screen

- Install SlopBar and start it from the Start menu. Agents never start it over SSH: started from
  an SSH session it would run where nobody can see it.
- Install rclone and configure the same remote as on your other computer (`rclone config`), or
  sign in to the same synced folder (OneDrive, Dropbox…). Pairing points SlopBar at the same rclone
  remote by name; with a synced folder, choose it in Settings → Data → Other computers first.
- Leave Windows signed in. Locking is fine until the sign-ins start.

## 3. Point the agent at it

Register SlopBar's MCP server with your agent on the computer with the accounts (see "Agent
access" in the README), then ask, for example:

> Set up SlopBar on newpc over SSH and sign in my accounts there.

The agent should:

1. Call `get_machine_readiness` with `ssh_host: "newpc"`. `problems` names anything in the way:
   `no_desktop_session` (nobody is signed in at its desktop), `app_not_running`, `desktop_locked`,
   `desktop_not_shown` (a closed Remote Desktop or another user switched in), `no_browser`.
2. Call `pair_machine` with `ssh_host: "newpc"`. It uses this computer's roster passphrase (or
   generates a 256-bit key when there is none), sends it over the SSH connection's input to the
   new computer's SlopBar, which saves it with Windows DPAPI as it saves any roster passphrase,
   and syncs both rosters. The key never appears in the tool's arguments, results, errors or logs.
   Nothing changes on either computer until the new one has shown it can share where this one
   does. If the new computer already has a different roster passphrase, pairing refuses unless you
   tell the agent to replace it (`replace_key: true`).
3. Call `run_on_machine` with `tool: "get_roster"` to list the accounts waiting there.
4. Ask you to sit at the new computer's screen, and wait until you say you are there.
5. Call `run_on_machine` with `tool: "sign_in_all"` and `arguments: { "confirm": true }`. Sign-in
   pages open on the new computer's desktop, an email at a time in a fresh browser profile. You
   complete every page; the agent never signs in or approves anything. If the desktop is locked or
   not shown, nothing starts and the agent hears why.
6. Follow progress with `run_on_machine` → `get_roster` or `get_usage`.

## Without an MCP client

`slopbar-mcp.exe` runs one tool from a shell and prints one JSON line (`{"ok": …}` or
`{"error": "…"}`):

```powershell
& "$env:LOCALAPPDATA\SlopBar\slopbar-mcp.exe" --call get_machine_readiness ssh_host=newpc
& "$env:LOCALAPPDATA\SlopBar\slopbar-mcp.exe" --call pair_machine ssh_host=newpc
& "$env:LOCALAPPDATA\SlopBar\slopbar-mcp.exe" --call run_on_machine ssh_host=newpc tool=get_roster
```

PowerShell does not wait for this windowless program unless its output is captured, so pipe it,
for example `| Out-String`.

## Troubleshooting

| Error | Fix |
| --- | --- |
| SSH could not run the command (Permission denied) | The public key is not in the right `authorized_keys` file, or that file's permissions are too open (administrators: the `icacls` line above). |
| SSH could not run the command (Host key verification failed) | Connect once by hand with `ssh newpc` and accept the host key. |
| SlopBar is not running on the other computer's desktop | Start it there from the Start menu. |
| The other computer has no working rclone remote named … (or readiness lists `rclone_remote_missing`) | Install rclone there (`winget install Rclone.Rclone`) and create a remote with the same name with `rclone config` at its screen: the cloud sign-in opens in its browser. |
| The other computer answered without a SlopBar result | Update SlopBar there; pairing needs the same version on both. |
