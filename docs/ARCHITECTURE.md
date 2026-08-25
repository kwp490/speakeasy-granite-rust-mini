# Architecture

> **One engine.** IBM Granite Speech 4.1 runs once over the retained recording
> after the user stops it, and that single pass produces the punctuation and
> casing along with the words. There is no live transcription and no second
> engine: a pass that fails, returns nothing, or fails the plausibility gate
> ends the dictation with a named reason rather than substituting a weaker
> result.

## Overview

SpeakEasy Mini is a Tauri v2 desktop app: a React/TypeScript frontend for
presentation only, and a Rust backend that owns every capability the frontend
can't be trusted with directly (filesystem, network, process spawning,
clipboard/input synthesis, credentials). The frontend talks to the backend only
through a narrow set of Tauri IPC commands; it has no direct filesystem,
network, or OS-input access.

## Data flow: mic to delivered text

```text
microphone (cpal) -> speakeasy-audio capture pipeline
                   -> retained audio, held in memory until stop
                        -> workers/granite-worker (child process)
                             -> llama.cpp, Granite Speech 4.1 2B
                        -> speakeasy-worker::judge_granite_pass
                             -> a transcript, or a named reason and nothing
                   -> speakeasy-transforms (dictionary, snippets, cleanup)
                   -> speakeasy-windows::CommitWriter (clipboard + synthesized Ctrl+V)
                        gated by speakeasy-delivery's target-safety checks
```

One entry point drives this pipeline: the **global hotkey** (`Ctrl+Alt+P` by
default), registered at startup via `tauri-plugin-global-shortcut`. Press once
to start capture, press again to stop, transcribe, personalize, and paste into
the focused window. The dock's Stop button ends a recording too — necessary
because the hotkey has a hands-free mode in which no key ends one.

The inference worker is a separate process, supervised (kill-on-close Windows
Job, deadlines, crash-loop quarantine after repeated failures) so a native
crash can't take down the desktop app. It talks to the desktop over a
length-prefixed JSON stdio protocol.

Capture endpointing is intentionally manual: the hotkey or the dock's Stop
button ends a dictation. There is no VAD. Every recording has a hard two-minute
safety ceiling; reaching it auto-stops capture and sends the retained audio
through the normal transcription and delivery path, sounds the stop cue, and
shows the `notice` window so the user knows the limit was reached.

**The buffer has to outlast the ceiling, and for a long time it did not.** The
retained utterance costs 36 bytes per frame — an `f32` plus a 32-byte
`ProcessedSampleMetadata` — so `max_buffered_bytes` of 64 MiB was 1,864,135
frames, or 116.5 s at 16 kHz, against a 121 s capacity and a 120 s ceiling. The
byte limit bound first: every maximum-length dictation rejected its last ~3.5 s,
raised `UtteranceIssues::BYTE_LIMIT`, and — because that was returned as an
error — had the whole recording discarded. It is 128 MiB now, and
`the_ceiling_stays_inside_the_pipeline_byte_limit` reads the limit out of the
config the code actually builds rather than holding its own copy of the number,
which is how it passed through the entire period the relationship was broken.

**A finished capture reports one failure and five annotations.** Only
`frames_buffered == 0` means there is nothing to transcribe. A dropped callback
block, a processing overrun and the three buffer limits all describe audio that
exists, so `judge_completion` returns them alongside the transcript instead of
in place of it, and the user is told what may be missing rather than losing the
recording.

## Crate map

| Crate | Owns |
|---|---|
| `speakeasy-domain` | Correlation/session IDs, state machines, cancellation/deadline contracts, the framed-JSON worker protocol, object-safe ports. No dependencies of its own. |
| `speakeasy-audio` | CPAL microphone capture, resampling, the capture pipeline, and the start/stop cues. VAD types remain available for experiments, but the shipped policy is manual stop plus a two-minute safety ceiling. |
| `speakeasy-worker` | The worker-protocol client boundary, the ordered finalization queue, and `judge_granite_pass` — the verdict on whether a transcript is fit to deliver. Links no native libraries. |
| `speakeasy-granite` | IBM Granite Speech 4.1 on llama.cpp. The only crate that compiles C++; see "The native runtime" below. |
| `speakeasy-models` | Trusted model manifest, download/verify/install/activate lifecycle, hardware inventory. |
| `speakeasy-delivery` | Target-safety classification (focus/process/element/integrity checks) and the pre-write delivery plan. |
| `speakeasy-transforms` | Dictionary/protected-term matching, text snippets, locale formatting (only `en-US` sentence-case is implemented; other locales pass through unchanged), optional rule-based cleanup. |
| `speakeasy-storage` | Settings, history, personalization persistence (SQLite + JSON, versioned, with backup/recovery). |
| `speakeasy-windows` | Clipboard/paste synthesis, process supervision, credential manager access, Windows-specific lifecycle handling. |
| `speakeasy-test-support` | Fakes/fixtures used by tests only. |

