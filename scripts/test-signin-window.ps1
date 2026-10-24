<#
.SYNOPSIS
  Shows that "Sign in all accounts" opens each email's sign-ins in a fresh, empty browser profile.

.DESCRIPTION
  Runs the ignored test `signin::tests::visual_sign_in_windows`, which opens the real Claude and
  Codex sign-in pages for synthetic emails in fresh profiles (Claude's through slopbar.exe as its
  BROWSER, exactly as the app does). At each step this captures the browser window from the
  screen with BitBlt (SRCCOPY|CAPTUREBLT) into -Out, so a person can check that:
    1-claude  a clean window offers no earlier account, with the email prefilled (its first tab names
              the account; the sign-in page is the tab in front)
    2-codex   Codex's page opens in the same browser and profile
    3-closed  ending the email's turn closes that browser and deletes its profile
    4-next    the next email gets a different fresh profile
  Nothing is typed, clicked or approved; the test closes every window it opened. Needs an
  unlocked desktop (a foreground window), the claude and codex CLIs, and a debug build
  (cargo build) for slopbar.exe.
#>
param(
  [string]$Out = (Join-Path $env:TEMP 'slopbar-signin-visual'),
  # On a machine without the sources (another PC, so the windows never open on the desktop being
  # worked in): a debug slopbar.exe, and the library's test binary
  # (target\debug\deps\slopbar_lib-*.exe from cargo test --no-run).
  [string]$Exe,
  [string]$TestExe
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$tauri = Join-Path $root 'src-tauri'
$exe = if ($Exe) { $Exe } else { Join-Path $tauri 'target\debug\slopbar.exe' }
if (-not (Test-Path $exe)) { Write-Host "CANNOT RUN: build $exe first (cargo build in src-tauri)."; exit 2 }

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public static class SigninShot {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr d, int x, int y, int w, int h, IntPtr s, int sx, int sy, uint op);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static bool Front(IntPtr h) {
    uint p; uint fg = GetWindowThreadProcessId(GetForegroundWindow(), out p); uint me = GetCurrentThreadId();
    AttachThreadInput(me, fg, true); bool ok = SetForegroundWindow(h); AttachThreadInput(me, fg, false); return ok;
  }
  // The browser's top-level windows: Chromium's are Chrome_WidgetWin_1.
  public static List<IntPtr> Windows(uint pid) {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { var sb = new StringBuilder(256); GetClassName(h, sb, 256);
        RECT r; GetWindowRect(h, out r);
        if (sb.ToString() == "Chrome_WidgetWin_1" && r.R - r.L > 200 && r.B - r.T > 200) list.Add(h); } return true; }, IntPtr.Zero);
    return list;
  }
}
"@
$T = [SigninShot]
[void]$T::SetProcessDpiAwarenessContext([IntPtr](-4))
if ($T::GetForegroundWindow() -eq [IntPtr]::Zero) {
  Write-Host 'CANNOT RUN: there is no foreground window (locked screen or disconnected session).'
  exit 2
}

function Save-Rect([SigninShot+RECT]$r, [string]$path) {
  $w = $r.R - $r.L; $h = $r.B - $r.T
  $bmp = New-Object Drawing.Bitmap $w, $h
  $g = [Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc(); $screen = $T::GetDC([IntPtr]::Zero)
  [void]$T::BitBlt($hdc, 0, 0, $w, $h, $screen, $r.L, $r.T, 0x40CC0020)
  [void]$T::ReleaseDC([IntPtr]::Zero, $screen); $g.ReleaseHdc($hdc); $g.Dispose()
  $bmp.Save($path, [Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}

New-Item -ItemType Directory -Force $Out | Out-Null
Get-ChildItem $Out -File | Remove-Item -Force
$sync = Join-Path $Out 'sync'; New-Item -ItemType Directory -Force $sync | Out-Null
$data = Join-Path $Out 'data'; New-Item -ItemType Directory -Force $data | Out-Null

$env:SLOPBAR_VISUAL_DIR = $sync; $env:SLOPBAR_EXE = $exe; $env:SLOPBAR_DATA_DIR = $data
$log = Join-Path $Out 'test.log'
$run = if ($TestExe) { @{ FilePath = $TestExe; ArgumentList = 'visual_sign_in_windows', '--ignored', '--nocapture'; WorkingDirectory = $Out } }
  else { @{ FilePath = 'cargo'; ArgumentList = 'test', '--lib', 'visual_sign_in_windows', '--', '--ignored', '--nocapture'; WorkingDirectory = $tauri } }
$test = Start-Process @run -RedirectStandardOutput $log -RedirectStandardError (Join-Path $Out 'build.log') -PassThru -WindowStyle Hidden

$report = [ordered]@{}
foreach ($step in '1-claude', '2-codex', '3-closed', '4-next', '5-closed') {
  $ready = Join-Path $sync "$step.ready"
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while (-not (Test-Path $ready) -and -not $test.HasExited -and $sw.Elapsed.TotalMinutes -lt 15) { Start-Sleep -Milliseconds 300 }
  if (-not (Test-Path $ready)) { Write-Host "FAIL: the test never reached $step (see $log)"; break }
  $text = Get-Content $ready -Raw
  $report[$step] = $text
  if ($text -match 'browser=Some\((\d+)\)') {
    $windows = $T::Windows([uint32]$Matches[1])
    $i = 0
    foreach ($w in $windows) {
      # Topmost while captured: a screen copy must show this window, never one of the user's.
      [void]$T::SetWindowPos($w, [IntPtr](-1), 0, 0, 0, 0, 0x13); [void]$T::Front($w); Start-Sleep -Milliseconds 1200
      $r = New-Object SigninShot+RECT; [void]$T::GetWindowRect($w, [ref]$r)
      Save-Rect $r (Join-Path $Out "$step-$i.png"); $i++
      [void]$T::SetWindowPos($w, [IntPtr](-2), 0, 0, 0, 0, 0x13)
    }
    $report[$step] += " windows=$i"
  }
  New-Item -ItemType File (Join-Path $sync "$step.go") | Out-Null
}
$test.WaitForExit()
$report.GetEnumerator() | ForEach-Object { Write-Host ("{0}: {1}" -f $_.Key, $_.Value.Trim()) }
Write-Host "Captures in $Out"
Get-ChildItem $Out -Filter *.png | ForEach-Object { Write-Host "  $($_.FullName)" }
