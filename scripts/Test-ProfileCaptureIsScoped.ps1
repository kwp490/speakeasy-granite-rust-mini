#Requires -Version 7
<#
.SYNOPSIS
    Proves `ProfileCapture.ps1` restores what it captured, follows no reparse
    point, and deletes nothing it does not recognise.

.DESCRIPTION
    The capture and restore that bracket `Test-SetupWizard.ps1` are the only thing
    between a proof run and the operator's profile, and they take a `-ConfigRoot`
    -- so they can be driven against a scratch directory under `target\` instead
    of installing anything. No profile, no `%APPDATA%`.

    A canary directory outside the scratch config, with a junction pointing at it,
    is asserted byte-identical after every case. That is what distinguishes
    "reported the junction" from "followed the junction and got away with it".

    Everything this control creates it names; cleanup removes those names and
    nothing else, and leaves the directory in place if anything unexpected is in
    it.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'ProfileCapture.ps1')

$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
$scratch = Join-Path $targetRoot ('profile-capture-control-' + [Guid]::NewGuid().ToString('N'))
$configRoot = Join-Path $scratch 'config'
$canaryRoot = Join-Path $scratch 'canary'
$canaryFile = Join-Path $canaryRoot 'do-not-follow-me.txt'
$createdScratch = $false
$passed = 0
# The ancestor-junction case: an ordinary directory tree plus a link one level
# above the config root it is addressed through. Both under `target\`.
$aliasedTarget = Join-Path $scratch 'aliased-target'
$ancestorLink = Join-Path $scratch 'ancestor-link'
$driveCwdLink = Join-Path $scratch 'drive-cwd-link'

# The starting profile this control writes, and the only names its reset removes.
$baselineFiles = @{
    'settings.json'        = '{"privacy":{"persisted_history_enabled":true}}'
    'settings.json.bak'    = '{"privacy":{"persisted_history_enabled":true}}'
    'personalization.json' = '{"dictionary":[{"source":"LogicMonitor"}]}'
    'install-provider.txt' = 'cuda'
}
# Names this control plants during a case. Reset removes exactly these too; it
# never enumerates and pipes, because a control that deletes whatever it finds is
# not entitled to assert that the code under test does not.
$caseFileNames = @('the-operators-own-notes.json', 'install-hotkey.txt', 'install-vocabulary.txt',
    'personalization.json.bak')
$caseDirectoryNames = @('an-unknown-directory', 'settings.json', 'personalization.json')
$caseLinkNames = @('a-junction-nobody-listed', 'personalization.json', 'settings.json.bak')
# Every child this control puts inside a directory it creates. `Remove-NamedEntry`
# removes exactly these and then requires the directory to be empty, so a child
# nobody listed keeps the directory alive instead of going with it.
$expectedChildren = @{
    'an-unknown-directory' = @('inside.txt')
    'settings.json'        = @('inside.txt')
    'config'               = @('settings.json')
}

function Get-ScratchState {
    $state = [ordered]@{}
    if (Test-Path -LiteralPath $script:configRoot -PathType Container) {
        foreach ($file in (Get-ChildItem -LiteralPath $script:configRoot -File -Force | Sort-Object Name)) {
            $state[$file.Name] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        }
    }
    (($state.Keys | ForEach-Object { "$_=$($state[$_])" }) -join '|')
}

function Set-ScratchFile {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Content)
    Set-Content -LiteralPath (Join-Path $script:configRoot $Name) -Value $Content -Encoding utf8 -NoNewline
}

function Remove-NamedEntry {
    # By name, and by kind: a link is unlinked rather than recursed into.
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Name)
    $path = Join-Path $Root $Name
    switch (Get-ConfigEntryKind -Path $path) {
        'Missing' { }
        'File' { Remove-Item -LiteralPath $path -Force }
        'ReparsePoint' { [IO.Directory]::Delete($path, $false) }
        'Directory' {
            foreach ($child in @($script:expectedChildren[$Name])) {
                if ([string]::IsNullOrEmpty($child)) { continue }
                $childPath = Join-Path $path $child
                if ((Get-ConfigEntryKind -Path $childPath) -eq 'File') {
                    Remove-Item -LiteralPath $childPath -Force
                }
            }
            $left = @(Get-ChildItem -LiteralPath $path -Force)
            if ($left.Count -gt 0) {
                throw ($path + ' holds ' + ((@($left | ForEach-Object { $_.Name })) -join ', ') +
                    ', which this control did not create; leaving the directory in place.')
            }
            [IO.Directory]::Delete($path, $false)
        }
        default { throw ('cannot remove ' + $path + ': it is a ' + (Get-ConfigEntryKind -Path $path) + '.') }
    }
}

function Reset-Scratch {
    foreach ($name in ($script:caseLinkNames + $script:caseDirectoryNames + $script:caseFileNames)) {
        Remove-NamedEntry -Root $script:configRoot -Name $name
    }
    foreach ($name in $script:baselineFiles.Keys) {
        Remove-NamedEntry -Root $script:configRoot -Name $name
    }
    if (-not (Test-Path -LiteralPath $script:configRoot -PathType Container)) {
        New-Item -ItemType Directory -Path $script:configRoot -Force | Out-Null
    }
    $left = @(Get-ChildItem -LiteralPath $script:configRoot -Force)
    if ($left.Count -gt 0) {
        throw ('the scratch config still holds ' + (($left | ForEach-Object { $_.Name }) -join ', ') +
            '; this control did not create it and will not remove it.')
    }
    foreach ($name in $script:baselineFiles.Keys) {
        Set-ScratchFile -Name $name -Content $script:baselineFiles[$name]
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory)][string]$What,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Expected,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Actual
    )
    if ($Expected -ne $Actual) { throw "${What}: expected <$Expected>, got <$Actual>." }
}

function Assert-CanaryIntact {
    param([Parameter(Mandatory)][string]$Stage)
    if (-not (Test-Path -LiteralPath $script:canaryFile -PathType Leaf)) {
        throw "${Stage}: the canary file is gone -- something followed the junction."
    }
    $now = (Get-FileHash -LiteralPath $script:canaryFile -Algorithm SHA256).Hash
    if ($now -ne $script:canaryHash) { throw "${Stage}: the canary changed: $now, was $script:canaryHash." }
    $extra = @(Get-ChildItem -LiteralPath $script:canaryRoot -Force | Where-Object { $_.Name -ne 'do-not-follow-me.txt' })
    if ($extra.Count -gt 0) {
        throw "${Stage}: something wrote into the canary directory: $(($extra | ForEach-Object { $_.Name }) -join ', ')."
    }
    Write-Host "  canary intact"
}

function Assert-RestoreReports {
    param(
        [Parameter(Mandatory)][string]$Stage,
        [Parameter(Mandatory)][string[]]$Containing
    )
    $threw = $false
    $message = $null
    try { Restore-ConfigCapture -ConfigRoot $script:configRoot -Capture $script:capture }
    catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw "${Stage}: the restore reported success instead of reporting the entry." }
    foreach ($needle in $Containing) {
        if ($message -notlike "*$needle*") {
            throw "${Stage}: the report does not mention '$needle'. Reported: $message"
        }
    }
    Write-Host "  reported: $message"
}

try {
    if (Test-Path -LiteralPath $scratch) {
        throw "The scratch directory already exists, which a fresh GUID makes impossible: $scratch"
    }
    New-Item -ItemType Directory -Path $configRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $canaryRoot -Force | Out-Null
    $createdScratch = $true
    Set-Content -LiteralPath $canaryFile -Value 'test-owned, and every case must leave it alone' -Encoding utf8 -NoNewline
    $canaryHash = (Get-FileHash -LiteralPath $canaryFile -Algorithm SHA256).Hash
    $tracked = @(Get-TrackedConfigFileNames)
    Write-Host "scratch config: $configRoot"
    Write-Host "canary        : $canaryFile ($canaryHash)"

    foreach ($seed in (Get-InstallSeedFileNames)) {
        if ($tracked -notcontains $seed) { throw "The seed $seed is not tracked." }
    }
    foreach ($required in 'settings.json', 'settings.json.bak', 'personalization.json',
        'personalization.json.bak') {
        if ($tracked -notcontains $required) { throw "Get-TrackedConfigFileNames does not list $required." }
    }
    Write-Host '  every seed and profile file is tracked'
    $passed++

    # --- 1. A captured file the run rewrote is put back -----------------------
    Write-Host ''
    Write-Host 'CASE: the run rewrites a captured file'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Set-ScratchFile -Name 'personalization.json' -Content '{"dictionary":[{"source":"Kenneth"}]}'
    if ((Get-ScratchState) -eq $pristine) { throw 'the rewrite changed nothing; the case is vacuous.' }
    Restore-ConfigCapture -ConfigRoot $configRoot -Capture $capture
    Assert-Equal -What 'after restoring a rewritten file' -Expected $pristine -Actual (Get-ScratchState)
    $passed++

    # --- 2. A captured file the run deleted is put back -----------------------
    Write-Host ''
    Write-Host 'CASE: the run deletes a captured file'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Remove-Item -LiteralPath (Join-Path $configRoot 'install-provider.txt') -Force
    Restore-ConfigCapture -ConfigRoot $configRoot -Capture $capture
    Assert-Equal -What 'after restoring a deleted file' -Expected $pristine -Actual (Get-ScratchState)
    $passed++

    # --- 3. Tracked files the run created are removed again -------------------
    Write-Host ''
    Write-Host 'CASE: the run creates tracked files that were absent'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Set-ScratchFile -Name 'personalization.json.bak' -Content 'rotated by the app'
    Set-ScratchFile -Name 'install-hotkey.txt' -Content 'Ctrl+Alt+P'
    Set-ScratchFile -Name 'install-vocabulary.txt' -Content 'Kenneth, Anthropic, Granite'
    Restore-ConfigCapture -ConfigRoot $configRoot -Capture $capture
    Assert-Equal -What 'after removing files the run created' -Expected $pristine -Actual (Get-ScratchState)
    $passed++

    # --- 4. An unrecognised file is left alone and reported -------------------
    Write-Host ''
    Write-Host 'CASE: a filename nothing anticipated turns up'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Set-ScratchFile -Name 'settings.json' -Content '{"privacy":{"persisted_history_enabled":false}}'
    Set-ScratchFile -Name 'the-operators-own-notes.json' -Content 'not ours to delete'
    Assert-RestoreReports -Stage 'an unknown file' -Containing 'the-operators-own-notes.json', 'File'
    $stranger = Join-Path $configRoot 'the-operators-own-notes.json'
    if (-not (Test-Path -LiteralPath $stranger -PathType Leaf)) {
        throw 'the restore deleted the unrecognised file it was supposed to leave alone.'
    }
    if ((Get-Content -LiteralPath $stranger -Raw) -ne 'not ours to delete') {
        throw 'the restore altered the unrecognised file.'
    }
    # The recognised half was still restored: reporting the surprise must not cost
    # the operator the restore.
    Remove-NamedEntry -Root $configRoot -Name 'the-operators-own-notes.json'
    Assert-Equal -What 'the captured files despite the unknown one' -Expected $pristine -Actual (Get-ScratchState)
    Write-Host '  the unknown file survived byte for byte and everything captured was restored'
    $passed++

    # --- 5. An unrecognised directory ----------------------------------------
    Write-Host ''
    Write-Host 'CASE: an unknown directory turns up'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    $unknownDirectory = Join-Path $configRoot 'an-unknown-directory'
    New-Item -ItemType Directory -Path $unknownDirectory -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $unknownDirectory 'inside.txt') -Value 'also not ours' -Encoding utf8 -NoNewline
    Assert-RestoreReports -Stage 'an unknown directory' -Containing 'an-unknown-directory', 'Directory'
    if (-not (Test-Path -LiteralPath (Join-Path $unknownDirectory 'inside.txt') -PathType Leaf)) {
        throw 'the restore emptied or removed the unknown directory.'
    }
    Assert-Equal -What 'the captured files despite the unknown directory' -Expected $pristine -Actual (Get-ScratchState)
    Write-Host '  the unknown directory and its contents are untouched'
    $passed++

    # --- 6. A junction nobody listed ------------------------------------------
    Write-Host ''
    Write-Host 'CASE: an unknown junction turns up, pointing at the canary'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    New-Item -ItemType Junction -Path (Join-Path $configRoot 'a-junction-nobody-listed') -Target $canaryRoot | Out-Null
    Assert-RestoreReports -Stage 'an unknown junction' -Containing 'a-junction-nobody-listed', 'ReparsePoint'
    Assert-CanaryIntact -Stage 'an unknown junction'
    Assert-Equal -What 'the captured files despite the junction' -Expected $pristine -Actual (Get-ScratchState)
    $passed++

    # --- 7. A directory at a tracked filename, after capture ------------------
    # `settings.json` was a file when captured and is a directory now. Writing the
    # captured bytes "back" would have to go through it; deleting it would take
    # whatever is inside. Neither: report it.
    Write-Host ''
    Write-Host 'CASE: a tracked filename becomes a directory after capture'
    Reset-Scratch
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Remove-NamedEntry -Root $configRoot -Name 'settings.json'
    $trackedAsDirectory = Join-Path $configRoot 'settings.json'
    New-Item -ItemType Directory -Path $trackedAsDirectory -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $trackedAsDirectory 'inside.txt') -Value 'inside a tracked name' -Encoding utf8 -NoNewline
    Assert-RestoreReports -Stage 'a tracked name as a directory' -Containing 'settings.json is now a Directory'
    if (-not (Test-Path -LiteralPath (Join-Path $trackedAsDirectory 'inside.txt') -PathType Leaf)) {
        throw 'the restore removed the directory standing at a tracked filename.'
    }
    if ((Get-ConfigEntryKind -Path $trackedAsDirectory) -ne 'Directory') {
        throw 'the restore wrote a file over the directory standing at a tracked filename.'
    }
    Write-Host '  the directory at a tracked name is untouched and was reported, not restored over'
    $passed++

    # --- 8. A reparse point at a tracked filename, after capture --------------
    Write-Host ''
    Write-Host 'CASE: a tracked filename becomes a junction after capture'
    Reset-Scratch
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Remove-NamedEntry -Root $configRoot -Name 'personalization.json'
    New-Item -ItemType Junction -Path (Join-Path $configRoot 'personalization.json') -Target $canaryRoot | Out-Null
    Assert-RestoreReports -Stage 'a tracked name as a junction' `
        -Containing 'personalization.json is now a ReparsePoint'
    Assert-CanaryIntact -Stage 'a tracked name as a junction'
    if ((Get-ConfigEntryKind -Path (Join-Path $configRoot 'personalization.json')) -ne 'ReparsePoint') {
        throw 'the restore replaced the junction standing at a tracked filename.'
    }
    Write-Host '  the junction at a tracked name was reported, not followed'
    $passed++

    # --- 9. A tracked name that is a junction at capture time -----------------
    # Absent and "a junction" are not the same thing, and only the first licenses
    # the restore to delete. The capture refuses rather than recording absence.
    Write-Host ''
    Write-Host 'CASE: a tracked filename is already a junction when the capture runs'
    Reset-Scratch
    Remove-NamedEntry -Root $configRoot -Name 'settings.json.bak'
    New-Item -ItemType Junction -Path (Join-Path $configRoot 'settings.json.bak') -Target $canaryRoot | Out-Null
    $threw = $false; $message = $null
    try { Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw 'the capture accepted a junction at a tracked filename.' }
    if ($message -notlike '*settings.json.bak is a ReparsePoint*') {
        throw "the capture refused for an unrelated reason: $message"
    }
    Write-Host "  refused: $message"
    Assert-CanaryIntact -Stage 'a junction at capture time'
    $passed++

    # --- 10. A reparse point at the config root itself ------------------------
    Write-Host ''
    Write-Host 'CASE: the config root is a junction'
    Reset-Scratch
    $rootLink = Join-Path $scratch 'config-link'
    New-Item -ItemType Junction -Path $rootLink -Target $configRoot | Out-Null
    try {
        foreach ($stage in 'capture', 'restore') {
            $threw = $false; $message = $null
            try {
                if ($stage -eq 'capture') { Get-ConfigCapture -ConfigRoot $rootLink -AlsoTrack $tracked }
                else { Restore-ConfigCapture -ConfigRoot $rootLink -Capture (Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked) }
            } catch { $threw = $true; $message = $_.Exception.Message }
            if (-not $threw) { throw "the $stage accepted a junction as the config root." }
            if ($message -notlike '*config root is a ReparsePoint*') {
                throw "the $stage refused for an unrelated reason: $message"
            }
            Write-Host "  $stage refused: $message"
        }
    } finally {
        [IO.Directory]::Delete($rootLink, $false)
    }
    Assert-CanaryIntact -Stage 'a junction config root'
    $passed++

    # --- 11. A junction on an *ancestor* of the config root -------------------
    # The leaf is an ordinary directory and the config child is an ordinary file;
    # everything one level up is a lie. A leaf-only check accepts this, and the
    # restore then writes the captured bytes over whatever the junction really
    # points at.
    Write-Host ''
    Write-Host 'CASE: a junction on an ancestor of the config root'
    $aliasedConfig = Join-Path $aliasedTarget 'config'
    New-Item -ItemType Directory -Path $aliasedConfig -Force | Out-Null
    $aliasedSettings = Join-Path $aliasedConfig 'settings.json'

    # Capture the poison, then leave the canary on disk: a restore that follows
    # the junction overwrites one with the other, which is visible.
    Set-Content -LiteralPath $aliasedSettings -Value '{"poison":true}' -Encoding utf8 -NoNewline
    $throughOrdinaryPath = Get-ConfigCapture -ConfigRoot $aliasedConfig -AlsoTrack $tracked
    Set-Content -LiteralPath $aliasedSettings -Value '{"canary":true}' -Encoding utf8 -NoNewline
    $aliasedCanaryHash = (Get-FileHash -LiteralPath $aliasedSettings -Algorithm SHA256).Hash

    New-Item -ItemType Junction -Path $ancestorLink -Target $aliasedTarget | Out-Null
    $viaAncestor = Join-Path $ancestorLink 'config'
    if ((Get-ConfigEntryKind -Path $viaAncestor) -ne 'Directory') {
        throw 'the aliased config root is not an ordinary directory; the case would prove nothing.'
    }

    $threw = $false; $message = $null
    try { Get-ConfigCapture -ConfigRoot $viaAncestor -AlsoTrack $tracked } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw 'the capture accepted a config root behind an ancestor junction.' }
    if ($message -notlike '*reparse point on the way to the config root*') {
        throw "the capture refused for an unrelated reason: $message"
    }
    Write-Host "  capture refused: $message"

    $threw = $false; $message = $null
    try { Restore-ConfigCapture -ConfigRoot $viaAncestor -Capture $throughOrdinaryPath } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw 'the restore accepted a config root behind an ancestor junction.' }
    if ($message -notlike '*reparse point on the way to the config root*') {
        throw "the restore refused for an unrelated reason: $message"
    }
    Write-Host "  restore refused: $message"

    $afterHash = (Get-FileHash -LiteralPath $aliasedSettings -Algorithm SHA256).Hash
    if ($afterHash -ne $aliasedCanaryHash) {
        throw 'the restore followed the ancestor junction and overwrote the canary behind it.'
    }
    Write-Host '  the file behind the junction is untouched'
    [IO.Directory]::Delete($ancestorLink, $false)
    Assert-CanaryIntact -Stage 'an ancestor junction'
    $passed++

    # --- 12. Cleanup refuses a directory holding something it did not create ---
    # This control's own rule, turned on itself: `Remove-NamedEntry` removes the
    # children it listed and then requires the directory to be empty.
    Write-Host ''
    Write-Host "CASE: this control's cleanup refuses an unexpected child"
    Reset-Scratch
    $unexpectedHome = Join-Path $configRoot 'an-unknown-directory'
    New-Item -ItemType Directory -Path $unexpectedHome -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $unexpectedHome 'inside.txt') -Value 'mine' -Encoding utf8 -NoNewline
    Set-Content -LiteralPath (Join-Path $unexpectedHome 'not-mine.txt') -Value 'somebody else' -Encoding utf8 -NoNewline
    $threw = $false; $message = $null
    try { Remove-NamedEntry -Root $configRoot -Name 'an-unknown-directory' } catch { $threw = $true; $message = $_.Exception.Message }
    if (-not $threw) { throw 'the cleanup removed a directory holding a file this control did not create.' }
    if ($message -notlike '*not-mine.txt*') { throw "the cleanup refused for an unrelated reason: $message" }
    Write-Host "  refused: $message"
    if (-not (Test-Path -LiteralPath (Join-Path $unexpectedHome 'not-mine.txt') -PathType Leaf)) {
        throw 'the unexpected child was deleted.'
    }
    if ((Get-ConfigEntryKind -Path $unexpectedHome) -ne 'Directory') {
        throw 'the directory was removed despite holding an unexpected child.'
    }
    Write-Host '  the unexpected child and its directory both survive'
    # Removed by hand, because the control created this one deliberately.
    Remove-Item -LiteralPath (Join-Path $unexpectedHome 'not-mine.txt') -Force
    Remove-NamedEntry -Root $configRoot -Name 'an-unknown-directory'
    $passed++

    # --- 13. The path chain keeps the filesystem root ------------------------
    Write-Host ''
    Write-Host 'CASE: the path chain preserves the filesystem root'
    $chain = @(Get-ConfigPathChain -Path $configRoot)
    $expectedRoot = [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($configRoot))
    if ($chain[0] -ne $expectedRoot) {
        throw "the chain starts at '$($chain[0])', not the filesystem root '$expectedRoot'."
    }
    # `C:` is drive-*relative*; `C:\` is the root. The difference is the defect.
    if ($chain[0] -eq $expectedRoot.TrimEnd([IO.Path]::DirectorySeparatorChar)) {
        throw "the chain starts at a drive-relative '$($chain[0])' rather than the root."
    }
    if ($chain[-1] -ne [IO.Path]::GetFullPath($configRoot)) {
        throw "the chain ends at '$($chain[-1])', not the config root."
    }
    $rootChain = @(Get-ConfigPathChain -Path $expectedRoot)
    if ($rootChain.Count -ne 1 -or $rootChain[0] -ne $expectedRoot) {
        throw "the chain for the root itself is '$($rootChain -join ', ')'."
    }
    Write-Host "  chain runs $($chain[0]) .. $($chain[-1]), $($chain.Count) component(s)"
    $passed++

    # --- 14. The drive's current location cannot cause a false refusal --------
    # `Get-Item C:` answers about whatever directory the process last used on
    # that drive. With the root trimmed to `C:`, standing in a junction made an
    # unrelated ConfigRoot refuse, naming a path the caller never gave.
    Write-Host ''
    Write-Host "CASE: the drive's current location cannot cause a false refusal"
    New-Item -ItemType Junction -Path $driveCwdLink -Target $canaryRoot | Out-Null
    Push-Location $driveCwdLink
    try {
        $standingIn = Get-Item -LiteralPath ([IO.Path]::GetPathRoot((Get-Location).Path).TrimEnd([IO.Path]::DirectorySeparatorChar)) -Force
        if (-not ($standingIn.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'the drive-relative name does not resolve to the junction; the case would prove nothing.'
        }
        foreach ($unrelated in @($expectedRoot, $configRoot)) {
            Assert-OrdinaryConfigRoot -ConfigRoot $unrelated -Stage 'Probe'
            Write-Host "  accepted while standing in a junction: $unrelated"
        }
    } finally {
        Pop-Location
    }
    [IO.Directory]::Delete($driveCwdLink, $false)
    Assert-CanaryIntact -Stage "the drive's current location"
    $passed++

    # --- 15. A classification failure is not Missing --------------------------
    # Missing is what licenses a restore to write. A path the provider cannot
    # classify at all -- access denied, an unknown drive, an invalid name -- must
    # therefore throw and name itself, not be folded into "nothing is there".
    Write-Host ''
    Write-Host 'CASE: a path that cannot be classified throws rather than reading as Missing'
    $usedDrives = (Get-PSDrive -PSProvider FileSystem).Name
    $freeDrive = 67..90 | ForEach-Object { [char]$_ } | Where-Object { $usedDrives -notcontains "$_" } | Select-Object -First 1
    if (-not $freeDrive) {
        Write-Host '  SKIPPED: every drive letter is in use, so no unclassifiable path is available' -ForegroundColor Yellow
    } else {
        $unclassifiable = "${freeDrive}:" + [IO.Path]::DirectorySeparatorChar + 'nope'
        $threw = $false; $message = $null
        try { $null = Get-ConfigEntryKind -Path $unclassifiable } catch { $threw = $true; $message = $_.Exception.Message }
        if (-not $threw) { throw "Get-ConfigEntryKind returned a kind for $unclassifiable instead of throwing." }
        if ($message -notlike "*$unclassifiable*") { throw "the failure does not name the path: $message" }
        if ($message -notlike '*DriveNotFound*') { throw "the failure does not name the provider error: $message" }
        Write-Host "  threw: $message"
        # And a genuinely absent path under an ordinary directory is still Missing.
        if ((Get-ConfigEntryKind -Path (Join-Path $configRoot 'definitely-not-here.txt')) -ne 'Missing') {
            throw 'an absent name no longer reads as Missing.'
        }
        Write-Host '  an absent name still reads as Missing'
        $passed++
    }

    # --- 16. A run that changed nothing --------------------------------------
    Write-Host ''
    Write-Host 'CASE: the run changed nothing'
    Reset-Scratch
    $pristine = Get-ScratchState
    $capture = Get-ConfigCapture -ConfigRoot $configRoot -AlsoTrack $tracked
    Restore-ConfigCapture -ConfigRoot $configRoot -Capture $capture
    Assert-Equal -What 'after a no-op restore' -Expected $pristine -Actual (Get-ScratchState)
    $passed++

    Assert-CanaryIntact -Stage 'every case'
    Write-Host ''
    Write-Host "profile capture is scoped: passed ($passed assertions)"
}
finally {
    # Exact names, then the directories non-recursively. A control that asserts a
    # restore does not delete what it did not create cannot clean up with
    # `-Recurse`, or by enumerating and piping.
    if ($createdScratch) {
        foreach ($name in @('config-link', 'ancestor-link', 'drive-cwd-link')) { Remove-NamedEntry -Root $scratch -Name $name }
        if (Test-Path -LiteralPath $aliasedTarget -PathType Container) {
            Remove-NamedEntry -Root $aliasedTarget -Name 'config'
            $left = @(Get-ChildItem -LiteralPath $aliasedTarget -Force)
            if ($left.Count -eq 0) { [IO.Directory]::Delete($aliasedTarget, $false) }
            else { Write-Warning "$aliasedTarget holds $(($left | ForEach-Object { $_.Name }) -join ', ') and stays" -WarningAction Continue }
        }
        if (Test-Path -LiteralPath $configRoot -PathType Container) {
            foreach ($name in ($caseLinkNames + $caseDirectoryNames + $caseFileNames + @($baselineFiles.Keys))) {
                Remove-NamedEntry -Root $configRoot -Name $name
            }
            $left = @(Get-ChildItem -LiteralPath $configRoot -Force)
            if ($left.Count -eq 0) { [IO.Directory]::Delete($configRoot, $false) }
            else { Write-Warning "$configRoot holds $(($left | ForEach-Object { $_.Name }) -join ', ') and stays" -WarningAction Continue }
        }
        if (Test-Path -LiteralPath $canaryRoot -PathType Container) {
            Remove-NamedEntry -Root $canaryRoot -Name 'do-not-follow-me.txt'
            $left = @(Get-ChildItem -LiteralPath $canaryRoot -Force)
            if ($left.Count -eq 0) { [IO.Directory]::Delete($canaryRoot, $false) }
            else { Write-Warning "$canaryRoot holds $(($left | ForEach-Object { $_.Name }) -join ', ') and stays" -WarningAction Continue }
        }
        if ([IO.Directory]::Exists($scratch)) {
            $left = @(Get-ChildItem -LiteralPath $scratch -Force)
            if ($left.Count -eq 0) { [IO.Directory]::Delete($scratch, $false) }
            else { Write-Warning "$scratch holds $(($left | ForEach-Object { $_.Name }) -join ', ') and stays" -WarningAction Continue }
        }
    }
}

# Explicit, so a caller reading the exit code gets this run's answer.
exit 0