`speakeasy-worker` was `speakeasy-asr`, and it linked sherpa-onnx. What it kept
is the half that was never about recognition — a protocol boundary, an ordering
queue, and a plausibility gate — so it now builds and tests in seconds with no
model, no GPU, and no toolchain beyond rustc.

Apps and workers:

- `apps/desktop` — the shipped Tauri app.
- `apps/bootstrapper` — one binary, two entry modes: the setup installer, and
  the backup/restore tool it absorbed from `apps/repair`. Has no Tauri
  dependency. See "Setup" below.
- `workers/granite-worker` — the supervised child process that loads Granite
  Speech and runs the transcription pass.

The dependency graph is enforced by `scripts/Test-DependencyPolicy.ps1` and
`deny.toml`/`.cargo/audit.toml` (license/advisory allowlist, exact-pinned
versions).

### The native runtime: llama.cpp

`speakeasy-granite` runs IBM Granite Speech 4.1 2B, which is a language model
with a speech encoder and an audio projector rather than an ASR model —
transcription is autoregressive text generation. There is no ONNX export of it,
so it runs on llama.cpp via `llama-cpp-2` with the `mtmd` feature. It is a
separate crate so that its build cost falls only on what needs it.

That build cost is real: `llama-cpp-sys-2` *compiles* llama.cpp rather than
downloading a prebuilt library, which makes three things build prerequisites:

- **A C++ toolchain** (MSVC).
- **CMake.** Staged under `.tools/cmake-4.4.0-windows-x86_64`, and put on `PATH`
  by `Enter-DevEnvironment.ps1`. Without it the build fails with ``is `cmake` not
  installed?`` several minutes in.
- **libclang**, for bindgen. Located by `scripts/Resolve-Libclang.ps1`, which
  checks `LIBCLANG_PATH` first and then the usual install locations. Without it
  the build panics with `Unable to find libclang`.

Why this is called out rather than assumed: the crate built on 2026-08-03 and
failed in the same checkout the next day, because both CMake and libclang were
being satisfied by whatever happened to be on `PATH` in one shell.
`Enter-DevEnvironment.ps1` warns when either is missing;
`Invoke-ScaffoldChecks.ps1` refuses to start, because the gate builds this crate.

Two further consequences:

- **`git config --global core.longpaths true` is required.** llama.cpp's
  `tools/ui` tree, checked out under a workspace-local `CARGO_HOME`, exceeds
  Windows' 260-character `MAX_PATH` by a few characters, and cargo's fetch fails
  with `path too long`. The machine-wide `LongPathsEnabled` registry flag is not
  sufficient on its own — git and libgit2 need telling as well. This has to be
  global rather than repo-local, because the checkout lives in `CARGO_HOME`,
  outside this repository.
- **`[patch.crates-io]` pins `llama-cpp-sys-2` to a fork**, because the llama.cpp
  that `llama-cpp-2` 0.1.153 vendors does not transcribe with Granite Speech. The
  fork carries one upstream cherry-pick and no invented code; `Cargo.toml` has the
  full reasoning, `Test-DependencyPolicy.ps1` enforces its shape, and `deny.toml`
  allows exactly that one git source. It is meant to be retired.

Because `speakeasy-granite` is the only crate that pays this cost, the gate runs
it as a separate job (`Invoke-ScaffoldChecks.ps1 -SkipGranite` and
`-GraniteOnly`) so a cold C++ build cannot starve the checks that catch
everything else. Every other crate now builds in seconds.

### Which provider runs, and how you find out

