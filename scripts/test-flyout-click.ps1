<#
.SYNOPSIS
  Clicks the running SlopBar's taskbar widget and checks that the flyout is really drawn on screen.

.DESCRIPTION
  Posts a click to the native widget window (class SlopBarTaskbarWidget), waits for the flyout
  to become visible, fronts it the way a real click would, raises it topmost and captures the
  screen where it is with BitBlt (SRCCOPY|CAPTUREBLT). A second click closes it, and the same
  rectangle is captured again. If the two captures match, nothing of the flyout reached the
  screen (a blank or transparent panel).

  PrintWindow is deliberately not used: it asks the window to repaint into a bitmap, which hides
  exactly the bug this looks for (a flyout that is shown but never composited).

  FAIL means: the flyout did not appear within -TimeoutMs (2 s), or the capture matches what is
  behind it by more than 50%, or it has fewer than 20 distinct colours. The two PNGs are written
  to -OutDir (%TEMP% by default), never into the repository.

  Run it in an unlocked, interactive desktop session. It refuses to run when there is no
  foreground window (a locked screen, a disconnected remote session): nothing composites then,
  and every capture is meaningless.

  Exit codes: 0 PASS, 1 FAIL, 2 could not run.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/test-flyout-click.ps1

.EXAMPLE
  1..5 | % { powershell -ExecutionPolicy Bypass -File scripts/test-flyout-click.ps1 }
#>
param(
  [int]$WaitMs = 400,       # how long the flyout gets to draw before the capture
  [int]$TimeoutMs = 2000,   # how long it may take to become visible
  [string]$OutDir = $env:TEMP
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public static class FlyoutTest {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool AllowSetForegroundWindow(uint pid);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr d, int x, int y, int w, int h, IntPtr s, int sx, int sy, uint op);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  // A real click leaves the flyout in front with focus. Attaching to the foreground thread's
  // input lets this process hand the foreground over the same way.
  public static bool Front(IntPtr h) {
    uint p; uint fg = GetWindowThreadProcessId(GetForegroundWindow(), out p); uint me = GetCurrentThreadId();
    AttachThreadInput(me, fg, true); bool ok = SetForegroundWindow(h); AttachThreadInput(me, fg, false); return ok;
  }
  public static List<IntPtr> Visible(uint pid, string skipClass) {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { var sb = new StringBuilder(256); GetClassName(h, sb, 256);
        if (sb.ToString() != skipClass) list.Add(h); } return true; }, IntPtr.Zero);
    return list;
  }
}
"@
$T = [FlyoutTest]
$WidgetClass = 'SlopBarTaskbarWidget'
$WM_LBUTTONDOWN = 0x0201; $WM_LBUTTONUP = 0x0202
$SRCCOPY_CAPTUREBLT = 0x40CC0020
$HWND_TOPMOST = [IntPtr](-1); $HWND_NOTOPMOST = [IntPtr](-2)
$SWP_NOSIZE_NOMOVE_NOACTIVATE = 0x13

# Real pixels, like the app (per-monitor DPI aware v2), so window rectangles match the screen.
[void]$T::SetProcessDpiAwarenessContext([IntPtr](-4))

if ($T::GetForegroundWindow() -eq [IntPtr]::Zero) {
  Write-Host 'CANNOT RUN: there is no foreground window (locked screen or disconnected session). Nothing is composited, so any capture would be meaningless. Run this on an unlocked, interactive desktop.'
  exit 2
}
$widget = $T::FindWindow($WidgetClass, [NullString]::Value)
if ($widget -eq [IntPtr]::Zero) {
  Write-Host "CANNOT RUN: no $WidgetClass window. Is SlopBar running with the native taskbar widget on?"
  exit 2
}
[uint32]$appPid = 0; [void]$T::GetWindowThreadProcessId($widget, [ref]$appPid)

