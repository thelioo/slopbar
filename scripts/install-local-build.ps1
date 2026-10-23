<#
Installs this checkout's release build over the local SlopBar without anyone quitting it by hand.

  cargo build --locked --release --features tauri/custom-protocol --manifest-path src-tauri/Cargo.toml
  pwsh scripts/install-local-build.ps1

0. Takes a machine-wide install lock: a second install started meanwhile refuses (non-zero exit)
   instead of swapping executables and restarting SlopBar under the first.
1. Checks the built slopbar.exe reports this checkout's commit and tree, clean, with no dev mode.
   Refuses when more than one SlopBar is running: it asks the copy it reaches through the door to
   quit, waits, and stops if more than one is still left.
2. Moves the installed executables (even while running; Windows allows renaming a running image)
   into a rollback folder outside the repository, and copies the new ones in.
3. Asks the running SlopBar to restart itself through its agent door (slopbar-mcp restart_app),
   which exits exactly like the tray's Quit, so no token rotation is cut short. If SlopBar is not
   running, starts it.

-StopAfterRefresh is only for a running build older than restart_app: it asks that build to
finish a usage refresh through the door and stops it the moment the refresh returns. Monitoring
tokens rotate only during a refresh or a sign-in, so none is in flight at that moment. The
journal then records a missing clean shutdown, as after any crash.
4. Removes rollback folders beyond the newest three, except one an executable still runs from.

Rollback restores executables only; app data and monitoring sessions are never touched.
#>
param(
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'SlopBar'),
  [string]$RollbackRoot = (Join-Path $env:LOCALAPPDATA 'SlopBar-Rollback'),
  [switch]$StopAfterRefresh
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-guard.ps1')

# 0. One install at a time.
$lock = Enter-InstallLock
if (-not $lock) { throw 'Another install of SlopBar is running on this machine; run this again once it has finished.' }
# (The rest of the script runs inside this try; the lock is released however it ends.)
try {

$repo = Split-Path -Parent $PSScriptRoot
$built = Join-Path $repo 'src-tauri\target\release'
$names = 'slopbar.exe', 'slopbar-mcp.exe'

# 1. Provenance.
$head = (git -C $repo rev-parse HEAD).Trim()
$tree = (git -C $repo rev-parse 'HEAD^{tree}').Trim()
$infoPath = Join-Path ([IO.Path]::GetTempPath()) "slopbar-build-info-$PID.json"
$p = Start-Process -FilePath (Join-Path $built 'slopbar.exe') -ArgumentList @('--build-info-to', $infoPath) -WindowStyle Hidden -Wait -PassThru
if ($p.ExitCode -ne 0) { throw 'Build metadata command failed.' }
$info = Get-Content -LiteralPath $infoPath -Raw | ConvertFrom-Json
Remove-Item -LiteralPath $infoPath
if ($info.commit -ne $head -or $info.source_tree -ne $tree -or $info.dirty -or $info.tauri_dev -ne $false) {
  throw "Built executable does not match a clean build of $head; rebuild first."
}

# Runs slopbar-mcp.exe with these JSON-RPC lines on stdin and returns its output lines. (PowerShell's
# `&` does not capture a windowless executable's output, so this uses Process directly.)
function Invoke-Mcp([string]$mcp, [string[]]$lines) {
  $info = [Diagnostics.ProcessStartInfo]::new($mcp)
  $info.UseShellExecute = $false
  $info.RedirectStandardInput = $true
  $info.RedirectStandardOutput = $true
  $info.CreateNoWindow = $true
  $process = [Diagnostics.Process]::Start($info)
  foreach ($line in $lines) { $process.StandardInput.WriteLine($line) }
  $process.StandardInput.Close()
  $output = $process.StandardOutput.ReadToEnd()
  if (-not $process.WaitForExit(90000)) { $process.Kill(); throw 'slopbar-mcp did not finish.' }
  return $output -split "`r?`n" | Where-Object { $_ }
}

# Calls one door tool through slopbar-mcp.exe; returns the tool result, or $null if unreachable.
function Invoke-Door([string]$mcp, [string]$tool) {
  $out = Invoke-Mcp $mcp @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"install-local-build","version":"1"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    ('{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"' + $tool + '","arguments":{}}}')
  )
  foreach ($line in $out) {
    $m = $line | ConvertFrom-Json
    if ($m.id -eq 2) { return $m.result }
  }
  return $null
}