Granite runs on the GPU or the CPU, and — unlike the streaming engine this
replaced — that is decided by **which worker binary is installed**, not by which
model pack was downloaded. Granite's CUDA support is a compile-time feature of
`speakeasy-granite`, so a machine with a perfectly good NVIDIA card still runs on
the CPU unless a CUDA-built `granite-worker.exe` is present beside it with the
CUDA libraries the catalog pins.

Two consequences follow, and both are deliberate:

- **There is no provider-override setting.** One existed for the streaming
  engine, where both packs were downloadable and preferring either was
  meaningful. Here no setting can conjure a worker binary, so a control offering
  the choice would report a state the engine will not be in.
- **Setup records which configuration it installed.** Without that, "running on
  CPU because you chose CPU" and "running on CPU because the GPU worker will not
  load" are the same silent outcome. With it, the first is normal and the second
  is an error with instructions.

#### The record is proof, and it takes three facts

Rewritten 2026-08-20, from a support log that read
`engine=cpu_gpu_runtime_missing device=cpu installed=cuda`. Every field was
correct. The combination is impossible, and nothing anywhere compared them.

The record came from the wizard's provider radio button, which was never
disabled, so a user on a CUDA-capable machine could select "Use the graphics
card" and setup would write `cuda` while installing the only configuration it
carries. `speakeasy_models::granite_gpu` is now the one place the question is
answered, and it separates three genuinely independent facts:

1. **Published** — is a CUDA-capable worker pinned in the trusted manifest, as a
   `native-runtime` artifact with id `granite-worker-cuda-windows-x64`? A CUDA
   `final-asr` *pack* answers nothing: there is one GGUF and the CUDA worker
   offloads that same file, so a pack entry would be a duplicate of the CPU one.
   Asking the manifest for a pack is exactly what the old check did.
2. **Present** — is that worker on this disk with every library the catalog
   requires beside it? `cudart` and `cuBLAS` are pinned in the same catalog and
   their `proof_files` are the requirement, read from there rather than written
   down a second time. A CUDA build without them does not run slower — and it
   does not always fail at startup either, which is why this is a precondition
   rather than something read off a worker that came up. Measured 2026-08-21:
   `cublas64_13.dll` is an import of the image and its absence stops the process
   before `main`, while `cublasLt64_13.dll` is loaded by cuBLAS at the first
   matmul, so a worker missing only that one starts, loads the weights, and
   fails ~36 s later mid-dictation. `cudart64_13.dll` is the third case: it is
   required and **never loaded**, because ggml links the CUDA runtime statically
   on Windows. The requirement list is a deliberate superset — see
   "The CUDA requirement list is a superset, deliberately" below.
3. **Operational** — is a live worker process holding a CUDA context? Nothing
   static can say. The startup handshake (`Hello` → `compiled_accelerators`)
   reports what the binary *could* do, and a refusing driver, a claimed card or
   exhausted VRAM runs that same binary on the processor while llama.cpp notes
   the fallback in its own stderr. NVML lists the pids holding a compute context
   per device, so the proof is the worker's **own process id** appearing there —
   a name match would be satisfied by a second copy started by something else.

Setup's engine check requires all three before `install-provider.txt` says
`cuda`, and writes nothing at all if the check never ran (the app reads that as
`unrecorded`). The bootstrapper's `--verify-provider` verb runs that same check
against an installed build and rewrites the marker from its verdict, so the
record can be re-proved without a reinstall — which is what
`scripts/Enable-GraniteCuda.ps1` calls after it stages a CUDA worker, and after
`-Revert` puts the processor one back. **Two callers, one implementation.** The
script reads no NVML and writes no marker: `install-provider.txt` having exactly
one writer is what makes a claim assembled from an intention unrepeatable, and a
PowerShell re-implementation of the three gates would have been a second writer
wearing a different hat. The app re-checks the third at every warm and compares it against
the record. `ProviderIntegrity` has five values: `ok`, `unrecorded`,
`gpu_install_not_operational` — the actionable fault — `running_beyond_record`,
which is what `scripts/Enable-GraniteCuda.ps1` produces on purpose and is
disclosed rather than treated as a failure, and `gpu_record_unconfirmed`.

