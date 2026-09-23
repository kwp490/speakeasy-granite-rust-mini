# Handoff — SpeakEasy Mini

The current state of the fork: what is true, what is open, and what has been
decided. **Read `CLAUDE.md` first** — this file assumes it, and does not repeat
the traps recorded there.

This is a state document, not a log. A finding is kept while it is open and
removed once it is closed; the reasoning behind a closed one lives in the commit
that closed it, and any hazard general enough to bite again lives in
`CLAUDE.md`'s trap list.

## Where the tree is

| | |
| --- | --- |
| Branch | `main`, on `kwp490/speakeasy-granite-rust-mini` (public) |
| Latest release | `v1.11.1`, 2026-09-23, `SpeakEasyMiniSetup.exe` with `SHA256SUMS` |
| Workspace version | `& .\scripts\Get-ProductVersion.ps1` — currently `v1.12.0` (the Settings rework), built and installed locally, not published |
| Full gate | Run it; `Invoke-ScaffoldChecks.ps1` is the only current answer |
| Ignored tests | nine, all hardware or real-registry. See below |

**Ask git, not this file**, where the branch stands and whether it carries
commits in no release — both change with every commit:

```powershell
git status -sb
git log --oneline origin/main..HEAD
git log --oneline $(git describe --tags --abbrev=0)..main
```

### What v1.11.1 shipped and what proved it (2026-09-23)

The dock redesign: the narrow-width `main` padding rule no longer reaches the
dock, the status row folded into the button, the meter is 178px of normalised,
full-height bars, and the engine chip sits directly above the button. See
`docs/UI-GUIDE.md` "The dock's geometry". `v1.11.0` was built and installed on
the owner's machine only and never published; the `v1.11.1` release notes carry
its changes. Tag `v1.11.1` is at `d0ac491`; `SpeakEasyMiniSetup.exe` is
38,231,267 bytes, SHA-256
`ec0571443a9315e7f1b26fc37cc4ec5035b6271fc4b419887d0a4efed52b8d29`, downloaded
back and re-hashed to that value, from `Build-LocalInstaller.ps1`'s fresh
build. The gate, `Test-InstallerLifecycle.ps1`
and `Test-SetupWizard.ps1` passed against it, the wizard on the graphics card
(RTX 5090 host) with the config restored byte-identical. It was then installed
through the wizard on the same host, graphics card chosen; the engine check
transcribed word for word on the card. The installed dock was screen-captured
at 78x500 physical (62x400 logical at 125%) showing the new row order.

The owner then dictated on the installed build and reported the dock working
as intended; no measurement of the meter against real speech was recorded. The
five states were also checked in a browser against a mocked status poll.

### What v1.10.3 shipped and what proved it (2026-09-22)

One device walk per dictation instead of three, and the start cue played from
the first block of audio rather than from the request. Tag `v1.10.3` is at
`dce3e93`; `SpeakEasyMiniSetup.exe` is 38,285,027 bytes, SHA-256
`eb41c6610ad3c28f648543210b6fef61ca1e08cd10d0da104e8613c8af8783a9`, downloaded
back and re-hashed to that value. The CUDA worker did not change and is still
the one pinned for `v1.10.2`.

Measured on the installed release build, RTX 5090 host, SteelSeries Sonar
default input, five dictations driven by the real shortcut into a scratch
Notepad file: `capture_first_audio` reported `first_audio_ms` 76 on the first
dictation after launch and 28-31 on the other four, with `select_ms` 35 and
then 15-16. Device selection alone measured 43 ms median before and 14 ms
after, in a release probe on the same machine.

The gate, the lifecycle proof and the wizard proof passed, the latter on the
graphics card; the live installation was reinstalled through the wizard and
its config restored byte-identical. The version bump first left `Cargo.lock`
at the old version, and the script was not at fault: it was run piped into
`Select-Object -First 3`, which stops the pipeline — and the script — once it
has three lines, so `cargo update` was killed before it wrote the lockfile. Its
closing "Cargo.lock refreshed" line never printed, which is the tell. Re-running
`cargo update --workspace --offline` moved exactly the 13 workspace crates.

### What v1.10.2 shipped and what proved it (2026-09-22)

