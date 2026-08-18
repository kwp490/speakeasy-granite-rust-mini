<#
.SYNOPSIS
Measures a running native (Win32) window and its controls in real physical
pixels, from a per-monitor-DPI-aware process.

.DESCRIPTION
`Invoke-WebviewProbe.ps1` measures the app's WebView windows. This is the
equivalent for the ones that have no WebView in them — the bootstrapper's setup
wizard, drawn with `winsafe` — and it exists because the obvious way to measure
such a window is silently wrong.

**Windows lies to a DPI-unaware process about every coordinate it reads.**
`GetWindowRect` in a process that has not declared awareness is virtualized:
the numbers come back divided by the scale factor, so a window that is exactly
right and a window that never scaled at all report the same figures. That cost
this project an afternoon and a wrong entry in
`apps/bootstrapper/speakeasy-bootstrapper.manifest`. Measured 2026-08-15 from a
DPI-unaware probe on a 250%-scale display, the wizard's client area came back as
620x460 under PerMonitorV2, under system-aware, and with no manifest at all —
"three declarations, one number", read as evidence that the DPI declaration did
nothing and the window was rendering at 40% of its intended size. Re-measured
2026-08-17 from this probe, the same window is 1550x1150 physical, which is
620x460 logical at 240 dpi: correct, and correct under all three declarations.
The declaration was never the variable. The probe was.

So this script declares `PER_MONITOR_AWARE_V2` before it reads anything, and
refuses to report if that did not take effect. Everything it prints is physical
pixels.

`-Fit` additionally answers "does the text fit the box?", which is not the same
question as "is the window the right size" and does not have a DPI-independent
answer: Segoe UI 9pt occupies a 15 px cell at 96 dpi and a 41 px cell at 240 dpi,
where linear scaling would predict 37.5. A label sized in fixed logical units
therefore holds a slightly different number of lines at every scale, and a step
whose copy fits at 100% can clip at 250%. Measured 2026-08-17, the wizard's
findings label holds 10.4 lines at 240 dpi and 10.6 at 96 dpi against a longest
current content of 8, so there is headroom today — but each new wizard step adds
copy to that box, and this is how to check it rather than guess.

.PARAMETER TitleContains
Substring of the window title to match. Invisible windows are skipped.

.PARAMETER Fit
Also report, per label, how its text wraps and whether the result fits.

.EXAMPLE
./scripts/Measure-NativeWindow.ps1 -TitleContains 'SpeakEasy setup'

.EXAMPLE
./scripts/Measure-NativeWindow.ps1 -TitleContains 'SpeakEasy setup' -Fit
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$TitleContains,

    [switch]$Fit
)

$ErrorActionPreference = 'Stop'

if (-not ('SpeakEasy.NativeWindowProbe' -as [type])) {
    Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace SpeakEasy {
public static class NativeWindowProbe {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct SIZE { public int cx, cy; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct LOGFONT {
        public int lfHeight, lfWidth, lfEscapement, lfOrientation, lfWeight;
        public byte lfItalic, lfUnderline, lfStrikeOut, lfCharSet, lfOutPrecision,
                    lfClipPrecision, lfQuality, lfPitchAndFamily;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string lfFaceName;
    }

    public delegate bool EnumProc(IntPtr hwnd, IntPtr param);

    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] static extern uint GetAwarenessFromDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder buffer, int count);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextLengthW(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder buffer, int count);
    [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern int GetObjectW(IntPtr obj, int size, out LOGFONT font);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateFontIndirectW(ref LOGFONT font);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern bool GetTextExtentPoint32W(IntPtr hdc, string text, int length, out SIZE size);

    const uint WM_GETFONT = 0x0031;

    /// -4 is DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2. Must run before any
    /// coordinate is read, and before this process owns a window.
    public static bool DeclarePerMonitorV2() { return SetProcessDpiAwarenessContext(new IntPtr(-4)); }

    /// 2 is DPI_AWARENESS_PER_MONITOR_AWARE. This, not the return value above,
    /// is the authority: a host that was already per-monitor aware refuses the
    /// call and is nonetheless correct to read from.
    public static uint Awareness() { return GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()); }

    public static string Text(IntPtr hwnd) {
        var buffer = new StringBuilder(GetWindowTextLengthW(hwnd) + 2);
        GetWindowTextW(hwnd, buffer, buffer.Capacity);
        return buffer.ToString();
    }

    public static string ClassName(IntPtr hwnd) {
        var buffer = new StringBuilder(256);
        GetClassNameW(hwnd, buffer, buffer.Capacity);
        return buffer.ToString();
    }

    public static List<IntPtr> TopLevelWindows() {
        var found = new List<IntPtr>();
        EnumWindows((hwnd, param) => { found.Add(hwnd); return true; }, IntPtr.Zero);
        return found;
    }

    public static List<IntPtr> ChildWindows(IntPtr parent) {
        var found = new List<IntPtr>();
        EnumChildWindows(parent, (hwnd, param) => { found.Add(hwnd); return true; }, IntPtr.Zero);
        return found;
    }

    public static RECT WindowRect(IntPtr hwnd) { RECT r; GetWindowRect(hwnd, out r); return r; }
    public static RECT ClientRect(IntPtr hwnd) { RECT r; GetClientRect(hwnd, out r); return r; }

    public static LOGFONT FontOf(IntPtr hwnd) {
        IntPtr handle = SendMessageW(hwnd, WM_GETFONT, IntPtr.Zero, IntPtr.Zero);
        LOGFONT font;
        GetObjectW(handle, Marshal.SizeOf(typeof(LOGFONT)), out font);
        return font;
    }

    /// How the control's text wraps in the control's own font, returned as
    /// {line count, cell height in physical pixels}.
    ///
    /// The font is re-created from the LOGFONT rather than selected by handle,
    /// and that is not tidiness: a GDI font handle belonging to ANOTHER process
    /// is refused by SelectObject, which then leaves the stock 8 px System font
    /// selected and reports a plausible smaller number instead of failing.
    /// Measured 2026-08-17: "Check this computer" came back 163 px that way
    /// against a true 273 px, which reads as text that comfortably fits.
    public static int[] WrapMetrics(string text, LOGFONT font, int widthPixels) {
        IntPtr created = CreateFontIndirectW(ref font);
        IntPtr hdc = GetDC(IntPtr.Zero);
        IntPtr previous = SelectObject(hdc, created);
        int lines = 0, cell = 0;
        foreach (string paragraph in text.Replace("\r\n", "\n").Split('\n')) {
            string line = "";
            int wrapped = 1;
            foreach (string word in paragraph.Split(' ')) {
                string candidate = line.Length == 0 ? word : line + " " + word;
                SIZE size;
                GetTextExtentPoint32W(hdc, candidate, candidate.Length, out size);
                if (size.cy > cell) { cell = size.cy; }
                if (size.cx > widthPixels && line.Length > 0) { wrapped++; line = word; }
                else { line = candidate; }
            }
            lines += wrapped;
        }
        SelectObject(hdc, previous);
        DeleteObject(created);
        ReleaseDC(IntPtr.Zero, hdc);
        return new int[] { lines, cell };
    }
}
}
'@
}