That fifth one is the fault's other half, split out on 2026-08-21. Only a
**definitive** negative is a fault: a worker with no CUDA backend, or one NVML
answered about and did not list. A probe that could not be asked — and a worker
that never answered its handshake — prove nothing in either direction, and
reporting them as the fault told the user dictation had moved to the processor
on the strength of a failed driver query. That is the one inference
`granite_gpu`'s own header forbids, made one layer up where nothing was looking.
The verdict and the device now agree by construction: `cpu` is the fault,
`cuda_unverified` and `unknown` are unconfirmed, and no layer names a device it
did not establish. Folding the case into `ok` was rejected for the mirror-image
reason — that claims an agreement nothing verified.

`GraniteEngineCoordinator::engine_reason` carries a stable code for why this
machine is on the provider it is on — `probe_preferred`,
`cpu_gpu_pack_not_installed`, `cpu_gpu_runtime_missing` — and that code reaches
the diagnostics view and the log. It names the **pack**, not the device:
`device=` is the device, and it reads `cuda` only where NVML confirmed the
context, `cuda_unverified` for a CUDA build whose context could not be checked,
and `cpu` otherwise. GPU *admission* (the probe says the card qualifies) stays
distinct from GPU *qualification* (a model has actually executed on it); the app
reports the difference rather than conflating them.

Measured on an RTX 5090: Granite Q4_K_M resident run 1,571.9 ms on CPU versus
156.4 ms on CUDA, RTF 0.158 versus 0.0157, holding ~3.27 GiB of VRAM. Cold load
is 5,218 ms against 2,104 ms.

Measured 2026-08-21 on an **RTX 4070 Laptop GPU** (compute 8.9, driver 596.36,
8,188 MiB), through the app's own resident path on a 6.42 s clip: resident pass
2,928 ms on the processor against **361 ms** on CUDA — RTF 0.456 against 0.0563,
a factor of 8.1. Different card, different clip and a different harness from the
5090 figures above, so the two sets are not comparable to each other; both are
comparable within themselves, which is the only comparison either was made for.

**The transcript is byte-identical on both devices.** The same whole-transcript
assertion passes against the CPU worker and the CUDA worker, which is worth more
than the speed: setup's engine check compares a whole transcript against one
pinned ground truth, so a CUDA path that changed the greedy decode by a single
mark would fail that check on every graphics-card machine, and nothing would have
found out until one existed.

That claim rests on **one fixed WAV** through both workers, and it can only be
made that way. The real-speech figures below are a different experiment and do
not bear on it in either direction — a reader comparing the two transcripts a
person produced would be comparing two different recordings.

### Measured on real speech, 2026-08-25

The first figures in this document that are not from a harness. A person read a
prepared 230-word passage into an installed release build on the same RTX 4070
Laptop rig, once per provider.

| | Card | Processor |
| --- | --- | --- |
| Audio | 105.248 s | 120.183 s (capture ceiling) |
| Inference | **4,171 ms** | **44,493 ms** |
| RTF | **0.0396** | **0.3702** |
| Press-to-paste | **4,246 ms** | n/a — ended by the ceiling |

**The processor is 9.34x the card**, against the 8.1x the 6.42 s fixture
predicted; the fixture is a fair guide and errs in the optimistic direction for
the processor. RTF is the only figure comparable across the two runs, because the
two recordings differ in length.

Press-to-paste decomposes as 54 ms from the stop press to the finalisation job
starting, 4,171 ms of inference, and 21 ms to inspect the foreground window and
paste. **Inference is 98% of the user-visible latency**, so nothing else in this
path is worth optimising.

One consequence belongs here rather than in the handoff, because it follows from
the delivery rule stated in the next section: on the processor there are ~44
seconds between the user stopping speaking and the paste, and the target is
resolved at the *end* of that. Every window the user touches in the interval is
a candidate. This is not the hijack hazard below — that is detected and falls
back to the clipboard — but a successful delivery into an application the user
had moved on to, which the log cannot distinguish from a correct one.

## Delivery safety

**The target is whatever Windows reports as the foreground window** at the
moment a dictation finishes. That single fact is the premise the rest of this
section rests on, and it makes any window SpeakEasy Mini itself puts in the
foreground a delivery target — including windows the user cannot see. Three
separate causes have done exactly that (a hidden-but-focused settings window, a
console allocated because the release binary had no `windows_subsystem`
attribute, and one console per worker once that first console was gone), and none
of them errored: each produced a `target_inspect_refused` refusal and a clipboard
fallback, which reads as a fault in some other subsystem. So a new window or
spawned process is a delivery-safety change, not just a UI one.