The first-word lead-in and the warm-up pass at model load, plus a rebuilt
graphics-card worker that carries both. Tag `v1.10.2` is at `5d8edc9`;
`SpeakEasyMiniSetup.exe` is 38,275,811 bytes, SHA-256
`4cf7fc9368b9d6ea172e9926c7fafc5526fd23838d164e06702c01fedbb70444`, and the
published asset was downloaded back and re-hashed to that value.

The CUDA worker is `orangeblue39/speakeasy-mini-runtime@72c7af40`, built from
`a60eeb7` and pinned in `models/trusted-manifest.json`; it was downloaded back
by pinned revision and matched byte for byte before the pin moved.

Proved on an RTX 5090 host where `Test-HostProfilePathIdentity.ps1` passed
first: the full gate, the installer lifecycle proof, and `Test-SetupWizard.ps1`,
which downloaded the new worker, transcribed the fixture word for word on the
card and logged `device=cuda installed=cuda provider=ok bytes=verified`. The
two new ignored tests and the device, fixture and resident tests passed against
both the processor and the CUDA worker. Not re-run: the 300 s idle gap,
`registry_hive`, `the_real_nvidia`, and the workflow controls, none of whose
code moved.

The live installation was uninstalled with `--keep-user-data` for the proofs
and reinstalled through the wizard on the graphics card; its config files were
restored and compared byte-identical to the pre-run copies.

### What v1.9.0 shipped and what proved it (2026-09-20)

`v1.9.0` is published from `08942ed`: the Settings workspace redesign and the
rustls fix for RUSTSEC-2026-0285. The release carries
`SpeakEasyMiniSetup.exe`, 38,006,499 bytes, SHA-256
`bb270a91d77739811211de969d57f427a976eb3e3345642a75c8eb9ea77fd91f`, with a
one-line `SHA256SUMS`. The published asset was downloaded back and re-hashed to
that value, so the bytes a stranger receives are the bytes that were proved.

`Build-LocalInstaller.ps1` produced the artifact from a fresh cold build, and
every release proof ran against that artifact on an RTX 4070 Laptop GPU under
Windows 11 Pro: the gate at exit 0 with `no leaks found`, the installer
lifecycle, the four host proofs, and the stale-artifact, delete-containment,
build-root, profile-capture and planted-directory controls.

`Test-SetupWizard.ps1` passed end to end this time, transcribing the recording
word for word on the graphics card with `device=cuda` held by the worker's own
pid. The host-identity preflight passed first, in the same shell; that guard is
what refused the previous attempt, and it is never to be bypassed.

The seven ignored hardware and real-registry tests were **not** re-run for this
release. They last ran against `v1.8.1`, the warm path did not move in `1.9.0`,
and the wizard proof above exercised a real CUDA dictation against this
artifact. Re-run them when the warm path next moves.

### What v1.9.1 shipped and what proved it (2026-09-20)

An independent code review ("Astra") raised eight findings and all eight are
fixed in the working tree, each with a red control that reproduced the defect
before the fix and was restored byte-exactly afterwards. Two were disclosure
defects (Settings claimed terminals were excluded; the engine-restart button
claimed a restart it never performed), three were correctness (hotkey rollback,
session-identifier collisions, an unbounded worker write), two were security
(cache reuse without digests, a stale target decision before synthesized input)
and one was the missing automatic-paste control.

**RUSTSEC-2026-0285 was taken independently in both `1.9.0` and this batch, and
the two fixes were not identical.** `1.9.0` updated `Cargo.lock` to `rustls`
0.23.45 and re-recorded the build-script inventory, but left the workspace pin
in `Cargo.toml` at `=0.23.42`. Nothing resolves that pin today, because no crate
takes `rustls` directly — so the disagreement was latent rather than broken. This
version moves the pin to `=0.23.45` so the manifest and the lockfile agree, and
adds a `note` to the `aws-lc-sys` entry recording that its `builder/cc_builder.rs`
`https://` strings are all comments citing upstream issues, which is the same
false-positive class as `web_atoms`.

`speakeasy-desktop` now takes `getrandom` as a direct dependency, recorded in
`Test-DependencyPolicy.ps1` with its reason. It adds no package to the graph.

The workspace and installed application are `1.9.1`. The install was a direct
`Decision::Upgrade` from 1.9.0 rather than an uninstall and reinstall, so the
existing personalization and settings records were untouched.

The local, unsigned development installer is
`target\local-development\1.9.1\SpeakEasyMiniSetup.exe`, 38,249,187 bytes,
SHA-256 `5c3f960b711836c27217730bceac6fe5f4ecd361a099a6d89b536e361ab75cb3`.
This is a local artifact, not a published release asset.

