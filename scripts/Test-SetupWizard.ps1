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

    Assert-Page -Window $window -Heading 'Can this computer run it?' -Number 1
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Where should it run?' -Number 2
    $providerStatus = Get-Status -Window $window
    if ($providerStatus -notmatch 'processor') {
        throw "The provider page said nothing about the processor configuration: $providerStatus"
    }
    # The option that cannot be installed is shown and **disabled**, which is
    # `UI-GUIDE`'s rule for this page and was not implemented until 2026-08-20.
    # Selecting it is what wrote `installed=cuda` onto an installation carrying no
    # CUDA worker: the app then correctly ran on the processor and the support log
    # read `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`.
    #
    # Asserted by *name*, because "the first radio button" is the kind of index
    # that keeps pointing at the right control while meaning the wrong one.
    $providerOptions = @(Get-Controls -Window $window |
        Where-Object { $_.Class -eq 'Button' -and $_.Visible -and $_.Text -like 'Use the *' })
    if ($providerOptions.Count -ne 2) {
        throw "Expected two provider options and found $($providerOptions.Count)."
    }
    $graphicsCard = @($providerOptions | Where-Object { $_.Text -eq 'Use the graphics card' })[0]
    if (-not $graphicsCard) { throw 'The provider page does not offer the graphics-card option at all.' }
    # Hidden would be wrong too: it reads as setup not having examined the card.
    if ($graphicsCard.Enabled) {
        throw ('The graphics-card option is selectable on a payload that carries no CUDA ' +
            'worker. Selecting it is what records a graphics-card installation that does not exist.')
    }
    Write-Host "  provider: graphics-card option shown and disabled; $($providerStatus.Split([Environment]::NewLine)[0])"
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Download the models' -Number 3
    # Generous, because this is the step that fetches ~2.2 GB on a machine that
    # does not already have the weights. Read before the click, not after: the
    # first version reported the *install* page's notice under the heading
    # "download", because by then the wizard had already advanced.
    Invoke-Next -Window $window -TimeoutSeconds 3600
    Write-Host '  download: satisfied or transferred, and Next opened'

    Assert-Page -Window $window -Heading 'Install' -Number 4
    Invoke-Next -Window $window -TimeoutSeconds 300

    Assert-Page -Window $window -Heading 'Pick your shortcut' -Number 5
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
    # A comma-separated list, which is what the page now asks for. Deliberately
    # spaced unevenly and given a trailing comma: a box that only reads a tidy
    # list is a box that loses words for real users.
    $terms = 'Kenneth,Anthropic , Granite,'
    [void][SpeakEasy.Win32]::SendMessageW($edit.Handle, $WM_SETTEXT, [IntPtr]::Zero, $terms)
    # Back and forward again, rather than reading the status straight away.
    # `WM_SETTEXT` does not reliably raise `EN_CHANGE` on a multi-line edit, so
    # the count is recomputed on arrival at the page -- and this additionally
    # proves that Back does not lose what was typed, which was never checked.
    Invoke-Click -Handle (Get-Button -Window $window -Text 'Back').Handle
    Assert-Page -Window $window -Heading 'Pick your shortcut' -Number 5
    Invoke-Next -Window $window
    Assert-Page -Window $window -Heading 'Add your words' -Number 6
    $wordStatus = Get-Status -Window $window
    if ($wordStatus -ne '3 words will be added: Kenneth, Anthropic, Granite') {
        throw "The words page read the list as '$wordStatus'."
    }
    Write-Host "  words: $wordStatus"
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'What should it keep?' -Number 7
    $boxes = @(Get-Controls -Window $window |
        Where-Object { $_.Class -eq 'Button' -and $_.Visible -and $_.Text -notin @('Back', 'Next', 'Cancel', 'Retry', 'Finish') })
    if ($boxes.Count -ne 2) {
        throw "Expected two check boxes on the retention page and found $($boxes.Count)."
    }
    Write-Host "  keeps: $($boxes.Text -join ' | ')"
    # Left at their defaults deliberately: retention off is the owner's decision
    # and the seed written below is what proves it travelled.
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Does dictation actually work?' -Number 8
    # Nothing may have been recorded yet. The installed-configuration marker is
    # written from this page's verdict and from nowhere else, so its *absence*
    # here is the assertion that it is no longer derived from the provider radio
    # three pages back. Checked before the verdict lands, because after it the
    # file is expected to exist.
    $providerRecord = Join-Path $configRoot 'install-provider.txt'
    if (Test-Path -LiteralPath $providerRecord) {
        throw ('install-provider.txt exists before the engine check has run, so it was not ' +
            'written from what setup proved.')
    }
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
    if ($verdict -notmatch 'transcribed the recording word for word') {
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
        # `cpu`, and it has to be: this payload carries no CUDA worker, so the
        # engine check cannot prove a graphics-card installation however capable
        # the card in this machine is. A `cuda` here would mean the marker had
        # gone back to describing an intention.
        'install-provider.txt'   = 'cpu'
        'install-vocabulary.txt' = 'Kenneth, Anthropic, Granite'
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

    # How much of the diagnostic log already existed. Everything after this is
    # what *this* launch wrote, and nothing before it may be read as evidence:
    # the log survives an uninstall by design, so a previous install's
    # `granite_warm` line is sitting in it and matches every pattern below.
    # Measured 2026-08-20 -- this assertion passed against a line written before
    # the binary under test was even built, and then failed against a stale line
    # from the format that predated the field it was checking.
    $logPath = Join-Path $dataRoot 'logs\speakeasy.log'
    $logBytesBefore = if (Test-Path -LiteralPath $logPath) { (Get-Item -LiteralPath $logPath).Length } else { 0 }

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
    # The app's own reading of the marker, in its log, beside what it actually
    # ran on. `provider=ok` is the comparison being made rather than left for a
    # reader; the combination this replaced was three correct fields whose
    # disagreement nothing looked at.
    $warmDeadline = [DateTime]::UtcNow.AddSeconds(180)
    $warmLine = $null
    while ([DateTime]::UtcNow -lt $warmDeadline) {
        if (Test-Path -LiteralPath $logPath) {
            # Only the bytes this launch appended. Reading the whole file finds a
            # previous install's line and reports it as this one's.
            $stream = [IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
            try {
                if ($stream.Length -gt $logBytesBefore) {
                    [void]$stream.Seek($logBytesBefore, 'Begin')
                    $reader = New-Object IO.StreamReader($stream)
                    $fresh = $reader.ReadToEnd()
                    $warmLine = @($fresh -split "`r?`n" | Where-Object { $_ -match 'event=granite_warm' }) |
                        Select-Object -Last 1
                }
            } finally {
                $stream.Dispose()
            }
            if ($warmLine) { break }
        }
        Start-Sleep -Seconds 2
    }
    if (-not $warmLine) { throw 'This launch never logged granite_warm, so its provider report cannot be read.' }
    if ($warmLine -notmatch 'installed=cpu') {
        throw "The app read a different installed configuration than setup recorded: $warmLine"
    }
    if ($warmLine -notmatch 'device=cpu') {
        throw "This payload has no CUDA worker, so the device cannot be anything but cpu: $warmLine"
    }
    if ($warmLine -notmatch 'provider=ok') {
        throw "The provider record and the running device disagree: $warmLine"
    }
    Write-Host "  warm: $($warmLine -replace '^\d+ ', '')"
    # Waited for by *content*, not by existence. The vocabulary is applied while
    # the app is still building its coordinators, a little after the seeds are
    # consumed, so reading it the instant the last seed disappeared found no
    # file at all -- and reported the words as lost when they arrived a moment
    # later. Waiting for the file to appear fixed that and then broke the same
    # way for the opposite reason: an ordinary uninstall keeps
    # `personalization.json`, so on a reinstall the file is already there and the
    # wait returns instantly with the *previous* install's words. Measured
    # 2026-08-20 against an app that had applied the new list correctly.
    #
    # `user_entry` rather than the `installer-*` id, because the id is what the
    # bug was about: setup names its entries by position, and matching on the
    # origin is the only way to see a stale entry a shorter list left behind.
    #
    # Exact contents, not a count, and not `-contains`. Until 2026-08-20 a
    # reinstall over kept state made the dictionary validator reject the user's
    # entire list as a conflicting rule, leaving the stale words in place and
    # reporting nothing. "The words that arrived are exactly the words typed" is
    # the only form of this check that fails on that -- and the deadline below is
    # what keeps it able to fail at all.
    $personalizationPath = Join-Path $configRoot 'personalization.json'
    $expectedWords = @('Kenneth', 'Anthropic', 'Granite')
    $sources = @()
    $personalizationDeadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $personalizationDeadline) {
        if (Test-Path -LiteralPath $personalizationPath -PathType Leaf) {
            try {
                $personalization = Get-Content -LiteralPath $personalizationPath -Raw | ConvertFrom-Json
                $installed = @($personalization.dictionary | Where-Object { $_.origin -eq 'user_entry' })
                $sources = @($installed | ForEach-Object { $_.source })
            } catch {
                # A read that landed mid-replace. The write is atomic, so the
                # next poll sees one whole file or the other.
                $sources = @()
            }
            if (($sources -join '|') -eq ($expectedWords -join '|')) { break }
        }
        Start-Sleep -Milliseconds 500
    }
    if (($sources -join '|') -ne ($expectedWords -join '|')) {
        throw "The dictionary holds '$($sources -join ', ')' where setup collected '$($expectedWords -join ', ')'."
    }
    $installed = @($sources)
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
            # `--keep-user-data` so a wizard run does not delete the 2.14 GB of
            # weights it just proved the engine against. The production default
            # removes them (owner decision 2026-08-21); this script exists to be
            # run repeatedly, which is exactly the case the flag is for.
            & $uninstaller --uninstall /S --keep-user-data | Out-Null
        }
    }
}
