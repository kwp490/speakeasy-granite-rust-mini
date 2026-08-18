[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$githubRoot = Join-Path $repositoryRoot '.github'

# SpeakEasy is intentionally a local-only project. A checked-in workflow,
# Dependabot configuration, or other .github automation would move builds,
# tests, or dependency changes back onto GitHub-hosted infrastructure.
if (Test-Path -LiteralPath $githubRoot) {
    $files = @(Get-ChildItem -LiteralPath $githubRoot -Recurse -File -Force)
    if ($files.Count -gt 0) {
        throw ('Local-only policy failed: .github automation/configuration is not allowed. ' +
            'Remove these files and run the local scripts instead: ' +
            ($files.FullName -join ', '))
    }
}

Write-Host 'Local-only policy: passed (no GitHub workflows, runners, or automation configured)'