The full `Invoke-ScaffoldChecks.ps1 -SkipNpmInstall` gate passed with exit 0 and
ended `no leaks found`.

`v1.9.1` is published from `76bf4c4`. The release carries
`SpeakEasyMiniSetup.exe`, 38,249,187 bytes, SHA-256
`5c3f960b711836c27217730bceac6fe5f4ecd361a099a6d89b536e361ab75cb3`, with a
one-line `SHA256SUMS`. The published asset was downloaded back and re-hashed to
that value, so the bytes a stranger receives are the bytes that were proved.

**Both installer proofs ran against that artifact and passed**, on a host where
`Test-HostProfilePathIdentity.ps1` passed first in the same shell. The installer
lifecycle proof covered the Add/Remove Programs values, the running-app,
same-version and downgrade refusals, the single-file payload and the derived
default install root. `Test-SetupWizard.ps1` installed for real and transcribed
the recording word for word on the graphics card, `device=cuda` with
`bytes=verified`, then restored every captured config file byte-identically.
All nine workflow controls passed.

The existing installation was uninstalled with `--keep-user-data` to let those
proofs run, and reinstalled afterwards. The 5.2 GB model cache and both config
files came through byte-identical, so no weights were re-fetched.

The seven ignored hardware and real-registry tests were **not** re-run. The warm
path did not move in `1.9.1`, and the wizard proof above drove a real CUDA
dictation against this artifact. Re-run them when the warm path next moves.
One UI measurement from the 1.9.0 handoff is not carried forward: the settings
pages were re-laid-out only in so far as one checkbox and one button label
changed, and the responsive and contrast sweeps were not re-run here. Re-run
them before a release.

Cutting a release from what the last one lists is a decision, not a formality —
see "Before the next release". Test counts are deliberately not listed for the
same reason; run the gate and read the totals, not the verdict, because a suite
that silently got smaller also reports zero failures. About six minutes cold:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

It must end `no leaks found` and exit 0.

## What is open

| | |
| --- | --- |
| **A clean clone builds; the recorded failure did not reproduce** | Kept as a watch item, not a defect. The environment it was proved in is below |
| **Model integrity is not execution-time** | The digest pass is desktop-side and the worker reopens by path. Needs a threat-model decision, not code. Narrowed in 1.9.1: setup no longer *reuses* or *stages* bytes it has not re-hashed |
| **`NotAttempted` transcripts are retained** | With auto-paste off nothing classifies the target, so history keeps the row. Disclosed rather than fixed |
| **GPU qualification cannot be proved** | Nothing can promote a card to proven, so the claim left the UI and then the payload |
| **The Hugging Face CDN host may be regional** | One host is allowed and it looks US-specific. Affects any install that fetches the model, not only the graphics-card one |
| **Five advisory allowlist entries expire 2026-10-19** | RUSTSEC-2025-0075/0080/0081/0098/0100, all `unic-*` crates reached through Tauri's `urlpattern`. The gate fails on the expiry date whether or not anything changed, so the entries need re-reviewing and re-dating, or dropping if Tauri has moved off them |
| **Focused delivery re-checks identity, not sensitivity** | A focus change *within* the target process is not caught. Closing it needs a UI Automation read at input time, which costs up to seconds |

Four of the five longstanding entries are **deliberate residuals carried forward
from 1.8.1**, not code left out by accident: the integrity gap and the
`NotAttempted` retention are documented limitations awaiting a threat-model
decision, the GPU claim stays out of the UI until a real inference sample can
support it, and the CDN allowlist is not widened by guesswork. Each is disclosed
where a user would look. The clean-clone failure is contributor-only and watched
rather than fixed. The v1.9.1 row is the current release milestone, and
the allowlist row is the only one with a deadline attached.

The focused-delivery row is new in 1.9.1 and is a residual of the fix rather
than a gap in it: delivery now re-reads the foreground window, its process and
that process's start time after every wait and immediately before the keystroke,
which closes the case where focus moved to a *different* application. Identity
is all it can afford there — a full `TargetSnapshot` costs a UI Automation
inspection measured at 68 ms into an empty Notepad and 12.8 s into a WebView2
window — so a move into a password box *inside* the same window still passes.
It is documented beside the protection in `docs/ARCHITECTURE.md` and in
`validate_foreground_identity`.

