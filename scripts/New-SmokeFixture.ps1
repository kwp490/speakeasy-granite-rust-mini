<#
.SYNOPSIS
Generates the engine smoke test's speech fixture with Windows' own speech synthesiser.

.DESCRIPTION
Setup's engine smoke test transcribes a short clip and compares the result
against known ground truth, word for word. That test is what earns the rest of
the install: a speech model whose audio projector failed to attach does not
error, it writes fluent text from the instruction alone, so "it returned a
transcript" proves nothing and only content does.

The clip is *generated* rather than recorded, and the reason is reproducibility
rather than convenience. The previous fixture -- `beckett.wav` -- was a
recording that lived in a gitignored directory, and by 2026-08-18 it had been
lost from every checkout and every backup on this machine, taking the ability
to regenerate the proof with it. Anything this script makes can be made again
by anyone, on a machine with no microphone, with no recording session to
schedule.

What is given up, stated plainly: synthesised speech is not microphone input.
It has no room, no breath and no clipping, so this clip cannot tell you the
capture pipeline works -- only that the model reads audio and returns what was
in it. That is precisely the failure the smoke test exists to catch, so the
trade is worth making here and would not be worth making for a capture test.

The text is original and deliberately so. It carries sentence-final
punctuation, an internal comma and a proper noun, so a transcript that lost
Granite's single-pass punctuation and casing fails the comparison rather than
passing a lowercase run-on.

.PARAMETER Destination
Where to write the WAV. Defaults to the committed fixture the bootstrapper
embeds with `include_bytes!`.

.PARAMETER Voice
Which installed SAPI voice to use. Defaults to whatever Windows has set, which
is why the ground truth is verified by transcription rather than assumed --
see `Test-SmokeFixture.ps1`.
#>
[CmdletBinding()]
param(
    [string]$Destination,
    [string]$Voice
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
if (-not $Destination) {
    $Destination = Join-Path $repositoryRoot 'apps/bootstrapper/fixtures/smoke.wav'
}

# The sentence the fixture speaks. Kept here and in `smoke.rs` -- and checked
# against each other by the bootstrapper's own test -- because a fixture whose
# ground truth is only written down in one place drifts the first time either
# is edited.
#
# Every word here has to survive synthesis *and* transcription, which is not
# the same as being easy to read. The first version ended "and Granite writes
# it down"; Microsoft David's pronunciation of the product's own name came back
# from the model as "Granit", so the fixture would have pinned a
# mis-transcription and broken the day a model update got it right. Proper
# nouns earn their place here -- they are what proves the single pass produced
# casing -- but they have to be ones the synthesiser says plainly. Change this
# line and you must re-run `transcribe_file` and re-verify; do not assume.
$spoken = 'The quick brown fox jumps over the lazy dog, and Monday begins at dawn.'

Add-Type -AssemblyName System.Speech

# 16 kHz, 16-bit, mono: what Granite's encoder consumes. Asking the synthesiser
# for it directly avoids a resample step whose artefacts would be baked into a
# committed file where nobody would look for them.
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(
    16000,
    [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
    [System.Speech.AudioFormat.AudioChannel]::Mono)

$directory = Split-Path -Parent $Destination
if (-not (Test-Path -LiteralPath $directory)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}

$synthesizer = New-Object System.Speech.Synthesis.SpeechSynthesizer
$voiceName = $null
try {
    if ($Voice) { $synthesizer.SelectVoice($Voice) }
    # Read before the finally block disposes it: a disposed synthesiser reports
    # an empty voice name rather than throwing, so the first version of this
    # script printed a blank line where the provenance of the bytes should be.
    $voiceName = $synthesizer.Voice.Name
    # Slower than default. A synthesiser at full rate produces clipped word
    # boundaries that a real speaker does not, and the point of this clip is to
    # be transcribable, not to be fast.
    $synthesizer.Rate = -2
    $synthesizer.SetOutputToWaveFile($Destination, $format)
    $synthesizer.Speak($spoken)
} finally {
    $synthesizer.SetOutputToNull()
    $synthesizer.Dispose()
}

$item = Get-Item -LiteralPath $Destination
$seconds = [math]::Round(($item.Length - 44) / (16000 * 2), 2)
Write-Host "Wrote $Destination"
Write-Host "  voice   : $voiceName"
Write-Host "  spoken  : $spoken"
Write-Host "  bytes   : $($item.Length)  (~$seconds s at 16 kHz mono PCM16)"