# The text of a failed tool result, for error messages.
function Get-DoorError($result) {
  if ($result -and $result.content) { return $result.content[0].text }
  return 'SlopBar did not answer'
}

# Exactly one SlopBar may be running: restarting one copy would leave the others renewing the same
# monitoring sign-ins. Quit the one the door reaches; a copy of an older build may be left.
$running = Get-SlopBarProcesses
if ((Get-InstanceVerdict $running.Count) -eq 'refuse') {
  $builtMcp = Join-Path $built 'slopbar-mcp.exe'
  $info = Invoke-Door $builtMcp 'get_app_info'
  $doorPid = if ($info -and -not $info.isError) { $info.structuredContent.pid } else { $null }
  $null = Invoke-Door $builtMcp 'quit_app'
  $deadline = (Get-Date).AddSeconds(30)
  do {
    Start-Sleep -Seconds 1
    $gone = if ($doorPid) { -not (Get-Process -Id $doorPid -ErrorAction SilentlyContinue) } else { (Get-SlopBarProcesses).Count -lt $running.Count }
  } until ($gone -or (Get-Date) -gt $deadline)
  $running = Get-SlopBarProcesses
  if ((Get-InstanceVerdict $running.Count) -eq 'refuse') {
    throw "$($running.Count) copies of SlopBar are running (process ids $($running.Id -join ', ')). End them in Task Manager, then run this again; nothing was installed."
  }
}

# 2. Swap executables; the running ones move aside intact.
$backup = Join-Path $RollbackRoot (Get-Date -Format 'yyyyMMdd-HHmmss')
$null = New-Item -ItemType Directory -Force -Path $backup
foreach ($name in $names) {
  $target = Join-Path $InstallDir $name
  if (Test-Path -LiteralPath $target) { Move-Item -LiteralPath $target -Destination (Join-Path $backup $name) }
  Copy-Item -LiteralPath (Join-Path $built $name) -Destination $target
  if ((Get-FileHash -LiteralPath $target).Hash -ne (Get-FileHash -LiteralPath (Join-Path $built $name)).Hash) { throw "Copy of $name failed." }
}
$newMcp = Join-Path $InstallDir 'slopbar-mcp.exe'

# 3. Restart into the new build.
if (-not $running) {
  Start-Process -FilePath (Join-Path $InstallDir 'slopbar.exe') -WindowStyle Hidden
} else {
  $r = Invoke-Door $newMcp 'restart_app'
  if (-not $r -or $r.isError) {
    # A running build older than restart_app does not understand it.
    if (-not $StopAfterRefresh) { throw "restart_app failed: $(Get-DoorError $r). Executables are swapped; re-run with -StopAfterRefresh if the running build predates restart_app." }
    $r = Invoke-Door $newMcp 'refresh_usage'
    if (-not $r -or $r.isError) { throw "refresh_usage failed: $(Get-DoorError $r). Executables are swapped; nothing was stopped." }
    $running | Stop-Process -Force
    $running | Wait-Process -Timeout 30
    Start-Process -FilePath (Join-Path $InstallDir 'slopbar.exe') -WindowStyle Hidden
  }
}

# Confirm the new build answers on the door.
$deadline = (Get-Date).AddSeconds(60)
do {
  Start-Sleep -Seconds 2
  $r = Invoke-Door $newMcp 'get_app_info'
} until (($r -and -not $r.isError -and $r.structuredContent.build.commit -eq $head) -or (Get-Date) -gt $deadline)
if (-not $r -or $r.isError -or $r.structuredContent.build.commit -ne $head) { throw 'The new build did not come up on the door within 60 seconds.' }
Write-Output "SlopBar $head is running. Previous executables: $backup"

# 4. Keep the newest few rollback folders; the old instance has exited by now. Folders an
# executable still runs from (agents' MCP servers of an older build) stay until a later install.
$pruned = Remove-OldRollbacks $RollbackRoot $backup (Get-SlopBarExecutablePaths)
if ($pruned) { Write-Output "Removed $($pruned.Count) older rollback folder(s)." }

} finally {
  Exit-InstallLock $lock
}