[void][SpeakEasy.NativeWindowProbe]::DeclarePerMonitorV2()
$awareness = [SpeakEasy.NativeWindowProbe]::Awareness()
if ($awareness -ne 2) {
    throw ("This process is DPI awareness $awareness, not 2 (per-monitor). Every " +
        "coordinate it reads would be virtualized, and a correctly scaled window " +
        "would be indistinguishable from one that never scaled. Refusing to report.")
}

$systemDpi = [SpeakEasy.NativeWindowProbe]::GetDpiForSystem()
Write-Output ("probe: per-monitor aware. system dpi {0}, primary screen {1}x{2} physical." -f `
        $systemDpi, [SpeakEasy.NativeWindowProbe]::GetSystemMetrics(0), [SpeakEasy.NativeWindowProbe]::GetSystemMetrics(1))

$matched = 0
foreach ($hwnd in [SpeakEasy.NativeWindowProbe]::TopLevelWindows()) {
    $title = [SpeakEasy.NativeWindowProbe]::Text($hwnd)
    if (-not $title.Contains($TitleContains)) { continue }
    if (-not [SpeakEasy.NativeWindowProbe]::IsWindowVisible($hwnd)) { continue }
    $matched++

    $dpi = [SpeakEasy.NativeWindowProbe]::GetDpiForWindow($hwnd)
    $window = [SpeakEasy.NativeWindowProbe]::WindowRect($hwnd)
    $client = [SpeakEasy.NativeWindowProbe]::ClientRect($hwnd)

    Write-Output ''
    Write-Output ("window '{0}'" -f $title)
    Write-Output ("  dpi           {0} ({1}% scale){2}" -f $dpi, [math]::Round($dpi / 96.0 * 100), `
        $(if ($dpi -ne $systemDpi) { ' — differs from the system dpi' } else { '' }))
    Write-Output ("  window rect   {0}x{1} physical at ({2},{3})" -f `
        ($window.Right - $window.Left), ($window.Bottom - $window.Top), $window.Left, $window.Top)
    Write-Output ("  client rect   {0}x{1} physical = {2}x{3} logical at this dpi" -f `
        ($client.Right - $client.Left), ($client.Bottom - $client.Top), `
        [math]::Round(($client.Right - $client.Left) * 96.0 / $dpi, 1), `
        [math]::Round(($client.Bottom - $client.Top) * 96.0 / $dpi, 1))

    foreach ($child in [SpeakEasy.NativeWindowProbe]::ChildWindows($hwnd)) {
        $rect = [SpeakEasy.NativeWindowProbe]::WindowRect($child)
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top
        $text = [SpeakEasy.NativeWindowProbe]::Text($child)
        $oneLine = ($text -replace '\r?\n', ' | ')
        $shown = if ($oneLine.Length -gt 40) { $oneLine.Substring(0, 40) + '…' } else { $oneLine }
        Write-Output ("    {0,-18} {1,5}x{2,-5} at +{3},+{4}  '{5}'" -f `
            [SpeakEasy.NativeWindowProbe]::ClassName($child), $width, $height, `
            ($rect.Left - $window.Left), ($rect.Top - $window.Top), $shown)

        if (-not $Fit -or $text.Trim().Length -eq 0) { continue }
        $font = [SpeakEasy.NativeWindowProbe]::FontOf($child)
        $metrics = [SpeakEasy.NativeWindowProbe]::WrapMetrics($text, $font, $width)
        $needed = $metrics[0] * $metrics[1]
        Write-Output ("      font {0} lfHeight={1}, cell {2}px; wraps to {3} line(s) = {4}px in {5}px -> {6}" -f `
                $font.lfFaceName, $font.lfHeight, $metrics[1], $metrics[0], $needed, $height, `
            $(if ($needed -le $height) { 'fits' } else { 'CLIPPED' }))
    }
}

if ($matched -eq 0) {
    throw "No visible window whose title contains '$TitleContains'. Is it running?"
}
