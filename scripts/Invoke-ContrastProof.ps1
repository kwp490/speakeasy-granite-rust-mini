<#
.SYNOPSIS
Measures rendered text contrast in both SpeakEasy windows against WCAG AA.

.DESCRIPTION
UI-GUIDE.md states that the light and dark palettes meet WCAG AA for normal text
and UI boundaries. That is a number, and it was not true: the transcriber's
primary button used `--accent`, which flips to a light coral under a dark system
theme, against near-white text — 1.9:1. The transcriber's surface never follows
the system theme, so its accent must not either, and it now has its own tokens.

This reads the **computed** colours out of the live document and computes the
ratios, rather than reasoning about the stylesheet. Elements are resolved against
their nearest painted ancestor background, which is what a reader actually sees.

Thresholds are WCAG AA: 4.5:1 for normal text, 3.0:1 for text at 18.66px bold or
24px and above, and 3.0:1 for the borders of interactive controls.

Requires the app running with the WebView2 debugging port open:

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    npm run tauri -- dev

.EXAMPLE
./scripts/Invoke-ContrastProof.ps1

.EXAMPLE
./scripts/Invoke-ContrastProof.ps1 -Window transcriber
#>
[CmdletBinding()]
param(
    [ValidateSet('settings', 'transcriber', 'both')]
    [string]$Window = 'both',

    # UI-GUIDE claims AA for *both* palettes, so both are measured. The scheme is
    # emulated rather than requiring the host theme to be switched between runs.
    [ValidateSet('light', 'dark', 'both')]
    [string]$Scheme = 'both',

    [int]$Port = 9222,

    # The `--surface` value each palette must resolve to. Kept as parameters so a
    # token change fails this check loudly instead of being absorbed by it.
    [string]$LightSurface = '#f5f7f2',

    [string]$DarkSurface = '#121915'
)

$ErrorActionPreference = 'Stop'

$probe = Join-Path $PSScriptRoot 'Invoke-WebviewProbe.ps1'

$measure = @'
JSON.stringify((() => {
  const parse = (value) => {
    const parts = value.match(/[\d.]+/g);
    if (parts === null) return null;
    const numbers = parts.map(Number);
    // Chromium serialises a computed `color-mix()` as `color(srgb r g b)` with
    // channels in 0..1, not 0..255. Read as 0..255 those came out near-black and
    // produced 90-odd bogus failures against backgrounds that are actually
    // near-white — the numbers were wrong, not the colours.
    const scale = value.startsWith('color(') ? 255 : 1;
    const [r, g, b, a] = numbers;
    if (a !== undefined && a === 0) return null;
    return [r * scale, g * scale, b * scale];
  };
  const channel = (value) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };
  const luminance = ([r, g, b]) =>
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  const ratio = (a, b) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  // What a reader actually sees behind this element: the nearest ancestor that
  // paints an opaque background.
  const backdrop = (element) => {
    for (let node = element; node !== null; node = node.parentElement) {
      const colour = parse(getComputedStyle(node).backgroundColor);
      if (colour !== null) return colour;
    }
    return [255, 255, 255];
  };

  const findings = [];
  for (const element of document.querySelectorAll('*')) {
    if (element.offsetParent === null && element !== document.body) continue;
    const text = [...element.childNodes]
      .filter((node) => node.nodeType === 3)
      .map((node) => node.textContent.trim())
      .join('');
    const style = getComputedStyle(element);
    const size = parseFloat(style.fontSize);
    const bold = Number(style.fontWeight) >= 700;
    const large = size >= 24 || (bold && size >= 18.66);
    const back = backdrop(element);

    if (text !== '') {
      const front = parse(style.color);
      if (front !== null) {
        const value = ratio(front, back);
        const need = large ? 3 : 4.5;
        if (value < need) {
          findings.push({
            kind: 'text',
            where: element.tagName.toLowerCase() +
              (element.className === '' ? '' : '.' + String(element.className).split(' ')[0]),
            sample: text.slice(0, 44),
            ratio: Math.round(value * 100) / 100,
            need,
            colour: style.color,
            behind: 'rgb(' + back.join(',') + ')',
          });
        }
      }
    }

    // Interactive borders carry state, so they have their own 3:1 floor.
    if (/^(BUTTON|SELECT|INPUT|TEXTAREA)$/.test(element.tagName) && element.disabled !== true) {
      const edge = parse(style.borderTopColor);
      if (edge !== null && parseFloat(style.borderTopWidth) > 0) {
        const value = ratio(edge, backdrop(element.parentElement ?? element));
        if (value < 3) {
          findings.push({
            kind: 'border',
            where: element.tagName.toLowerCase() +
              (element.className === '' ? '' : '.' + String(element.className).split(' ')[0]),
            sample: (element.textContent ?? '').trim().slice(0, 44),
            ratio: Math.round(value * 100) / 100,
            need: 3,
            colour: style.borderTopColor,
            behind: 'rgb(' + backdrop(element.parentElement ?? element).join(',') + ')',
          });
        }
      }
    }
  }
  return {
    surface: getComputedStyle(document.documentElement).getPropertyValue('--surface').trim(),
    findings,
  };
})())
'@

