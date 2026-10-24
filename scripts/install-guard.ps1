<#
Guards for install-local-build.ps1, dot-sourced by it and by test-install-guard.ps1.

Enter-InstallLock: one install at a time on this machine. Parallel installs each swapped the
executables and asked SlopBar to restart, and left two copies running.
Get-InstanceVerdict: what an install does given how many SlopBar copies run in this session.
#>

$script:InstallLockName = 'Global\SlopBar-install-local-build'

# Takes the machine-wide install lock without waiting; returns the held mutex, or $null when
# another install holds it. A lock left by an install that died counts as free.
function Enter-InstallLock([string]$Name = $script:InstallLockName) {
  $mutex = [Threading.Mutex]::new($false, $Name)
  try {
    if ($mutex.WaitOne(0)) { return $mutex }
  } catch [Threading.AbandonedMutexException] {
    return $mutex
  }
  $mutex.Dispose()
  return $null
}

function Exit-InstallLock($mutex) {
  if ($mutex) { $mutex.ReleaseMutex(); $mutex.Dispose() }
}

# 'start' (none running), 'restart' (exactly one), or 'refuse' (more than one: restarting one
# leaves the others renewing the same monitoring sign-ins).
function Get-InstanceVerdict([int]$Count) {
  if ($Count -le 0) { return 'start' }
  if ($Count -eq 1) { return 'restart' }
  return 'refuse'
}

# SlopBar app processes in this Windows session, wherever they run from (an install moves the
# running executable into a rollback folder). Ones started in the last few seconds are left out:
# another checkout's moment-long `--build-info-to`, or a restart still deciding whether to run.
function Get-SlopBarProcesses {
  $session = (Get-Process -Id $PID).SessionId
  $settled = (Get-Date).AddSeconds(-5)
  @(Get-Process -Name slopbar -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session -and $_.StartTime -lt $settled })
}

# Rollback folders kept after an install (each holds a full build).
$script:RollbackKept = 3

# Removes rollback folders under $Root beyond the newest $Keep (by name: yyyyMMdd-HHmmss), never
# $Current (the one this install just made) and never one an executable still runs from: the
# instance an install moved aside runs from there until it restarts, and agents' slopbar-mcp
# servers keep running from the folder of the build that started them. $RunningPaths are the
# running executables' full paths. Returns the folders removed.
function Remove-OldRollbacks([string]$Root, [string]$Current, [string[]]$RunningPaths, [int]$Keep = $script:RollbackKept) {
  if (-not (Test-Path -LiteralPath $Root)) { return @() }
  $folders = @(Get-ChildItem -LiteralPath $Root -Directory | Where-Object { $_.Name -match '^\d{8}-\d{6}$' } | Sort-Object Name -Descending)
  $current = if ($Current) { [IO.Path]::GetFullPath($Current).TrimEnd('\') } else { '' }
  $removed = @()
  foreach ($folder in ($folders | Select-Object -Skip $Keep)) {
    $full = $folder.FullName.TrimEnd('\')
    if ($full -ieq $current) { continue }
    $inUse = $RunningPaths | Where-Object { $_ -and $_.StartsWith($full + '\', [StringComparison]::OrdinalIgnoreCase) }
    if ($inUse) { continue }
    try {
      Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction Stop
      $removed += $full
    } catch {
      # A file opened meanwhile: the next install tries again.
    }
  }
  return $removed
}

# Full paths of the SlopBar executables running in this session (app and agents' MCP servers).
function Get-SlopBarExecutablePaths {
  $session = (Get-Process -Id $PID).SessionId
  @(Get-Process -Name slopbar, slopbar-mcp -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session } | ForEach-Object { $_.Path } | Where-Object { $_ })
}
