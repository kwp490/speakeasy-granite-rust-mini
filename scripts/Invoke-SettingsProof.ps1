<#
.SYNOPSIS
Walks every page of the running settings workspace and reports the layout facts
docs/UI-GUIDE.md's responsive rules state: no horizontal scrolling, one scroll
region, and no element wider than the window.

.DESCRIPTION
This measures rather than eyeballs. The first version read `WS_HSCROLL` from the
window's child HWNDs and reported a horizontal scrollbar on all five pages *and*
on the transcriber, which has `overflow: hidden` and cannot scroll —
`Chrome_RenderWidgetHostHWND` carries both scrollbar styles unconditionally. It
measured nothing while looking like it had found five bugs. Numbers now come from
the document, through `Invoke-WebviewProbe.ps1`.

Pages are selected **by selector**, so a click either lands on the real nav-rail
button or fails loudly. The pixel-offset harness's worst failure mode — an offset
that has drifted onto the neighbouring control, indistinguishable from a broken
control — is not reachable here.

Widths are applied with `Emulation.setDeviceMetricsOverride`, which resizes the
viewport without touching the OS window, so the same run can check the 720 px
minimum, the 760 px point where the nav rail is meant to become a row, and a wide
desktop. The override is cleared afterwards.

Requires the app running with the WebView2 debugging port open:

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    npm run tauri -- dev

.EXAMPLE
./scripts/Invoke-SettingsProof.ps1

.EXAMPLE
./scripts/Invoke-SettingsProof.ps1 -Widths 720,1400
#>
[CmdletBinding()]
param(
    [int[]]$Widths = @(720, 880, 1200),
    [int]$Height = 700,
    [int]$Port = 9222
)

$ErrorActionPreference = 'Stop'

$probe = Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1'
$pages = @('general', 'audio', 'transcription', 'output', 'log', 'advanced')

# Every number describes the same frame, so they cannot disagree with each other.
$measure = @'
JSON.stringify((() => {
  const root = document.documentElement;
  const overflowing = [...document.querySelectorAll('*')]
    .filter((element) => element.getBoundingClientRect().right > root.clientWidth + 1)
    .slice(0, 6)
    .map((element) => {
      const id = element.id === '' ? '' : '#' + element.id;
      const cls = element.className === '' ? '' : '.' + String(element.className).split(' ')[0];
      return element.tagName.toLowerCase() + id + cls +
        ' right=' + Math.round(element.getBoundingClientRect().right);
    });
  const scrollers = [...document.querySelectorAll('*')].filter((element) => {
    const overflowY = getComputedStyle(element).overflowY;
    return (overflowY === 'auto' || overflowY === 'scroll') &&
      element.scrollHeight > element.clientHeight + 1;
  }).length;
  const active = [...document.querySelectorAll('[role="tab"]')]
    .find((tab) => tab.getAttribute('aria-selected') === 'true');
  const rail = document.querySelector('.settings-rail');
  const body = document.querySelector('.settings-body');
  return {
    page: active === undefined ? null : active.id,
    clientWidth: root.clientWidth,
    scrollWidth: root.scrollWidth,
    overflow: root.scrollWidth - root.clientWidth,
    culprits: overflowing,
    nestedScrollers: scrollers,
    railIsRow: rail === null || body === null ? null :
      getComputedStyle(body).gridTemplateColumns.trim().split(/\s+/).length === 1,
  };
})())
'@

function Set-Viewport {
    param([int]$Width, [int]$ViewportHeight)
    & $probe -Window settings -Port $Port -Cdp 'Emulation.setDeviceMetricsOverride' `
        -CdpParams "{`"width`": $Width, `"height`": $ViewportHeight, `"deviceScaleFactor`": 0, `"mobile`": false}" | Out-Null
}

function Clear-Viewport {
    & $probe -Window settings -Port $Port -Cdp 'Emulation.clearDeviceMetricsOverride' -CdpParams '{}' | Out-Null
}

$failures = @()

try {
    foreach ($width in $Widths) {
        Set-Viewport -Width $width -ViewportHeight $Height
        Start-Sleep -Milliseconds 400
        Write-Host ''
        Write-Host "=== settings viewport ${width}x${Height} CSS px ===" -ForegroundColor Cyan

        foreach ($page in $pages) {
            $facts = (& $probe -Window settings -Port $Port -Click "#settings-tab-$page" -Expression $measure |
                    ConvertFrom-Json) | ConvertFrom-Json

            $verdict = if ($facts.overflow -gt 0) { 'OVERFLOWS' } else { 'ok' }
            $railShape = if ($facts.railIsRow) { 'row' } else { 'rail' }
            Write-Host ("  {0,-14} {1,-10} client={2,-5} scroll={3,-5} overflow={4,-4} scrollRegions={5} nav={6}" -f `
                    $page, $verdict, $facts.clientWidth, $facts.scrollWidth, $facts.overflow,
                $facts.nestedScrollers, $railShape)

            if ($facts.page -ne "settings-tab-$page") {
                $failures += "${width}px/${page}: did not become the active page (got '$($facts.page)')"
            }
            if ($facts.overflow -gt 0) {
                $failures += "${width}px/${page}: overflows by $($facts.overflow)px"
                foreach ($culprit in $facts.culprits) { Write-Host "      wider than the viewport: $culprit" }
            }
            if ($facts.nestedScrollers -gt 1) {
                $failures += "${width}px/${page}: $($facts.nestedScrollers) scroll regions; only one is allowed"
            }
            # Below 760 px the rail must become a row rather than a scroll trap.
            $expectRow = $width -lt 760
            if ($null -ne $facts.railIsRow -and $facts.railIsRow -ne $expectRow) {
                $failures += "${width}px/${page}: nav is a $railShape; expected $(if ($expectRow) { 'row' } else { 'rail' })"
            }
        }
    }
} finally {
    Clear-Viewport
}

Write-Host ''
if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Warning $failure }
    exit 1
}
Write-Host 'Every page at every width: no horizontal overflow, one scroll region, nav shape correct.' -ForegroundColor Green
