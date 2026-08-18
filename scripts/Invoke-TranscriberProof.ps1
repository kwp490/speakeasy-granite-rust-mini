<#
.SYNOPSIS
Drives the running SpeakEasy app the way a person would, and reports what it did.

.DESCRIPTION
docs/archive/UI-REDESIGN.md §19 is blunt that a successful compile is not verification, and
the streaming path has already produced bugs that were invisible without the
real model. This is the harness that made the difference during the redesign:
it clicks the transcriber's real buttons, speaks real audio at the real
microphone, and reads the sanitized diagnostic log to see what actually
happened.

Three bugs were found with it that every green test suite had missed:

  * a dictation that hit the duration ceiling was retained and never transcribed
  * the level meter had never been wired to anything
  * `core:default` grants neither allow-start-dragging, allow-minimize nor
    allow-close, so those buttons were inert with no error of any kind

Speech is synthesized through the speakers and picked up by the microphone.
That is a genuine acoustic path, so it exercises the mic -> capture link rather
than injecting samples. It needs the output volume up and the microphone
unmuted; if the transcript comes back empty, check those first.

.PARAMETER Action
  screenshot  save a PNG of the transcriber so its layout can be looked at
  click       click one control, by physical offset from the window's edges
  dictate     drive a full dictation and report the log and delivered text

.PARAMETER Driver
For -Action dictate: `hotkey` uses Ctrl+Alt+L, `buttons` clicks the
transcriber. Running both is how delivery parity (§16) is checked.

.EXAMPLE
./scripts/Invoke-TranscriberProof.ps1 -Action screenshot -Out hud.png

.EXAMPLE
./scripts/Invoke-TranscriberProof.ps1 -Action dictate -Driver buttons

.NOTES
Button offsets are measured from a screenshot and are in PHYSICAL pixels from
the window's right and top edges. Take a screenshot first and re-measure after
any layout change — an offset that has drifted onto the neighbouring control
looks exactly like a broken button. That mistake cost an hour during the
redesign: the harness was clicking Cancel and it was read as "Stop is broken".
#>
[CmdletBinding()]
param(
    [ValidateSet('screenshot', 'click', 'dictate')]
    [string]$Action = 'screenshot',

    [string]$Out = "$env:TEMP\speakeasy-transcriber.png",

    [int]$FromRight = 109,
    [int]$FromTop = 125,

    [ValidateSet('hotkey', 'buttons')]
    [string]$Driver = 'hotkey',

    [string]$Phrase = 'Ever tried. Ever failed. No matter. Try again. Fail again. Fail better.',

    [int]$FinalPassSeconds = 25
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Speech
Add-Type -AssemblyName System.Windows.Forms

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public struct SpeakEasyRect { public int Left, Top, Right, Bottom; }
public struct SpeakEasyPoint { public int X, Y; }

public static class SpeakEasyWin32 {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out SpeakEasyRect r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out SpeakEasyRect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref SpeakEasyPoint p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  // Without this the window rect comes back virtualized on a scaled display and
  // every click lands somewhere else entirely.
  // System-DPI awareness is not enough on a multi-monitor host with mixed
  // scaling: coordinates come back virtualized and a click lands hundreds of
  // pixels away. Verified on this host — a taskbar button at y=2088 was clicked
  // at y=2159 and the restore silently did nothing. Per-monitor-v2 first, with
  // the older call as a fallback for anything that refuses it.
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public delegate bool EnumProc(IntPtr h, IntPtr p);

  public const uint LEFTDOWN = 0x0002, LEFTUP = 0x0004, KEYUP = 0x0002;
  public const byte VK_CONTROL = 0x11, VK_MENU = 0x12, VK_L = 0x4C;
  public const int GWL_EXSTYLE = -20;
  public const int WS_EX_NOACTIVATE = 0x08000000;

  public static System.Collections.Generic.List<IntPtr> Visible() {
    var found = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, p) => { if (IsWindowVisible(h)) found.Add(h); return true; }, IntPtr.Zero);
    return found;
  }
  public static string Text(IntPtr h) { var b = new StringBuilder(512); GetWindowTextW(h, b, 512); return b.ToString(); }
  public static string Cls(IntPtr h) { var b = new StringBuilder(512); GetClassNameW(h, b, 512); return b.ToString(); }
}
'@ -ErrorAction SilentlyContinue