$windows = if ($Window -eq 'both') { @('transcriber', 'settings') } else { @($Window) }
$schemes = if ($Scheme -eq 'both') { @('light', 'dark') } else { @($Scheme) }
$total = 0
# A run that measures nothing prints "meets WCAG AA" and exits 0, which is
# indistinguishable from a clean pass. That happened once here, when `-Scheme` was
# referenced before it existed as a parameter and the inner loop ran zero times.
# Every measurement increments this, and a run that ends at zero fails.
$measured = 0
$mismatched = @()

<#
Two ways of switching palette were tried and neither works from outside the app.

`Emulation.setEmulatedMedia` is scoped to the CDP session that sets it, and the
probe opens a socket per call, so the override is gone before the measurement
runs. Changing the host theme (`AppsUseLightTheme`) does not help either: a
running WebView2 kept reporting dark after the switch.

Both failed *silently* — the first version of this script reported the light
palette as clean while measuring dark twice.

So the light palette is rendered by applying its own token values inline on
`:root`, which outranks the dark media block. That is the real DOM with the real
light values, measured the same way as dark; the values are read out of the
stylesheet's own `:root` rule rather than duplicated here, so they cannot drift
from the product. Every measurement reports the live `--surface`, and a run that
did not get the palette it asked for fails.
#>
$applyLightTokens = @'
JSON.stringify((() => {
  // The light palette is the `:root` rule; dark lives in a media block that
  // inline styles outrank.
  for (const sheet of document.styleSheets) {
    let rules;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const rule of rules) {
      if (rule.selectorText !== ':root' || rule.style === undefined) continue;
      const applied = [];
      for (const name of rule.style) {
        if (!name.startsWith('--')) continue;
        document.documentElement.style.setProperty(name, rule.style.getPropertyValue(name).trim());
        applied.push(name);
      }
      return { applied: applied.length };
    }
  }
  return { applied: 0 };
})())
'@

$clearForcedTokens = @'
JSON.stringify((() => {
  document.documentElement.removeAttribute('style');
  return { cleared: true };
})())
'@

function Set-Scheme {
    param([string]$Target, [string]$Value)
    if ($Value -eq 'light') {
        $applied = ((& $probe -Window $Target -Port $Port -Expression $applyLightTokens |
                    ConvertFrom-Json) | ConvertFrom-Json).applied
        if ($applied -eq 0) { throw 'Could not read the light palette out of the stylesheet.' }
    } else {
        & $probe -Window $Target -Port $Port -Expression $clearForcedTokens | Out-Null
    }
}

function Clear-Scheme {
    param([string]$Target)
    & $probe -Window $Target -Port $Port -Expression $clearForcedTokens | Out-Null
}

foreach ($name in $windows) {
  foreach ($scheme in $schemes) {
    Set-Scheme -Target $name -Value $scheme
    Start-Sleep -Milliseconds 400
    $expectedSurface = if ($scheme -eq 'light') { $LightSurface } else { $DarkSurface }
    Write-Host ''
    Write-Host "=== $name / $scheme ===" -ForegroundColor Cyan

    if ($name -eq 'settings') {
        # Each page paints different content, so each page is measured.
        foreach ($page in @('general', 'audio', 'transcription', 'output', 'advanced')) {
            $report = (& $probe -Window settings -Port $Port -Click "#settings-tab-$page" -Expression $measure |
                    ConvertFrom-Json) | ConvertFrom-Json
            $measured += 1
            if ($report.surface -ne $expectedSurface) {
                $mismatched += "settings/$page asked for $scheme (--surface $expectedSurface), rendered $($report.surface)"
            }
            Write-Host ("  {0,-14} surface={1} failures={2}" -f $page, $report.surface, $report.findings.Count)
            foreach ($finding in $report.findings) {
                $total += 1
                Write-Host ("      {0} {1} {2}:1 (needs {3}) {4} on {5}  '{6}'" -f `
                        $finding.kind, $finding.where, $finding.ratio, $finding.need,
                    $finding.colour, $finding.behind, $finding.sample) -ForegroundColor Yellow
            }
        }
    } else {
        $report = (& $probe -Window $name -Port $Port -Expression $measure | ConvertFrom-Json) | ConvertFrom-Json
        $measured += 1
        if ($report.surface -ne $expectedSurface) {
            $mismatched += "$name asked for $scheme (--surface $expectedSurface), rendered $($report.surface)"
        }
        Write-Host ("  surface={0} failures={1}" -f $report.surface, $report.findings.Count)
        foreach ($finding in $report.findings) {
            $total += 1
            Write-Host ("      {0} {1} {2}:1 (needs {3}) {4} on {5}  '{6}'" -f `
                    $finding.kind, $finding.where, $finding.ratio, $finding.need,
                $finding.colour, $finding.behind, $finding.sample) -ForegroundColor Yellow
        }
    }
    Clear-Scheme -Target $name
  }
}

Write-Host ''
if ($measured -eq 0) {
    Write-Warning 'No surface was measured. This is a broken run, not a pass.'
    exit 1
}
if ($mismatched.Count -gt 0) {
    foreach ($entry in $mismatched) { Write-Warning $entry }
    Write-Warning ('The requested palette was not the one rendered, so those numbers describe the ' +
        'wrong palette.')
    exit 1
}
if ($total -gt 0) {
    Write-Warning "$total contrast failures. UI-GUIDE.md claims WCAG AA for both palettes."
    exit 1
}
Write-Host "Every measured element meets WCAG AA across $measured surfaces." -ForegroundColor Green
