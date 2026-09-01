#Requires -Version 7
<#
.SYNOPSIS
    Proves the host path-identity check's behaviour without a profile and without
    the administrative share.

.DESCRIPTION
    `HostProfilePathIdentity.ps1` takes both directory views explicitly for this
    reason: the contract can be driven against two directories under `target\`,
    so the shell running it needs no host, no elevation and no share. Deriving the
    real administrative-share view is a separate, pure string function and is
    proved here by mapping alone.

    What is checked:

    - two syntactically distinct paths that address the same directory pass, in
      both directions, and the probe is gone afterwards;
    - two different directories are refused, naming the first direction;
    - a one-way mirror is refused in whichever direction is broken -- the case
      that makes each of the two comparisons load-bearing, and the reason the
      checker exposes a write-view seam;
    - an entry of any kind already at the probe name is refused and not altered;
    - an entry of the wrong kind at the probe name when cleanup runs is left
      where it is and reported, never read as an absence;
    - a path that cannot be classified, and one whose parent is not a reachable
      directory, throw rather than becoming `Missing`;
    - the drive is read from the path rather than assumed, and a path that cannot
      be mapped unambiguously is refused;
    - the four proofs that install for real call the guard before they first
      touch the live profile, the registry or a process.

    **Every case ends by asserting each view holds exactly its own canary file.**
    That is what catches a cleanup which enumerated a directory rather than
    removing one name: the probe would be gone either way, and the canary would
    not be.

    The last group is a source assertion, deliberately: the boundary it describes
    is the real `%APPDATA%`, and a proof that reached it before the guard would
    already have done the damage the guard exists to prevent.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'HostProfilePathIdentity.ps1')
. (Join-Path $PSScriptRoot 'DeleteContainment.ps1')

$targetRoot = (Resolve-Path -LiteralPath (Join-Path $repositoryRoot 'target')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
# A GUID rather than the process id: a pid is reused, and the `finally` removes
# this directory recursively.
$scratch = Join-Path $targetRoot ('host-path-identity-control-' + [Guid]::NewGuid().ToString('N'))
$viewA = Join-Path $scratch 'view-a'
$viewB = Join-Path $scratch 'view-b'
$elsewhere = Join-Path $scratch 'elsewhere'
$viewALink = Join-Path $scratch 'view-a-link'
$canaryName = 'operator-would-notice-this.txt'
$canaryHashes = @{}
$passed = 0
$scratchCreated = $false
$linkCreated = $false

function Assert-Refused {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][scriptblock]$Action,
        [string[]]$MessageContains = @()
    )
    $message = $null
    try {
        & $Action | Out-Null
    } catch {
        $message = $_.Exception.Message
    }
    if ($null -eq $message) {
        throw "CASE '$Name': expected a refusal, and the check returned successfully."
    }
    foreach ($needle in $MessageContains) {
        if ($message -notlike "*$needle*") {
            throw "CASE '$Name': the refusal does not mention '$needle'. It said: $message"
        }
    }
    $message
}

function Assert-ViewsHoldOnlyTheirCanary {
    param([Parameter(Mandatory)][string]$Stage)
    foreach ($view in @($viewA, $viewB, $elsewhere)) {
        $entries = @(Get-ChildItem -LiteralPath $view -Force | Sort-Object Name)
        $names = @($entries | ForEach-Object { $_.Name })
        if ($names.Count -ne 1 -or $names[0] -ne $canaryName) {
            throw ("${Stage}: $view should hold only $canaryName and holds: " +
                $(if ($names.Count -gt 0) { $names -join ', ' } else { '(nothing)' }))
        }
        $hash = (Get-FileHash -LiteralPath (Join-Path $view $canaryName) -Algorithm SHA256).Hash
        if ($hash -ne $canaryHashes[$view]) {
            throw "${Stage}: the canary in $view is $hash and was $($canaryHashes[$view])."
        }
    }
}

# --- source policy helpers -------------------------------------------------
# Only the statements that actually run at load time count. A touch point inside
# a function body has not executed, and every call site of those functions is
# asserted to be after the guard separately.
function Test-IsInsideFunction {
    param([Parameter(Mandatory)]$Ast)
    $node = $Ast.Parent
    while ($null -ne $node) {
        if ($node -is [System.Management.Automation.Language.FunctionDefinitionAst]) { return $true }
        $node = $node.Parent
    }
    $false
}

function Get-ParsedScript {
    param([Parameter(Mandatory)][string]$Path)
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$null, [ref]$errors)
    if ($errors.Count -gt 0) {
        throw "$Path does not parse: $($errors[0].Message)"
    }
    $ast
}

# `$env:APPDATA`, `$env:LOCALAPPDATA`, an `HKCU:` path, or a process lookup:
# the first thing any of these four proofs does that reads or changes the machine.
function Get-LiveMachineTouchPoints {
    param([Parameter(Mandatory)]$Ast, [switch]$IncludeInsideFunctions)
    $ast.FindAll({
        param($node)
        if ($node -is [System.Management.Automation.Language.VariableExpressionAst]) {
            return $node.VariablePath.UserPath -in @('env:APPDATA', 'env:LOCALAPPDATA')
        }
        if ($node -is [System.Management.Automation.Language.CommandAst]) {
            return $node.GetCommandName() -eq 'Get-Process'
        }
        if ($node -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
            return $node.Value -like 'HKCU:*'
        }
        $false
    }, $true) | Where-Object { $IncludeInsideFunctions -or -not (Test-IsInsideFunction -Ast $_) }
}

function Get-GuardCalls {
    param([Parameter(Mandatory)]$Ast)
    $ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.CommandAst] -and
            $node.GetCommandName() -eq 'Assert-HostProfilePathIdentity'
    }, $true)
}

try {
    foreach ($view in @($viewA, $viewB, $elsewhere)) {
        New-Item -ItemType Directory -Path $view -Force | Out-Null
    }
    $scratchCreated = $true
    foreach ($view in @($viewA, $viewB, $elsewhere)) {
        $canary = Join-Path $view $canaryName
        Set-Content -LiteralPath $canary -Value "not this control's file: $view" -NoNewline -Encoding utf8
        $canaryHashes[$view] = (Get-FileHash -LiteralPath $canary -Algorithm SHA256).Hash
    }
    # A junction, so the two views are genuinely different path strings resolved
    # by the filesystem rather than two spellings .NET collapses before it opens
    # anything. That is the shape of the real pair: an ordinary path and an
    # administrative share reaching one object.
    New-Item -ItemType Junction -Path $viewALink -Target $viewA | Out-Null
    $linkCreated = $true

    Write-Host "scratch: $scratch"
    Write-Host "  view-a      : $viewA"
    Write-Host "  view-a-link : $viewALink (junction to view-a)"
    Write-Host "  view-b      : $viewB"
    Write-Host "  elsewhere   : $elsewhere"
    Write-Host "  canary      : $canaryName in each view"
    Assert-ViewsHoldOnlyTheirCanary -Stage 'before any case'

    Write-Host ''
    Write-Host 'CASE: two distinct paths that address the same directory'
    $result = Test-DirectoryViewIdentity -Label 'same-storage' -OrdinaryView $viewA -IndependentView $viewALink
    if (-not $result.FirstDirection -or -not $result.SecondDirection) {
        throw 'CASE same-storage: the check returned without proving both directions.'
    }
    if (-not $result.ProbeName.StartsWith('speakeasy-mini-path-identity-')) {
        throw "CASE same-storage: unexpected probe name $($result.ProbeName)."
    }
    # The name was generated, not supplied, so this also proves the generator's
    # file really was removed rather than a name the control chose.
    Write-Host "  passed both directions; probe $($result.ProbeName) is gone"
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE same-storage'
    $passed++

    Write-Host ''
    Write-Host 'CASE: two different directories'
    $message = Assert-Refused -Name 'different-directories' -MessageContains @(
        'first direction failed', 'do not address the same storage'
    ) -Action {
        Test-DirectoryViewIdentity -Label 'different-directories' `
            -OrdinaryView $viewA -IndependentView $viewB `
            -TestOnlyProbeName 'planted-probe-name.probe'
    }
    Write-Host "  refused: $message"
    # Cleanup after the refusal, asserted by exact name in every view it could
    # have reached, and then by the canary assertion below.
    foreach ($view in @($viewA, $viewB, $elsewhere)) {
        if (Test-Path -LiteralPath (Join-Path $view 'planted-probe-name.probe')) {
            throw "CASE different-directories: the probe survived in $view."
        }
    }
    Write-Host '  the probe is absent from every view'
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE different-directories'
    $passed++

    Write-Host ''
    Write-Host 'CASE: a one-way mirror the first direction has to catch'
    # The views are the same storage, so the second comparison would pass. Only
    # the first one can see that the write never arrived.
    $message = Assert-Refused -Name 'one-way-first' -MessageContains @('first direction failed') -Action {
        Test-DirectoryViewIdentity -Label 'one-way-first' `
            -OrdinaryView $viewA -IndependentView $viewALink `
            -TestOnlyProbeName 'planted-probe-name.probe' `
            -TestOnlyFirstDirectionWriteView $elsewhere
    }
    Write-Host "  refused: $message"
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE one-way-first'
    $passed++

    Write-Host ''
    Write-Host 'CASE: a one-way mirror the second direction has to catch'
    # The mirror image: the first comparison passes, because the ordinary write
    # really did reach the independent view. Only the second one sees that the
    # write coming back landed somewhere else.
    $message = Assert-Refused -Name 'one-way-second' -MessageContains @('second direction failed') -Action {
        Test-DirectoryViewIdentity -Label 'one-way-second' `
            -OrdinaryView $viewA -IndependentView $viewALink `
            -TestOnlyProbeName 'planted-probe-name.probe' `
            -TestOnlySecondDirectionWriteView $elsewhere
    }
    Write-Host "  refused: $message"
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE one-way-second'
    $passed++

    Write-Host ''
    Write-Host 'CASE: the probe name is already there'
    $planted = Join-Path $viewA 'planted-probe-name.probe'
    Set-Content -LiteralPath $planted -Value 'somebody else wrote this' -NoNewline -Encoding utf8
    $plantedHash = (Get-FileHash -LiteralPath $planted -Algorithm SHA256).Hash
    $message = Assert-Refused -Name 'name-collision' -MessageContains @('already exists') -Action {
        Test-DirectoryViewIdentity -Label 'name-collision' `
            -OrdinaryView $viewA -IndependentView $viewALink `
            -TestOnlyProbeName 'planted-probe-name.probe'
    }
    Write-Host "  refused: $message"
    if (-not (Test-Path -LiteralPath $planted -PathType Leaf)) {
        throw 'CASE name-collision: the refusal removed the file it refused over.'
    }
    if ((Get-FileHash -LiteralPath $planted -Algorithm SHA256).Hash -ne $plantedHash) {
        throw 'CASE name-collision: the refusal overwrote the file it refused over.'
    }
    Write-Host '  the file it refused over is byte-identical and still there'
    Remove-Item -LiteralPath $planted -Force
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE name-collision'
    $passed++

    Write-Host ''
    Write-Host 'CASE: the classifier answers about the exact path, and calls nothing Missing it cannot prove absent'
    $kinds = @(
        @{ Path = (Join-Path $viewA $canaryName); Expect = 'File' }
        @{ Path = $viewA; Expect = 'Directory' }
        @{ Path = $viewALink; Expect = 'ReparsePoint' }
        @{ Path = (Join-Path $viewA 'nothing-has-ever-been-here.txt'); Expect = 'Missing' }
    )
    foreach ($k in $kinds) {
        $actual = Get-ExactEntryKind -Path $k.Path
        if ($actual -ne $k.Expect) {
            throw "CASE classifier: $($k.Path) classified $actual, expected $($k.Expect)."
        }
        Write-Host "  $($k.Expect.PadRight(12)) $($k.Path)"
    }
    # `Missing` has to mean absent from a view that is still there. A not-found
    # does not say which of the two it is -- an administrative-share path that
    # stops resolving raises the same exception -- so the parent is inspected
    # before the answer is allowed to be `Missing`. The reachable-parent case is
    # the `Missing` row above.
    $unreachableParents = @(
        @{ Name = 'a parent that does not exist'
           Path = (Join-Path (Join-Path $viewA 'no-such-parent-directory') 'child.probe')
           Because = 'is not reachable' }
        @{ Name = 'a parent that is an ordinary file'
           Path = (Join-Path (Join-Path $viewA $canaryName) 'child.probe')
           Because = 'is not a directory' }
    )
    foreach ($u in $unreachableParents) {
        $message = Assert-Refused -Name "unreachable parent -- $($u.Name)" -MessageContains @(
            'Cannot classify', $u.Because
        ) -Action {
            Get-ExactEntryKind -Path $u.Path
        }
        Write-Host "  $($u.Name): $message"
    }
    # The defect this classifier replaces, stated as an assertion: a directory at
    # a probe-shaped name is a directory, and `File.Exists` calls it absent.
    $probeShaped = Join-Path $viewA 'kind-check.probe'
    New-Item -ItemType Directory -Path $probeShaped | Out-Null
    if ((Get-ExactEntryKind -Path $probeShaped) -ne 'Directory') {
        throw 'CASE classifier: a directory at a probe-shaped name did not classify as Directory.'
    }
    if ([IO.File]::Exists($probeShaped)) {
        throw 'CASE classifier: File.Exists answered true for a directory; the premise of this case is wrong.'
    }
    Write-Host '  a directory at a probe-shaped name classifies Directory, where File.Exists answers false'
    [IO.Directory]::Delete($probeShaped)
    # Unclassifiable rather than absent. An embedded NUL cannot be normalised into
    # a path, so the filesystem is never asked and "not found" was never the
    # answer -- exactly the case that must not be folded into Missing.
    $malformed = $viewA + [IO.Path]::DirectorySeparatorChar + 'bad' + [char]0 + 'name.probe'
    $message = Assert-Refused -Name 'unclassifiable path' -MessageContains @('Cannot classify') -Action {
        Get-ExactEntryKind -Path $malformed
    }
    Write-Host "  refused: $message"
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE classifier'
    $passed++

    Write-Host ''
    Write-Host 'CASE: an entry of the wrong kind at the probe name is refused, not overwritten'
    $plantedDirectory = Join-Path $viewA 'planted-probe-name.probe'
    $insideName = 'operator-put-this-here.txt'
    $insideText = 'this file is inside the substituted directory'
    New-Item -ItemType Directory -Path $plantedDirectory | Out-Null
    Set-Content -LiteralPath (Join-Path $plantedDirectory $insideName) -Value $insideText -NoNewline -Encoding utf8
    $message = Assert-Refused -Name 'directory-collision' -MessageContains @(
        'already exists', 'as a Directory'
    ) -Action {
        Test-DirectoryViewIdentity -Label 'directory-collision' `
            -OrdinaryView $viewA -IndependentView $viewALink `
            -TestOnlyProbeName 'planted-probe-name.probe'
    }
    Write-Host "  refused: $message"
    if ((Get-ExactEntryKind -Path $plantedDirectory) -ne 'Directory') {
        throw 'CASE directory-collision: the refusal did not leave the directory it refused over.'
    }
    if ([IO.File]::ReadAllText((Join-Path $plantedDirectory $insideName)) -ne $insideText) {
        throw 'CASE directory-collision: the file inside the directory changed.'
    }
    Write-Host '  the directory and the file inside it are exactly as they were'
    # By hand, by exact name, and not through the code under test.
    Remove-Item -LiteralPath (Join-Path $plantedDirectory $insideName) -Force
    [IO.Directory]::Delete($plantedDirectory)
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE directory-collision'
    $passed++

    Write-Host ''
    Write-Host 'CASE: a directory stands at the probe name when cleanup runs'
    # The probe itself succeeds -- the two views are the same storage -- so the
    # refusal below is the cleanup's alone. `File.Exists` answered false for the
    # substituted directory and reported the probe removed; that is the defect.
    $message = Assert-Refused -Name 'directory-at-cleanup' -MessageContains @(
        'cleanup:', 'is a Directory and was left in place', 'cannot be accounted for'
    ) -Action {
        Test-DirectoryViewIdentity -Label 'directory-at-cleanup' `
            -OrdinaryView $viewA -IndependentView $viewALink `
            -TestOnlyProbeName 'planted-probe-name.probe' `
            -TestOnlyBeforeCleanup {
                param($ProbePath)
                [IO.File]::Delete($ProbePath)
                New-Item -ItemType Directory -Path $ProbePath | Out-Null
                Set-Content -LiteralPath (Join-Path $ProbePath $insideName) -Value $insideText -NoNewline -Encoding utf8
            }
    }
    Write-Host "  refused: $message"
    if ((Get-ExactEntryKind -Path $plantedDirectory) -ne 'Directory') {
        throw 'CASE directory-at-cleanup: cleanup did not leave the substituted directory in place.'
    }
    if ([IO.File]::ReadAllText((Join-Path $plantedDirectory $insideName)) -ne $insideText) {
        throw 'CASE directory-at-cleanup: the file inside the substituted directory changed.'
    }
    Write-Host '  the substituted directory and the file inside it survive untouched'
    Remove-Item -LiteralPath (Join-Path $plantedDirectory $insideName) -Force
    [IO.Directory]::Delete($plantedDirectory)
    Assert-ViewsHoldOnlyTheirCanary -Stage 'CASE directory-at-cleanup'
    $passed++

    Write-Host ''
    Write-Host 'CASE: a local drive path maps to its administrative share'
    $mappings = @(
        @{ In = 'C:\some\path'; Out = '\\localhost\C$\some\path' }
        @{ In = 'D:\x\y'; Out = '\\localhost\D$\x\y' }
        # The drive is read from the path, not assumed, and normalised upwards so
        # the share name is the one the SMB server publishes.
        @{ In = 'e:\Users\someone\AppData\Roaming'; Out = '\\localhost\E$\Users\someone\AppData\Roaming' }
        # A trailing separator, and a `..` the caller left in: both collapse
        # before the share name is built.
        @{ In = 'C:\a\b\'; Out = '\\localhost\C$\a\b' }
        @{ In = 'C:\a\b\..\c'; Out = '\\localhost\C$\a\c' }
        @{ In = 'C:\'; Out = '\\localhost\C$\' }
    )
    foreach ($mapping in $mappings) {
        $actual = Get-AdminSharePath -Path $mapping.In
        if ($actual -ne $mapping.Out) {
            throw "CASE mapping: $($mapping.In) mapped to $actual, expected $($mapping.Out)."
        }
        Write-Host "  $($mapping.In) -> $actual"
    }
    $passed++

    Write-Host ''
    Write-Host 'CASE: a path that cannot be mapped unambiguously is refused'
    $unmappable = @(
        @{ In = ''; Because = 'empty' }
        @{ In = '   '; Because = 'empty' }
        @{ In = 'relative\path'; Because = 'not fully qualified' }
        @{ In = '..\up\one'; Because = 'not fully qualified' }
        @{ In = 'C:relative'; Because = 'not fully qualified' }
        @{ In = '\path\without\a\drive'; Because = 'not fully qualified' }
        @{ In = '\\server\share\path'; Because = 'UNC or device path' }
        @{ In = '\\?\C:\device\path'; Because = 'UNC or device path' }
        @{ In = '//server/share'; Because = 'UNC or device path' }
    )
    foreach ($case in $unmappable) {
        $message = Assert-Refused -Name "unmappable '$($case.In)'" -MessageContains @($case.Because) -Action {
            Get-AdminSharePath -Path $case.In
        }
        Write-Host "  '$($case.In)' refused: $message"
    }
    $passed++

    Write-Host ''
    Write-Host 'CASE: the four live-profile proofs call the guard before they touch the machine'
    $gated = @(
        'Test-SetupWizard.ps1'
        'Test-PreflightRefusalIsInert.ps1'
        'Test-ProfileRestoreOnAbort.ps1'
        'Test-CleanupFailureRestoresConfig.ps1'
    )
    foreach ($name in $gated) {
        $path = Join-Path $PSScriptRoot $name
        $ast = Get-ParsedScript -Path $path
        $guards = @(Get-GuardCalls -Ast $ast | Where-Object { -not (Test-IsInsideFunction -Ast $_) })
        if ($guards.Count -eq 0) {
            throw "$name never calls Assert-HostProfilePathIdentity outside a function."
        }
        $guardLine = ($guards | ForEach-Object { $_.Extent.StartLineNumber } | Sort-Object)[0]

        # The dot-source has to come first or the call is an unknown command.
        $dotSource = @($ast.FindAll({
            param($node)
            $node -is [System.Management.Automation.Language.StringConstantExpressionAst] -and
                $node.Value -eq 'HostProfilePathIdentity.ps1'
        }, $true))
        if ($dotSource.Count -eq 0) {
            throw "$name calls the guard without dot-sourcing HostProfilePathIdentity.ps1."
        }
        if (($dotSource | ForEach-Object { $_.Extent.StartLineNumber } | Sort-Object)[0] -ge $guardLine) {
            throw "$name dot-sources HostProfilePathIdentity.ps1 after it calls the guard."
        }

        $touches = @(Get-LiveMachineTouchPoints -Ast $ast)
        if ($touches.Count -eq 0) {
            throw "$name reads no live machine state, so gating it proves nothing; the policy is stale."
        }
        $firstTouch = ($touches | ForEach-Object { $_.Extent.StartLineNumber } | Sort-Object)[0]
        if ($guardLine -ge $firstTouch) {
            throw ("$name touches live machine state at line $firstTouch and calls the guard at line " +
                "$guardLine; the guard must come first.")
        }

        # A touch point inside a function body was excluded above, so this closes
        # the loophole it opens: no function that reads live machine state may be
        # invoked before the guard.
        $liveFunctions = @($ast.FindAll({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
        }, $true) | Where-Object {
            @(Get-LiveMachineTouchPoints -Ast $_.Body -IncludeInsideFunctions).Count -gt 0
        } | ForEach-Object { $_.Name })
        $earlyCalls = @($ast.FindAll({
            param($node)
            $node -is [System.Management.Automation.Language.CommandAst]
        }, $true) | Where-Object {
            -not (Test-IsInsideFunction -Ast $_) -and
            $_.Extent.StartLineNumber -lt $guardLine -and
            $_.GetCommandName() -in $liveFunctions
        })
        if ($earlyCalls.Count -gt 0) {
            $where = ($earlyCalls | ForEach-Object { "$($_.GetCommandName()) at line $($_.Extent.StartLineNumber)" }) -join ', '
            throw "$name calls a function that reads live machine state before the guard: $where"
        }

        Write-Host "  $name : guard at line $guardLine, first live touch at line $firstTouch"
    }

    # The helper-only mode of the cleanup control has to stay runnable where the
    # profile is redirected, so its guard belongs inside the `-ArtifactRoot`
    # branch and nowhere else.
    $cleanupAst = Get-ParsedScript -Path (Join-Path $PSScriptRoot 'Test-CleanupFailureRestoresConfig.ps1')
    foreach ($guard in (Get-GuardCalls -Ast $cleanupAst)) {
        $node = $guard.Parent
        $guarded = $false
        while ($null -ne $node) {
            if ($node -is [System.Management.Automation.Language.IfStatementAst]) {
                foreach ($clause in $node.Clauses) {
                    if ($clause.Item1.Extent.Text -like '*ArtifactRoot*') { $guarded = $true }
                }
            }
            $node = $node.Parent
        }
        if (-not $guarded) {
            throw ('Test-CleanupFailureRestoresConfig.ps1 calls the guard outside an -ArtifactRoot ' +
                'branch, so its helper-only mode would need a host.')
        }
    }
    Write-Host '  Test-CleanupFailureRestoresConfig.ps1 : the guard is inside the -ArtifactRoot branch only'
    $passed++

    Write-Host ''
    Write-Host 'CASE: the guard is not wired into checks that must run without a host'
    $ungated = @(
        'Test-ProfileCaptureIsScoped.ps1'
        'Test-BuildRootContainment.ps1'
        'Test-DeleteContainment.ps1'
        'Test-HostProfilePathIdentityIsScoped.ps1'
        # It redirects both profile roots under `target\` on purpose, so the real
        # roots are not what it uses and proving them would be beside the point.
        'Test-InstallerLifecycle.ps1'
    )
    foreach ($name in $ungated) {
        $path = Join-Path $PSScriptRoot $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "$name is missing, so this policy no longer describes the repository."
        }
        $calls = @(Get-GuardCalls -Ast (Get-ParsedScript -Path $path))
        if ($calls.Count -gt 0) {
            throw "$name calls the guard at line $($calls[0].Extent.StartLineNumber); it must run without a host."
        }
        Write-Host "  $name : ungated, as it must be"
    }
    $passed++

    Write-Host ''
    Write-Host 'CASE: the test seams cannot be reached from the real-host entry point'
    $entryPath = Join-Path $PSScriptRoot 'Test-HostProfilePathIdentity.ps1'
    $entryAst = Get-ParsedScript -Path $entryPath
    $entryParameters = @($entryAst.ParamBlock.Parameters | ForEach-Object { $_.Name.VariablePath.UserPath })
    if ($entryParameters.Count -ne 0) {
        throw "Test-HostProfilePathIdentity.ps1 takes parameters ($($entryParameters -join ', ')); it must take none."
    }
    if ((Get-Content -LiteralPath $entryPath -Raw) -match 'TestOnly') {
        throw 'Test-HostProfilePathIdentity.ps1 mentions a test seam.'
    }
    $sharedAst = Get-ParsedScript -Path (Join-Path $PSScriptRoot 'HostProfilePathIdentity.ps1')
    $guardDefinition = @($sharedAst.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -eq 'Assert-HostProfilePathIdentity'
    }, $true))
    if ($guardDefinition.Count -ne 1) {
        throw 'HostProfilePathIdentity.ps1 does not define Assert-HostProfilePathIdentity exactly once.'
    }
    $guardParameters = @($guardDefinition[0].Body.ParamBlock.Parameters |
        ForEach-Object { $_.Name.VariablePath.UserPath })
    $seams = @($guardParameters | Where-Object { $_ -like 'TestOnly*' })
    if ($seams.Count -gt 0) {
        throw "Assert-HostProfilePathIdentity exposes a test seam: $($seams -join ', ')"
    }
    Write-Host "  Test-HostProfilePathIdentity.ps1 : no parameters, no seam"
    Write-Host "  Assert-HostProfilePathIdentity   : $($guardParameters -join ', ') -- no seam"
    $passed++

    Write-Host ''
    Write-Host "host profile path identity is scoped: passed ($passed cases)"
}
finally {
    # The junction first, and with the .NET call rather than `Remove-Item`:
    # deleting a link is what is wanted, and a recursive delete that followed it
    # would take view-a's contents with it.
    if ($linkCreated -and (Test-Path -LiteralPath $viewALink)) {
        [IO.Directory]::Delete($viewALink)
        if (-not (Test-Path -LiteralPath $viewA -PathType Container)) {
            throw "Removing the junction took its target with it: $viewA is gone."
        }
        Write-Host "removed the junction; view-a is still there: $(Test-Path -LiteralPath $viewA)"
    }
    if ($scratchCreated) {
        [void](Remove-ContainedDirectory -Path $scratch -Root $targetRoot -Label "This control's scratch directory")
    }
}