### One menu click was dispatched twice, and a switch undid itself

Found by the owner on the installed `v1.10.0`, 2026-09-22: after switching to
the processor the dock badge read `CPU`, and the dock's right-click menu went
on offering **Switch to CPU**.

`TrayIconBuilder::on_menu_event` does not scope anything to the tray. It pushes
its handler onto `manager.menu.global_event_listeners`, the same list
`AppHandle::on_menu_event` appends to (tauri 2.11.5, `src/tray/mod.rs`,
`TrayIcon::register`), and the event loop calls every listener on it for every
menu event from any surface. `composition.rs` registered
`dispatch_menu_action` on both, so every click ran it twice.

For an action that reads back what the previous pass wrote, twice is not
harmless. Pass one stored the processor override; pass two re-resolved the
target, saw the processor already effective, asked for the graphics card
instead, and cleared the preference. The resident CPU worker survived only
because the second warm found it already loaded, which is why the engine looked
switched while the preference was not. The diagnostic log carried both warms a
millisecond apart, `engine_override=applied` then `engine_override=none`.
**Reload the model** and **Close SpeakEasy** ran twice for the same reason.

Fixed in `v1.10.1` by deleting the tray builder's registration. Guarded by
`the_menu_dispatcher_is_registered_exactly_once`, a source assertion because
the defect is in composition: it happens once at startup, errors nothing, and
shows up only as a second dispatch no unit test of the dispatcher can see. It
was proved with a red control that restored the duplicate registration.

**Still unverified:** the label is derived from the resolved preference, while
the dock badge is derived from the device the worker actually ran on. Those two
agree again now that the double dispatch is gone, but nothing asserts they
must. `CLAUDE.md`'s "UI provider reporting uses the device, never the pack"
covers the badge; whether the menu's label should be re-derived from the device
as well is open, and the answer is not obviously yes — a label that names the
device can offer a switch that resolves to a no-op when a preference and a
device disagree for any other reason.

### The nine ignored tests, and how to run them

Nine tests are `#[ignore]`d, and seven of them are the only proofs here that
drive a real worker process. Two were added after `v1.10.1` and ran on an RTX
5090 against both the processor and the CUDA worker:
`granite_final_pass_hears_a_first_word_spoken_at_the_key_press` (the 300 ms
lead-in) and `a_warmed_engine_runs_its_first_dictation_as_fast_as_its_second`
(the worker's warm-up pass at load). **The other seven ran against `v1.8.1`** on
an RTX 4070 Laptop: the three processor desktop passes (including the 300 s idle gap), the
bootstrapper engine smoke, the real-`HKCU` registry test, the pinned NVIDIA
archive, and the CUDA proof, which reported `device=cuda` from NVML placing the
worker's own pid on the card.

That is a statement about one commit. Re-run them whenever the warm path moves:
a signature change that compiles is not a signature change that works.

`target/debug/proof/granite-worker.exe` goes stale between sessions, and a stale
worker fails as `StaleEvent`, which names the protocol rather than the binary.
Re-stage first.

**Seven of the nine do not require a CUDA worker.** Five of them drive the
processor worker -- the four desktop passes and the bootstrapper's engine
smoke -- and two drive no worker at all: `registry_hive` writes to the real
`HKCU`, and `the_real_nvidia` extracts a pinned NVIDIA archive. These four
commands cover all seven, and no other filter does:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Stage-DevRuntime.ps1
cargo test -p speakeasy-desktop --lib granite_final_pass -- --ignored --nocapture
cargo test -p speakeasy-bootstrapper smoke -- --ignored --nocapture
cargo test -p speakeasy-bootstrapper registry_hive -- --ignored --nocapture
cargo test -p speakeasy-models the_real_nvidia -- --ignored --nocapture
```

**The filter matters.** `granite_final_pass` selects exactly the four
processor-only desktop tests, including the 300 s idle gap (add
`--skip survives_an_idle_gap` to leave it out). The broader `granite` also selects
`a_cuda_worker_reports_the_device_its_context_probe_can_prove`, which **asserts**
rather than skips when the staged worker is not a CUDA build — so running it
after `Stage-DevRuntime.ps1`, which stages the processor worker, produces a
failure that means "wrong binary" and reads like a broken engine.

**Two need a CUDA worker staged by hand.** `a_cuda_worker_reports` asserts on
it. `a_warmed_engine` *passes* on the processor worker whether or not the
warm-up exists, because the processor build has no first-pass penalty to hide;
only a CUDA worker can fail it.
`Enable-GraniteCuda.ps1` did this and was retired on 2026-08-26 when
setup learned to fetch a published worker, so there is no script for it. The
three CUDA libraries are already in `target\debug\proof\`; only the worker has
to be replaced, and it must be done **after** `Stage-DevRuntime.ps1` rather than
before, because that script overwrites the same path with the processor build:

```powershell
cargo build --release -p speakeasy-granite-worker --features cuda
Copy-Item target\release\speakeasy-granite-worker.exe `
  target\debug\proof\granite-worker.exe -Force
cargo test -p speakeasy-desktop --lib a_cuda_worker_reports -- --ignored --nocapture
cargo test -p speakeasy-desktop --lib a_warmed_engine -- --ignored --nocapture
```

Run `a_warmed_engine` on its own. Filtered together with `a_cuda_worker_reports`
it shares the card and failed once at 308 ms against 197 ms; alone it passed
three of three.

**The published CUDA worker is rebuilt on every change to the worker or
`speakeasy-granite`**, because setup downloads it rather than taking it from the
installer: a release that changes only the payload leaves graphics-card
installs on the old worker. It is built with `CMAKE_CUDA_ARCHITECTURES` unset,
which gives ggml's redistributable list (native `sm_86`/`89`/`120`/`121`, PTX
for `75`/`80`/`90`). The 1.5.1 worker had been built with only `sm_89`, which an
RTX 30-series card cannot run. Zip it as `granite-worker.exe`, upload with
`hf upload orangeblue39/speakeasy-mini-runtime`, download the pinned revision
back and compare digests, then re-pin `granite-worker-cuda-windows-x64` in
`models/trusted-manifest.json`.

