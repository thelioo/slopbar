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

if ($failures) { Write-Output "$failures failed"; exit 1 }
Write-Output 'all passed'
