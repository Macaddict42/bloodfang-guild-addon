param(
  [Parameter(Mandatory = $true)][string]$Exe
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe" }
$proc = Start-Process -FilePath $Exe -ArgumentList "--hidden" -PassThru
try {
  Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class LaunchSmokeWin {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  public static List<IntPtr> ForPid(uint pid) {
    var found = new List<IntPtr>();
    EnumWindows((h, l) => {
      uint owner;
      GetWindowThreadProcessId(h, out owner);
      if (owner == pid) found.Add(h);
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@
  $windows = @()
  foreach ($i in 1..40) {
    if ($proc.HasExited) { throw "process exited during launch: $($proc.ExitCode)" }
    $windows = [LaunchSmokeWin]::ForPid([uint32]$proc.Id)
    if ($windows.Count -gt 0) { break }
    Start-Sleep -Milliseconds 500
  }
  if ($windows.Count -lt 1) { throw "no window for pid $($proc.Id)" }
  foreach ($hwnd in $windows) {
    [void][LaunchSmokeWin]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
  }
  Start-Sleep -Seconds 2
  $proc.Refresh()
  $kept = -not $proc.HasExited
  $restored = "skipped"
  if ($kept) {
    $visible = $false
    foreach ($hwnd in $windows) {
      [void][LaunchSmokeWin]::ShowWindow($hwnd, 5)
      if ([LaunchSmokeWin]::IsWindowVisible($hwnd)) { $visible = $true }
    }
    $restored = $(if ($visible) { "ShowWindow" } else { "not_visible" })
  }
  Write-Output "launch=ok close_kept_process=$kept restore=$restored exit_menu=not_run"
  if ($kept -and $restored -eq "not_visible") { throw "window stayed hidden after ShowWindow" }
} finally {
  if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
}