# Per-monitor-v2 DPI awareness (-4). System awareness is not enough on a
# multi-monitor host with mixed scaling.
if (-not [SpeakEasyWin32]::SetProcessDpiAwarenessContext([IntPtr](-4))) {
    [void][SpeakEasyWin32]::SetProcessDPIAware()
}


$diagnosticLog = Join-Path $env:APPDATA 'ai.speakeasy.desktop\logs\speakeasy.log'

function Get-TranscriberWindow {
    foreach ($handle in [SpeakEasyWin32]::Visible()) {
        if ([SpeakEasyWin32]::Cls($handle) -ne 'Tauri Window') { continue }
        if ([SpeakEasyWin32]::Text($handle) -like '*transcriber*') { return $handle }
    }
    throw 'The transcriber window was not found. Is SpeakEasy running?'
}

function Get-TranscriberRect([IntPtr]$Handle) {
    $rect = New-Object SpeakEasyRect
    [void][SpeakEasyWin32]::GetWindowRect($Handle, [ref]$rect)
    return $rect
}

function Get-LogLineCount {
    if (Test-Path $diagnosticLog) { return (Get-Content $diagnosticLog).Count }
    return 0
}

function Write-NewLogLines([int]$Since) {
    Write-Host ''
    Write-Host '--- diagnostic log ---'
    if (-not (Test-Path $diagnosticLog)) {
        Write-Host '  (no log; disk logging may be off)'
        return
    }
    $lines = Get-Content $diagnosticLog | Select-Object -Skip $Since
    if ($lines.Count -eq 0) {
        Write-Host '  (nothing was recorded, which is itself a finding)'
        return
    }
    $lines | ForEach-Object { Write-Host "  $_" }
}

