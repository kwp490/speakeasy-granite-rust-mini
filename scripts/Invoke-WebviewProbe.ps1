<#
.SYNOPSIS
Evaluates JavaScript inside a running SpeakEasy window and returns the result.

.DESCRIPTION
Layout rules like "no horizontal scrolling anywhere" (docs/archive/UI-REDESIGN.md §13,
UI-GUIDE.md) are numbers, and a screenshot is a poor instrument for them. The
first attempt at measuring overflow read `WS_HSCROLL` from the window's child
HWNDs — and reported a horizontal scrollbar on all five settings pages *and* on
the transcriber, which has `overflow: hidden` and cannot scroll at all.
`Chrome_RenderWidgetHostHWND` carries both scrollbar styles unconditionally. The
check measured nothing while looking exactly like a check that had found five
bugs. That is the vacuous-assertion failure the redesign handoff §5 warns about,
so this replaces it with a real reading.

WebView2 exposes the Chrome DevTools Protocol when the host process is started
with a debugging port. Nothing in the app changes to allow it: the port comes from
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` in the environment of whoever launches it,
and the strict CSP governs page content, not the protocol.

Start the app with the port open first:

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    npm run tauri -- dev

Besides measuring, this can drive the UI **by selector**, which removes the
worst hazard in the pixel-offset harness: an offset that has drifted onto the
neighbouring control looks identical to a broken control. `-Click` dispatches a
real click on a real element or fails loudly because the selector matched nothing.

.PARAMETER Window
Document title substring identifying the target: "settings" or "transcriber".

.PARAMETER Expression
JavaScript to evaluate. The value is returned as JSON.

.PARAMETER Click
CSS selector to click before evaluating. Fails if it matches no element.

.EXAMPLE
./scripts/Invoke-WebviewProbe.ps1 -Window settings -Expression 'document.documentElement.scrollWidth'

.EXAMPLE
./scripts/Invoke-WebviewProbe.ps1 -Window settings -Click '#settings-tab-audio' -Expression 'document.title'
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Window,

    [string]$Expression,

    [string]$Click,

    # A raw CDP method and its JSON parameters, for things with no JavaScript
    # equivalent — `Emulation.setDeviceMetricsOverride` above all, which is how
    # layout is checked at several widths without resizing the OS window.
    [string]$Cdp,

    [string]$CdpParams = '{}',

    [int]$Port = 9222,

    [int]$TimeoutSeconds = 15
)

$ErrorActionPreference = 'Stop'

function Get-DebugTarget {
    try {
        $targets = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 5
    } catch {
        throw ("No DevTools endpoint on port $Port. Start the app with " +
            "`$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=$Port'.")
    }
    # Both windows load the same bundle, so both report the same document title.
    # They are told apart by the root element each one renders (§20.1).
    $marker = switch ($Window) {
        'settings' { 'desktop-scaffold' }
        'transcriber' { 'capture-hud' }
        'dock' { 'capture-hud-dock' }
        default { throw "Window must be 'settings', 'transcriber', or 'dock'; got '$Window'." }
    }

    $check = @{
        method = 'Runtime.evaluate'
        params = @{
            expression    = "document.querySelector('[data-testid=`"$marker`"]') !== null"
            returnByValue = $true
        }
    }

    foreach ($candidate in ($targets | Where-Object { $_.type -eq 'page' })) {
        $probe = Invoke-Cdp -SocketUrl $candidate.webSocketDebuggerUrl -Commands @($check)
        if ($probe[0].result.result.value -eq $true) { return $candidate }
    }

    # Nothing matched. A window whose React root is empty has loaded the document
    # but never mounted — seen in `tauri dev` when a webview reaches the Vite
    # server before it is ready. Reload once and say so, rather than reporting the
    # window as absent when it is present and blank.
    foreach ($candidate in ($targets | Where-Object { $_.type -eq 'page' })) {
        $empty = Invoke-Cdp -SocketUrl $candidate.webSocketDebuggerUrl -Commands @(
            @{ method = 'Runtime.evaluate'; params = @{ expression = 'document.getElementById("root")?.childElementCount ?? -1'; returnByValue = $true } }
        )
        if ($empty[0].result.result.value -ne 0) { continue }
        Write-Warning "A window loaded but never mounted (empty React root). Reloading it."
        Invoke-Cdp -SocketUrl $candidate.webSocketDebuggerUrl -Commands @(
            @{ method = 'Page.reload'; params = @{ ignoreCache = $true } }
        ) | Out-Null
        Start-Sleep -Seconds 3
        $probe = Invoke-Cdp -SocketUrl $candidate.webSocketDebuggerUrl -Commands @($check)
        if ($probe[0].result.result.value -eq $true) { return $candidate }
    }

    throw "No page rendering [data-testid=`"$marker`"]. Is the $Window window loaded?"
}

