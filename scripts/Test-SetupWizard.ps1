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
    # The graphics-card option is **shown** on every machine, and enabled only
    # where a graphics-card configuration is both published and suited to the
    # card. Hiding it would read as setup not having examined the card;
    # enabling it where it cannot be installed is what wrote `installed=cuda`
    # onto an installation carrying no CUDA worker, and the app then correctly
    # ran on the processor while the support log read
    # `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`.
    #
    # **This assertion inverted on 2026-08-26**, when the CUDA worker was
    # published. Before then it demanded the option be disabled on every
    # machine, which was right for as long as no worker existed and is now a
    # statement about the card in whatever machine this runs on. So the
    # expectation is *derived*: whether the option is enabled decides what the
    # rest of this script requires of the download and of the marker. Hardcoding
    # either answer would make this pass on one class of machine and be a lie on
    # the other.
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
    $graphicsCardOffered = [bool]$graphicsCard.Enabled
    if ($graphicsCardOffered) {
        # **Chosen, not merely offered.** This clicked Next and accepted whatever
        # the page arrived holding, so on a capable machine the whole
        # graphics-card path rode on that option happening to be the *default*
        # selection -- a property nothing asserts and any layout change could
        # move. If it moved, the four-item plan below would begin failing on a
        # machine where nothing was actually wrong, which is the least useful
        # direction for a proof to break in.
        #
        # Clicking it makes the answer this script's own, so the plan asserted on
        # the next page is evidence that a deliberate choice reached
        # `download::plan` rather than evidence about a default.
        #
        # Still not asserted by reading the radio back: `Get-Controls` reports
        # `Visible` and `Enabled` and no check state, so `$graphicsCard.Checked`
        # is `$null`, inverts to true, and would throw on every capable machine
        # -- a broken instrument shaped exactly like the failure it claims to
        # find. What confirms the click is the wizard's own next page.
        Invoke-Click -Handle $graphicsCard.Handle
        Write-Host "  provider: graphics-card option offered and selected; $($providerStatus.Split([Environment]::NewLine)[0])"
    } else {
        # Disabled must come with the reason. A greyed control and no sentence is
        # the state this page exists to avoid.
        if ($providerStatus -notmatch 'graphics card|graphics-card') {
            throw ('The graphics-card option is disabled and the page does not say why: ' +
                $providerStatus)
        }
        Write-Host "  provider: graphics-card option shown and disabled with a reason; $($providerStatus.Split([Environment]::NewLine)[0])"
    }
    Invoke-Next -Window $window

    Assert-Page -Window $window -Heading 'Download the models' -Number 3
    # **The provider answer reaching the plan, proved in the wizard's own words.**
    # This page lists what it will fetch, one artifact per line, and the list is
    # built from `download::plan(provider)` -- so a graphics-card machine naming
    # the engine here is the end-to-end evidence that the radio button three
    # pages back is read rather than merely drawn. It was not read until
    # 2026-08-26: the plan took the machine's *capability*, which is the same
    # value as the answer for exactly as long as the option stays disabled, so
    # nothing before the worker was published could have told the two apart.
    #
    # Read before the click, not after: the first version of this reported the
    # *install* page's notice under the heading "download", because by then the
    # wizard had already advanced.
    $downloadNotice = Get-Status -Window $window
    if ([string]::IsNullOrWhiteSpace($downloadNotice)) {
        $downloadNotice = (Get-Controls -Window $window |
            Where-Object { $_.Class -eq 'Static' -and $_.Visible } |
            ForEach-Object { $_.Text }) -join "`n"
    }
    # A page that reports everything already present names no artifacts, by
    # design -- nothing was transferred and the reason is that the files are
    # there and still match their digests. That is a legitimate state on a
    # machine that has run this before, and asserting the artifact list against
    # it would fail the proof for the one reason that is not a defect.
    # **Read the item *count*, not the artifact names.** The first version of this
    # required the notice to contain "Graphics-card engine", and it failed on a
    # run where everything worked: arriving at this page calls `begin_transfer`,
    # which replaces the plan listing with a progress line within the poll
    # interval, and the progress line names only the item in flight. So the
    # naming assertion was racing the transfer it was meant to describe.
    #
    # "(N of M)" is in the progress line and survives the whole step, and M is a
    # better signal than the names anyway: 4 against 1 distinguishes the two
    # plans unambiguously, and it cannot be satisfied by a label appearing
    # somewhere for another reason.
    $alreadyPresent = $downloadNotice -match 'already here'
    $planned = if ($downloadNotice -match '\(\d+ of (\d+)\)') { [int]$Matches[1] } else { 0 }
    if ($alreadyPresent) {
        Write-Host '  download: everything already present, so there is no plan to count'
    } elseif ($planned -eq 0) {
        # Neither a count nor an already-present notice. Refused rather than
        # skipped: an unreadable page here is the instrument failing, and
        # passing over it would make this whole assertion optional.
        throw "The download page states neither a plan nor a progress count: $downloadNotice"
    } elseif ($graphicsCardOffered) {
        # The weights, the engine, and the two NVIDIA redistributables.
        if ($planned -ne 4) {
            throw ('The graphics card was offered and the plan has ' + $planned +
                ' item(s), not 4, so the answer did not reach the plan: ' + $downloadNotice)
        }
        Write-Host '  download: plan has 4 items, so the graphics-card answer reached it'
    } else {
        if ($planned -ne 1) {
            throw ('The processor was the only option and the plan has ' + $planned +
                ' item(s), not 1: ' + $downloadNotice)
        }
        Write-Host '  download: plan has 1 item, the weights alone'
    }
    # Generous, because this is the step that fetches ~2.3 GB of weights on a
    # machine that does not already have them, and ~400 MB more on a
    # graphics-card install.
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

    # **The box arrives holding the shipped default**, checked before anything
    # overwrites it. Setup fills it from `catalog::DEFAULT_VOCABULARY` so the
    # common tools work without the user knowing the feature exists, and an
    # empty box is exactly what a regression here looks like -- silently, since
    # every assertion below still passes against a box this script filled
    # itself. Asserted as a count and two members rather than the whole string:
    # pinning the list here would mean editing this script to add a word.
    # `Get-Controls` reads every child's text with `GetWindowTextW`, which on an
    # Edit returns its contents -- so the box was already captured above and
    # needs no second round trip.
    $prefilled = $edit.Text
    $prefilledWords = @($prefilled -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
    if ($prefilledWords.Count -lt 10) {
        throw "The vocabulary box did not arrive prefilled; it read '$prefilled'."
    }
    foreach ($expected in 'LogicMonitor', 'ServiceNow') {
        if ($prefilledWords -notcontains $expected) {
            throw "The prefilled vocabulary is missing '$expected': '$prefilled'."
        }
    }
    Write-Host "  prefilled: $($prefilledWords.Count) words"

    # And the page counts what it arrived holding, without anyone touching the
    # box. `WM_SETTEXT` does not raise `EN_CHANGE` on a multi-line edit, so a
    # page that only recomputed on change would say "No words yet" over a full
    # box -- the exact shape of the defect the Back/Next dance below exists for.
    $arrivalStatus = Get-Status -Window $window
    if ($arrivalStatus -notmatch "^$($prefilledWords.Count) words will be added: ") {
        throw "The words page did not count its own prefilled list: '$arrivalStatus'."
    }

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

    # What the marker must say, taken from the sentence the user is shown rather
    # than from what this script expected. The two agreeing is the property worth
    # proving: the marker is written from this verdict and from nowhere else, so a
    # page that says one thing while the file says another is the 2026-08-20
    # defect returning by a different route.
    #
    # `Provider-FromVerdict` deliberately does not accept "the card is capable"
    # as evidence. A machine whose driver refuses, whose card is claimed, or
    # whose VRAM is exhausted runs the same binary on the processor, and the
    # honest record for that is `cpu` even though the graphics card was chosen
    # three pages back.
    $expectedProvider = if ($verdict -match 'graphics card') { 'cuda' } else { 'cpu' }
    if (-not $graphicsCardOffered -and $expectedProvider -ne 'cpu') {
        throw ('The engine check proved a graphics-card configuration on a machine that was ' +
            'never offered one. The marker would be recording something setup did not install.')
    }
    if ($graphicsCardOffered -and $expectedProvider -eq 'cpu') {
        # Not a failure of this script: a real and disclosed outcome. Said loudly
        # rather than passed over, because it means the graphics-card payload was
        # fetched and staged and then did not take, and the run below is
        # therefore not exercising the path it looks like it is exercising.
        Write-Host ('  engine: NOTE - the graphics card was offered and chosen, and the check ' +
            'proved the processor. That is honest, not a pass of the CUDA path.') -ForegroundColor Yellow
    }
    Write-Host "  engine: verdict names $expectedProvider; the marker must agree"

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
        # Filled in below, from what the engine check actually proved rather
        # than from what this script hoped for. It was a hardcoded `cpu` until
        # 2026-08-26, correct only while no CUDA worker was published.
        'install-provider.txt'   = $expectedProvider
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
    # Both of these were hardcoded `cpu` until 2026-08-26, and both were correct
    # only while no CUDA worker was published. They are the third and fourth in
    # this script to invert on the pin -- the provider page and the marker were
    # the first two, found and fixed before this run, which is why finding two
    # more here is worth writing down: a sweep that stops at the first pair is
    # not a sweep.
    if ($warmLine -notmatch "installed=$expectedProvider") {
        throw ("The app read a different installed configuration than setup proved " +
            "($expectedProvider): $warmLine")
    }
    # The device the app is *live* on, which is a separate reading from the
    # record and the whole reason both are logged. `cuda_unverified` is a
    # legitimate third answer -- the worker reported a CUDA backend and NVML
    # could not be asked to confirm it -- and reading it as a failure would
    # report a fault on a machine whose driver merely would not answer.
    $expectedDevices = if ($expectedProvider -eq 'cuda') { @('cuda', 'cuda_unverified') } else { @('cpu') }
    $device = if ($warmLine -match 'device=(\S+)') { $Matches[1] } else { '' }
    if ($device -notin $expectedDevices) {
        throw ("The app is running on '$device', which is not one of " +
            "$($expectedDevices -join '/') for an installation proved as " +
            "$expectedProvider): $warmLine")
    }
    # `engine=` is deliberately not asserted against the provider. It is the
    # *pack* reason, and on a graphics-card machine the correct value is
    # `cpu_gpu_pack_not_installed` -- there is one GGUF, its id ends `-cpu`, and
    # the CUDA worker offloads that same file, so a CUDA pack would be a
    # duplicate. Reading that as a fault is the mistake `download::plan`'s own
    # doc comment exists to prevent.
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