That build took about two minutes cold for a single architecture. With
`CMAKE_CUDA_ARCHITECTURES` unset, which is how the published worker is built, it
ran past ten minutes cold here on 2026-09-22; run it in the background. Afterwards the staged worker is a CUDA
build, so `Stage-DevRuntime.ps1` or `npm run tauri -- dev` silently reverts it to
the processor one — re-stage deliberately when you are done, and say which worker
was in place for any measurement you report.

`run_granite_final_pass_survives_an_idle_gap_before_a_second_dictation` sleeps
300 s by default — run it deliberately or not at all, and say which.
`registry_hive` writes to the real `HKCU` under a pid-suffixed key of its own,
which is why it is out of the default gate; it fails rather than skips if the
hive is unwritable. **Assert whole transcripts, never a prefix** — a
`contains("ever tried")` assertion once passed on a transcript missing a third of
the utterance.

### A clean clone builds

The recorded failure — `llama-cpp-sys-2`'s CMake configure answering
`No CMAKE_C_COMPILER could be found` — **did not reproduce**. Measured
2026-08-30 from a `git clone` into an empty directory with a target directory
that did not exist, twice, in PowerShell 7 with no developer shell and `cl.exe`
not on `PATH`:

| | |
| --- | --- |
| CMake | 4.4.2, ambient, `C:\Program Files\CMake\bin\cmake.exe`; nothing staged under `.tools` |
| Generator | `Visual Studio 17 2022`, platform `x64`, selected by CMake itself |
| Toolset | Build Tools 2022 17.14.37516.0, MSVC 19.44.35228.0, found without a developer shell |
| libclang | `C:\Program Files\LLVM\bin` |
| Rust | cargo/rustc 1.98.0 |
| Result | `cargo build -p speakeasy-granite --locked` finished in ~1m35s and ~1m54s; `llama.lib`, `ggml.lib` and `ggml-base.lib` written under the fresh target |

That the build is incremental here is still true and still worth distrusting, so
this is kept as a watch item rather than deleted: prove a clean checkout with a
separate target directory before calling a release green. But **the pinned-CMake
explanation is disproved** — 4.4.2 configures and compiles — so do not stage
4.4.0 expecting it to fix anything, and do not make the version a requirement.

What was real is one level down. `Enter-DevEnvironment.ps1` matched
`cmake-4.4.0-windows-x86_64` **exactly**, while `docs/NEW-MACHINE.md` tells a
reader to stage `.tools\cmake-<version>-windows-x86_64` — so a CMake staged
precisely as documented, at any other version, was ignored without a word and the
ambient one used instead. It now matches any staged version, prefers the highest,
and **prints which CMake it resolved, its version, and whether that came from
`.tools` or `PATH`**, along with the resolved libclang directory. The previous
investigation spent itself on a hypothesis a line of output would have settled.