function Invoke-Cdp {
    param([string]$SocketUrl, [System.Collections.IEnumerable]$Commands)

    $socket = New-Object System.Net.WebSockets.ClientWebSocket
    $cancel = New-Object System.Threading.CancellationTokenSource ([TimeSpan]::FromSeconds($TimeoutSeconds))
    try {
        # `GetResult()` on a void task emits a VoidTaskResult into the pipeline.
        # Left unsuppressed, those leak out of this function ahead of the replies
        # and `$replies[0]` is a task result rather than a CDP message — which is
        # how the target probe came to report every window as unmatched.
        [void]$socket.ConnectAsync([Uri]$SocketUrl, $cancel.Token).GetAwaiter().GetResult()

        $results = @()
        $id = 0
        foreach ($command in $Commands) {
            $id += 1
            $command.id = $id
            $json = $command | ConvertTo-Json -Depth 8 -Compress
            $payload = [System.Text.Encoding]::UTF8.GetBytes($json)
            $segment = New-Object System.ArraySegment[byte] (, $payload)
            [void]$socket.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cancel.Token).GetAwaiter().GetResult()

            # Read until the reply carrying this id arrives; CDP interleaves events.
            while ($true) {
                $buffer = New-Object byte[] 262144
                $received = New-Object System.Text.StringBuilder
                do {
                    $chunk = New-Object System.ArraySegment[byte] (, $buffer)
                    $result = $socket.ReceiveAsync($chunk, $cancel.Token).GetAwaiter().GetResult()
                    [void]$received.Append([System.Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count))
                } while (-not $result.EndOfMessage)

                $message = $received.ToString() | ConvertFrom-Json
                if ($message.PSObject.Properties.Name -contains 'id' -and $message.id -eq $id) {
                    $results += $message
                    break
                }
            }
        }
        return $results
    } finally {
        if ($socket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
            [void]$socket.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, 'done', $cancel.Token).GetAwaiter().GetResult()
        }
        $socket.Dispose()
        $cancel.Dispose()
    }
}

if ($Expression -eq '' -and $Cdp -eq '') {
    throw 'Pass -Expression, -Cdp, or both.'
}

$target = Get-DebugTarget
Write-Verbose "target: '$($target.title)'"

if ($Cdp -ne '') {
    $reply = Invoke-Cdp -SocketUrl $target.webSocketDebuggerUrl -Commands @(
        @{ method = $Cdp; params = ($CdpParams | ConvertFrom-Json) }
    )
    if ($null -ne $reply[0].error) {
        throw "$Cdp failed: $($reply[0].error.message)"
    }
    if ($Expression -eq '') {
        $reply[0].result | ConvertTo-Json -Depth 8
        return
    }
}

$commands = New-Object System.Collections.ArrayList

if ($PSBoundParameters.ContainsKey('Click') -and $Click -ne '') {
    # Fails loudly when the selector matches nothing, rather than reporting a
    # successful click that never landed on anything.
    $clickScript = @"
(() => {
  const element = document.querySelector($($Click | ConvertTo-Json));
  if (element === null) return 'NO_MATCH';
  element.click();
  return 'CLICKED';
})()
"@
    [void]$commands.Add(@{ method = 'Runtime.evaluate'; params = @{ expression = $clickScript; returnByValue = $true; awaitPromise = $true } })
}

[void]$commands.Add(@{ method = 'Runtime.evaluate'; params = @{ expression = $Expression; returnByValue = $true; awaitPromise = $true } })

$replies = Invoke-Cdp -SocketUrl $target.webSocketDebuggerUrl -Commands $commands

$index = 0
if ($PSBoundParameters.ContainsKey('Click') -and $Click -ne '') {
    $clickResult = $replies[0].result.result.value
    if ($clickResult -ne 'CLICKED') {
        throw "The selector '$Click' matched no element. The click never landed."
    }
    Write-Verbose "clicked '$Click'"
    $index = 1
    # Let React re-render before the measurement reads the DOM.
    Start-Sleep -Milliseconds 400

    $replies = Invoke-Cdp -SocketUrl $target.webSocketDebuggerUrl -Commands @(
        @{ method = 'Runtime.evaluate'; params = @{ expression = $Expression; returnByValue = $true; awaitPromise = $true } }
    )
    $index = 0
}

$reply = $replies[$index]
if ($null -ne $reply.result.exceptionDetails) {
    throw "Evaluation failed: $($reply.result.exceptionDetails.exception.description)"
}
$reply.result.result.value | ConvertTo-Json -Depth 8
