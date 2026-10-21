param([Parameter(Mandatory=$true,ParameterSetName='Select')][string]$Path,[Parameter(Mandatory=$true,ParameterSetName='Cancel')][switch]$Cancel,[Parameter(Mandatory=$true,ParameterSetName='Accept')][switch]$Accept)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;using System.Text;using System.Collections.Generic;using System.Runtime.InteropServices;
public class NativeFilePicker {
 public delegate bool EnumWindow(IntPtr h,IntPtr p);
 [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h,EnumWindow cb,IntPtr p);
 [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h,StringBuilder b,int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint msg,IntPtr w,string text);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
 public static IntPtr Filename(IntPtr dialog) {
  IntPtr found=IntPtr.Zero;
  EnumChildWindows(dialog,delegate(IntPtr h,IntPtr p){var b=new StringBuilder(128);GetClassName(h,b,128);if(b.ToString()=="Edit" && (GetDlgCtrlID(h)==1001 || GetDlgCtrlID(h)==1148)){found=h;return false;}return true;},IntPtr.Zero);
  return found;
 }
 public static string Controls(IntPtr dialog) {
  var all=new List<string>();
  EnumChildWindows(dialog,delegate(IntPtr h,IntPtr p){var b=new StringBuilder(128);GetClassName(h,b,128);all.Add(GetDlgCtrlID(h)+":"+b.ToString());return true;},IntPtr.Zero);
  return string.Join(",",all);
 }
}
'@
$deadline=[DateTime]::UtcNow.AddSeconds(30)
$condition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty,'#32770')
$lastFailure='Native file dialog was not found'
while([DateTime]::UtcNow -lt $deadline) {
 try {
  # Explorer may replace elements while loading. Reacquire the complete tree on each attempt.
  $root=[System.Windows.Automation.AutomationElement]::RootElement
  $dialogs=$root.FindAll([System.Windows.Automation.TreeScope]::Children,$condition)
  foreach($dialog in $dialogs) {
   if($dialog.Current.Name -notin @('Open','Save As')){continue}
   $owner=Get-Process -Id $dialog.Current.ProcessId -ErrorAction SilentlyContinue
   if(!$owner -or $owner.ProcessName -ne 'x-talk'){continue}
   $handle=[IntPtr]$dialog.Current.NativeWindowHandle
   if($Cancel){[NativeFilePicker]::PostMessage($handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null;exit 0}
   if($Accept){[NativeFilePicker]::PostMessage($handle,0x0111,[IntPtr]1,[IntPtr]::Zero) | Out-Null;exit 0}
   $edits=$dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,(New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Edit)))
   foreach($edit in $edits) {
    if($edit.Current.Name -notmatch 'File name'){continue}
    $value=$edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    $edit.SetFocus()
    $value.SetValue($Path)
    $buttons=$dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,(New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button)))
    foreach($button in $buttons){if($button.Current.Name -in @('Open','Save')){$button.SetFocus();$invoke=$button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern);$invoke.Invoke();exit 0}}
    $lastFailure='File dialog action button was not found'
   }
   $nativeEdit=[NativeFilePicker]::Filename($handle)
   if($nativeEdit -ne [IntPtr]::Zero){
    [NativeFilePicker]::SendMessage($nativeEdit,0x000C,[IntPtr]::Zero,$Path) | Out-Null
    [NativeFilePicker]::PostMessage($handle,0x0111,[IntPtr]1,[IntPtr]::Zero) | Out-Null
    exit 0
   }
   $lastFailure='File name input was not found; native controls: '+[NativeFilePicker]::Controls($handle)
   # Retain the native control diagnostics when UI Automation has no filename element.
  }
 } catch {$lastFailure='Native dialog elements were replaced before selection completed'}
 Start-Sleep -Milliseconds 150
}
throw $lastFailure