If the failure ever returns, record the generator from
`CMakeCache.txt`'s `CMAKE_GENERATOR:INTERNAL` before anything else: every
reproduction attempt here selected the Visual Studio generator, and the reported
error is what a Ninja or Makefile generator says when no `cc` is on `PATH`.

### Model integrity is not an execution-time check

One digest pass survives per launch and it is the right one — taken immediately
before the worker is handed the `model_root`, on its own thread. But it runs in
the **desktop** process and the worker reopens the files by path;
`granite-worker`'s own docs say it checks presence rather than digests,
deliberately, because the caller verified.

Closing the gap means verifying inside the worker or handing it already-verified
handles. It needs a threat-model decision about what a local attacker who can
write to `%LOCALAPPDATA%` is entitled to do, not more code.

### `NotAttempted` transcripts are retained

`deliver_final_text` returns `DeliveryTarget::NotAttempted` without inspecting any
window on two paths: auto-paste turned off, and `dictation_retry`.
`NotAttempted.permits_history()` is `true`, so with history enabled those
transcripts are stored.

That is correct as far as it goes — no application received the text, so there is
no target to classify — but a user who dictates with auto-paste off and then
pastes into a password field themselves has that transcript on disk. The app
cannot observe a manual paste, and inspecting the foreground at some later moment
would be a guess about an action it never saw.

**Disclosed rather than fixed**, in `docs/PRIVACY.md` and `docs/USER-GUIDE.md`.
The recommendation there is to turn persisted history off before dictating
anything sensitive, because that is the only setting that holds on every path and
it is already the default. Auto-paste on does make the classification run, but it
is deliberately **not** offered as the safer configuration in general: it hands
the text to whatever holds the foreground when the transcription finishes, which
on a processor install is tens of seconds later and need not be the window the
user started in. Closing the gap properly means deciding what a non-delivering
path should inspect, and why that would be more truthful than saying nothing.

### GPU qualification cannot be proved

`GpuQualification::Qualified` carries an `ExecutionEvidence` whose
`inference_sample_count` exists precisely so a caller cannot claim success without
having inferred anything — and nothing at warm time has that number. `device=cuda`
proves a held context and loaded weights, not samples pushed through.

So the promotion from admissible to proven has no writer, the sentence it drove
was removed from Settings, and the field is now absent from `GpuStatusView` rather
than present and permanently false. Restoring it means running a real inference at
warm and counting it. Inventing the number would be the manufactured claim this
area exists to remove.

### The Hugging Face CDN host may be regional

`us.aws.cdn.hf.co` is the only Hugging Face CDN host in `redirect_hosts`, and it is
what this machine is redirected to. A user sent to a different regional host fails
closed at transfer time. **This affects the 2.30 GB of weights fetched by any
install without a verified copy already on disk**, not only the graphics-card
worker. Nothing here can test it from one country.

## Before the next release

Not blockers for the tree; blockers for cutting a build from it.

All five steps are complete for `v1.11.1`, which is published. Nothing here is
carried over: the next release starts at step 1, because the workspace version
now equals the published one and `install::decide_now` refuses an equal stamp.

1. **The version must move**, before anything is built. `install::decide_now`
   returns `RefuseSameVersion` on an equal stamp, so a rebuilt version cannot
   install over the one already published.
   `Increment-ProductVersion.ps1` moves the version, the `HKCU` stamp and the
   install manifest together — **read its whole output**, because a run that fails
   to refresh `Cargo.lock` surfaces later as a `--locked` gate failure that reads
   as a broken checkout. Its default is `-Bump Minor`; pass the one you mean.
2. **`CHANGELOG.md` must carry the new version.**
   `Build-LocalInstaller.ps1` copies it into the artifact root alongside the
   privacy, security and third-party notices, so it is shipped and not merely
   published.
3. **Run the proofs against the new build**, not a previous one. Kill any
   `ai-speakeasy-mini` first. `Build-LocalInstaller.ps1` requires `-FreshBuild`,
   so every release pays a cold build, about eight minutes here.

   ```powershell
   .\scripts\Build-LocalInstaller.ps1
   .\scripts\Test-InstallerLifecycle.ps1 -ArtifactRoot 'target\local-development\<version>'
   .\scripts\Test-SetupWizard.ps1 -ArtifactRoot 'target\local-development\<version>'
   ```

