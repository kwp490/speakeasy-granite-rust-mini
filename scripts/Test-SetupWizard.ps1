#Requires -Version 7
<#
.SYNOPSIS
    Drives the real setup wizard through every step and asserts what it did.

.DESCRIPTION
    `Test-InstallerLifecycle.ps1` proves the install *contract* -- refusals, exit
    codes, uninstall -- by driving `--install`, which is a code path no user
    takes. This drives the one they do: `SpeakEasyMiniSetup.exe` with no
    arguments, through the eight-page wizard, to a launched app.

    Everything between those two is only reachable this way: the hardware page,
    the provider choice, the shortcut check, the vocabulary box, the retention
    question, the seed files those answers produce, the engine check, and the
    launch. None of it was covered by anything before 2026-08-19.

    **It asserts the page it is on before every click.** A driver that presses
    Next eight times passes on a wizard stuck on page one, and this project's
    recurring failure is exactly that: an instrument that cannot fail is
    indistinguishable from one that passes.

    This installs SpeakEasy Mini for real, into `%LOCALAPPDATA%\SpeakEasy Mini`,
    and leaves it installed and running. Pass `-Uninstall` to take it back off
    afterwards.

.PARAMETER ArtifactRoot
    Where `Build-LocalInstaller.ps1` put `SpeakEasyMiniSetup.exe`.

.PARAMETER Uninstall
    Silently uninstall at the end, keeping user data, so the machine is left as
    it was found.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArtifactRoot,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
# Resolved against the repository root, not the current directory. `Set-Location`
# does not move the *process* working directory that `[IO.Path]::GetFullPath`
# reads, so a relative artifact root resolved against wherever PowerShell was
# started -- which on the first run here was `apps\desktop`, and produced a
# missing-file error naming a path nobody had typed.
$artifactRoot = if ([IO.Path]::IsPathRooted($ArtifactRoot)) {
    $ArtifactRoot
} else {
    Join-Path $repositoryRoot $ArtifactRoot
}
$setup = Join-Path ([IO.Path]::GetFullPath($artifactRoot)) 'SpeakEasyMiniSetup.exe'
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) {
    throw "The single-file installer was not found: $setup"
}

# The Win32 surface this needs, and no more. `BM_CLICK` rather than moving a
# real cursor: a synthetic click on a button by handle is what the button's own
# handler receives, and it does not depend on the window being unobscured, on
# the screen resolution, or on nothing else stealing focus mid-run.
if (-not ('SpeakEasy.Win32' -as [type])) {
    Add-Type -Namespace SpeakEasy -Name Win32 -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
[DllImport("user32.dll")]
public static extern bool EnumChildWindows(IntPtr hWnd, EnumWindowsProc lpEnumFunc, IntPtr lParam);
public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern int GetWindowTextW(IntPtr hWnd, System.Text.StringBuilder lpString, int nMaxCount);
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern int GetClassNameW(IntPtr hWnd, System.Text.StringBuilder lpClassName, int nMaxCount);
[DllImport("user32.dll")]
public static extern bool IsWindowVisible(IntPtr hWnd);
[DllImport("user32.dll")]
public static extern bool IsWindowEnabled(IntPtr hWnd);
[DllImport("user32.dll")]
public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr SendMessageW(IntPtr hWnd, uint Msg, IntPtr wParam, string lParam);
[DllImport("user32.dll")]
public static extern IntPtr SendMessageW(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
public struct RECT { public int Left, Top, Right, Bottom; }
'@
}

$BM_CLICK = 0x00F5
$WM_SETTEXT = 0x000C

function Get-WizardWindow {
    param([int]$TimeoutSeconds = 30)
    # Through the process list rather than `FindWindow`, and that is a fix
    # rather than a preference: PowerShell marshals `$null` for a `string`
    # parameter as an empty string, so `FindWindow($null, $title)` searches for
    # a window of class "" and finds nothing. It reported "the wizard never
    # appeared" with the wizard on screen -- a broken instrument reading exactly
    # like the failure it was written to detect.
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $process = @(Get-Process -Name 'SpeakEasyMiniSetup', 'speakeasy-bootstrapper' -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowTitle -eq 'SpeakEasy Mini setup' }) | Select-Object -First 1
        if ($process) { return $process.MainWindowHandle }
        Start-Sleep -Milliseconds 200
    }
    throw 'The setup wizard never appeared. Setup re-launches itself detached, so look for a second process.'
}

