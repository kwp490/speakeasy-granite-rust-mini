<#
.SYNOPSIS
Reports the real window state of a running SpeakEasy: which windows exist, which
are visible, and which own a taskbar button.

.DESCRIPTION
The startup flip makes four claims that all look alike from the code and differ
only at runtime: the transcriber starts visible, settings starts hidden, the
transcriber owns a taskbar button so minimize is recoverable, and settings can
be re-created after it is closed.

`Invoke-TranscriberProof.ps1` drives the transcriber's controls; this reports the
window facts those claims rest on. Both exist because each of these looked
"obviously fine" from the code and was not.

This no longer claims to know whether a window owns a taskbar button, because it
cannot, and the previous answer was wrong.

That earlier version documented "Tauri implements `skipTaskbar` with
WS_EX_TOOLWINDOW" and reported `TaskbarButton = WS_EX_APPWINDOW -or -not
WS_EX_TOOLWINDOW`. It is not true of the pinned tao: `set_skip_taskbar`
(tao-0.35.3, platform_impl/windows/window.rs:1529) calls
`ITaskbarList::DeleteTab`/`AddTab` and never touches an extended style.
Measured 2026-08-11: `hud-dock` is declared `"skipTaskbar": true` and reports
WS_EX_TOOLWINDOW **clear**, so the old column claimed it owned a taskbar button
when it does not.

DeleteTab leaves no readable trace on the window, so absence cannot be confirmed
from the window at all. The shell's real button list is not reachable here
either: Windows 11's taskbar is XAML, so it has no `MSTaskListWClass` toolbar to
send TB_GETBUTTON to, and `IUIAutomation` has no IDispatch, so PowerShell cannot
late-bind it without C# interop scaffolding.

So the two things that *are* knowable are reported separately, and the
unknowable one is not guessed at:

  * `SkipTaskbarDeclared` -- what `tauri.conf.json` asks for.
  * `StyleAllowsButton`   -- whether the extended styles permit a button. A
    window can have this true and still have no button, which is exactly the
    `hud-dock` case.

A disagreement between the two columns is the interesting signal. To confirm a
button really exists, capture the taskbar with the app running and again with it
stopped and compare -- that is how the blank tray icon was pinned down.

.EXAMPLE
./scripts/Get-SpeakEasyWindows.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public struct SpeakEasyProbeRect { public int Left, Top, Right, Bottom; }

public static class SpeakEasyProbe {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out SpeakEasyProbeRect r);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  // System-DPI awareness is not enough on a multi-monitor host with mixed
  // scaling: coordinates come back virtualized and a click lands hundreds of
  // pixels away. Verified on this host — a taskbar button at y=2088 was clicked
  // at y=2159 and the restore silently did nothing. Per-monitor-v2 first, with
  // the older call as a fallback for anything that refuses it.
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  public delegate bool EnumProc(IntPtr h, IntPtr p);

  public const int GWL_EXSTYLE = -20, GWL_STYLE = -16;
  public const int WS_EX_NOACTIVATE = 0x08000000, WS_EX_TOPMOST = 0x00000008;
  public const int WS_EX_APPWINDOW = 0x00040000, WS_EX_TOOLWINDOW = 0x00000080;

  public static System.Collections.Generic.List<IntPtr> All() {
    var found = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, p) => { found.Add(h); return true; }, IntPtr.Zero);
    return found;
  }
  public static string Text(IntPtr h) { var b = new StringBuilder(512); GetWindowTextW(h, b, 512); return b.ToString(); }
  public static string Cls(IntPtr h) { var b = new StringBuilder(512); GetClassNameW(h, b, 512); return b.ToString(); }
}
'@ -ErrorAction SilentlyContinue

# Per-monitor-v2 DPI awareness (-4). System awareness is not enough on a
# multi-monitor host with mixed scaling.
if (-not [SpeakEasyProbe]::SetProcessDpiAwarenessContext([IntPtr](-4))) {
    [void][SpeakEasyProbe]::SetProcessDPIAware()
}


# What the config asks for, keyed by window title, so the declared intent can be
# shown next to what the styles permit. Titles are the only thing the enumeration
# has to join on -- a window handle carries no Tauri label.
$configPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'apps\desktop\src-tauri\tauri.conf.json'
$declaredSkipTaskbar = @{}
if (Test-Path -LiteralPath $configPath) {
    foreach ($window in (Get-Content -Raw -Encoding utf8 $configPath | ConvertFrom-Json).app.windows) {
        if ($window.title) {
            $declaredSkipTaskbar[[string]$window.title] = [bool]$window.skipTaskbar
        }
    }
}

$windows = foreach ($handle in [SpeakEasyProbe]::All()) {
    if ([SpeakEasyProbe]::Cls($handle) -ne 'Tauri Window') { continue }
    $title = [SpeakEasyProbe]::Text($handle)
    if ($title -notlike '*SpeakEasy*') { continue }

    $exStyle = [SpeakEasyProbe]::GetWindowLongW($handle, [SpeakEasyProbe]::GWL_EXSTYLE)
    $rect = New-Object SpeakEasyProbeRect
    [void][SpeakEasyProbe]::GetWindowRect($handle, [ref]$rect)
    $dpi = [SpeakEasyProbe]::GetDpiForWindow($handle)
    $scale = if ($dpi -gt 0) { [math]::Round($dpi / 96, 2) } else { 0 }

    $toolWindow = ($exStyle -band [SpeakEasyProbe]::WS_EX_TOOLWINDOW) -ne 0
    $appWindow = ($exStyle -band [SpeakEasyProbe]::WS_EX_APPWINDOW) -ne 0

    [pscustomobject]@{
        Title       = $title
        Visible     = [SpeakEasyProbe]::IsWindowVisible($handle)
        Minimized   = [SpeakEasyProbe]::IsIconic($handle)
        # Declared intent and permitted-by-style, never a claim that a button
        # exists: tao removes one through ITaskbarList without changing a style,
        # so `StyleAllowsButton = True` alongside `SkipTaskbarDeclared = True`
        # means the button is gone despite the styles allowing it. See the header.
        SkipTaskbarDeclared = if ($declaredSkipTaskbar.ContainsKey($title)) {
            $declaredSkipTaskbar[$title]
        } else {
            $null
        }
        StyleAllowsButton = $appWindow -or (-not $toolWindow)
        NoActivate  = ($exStyle -band [SpeakEasyProbe]::WS_EX_NOACTIVATE) -ne 0
        AlwaysOnTop = ($exStyle -band [SpeakEasyProbe]::WS_EX_TOPMOST) -ne 0
        Physical    = "$($rect.Right - $rect.Left)x$($rect.Bottom - $rect.Top)"
        Scale       = "${scale}x"
    }
}

if ($null -eq $windows) {
    Write-Host 'No SpeakEasy windows found. Is the app running?'
    exit 1
}

$windows | Format-Table -AutoSize | Out-String | Write-Host
