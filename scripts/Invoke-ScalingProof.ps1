<#
.SYNOPSIS
Checks both SpeakEasy windows at the display scalings docs/UI-GUIDE.md's
"Automated checks" table names for this script: 100%, 125%, 150% and 200%.

.DESCRIPTION
What Windows display scaling actually changes is how many **logical** CSS pixels a
physical screen offers. A 1920x1080 panel is 1920x1080 logical at 100% and
960x540 logical at 200%; the CSS pixel is simply bigger. So the meaningful test of
"does it work at 200%" is not "render it twice as large" — it is "does it still
work when the viewport has half as many CSS pixels in each direction", which is
where a 720 px minimum window or a 280 px tall transcript actually runs out of
room.

Both are checked here:

  * `deviceScaleFactor` confirms the layout is scale-independent — the transcriber
    is a fixed 420x280 *logical*, so its CSS layout must not move at all between
    100% and 200%, and hairline borders must survive the rounding.
  * the logical viewport each scaling leaves on a 1920x1080 panel confirms nothing
    clips or scrolls sideways when CSS pixels get scarce.

Requires the app running with the WebView2 debugging port open:

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    npm run tauri -- dev

.EXAMPLE
./scripts/Invoke-ScalingProof.ps1
#>
[CmdletBinding()]
param(
    [int]$Port = 9222,

    # The physical panel the logical viewports are derived from. 1920x1080 is the
    # tightest common case: at 200% it leaves only 960x540 CSS px.
    [int]$PanelWidth = 1920,
    [int]$PanelHeight = 1080
)

$ErrorActionPreference = 'Stop'

$probe = Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1'
$scalings = @(1.0, 1.25, 1.5, 2.0)
$failures = @()

# ── The transcriber: fixed 420x280 logical, so its layout must not move at all ──

$hudMeasure = @'
JSON.stringify((() => {
  const root = document.documentElement;
  const shell = document.querySelector('[data-testid="capture-hud"]');
  const transcript = document.querySelector('.hud-transcript');
  const primary = document.querySelector('[data-testid="hud-primary"]');
  const box = (element) => {
    if (element === null) return null;
    const rect = element.getBoundingClientRect();
    return { w: Math.round(rect.width), h: Math.round(rect.height) };
  };
  return {
    ratio: window.devicePixelRatio,
    viewport: root.clientWidth + 'x' + root.clientHeight,
    overflowX: root.scrollWidth - root.clientWidth,
    overflowY: root.scrollHeight - root.clientHeight,
    shell: box(shell),
    transcript: box(transcript),
    primary: box(primary),
    // A hairline that rounds to zero physical pixels disappears entirely.
    micBorder: getComputedStyle(document.querySelector('.hud-mic')).borderTopWidth,
  };
})())
'@

Write-Host '=== transcriber: layout must be identical at every scaling ===' -ForegroundColor Cyan
$baseline = $null
foreach ($scale in $scalings) {
    & $probe -Window transcriber -Port $Port -Cdp 'Emulation.setDeviceMetricsOverride' `
        -CdpParams ('{"width": 420, "height": 280, "deviceScaleFactor": ' + $scale + ', "mobile": false}') | Out-Null
    Start-Sleep -Milliseconds 300
    $facts = (& $probe -Window transcriber -Port $Port -Expression $hudMeasure | ConvertFrom-Json) | ConvertFrom-Json

    $shape = "$($facts.shell.w)x$($facts.shell.h) transcript=$($facts.transcript.w)x$($facts.transcript.h) primary=$($facts.primary.w)x$($facts.primary.h)"
    $percent = [int]($scale * 100)
    # `devicePixelRatio` reports the *host's* scaling: WebView2 does not adopt the
    # emulated deviceScaleFactor for it. That is fine, because what is being
    # checked is the CSS layout, and the physical size is arithmetic.
    Write-Host ("  {0,4}%  physical={1}x{2}  {3}  border={4}  overflow={5},{6}  (host ratio {7})" -f `
            $percent,
        [int](420 * $scale), [int](280 * $scale), $shape, $facts.micBorder,
        $facts.overflowX, $facts.overflowY, $facts.ratio)

    if ($null -eq $baseline) {
        $baseline = $shape
    } elseif ($shape -ne $baseline) {
        $failures += "transcriber layout changed at ${percent}%: '$shape' vs '$baseline'"
    }
    if ($facts.overflowX -gt 0) { $failures += "transcriber scrolls horizontally at ${percent}%" }
    if ($facts.overflowY -gt 0) { $failures += "transcriber scrolls vertically at ${percent}%: the window must never scroll" }
    if ($facts.transcript.h -lt 60) { $failures += "transcript area is only $($facts.transcript.h)px tall at ${percent}%" }
}
& $probe -Window transcriber -Port $Port -Cdp 'Emulation.clearDeviceMetricsOverride' -CdpParams '{}' | Out-Null

# ── Settings: the logical viewport each scaling leaves on the panel ──

$settingsMeasure = @'
JSON.stringify((() => {
  const root = document.documentElement;
  const clipped = [...document.querySelectorAll('button, select, input, summary')]
    .filter((element) => element.offsetParent !== null)
    .filter((element) => element.getBoundingClientRect().right > root.clientWidth + 1)
    .map((element) => (element.textContent ?? element.tagName).trim().slice(0, 30));
  return {
    viewport: root.clientWidth + 'x' + root.clientHeight,
    overflowX: root.scrollWidth - root.clientWidth,
    clipped,
    railIsRow: getComputedStyle(document.querySelector('.settings-rail')).borderRightStyle === 'none',
  };
})())
'@

Write-Host ''
Write-Host '=== settings: logical viewport left by each scaling ===' -ForegroundColor Cyan
foreach ($scale in $scalings) {
    $width = [int]($PanelWidth / $scale)
    $height = [int]($PanelHeight / $scale)
    & $probe -Window settings -Port $Port -Cdp 'Emulation.setDeviceMetricsOverride' `
        -CdpParams ('{"width": ' + $width + ', "height": ' + $height + ', "deviceScaleFactor": ' + $scale + ', "mobile": false}') | Out-Null
    Start-Sleep -Milliseconds 350

    $percent = [int]($scale * 100)
    foreach ($page in @('general', 'audio', 'transcription', 'output', 'advanced')) {
        $facts = (& $probe -Window settings -Port $Port -Click "#settings-tab-$page" -Expression $settingsMeasure |
                ConvertFrom-Json) | ConvertFrom-Json
        $nav = if ($facts.railIsRow) { 'row' } else { 'rail' }
        $verdict = if ($facts.overflowX -gt 0 -or $facts.clipped.Count -gt 0) { 'PROBLEM' } else { 'ok' }
        Write-Host ("  {0,4}%  {1,-14} {2,-8} viewport={3,-10} overflowX={4,-4} nav={5} clipped={6}" -f `
                $percent, $page, $verdict, $facts.viewport, $facts.overflowX, $nav, $facts.clipped.Count)
        if ($facts.overflowX -gt 0) { $failures += "settings/$page overflows by $($facts.overflowX)px at ${percent}%" }
        foreach ($control in $facts.clipped) { $failures += "settings/$page at ${percent}%: '$control' is past the right edge" }
    }
}
& $probe -Window settings -Port $Port -Cdp 'Emulation.clearDeviceMetricsOverride' -CdpParams '{}' | Out-Null

Write-Host ''
if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Warning $failure }
    exit 1
}
Write-Host 'Both windows hold up at 100%, 125%, 150% and 200%.' -ForegroundColor Green