function Get-Controls {
    param([Parameter(Mandatory)][IntPtr]$Window)
    $found = [Collections.Generic.List[object]]::new()
    $callback = [SpeakEasy.Win32+EnumWindowsProc] {
        param([IntPtr]$child, [IntPtr]$unused)
        $class = [Text.StringBuilder]::new(64)
        [void][SpeakEasy.Win32]::GetClassNameW($child, $class, $class.Capacity)
        $text = [Text.StringBuilder]::new(4096)
        [void][SpeakEasy.Win32]::GetWindowTextW($child, $text, $text.Capacity)
        $rect = New-Object SpeakEasy.Win32+RECT
        [void][SpeakEasy.Win32]::GetWindowRect($child, [ref]$rect)
        $found.Add([pscustomobject]@{
            Handle  = $child
            Class   = $class.ToString()
            Text    = $text.ToString()
            Top     = $rect.Top
            Left    = $rect.Left
            Visible = [SpeakEasy.Win32]::IsWindowVisible($child)
            Enabled = [SpeakEasy.Win32]::IsWindowEnabled($child)
        })
        return $true
    }
    [void][SpeakEasy.Win32]::EnumChildWindows($Window, $callback, [IntPtr]::Zero)
    # Top-to-bottom, then left-to-right. Enumeration order is z-order, which is
    # neither creation order nor reading order; sorting by position is what lets
    # "the heading" mean the topmost label rather than a guess at an index.
    #
    # Emitted to the pipeline one control at a time. Returning `,@(...)` -- the
    # usual PowerShell guard against a one-element array unrolling to a scalar
    # -- hands the *whole list* to the next `Where-Object` as a single object,
    # and `$_.Class -eq 'Static'` against an array filters rather than compares,
    # so it comes back non-empty and truthy. The caller then read the entire
    # window's text as one heading. Every caller here wraps in `@()` already.
    $found | Sort-Object Top, Left
}

function Get-Heading {
    param([Parameter(Mandatory)][IntPtr]$Window)
    $labels = @(Get-Controls -Window $Window | Where-Object { $_.Class -eq 'Static' -and $_.Visible })
    if ($labels.Count -eq 0) { return '' }
    $labels[0].Text
}

function Get-StepPosition {
    param([Parameter(Mandatory)][IntPtr]$Window)
    $labels = @(Get-Controls -Window $Window | Where-Object { $_.Class -eq 'Static' -and $_.Visible })
    if ($labels.Count -lt 2) { return '' }
    $labels[1].Text
}

function Get-Button {
    param([Parameter(Mandatory)][IntPtr]$Window, [Parameter(Mandatory)][string]$Text)
    $button = @(Get-Controls -Window $Window |
        Where-Object { $_.Class -eq 'Button' -and $_.Visible -and $_.Text -eq $Text }) |
        Select-Object -First 1
    if (-not $button) { throw "No visible button labelled '$Text' on the current page." }
    $button
}

function Assert-Page {
    param(
        [Parameter(Mandatory)][IntPtr]$Window,
        [Parameter(Mandatory)][string]$Heading,
        [Parameter(Mandatory)][int]$Number
    )
    $actual = Get-Heading -Window $Window
    if ($actual -ne $Heading) {
        throw "Expected to be on '$Heading' and the wizard is showing '$actual'."
    }
    $position = Get-StepPosition -Window $Window
    if ($position -notmatch "^Step $Number of \d+$") {
        throw "Page '$Heading' reports '$position', not step $Number."
    }
    Write-Host "  page $Number : $Heading"
}

function Invoke-Click {
    param([Parameter(Mandatory)][IntPtr]$Handle)
    [void][SpeakEasy.Win32]::SendMessageW($Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    # The click is dispatched synchronously, but the handler it runs may repaint
    # and re-enable controls; give the message loop a beat before reading them.
    Start-Sleep -Milliseconds 400
}

function Invoke-Next {
    param([Parameter(Mandatory)][IntPtr]$Window, [int]$TimeoutSeconds = 30)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $next = Get-Button -Window $Window -Text 'Next'
        if ($next.Enabled) {
            Invoke-Click -Handle $next.Handle
            return
        }
        Start-Sleep -Milliseconds 250
    }
    throw "Next never became available; the page is gating progress and the gate did not open."
}

function Get-Status {
    param([Parameter(Mandatory)][IntPtr]$Window)
    # The lowest visible label. On a page that asks something that is the status
    # line; on a page that reports, it is the notice.
    $labels = @(Get-Controls -Window $Window | Where-Object { $_.Class -eq 'Static' -and $_.Visible })
    if ($labels.Count -eq 0) { return '' }
    $labels[-1].Text
}