All three windows — `main`, `hud-dock` and `log` — declare `focus: false` and
are made non-focusable in `configure_hud`. A scaffold test asserts it for every
declared window, so adding a fourth without that fails the gate rather than
producing a delivery bug months later. `UI-GUIDE.md` carries the resulting rule;
`CLAUDE.md` carries the trap.

`speakeasy-delivery` classifies the focused target before any automatic
write. A password field, the secure desktop, or a window confirmed to be
running at higher (or unknown) integrity refuses *all* automatic delivery —
clipboard included — and falls back to the result view, because SpeakEasy Mini
has positive evidence the target is sensitive. When delivery is allowed,
exactly one `Ctrl+V` is sent (tagged via `dwExtraInfo` so SpeakEasy Mini can
distinguish its own synthesized input) — never an automatic Enter.
`VerifiedRangeReplace` and `AppendOnlyLive` (in-place, live-editing delivery
modes) exist in the type system but aren't reachable in the current build;
`CommitOnFinish` (paste after the final transcript is ready) is the only
delivery mode that actually runs.

Every other refusal — a read-only target, a terminal, an interrupted paste
(focus changed, a modifier was held, the clipboard raced another writer), or
a target that could not be inspected at all
(`DeliveryRefusal::TargetInaccessible`) — falls back to an automatic
clipboard-only copy instead of silently dropping the transcript.
`TargetInaccessible` is distinct from `ElevatedTarget`: it fires when the
`OpenProcess` call in
`speakeasy-windows::target::inspect_current` itself fails, which happens for
packaged, AppContainer-sandboxed processes (New Outlook for Windows is one)
denying even `PROCESS_QUERY_LIMITED_INFORMATION` to an unpackaged caller —
a sandbox boundary, not evidence of elevation. Because that failure happens
before a `TargetSnapshot` exists, SpeakEasy Mini has no way to know whether the
focused control was sensitive; the clipboard fallback still runs there by
deliberate choice, accepting the residual risk in exchange for never
silently losing a dictation. The sanitized numeric OS error behind a
`TargetInaccessible` refusal (never the OS-provided message text) is
available via `TargetObserver::last_os_error()` and logged alongside the
refusal reason.

## Model

The app installs one Granite pack: `granite-speech-4.1-2b-q4_k_m-cpu`. Its
manifest entry, per-file checksums and licences live in
`models/trusted-manifest.json`, and the worker rejects any other artifact ID.

Q4_K_M is the shipped quantization on measurement rather than by decision — ~21%
faster than Q8_0 on a 120 s utterance (RTF 0.277 versus 0.352) with an identical
transcript but for one punctuation choice. `granite-speech-4.1-2b-q8_0-cpu`
stays in the catalog as the recorded alternative, not as a second configuration
to keep working.

The packs are the schema-v3 archive-less shape: Hugging Face serves the GGUFs as
loose files, so there is no single archive digest and each required file carries
its own URL and SHA-256. Model bytes are never bundled — setup downloads and
verifies them, then the app works fully offline.

The two CUDA redistributables in the catalog (`cudart`, `cuBLAS`) are there for
llama.cpp's GPU build. The cuFFT and cuDNN entries this catalog used to carry
belonged to ONNX Runtime and left with it.

### The CUDA requirement list is a superset, deliberately

Three files are enforced and only two are loaded. Measured 2026-08-21 against
the CUDA worker this workspace builds, with the CUDA Toolkit stripped from
`PATH` — which matters, because the toolkit puts `bin\x64` on it and Windows
will then resolve a library that is not beside the worker at all:

| File | Named in the image | Deleting it |
| --- | --- | --- |
| `cublas64_13.dll` | yes | the process does not start |
| `cublasLt64_13.dll` | no | starts, loads the weights, fails ~36 s in at the first matmul |
| `cudart64_13.dll` | no | nothing: transcribes, and NVML confirms the context |

`cudart` is never loaded because ggml links the CUDA runtime statically on
Windows. It stays enforced anyway, and that is a decision rather than an
oversight: `CMAKE_CUDA_RUNTIME_LIBRARY` is one build flag away from making it
load-bearing again with nothing anywhere noticing, and every file this catalog
requires is a file it pins by digest — the property that lets presence imply
provenance, and the reason `cudart64_*.dll` by pattern was rejected. The cost is
551 KB and a refusal that no published payload can trigger, since the worker and
its libraries are pinned and shipped as one artifact.

