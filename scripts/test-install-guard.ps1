<#
Tests install-guard.ps1 with a synthetic lock name; touches no SlopBar install or process.

  pwsh scripts/test-install-guard.ps1
#>
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install-guard.ps1')
$failures = 0
function Check([string]$what, [bool]$ok) {
  if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $script:failures++ }
}

# The verdict on how many copies run.
Check 'none running starts one' ((Get-InstanceVerdict 0) -eq 'start')
Check 'one running is restarted' ((Get-InstanceVerdict 1) -eq 'restart')
Check 'two running are refused' ((Get-InstanceVerdict 2) -eq 'refuse')
Check 'five running are refused' ((Get-InstanceVerdict 5) -eq 'refuse')

# The lock, held by another process (as a parallel install would).
$name = "Local\SlopBar-install-test-$PID"
$guard = Join-Path $PSScriptRoot 'install-guard.ps1'
$holder = Start-Process -FilePath (Get-Process -Id $PID).Path -PassThru -WindowStyle Hidden -ArgumentList @(
  '-NoProfile', '-Command', ". '$guard'; `$m = Enter-InstallLock '$name'; if (-not `$m) { exit 3 }; Start-Sleep -Seconds 30")
$deadline = (Get-Date).AddSeconds(20)
do {
  Start-Sleep -Milliseconds 200
  $probe = Enter-InstallLock $name
  if ($probe) { Exit-InstallLock $probe }
} while ($probe -and (Get-Date) -lt $deadline -and -not $holder.HasExited)
Check 'a second install is refused while one holds the lock' (-not $probe)

# The holder dies without releasing: its lock is abandoned and free.
Stop-Process -Id $holder.Id -Force
$holder.WaitForExit()
$taken = Enter-InstallLock $name
Check 'a lock left by a dead install is free' ([bool]$taken)
Exit-InstallLock $taken

# Released normally: free again.
$again = Enter-InstallLock $name
Check 'a released lock is free' ([bool]$again)
Exit-InstallLock $again

# Rollback pruning, on synthetic folders.
$root = Join-Path ([IO.Path]::GetTempPath()) "slopbar-rollback-test-$PID"
$names = '20261001-100000', '20261002-100000', '20261003-100000', '20261004-100000', '20261005-100000', '20261006-100000'
foreach ($n in $names) {
  $null = New-Item -ItemType Directory -Force (Join-Path $root $n)
  Set-Content -LiteralPath (Join-Path $root "$n\slopbar.exe") -Value 'x'
}
$null = New-Item -ItemType Directory -Force (Join-Path $root 'notes')
$running = @((Join-Path $root '20261002-100000\slopbar-mcp.exe'))
# The install just made the oldest-named folder (a clock set back): it stays too.
$removed = Remove-OldRollbacks $root (Join-Path $root '20261001-100000') $running
$left = @(Get-ChildItem -LiteralPath $root -Directory | ForEach-Object Name | Sort-Object)
Check 'the newest three rollbacks are kept' ($left -contains '20261006-100000' -and $left -contains '20261005-100000' -and $left -contains '20261004-100000')
Check 'the rollback just made is never removed' ($left -contains '20261001-100000')
Check 'a rollback an executable runs from is kept' ($left -contains '20261002-100000')
Check 'other old rollbacks are removed' (-not ($left -contains '20261003-100000') -and $removed.Count -eq 1)
Check 'folders that are not rollbacks are left alone' ($left -contains 'notes')
$again = Remove-OldRollbacks $root '' @()
Check 'once nothing holds them, only the newest three are left' ((@(Get-ChildItem -LiteralPath $root -Directory).Count) -eq 4 -and $again.Count -eq 2)
Check 'a missing rollback root is fine' ((@(Remove-OldRollbacks (Join-Path $root 'absent') '' @())).Count -eq 0)
Remove-Item -LiteralPath $root -Recurse -Force

if ($failures) { Write-Output "$failures failed"; exit 1 }
Write-Output 'all passed'