function Click-Widget {
  $lp = [IntPtr]((10 -shl 16) -bor 10)
  [void]$T::PostMessage($widget, $WM_LBUTTONDOWN, [IntPtr]1, $lp)
  [void]$T::PostMessage($widget, $WM_LBUTTONUP, [IntPtr]0, $lp)
}
function Capture([FlyoutTest+RECT]$r) {
  $w = $r.R - $r.L; $h = $r.B - $r.T
  $bmp = New-Object Drawing.Bitmap $w, $h
  $g = [Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc(); $screen = $T::GetDC([IntPtr]::Zero)
  [void]$T::BitBlt($hdc, 0, 0, $w, $h, $screen, $r.L, $r.T, $SRCCOPY_CAPTUREBLT)
  [void]$T::ReleaseDC([IntPtr]::Zero, $screen); $g.ReleaseHdc($hdc); $g.Dispose()
  $bmp
}

$before = $T::Visible($appPid, $WidgetClass)
[void]$T::AllowSetForegroundWindow($appPid)   # a real click lets the app take the foreground
$sw = [Diagnostics.Stopwatch]::StartNew()
Click-Widget
$flyout = [IntPtr]::Zero
while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
  $new = $T::Visible($appPid, $WidgetClass) | Where-Object { $before -notcontains $_ }
  if ($new) { $flyout = @($new)[0]; break }
  Start-Sleep -Milliseconds 10
}
$visibleMs = $sw.ElapsedMilliseconds
if ($flyout -eq [IntPtr]::Zero) {
  Write-Host "FAIL: the flyout did not become visible within $TimeoutMs ms. (If it was already open, the click closed it: close it and run again.)"
  exit 1
}

$fronted = $T::Front($flyout)
Start-Sleep -Milliseconds $WaitMs
$r = New-Object FlyoutTest+RECT; [void]$T::GetWindowRect($flyout, [ref]$r)
[void]$T::SetWindowPos($flyout, $HWND_TOPMOST, 0, 0, 0, 0, $SWP_NOSIZE_NOMOVE_NOACTIVATE)
Start-Sleep -Milliseconds 150
$shot = Capture $r
[void]$T::SetWindowPos($flyout, $HWND_NOTOPMOST, 0, 0, 0, 0, $SWP_NOSIZE_NOMOVE_NOACTIVATE)

# A second click closes it, as for a user; then capture what was behind it.
Start-Sleep -Milliseconds 300
Click-Widget
$closeWatch = [Diagnostics.Stopwatch]::StartNew()
while ($T::IsWindowVisible($flyout) -and $closeWatch.ElapsedMilliseconds -lt 2000) { Start-Sleep -Milliseconds 10 }
$closed = -not $T::IsWindowVisible($flyout)
Start-Sleep -Milliseconds 500   # let the close animation finish
$behind = Capture $r

# Sample a grid: distinct colours (to 4 bits a channel) and exact matches with what is behind.
$colors = @{}; $same = 0; $n = 0
for ($y = 0; $y -lt $shot.Height; $y += 6) {
  for ($x = 0; $x -lt $shot.Width; $x += 6) {
    $a = $shot.GetPixel($x, $y).ToArgb(); $n++
    $colors[$a -band 0xF0F0F0] = 1
    if ($a -eq $behind.GetPixel($x, $y).ToArgb()) { $same++ }
  }
}
$match = if ($n) { $same / $n } else { 1 }
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$shotPath = Join-Path $OutDir "slopbar-flyout-$stamp.png"
$behindPath = Join-Path $OutDir "slopbar-flyout-$stamp-behind.png"
$shot.Save($shotPath, [Drawing.Imaging.ImageFormat]::Png); $behind.Save($behindPath, [Drawing.Imaging.ImageFormat]::Png)
$shot.Dispose(); $behind.Dispose()

$reasons = @()
if ($match -gt 0.5) { $reasons += 'the capture matches what is behind the flyout by more than 50%' }
if ($colors.Count -lt 20) { $reasons += 'the capture has fewer than 20 distinct colours' }
Write-Host ("visible after:   {0} ms" -f $visibleMs)
Write-Host ("flyout:          {0}x{1} at {2},{3}; fronted: {4}" -f ($r.R - $r.L), ($r.B - $r.T), $r.L, $r.T, $fronted)
Write-Host ("distinct colours: {0}" -f $colors.Count)
Write-Host ("match behind:    {0:P0}" -f $match)
if (-not $closed) { Write-Host 'warning:         the flyout was still visible after the closing click, so the behind capture includes it' }
Write-Host "shot:            $shotPath"
Write-Host "behind:          $behindPath"
if ($reasons) { Write-Host ('FAIL: ' + ($reasons -join '; ')); exit 1 }
Write-Host 'PASS'
exit 0