4. **Run the workflow controls when the code they guard changes**, not every
   release. Read the `Needs` column: the ones that say `nothing` run anywhere,
   and the ones that say `host` install for real and need a host with nothing
   installed, with the identity preflight passing first. None replaces the fresh
   build above.

   | Control | Run it when | Needs |
   | --- | --- | --- |
   | `Test-HostProfilePathIdentityIsScoped.ps1` | `HostProfilePathIdentity.ps1` or a proof's gating changes | nothing |
   | `Test-HostProfilePathIdentity.ps1` | before any host proof below | host, administrative share |
   | `Test-StaleArtifactRefusal.ps1` | packaging or build-boundary logic changes | packages its own artifact |
   | `Test-BuildRootContainment.ps1` | the build-root validation changes | nothing |
   | `Test-DeleteContainment.ps1` | `DeleteContainment.ps1` or any recursive delete changes | nothing |
   | `Test-ProfileCaptureIsScoped.ps1` | `ProfileCapture.ps1` changes | nothing |
   | `Test-PlantedDirectoryIsScoped.ps1` | `PlantedDirectory.ps1`, or the preflight control's planted-directory capture or removal, changes | nothing |
   | `Test-CleanupFailureRestoresConfig.ps1` | `WizardCleanup.ps1` or the wizard cleanup changes | artifact root for its end-to-end case only |
   | `Test-PreflightRefusalIsInert.ps1` | the wizard pre-flight or cleanup changes | artifact root, host |
   | `Test-ProfileRestoreOnAbort.ps1` | the config capture or restore changes | artifact root, host |

   The four host proofs are one sequence, and each calls the identity preflight
   again itself. Run the preflight first: a failure means the shell is redirected
   or cannot prove host identity independently, and none of them may be run in
   it.

   ```powershell
   .\scripts\Test-HostProfilePathIdentity.ps1

   $root = 'target\local-development\<version>'
   .\scripts\Test-PreflightRefusalIsInert.ps1 -ArtifactRoot $root
   .\scripts\Test-ProfileRestoreOnAbort.ps1 -ArtifactRoot $root
   .\scripts\Test-CleanupFailureRestoresConfig.ps1 -ArtifactRoot $root
   .\scripts\Test-SetupWizard.ps1 -ArtifactRoot $root
   ```

5. **Publish `SHA256SUMS` with the artifact**, then download the published file
   back and re-hash it. The build is unsigned by decision, so a digest a stranger
   can compare is the only assurance on offer — and uploading the right bytes and
   publishing the right digest are two separate things to get wrong.
6. **Re-read `packaging/THIRD-PARTY-NOTICES.txt` and `packaging/MODEL-NOTICES.md`
   whenever the payload changes.** Nothing checks them against
   `Build-LocalInstaller.ps1`'s actual output, and a notice describing a payload
   the installer does not have is worse than one that is merely terse.

## Settled decisions — do not re-open without new evidence

Owner decisions. `CLAUDE.md` carries the longer list and the measurements behind
them; these are what a handoff reader asks about first.

- **One engine, one pass, no fallback.** Granite produces the transcript and its
  punctuation together. A pass that fails, returns nothing, or fails the
  plausibility gate ends the dictation with a named reason. Substituting a weaker
  result is the behaviour this fork exists to remove.
- **Failures must be actionable**, on two surfaces: the dock's action row and a
  panel at the top of Settings → Transcription. A Windows toast was specified and
  refused — the WinRT route needs an AppUserModelID from an installed Start Menu
  shortcut and otherwise displays nothing *while reporting success*.
- **No provider-override control.** Granite's GPU support is a build feature; no
  setting can conjure a worker binary. The installer fetches one when the user
  asks for it on hardware that warrants it.
- **No in-app setup wizard.** The installer is the only setup path.
- **The dictation floor is Granite's floor** (8 GiB), refused before a sample is
  captured.
- **`immediate_repetitions` and `self_corrections` never run**, and have no
  toggles: `resolve_self_correction` discards everything before `" I mean "`,
  which is live data loss.
- **Q4_K_M ships; Q8_0 is catalogued and not install-eligible.**
- **Retention defaults to clearing on close, implemented by never writing** — a
  delete-on-exit is a promise a crash cannot keep.