`cublasLt` is the case that justifies the shape of the check. A gate that
concluded "the worker started, so its libraries are fine" would have passed that
payload and lost a dictation half a minute later.

## Trust boundaries

- React is presentation-only. The main window has only `core:default`
  Tauri capabilities plus the specific commands the app registers; it has no
  filesystem, network, shell/process, or raw-input authority.
- `withGlobalTauri` is false; CSP allows only local application/IPC/asset
  sources.
- The dock's poll is read-only (`capture_hud_status` never mutates state) and
  the dock is not focusable. Its command allowlist is explicit and asserted
  against source in both the Rust and frontend suites.
- The pinned log window has its own, narrower gate
  (`require_main_or_log_window`). It gets the addressable
  `session_transcript_copy` that the dock is refused, because browsing the log
  is the entire purpose of that window — and the dock, which is on screen during
  every dictation, still cannot reach it.
- Legacy credential entries are inspected only for migration reporting and are
  never returned through IPC, logged, or placed in argv/environment.

## What's implemented vs. not

Implemented and wired into the shipped app: local Granite transcription with
punctuation in a single pass, the global hotkey with auto-paste, the side dock,
the pinnable transcript log with optional on-disk retention,
dictionary/snippet personalization, `en-US` locale formatting, settings
persistence, and model install/update/remove.

Not implemented: automatic (VAD) endpointing — deliberately excluded from the
shipped policy — push-to-talk/hands-free activation (the reducer logic exists
and is unit-tested, but isn't exercised outside tests), non-English locale
formatting, and diagnostic WAV export.

Deliberately removed, and not coming back without new evidence: live
transcription, the large transcriber HUD, the in-app setup wizard, the
provider-override control, and the `immediate_repetitions` / `self_corrections`
cleanup rules.

## Setup

Setup is a single downloadable executable — `apps/bootstrapper` — and it is the
only setup path. There is no in-app wizard; the app assumes it was installed by
something that already checked the machine.

What it does, in order:

1. Probes the hardware and picks the CPU or GPU configuration.
2. Downloads what that configuration needs, resumably, and verifies every file
   against a SHA-256 digest pinned in `models/trusted-manifest.json`.
3. Runs an engine smoke test: transcribes a short bundled clip and compares the
   result against known ground truth, **word for word**. This is the step that
   earns the rest. A speech model whose audio projector failed to attach does
   not error — it writes fluent text from the instruction alone — so
   "it returned a transcript" proves nothing. Only content does.
4. Asks whether to retain transcripts between sessions, and seeds the answer
   into the profile (default: no).
5. Records which configuration it installed, so the app can later tell a CPU
   install apart from a broken GPU one.
6. Launches the app.

Four structural decisions, recorded with their costs:

- **NSIS is replaced entirely**, so the bootstrapper carries the version stamp
  and its downgrade refusal, refuse-while-running, ARP registration, Start Menu
  shortcuts, WebView2 provisioning, and the uninstaller's data-retention
  prompt. What it deliberately did *not* inherit is NSIS's `/SD IDYES`
  keep-everything default: since 2026-08-21 an uninstall removes the program
  directory and the profile whole, asks once with the full scope named, and takes
  `--keep-user-data` for install/uninstall cycles that would otherwise
  re-download the weights.
- **The wizard is native**, drawn with `winsafe`'s `gui` and `shell` features,
  because the thing that provisions WebView2 is the thing being replaced and
  repair mode runs on already-broken machines.
- **It absorbs `apps/repair`**, so it is one binary with two entry modes rather
  than a third executable. The repair CLI verbs are preserved exactly and
  `docs/RUNBOOK.md` stays accurate.
- **It stays console-subsystem** and re-launches itself detached for the window
  half. A `windows_subsystem = "windows"` binary cannot also answer a script,
  and it fails in the shape that looks like success — see `CLAUDE.md`.

The GPU worker is fetched rather than bundled: a CUDA `granite-worker.exe` plus
`cudart` and `cuBLAS`, hosted on Hugging Face and pinned by digest like the
model weights. Hosting it there rather than in this repository's releases keeps
the code repository private while leaving the download anonymous.

This project is **never signed** (owner decision, 2026-08-14) — a decision, not
a deferral, so setup must not imply otherwise.