$installRoot = Join-Path $env:LOCALAPPDATA 'SpeakEasy Mini'
$dataRoot = Join-Path $env:APPDATA 'ai.speakeasy.mini'
$configRoot = Join-Path $dataRoot 'config'

Push-Location $repositoryRoot
try {
    $running = @(Get-Process -Name 'ai-speakeasy-mini' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        throw 'SpeakEasy Mini is running; setup will refuse to install over it. Close it and rerun.'
    }
    $stamp = (Get-ItemProperty -Path 'HKCU:\Software\SpeakEasy Mini\LocalDevelopment' -Name Version -ErrorAction SilentlyContinue).Version
    if ($stamp) {
        throw "SpeakEasy Mini $stamp is already installed, so setup will refuse as a same-version reinstall. Uninstall it and rerun."
    }
    # Cleared so that "the seed exists afterwards" means this run wrote it.
    foreach ($seed in 'install-hotkey.txt', 'install-logging.txt', 'install-retention.txt', 'install-vocabulary.txt', 'install-provider.txt') {
        Remove-Item -LiteralPath (Join-Path $configRoot $seed) -Force -ErrorAction SilentlyContinue
    }

    Write-Host "starting $setup"
    # No `-Wait`: setup re-launches itself detached to shed its console and the
    # first process exits in milliseconds. The window belongs to the second one.
    Start-Process -FilePath $setup | Out-Null
    $window = Get-WizardWindow

    Assert-Page -Window $window -Heading 'Check this computer' -Number 1
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Choose how it runs' -Number 2
    $providerStatus = Get-Status -Window $window
    if ($providerStatus -notmatch 'processor') {
        throw "The provider page said nothing about the processor configuration: $providerStatus"
    }
    Write-Host "  provider: $($providerStatus.Split([Environment]::NewLine)[0])"
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Download what is needed' -Number 3
    # Generous, because this is the step that fetches ~2.2 GB on a machine that
    # does not already have the weights. Read before the click, not after: the
    # first version reported the *install* page's notice under the heading
    # "download", because by then the wizard had already advanced.
    Invoke-Next -Window $window -TimeoutSeconds 3600
    Write-Host '  download: satisfied or transferred, and Next opened'

    Assert-Page -Window $window -Heading 'Install' -Number 4
    Invoke-Next -Window $window -TimeoutSeconds 300

    Assert-Page -Window $window -Heading 'Choose your shortcut' -Number 5
    $shortcutStatus = Get-Status -Window $window
    if ($shortcutStatus -notmatch 'Ctrl \+ Alt \+ P|Ctrl\+Alt\+P') {
        throw "The shortcut page did not report on the default combination: $shortcutStatus"
    }
    if ($shortcutStatus -notmatch 'is free') {
        throw "Ctrl+Alt+P is not free on this machine, so this run cannot prove the happy path: $shortcutStatus"
    }
    Write-Host "  shortcut: $($shortcutStatus.Split([Environment]::NewLine)[0])"
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Add your words' -Number 6
    $edit = @(Get-Controls -Window $window | Where-Object { $_.Class -eq 'Edit' -and $_.Visible }) | Select-Object -First 1
    if (-not $edit) { throw 'The vocabulary page has no text box.' }
    $terms = "Granite`r`nSpeakEasy"
    [void][SpeakEasy.Win32]::SendMessageW($edit.Handle, $WM_SETTEXT, [IntPtr]::Zero, $terms)
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Choose what is kept' -Number 7
    $boxes = @(Get-Controls -Window $window |
        Where-Object { $_.Class -eq 'Button' -and $_.Visible -and $_.Text -notin @('Back', 'Next', 'Cancel', 'Retry', 'Finish') })
    if ($boxes.Count -ne 2) {
        throw "Expected two check boxes on the retention page and found $($boxes.Count)."
    }
    Write-Host "  keeps: $($boxes.Text -join ' | ')"
    # Left at their defaults deliberately: retention off is the owner's decision
    # and the seed written below is what proves it travelled.
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Check that dictation works' -Number 8
    # A cold model load plus a transcription. Measured at about five seconds on
    # the machine this was written on; bounded well above that so a slow disk is
    # not read as a hang.
    $verdict = ''
    $deadline = [DateTime]::UtcNow.AddMinutes(6)
    while ([DateTime]::UtcNow -lt $deadline) {
        $verdict = Get-Status -Window $window
        if ($verdict -notmatch 'Loading the speech model') { break }
        Start-Sleep -Seconds 2
    }
    Write-Host "  engine: $($verdict.Split([Environment]::NewLine)[0])"
    if ($verdict -notmatch 'transcribed the recording correctly') {
        throw "The engine check did not pass: $verdict"
    }

    # Everything the pages claimed, checked against the disk rather than the
    # window. A wizard that painted eight correct pages and installed nothing
    # would have passed every assertion above.
    foreach ($relative in 'ai-speakeasy-mini.exe', 'speakeasy-bootstrapper.exe', 'proof\granite-worker.exe') {
        if (-not (Test-Path -LiteralPath (Join-Path $installRoot $relative) -PathType Leaf)) {
            throw "Setup reported success without placing $relative."
        }
    }
    $expectedSeeds = @{
        'install-hotkey.txt'     = 'Ctrl+Alt+P'
        'install-logging.txt'    = '1'
        'install-retention.txt'  = '0'
        'install-provider.txt'   = 'cpu'
        'install-vocabulary.txt' = "Granite`r`nSpeakEasy"
    }
    foreach ($seed in $expectedSeeds.Keys) {
        $path = Join-Path $configRoot $seed
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Setup did not record $seed, so that answer was collected and discarded."
        }
        $actual = (Get-Content -LiteralPath $path -Raw).TrimEnd("`r", "`n")
        if ($actual -ne $expectedSeeds[$seed]) {
            throw "$seed holds '$actual', not '$($expectedSeeds[$seed])'."
        }
    }
    Write-Host '  seeds: all five recorded with the answers given'

    $finish = Get-Button -Window $window -Text 'Finish'
    Invoke-Click -Handle $finish.Handle
    $appDeadline = [DateTime]::UtcNow.AddSeconds(60)
    $app = $null
    while ([DateTime]::UtcNow -lt $appDeadline) {
        $app = @(Get-Process -Name 'ai-speakeasy-mini' -ErrorAction SilentlyContinue) | Select-Object -First 1
        if ($app) { break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $app) { throw 'Finish did not start the app.' }
    Write-Host "  launch: ai-speakeasy-mini started (pid $($app.Id))"

    # The app consumes four of the five seeds on its first start and leaves the
    # installed-configuration record alone. Asserting the deletion is what
    # proves the app read them rather than merely started.
    $consumeDeadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $consumeDeadline) {
        $remaining = @('install-hotkey.txt', 'install-logging.txt', 'install-retention.txt', 'install-vocabulary.txt' |
            Where-Object { Test-Path -LiteralPath (Join-Path $configRoot $_) })
        if ($remaining.Count -eq 0) { break }
        Start-Sleep -Seconds 1
    }
    if ($remaining.Count -gt 0) {
        throw "The app started but did not consume: $($remaining -join ', ')"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $configRoot 'install-provider.txt'))) {
        throw 'The installed-configuration record was consumed; it is meant to persist.'
    }
    $settings = Get-Content -LiteralPath (Join-Path $configRoot 'settings.json') -Raw | ConvertFrom-Json
    if ($settings.hotkey.activation_binding -ne 'Ctrl+Alt+P') {
        throw "The app's saved shortcut is '$($settings.hotkey.activation_binding)', not the one setup recorded."
    }
    if ($settings.privacy.persisted_history_enabled -ne $false) {
        throw 'The retention answer did not reach the profile.'
    }
    # Waited for, not assumed present. The vocabulary is applied while the app
    # is still building its coordinators, a little after the seeds are consumed,
    # so reading it the instant the last seed disappeared found no file at all
    # -- and reported the words as lost when they arrived a moment later.
    $personalizationPath = Join-Path $configRoot 'personalization.json'
    $personalizationDeadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $personalizationDeadline -and
        -not (Test-Path -LiteralPath $personalizationPath -PathType Leaf)) {
        Start-Sleep -Milliseconds 500
    }
    if (-not (Test-Path -LiteralPath $personalizationPath -PathType Leaf)) {
        throw 'The app consumed the vocabulary seed and wrote no dictionary.'
    }
    $personalization = Get-Content -LiteralPath $personalizationPath -Raw | ConvertFrom-Json
    $installed = @($personalization.dictionary | Where-Object { $_.id -like 'installer-*' })
    if ($installed.Count -ne 2) {
        throw "Expected the two words setup collected in the dictionary and found $($installed.Count)."
    }
    Write-Host "  profile: shortcut, retention and $($installed.Count) protected words all arrived"

    Write-Host 'SpeakEasy Mini setup wizard: passed'
}
finally {
    Pop-Location
    if ($Uninstall) {
        Get-Process -Name 'ai-speakeasy-mini' -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Seconds 2
        $uninstaller = Join-Path $installRoot 'speakeasy-bootstrapper.exe'
        if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
            & $uninstaller --uninstall /S | Out-Null
        }
    }
}
