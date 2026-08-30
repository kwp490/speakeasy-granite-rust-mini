<#
.SYNOPSIS
    Asserts the pinned line-coverage floors in dependency-policy/coverage-floors.json.

.DESCRIPTION
    A floor per file, over the modules that carry privacy, delivery and mutation
    behaviour. The gate reported a workspace percentage and enforced nothing --
    "Rust coverage (no threshold)" -- which is a measurement rather than a
    guarantee, and a number nobody can fail is a number nobody reads.

    Two rules, and the second one is the point.

    A named file below its floor fails. A named file **absent from the report**
    also fails, and says so differently: a floor over something nobody measured
    guards nothing, and it is the failure mode a coverage check reaches on its
    own -- rename a file, or narrow a runner's include list, and every floor over
    it silently stops applying while the check goes on printing a pass. That is
    the same shape as `cargo test --lib` skipping the `--bin` targets.

    Headroom is printed on every run so raising a floor stays a deliberate act
    with a number in front of it. It does not fail: a self-tightening ratchet
    goes red on work that had nothing to do with it, and a check that fails for
    reasons the author cannot act on is a check that gets commented out.

.PARAMETER RepositoryRoot
    Defaults to the parent of this script's directory.
#>
[CmdletBinding()]
param(
    [string]$RepositoryRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

$policyPath = Join-Path $RepositoryRoot 'dependency-policy/coverage-floors.json'
if (-not (Test-Path -LiteralPath $policyPath)) {
    throw "Coverage floors are missing: $policyPath"
}
$policy = Get-Content -Raw -Encoding utf8 $policyPath | ConvertFrom-Json

# Both reports key their files by absolute path with the platform's separator.
# Normalised to repository-relative forward slashes so the policy file reads the
# way the repository does.
$root = ((Resolve-Path -LiteralPath $RepositoryRoot).Path.TrimEnd('\', '/')) + [IO.Path]::DirectorySeparatorChar
function ConvertTo-RelativePath([string]$path) {
    $full = [IO.Path]::GetFullPath($path)
    if ($full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        $full = $full.Substring($root.Length)
    }
    return $full.Replace('\', '/')
}

<#
Reads the llvm-cov JSON export into a relative-path -> line-percent map.
#>
function Get-RustCoverage([string]$path) {
    $report = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
    $measured = @{}
    foreach ($file in $report.data[0].files) {
        $measured[(ConvertTo-RelativePath $file.filename)] = [double]$file.summary.lines.percent
    }
    return $measured
}

<#
Reads vitest's `json-summary` reporter output into the same shape. Its `total`
key is the whole-run figure and is deliberately not a file.
#>
function Get-FrontendCoverage([string]$path) {
    $report = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
    $measured = @{}
    foreach ($entry in $report.PSObject.Properties) {
        if ($entry.Name -eq 'total') { continue }
        $measured[(ConvertTo-RelativePath $entry.Name)] = [double]$entry.Value.lines.pct
    }
    return $measured
}

$failures = New-Object System.Collections.Generic.List[string]
$raisable = New-Object System.Collections.Generic.List[string]

foreach ($ecosystem in @('rust', 'frontend')) {
    $section = $policy.$ecosystem
    $reportPath = Join-Path $RepositoryRoot $section.source
    if (-not (Test-Path -LiteralPath $reportPath)) {
        throw ("No $ecosystem coverage report at $reportPath. " +
            'Run the quality gate, which produces both before calling this.')
    }
    $measured = if ($ecosystem -eq 'rust') {
        Get-RustCoverage $reportPath
    } else {
        Get-FrontendCoverage $reportPath
    }

    # An empty report is the other way this check passes without checking. It
    # happens for real: a runner whose include list matches nothing writes a
    # valid summary with one `total` key in it.
    if ($measured.Count -eq 0) {
        throw "The $ecosystem coverage report at $reportPath names no files."
    }

    foreach ($entry in $section.files.PSObject.Properties) {
        $file = $entry.Name
        $floor = [double]$entry.Value
        if (-not $measured.ContainsKey($file)) {
            $failures.Add("$file has a floor of $floor% and does not appear in the $ecosystem coverage report at all")
            continue
        }
        $actual = [math]::Round($measured[$file], 2)
        if ($actual -lt $floor) {
            $failures.Add("$file is at $actual% line coverage, below its pinned floor of $floor%")
        } else {
            Write-Host ("  {0,-52} {1,6:N2}%  floor {2,3}%  (+{3:N2})" -f $file, $actual, $floor, ($actual - $floor))
            if (($actual - $floor) -ge 5) {
                $raisable.Add("$file is $([math]::Round($actual - $floor, 2)) points above its floor; consider pinning $([math]::Floor($actual))")
            }
        }
    }
}

if ($raisable.Count -gt 0) {
    Write-Host ''
    Write-Host 'Floors that could be raised (not a failure):'
    foreach ($line in $raisable) { Write-Host "  $line" }
}

if ($failures.Count -gt 0) {
    Write-Host ''
    foreach ($line in $failures) { Write-Host "COVERAGE FLOOR: $line" }
    throw "$($failures.Count) coverage floor(s) not met."
}

Write-Host 'Coverage floors: passed.'