- **An uninstall leaves nothing.** `--keep-user-data` is the testing opt-out, and
  it seeds the interactive page's check boxes rather than only working under `/S`.
- **The build is never signed.** A decision, not a deferral, so nothing may imply
  otherwise.
- **Local-only.** No GitHub Actions, no hosted runners; the build, the proofs and
  the upload are run by hand. `scripts/Test-LocalOnlyPolicy.ps1` enforces it.
- **Distribution is two paths**: the one-exe installer from GitHub Releases, and
  handing the repository to an LLM so a cautious user can watch every step. Both
  are in `README.md` and both must keep working.

## Current invariants worth knowing

State, not history. Each has a test that fails without it.

**Engine warm.** A warm's verdict is returned, never stored — `ensure_ready`
answers with an `EnsureReadyOutcome` carrying the verification *and* the error, so
a warm that hashed a pack and then failed to spawn can still condemn those bytes.
There is no "last warm" field: a dictation's own warm calls the same function, and
a field would let it overwrite a verdict the launch warm was about to act on.

**Resident packs.** The loaded identity lives with the adapter. `resident_answer`
returns `Reuse` only on an exact id-and-revision match; anything else is `Refuse`,
which carries no adapter at all, clears the slot, and reports
`granite_resident_pack_mismatch`.

**One rule refuses a dictation.** `dictation_blocker` is pure and is consumed by
both the dock and the shortcut, before any audio is captured. Every code it can
answer has catalog copy.

**History is written last, and only after classification.**
`persist_delivered_history` is the single writer and takes a `DeliveryTarget`;
`FinalizedDictation` carries the pending row and is produced only by
`publish_successful_transcript`, so a row cannot exist until the session log, the
recoverable result and the capture state have all been updated.

**The recent-transcripts list is not "this session only".** It is seeded at launch
from the optional history, so with retention on it spans earlier runs. It holds
every transcript the engine completed, including ones whose delivery was refused
or never attempted — which is the point, since a refused paste must still leave
the text reachable. `history_delete_all` removes the seeded entries and keeps what
this run produced: for a secure-target or undelivered transcript, that list is the
only copy.

**Polling.** The transcript list does not poll: one retried read on mount, then one
per `transcript-log-changed`. **That event's payload must stay `()`** — an event
reaches every listener in the window with no per-command guard, so text in a
payload would bypass `session_transcript_log`'s window check. Audio makes one
self-scheduling call rather than two on an interval. The rule that enforces all of
it is `no effect can read a race-prone command without retrying or polling`.

**Settings mutations.** Every action goes through `useMutation`, which resolves to
`null` on rejection so callers update state only on success, and refuses a second
submission while one is in flight. Every code any of them can return has catalog
copy.

**Coverage is a floor per file**, in `dependency-policy/coverage-floors.json`. A
file named there and absent from the report **fails** — that is how a coverage
check silently stops guarding anything.

**The installer proof derives its own install root once.** Every other scenario
in `Test-InstallerLifecycle.ps1` passes `--install-root`, so the branch a user
actually takes — `probe::install_root`, `%LOCALAPPDATA%\SpeakEasy Mini` — is
driven at the end of the script with `LOCALAPPDATA` and `APPDATA` redirected
under `target\installer-lifecycle`. That is also the one place the *production*
uninstall default runs for real: everywhere else passes `--keep-user-data` so a
run does not re-download the weights, and a redirected profile costs nothing to
delete. `Assert-ArpValues` pins all eight Add/Remove Programs values — including
`Publisher` and the exact `UninstallString` — against both the explicit and the
derived root, because "the key exists" is how `DisplayName: SpeakEasy` survived.

## Repository facts

- `kwp490/speakeasy-granite-rust-mini` on GitHub, public, releases included.
- The tree is ~300 files, down from 2,611 — `vendor/transcribe.cpp` alone was
  2,265 of them.
- `speakeasy-worker` links **no native libraries** and checks in seconds. Only
  `speakeasy-granite` compiles C++.
- The CUDA worker is published at `orangeblue39/speakeasy-mini-runtime` on Hugging
  Face, pinned at an immutable commit. The two NVIDIA redistributables are fetched
  from NVIDIA's own CDN and are not redistributed by this project.
- `.cargo/config.toml` is gone; **`.cargo/audit.toml` is not** — it was collateral
  damage of deleting that directory wholesale, and the gate needs it.
