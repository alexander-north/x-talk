param([ValidateSet('toggle','cancel')][string]$Action='toggle')
$ErrorActionPreference='Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class KeysForTest {
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
 [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from,uint to,bool attach);
 [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint id);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
 public static void Press(byte key) { keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(0x12,0,0,UIntPtr.Zero);keybd_event(key,0,0,UIntPtr.Zero);keybd_event(key,0,0,UIntPtr.Zero);keybd_event(key,0,2,UIntPtr.Zero);keybd_event(0x12,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero); }
}
'@
$notepad=Start-Process notepad.exe -PassThru
try {
 $notepad.WaitForInputIdle(5000) | Out-Null
 $deadline=[DateTime]::UtcNow.AddSeconds(5)
 do {$notepad.Refresh();if($notepad.MainWindowHandle -ne [IntPtr]::Zero){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
 [uint32]$owner=0
 $foregroundThread=[KeysForTest]::GetWindowThreadProcessId([KeysForTest]::GetForegroundWindow(),[ref]$owner)
 $currentThread=[KeysForTest]::GetCurrentThreadId()
 $attached=$false
 try {
  if($foregroundThread -ne 0 -and $foregroundThread -ne $currentThread){$attached=[KeysForTest]::AttachThreadInput($currentThread,$foregroundThread,$true)}
  [KeysForTest]::BringWindowToTop($notepad.MainWindowHandle) | Out-Null
  [KeysForTest]::SetForegroundWindow($notepad.MainWindowHandle) | Out-Null
 } finally {if($attached){[KeysForTest]::AttachThreadInput($currentThread,$foregroundThread,$false) | Out-Null}}
 Start-Sleep -Milliseconds 300
 [uint32]$foreground=0
 [KeysForTest]::GetWindowThreadProcessId([KeysForTest]::GetForegroundWindow(),[ref]$foreground) | Out-Null
 if($foreground -ne $notepad.Id){throw 'Could not give another application focus'}
 $key=switch($Action){'toggle'{0x20}'cancel'{0x1B}}
 [KeysForTest]::Press($key)
 Start-Sleep -Milliseconds 150
 Write-Output 'Shortcut sent with Notepad in foreground.'
} finally {if(!$notepad.HasExited){$notepad.CloseMainWindow() | Out-Null}}
