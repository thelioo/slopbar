<#
.SYNOPSIS
  Measures a SlopBar build's idle footprint beside the installed app, without touching it.

.DESCRIPTION
  Copies the installed app data to a temporary folder (without monitoring sessions, whose
  refresh tokens rotate, and without diagnostics), turns off everything that reaches outside
  the app (sharing, automatic switching and sign-ins, notifications, updates, the hotkey), and
  starts the build with SLOPBAR_DATA_DIR on that copy under a different executable name, so
  the installed instance neither counts nor ends it. After a warm-up it sums the private bytes
  and working sets of the process and its WebView2 descendants, and their CPU time over the
  sample, then ends the build and removes the copy.

.EXAMPLE
  pwsh scripts/measure-footprint.ps1 -Exe src-tauri/target/release/slopbar.exe

.EXAMPLE
  pwsh scripts/measure-footprint.ps1 -Exe src-tauri/target/release/slopbar.exe -Widget web
  The same build with the web taskbar widget (native_widget off), to compare with -Widget native.
#>
param(
  [Parameter(Mandatory)] [string] $Exe,
  [int] $WarmupSeconds = 150,
  [int] $SampleSeconds = 30,
  [string] $Source = (Join-Path $env:APPDATA 'dev.thelio.slopbar'),
  [switch] $Keep,
  # Which taskbar widget to measure (setting native_widget); the copy's own value when omitted.
  [ValidateSet('', 'native', 'web')] [string] $Widget = ''
)
$ErrorActionPreference = 'Stop'

$root = Join-Path $env:TEMP ("slopbar-footprint-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $root 'appdata'
New-Item -ItemType Directory -Force $data | Out-Null
Get-ChildItem -LiteralPath $Source -Force | Where-Object { $_.Name -notin @('monitoring', 'diagnostics') } |
  ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $data -Recurse -Force }

$settingsPath = Join-Path $data 'settings.json'
if (Test-Path $settingsPath) {
  $s = Get-Content -Raw $settingsPath | ConvertFrom-Json
  $s.presence_folder = $null
  $s.presence_remote = $null
  $s.auto_connect_monitoring = $false
  $s.auto_update = $false
  $s.switcher_hotkey = ''
  foreach ($p in 'claude', 'codex') { $s.balancer.$p.auto = $false }
  foreach ($n in @($s.notifications.PSObject.Properties.Name)) { $s.notifications.$n = $false }
  if ($s.phone) { $s.phone.enabled = $false }
  if ($s.calendar) { $s.calendar.enabled = $false }
  if ($Widget) { $s | Add-Member -Force -NotePropertyName native_widget -NotePropertyValue ($Widget -eq 'native') }
  $s | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8 $settingsPath
}

$copy = Join-Path $root 'slopbar-measure.exe'
Copy-Item -LiteralPath $Exe -Destination $copy
$env:SLOPBAR_DATA_DIR = $data
$proc = Start-Process -FilePath $copy -PassThru
Remove-Item Env:SLOPBAR_DATA_DIR

function Get-Tree([int] $RootId) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
  $ids = [System.Collections.Generic.HashSet[int]]::new()
  [void]$ids.Add($RootId)
  do {
    $added = $false
    foreach ($p in $all) {
      if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added = $true }
    }
  } while ($added)
  $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
}

try {
  Start-Sleep -Seconds $WarmupSeconds
  $before = @{}
  foreach ($p in Get-Tree $proc.Id) { $before[$p.Id] = $p.TotalProcessorTime.TotalMilliseconds }
  $threads = @{}
  foreach ($t in (Get-Process -Id $proc.Id).Threads) { $threads[$t.Id] = $t.TotalProcessorTime.TotalMilliseconds }
  $clock = [System.Diagnostics.Stopwatch]::StartNew()
  Start-Sleep -Seconds $SampleSeconds
  $elapsed = $clock.Elapsed.TotalMilliseconds
  $tree = @(Get-Tree $proc.Id)
  # The app's busiest threads (ids only: Windows keeps no names for them here).
  $busy = foreach ($t in (Get-Process -Id $proc.Id).Threads) {
    [pscustomobject]@{ Thread = $t.Id; StartedAfterSeconds = [math]::Round(($t.StartTime - $proc.StartTime).TotalSeconds, 1); CpuPercentOfCore = [math]::Round(100 * ($t.TotalProcessorTime.TotalMilliseconds - ($threads[$t.Id] ?? 0)) / $elapsed, 2) }
  }
  $lines = @{}
  foreach ($c in Get-CimInstance Win32_Process) { $lines[[int]$c.ProcessId] = $c.CommandLine }
  $rows = foreach ($p in $tree) {
    $cpu = $p.TotalProcessorTime.TotalMilliseconds - ($before[$p.Id] ?? 0)
    $kind = if ($lines[$p.Id] -match '--type=(\S+)') { $Matches[1] } elseif ($p.Id -eq $proc.Id) { 'app' } else { 'browser' }
    [pscustomobject]@{
      Pid = $p.Id
      Name = $p.ProcessName
      Type = $kind
      PrivateMB = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
      WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
      CpuPercentOfCore = [math]::Round(100 * $cpu / $elapsed, 2)
    }
  }
  $rows | Sort-Object PrivateMB -Descending | Format-Table -AutoSize | Out-String | Write-Output
  $busy | Sort-Object CpuPercentOfCore -Descending | Select-Object -First 5 | Format-Table -AutoSize | Out-String | Write-Output
  $app = $rows | Where-Object Pid -eq $proc.Id
  $web = $rows | Where-Object Pid -ne $proc.Id
  [pscustomobject]@{
    AppPrivateMB = $app.PrivateMB
    WebViewProcesses = @($web).Count
    WebViewPrivateMB = [math]::Round(($web | Measure-Object PrivateMB -Sum).Sum, 1)
    TotalPrivateMB = [math]::Round(($rows | Measure-Object PrivateMB -Sum).Sum, 1)
    TotalWorkingSetMB = [math]::Round(($rows | Measure-Object WorkingSetMB -Sum).Sum, 1)
    TotalCpuPercentOfCore = [math]::Round(($rows | Measure-Object CpuPercentOfCore -Sum).Sum, 2)
  } | Format-List | Out-String | Write-Output
} finally {
  foreach ($p in @(Get-Tree $proc.Id) | Sort-Object Id -Descending) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 2
  if (-not $Keep) {
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    # The isolated identifier's WebView2 folder.
    Remove-Item -Recurse -Force (Join-Path $env:LOCALAPPDATA 'dev.thelio.slopbar.isolated') -ErrorAction SilentlyContinue
  }
}