function Invoke-TranscriberClick([int]$Right, [int]$Top, [string]$Label) {
    $handle = Get-TranscriberWindow
    $rect = Get-TranscriberRect $handle
    $x = $rect.Right - $Right
    $y = $rect.Top + $Top
    $foregroundBefore = [SpeakEasyWin32]::GetForegroundWindow()
    [void][SpeakEasyWin32]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 200
    [SpeakEasyWin32]::mouse_event([SpeakEasyWin32]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [SpeakEasyWin32]::mouse_event([SpeakEasyWin32]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    $foregroundAfter = [SpeakEasyWin32]::GetForegroundWindow()
    $held = $foregroundBefore -eq $foregroundAfter
    Write-Host "  $Label at ($x,$y); foreground held: $held"
    if (-not $held) {
        # Decision 2 is about the *session* controls: clicking Start must not pull
        # focus off the app the text is being delivered into. Three controls move
        # the foreground legitimately and will always report false here — the gear
        # focuses settings on purpose, and minimize and close remove the window
        # the foreground was on. Do not read those as violations.
        Write-Warning ('Clicking the transcriber changed the foreground window. That is a ' +
            'decision-2 violation for Start/Stop/Cancel, and expected for the gear, minimize and close.')
    }
}

<#
Clicks a control located by CSS selector, with a real mouse event.

Fixed offsets from the window edge are the harness's worst failure mode, and it
has now bitten twice. First it clicked Cancel instead of Stop, which read for an
hour as "the Stop button does not work". Then the primary button's *width* turned
out to depend on its state — "Done" is 57 CSS px where "Start dictation" is 117 —
so an offset measured in one state silently misses in another, and a run that
touched nothing at all looks exactly like a product that delivered nothing.

The element is found through `Invoke-WebviewProbe.ps1` and its centre converted
from CSS pixels to screen pixels through the client-area origin, so the click is
still a genuine mouse event against the real window: the no-activate focus
behaviour that decision 2 rests on is still being exercised, unlike a synthetic
`element.click()`.

Needs the app started with the WebView2 debugging port open. Falls back to the
offset path when it is not available, and says which one it used.
#>
function Invoke-TranscriberSelectorClick {
    param([string]$Selector, [string]$Label)

    $probe = Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1'
    $expression = @"
JSON.stringify((() => {
  const element = document.querySelector('$Selector');
  if (element === null) return null;
  const rect = element.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return null;
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, label: element.textContent };
})())
"@
    $found = $null
    try {
        $found = (& $probe -Window transcriber -Expression $expression | ConvertFrom-Json) | ConvertFrom-Json
    } catch {
        Write-Warning "No DevTools endpoint; falling back to fixed offsets for '$Label'."
        return $false
    }
    if ($null -eq $found) {
        throw "The selector '$Selector' matched no visible element. The click would have hit nothing."
    }

    $handle = Get-TranscriberWindow
    $client = New-Object SpeakEasyRect
    [void][SpeakEasyWin32]::GetClientRect($handle, [ref]$client)
    $origin = New-Object SpeakEasyPoint
    [void][SpeakEasyWin32]::ClientToScreen($handle, [ref]$origin)
    $scale = [SpeakEasyWin32]::GetDpiForWindow($handle) / 96.0

    $x = [int]($origin.X + $found.x * $scale)
    $y = [int]($origin.Y + $found.y * $scale)

    $foregroundBefore = [SpeakEasyWin32]::GetForegroundWindow()
    [void][SpeakEasyWin32]::SetCursorPos($x, $y)
    Start-Sleep -Milliseconds 200
    [SpeakEasyWin32]::mouse_event([SpeakEasyWin32]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [SpeakEasyWin32]::mouse_event([SpeakEasyWin32]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    $held = $foregroundBefore -eq [SpeakEasyWin32]::GetForegroundWindow()
    Write-Host "  $Label ('$($found.label)') at ($x,$y); foreground held: $held"
    if (-not $held) {
        Write-Warning 'Clicking a session control changed the foreground window. Decision 2 forbids it.'
    }
    return $true
}

function Send-ActivationHotkey {
    [SpeakEasyWin32]::keybd_event([SpeakEasyWin32]::VK_CONTROL, 0, 0, [UIntPtr]::Zero)
    [SpeakEasyWin32]::keybd_event([SpeakEasyWin32]::VK_MENU, 0, 0, [UIntPtr]::Zero)
    [SpeakEasyWin32]::keybd_event([SpeakEasyWin32]::VK_L, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [SpeakEasyWin32]::keybd_event([SpeakEasyWin32]::VK_L, 0, [SpeakEasyWin32]::KEYUP, [UIntPtr]::Zero)
    [SpeakEasyWin32]::keybd_event([SpeakEasyWin32]::VK_MENU, 0, [SpeakEasyWin32]::KEYUP, [UIntPtr]::Zero)
    [SpeakEasyWin32]::keybd_event([SpeakEasyWin32]::VK_CONTROL, 0, [SpeakEasyWin32]::KEYUP, [UIntPtr]::Zero)
}

# Stops only the notepad processes that appeared after `$Existing` was sampled.
# Killing every notepad would close windows the user opened, and killing the one
# id `Start-Process -PassThru` returns misses the sibling that packaged notepad
# spawns alongside it — which is how a proof run left a notepad behind.
function Stop-AddedNotepads([int[]]$Existing) {
    Get-Process notepad -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -notin $Existing } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
}

switch ($Action) {
    'screenshot' {
        $handle = Get-TranscriberWindow
        $rect = Get-TranscriberRect $handle
        $exStyle = [SpeakEasyWin32]::GetWindowLongW($handle, [SpeakEasyWin32]::GWL_EXSTYLE)
        $noActivate = ($exStyle -band [SpeakEasyWin32]::WS_EX_NOACTIVATE) -ne 0
        $dpi = [SpeakEasyWin32]::GetDpiForWindow($handle)
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top
        Write-Host "transcriber at ($($rect.Left),$($rect.Top)) ${width}x${height} physical"
        Write-Host "  scale $([math]::Round($dpi / 96, 2))x  WS_EX_NOACTIVATE=$noActivate"

        $pad = 8
        $bitmap = New-Object System.Drawing.Bitmap(($width + $pad * 2), ($height + $pad * 2))
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen(
            ($rect.Left - $pad), ($rect.Top - $pad), 0, 0,
            (New-Object System.Drawing.Size($bitmap.Width, $bitmap.Height)))
        $graphics.Dispose()
        $bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
        $bitmap.Dispose()
        Write-Host "saved $Out"
    }

    'click' {
        Invoke-TranscriberClick $FromRight $FromTop 'clicked'
    }

    'dictate' {
        $before = Get-LogLineCount

        Write-Host "driver: $Driver"

        $voice = New-Object System.Speech.Synthesis.SpeechSynthesizer
        $voice.Volume = 100

        # The record button reports its own readiness, so wait for it instead of
        # assuming it. This replaces a step that dismissed the old Done button:
        # a finished dictation now returns the button straight to Start recording,
        # so there is nothing left to dismiss. It also covers the state that step
        # never handled — at launch the button reads Loading model while the
        # engine loads, and a start issued there blocks on the load's own mutex,
        # which is indistinguishable from a dictation that simply did nothing.
        $label = ''
        foreach ($attempt in 1..40) {
            $label = & (Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1') -Window transcriber `
                -Expression 'document.querySelector("[data-testid=\"hud-primary\"]").textContent'
            if ($label -match 'Start recording|Try again') { break }
            Write-Host "waiting for the transcriber to be ready (button reads '$label')"
            Start-Sleep -Milliseconds 500
        }
        if ($label -notmatch 'Start recording|Try again') {
            throw "the transcriber never became ready to record; the button reads '$label'"
        }

        # A real delivery target, so the paste has somewhere to land and the
        # result can be read back rather than assumed. Opened only now, once the
        # engine is ready: the readiness wait above can bail out, and there is no
        # reason to leave a stray notepad behind when it does.
        #
        # The foreground window is *verified* to belong to notepad rather than
        # assumed after a sleep. A run on 2026-08-11 took the foreground two
        # seconds after launching notepad and got SpeakEasy's own settings window;
        # the dictation then reported `target_inspect_refused reason=ElementChanged`
        # against a target nobody had chosen, which reads like a delivery bug and
        # is not one.
        #
        # Notepad on Windows 11 is packaged, so `Start-Process -PassThru` does not
        # reliably hand back the process that owns the window — that run produced
        # two notepad processes and the returned id was not the one with the
        # window, which is why one survived the old cleanup. Cleanup now stops the
        # notepad processes this run added, and only those, so a notepad the user
        # already had open is left alone.
        # A dedicated scratch FILE, never a bare `notepad`.
        #
        # This matters more than it looks. Windows 11 Notepad restores its previous
        # tabs, so `Start-Process notepad` does not open an empty document -- it
        # surfaces whatever the user last had open. On 2026-08-11 that was an
        # unsaved personal note, the run accepted it as "a notepad window", and the
        # dictation was pasted into the middle of the user's own writing. Verifying
        # the *process* was not enough; the window has to be provably the scratch
        # document this script created.
        #
        # Opening a named file also gives Notepad a path, so the tab closes at the
        # end without a save prompt.
        $scratch = Join-Path $env:TEMP "speakeasy-proof-target-$PID.txt"
        Set-Content -LiteralPath $scratch -Value 'SpeakEasy proof scratch target. Safe to delete.' -Encoding utf8
        $scratchLeaf = Split-Path -Leaf $scratch
        $existingNotepads = @(
            Get-Process notepad -ErrorAction SilentlyContinue |
                Select-Object -ExpandProperty Id
        )
        $null = Start-Process notepad -ArgumentList $scratch
        $target = [IntPtr]::Zero
        foreach ($attempt in 1..40) {
            Start-Sleep -Milliseconds 250
            $candidate = [SpeakEasyWin32]::GetForegroundWindow()
            $ownerPid = 0
            [void][SpeakEasyWin32]::GetWindowThreadProcessId($candidate, [ref]$ownerPid)
            $ownerName = (Get-Process -Id $ownerPid -ErrorAction SilentlyContinue).ProcessName
            # The title has to name this script's own file. A notepad-owned window
            # showing some other document is exactly the case that went wrong.
            if ($ownerName -eq 'notepad' -and
                [SpeakEasyWin32]::Text($candidate) -like "*$scratchLeaf*") {
                $target = $candidate
                break
            }
        }
        if ($target -eq [IntPtr]::Zero) {
            $foreground = [SpeakEasyWin32]::GetForegroundWindow()
            Stop-AddedNotepads $existingNotepads
            Remove-Item -LiteralPath $scratch -Force -ErrorAction SilentlyContinue
            throw ("the scratch document '$scratchLeaf' never took the foreground; it is " +
                "still '$([SpeakEasyWin32]::Text($foreground))'. Refusing to dictate: a " +
                'delivery aimed at an unknown window cannot be attributed, and if that ' +
                "window belongs to someone's real document it gets written into.")
        }
        Write-Host "delivery target: '$([SpeakEasyWin32]::Text($target))'"

        Write-Host ''
        Write-Host 'start'
        # One button relabelled by state: Start recording, then Recording — so the
        # same selector still drives both halves and cannot land on Cancel. The
        # fallback offsets are its centre in the header, inside every label's box.
        if ($Driver -eq 'hotkey') { Send-ActivationHotkey }
        elseif (-not (Invoke-TranscriberSelectorClick '[data-testid="hud-primary"]' $label)) {
            Invoke-TranscriberClick 60 31 $label
        }

        Start-Sleep -Milliseconds 900
        $voice.Speak($Phrase)
        Start-Sleep -Milliseconds 900

        Write-Host 'stop'
        if ($Driver -eq 'hotkey') { Send-ActivationHotkey }
        elseif (-not (Invoke-TranscriberSelectorClick '[data-testid="hud-primary"]' 'Recording')) {
            Invoke-TranscriberClick 60 31 'Recording'
        }

        Write-Host "waiting ${FinalPassSeconds}s for the authoritative final pass..."
        Start-Sleep -Seconds $FinalPassSeconds

        Write-NewLogLines $before

        # Read back what was actually delivered.
        #
        # Two SendWait calls, not one `'^a^c'`: .NET rejects that string outright
        # with "SendKeys string '^a^c' is not valid", and the throw landed between
        # the sentinel write and the read — so the proof reported no delivered text
        # at all and leaked its notepad, on the one run that mattered. `'^a'` and
        # `'^c'` each pass. Verified on this host after the fix.
        #
        # The transcript is also read from the app's own result view below, because
        # the sentinel write necessarily destroys whatever delivery put on the
        # clipboard — and `clipboard_fallback_committed` is a *successful* outcome
        # whose only evidence was that clipboard. Reading the window as well means a
        # fallback delivery can still be checked against what was spoken.
        Write-Host ''
        Write-Host '--- text in the result view (survives the sentinel below) ---'
        $shown = & (Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1') -Window transcriber `
            -Expression 'document.querySelector("[data-testid=\"hud-transcript\"]").textContent'
        Write-Host "  $shown"
        $outcome = & (Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1') -Window transcriber `
            -Expression 'document.querySelector("[data-testid=\"hud-status\"]").textContent'
        Write-Host "  outcome: $outcome"

        # A sentinel first, so a stale clipboard cannot be mistaken for a
        # successful paste.
        [System.Windows.Forms.Clipboard]::SetText('SPEAKEASY-PROOF-NOTHING-DELIVERED')
        Start-Sleep -Milliseconds 200
        [System.Windows.Forms.SendKeys]::SendWait('^a')
        [System.Windows.Forms.SendKeys]::SendWait('^c')
        Start-Sleep -Milliseconds 500
        Write-Host ''
        Write-Host '--- text delivered into the target ---'
        try { Write-Host "  $([System.Windows.Forms.Clipboard]::GetText())" }
        catch { Write-Host '  (clipboard could not be read)' }

        # Save and close the scratch tab before anything else, so a Notepad that was
        # already running is handed back exactly as it was found -- one fewer tab,
        # nothing modified. Ctrl+S first because the tab has a real path and would
        # otherwise prompt.
        [System.Windows.Forms.SendKeys]::SendWait('^s')
        Start-Sleep -Milliseconds 400
        [System.Windows.Forms.SendKeys]::SendWait('^w')
        Start-Sleep -Milliseconds 400
        Stop-AddedNotepads $existingNotepads
        Remove-Item -LiteralPath $scratch -Force -ErrorAction SilentlyContinue
    }
}
