# Handoff — SpeakEasy Mini, as of 2026-08-30

The state of the fork, what is finished, what is not, and the things that will
cost you an afternoon if you rediscover them yourself.

Read `CLAUDE.md` first. This file assumes it.

> **Picking this up cold? Four things, in order.**
>
> **1. There are commits on `main` that have not been pushed.** `git log
> --oneline origin/main..HEAD` is the count -- five as this was written, four of
> them code. They were held for review deliberately, not forgotten.
> `docs/handoff/NEXT-SESSION-PROMPT.md` says what is in them and what the two
> open review questions are.
>
> **2. The hardware tests have not run against any of them.** Seven tests are
> `#[ignore]`d and five of those are the only proofs in this repository that
> touch a real worker process. `GraniteEngineCoordinator::ensure_ready` changed
> signature twice in those commits and none of them ran. Everything needed to run
> them is staged on this machine. **This is the first job.**
>
> **3. Numbered items 0–18 below are history.** Read them for what they found,
> not for what to do. Items 19–28 are the current state of the areas they name,
> written as invariants rather than as a diary — the convention changed on
> 2026-08-30 and `CLAUDE.md` "Conventions" carries it.
>
> **4. A clean clone of `main` still does not build the Rust workspace**, and it
> has not been retried. See the entry below; the leading explanation is one
> untested command away.

## What is open

| | |
| --- | --- |
| **Hardware tests unrun** | Five worker-touching proofs, `#[ignore]`d, not run against the last four commits. Inputs are staged; see the next-session prompt |
| **Unpushed commits** | `git log --oneline origin/main..HEAD`. `origin/main` was at `a34adc3` when this was written |
| **Clean clone does not build** | `llama-cpp-sys-2` CMake configure. Leading explanation untested — see the entry below |
| **Model integrity is not execution-time** | The one digest pass runs desktop-side and the worker reopens by path. Item 21. Needs a threat-model decision, not code |
| **GPU qualification cannot be proved** | `GpuQualificationCoordinator::record` is gone and `qualified` is permanently false. Restoring it needs an `inference_sample_count` nothing at warm time has. Item 24 |
| **Auto-paste off leaves a history row** | Nothing inspects the foreground on a path that does not deliver, so dictate-then-paste-by-hand into a password field still records. Item 19 |


## Start here

**Read in this order:** `CLAUDE.md`, then this file, then `docs/ARCHITECTURE.md`
if you need the shape of the system.

**Prove the tree is where this file says it is**, before believing anything
below. Two commands, about six minutes cold:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

That should end with `no leaks found` and exit 0. If it does not, something has
changed since 2026-08-18 and the rest of this file is describing a tree you do
not have.

**What is true as of the last commit**, all of it measured rather than
predicted:

| | |
| --- | --- |
| Full gate | passes end to end |
| A real dictation | delivered, `hotkey_delivery result=committed` (2026-08-18) |
| **A real dictation, measured** | **done 2026-08-25, both providers.** Card: 105.2 s of speech, press-to-paste **4,246 ms**, RTF 0.0396. Processor: a 120.183 s ceiling stop, inference 44.5 s, RTF 0.3702 — **9.34x** the card. No truncation either run; the ceiling cue confirmed by ear |
| The ceiling's notice window | seen for the first time, found 16 px too short for its own copy, **raised to 192 and measured at zero overflow (2026-08-26)** — item 17 |
| Settings' status reads | **all five surfaces converted, both causes measured (2026-08-26)** — item 14. The shipped 1.5.1 renders Advanced as five headings and zero facts |
| Installer lifecycle | `Test-InstallerLifecycle.ps1` passes, including the single-file path |
| **The wizard, end to end** | `Test-SetupWizard.ps1` passes: eight pages, real install, engine check, launched app |
| Setup's engine check | transcribes the bundled clip through the real worker in ~5 s |
| `speakeasy-granite` | compiles, ~2 min cold |
| Broken doc links | none, `--document-private-items` and denied, workspace-wide (four were reintroduced on 2026-08-19 and cleared 2026-08-20) |
| **The provider a machine reports** | proved, never chosen. All five states now produced on hardware — see item 0 |
| Graphics-card path | **run on hardware** (2026-08-21, RTX 4070 Laptop): resident pass 361 ms on CUDA against 2,928 ms on the processor, transcript byte-identical. Still nothing published |
| **Setup's graphics-card download** | **live 2026-08-26.** Worker published to `orangeblue39/speakeasy-mini-runtime` and pinned at an immutable commit; a capable machine fetches four items and records `cuda` from the engine check. Seven tests inverted on the pin — item 3 |
| The CUDA pin | 13.3.1, produced by `scripts/Get-CudaRuntime.ps1` and byte-identical to this workspace's toolkit |
| Uninstall | **leaves nothing** (2026-08-21). Asks first, interactively; `--keep-user-data` is the testing opt-out both proof scripts pass |
| Branch | `main`, pushed to `kwp490/speakeasy-granite-rust-mini` |

**The installer's copy, its vocabulary box and the words it collects were
reworked on 2026-08-20** — see the section immediately below. The words it
collected had never reached the app; that was two independent defects, and the
second one was on the React side of the IPC boundary with the data already
correct on disk.

**Nothing is left of the four this file used to list.** The last of them — the
seed channel — landed on 2026-08-19 along with the distribution work around it;
see "What happened on 2026-08-19 (second session)" below for what each turned
out to involve, which was in every case more than the entry described.

**A user can now go from a bare Windows 11 machine to a working dictation
without this repository.** Download `SpeakEasyMiniSetup.exe` from Releases, run
it, answer eight pages, and the app is installed, configured and running. That
sentence was false in three separate places before this session: there was no
single file to download, setup discarded every answer it collected, and Finish
closed the window rather than starting anything.

**Setup can no longer record a configuration it did not prove** (2026-08-20,
second session). The reported failure was `engine=cpu_gpu_runtime_missing
device=cpu installed=cuda` — a claim assembled from a radio button nobody had
disabled. `speakeasy_models::granite_gpu` is now the single reader of the
question, the marker is written from the engine check's verdict, and the app
compares the two at every warm and says so when they disagree. What has never
happened is the *other* side of it: no machine has run this code with a CUDA
worker present, which is item 0.

**Two things need you rather than an agent**: publishing the CUDA worker
(item 3, needs Hugging Face credentials and the attribution review — and
deliberately *not* part of item 0), and any decision about what setup *says*,
since its copy is reviewable by rule.

**Before running the installer lifecycle test**, kill any `ai-speakeasy-mini`.
An aborted run leaves the app it launched for the running-app check alive, and
the pre-flight guard then refuses every retry.

## What happened on 2026-08-20 (second session)

The brief came from a support log: `engine=cpu_gpu_runtime_missing device=cpu
installed=cuda`. Three correct fields, an impossible combination, and nothing
anywhere that compared them.

### Where the claim came from

The install marker was written from the wizard's provider radio button, three
pages before anything ran. `UI-GUIDE.md` had said since the page was designed
that an option which cannot be installed is shown **disabled** with the reason;
nothing ever disabled it. So a user on a CUDA-capable machine selected "Use the
graphics card", setup installed the only configuration it carries, and wrote
`cuda`. The app then correctly found no GPU path, ran on the processor, and
reported the installation it had been told about.

The check that gated the option was wrong in a second, independent way: it asked
the manifest for a CUDA `final-asr` **pack**. That answers nothing about whether
a GPU path exists — there is one GGUF and a CUDA worker offloads that same file,
so a CUDA pack entry would be a duplicate of the CPU one.

### Three facts, one reader

`crates/speakeasy-models/src/granite_gpu.rs` is now the only place the question
is answered, and it keeps the three apart because they are genuinely
independent:

- **Published** — a `native-runtime` artifact `granite-worker-cuda-windows-x64`
  in the trusted manifest. Its *absence* is the declaration, which is why the
  constant and the manifest's own limitation both name the id.
- **Present** — that worker plus every library the catalog's `proof_files` pin,
  in the worker's own directory, because that is where Windows resolves a
  dynamically loaded DLL's dependencies. The file names come from the manifest
  rather than a second hand-written list; this workspace had `cudart64_13.dll` in
  a script and `cudart64_12.dll` pinned in the catalog.
- **Operational** — NVML lists the worker's **own process id** as holding a
  compute context. Not the executable's name: a second copy started by something
  else would satisfy that and say nothing about this process.

Compiled-in is not running-on, and that is the gate no static check can replace.
`compiled_accelerators` at `Hello` says what the binary could do; a refusing
driver, a claimed card or exhausted VRAM runs the same binary on the processor
and llama.cpp reports the fallback in its own stderr.

### What changed, layer by layer

- **Manifest** — the GPU limitation now names the artifact id whose absence is
  the declaration, and says why a pack would not have meant anything.
- **Wizard** — the graphics-card option is disabled when the configuration is not
  installable, and the status line names which of the three refusals applies (the
  missing-libraries one names the files).
- **Packaging** — `scripts/GraniteWorkerProvider.ps1` reads the built worker for
  `ggml-cuda` and refuses to assemble a payload carrying a CUDA worker without
  the libraries it loads. Both packagers go through it, and it reads the required
  file names out of the same manifest the Rust does.
- **Install marker** — written by `seed::record_installed_provider`, from the
  engine check's verdict, on the last page. Not with the seeds and not from a
  choice. A check that never ran writes nothing, which the app reads as
  `unrecorded`.
- **Engine check** — `smoke::ProviderEvidence` carries the three facts
  separately and `proven()` requires all of them. Its `code()` names which gate
  closed, and it is what the last page and the log report.
- **Runtime** — `ProviderIntegrity` compares the marker against what the worker
  turned out to be, at every warm: `ok`, `unrecorded`,
  `gpu_install_not_operational` (the actionable fault) or `running_beyond_record`
  (`Enable-GraniteCuda.ps1`'s own outcome, disclosed rather than treated as a
  failure). In `granite_warm` as `provider=`.
- **UI** — the disclosure reads the **device**, not the pack: `cpu`, `cuda`,
  `cuda_unverified`, `unknown`. The integrity line appears only when it says
  something, and whether an outcome is a fault is decided in Rust and sent as a
  boolean.

### What was deliberately not done

**Dictation is not refused** when the marker and the device disagree. The same
GGUF produces the same transcript on the processor, so refusing would cost the
user their dictation to make a point about provisioning. What was wrong was the
*label*, and the label is now checked, named and surfaced. The failure is loud;
the fallback it describes is not silent any more.

### Tests

Every combination is reachable without a graphics card, which was the point of
factoring the decisions out of the I/O: `WorkerProvider` and
`assess_provider_integrity` take their facts as arguments, and
`verify_engine_with` takes the NVML probe. The regression set covers GPU hardware
with a CPU payload, a published worker that was not installed, a present worker
with no runtime libraries (each missing file named), a CUDA build that never got
a context, a driver that would not answer, and a silent handshake. The two
requirement lists — PowerShell's and Rust's — are pinned against each other.

`Test-SetupWizard.ps1` additionally asserts, against the running window, that the
graphics-card option is shown and disabled, that `install-provider.txt` does not
exist before the engine check has run, and that the app's own `granite_warm` line
reads `installed=cpu device=cpu provider=ok`.

## What happened on 2026-08-20

The brief came from actually running the installer: the copy is a wall of text
nobody reads, the vocabulary box wants one word per line, and **the words it
collects never show up in the app**. The first two are writing and a text box.
The third was two independent defects with one symptom, and neither was where it
looked.

### The wizard's copy is now a question, a key line, and two sentences

Every page had three to five careful, correct paragraphs. That is the failure:
nobody reads an installer, so an honesty obligation discharged in the fourth
sentence of the third paragraph is discharged on paper and not in fact.
`catalog::Step` now carries `heading`, `key`, `key_tone` and `body` separately,
so the shape cannot quietly drift back into prose, and `catalog::Tone` marks
which line matters — ordinary, accent, warning, good.

**Colour, and — since 2026-08-26 — weight and two sizes.** Accent renders blue,
warning red, good green, verified on screen at 250% rather than assumed: a
`WM_CTLCOLORSTATIC` handler that is never reached looks identical in every
measurement. Colour is still never the only signal.

What changed is the type. This page said "and no bold", because emphasising a
label's font needs `WM_SETFONT`, `winsafe` only sends messages through an
`unsafe` call, and the workspace forbids `unsafe` — all still true, and it also
meant every control drew at `winsafe`'s one process-wide font, Segoe UI 9pt, the
size Windows uses for menu bars. The owner reported it as too small to read.
`apps/bootstrapper/Cargo.toml` now declares `unsafe_code = "deny"` for that one
crate instead of inheriting `forbid`, and `src/typeface.rs` holds the two
`#[allow]`s: body text at four thirds of the system UI font, the heading at five
thirds and semibold. Every layout height grew with it, and
`Measure-NativeWindow.ps1 -Fit` reports every label on all eight pages and on the
uninstall page fitting its box at 240 dpi.

### The vocabulary box takes a comma-separated list

`seed::parse_vocabulary` splits on commas **and** newlines, trims, drops
case-insensitive duplicates and bounds the list, and the page reports "3 words
will be added: Kenneth, Anthropic, Granite" from that same function. Echoing the
words matters as much as the count: a missing comma is invisible in "2 words" and
obvious in "1 word: Kenneth Perry".

### The words reached the disk and not the screen

Two defects, either of which alone loses the answer, and the second only becomes
visible once the first is fixed.

**One: a merge keyed on positional ids fails closed on the whole batch.** Setup's
words become dictionary entries named `installer-0`, `installer-1`, … *by
position*, and an uninstall run with `--keep-user-data` keeps
`personalization.json` -- which is what the proof scripts pass. (A production
uninstall removes it as of 2026-08-21; see item 0b.) So a
second install merged a shorter list over the old ids, left one behind, and where
the survivor held a word the new list also held the two were a `ConflictingRule`
to the dictionary validator — which rejects **every entry in the merge**. The user
got none of their words and kept the previous install's. Two words differing only
in case (`Ken, ken`) did the same on a first install. The apply site was
`let _ = personalization.add_protected_terms(...)`, so nothing was logged, nothing
shown, and the state was indistinguishable from having typed nothing.

Fixed three ways, all needed: the parse de-duplicates, `replace_user_entry_terms`
replaces the entries setup owns instead of merging them, and the outcome is
written to the log as `installer_vocabulary count=3 result=applied`. The
regression test in `personalization.rs` asserts the **old** path still rejects the
same input as a control, so it cannot pass because the collision stopped being
possible.

**Two: the Settings page read `personalization_status` once, on mount, with no
rejection handler.** Every window's webview loads while `setup` is still managing
coordinators, so that read can be refused with "state not managed for field
`state` on command …" — and the page then shows an **empty dictionary list for the
life of the process**. An empty list is not a blank page anyone reports; it says
"you have no protected terms". `useProfile.ts` had carried a retry for exactly
that race since the day it was found, with a comment naming the error string, and
nothing else did. Status reads that can lose it now go through
`readWithRetry`, and one that never succeeds says so beside the list.

This was found with the words already correct on disk and `result=applied` in the
log — i.e. after the backend was fixed and still looking broken. **When an answer
"did not arrive", check the disk and the window separately.** They are two
failures with one symptom.

### Two instruments were wrong in the same place

`Test-SetupWizard.ps1` waited for `personalization.json` to *exist* and then read
it. An uninstall run with `--keep-user-data` keeps that file, so on a reinstall
the wait returned instantly
with the previous install's words and reported the new ones lost — against an app
that had applied them correctly a moment later. It polls for the content it is
asserting now, with a deadline, so it can still fail. And `WM_SETTEXT` does not
raise `EN_CHANGE` on a multi-line edit, so setting the box and reading the count
back got the answer from before: the test presses Back and Next to force the
recompute, which also proves Back does not lose what was typed.

### Four dead doc links, cleared

`cargo doc --no-deps --document-private-items --workspace` with broken links
denied exited **101** at `4416b00` — two `[`tests::…`]` links rustdoc cannot
resolve outside `cfg(test)`, and two `[`write`]` links ambiguous between the
function and the macro. That is not one broken link, it is the whole check
unable to run, which hides every future one. `CLAUDE.md` claimed this was clean;
it was clean on 2026-08-19 and the two commits that day reintroduced it. Now two
warnings, both pre-existing and named in `CLAUDE.md`, and exit 0.

### Validated on this machine

Uninstalled (keeping the 2.14 GB of weights and the kept configuration),
re-seeded a colliding stale dictionary, rebuilt the single-file installer and
drove `Test-SetupWizard.ps1` through all eight pages to a launched app: pass. The
dictionary afterwards holds exactly `Kenneth, Anthropic, Granite` — no leftovers
— and `Invoke-WebviewProbe.ps1` reads the same three words out of Settings →
Transcription. The full gate is green and `cargo doc` exits 0.

## What happened on 2026-08-19 (second session)

The brief was "finish this project": a user should be able to go to a new
computer, download the installer, and end up with a running app. The seed
channel was the one entry this file listed. It was not the reason that sentence
was false — there were three others, and none of them was written down anywhere,
because each is a gap *between* components that every component's own tests
pass over.

### There was no installer to download

`Build-LocalInstaller.ps1` produced `speakeasy-bootstrapper.exe` **and a
`payload\` directory beside it**, because `install::payload_directory` looked
for the payload as a sibling. `README.md` had told people since the fork to
download `SpeakEasyMiniSetup.exe` from Releases — a file that no script
produced, of a shape the installer could not have used.

Nothing was wrong with any of it in isolation. The lifecycle proof passed
because it drives the developer's layout, which is the layout that exists on the
machine the proof runs on.

The fix is `apps/bootstrapper/src/payload.rs`: the payload is appended to the
finished executable past the end of the PE image, which Windows' loader ignores,
and `stage()` extracts it to a temporary directory — falling back to the sibling
directory when there is no archive, which is what keeps the developer's layout
and the lifecycle proof working unchanged.

Three things about it are worth knowing before touching it:

- **Every entry carries a SHA-256, and it is not about tampering.** The whole
  executable is untrusted until someone runs it. It is about a **truncated
  download**, which is the failure this shape invites: the missing bytes are not
  part of the program, so a half-downloaded installer *still launches*, still
  draws the wizard, and would install whatever fragment parsed. That is the only
  new silent-failure mode the design introduces, and it is closed by the digest.
- **`include_bytes!` was rejected**, and the reason generalises: it inverts the
  build order, so `cargo build -p speakeasy-bootstrapper` on a fresh tree would
  either fail on a missing file or — if that were papered over — produce an
  installer carrying an empty payload that still ran.
- **The format has exactly one implementation.** The packer is
  `apps/bootstrapper/src/bin/pack-payload.rs`, which `#[path]`-includes
  `payload.rs` rather than reimplementing the layout in PowerShell. A writer in
  the build script and a reader in the installer agree until somebody edits one
  of them, and the disagreement does not land on the build machine — it lands on
  a user, as "this download is damaged" for a file that downloaded perfectly.
  The cost is an `#[allow(dead_code)]` on the module in both binaries, since
  each uses the half the other does not.

### Setup collected five answers and discarded five answers

The recorded entry said two — the shortcut and the logging choice — and said
setup "already collects" them. It did not collect them. **Three of the eight
pages were placeholders rendering `STEP_NOT_BUILT`**: choose how it runs, choose
your shortcut, add your words. There was nothing to discard because there was
nothing to answer with, and the retention page did not exist at all.

So the work was the wizard's controls as well as the writer. `winsafe` panics if
a control is created after its parent window, so all of them are built in
`Wizard::new` and `show_questions` decides which are visible; the notice label
and the question controls deliberately share one band, because a step either
reports or asks and no step does both.

Each page carries a rule that the placeholder could not have:

- **The graphics-card option is shown, disabled, with the reason.** Granite's
  GPU support is compiled into the worker rather than loaded beside it, so no
  published CUDA worker means no graphics-card install however good the card is.
  Hiding the option reads as setup not having looked; enabling it is a control
  that silently installs something else. The predicate is
  `download::graphics_card_configuration_published`, read from the manifest, so
  it becomes true the day the worker is pinned there — one edit, not two.
  It uses `preferred_provider`, **not** `is_qualified`: qualification means an
  execution test has passed, setup has not run one at that point, and reading it
  would leave the option disabled forever even after a worker exists.
- **The shortcut is verified by taking it.** Windows will not say who owns a
  global hotkey, so setup registers the chosen combination and immediately
  releases it. Releasing is not tidiness: holding it would make setup the owner,
  and the app's own registration would then fail against a conflict setup
  created.
- **Retention defaults off**, stated as a promise rather than a checkbox —
  unticked means never written to disk, which is a stronger claim than deleting
  on exit.

`seed.rs` writes all five answers, and `every_seed_written_here_is_read_by_the_app`
reads the desktop crate's own source to assert each one has a consumer. That
test failed on its first run, which is the point of it: three of the five had no
reader yet.

**The installed configuration is a record, not a seed.** The other four are
one-shot and deleted on first launch, because a seed is a starting value and a
user's later change must win. `install-provider.txt` persists, because it
answers a question the app cannot re-derive — whether running on the processor
is what was installed or a failure to load what was installed. It shows up as
`installed=` in `granite_warm`.

### Finish closed the window

Setup ended by destroying its own window. `README.md` had described a step that
launched the app since before the fork. `launch_installed_app` now starts it and
**returns whether it started**: the wizard stays open with the reason on screen
if it did not, because closing regardless leaves someone who watched every step
succeed looking at an empty desktop.

### The wizard is now actually driven, and it found two of its own bugs

`scripts/Test-SetupWizard.ps1` runs the real `SpeakEasyMiniSetup.exe` through
all eight pages by posting `BM_CLICK` to the buttons, and asserts the heading
and the "Step N of 8" line before every click. That last part is the whole
difference between a proof and a script that presses Next eight times: the
second one passes on a wizard stuck on page one.

It also asserts against the disk rather than the window — the installed files,
the five seed files and their contents, the app process, and then the app's own
`settings.json` and `personalization.json` after it has consumed them. A wizard
that painted eight correct pages and installed nothing would have satisfied
every assertion made about the window.

Two failures in the driver itself, both worth recording because both are the
"instrument that cannot fail" shape:

- **`FindWindow($null, $title)` finds nothing, ever, from PowerShell.** `$null`
  for a `string` parameter marshals as an empty string, so it searches for a
  window whose class name is `""`. It reported "the wizard never appeared" with
  the wizard on screen. It goes through the process list now.
- **`, @(...)` handed the whole control list to `Where-Object` as one object**,
  and `$_.Class -eq 'Static'` against an array *filters* rather than compares,
  so it came back non-empty and truthy. The caller then read the entire window's
  text as the page heading. The usual guard against a one-element array
  unrolling was the bug.

And one product-adjacent finding: the vocabulary lands in `personalization.json`
a moment *after* the seeds are consumed, because the coordinator that owns it is
built later in `setup`. Reading the file the instant the last seed disappeared
found nothing and reported the words lost. The proof waits.

### Measured, on this machine

- The wizard at 250% on a 3840x2400 display: client rect **1550x1515 physical =
  620x606 logical**, exactly as declared, and **every label and control fits its
  box** by `Measure-NativeWindow.ps1 -Fit`. The longest new string — the
  diagnostic-log checkbox, 70 characters — wraps to one line in a 1470 px box.
  Re-measured 2026-08-26 after the type grew; it was 620x500 at 9pt, and every
  band kept the line capacity it had — the findings box holds 10.7 lines of a
  54 px cell against 10.4 lines of a 41 px one, and the longest thing it shows is
  eleven.
- `SpeakEasyMiniSetup.exe` is **37.5 MB**: a 7.6 MB bootstrapper plus a 29.9 MB
  payload of three executables. The payload deliberately includes a second copy
  of the bootstrapper rather than self-copying, so the installed uninstaller is
  byte-identical to the one the install manifest describes.
- Setup's engine check passed against the real worker and the real weights on an
  installed release build, which is the first time anything in this project has
  been exercised on one.
- `granite_warm result=ok engine=cpu_gpu_runtime_missing device=cpu
  installed=cpu` — the correct four facts for this machine: an NVIDIA card, no
  published CUDA worker, running on the processor, and that being what was
  installed rather than a failure.

## What happened on 2026-08-19 (first session)

Six commits, `e03eb78`..`da612fa`. Three of the four "most worth doing next"
entries above are gone; each was larger than its entry said, and the overrun was
the same shape every time — **the entry described the symptom someone had
noticed, not the condition underneath it.** Budget for that on the fourth.

### The citation sweep (`e03eb78`) — 3× the recorded size

Recorded as "25 comments across 21 files cite deleted handoff documents". It was
**35 citations across 27 files naming six deleted docs**, because the original
count searched three of the six names. None of the six was ever in this
repository's history, so most citations had nothing to be rewritten *against*.

Three citation classes exist, and a sweep that finds one looks finished:

- the **filename** (`granite-final-pass.md`) — greppable;
- the **bare number** (`§9.4`, `Phase 6`) — 96 and 22 of them, and the larger
  half of the debt. `docs/UI-GUIDE.md` does not number its headings, so nothing
  could be carried across;
- the **prose** ("the handoff", "the brief", "the GPU migration handoff, item
  14") — matches no grep for a path or a `§`.

A fourth trap: the first sweep filtered `*.ts`/`*.tsx` and the scaffold suite is
`.mjs`, which hid 21 more. **Enumerate extensions before believing a zero.**

**And a fifth, found 2026-08-28.** The bare-number class has a third spelling
this sweep never grepped for: **`decision N`**. Twenty-four of them sat across
thirteen files, pointing at a numbered decision list that has never existed
anywhere in this repository, plus a `Known risk #12` inside an assertion message
and two prose `item 17 in the handoff` references. Each one already stated the
fact it was citing, so all of them were cleared by deleting the number and
keeping the sentence. **A class list is not a pattern list**: the 2026-08-19
sweep enumerated the classes correctly and still finished with a third of one of
them untouched, because it grepped `§` and `Phase N` and stopped.

**The 2026-08-28 sweep then hit the extension trap recorded four paragraphs
above, in its own first pass, and shipped the bad glob as its artefact.** It
searched `apps\**\*.rs`, `*.ts`, `*.tsx`, `crates\**\*.rs`, `workers\**\*.rs`,
found thirteen citations, verified them clean, ran the gate, committed, and wrote
that glob into `CLAUDE.md` as the grep to use next time. Twelve more were sitting
in `apps/desktop/tests/scaffold.test.mjs` (nine),
`scripts/Invoke-TranscriberProof.ps1` (three) and `docs/UI-GUIDE.md` (one) — and
`.mjs` is the *same* extension that hid 21 citations from the sweep before it.
Everything about the first pass was verified except the thing that decided what
got looked at. **A glob written from memory is an unchecked claim about the
repository, and it fails by returning a clean answer about the files it named.**
Enumerate from `git ls-files`.

Two spellings the partial glob also hid, both worth knowing: `owner decision 1`
— a *numbered* owner decision, which a filter for the dated form does not
exclude and a search for `decision [0-9]` only finds if it reaches the file —
and the prose class, which turned up twice as `item 17 in the handoff`.

The grep, corrected. It scans 213 files and must return nothing but this
section's own account of the sweep. The `Where-Object` is load-bearing because a
dated `owner decision 2026-08-12` is a fact rather than a pointer and must not be
edited — there are six of those now against the one this class was first written
up with, so "exactly one hit" has stopped being a usable pass condition:

```powershell
$files = git ls-files apps crates workers scripts docs models |
  Where-Object { $_ -notmatch '\.(png|wav|svg|ico|icns)$' }
Select-String -Path $files -Pattern 'decision [0-9]|Known risk #[0-9]|item [0-9]+ in the' |
  Where-Object { $_.Line -notmatch 'decision 20[0-9]{2}-' }
```

**A seventh, swept 2026-08-28: the claim a dated change invalidated.** Publishing
the CUDA worker on 2026-08-26 inverted seven tests, and the tests are what said
the change had landed. Comments are what did not invert with them. Four were
still describing the world before the pin: `download.rs` said "nothing is
published, so a graphics-card plan is one item today" **three lines above the
assertion proving the opposite**, which had itself been rewritten that day;
`granite_gpu.rs` said the constant was an id a worker "will carry" and that the
artifact's *absence* was the declaration; `catalog.rs` said the honest answer for
a capable card was that the worker is unpublished, when the function had already
been rewritten to name which half is missing; and `smoke.rs` described a payload
as having "an unpublished worker".

**A test inverting is not the same as the reasoning around it being re-read.**
When a dated change flips a set of assertions, grep the prose for the state it
flipped *out of* — "nothing is published", "by absence", "will carry", "future
state", "not yet". Two spellings that survived the first pass here were the
future-tense verb (`will carry`) and the argument-from-absence, neither of which
matches a search for the word "published".

**A sixth class the citation classes do not cover: the stale window label.**
`capture.rs` said "Two windows poll it at 10 Hz (`hud` and the hidden
`hud-dock`)" and matched `"hud" | "hud-dock"` on close, for a window
`tauri.conf.json` has not declared since the fork — it left with the large HUD,
alongside `main`, `hud-dock`, `notice` and `log`. A label is a string, not an
item, so no citation grep and no rustdoc check can see it; the arm compiles, the
comment reads as fact, and a reader concludes the window exists. The arm is
`"hud-dock"` alone now and the comment says what the second half was. **Grep the
window labels against `tauri.conf.json` when a window is removed**, the same way
`every_menu_id_that_is_built_has_a_handler` compares menu ids against their
dispatch.

**Nothing pinned that arm, and the test that looked like it did cannot run.**
`scaffold.test.mjs:1542` asserts `/"hud" =>.../` against the backend source — but
it sits inside a `test.skip`, and that block sets `const backend = ""` at its
top, so the assertion is disabled twice over and would fail rather than pass if
it were re-enabled. **Seven scaffold tests are in that state**, every one of them
with `const backend = ""`, disabled by the fork itself (`3495d6e`, 2026-08-18)
with rationalisations like "covered by Rust tests" and "placeholder keeps the
architecture note visible". They read in a diff as merely skipped, which is the
shape recorded above for the `.tools/` fixtures and the deleted `granite_smoke`
rigs — reporting nothing while looking like coverage that is temporarily off.

### What the seven actually assert (measured 2026-08-28)

Every assertion in all seven was re-run against the **real** sources instead of
the stub. This is a measurement, not a reading: symbol-presence greps had put two
of these in the wrong column.

| Line | Test | Verdict |
| --- | --- | --- |
| 428 | nothing slow or blocking between the key press and the microphone | **Partial** — 6 of 10 pass |
| 491 | the graphics-card runtime is offered with its size and never silently | **Delete** — subsystem gone |
| 565 | startup model verification is explicit and failure-visible | **Revive** — 14 of 14 pass |
| 1481 | the HUD's session controls share one implementation with the shortcut | **Revive** — 9 of 10, 1 rewrite |
| 1511 | window close means different things for the transcriber and settings | **Revive** — 14 of 14 pass |
| 1562 | the transcriber is the surface a relaunch and a restore bring back | **Revive** — 6 of 6 pass |
| 1637 | a dictation that hits the duration ceiling delivers instead of vanishing | **Revive** — 8 of 9, 1 stale bound |

**Only 491 is genuinely dead.** It tests the CUDA *runtime download* —
`CudaRuntimeCoordinator`, `cuda_runtime_install_start`, `cuda_runtime_error_code`,
`cuda_runtime_status`, none of which exist; this fork has no runtime download at
all, as recorded above. Its startup-race half is superseded by the derived test
at line 1760, which is strictly better. **One fragment was worth keeping**, and
it took two goes to state correctly. The first attempt salvaged `ENGINE_LOADING =
{"cold","warming"}` together with `case "loading_model": disabled: true`, on the
claim that both were live — but `disabled: true` does not appear anywhere in the
frontend, and the pattern had been asserted against a concatenation of the whole
tree where it never matched either. What is actually live is better: the loading
set is written down **twice**, as `ENGINE_LOADING` in `transcriberState.ts` and
`ENGINE_PENDING` in `HudDockApp.tsx`, whose own comment says it is "kept in step
with" the other, and nothing checked that. The two now have to be the same set,
and a start press during the load has to be refused. Both moved into "the engine
chip never claims a device the worker has not reported".

**Both failures in the revive column were stale assertions, not drift** — but the
diagnosis of one of them was wrong, and only re-running it caught that. 1637's
ceiling watcher does still reach `transcribe_and_deliver`; the regex allowed
2,400 characters from the function head and the call sits at 3,878, because the
function grew when the notice window landed on 2026-08-25.

1481's "exactly one delivery path" counted
`deliver_final_text(&app, &text, source_reason)`, a three-argument signature whose
`source_reason` left with the streaming fallback, so the count silently became 0.
It was recorded here as "both call sites are inside `transcribe_and_deliver`,
which is the only thing that reaches delivery". **That is not true**, and it was
inferred from the call sites sitting at `views.rs:1155`–`:1156`, a few dozen lines
below `transcribe_and_deliver` at `:1079`, without reading which function encloses
them. `transcribe_and_deliver` does not deliver at all: it submits the audio to
`OrderedFinalizationQueue`. The queue's single consumer, `process_finalization_job`
at `:1138`, is the only caller of `deliver_final_text`, and the composition root
wires that consumer exactly once — which is what stops two utterances racing. The
revived assertion names the enclosing function and pins the single wiring.
**Proximity is not containment**, and a line number is not a call graph.

**A count is the wrong shape for a "there is exactly one path" rule** either way:
it fails when the signature changes and passes when a second caller copies the
same line.

The substitutions the revivals need, all mechanical: `"hud"` → `"hud-dock"` and
`StreamingEngineCoordinator` → `GraniteEngineCoordinator` (1511),
`show_transcriber` → `show_dock` and drop the `main.focus` line as duplicated by
the live test at 632 (1562), drop `admitted_asr_pack_with_preference` and
`provider_override` (565, and no provider override can ever come back — it is a
settled decision), drop `try_ready` / `build_capture_tap` /
`warm_streaming_engine` / the ONNX provider check (428).

**428 is the one worth reviving even at 6 of 10.** The four survivors nothing else
guards are the press-time UIA snapshot staying deleted (`capture_target`,
`pending_session` — measured at 68 ms into Notepad, 1.7 s into VS Code and 12.8 s
into a WebView2 window, for a snapshot that was stored and never read) and
`inspect` keeping a bounded `recv_timeout`. Those are recorded measurements with
no other check behind them.

### Done 2026-08-28 — and every revival was proved able to fail

Six revived, one deleted, `test.skip` and `const backend = ""` both gone from the
suite: **66 tests, 66 passing, 0 skipped**. Each revived test opens with an
instrument self-check, because every one of them is a set of `match`es on source
and a file that failed to load reports exactly what a real regression reports.

**Passing was never the evidence — these passed for months.** Each was broken on
purpose and had to fail: a re-added `fn capture_target`; `ENGINE_PENDING` narrowed
to `{"cold"}`; `reverify` pointed at a default spec; a second `deliver_final_text`
call outside the queue consumer; the relaunch handler switched to
`show_settings_window`; the ceiling watcher's delivery removed; the close arm
renamed. Two of those controls were themselves too weak on the first attempt and
proved nothing — adding a delivery call *inside* the consumer does not violate
"only the consumer delivers", and re-adding `"hud" | "hud-dock"` leaves
`"hud-dock" =>` matching. **A control that does not fail has not verified
anything**, and the second of those exposed a real gap: the new rule read one
label per arm, so it saw the live half of an or-pattern and missed the dead half.
It reads whole arm patterns now, and both spellings fail it.

Two findings came out of doing the work rather than the audit. `on_window_event`
had no rule that its match arms name declared windows — it does now, the same
shape as `every_menu_id_that_is_built_has_a_handler`. And **`MicPicker.tsx`
exports a component nothing imports**: it was the large HUD's device list, a 62 px
dock has nowhere to put one, and two assertions about its JSX survived only
because the test never ran — one of them naming a call site that does not exist.
It is pinned as unrendered rather than deleted, because the file is still in the
tree and the next reader will assume it is wired up. **Deleted the next day**
(`220d39d`), which took `capture_devices` and `capture_device_configure` off the
dock's allowlist with it — authority granted for that component and exercised by
nothing since the fork.

### A clean clone does not build on this machine — open, found 2026-08-28

**Everything on `main` is green, and that is not the same as buildable from
scratch.** `git clone` of `main` into an empty directory, then the whole gate:

- The **frontend half passes completely** — `npm install` then the scaffold
  suite, 68 tests, 68 pass, 0 skipped, identical to the working tree. The
  committed fixture `apps/bootstrapper/fixtures/smoke.wav` is present and
  `.tools/` is correctly absent.
- The **Rust half fails at `speakeasy-granite`**, on `llama-cpp-sys-2`'s CMake
  configure: `No CMAKE_C_COMPILER could be found`, then the same for CXX.

Not the sandbox, and not a missing prerequisite as `NEW-MACHINE.md` lists them.
It reproduces with the sandbox off, and this machine has Visual Studio Build
Tools 2022 with `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`, MSVC toolset
14.44.35207, and Windows SDK 10.0.26100.0. It also reproduces inside a
`Enter-VsDevShell` Developer shell where `cl.exe` is on `PATH`, so the generator
itself is not finding the toolset.

**The leading explanation is the pinned CMake.**
`Enter-DevEnvironment.ps1` prefers `.tools\cmake-4.4.0-windows-x86_64\bin\cmake.exe`
and falls back to whatever is on `PATH` with only a warning. That directory **does
not exist on this machine**, so the fallback is the system CMake, **4.4.2**. The
same gitignored-`.tools`-dependency-that-vanished shape as `beckett.wav` and the
CUDA archives, with a failure that names something else entirely.

**Why nobody noticed**: `target/` carries a successful llama.cpp build from
2026-08-18 and 2026-08-25, so every incremental build since has skipped configure
entirely. `Stage-DevRuntime.ps1` finished in 7.64 s during this session against
that cache. The condition is invisible day-to-day and hits a second machine, or a
`cargo clean`, immediately.

**Not diagnosed further, and not fixed.** Establishing whether 4.4.2 is the
deciding factor means staging 4.4.0 and re-running, which is the first thing to
try. Until then, treat "the gate is green" as a statement about an incrementally
built tree.

### The hardware tests, run 2026-08-28 — six pass, one correctly refuses

Run because nothing had run them in an unknown period and two earlier cases had
rotted silently while reading as merely `#[ignore]`d. The fixtures and a
CUDA-capable card are on this machine.

| test | outcome |
| --- | --- |
| `granite_final_pass_transcribes_the_fixture_through_the_real_worker_process` | pass |
| `run_granite_final_pass_reuses_the_resident_worker_across_dictations` | pass |
| `run_granite_final_pass_survives_an_idle_gap_before_a_second_dictation` | pass, did not reproduce — its documented non-failure outcome |
| `a_cuda_worker_reports_the_device_its_context_probe_can_prove` | refuses: "the staged worker is not a CUDA build", which is correct |
| `smoke::the_real_engine_transcribes_the_bundled_clip` | pass, `provider=Processor evidence=gpu_runtime_files_missing` |
| `the_real_nvidia_cudart_redistributable_extracts_at_its_pinned_digest` | pass |

**The first two failed before they passed, and the error named the wrong thing.**
Both returned `DomainError { code: StaleEvent, recoverable: true }` against a
`target/debug/proof/granite-worker.exe` staged two days earlier. `StaleEvent` is
raised in `worker_process.rs` for a protocol-version *or* request-id mismatch,
and `WORKER_PROTOCOL_VERSION` had not moved — so it reads as "this build's
protocol is broken". `Stage-DevRuntime.ps1` and an otherwise identical command
made both pass. **Nothing checks that staged binary's freshness**, and the
weights being correctly staged the whole time is what made the failure look like
a code fault. Recorded in `CLAUDE.md` beside the existing staging trap.

One number worth knowing and *not* worth writing into the recorded table: the
resident second pass measured **4.56 s** on the processor over the 6.42 s
committed clip, against the 2,928 ms `CLAUDE.md` records from 2026-08-21. That is
1.5x, on an unknown machine state, from a single run. Revising a recorded
measurement means building a rig and saying so, not editing a table because one
run disagreed.

**The backticked-identifier class was swept the same day**, by hand, against the
scanner described in `CLAUDE.md` — 2,554 backticked spans, 1,193 item-shaped,
257 resolving to nothing, of which **22 were real** and the rest were correctly
not Rust items. Two things came out of doing it that the measurement did not
predict. Half the citations named in the brief as dead (`resident_retained_pass`,
`granite_smoke`, `FinalDivergence`, the Nemotron test name) turned out to be
**correctly-written history** — "It had a second caller … until the streaming
engine left" — so a scanner hit is a question, not a finding. And three of the
real ones claimed a **test exists that does not**:
`the_policy_matches_the_app` (nothing pins the bootstrapper's download policy to
the app's — they are identical by inspection only),
`uninstall_removes_everything_unless_told_to_keep_user_data` (nothing pins
`main::remove`'s inverted default), and `every_declared_window_is_non_focusable`
(the real test is `configure_hud_reaches_every_window_that_can_show_during_a_dictation`).
The first two are now stated as gaps rather than as guarantees. **A citation of a
test is a claim about coverage**, and it is the most expensive kind to get wrong.

Three defects fell out that no citation sweep was looking for:

- `speakeasy-granite`'s crate doc claimed the delivered transcript came from the
  *streaming* model run twice. It has not since the fork.
- **Eight invisible U+009D control characters** in comments across five files,
  present since the first commit, each following an em-dash. Found only because
  a scripted replacement refused to match a line identical on screen.
- **Three broken rustdoc links**, which `cargo doc` had never reported —
  see `CLAUDE.md`'s entry on `--document-private-items`.

### 1.7.0 installed and tested on hardware, 2026-08-28 — three Settings defects

The whole cycle was run on this machine: production uninstall (nothing left —
install root, profile, ARP entry and HKCU stamp all verified gone), the Granite
pack preserved and restored, the real eight-page wizard, and a launched app.
`Test-SetupWizard.ps1` passed, the graphics-card option was offered and chosen,
the download plan carried 4 items, and setup's engine check transcribed the
bundled clip word for word **on the card**.

**The machine had been running on the processor with the CUDA payload already
downloaded and never staged** — `device=cpu installed=unrecorded` — so it was
paying ~44 s per long dictation instead of ~4 s. The reinstall staged the CUDA
worker (57,052,672 bytes against the CPU build's 4,333,056) and recorded `cuda`
from the engine check's verdict.

**Only restore the Granite pack, not the whole model store.** Restoring
everything makes setup's download page report "already here", and
`Test-SetupWizard.ps1` then *skips* its plan-count assertion by design — the one
that proves the provider radio reached `download::plan`. Restoring the weights
alone and letting the ~418 MB of CUDA payload re-fetch keeps that proof alive.
It also dropped 2.3 GB of duplicate `downloads\` GGUF copies that the old profile
had been carrying.

Then the owner dictated, and reported the app claiming GPU while suspecting CPU.
It was genuinely on the GPU — 2,365 MiB of VRAM with the worker resident against
**0 MiB** with it stopped, and a 24.3 s dictation inferred in 1,424 ms (RTF
0.059, against ~0.37 for this rig's processor). But the suspicion was well
founded, because **Settings was telling them CPU in two places**:

| defect | cause |
| --- | --- |
| Advanced `PROVIDER: Processor (CPU)` on a CUDA machine | `diagnostics_status` filled it from the pack's provider capability; the 2026-08-21 rule was applied only to Transcription |
| Advanced `WORKER: cpu_gpu_runtime_missing`, stale for the process | the page mounted eagerly and read before the worker's `Hello`; a reload returned `cpu_gpu_pack_not_installed` |
| "The graphics-card engine … has not passed its local execution check yet" | `GpuQualificationCoordinator::record` was deleted on a premise that expired when the CUDA worker shipped |

All three are fixed and pinned by tests, and `CLAUDE.md` carries both lessons.
The stale read is the interesting one: **`readWithRetry` could not have fixed
it**, because `cpu_gpu_runtime_missing` is a legitimate terminal answer on a
machine with no CUDA worker, so no `settled` predicate separates "not yet" from
"not ever" without spinning on every processor install. Mounting Advanced on tab
activation is what makes the read late enough — and it is also the only thing
that stops the RTF and latency figures being frozen at window-creation time.

**Open gap, deliberately not closed:** nothing can promote a graphics card from
`admissible_execution_untested` to `qualified`. `GpuQualification::Qualified`
carries an `ExecutionEvidence` whose `inference_sample_count` exists so a caller
cannot claim success without having inferred anything, and no warm-time caller
has that number — `device=cuda` proves a held context and loaded weights, not
samples pushed through. The honest place to promote from is a *completed
dictation* that ran on the card, with its real sample count. Until then the
device line and the provider-integrity line answer the same question from
evidence that is actually reachable, and the unreachable sentence is gone rather
than faked. `scaffold.test.mjs`'s "nothing claims the graphics card passed an
execution check that cannot pass" is one-directional on purpose and says to
delete itself if the promotion returns.

### Phase 3.5 closed 2026-08-28 — the phased-fixes brief is now empty

`docs/RUNBOOK.md` gained a "Reading `engine=` and `device=`" block under
Diagnostics: the healthy graphics-card warm line verbatim, the one-sentence rule
that `device=` is the authority on where Granite ran while `engine=` names which
*pack* was selected and why, and why the two correctly disagree on a GPU machine.
No code, no strings, no reason codes — which is what the phase asked for, and it
held.

Two things came out of doing it that the brief did not anticipate, both about
where the explanation goes rather than what it says.

**The brief's premise was half stale.** It said `RUNBOOK.md` "mentions
engine/device state and stops there". It does not: "Which engine is it running on,
and re-proving it" already documents `device=`, `installed=` and all of
`provider=`'s verdicts, including `gpu_install_not_operational`, which *is* the
`device=cpu installed=cuda` fault the phase asked to have named. So the genuine
gap was narrower than described — `engine=` alone — and writing the fault out in
full would have been the second copy the brief itself warns against. The new
block names the pattern and hands off to that section.

**The `device=cpu installed=cuda` example is not a hardware fault, and calling
it one would have been wrong.** The line the brief supplies verbatim is the
2026-08-20 support log, and that combination was *impossible* — `installed=cuda`
came from a radio button nobody had disabled, which is the defect
`speakeasy_models::granite_gpu` was written to remove. The pattern is still
reachable today (a fetched CUDA worker whose runtime libraries go missing), so
the example earns its place, but it is presented as the line that motivated the
comparison rather than as a real fallback. **A verbatim quote can carry a
provenance claim the quoting document does not intend.**

Both sample lines were checked against the emitting code rather than trusted:
`coordinators.rs`'s `granite_warm` emits `result`, `engine`, `device`,
`installed`, `provider` in that order, and `choose_granite_pack` confirms a
machine with a working CUDA worker reaches `CpuGpuPackNotInstalled` — so the
healthy line is reachable and not a document's invention.

### `install_root` (`21f2884`) — the value was already right

The recorded defect was the leaf, and the leaf had already been fixed. What
nothing had noticed is that the **fallback** returned `C:\` when `LOCALAPPDATA`
was unset: setup would have unpacked into the drive root, registered it as the
install location, and uninstall would then have walked `C:\`. Worse than the
recorded bug, and reachable from the same function.

It returns `Option<PathBuf>` now and the three writing callers refuse. Empty
counts as absent too — `PathBuf::from("").join(PRODUCT)` is a bare relative path.

**A test cannot set `LOCALAPPDATA`.** Edition 2024 under `unsafe_code =
"forbid"` makes `std::env::set_var` unsafe, so the decision lives in
`install_root_under`, which takes the environment as an argument. Any future
env-dependent decision worth pinning needs the same split.

### The two dead subsystems (`fa49173`)

Onboarding was recorded as "10 references"; it was 16 across 7 files **plus a
persisted settings field, an IPC view field, a registered Tauri command, and a
validation gate that could reject a whole settings file**. `setup_requirement`
was listed with them and is *live* — it is on the 10 Hz HUD poll.

Removing a persisted field is safe here because `Settings` has a
`#[serde(default, flatten)]` catch-all, so an older profile's `onboarding`
object lands in `extensions` and is written back. **That is pinned, not
assumed**: delete the catch-all and the migration test fails.

`proof-mode` went entirely. Its own `main.rs` comment already said no script
built it, and `run_phase2_installed_smoke` resolved a `StreamingAsr` pack that
cannot exist. Removing it also took a `not(feature = "proof-mode")` arm off the
release `windows_subsystem` attribute — an opt-out nobody could select, on the
one attribute deciding whether the shipped binary allocates a console and steals
the foreground from delivery.

### Setup's engine check (`f5e951a`, `0606275`, `da612fa`)

The spawn went to **`speakeasy-windows`**, not `speakeasy-worker`. Both are
reachable from both crates, but the spawn's hard parts — job-object ownership
and `CREATE_NO_WINDOW` — are Windows concerns already living there, and
`speakeasy-worker` depends on `speakeasy-domain` alone and checks in seconds.
`append_diagnostics_line` moved with it: it is the single redacting boundary for
diagnostics and `worker_process` writes through it, so leaving it behind would
have split that boundary across two crates.

The scaffold assertion pinning `CREATE_NO_WINDOW` followed and got stronger — it
now asserts the flag appears in exactly one place workspace-wide. Writing that
check immediately found a second `creation_flags` call, the bootstrapper's
`relaunch_detached`, which sets `DETACHED_PROCESS` for a different job. **The
rule names the constant, not the method**, so it does not sweep that up.

**The check compares words, and that is measured rather than cautious.** The
clip says

> The quick brown fox jumps over the lazy dog, and Monday begins at dawn.

and Granite `Q4_K_M` returned, 2026-08-19,

> The quick brown fox jumps over the lazy dog. And Monday begins at dawn.

A period for a comma, a capital for a lowercase, every word right. **An
exact-transcript comparison would have refused a working install.** The verbatim
pin stays in `granite_worker_smoke.rs`, where a change is a developer's finding
rather than a blocked user.

A failed check does not block the install (owner decision): Retry, with Continue
and Cancel both live, and copy that says what continuing costs. The clip is
`include_bytes!`'d rather than staged — the bootstrapper is one executable, and
that removes the "clip missing" verdict entirely.

`smoke.rs` also settled an open promise: `New-SmokeFixture.ps1` had claimed
since it was written that its sentence was "kept here and in `smoke.rs` — and
checked against each other by the bootstrapper's own test". Neither existed.

### How the engine check is proven, and how to re-run it

Not by its unit tests. `the_real_engine_transcribes_the_bundled_clip` spawns the
real worker, loads the real model and transcribes the committed clip in ~5 s. It
is `#[ignore]`d for hardware. Two env overrides let it run against an existing
install rather than a second copy of ~2 GB:

```powershell
$env:SPEAKEASY_GRANITE_WORKER = (Resolve-Path 'target\release\speakeasy-granite-worker.exe').Path
$env:SPEAKEASY_GRANITE_MODEL_ROOT = '<a directory holding both GGUFs>'
cargo test -p speakeasy-bootstrapper --offline the_real_engine -- --ignored
```

Both controls were run and both fail as they should: a nonexistent model root
gives `model_did_not_load`, and expecting a sentence the clip does not say gives
`Mismatch` carrying the real transcript. **Re-run the controls if you change the
comparison** — a smoke test that cannot fail is the exact thing this step exists
to prevent elsewhere.

On this machine the GGUFs are under the **parent** product's data directory
(`ai.speakeasy.desktop`), left by an earlier install. `.tools/` is empty.

## Where the project is

SpeakEasy Mini was forked from SpeakEasy in one session and reduced to two
changes that cascade through everything: **Granite is the only engine**, and
**the dock is the only HUD**. Five commits, on a fresh history.

Verified state, and the exact commands that produce it:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

That is the whole gate — fmt, clippy `-D warnings`, workspace tests, coverage,
the four frontend steps, dependency policy, `cargo deny`, both audits and
gitleaks. It had not been runnable at all since the fork; see below.

**As of 2026-08-18 it runs to the end and passes** — the first time it has, on
this fork. `cargo audit` finds 0 vulnerabilities and 13 informational warnings
(18 less the 5 the reviewed allowlist covers, which is also the check that
`.cargo/audit.toml` is being read at all); gitleaks scans 5 commits and finds
nothing. Getting there took six separate fixes to the gate and the things it
checks, none of which were reachable before it could start; they are recorded
below.

**Do not substitute `cargo test --workspace --lib` for it.** That was the
command this file used to quote, and `--lib` builds no `--bin` targets, which is
exactly how a bootstrapper test stayed red since the fork without anyone seeing
it. 294 Rust `--lib` tests and 36 bootstrapper binary tests pass; from
`apps/desktop`, `npm test`, `npm run lint` and `npm run typecheck` are clean —
55 frontend tests.

**There are no longer any crate exclusions, and that is the news.**
`speakeasy-granite` had not been compiled since the fork, and every command in
this file used to exclude it. It builds:
`cargo build --release -p speakeasy-granite-worker` finished in **1 m 50 s** on
2026-08-18 and produced a 4.3 MB `granite-worker.exe`. The prediction was right,
but it was a prediction until that ran.

Two prerequisites had to be satisfied first, and neither was on this machine:

- `git config --global core.longpaths true`, which `docs/ARCHITECTURE.md`
  already named as required. It was unset.
- `.tools/` did not exist at all — no staged CMake, no staged Node. The system
  CMake on `PATH` and the libclang `Resolve-Libclang.ps1` found were enough, so
  `Enter-DevEnvironment.ps1` warned about neither.

## The app has now been run, and it was broken in two places

The previous version of this file said the app had never been launched and that
this was the largest gap. It was, and launching it found what it was supposed to
find: **two leftovers from the amputation, neither covered by any test, that
between them made the app unlaunchable and dictation impossible.** Both are
fixed.

**1. Every dev launch threw before Vite started.**
`apps/desktop/src-tauri/tauri.proof.conf.json` still listed the removed
streaming engine — `inference-worker.exe`, both ONNX Runtime DLLs, both sherpa
APIs and `cargs.dll`. `Stage-DevRuntime.ps1` reads that file to learn the
installed layout, so it ran `cargo build -p speakeasy-inference-worker` against
a crate the fork deleted, failed, and took `beforeDevCommand` down with it. The
payload manifest and the uninstaller's `INSTALLED_PROOF_FILES` are pinned to
each other by `scaffold.test.mjs`, so all three moved together.

**2. `paths()` failed on every call, so every dictation would have failed.**
`RuntimeWizardCoordinator::paths()` resolved the streaming worker,
`onnxruntime.dll` and `sherpa-onnx-c-api.dll` as **required**, and did it
*before* `granite_worker` — which was `Option` and, in its own words, "never in
`required`". Those three can never exist again, so `paths()` returned
`runtime_resources_unavailable` unconditionally, `granite_worker_exe` was always
`None`, and `judge_granite_pass` would have ended every dictation with
`GraniteUnavailable`. The requirement is now inverted: the Granite worker is the
required path and the other three are gone.

That second one is worth dwelling on, because it shows how the gap hid itself.
Four tests covered `paths()`. Every one of them constructed the coordinator with
a root that does not exist, so every one asserted the *error* and not one
asserted success — a suite that was fully green about a function that could
never succeed. `a_staged_root_resolves_and_the_granite_worker_is_what_makes_it_one`
now asserts the positive case, and it was confirmed by reintroducing the bug and
watching it fail while the other four stayed green.

### What the first real dictation recorded

Measured 2026-08-18 from `%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log`,
which is the record to trust:

```text
event=granite_warm result=ok engine=cpu_gpu_runtime_missing device=cpu
event=dictation_start result=ok
event=dictation_stop result=ok
event=dictation_finalize result=started engine=llama.cpp
event=hotkey_delivery_target integrity=Equal executable=<redacted> process_id=…
event=hotkey_delivery result=committed
event=dictation_finalize result=finished
```

- 9.64 s of speech; **4.20 s** from finalize to delivery; 4.85 s total after
  stop. Delivery `committed` — a real paste into the focused window, not the
  clipboard fallback.
- `engine=cpu_gpu_runtime_missing device=cpu` is the correct code for this
  machine: an NVIDIA card with no published CUDA worker to load. This is the
  case `docs/ARCHITECTURE.md` says must be distinguishable from a chosen CPU
  install, and item 2 below is what makes it so.
- **That 4.20 s is not a measurement of anything.** It is a dev run — a debug
  desktop host around a release worker — and `CLAUDE.md` is explicit that dev
  timings have invalidated conclusions here twice. It says the path works. It
  does not say what it costs. The apparent RTF of 0.436 should not be compared
  against the recorded 0.158 until it is re-taken on an installed release build.

Also verified in that session, each against the running window rather than the
declaration:

- All three windows exist. The dock is **visible at exactly its declared size** —
  62×360 logical, 155×900 physical at this display's 250% scale — so
  `enforce_declared_size` defeats the creation-time width clamp in practice, not
  just in principle.
- The foreground window was never one of ours, before or after the worker
  spawned, and neither `granite-worker` nor its `conhost` has a visible window.
  The delivery-target trap has not reappeared.
- `granite-worker` goes resident at ~2.47 GiB with the weights loaded, and the
  desktop process falls back to ~65 MB after the warm.

One correction to `CLAUDE.md`'s guidance, found while checking this: a
`CREATE_NO_WINDOW` child **does** have a child `conhost.exe`. That flag creates
a console object and declines to display it. The "verify by the lack of a child
`conhost.exe`" rule is sound for the bootstrapper's `DETACHED_PROCESS` relaunch
and gives a false positive here; the test that actually distinguishes them is
whether the process owns a visible top-level window.

### Still not exercised

- The pinned log window has been created but never **shown**. Opening it is
  still the moment to re-check the foreground, because that is the window most
  likely to take it.
- Long dictations. The one recorded above was 9.6 s, so the ~30 s post-recording
  wait a two-minute dictation implies has not been felt by anyone yet, and the
  question of whether the dock's working indicator is enough is still open.
- ~~Anything at all on an installed release build.~~ Setup's engine check and the
  app's launch both ran on one on 2026-08-19. A **dictation** on one still has
  not, so every latency figure here remains a dev-build figure.
- **A download that actually transfers.** Every install proof so far has run on
  a machine that already had the weights, so the download step has only ever
  taken its already-satisfied branch. The resume logic is covered by
  `speakeasy-models`' own tests; the wizard's progress copy, its three phases and
  its bar are not covered by anything that has moved.

## The gate had not been runnable either

`Invoke-ScaffoldChecks.ps1` is this project's definition of green, and
`docs/handoff/CURRENT.md` told the next session to run it once and find out
whether `speakeasy-granite` still built. Running it found something else first:
**the gate threw on its own second step, and had done so since the fork.**

It asserted a version-stamped sherpa-onnx runtime under
`.tools/sherpa-onnx/current` before letting Cargo start — reasonable when
`sherpa-onnx-sys` linked against it through `SHERPA_ONNX_LIB_DIR`. The fork
removed the engine, `.cargo/config.toml`, and both scripts that block named
(`Get-NativeRuntimeVersion.ps1` and `Get-GpuRuntime.ps1`), but not the block.
So the gate died calling a script that no longer exists, with a
`CommandNotFoundException` that reads like a broken machine rather than a broken
gate — and every "the gate is green" claim in the previous handoff actually came
from running its sub-commands by hand. The block is gone; `speakeasy-granite`
compiles llama.cpp itself and there is nothing left to stage.

Two things surfaced the moment it could run to the end, and neither was
reachable by the commands that had been standing in for it:

- **`cargo fmt --check` fails are invisible to `cargo test` and `cargo clippy`.**
  The gate caught formatting in this session's own new test.
- **`cargo test --workspace --lib` builds no `--bin` targets**, so it never ran
  the bootstrapper's binary tests. One of them,
  `the_plan_names_both_engines_and_totals_their_transfer_sizes`, had been
  **failing since the fork** — it demanded the download plan name two packs,
  "one streaming pack and one Granite pack", and there is one engine now. It is
  rewritten as `the_plan_names_one_engine_and_totals_its_transfer_size`, and it
  now also pins that a GPU machine plans the same list, so the CUDA worker
  landing is what makes the count 2.

This is the previous session's "a whole crate went red unnoticed" one level
down: not a crate list this time, but a **target filter**. `--lib` is not the
workspace. Run the gate.

### The dependency policy was stale in four places

Reaching `Test-DependencyPolicy.ps1` for the first time since the fork found it
describing a workspace that no longer exists. Each of these threw in turn:

- **`.cargo/audit.toml` was missing.** The fork deleted the whole `.cargo/`
  directory to be rid of `config.toml`, whose only job was pointing
  `SHERPA_ONNX_LIB_DIR` at a pre-fetched archive — and took the advisory
  configuration with it. Regenerated, not invented: it is a projection of
  `dependency-policy/advisory-allowlist.json`, which survived intact with all
  five advisories, each carrying a package, a reason and an expiry of
  2026-10-19. The regenerated file is advisory-for-advisory identical to the
  parent repository's, which is the confirmation that the derivation is the
  right one.
- **The dependency table still called the worker crate `speakeasy-asr`**, so
  `speakeasy-desktop`'s real dependency on `speakeasy-worker` was "forbidden".
  Reconciled against `cargo metadata` rather than by hand. The entry is now one
  line — `speakeasy-worker = @('speakeasy-domain')` — where it used to carry
  `sherpa-onnx`, `transcribe-cpp`, `nvml-wrapper` and `unicode-segmentation`.
  That single line is the machine-checked evidence for this file's claim that
  the crate links nothing native.
- **Three entries named crates that no longer exist** — `speakeasy-remote`,
  `speakeasy-bench` and `speakeasy-inference-worker`. Removed, with a note
  saying they are absent deliberately, because a crate with no entry throws on
  sight and a quiet resurrection is exactly what this table exists to prevent.
- **`$allowedPatchedSources` still listed both `transcribe-cpp` path patches.**
  `[patch.crates-io]` has one entry now, the `llama-cpp-sys-2` fork.

Worth recording because it was the question underneath all of this: `cargo audit`
reports **zero vulnerabilities**. There are 18 informational warnings — 16
unmaintained, 2 unsound — of which the reviewed allowlist covers five. The
remaining thirteen are not suppressed anywhere and were not suppressed before
the fork either.

## The installer has now been built and run

`Test-InstallerLifecycle.ps1` passes end to end, which is the first
behavioural evidence about setup on this fork. Getting there took thirteen
fixes, and the shape of them is the lesson: **the fork updated every path it
executed and left every path it did not.** The dev launcher, the quality gate,
the dependency policy, the packager, the installer builder and the install
proof were all in the second category, and each one hid the next because they
fail fast.

Four were real product defects rather than harness debris, and all four were
this product writing into the parent's state:

- **`probe::install_root()` defaulted to `%LOCALAPPDATA%\SpeakEasy`.** Setup
  would have written this app's executables over an existing SpeakEasy
  installation, and `uninstall` removes the install directory whole -- more
  completely since 2026-08-21 -- so
  uninstalling Mini would have deleted SpeakEasy. `shortcut::start_menu_folder`
  had the same collision.
- **Add/Remove Programs showed `DisplayName: SpeakEasy`**, so the two products
  were indistinguishable in the list a user goes to in order to remove one.
- **The wizard called itself SpeakEasy throughout** -- 22 of 23 user-facing
  strings in `catalog.rs`, plus the window title and the repair message box.

What the passing run proves, precisely: fresh install places both executables;
refuse-while-running refuses and leaves the installed binary's SHA-256
unchanged; refuse-same-version and refuse-downgrade both fire with their own
messages; repair backup and verify round-trip; and a silent uninstall leaves
the install root and both registry keys clean, with `HKCU:\Software\SpeakEasy`
untouched throughout.

### What that proof does not cover

- **`install_root()`'s default is never exercised.** The test passes
  `--install-root` explicitly, so the worst defect above lived in a code path
  the proof does not touch. Nothing pins that leaf to the product identity, so
  it can regress in silence. A test asserting it is the cheapest insurance
  available and does not exist.
- **No model download and no transcription.** The engine smoke test still has
  no runner; the clip and its verified ground truth are committed and unused.
- **Nothing asserts the ARP strings.** The proof checks the key is created and
  removed, not what is in it. That is exactly how `DisplayName: SpeakEasy`
  survived.
- **Uninstall left an empty `HKCU:\Software\SpeakEasy Mini`** — fixed 2026-08-21,
  which **reverses the decision recorded here**. It was left alone on the
  argument that deleting a parent key should be a decision rather than a
  tidy-up, and that was right while an uninstall kept most of what it found. The
  contract changed on 2026-08-21: an uninstall leaves nothing, and removes the
  directories themselves rather than an empty tree that reads as clean. Under
  that contract an empty key carrying the product's name is the same residue one
  layer over, and the first real end-to-end uninstall left exactly it. The
  original concern is kept rather than overruled — `remove_key_if_empty` deletes
  the parent **only when it is empty**, so anything else that put a key there
  still decides its own fate.

### A rough edge in the harness

An aborted lifecycle run leaves the app it launched for the running-app check
alive. The pre-flight guard then refuses every retry -- correctly, it will not
terminate a process it does not own -- but the orphan is the script's own, and
it cost three runs to notice. Kill `ai-speakeasy-mini` before re-running.

## What happened on 2026-08-25 (fifth session)

A user report: "I ran a long transcription and it errored out. Then ran a short
one and it worked fine. I would expect the transcription process to automatically
end at the max time limit."

**The ceiling was working.** It fired at 120,176 ms, 176 ms past two minutes.
Everything after it was wrong.

### One failure, five annotations

`capture` ended with `if let Some(code) = issue_code(...) { return Err(code) }`,
and an `Err` there discards the recording. Six conditions could arrive; exactly
one of them — `frames_buffered == 0` — means there is nothing to transcribe. The
other five describe audio that exists. `judge_completion` is now a pure function
returning `Ok(Some(code))` for those, and `CapturedUtterance` carries the note
alongside the audio.

### Why it was *every* long dictation, not some of them

The first theory was the two single-event latches — one dropped callback block
or one processing overrun, latching for the whole utterance, so exposure scales
with duration. Plausible, and not what was happening. Measured:

```text
dictation_ceiling_stop result=delivering state=captured code=none quality=capture_byte_limit
```

The retained utterance costs **36 bytes per frame** — an `f32` plus a 32-byte
`ProcessedSampleMetadata`. `max_buffered_bytes` was 64 MiB, which is 1,864,135
frames, or **116.5 s** at 16 kHz, against a 121 s capacity and a **120 s**
ceiling. The byte limit bound 3.5 s inside the ceiling, so every maximum-length
recording rejected its tail, raised `BYTE_LIMIT`, and was destroyed.
Deterministic — which is exactly why the symptom was so clean.

Raised to 128 MiB. The same test now reads:

```text
dictation_ceiling_stop result=delivering state=captured code=none quality=none
```

### The test that should have caught it

`the_ceiling_stays_inside_the_pipeline_byte_limit` asserted the retained bytes
were under a hardcoded `128 * 1_024 * 1_024` while `pipeline_config` was built
with **64 MiB**. It passed at 66.5 MiB with the real limit already exceeded — an
instrument holding its own copy of the constant it exists to check. It now reads
`max_buffered_bytes` out of the config the code builds, and was made to fail by
restoring 64 MiB: *"the byte limit binds at 116 s, before the 120 s ceiling"*.

### What the user was shown

`errorUnknown` — "The operation stopped safely." Four of the five codes had no
catalog entry, and the `dictation_ceiling_stop` line logged `result` and `state`
but not the `error_code` the view was already holding. Both fixed;
`every_capture_annotation_has_catalog_copy` asserts the first against
`catalog.ts` source.

### Reaching the limit now says so

Owner decisions, taken during the session. The stop cue sounds whether or not
there is a transcript — it previously sounded only on the delivering branch, so
the one ending the user did not ask for was also the one that ended in silence.
And a `notice` window (360x172, always on top, `focus: false`, beside the dock,
15 s auto-dismiss) says the recording stopped at the maximum, the transcript was
delivered, and anything said afterwards was not recorded. It is **not** a dock
glyph (62 px) and **not** a toast (rejected: no AUMID, displays nothing while
reporting success). Shown while `deliver_final_text` is reading the foreground
window, so it must never take focus.

### Malwarebytes ate the toolchain, and the pin moved

Mid-session the gate began failing with `could not execute process
clippy-driver.exe ... (never executed)`. Malwarebytes was quarantining
`clippy-driver.exe` and `rustdoc.exe` out of `1.97.1-x86_64-pc-windows-msvc` as
`Malware.AI.3172041259`, within ~16 s of each rustup extraction, three times.
Byte-identical copies in the `stable` toolchain were untouched. Unsigned is
normal for official Rust, so the signature says nothing. Owner excluded the exact
versioned toolchain directory and moved the pin to **1.98.0**, which is
unaffected. `chunks_exact_to_as_chunks` is new in 1.98 and applied to four PCM
decoders; `Enter-DevEnvironment.ps1`, `Invoke-ScaffoldChecks.ps1`, `Cargo.toml`,
`CONTRIBUTING.md` and `TESTING-ON-WINDOWS.md` all moved with it.

## What happened on 2026-08-25 (sixth session)

A person read a prepared passage into an installed release build, twice — once
on the graphics card, once on the processor. That had never been done. Every
latency figure in this repository came from a test harness until this session,
and the two things everybody expected to find were both absent while four
things nobody was looking for turned up instead.

### The numbers, on real speech

Both runs read the same 230-word passage, deliberately invented so that a
fluent transcript could not pass by reciting something the model already knew —
the detached-audio-projector failure presents exactly as confident, unrelated
prose. Reading a famous text would have made a correct-looking transcript
worthless as evidence.

| | Card | Processor |
| --- | --- | --- |
| Audio captured | 105.248 s | 120.183 s (ceiling) |
| Inference | **4,171 ms** | **44,493 ms** |
| RTF | **0.0396** | **0.3702** |
| Press-to-paste | **4,246 ms** | n/a — the ceiling ended it |
| Delivery | `committed`, into the intended target | `committed`, into the *wrong window* — item 13 |

**The processor is 9.34x slower than the card**, against the 8.1x the 6.42 s
fixture predicted. Close enough that the fixture is a fair guide, and the
direction of the error is worth knowing: the fixture was optimistic about the
processor. RTF is the only figure here that survives the two runs having
different audio, because it normalises for duration.

The full decomposition of the card's press-to-paste, which is the first
end-to-end number this product has ever had: 54 ms from the stop press to the
finalisation job starting, 4,171 ms of inference, 21 ms to inspect the
foreground window and paste. Inference is 98% of it. Nothing else is worth
optimising until that is.

### Neither expected failure happened

**No truncation, either run.** The passage ended in six arbitrary words —
`pelican, brickwork, Tuesday, ninety-one, verdigris, and stop` — as a tripwire,
because a truncation is *precise* and plausible and the plausibility gate only
looks for transcripts that are too long. All six words arrived both times. See
item 11 for why this was never going to fail, which is the finding.

**Granite did not answer the prompt.** The passage contained a direct question
(*"Does any of that sound plausible to you?"*) as bait for the one failure
`is_plausible` still guards. Both runs transcribed the question. The guard was
never exercised, so it remains untested against a real occurrence.

### The transcripts, judged

90.0% (card) and 91.3% (processor) exact word match, but the raw figure
understates the result badly, because most of the difference is **normalisation
that a dictation product should do**: `twenty-fifth` to `25th`, `forty-two` to
`42`, `ninety-one` to `91`. The card turned "one thousand six hundred pounds"
into `£1,600`, inferring the currency and moving it to a prefix symbol.

The real errors fall into three classes, and the third is the one that matters.

- **Invented proper nouns**, expected and uninteresting: `Halloway` became
  `Holloway` both times.
- **Punctuation and structure.** A spoken comma-separated list came back with
  its items capitalised like proper nouns and its commas missing — entirely on
  the card, partly on the processor. The card also moved a sentence boundary in
  a way that changes the meaning: "to build a causeway instead. It was never
  built" became "to build Causeway. Instead, it was never built". A reader
  cannot detect that as an error, because both readings are grammatical.
- **A corrupted number.** The processor rendered "one thousand six hundred
  pounds" as **`£1,1600`**. This is the most dangerous single error either run
  produced and it is a different kind from the others: a wrong *figure* wearing
  valid formatting. Fluency review cannot catch it, `is_plausible` cannot catch
  it, and the user has no reason to doubt it. Nothing in this product guards a
  number.

### What this session cannot claim, and why it is worth writing down

The two providers agreed on 94.7% of words, and **that number must not be read
as a determinism comparison.** The passage was read twice by a person, so the
audio differed — 105.2 s against 120.2 s — and some differences plainly track
the speech rather than the hardware ("gives *us* the cost" is most likely what
was said). `ARCHITECTURE.md`'s **byte-identical on both devices** claim was
established by pushing one fixed WAV through both workers, which is a different
and still-valid experiment. This session neither confirms nor contradicts it,
and a future reader finding two provider transcripts side by side in this file
would be entitled to think otherwise unless told.

Doing it properly needs the same audio through both providers, which is what
the fixture test already does. There is no way to get it from a microphone.

### The ceiling fired for real, and the cue was heard

The processor run overran two minutes. `dictation_ceiling_stop result=delivering
state=captured code=none **quality=none**` at 120,183 ms — 183 ms past the
ceiling, matching the 176 ms of the original user report closely enough to
confirm the same path.

That line is the 1.5.1 fix confirmed against reality rather than against a test.
This was a genuine maximum-length dictation; under 1.5.0 it would have read
`quality=capture_byte_limit`, `capture` would have returned `Err`, and the whole
two minutes would have been destroyed. The byte limit at 128 MiB now binds at
~233 s, far outside a ceiling it used to bind 3.5 s inside.

**The stop cue was confirmed by ear** — owner report, 2026-08-25. That closes the
one thing the fifth session shipped without being able to verify.

**The notice window was seen, and it is broken** — item 17. Its content needs
188 CSS px against a declared 172, so the dismiss button sits 16 px below the
fold behind a scrollbar. It has clipped on every machine since it shipped, and
nothing found it for the same reason nothing found anything else about it: the
window had never been looked at. A feature whose first observation is a defect is
an argument about coverage, not about that window — a `notice` that only appears
after two minutes of continuous speech is not reachable by any test in this
repository, and the two proof scripts that drive real UI (`Test-SetupWizard.ps1`,
`Test-InstallerLifecycle.ps1`) never see the app's own windows at all.

## What is outstanding

Ordered by what unblocks the most.

### 0. Prove the graphics-card path on this machine — done 2026-08-21

All four rows of this item's acceptance table are satisfied on an **RTX 4070
Laptop GPU** (compute 8.9, driver 596.36, 8,188 MiB). Nothing was published:
`models/trusted-manifest.json` still carries no
`granite-worker-cuda-windows-x64`, `download::plan` still has one item, and the
wizard's graphics-card option is still disabled with the reason.

**The first step was the whole blocker it was described as.** The catalog pinned
CUDA 12.9 while every machine that can build a CUDA worker ships 13.x, and the
requirement had become *enforced* the day before, so a locally built worker was
refused for three libraries sitting beside it under their real names. Two things
the plan did not know:

- **CUDA 13 moved the libraries from `bin/` to `bin/x64/`.** A re-pin is not the
  12.9 paths with a digit changed, and the first attempt refused for exactly
  that reason. `required_cuda_runtime_files` survived the move only because it
  reduces `proof_files` to base names instead of stripping a known prefix.
- **`scripts/Get-CudaRuntime.ps1` is new**, and is how the entries are now
  produced: it cross-checks each archive against NVIDIA's own
  `redistrib_13.3.1.json`, takes the per-file digests from the verified archive
  because NVIDIA publishes none, and reports whether the installed toolkit's
  libraries are the *same bytes*. All three matched, which is what makes
  `Enable-GraniteCuda.ps1` staging from the toolkit sound — version strings
  agreeing would not have been that claim.

**What was measured.** Through the app's own resident path on the 6.42 s clip:
resident pass **2,928 ms on the processor against 361 ms on CUDA**, RTF 0.456
against 0.0563, a factor of 8.1. Not comparable to the RTX 5090 figures in
`ARCHITECTURE.md` — different card, different clip, different harness. **The
transcript is byte-identical on both devices**, which matters more than the
speed: setup's engine check compares a whole transcript against one pinned
ground truth, so a CUDA path that moved a single punctuation mark would have
failed that check on every graphics-card machine, and nothing would have found
out until one existed.

#### The acceptance table, with its evidence

| State | Evidence |
| --- | --- |
| `provider=ok` on the card | `granite_warm result=ok engine=cpu_gpu_pack_not_installed device=cuda installed=cuda provider=ok`; the running Settings window read `Dictation runs on: Graphics card (GPU)` with **no** integrity element in the DOM |
| `running_beyond_record` | `... device=cuda installed=cpu provider=running_beyond_record`; Settings rendered the not-a-fault copy with class `setting-detail`, not `warning` |
| `gpu_runtime_files_missing` | `--verify-provider` exited 1 with "The graphics-card engine cannot start: cublas64_13.dll is not beside the worker", and left the marker alone |
| `cuda_unverified` | `a_cuda_worker_reports_the_device_its_context_probe_can_prove`, on hardware: the real probe gives `cuda`, a staged `Err(LibraryMissing)` gives `cuda_unverified`, a staged `Ok(vec![])` gives `cpu` |

Both new assertions were also made to **fail** on purpose before being believed.
The `--verify-provider` argument parser was given back the first-fragment bug and
its test failed; the hardware test was pointed at a non-CUDA binary and its
`ggml-cuda` guard fired.

#### Two rows were unreachable as written, and why

`inspect_gpu_payload` answers **published, then present, then operational**, in
that order, and `WorkerNotPublished` short-circuits. So `payload: None` — which
`ProviderEvidence::proven()` requires before anything may record `cuda` — is
impossible while the artifact id is absent from the manifest. **Rows 1 and 3
therefore contradict this item's own "publish nothing" decision.**

They were produced by splicing a `granite-worker-cuda-windows-x64` entry into
the catalog locally, building a bootstrapper from it, taking the evidence, and
reverting; the manifest was then compared against its pre-splice state and is
byte-equivalent. What those two rows prove is that **the code path works once a
worker is published**, not that any release on this machine records `cuda`.
Anyone repeating it needs the same splice, and it has to be reverted before the
gate runs, because `the_shipped_catalog_publishes_no_graphics_card_worker` is
designed to fail against it.

#### Three findings this proof produced, none of them fixed

Each was found by running the thing rather than reading it.

1. **The engine-reason sentence contradicts the device beside it.** Settings
   renders, on this machine, verbatim: `Dictation runs on: Graphics card (GPU) —
   this computer's graphics card is supported, but the graphics-card model is not
   installed, so the processor model is being used.` The device is right, the
   reason code (`cpu_gpu_pack_not_installed`) is right, and the *sentence* is
   false. `ARCHITECTURE.md` predicted the two facts would disagree on exactly
   this machine, and nobody had looked at the rendered line.
2. **`cudart64_13.dll` is required and never loaded.** The CUDA worker's image
   names `cublas64_13.dll` and `nvcuda.dll` and does **not** name
   `cudart64_13.dll` at all — ggml links the CUDA runtime statically on Windows.
   So the catalog's requirement list is a superset, `granite_gpu.rs`'s claim that
   a CUDA build "links `cudart` and `cuBLAS` dynamically" is false for this
   build, and a payload missing only cudart is refused despite being able to run.
3. **`recorded=cuda` with `device=cuda_unverified` reports the actionable
   fault.** `assess_provider_integrity` returns `GpuInstallNotOperational` there,
   which asserts "dictation is running on the processor instead" on evidence that
   proves nothing either way — the one inference `granite_gpu.rs`'s own header
   says must never be made. Reachable when NVML stops answering on a machine
   where setup proved `cuda`.

#### Traps found while doing it

- **The machine that can build a CUDA worker is the one machine where a missing
  library cannot be reproduced.** The CUDA Toolkit puts `bin\x64` on `PATH`, so
  Windows resolves a library deleted from beside the worker and it starts anyway.
  Deleting `cublas64_13.dll` produced a *verified* run and a correct demotion to
  `cpu` rather than a refusal; the refusal needed the toolkit stripped from
  `PATH` for the invocation.
- **`.tools/fixtures/beckett.wav` is gone**, and all three `granite_engine`
  hardware tests read it, so they had been unrunnable for an unknown stretch
  while reading as merely `#[ignore]`d. They now use the committed
  `apps/bootstrapper/fixtures/smoke.wav`, whose ground truth was discovered by
  running the model. Second time this repository has lost a gitignored fixture.
- **`--nocapture` delivered nothing from `speakeasy-desktop`'s test binary**, and
  `--show-output` reported its stdout as empty, so both resident-run
  measurements had been printing into a void while passing. The timing is now
  written to `target/debug/granite-resident-timing.txt`. Cause not found.
- **An edit landed in a sibling hardware test with a similar name**, and the test
  filter selected the other one, so everything passed while producing nothing.
  Check which test ran, not that one did.
- **`Enable-GraniteCuda.ps1` still reverts on any reinstall or upgrade.** The
  payload copy overwrites the staged worker. Re-run it afterwards, and do not
  read a `device=cpu` after an upgrade as a regression.
- **So does `npm run tauri -- dev`, one directory over.** Found 2026-08-21:
  `beforeDevCommand` runs `Stage-DevRuntime.ps1`, which copies the CPU worker
  over `target/debug/proof/granite-worker.exe` — 57,042,432 bytes became
  4,333,568. That silently reverts a staged CUDA worker *and* disarms the three
  `granite_engine` hardware tests, which read the same path, without failing
  them. To look at the graphics-card path in a dev build, restore the worker and
  start `npm run dev` and `target/debug/ai-speakeasy-mini.exe` separately.

#### Where this machine was left

Install at `%LOCALAPPDATA%\SpeakEasy Mini`, **version 1.5.0**, with the **CUDA
worker staged** and its three libraries beside it, and `install-provider.txt`
reading `cpu` — so the app reports `device=cuda installed=cpu
provider=running_beyond_record`, disclosed as not-a-fault. That is the honest
resting state for a machine carrying a worker no release publishes.

Reached the hard way on 2026-08-21: uninstalled for real (weights included),
reinstalled by `Test-SetupWizard.ps1`, then re-staged with
`Enable-GraniteCuda.ps1`, which re-proves through `--verify-provider` and
recorded `device=cpu evidence=gpu_worker_not_published`. The last warm reads
`engine=cpu_gpu_pack_not_installed device=cuda installed=cpu
provider=running_beyond_record`.

**Now on 1.5.1** (2026-08-25), same shape: the release proofs uninstalled and
reinstalled it, and `Enable-GraniteCuda.ps1` was re-run afterwards as it always
must be. The toolchain is **Rust 1.98.0**; 1.97.1 was reinstalled through rustup
after the Malwarebytes quarantine and is intact and unused.


### 0b. Uninstall leaves nothing — done 2026-08-21

Owner decision, taken during the graphics-card session and shipped separately
from it. `--uninstall` removes the program directory *and* the profile — settings,
transcript history, the 2.14 GB of weights, recovery backups, and now the logs —
and removes the directories rather than leaving an empty tree that reads as
clean. The interactive path asks once with the full scope named. Keeping things is
`--keep-user-data`, a **testing** affordance for install/uninstall cycles, and
both proof scripts pass it.

**It began as a preference and turned out to be a correction.** The question was
whether unrecognised files in `proof/` should survive an uninstall. They survived
because of a rule that emptied that directory selectively, on the recorded
argument that an unknown file there was probably 500 MB of fetched CUDA runtime,
and that "leaving a file costs a few megabytes, deleting one costs a 2.97 GB
download". Every word of that was true when written. **None of it was true any
more:** this fork has no runtime download. It left with the streaming engine.
Nothing in the tree creates `.cuda-runtime-download` or `.cuda-runtime-stage`, and
the weights live under `%APPDATA%`. `Removable::GpuRuntime` — labelled "Downloaded
graphics-card runtime (about 2.3 GB)" — described a thing that could not exist.

Caught live, and worth keeping as the shape of it. Uninstalling the pre-change
build printed:

```text
SpeakEasy Mini has been removed.
Removed: version record, Add/Remove Programs entry, Start Menu shortcuts, program files
Kept: downloaded graphics-card runtime, configuration, transcript history, installed models, recovery backups
```

while leaving 517 MB of `Enable-GraniteCuda.ps1`'s staged libraries behind. Three
true-sounding lines, one of them naming a runtime this product has never
downloaded, and a machine the user believed was clean.

**What is where.** `Removable::Logs` took the retired slot, because without it
`everything()` left the logs directory and therefore the profile root behind.
`remove_program_files` lost its flag and empties `proof/` in two passes — declared
names as "program files", everything else reported by name in
`Outcome::removed_unrecognised` — so the confirmation can list them and the report
can distinguish them. `unrecognised_proof_files` exists to be asked *before*
anything is deleted. `Removals::default()` still selects nothing: a caller that
forgets to ask must still delete nothing, and the inversion belongs at the command
line where somebody has actually been asked.

**`--remove-all` is gone and is deliberately not an alias.** It named the thorough
behaviour and that is now the default; a flag meaning "do what you were going to
do anyway" lets a caller keep believing it is choosing. It is refused with the
misuse message, so whoever passes it reads the change.

**The per-item checkbox page was built on 2026-08-21** (`uninstall_page.rs`), and
with it the owner reversed the focused-button call: the page *is* the
confirmation, there is no dialog behind it, and its Remove button is the focused
one. A second prompt re-asking what the page just asked is the
sequential-prompts-answered-blind shape this module's header warns about.

Three things about it worth keeping:

- **`BS::DEFPUSHBUTTON` makes a button the default and does not focus it.**
  Measured both ways: with the explicit `SetFocus` removed the focus lands on the
  **heading static** — not the first check box, and not either button — so a page
  relying on the style alone would have shipped with Enter doing nothing.
- **Only the models entry names a size, and `uninstall::measure` walks the same
  path table the deletion does.** A figure derived from a second copy of those
  paths could describe one set of files while another is removed. The label this
  descends from said "about 2.3 GB" for a download this fork never had, which is
  what a written-down size eventually becomes. It read
  `Downloaded speech models (2.3 GB)` on this machine.
- **Cancel, the close box, and a window that could not be drawn all mean remove
  nothing**, because a page nobody saw is not consent. Driven for real: the page
  was launched against this machine's live installation, Cancel clicked, and the
  install root, the weights and the three staged CUDA libraries were all still
  there afterwards.

Measured at 250% with `Measure-NativeWindow.ps1 -Fit`: client rect 480x398
logical, every control fitting its box, the unrecognised-files block wrapping to
four of the roughly five lines it reserves. Re-measured 2026-08-26 after the type
grew: 560x494 logical, every control still fitting, and the unrecognised block
still reserving about six lines of the larger cell.

#### The real weights were finally deleted, and it found one thing

The production default had only ever been proved against a staged profile root.
On 2026-08-21 it was run for real, from the page, against this machine's live
installation: 2.14 GB of weights in the profile and 0.56 GB of program files,
including the three staged CUDA libraries reported as unrecognised. Both roots
went, along with the Add/Remove Programs entry and the Start Menu folder.

**What survived was `HKCU:\Software\SpeakEasy Mini`** — empty, no values, no
subkeys, carrying the product's name. `VERSION_KEY` is
`Software\SpeakEasy Mini\LocalDevelopment`, so `delete_subkey_all` takes
`LocalDevelopment` and stops. It is harmless — the version *value* is gone, so
the next install is not refused — and it is exactly the residue the same day's
work went out of its way to remove one layer down, where directories stopped
being left behind empty. `remove_key_if_empty` is now the registry counterpart of
`remove_directory_if_empty`, with the same rule: only when empty, because
something else putting a key there is not this uninstaller's to guess about.

Worth knowing before someone repeats this: **the weights survived anyway**, in
`target/debug/model-lifecycle`, because that tree is hardlinked to the profile's
copy and a hardlink keeps the data alive until the last link goes. So the
"long re-download" this exercise was deferred for is avoidable on a machine that
has the dev tree.

**Proofs.** Gate exit 0. `Test-InstallerLifecycle.ps1` passes, with its
"unexpected files in the install root" assertion replaced by "the install root
must not exist" — the comment there previously said the opposite was deliberate,
so it was rewritten rather than edited. The new Rust test was made to fail by
deleting the profile-root cleanup, and the lifecycle assertion was made to fail
for real by holding a handle open on an unrecognised file in `proof/`, which
produced:

```text
SpeakEasy Mini was only partly removed.
Could not be removed:
  proof/held-open.dll: The process cannot access the file because it is being used by another process. (os error 32)
```

exit 1, install root surviving. What has **not** been exercised is deleting a real
2.14 GB of weights: the production default was proved against a staged profile
root, not against this machine's, because doing it for real means a long
re-download. Someone should do it once on a machine they do not mind re-seeding.

### 1. Run the app end to end — done 2026-08-18
See above. It found two blocking defects; both are fixed and the first real
dictation delivered.

**An installed release build has now been exercised** (2026-08-19): setup's
engine check loaded the real weights and transcribed the bundled clip through
the installed `proof\granite-worker.exe`, and the app started from Finish and
logged `granite_warm result=ok`. What still has not happened on one is a
**dictation** — that needs a person and a microphone, and every timing figure in
this file is therefore still from a dev build. The pinned log window and a long
dictation remain from this item too.

### 1b. A real dictation on an installed build — done 2026-08-25

**Done, both providers.** The measurements, the transcript judgement and the six
findings are in "What happened on 2026-08-25 (sixth session)" above; the four
that are defects are items 11 to 14. The short version: press-to-paste on the
card is **4,246 ms** for 105 s of speech, of which inference is 98%; the
processor is **9.34x** slower; no truncation and no prompt-answering in either
run; the ceiling fired for real with `quality=none` and its cue was heard.

**Still unverified after this session**, stated plainly because the temptation is
to read a successful run as covering more than it did:

- ~~**The notice window.**~~ Seen, and it is **16 px too short for its own copy**
  — item 17. Both halves of the ceiling's feedback are now verified rather than
  asserted, and one of them was broken.
- **`is_plausible` against a real occurrence.** The bait question was
  transcribed rather than answered, so the only guard left has still never had
  to fire on real speech.
- **Provider determinism on identical audio.** Not testable from a microphone at
  all — see the caveat in the session section. The fixture test is the instrument
  for this and it already exists.
- **Any dictation on a machine that is not this one.** One rig, one microphone,
  one voice, one accent, one passage.
- **A dictation whose transcript contains a number the user relies on.** The
  `£1,1600` error says this is the sharpest untested edge, and there is no guard
  for it.

The procedure below is left in place because it is the thing to repeat on the
next machine, not because it is outstanding here.

<details>
<summary>The original procedure, for repeating this on another machine</summary>

Nobody has spoken into an installed release build. Setup's engine check
transcribes a bundled clip through the real worker, and the app warms and logs
`granite_warm result=ok`, but a *dictation* needs a person and a microphone — so
every timing figure in this repository is still from a harness. With a CUDA
worker stageable on this machine, a long dictation on the card is the single most
informative thing left.

**Do it on an installed build, in release, not on `tauri dev`.** A debug build's
SHA-256 dominates any timing that verifies a model — 17.5 s in debug against
2.36 s in release on the same rig.

#### Before you start

> **Superseded in one respect (2026-08-26).** The staging steps below invoke
> `scripts\Enable-GraniteCuda.ps1`, which is retired. Setup installs the
> graphics-card worker itself now, so the way to get a card into this measurement
> is to choose the graphics card on setup's provider page. The rest of this
> section — measure in release on an installed build, what to watch, what each
> signal means — is unchanged and is why it is kept rather than rewritten.

1. `Get-Process SpeakEasy*` — nothing running, or the single-instance lock
   silently absorbs the launch and you test the old binary.
2. Stage the graphics-card worker and re-prove the record, in that order:

   ```powershell
   .\scripts\Enable-GraniteCuda.ps1
   ```

   It calls `--verify-provider` itself. Skipping the re-prove is not neutral — it
   manufactures `running_beyond_record` out of a machine that is fine.
3. Open Settings → Transcription and read the disclosure before speaking. It
   should say `Dictation runs on: Graphics card (GPU)` with **no** integrity line
   beneath it if the marker was re-proved, or the not-a-fault
   `running_beyond_record` sentence if it was not.
4. Open the pinned log window, or tail
   `%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log`.

#### What to say

**Two to three minutes, continuously.** Not a sentence — the interesting failure
is length. `max_new_tokens` is 2048 and a 120 s clip needs roughly 400 tokens for
312 words, so a three-minute dictation is the first thing anybody has run that
gets within sight of the ceiling. Read from something you can compare against
afterwards, because you will need to check the *end* of the transcript rather
than the beginning.

Do it twice if you have the patience: once on the card, once after
`Enable-GraniteCuda.ps1 -Revert`, from the same script. That gives the first
processor-versus-card comparison on real speech rather than a 6.42 s fixture.

#### What to watch, and what each thing would mean

| Signal | Where | What it means |
| --- | --- | --- |
| `granite_warm result=ok engine= device= installed= provider=` | log | The provider four-tuple. `device=cuda` is the one to confirm before you speak |
| `hotkey_delivery result=committed` | log | The transcript reached the target application. Anything else is a delivery outcome, not a transcription one |
| `target_inspect_refused` | log | The foreground window was something SpeakEasy Mini owns, so it fell back to the clipboard. Reads like a delivery bug and is not one |
| The **last clause** of the transcript | the target window | The `max_new_tokens` ceiling stops generation with no error and no end-of-generation token. A truncation is *precise* and plausible, so nothing downstream catches it — you are the only instrument |
| Elapsed between the second hotkey press and the paste | a clock | The only real end-to-end number this product has |

**Do not paste into a Notepad window you did not create.** Windows 11 Notepad
restores its previous tabs, and a dictation proof has already written into
somebody's real unsaved note. Create a file, open it, and confirm its name in the
title bar first.

#### What would make it a finding

A transcript that ends mid-clause is the one everybody is looking for and nobody
has been able to produce. A transcript that is fluent, confident and *unrelated*
to what you said is Granite answering the prompt instead of transcribing, which
`is_plausible` is supposed to catch and which is the only guard left.

</details>

**Two corrections to the procedure above**, both found by running it on
2026-08-25 and both left in place rather than edited out, because the reasoning
that produced them is the interesting part:

- **"Two to three minutes, continuously" cannot be done.** The capture ceiling
  is 120 s, so a three-minute dictation is not a long recording — it is a
  120 s recording plus a minute of talking to a stopped microphone. Aim at
  ~115 s and size the passage for the reader's actual pace. Item 11 has the rest.
- **Step 3's "with no integrity line beneath it" is unreachable.** Re-proving the
  marker does not clear `running_beyond_record` and cannot, because the published
  gate fails on every machine until item 3 ships. Item 12.

### 2. Finish the installer (`apps/bootstrapper`) — done 2026-08-19
Everything this entry listed exists and is proven: the hardware probe, the
resumable digest-verified download, the native wizard with all eight of its
pages built, Start Menu shortcuts, WebView2 detection, the uninstaller, the
engine smoke test, the seed channel, the installed-configuration record, and
the launch. `Test-SetupWizard.ps1` drives the whole of it against a real
install.

The engine smoke test's original entry is worth keeping, because its reasoning
is why the step exists rather than what was left to do:

> A speech model whose audio projector failed to attach does not error. It
> answers the prompt from the instruction alone and writes fluent, confident,
> entirely invented text. "It returned a transcript" is therefore evidence of
> nothing, and only matching *content* separates a run that read the waveform
> from one that did not.
>
> Its ground truth is **verified, not typed**:
>
> ```text
> The quick brown fox jumps over the lazy dog. And Monday begins at dawn.
> ```
>
> Both guesses at it were wrong, which is the argument for
> `cargo run --release -p speakeasy-granite --example transcribe_file`
> existing at all. The first sentence ended "and Granite writes it down" and
> the model returned "Granit". The second was spoken with a comma — "dog, and
> Monday" — and Granite chose "dog. And Monday", a punctuation decision nobody
> would have written from memory. Swept across every thread count
> `recommended_thread_count` can return (1 through 8) and byte-identical at all
> of them, which is what makes a whole-transcript comparison safe on a machine
> whose core count nobody chose.

**Two smaller things this entry does not cover, and nothing else does either.**
Neither blocks a user. An interrupted download is resumable and proven by
`speakeasy-models`' own tests, but no proof here has ever *interrupted* one —
`Test-SetupWizard.ps1` runs on a machine whose weights are already present, so
it exercises the already-satisfied branch rather than the transfer. And the
`SMOKE_MISMATCH` and `SMOKE_UNAVAILABLE` pages have never been seen by anyone;
they are reachable by pointing `SPEAKEASY_GRANITE_MODEL_ROOT` at an empty
directory, which is how the unit-test controls were run.

### 2b. Distribution — the release, and what it costs to cut another

**1.7.1 was cut and published on 2026-08-28**, and it is the current release:
`SpeakEasyMiniSetup.exe`, 38,097,635 bytes, sha256
`2128ed7a20ed0fb0c77e8a4170dac596cd2bc3b2d7b20d4a55c592a39149a5ad`, with
`SHA256SUMS` beside it. All three proofs ran against that exact artifact — gate,
`Test-InstallerLifecycle.ps1`, `Test-SetupWizard.ps1` — and it was then installed
here and confirmed on the graphics card. The published file was **downloaded back
and re-hashed** against both its own `SHA256SUMS` and the build report; all three
agreed. Do that: uploading the right bytes and publishing the right digest are two
separate things to get wrong.

**v1.7.0 is deliberately never tagged.** It was bumped, built and installed on
this machine, and testing that install is what found the three Settings defects
1.7.1 fixes. `install::decide_now` returns `RefuseSameVersion` on an equal stamp,
so a rebuilt 1.7.0 could not have installed over the one already here — the same
edge that re-cut 1.6.0 as 1.6.1. The CHANGELOG presents one release carrying both
numbers' content, with a note saying so.

**A bump is not finished until `Cargo.lock` moves with it**, and the failure does
not name itself. `Increment-ProductVersion.ps1` refreshes the lock; on the 1.7.1
bump it did not, and the run's output was truncated to its first three lines so
nobody saw. The gate then failed with `cannot update the lock file ... because
--locked was passed`, which reads as a broken checkout. Read the whole output of
any step whose job is to change several files together.

**1.6.0 and 1.6.1 were cut on 2026-08-26**; the paragraphs below describe 1.5.x
and were the latest entry until this one.

**1.5.1 was cut on 2026-08-25** — a fix release, because 1.5.0 destroys every
dictation that reaches the two-minute ceiling. All three proofs pass against its
artifacts, and the install/uninstall cycle also confirmed the registry-residue
fix: `HKCU:\Software\SpeakEasy Mini` is now gone after an uninstall rather than
left empty. Published with `SHA256SUMS`; setup is
`53995ba86d61619b1f3ad0b4de5b799d8bce49f9623ff680357d8f4a24bfe159`.

**1.5.0 was cut on 2026-08-21** and all three proofs pass against its artifacts:
the gate, `Test-InstallerLifecycle.ps1`, and `Test-SetupWizard.ps1` end to end
including a real download, the engine check and a launched app. It was held
until the three provider-honesty findings landed, because cutting before them
would have shipped a build whose Settings page states something false on any
machine with a graphics-card engine.
`SpeakEasyMiniSetup.exe` is published on GitHub Releases and the repository is
public, which is what makes the README's first install path real. Cutting the
next one is four commands and no automation, per the local-only rule:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
.\scripts\Build-LocalInstaller.ps1
.\scripts\Test-InstallerLifecycle.ps1 -ArtifactRoot 'target\local-development\<version>'
.\scripts\Test-SetupWizard.ps1 -ArtifactRoot 'target\local-development\<version>' -Uninstall
```

then `gh release create v<version> …` with `SpeakEasyMiniSetup.exe` and
`SHA256SUMS`. **Publish the checksums file with it**: the build is unsigned by
owner decision, so a digest a user can compare is the only thing standing
between them and SmartScreen's warning being the whole story.

The **version is inherited from the parent product** (1.4.2) rather than
restarted at 0.1.0, because it is what the binary reports, what the HKCU stamp
records and what the install manifest carries; a tag disagreeing with three
recorded facts is worse than an odd first number. `Increment-ProductVersion.ps1`
moves all of them together.

### 3. Publish the CUDA Granite worker — needs the owner
Blocked on two things nobody else can supply: the CUDA Toolkit to build it
(`scripts\Enable-GraniteCuda.ps1` builds one locally) and Hugging Face
credentials to publish it. Target repo `orangeblue39/speakeasy-mini-runtime`,
carrying `granite-worker.exe` and its CUDA redistributables as **one artifact** —
a CUDA worker without cudart beside it fails to launch outright, so they are
physically one unit.

**Deliberately still not done, and item 0 above is not it.** The owner's decision
of 2026-08-20 is a local proof only: build and stage a CUDA worker on this
machine, produce the states, publish nothing. Publishing has a consequence that
is easy to miss — the artifact id is the *declaration*, so the moment
`granite-worker-cuda-windows-x64` appears in `models/trusted-manifest.json` the
wizard offers the graphics-card option and the packager begins refusing payloads
without the libraries, on every machine and in every build. That is a release
decision, not a step in a verification.

When it is taken: pin the worker and its libraries by SHA-256 under that exact
id, and give `download::plan` its second item. That function already takes
`provider` and deliberately ignores it, with a comment saying why, for exactly
this. `speakeasy_models::inspect_gpu_payload` needs no change at all — it starts
answering `Ok(())` on its own, which is what the id being a named constant buys.

Until then a GPU machine gets the CPU worker, the option is disabled with the
reason, no installation may record `cuda`, and the app says so honestly.

#### Started 2026-08-26: the worker is built and proven, the publish is not done

Owner decision 2026-08-26: do it. Three sub-decisions were taken with it.

- **Hugging Face carries the worker only.** The three CUDA DLLs stay fetched from
  NVIDIA's own CDN, where the manifest already pins both archives by digest and
  `required_cuda_runtime_files` already reads them. That is ~54 MB uploaded
  instead of ~450 MB, no new plumbing for the libraries, and no question about
  re-distributing NVIDIA binaries under the CUDA EULA from a personal repo. It
  **reverses** the "one artifact" intention written above; the atomicity that
  intention was protecting is provided by `inspect_gpu_payload` refusing until
  every file is present, not by the transport.
- **Repo under whichever account the token belongs to**, named
  `speakeasy-mini-runtime`. The `orangeblue39/...` above was never confirmed.
- **Upload, then fix the gap below, then pin.** Nothing user-visible changes
  until the whole path works.

**The worker exists and is proven on the card.** Built from `67c9498` with
`--features cuda`:

```text
target\release\speakeasy-granite-worker.exe
  57,052,672 bytes
  sha256 1d4a3ad57e72acaaa55a507f88733ac18f77909349c4bf7c7bc017269499170a
  image names cublas64_13.dll and nvcuda.dll; not cublasLt64_13.dll, not cudart64_13.dll
```

That import table is the expected one and matches what `granite_gpu.rs` measured
on 2026-08-21, which is the cheapest available check that a binary is genuinely a
CUDA build. Both hardware tests pass against **this** binary, staged into
`target\debug\proof\` with the three libraries beside it:
`a_cuda_worker_reports_the_device_its_context_probe_can_prove`, and
`granite_final_pass_transcribes_the_fixture_through_the_real_worker_process` with
`worker=cuda first=41.1286058s second=361.2019ms` — **361.2 ms resident on CUDA**,
matching the 2026-08-21 figure to the millisecond.

Three things learned doing it, all of which cost something to find:

- **A CUDA worker's digest is not reproducible, and its size is not a fingerprint.**
  Three CUDA workers on this machine — the one staged in the installed app, the
  one in `target\debug\proof\`, and this fresh build — have **three different
  SHA-256s**, and two of them have the *identical* 57,052,672-byte size. MSVC
  embeds a timestamp and a PDB path. So "same size" proves nothing about "same
  build", the digest that gets pinned pins one specific build, and a rebuild will
  not reproduce it. Publish the bytes that were tested, not a rebuild of them.
- **`--verify-provider` cannot prove the card until the worker is published.**
  Staging this worker and re-proving reported `provider_recorded device=cpu
  evidence=gpu_worker_not_published` — correct, and item 12 exactly: the first of
  the three gates asks whether a worker is *published*, so the proof is circular
  until the manifest names one. The instrument that works before publication is
  the two hardware tests above.
- **Publishing does not break the installer build.** `Invoke-ProofPackage.ps1`
  builds the worker with default (CPU) features and `Assert-GraniteWorkerPayload
  IsCoherent` only refuses a *staged* CUDA worker with no libraries beside it. The
  "packager begins refusing payloads" warning above is about staging, not pinning.

#### The gap: pinning the artifact is not enough, and this machine cannot see it

Item 3 says every layer below "starts answering `true` without a second edit".
**That is wrong**, and the reason is worth more than the correction.

The wizard gates its graphics-card radio on
`download::graphics_card_configuration_available()`, which calls
`inspect_gpu_payload` against `%LOCALAPPDATA%\SpeakEasy Mini\proof\granite-worker.exe`.
On a **first install that file does not exist yet** — setup has not extracted the
payload when the provider page is shown. So pinning the artifact alone leaves the
option disabled on every fresh machine, now reporting `WorkerNotInstalled` where
it used to report `WorkerNotPublished`.

`inspect_gpu_payload` conflates two questions that were always different:

| Question | Who asks it | What it needs |
| --- | --- | --- |
| Is a graphics-card configuration **installable**? | the wizard's provider page | published, and fetchable |
| Is one **installed here**? | the app's warm path, `--verify-provider`, `smoke::gpu_payload_rejection` | the files present in `proof/` |

Splitting them is the second edit. `download::plan` also still needs its second
item — it takes `provider` and deliberately ignores it, with a comment saying it
is for exactly this.

**And this machine is the worst possible place to notice**, which is why it is
written down rather than merely fixed. It has the CUDA worker and all three
libraries staged, so with the artifact pinned the option would light up *here* and
look correct. The wizard that only offers the graphics card to machines that
already have it would have shipped, and every test on this rig would have agreed.

#### Closed 2026-08-26, and proved by simulating the pin

`gpu_configuration_is_installable` is the new question: **published, and the
libraries pinned**, asked of the manifest and touching no disk at all. That last
part is the fix -- asking it of a directory is what made it wrong.
`inspect_gpu_payload` keeps the three-gate "installed here" answer and calls the
new function for its first gate, so the two cannot disagree about what published
means, and the order of rejections is unchanged. The wizard's
`graphics_card_configuration_available` now asks the installable question and no
longer looks at `install_root` at all.

The presence check was there for a real case -- "published alone would re-offer
the option on a machine where the runtime libraries never arrived" -- and that
case is answered later and better: `smoke::verify_engine` runs *after* the payload
is staged and the recorded provider comes from its verdict, and the app re-proves
the CUDA context at every warm. The wizard cannot do it correctly before the files
exist, so it should not try.

**The bug and the fix were both demonstrated rather than argued**, by temporarily
adding the artifact to the manifest with the proven digest and running the suite
against it. That simulation is what the pin step will really be, so its output is
the map:

| Test | Under a pinned manifest |
| --- | --- |
| `the_shipped_catalog_publishes_no_graphics_card_worker` | fails with `left: Err(WorkerNotInstalled)` -- **this is the bug, measured**: exactly what the wizard would have received on a first install |
| `installable_asks_the_release_and_never_the_disk` | fails with `left: Ok(())` -- **the fix, measured**: installable says yes while `inspect_gpu_payload` still says not-installed, which is the split working |
| `a_published_worker_that_was_not_installed_is_its_own_rejection` | fails in its *fixture* |
| `a_present_worker_with_no_runtime_libraries_names_every_missing_file` | fails in its *fixture* |
| `a_complete_payload_is_accepted_and_is_still_not_proof_it_runs` | fails in its *fixture* |
| `the_catalog_never_pins_a_worker_without_its_runtime` | passes |

Those three fixture failures are not logic failures and are worth knowing before
they cost an hour. `manifest_with_published_worker()` synthesises the artifact by
cloning `artifacts[0]` and renaming it, so once a real entry exists the manifest
has **two** with that id and `TrustedManifest::parse` refuses it. When pinning for
real, the fixture stops synthesising and returns the shipped manifest, and the
first two tests in the table invert. All five were anticipated: the shipped-catalog
test's own comment says it "is the assertion that flips on the day a CUDA worker is
pinned -- at which point the failing test is the reminder that the wizard, the
packager and the marker all now have work to do".

#### The download path is written, 2026-08-26. Only the upload and the pin are left

`download::plan` fetches the graphics-card payload now, setup places it, and the
uninstaller knows the difference between a library it staged and one it found.
All of it proved against a simulated pin, because that is still the only
instrument that works before publication.

**Four items, not two.** The plan is the weights, then the engine, then the two
NVIDIA redistributables — separate artifacts from separate hosts, per the
2026-08-26 decision, so `graphics_card_payload_sources` returns all of them or
none. Each gets its own label (`Graphics-card engine`, `Graphics-card runtime`,
`Graphics-card maths library`) because the step lists them one per line and names
one per progress line; a shared label prints the same sentence twice and reads as
setup having lost count.

**A graphics-card install downloads 2.69 GB, not 2.30.** The weights are
2,298,601,952 bytes; the two NVIDIA archives add 396,296,547, of which cuBLAS is
393.7 MB on its own. The worker archive is on top of that — call it ~25 MB
zipped from 57 MB. The step states the total before the user commits to it, so
this number is shown rather than discovered, but it is worth knowing that
choosing the graphics card costs a user 400 MB more than choosing the processor
and that most of it is one library.

**Publish the worker as a zip, not the bare `.exe`.** A `native-runtime` artifact
carries `archive_bytes`, `archive_sha256`, `archive_prefix` and `proof_files` as
required fields, so an archive is the only shape the schema has — and taking it
means `install_archive` already does the fetch, the digest check, the extraction
and the atomic activation. `archive_prefix` is empty for an archive with no
wrapping directory. The `proof_files` entry pins `granite-worker.exe` at the
**digest already measured** (`1d4a3ad5…`, 57,052,672 bytes) — those bytes, not a
rebuild of them, since an MSVC build is not reproducible.

Three things this found that the plan above does not mention:

- **The provider page's radio button was never read.** `plan` took the
  *machine's* `preferred_provider()`, and that was indistinguishable from the
  answer for exactly as long as the option stayed disabled — so pinning the
  artifact would have enabled a control that decided nothing, which is the defect
  the disabling exists to prevent arriving from the other side. It reads
  `selected_provider()` now. The record still comes from proof: choosing the
  processor stages no worker, so the engine check proves the processor.
- **The staging has to run *after* `install::perform`, not before.** The payload
  carries the processor worker under the same name and `perform` merges, so a
  CUDA worker placed first is overwritten by the copy, silently. Putting it after
  also fixes the reversion `Enable-GraniteCuda.ps1` documents as its sharpest
  edge — an upgrade re-lays the CPU worker and this puts the CUDA one back, every
  time. **That script can be retired once the pin lands**, along with the
  `granite-worker.cpu.exe` entry in `KNOWN_PROOF_ORPHANS` that exists only for it.
- **Staging keyed on what is on the disk hands the card to someone who asked for
  the processor.** The first version of `stage_graphics_card_payload` asked only
  whether the artifacts were installed under `model-lifecycle`, on the reasoning
  that this is *evidence* rather than intention — which is the right instinct and
  the wrong question. Those artifacts survive an uninstall with
  `--keep-user-data` and survive installing over an existing profile, so a second
  install where the user chose the processor would have found them, staged the
  CUDA worker over the CPU one, and the engine check would have dutifully proved
  and recorded the card. **Nothing would have reported it**, because nothing
  would have been wrong: every layer would be honestly describing what it found.
  It takes the provider now, and refuses for anything but CUDA. The distinction
  is that the answer *is* the fact here; what stays evidence-driven is the
  record, which still comes from the engine check's verdict.
- **The uninstaller would have accused setup's own files.** `proof/` is
  classified into "files this installer placed" and "files it did not", and the
  CUDA libraries were the canonical example of the second — because only
  `Enable-GraniteCuda.ps1` ever put one there. `staged_proof_files()` reads them
  from the manifest now, so an uninstall no longer asks a graphics-card user
  about the two libraries their installation cannot run without.
  `nothing_survives_proof_and_what_we_did_not_place_there_is_named` inverted for
  them and was rewritten with a genuinely foreign file, so the second pass is
  still exercised rather than merely reached.

#### Published 2026-08-26. Item 3 is closed except for the release

The worker is on Hugging Face, pinned in the manifest, and the seven tests that
declared its absence have inverted. `orangeblue39/speakeasy-mini-runtime`,
public, with `README.md` and a `NOTICE.md` carrying the MIT texts verbatim
(llama.cpp/ggml, nlohmann/json, llama-cpp-rs) and naming the NVIDIA CUDA Toolkit
EULA for the statically linked runtime.

**Pinned at an immutable commit, not at `main`.**
`.../resolve/18ec2c410d01d95754b43a57b680a4e5dddcb7c7/granite-worker-cuda-windows-x64.zip`,
the way the two Granite pack files already are. A `main` URL would let the bytes
move out from under the digest, which fails closed but fails on a user's machine.
Downloaded back after upload and verified byte-identical, and the redirect goes
to `us.aws.cdn.hf.co` — in `redirect_hosts` already — serving range requests, so
resume works.

**Seven tests inverted, and one of them was a count nobody predicted.** The six
expected ones flipped as their comments promised. The seventh was
`bundled_proof_manifest_is_embedded_valid_and_fail_closed` asserting
`proof_artifacts().len() == 2` — a hardcoded count, which is exactly why it
caught an entry *arriving* and not only one leaving.
`staged_manifest_publishing_the_cuda_worker` in both crates now returns the
shipped manifest and asserts its premise instead of forging one, and the
bootstrapper's `serde_json` dev-dependency went with it, so the dependency-policy
allowlist entry it needed is reverted too.

**The empty case still needs covering, and now has to be built deliberately.**
`a_worker_without_its_libraries_is_not_a_fetchable_configuration` trims the
redistributables out of the shipped JSON, because "nothing published" stopped
being the free answer the shipped catalog gave.

**`Test-SetupWizard.ps1` now derives its expectations from the machine.** It
demanded the graphics-card option be *disabled* and the marker be `cpu` — both
correct only while nothing was published, and both now statements about whichever
card the machine running it happens to have. It reads whether the option was
offered, requires the download page to name `Graphics-card engine` when it was
(which is the end-to-end proof the radio button reaches `download::plan`), and
takes the expected marker from the verdict sentence the user is shown rather than
from a constant. A run where the card was offered and the processor was proved
prints a loud note rather than passing quietly, because that means the CUDA path
was not exercised by a run that looks like it was.

**A `Checked` assertion was written and removed before it ever ran.**
`Get-Controls` reports `Visible` and `Enabled` and no check state, so
`$graphicsCard.Checked` reads `$null`, inverts to true, and would have thrown on
every capable machine — a broken instrument shaped exactly like the failure it
claimed to find. The download page's own words are the better assertion anyway.

#### The archive is built and proved, 2026-08-26. The blocker *was* a write token

**`hf auth login` was never the blocker, and repeating that cost a round trip.**
This machine has an authenticated token — `hf auth whoami` answers
`orangeblue39` — and it is **role `read`** ("speakeasy token 04.22.2026"), so
`hf repo create` returns `403 Forbidden: You don't have the rights to create a
model under the namespace`. A token that authenticates is not a token that can
write, and `whoami` cannot tell them apart. Ask
`https://huggingface.co/api/whoami-v2` for `auth.accessToken.role` before
believing an upload is possible.

**The archive exists and installs.** Built from the proven binary, with the exe
renamed to the name it must occupy in `proof/`:

```text
granite-worker-cuda-windows-x64.zip
  42,162,465 bytes   (26.1% smaller than the exe)
  sha256 03b7c33f83674edfa64549e43a5f87e3393e03100db44027b10940b5deb28147
  contains granite-worker.exe
    57,052,672 bytes
    sha256 1d4a3ad57e72acaaa55a507f88733ac18f77909349c4bf7c7bc017269499170a
```

That inner digest is the binary both hardware tests passed against, and its
import table was re-checked: `cublas64_13.dll` and `nvcuda.dll` present,
`cublasLt64_13.dll` and `cudart64_13.dll` absent, matching 2026-08-21 exactly.
**`strings` is not on PATH in this repository's shells**, so the first check
reported all four absent — four zeros from a program that does not exist, which
reads exactly like a CPU build. Use `grep -ao` and run a control.

**It was installed through the real path, not eyeballed.** A throwaway
integration test drove `InstallManager::install_archive` with the spec
`InstallSpec::from(NativeRuntimeSource)` will build — no `archive_prefix`, one
required file — and the worker landed under its own name, at the right length,
and passed `reverify`. So `Compress-Archive`'s zip is a format the `zip` crate's
`deflate-flate2`-only feature set reads, which was an assumption until then.

**Omit `archive_prefix`, do not set it to `""`.** It is `Option<String>` and an
empty string fails `check_relative_path`, which refuses an empty value. Absent
becomes `""` in `NativeRuntimeSource`, which is the no-op `strip_prefix` wants.

**The download policy already allows the host.** An HF `resolve` URL 302s to
`us.aws.cdn.hf.co`, which is in `redirect_hosts` — measured against the Granite
pack's own URL. Pin the worker at an immutable commit rather than `main`, the way
those two entries do (`/resolve/8267dad2…/`), so the URL cannot move out from
under the digest.

**Unverified risk, and it is not new: that redirect host looks regional.**
`us.aws.cdn.hf.co` is what this machine is sent to, and it is the *only* HF CDN
host in the policy. A user outside the US being sent to a different one would
fail closed at transfer time — and this applies to the 2.3 GB of weights the
shipped installer already fetches, not just to the worker. Nothing here can test
it from one country. Worth resolving before the next release is handed to anyone
far away.

**The artifact's `version` is `1.5.1`, and that is deliberate.** It describes the
*bytes*, which were built from `67c9498` when the workspace was at 1.5.1, and it
is the second component of the install path — so it identifies the binary rather
than the release that ships it. `source_commit` pins it exactly. The pack's
`minimum_worker_version` is `0.1.0`, so a 1.5.1 worker under a 1.6.0 app is not
a compatibility question.

**Done, in the order it happened.** A write-scoped Hugging Face token and the
upload; then the manifest entry (id, version, URL, `archive_bytes`,
`archive_sha256`, one `proof_files` entry for the worker, and no
`archive_prefix`);
then the four tests in the table above invert and
`staged_manifest_publishing_the_cuda_worker` — which exists in *two* crates now,
`speakeasy-models` and the bootstrapper — stops synthesising and returns the
shipped manifest. Then the release, and item 16's four sentences, which a
published worker closes on its own by restoring the qualification promotion.

**Nothing user-visible has changed yet**, and that is the point: with no worker
pinned, `graphics_card_payload_sources` is empty, the plan is one item, the option
stays disabled and `stage_graphics_card_payload` returns `Ok(false)` without
creating a directory.

### 4. The rebrand tail — done 2026-08-18, and it was not cosmetic
The remaining "SpeakEasy" strings were filed as naming. Three of them were
correctness bugs, each one this product writing into the *parent* product's
state, and all three were invisible because the installer had not been run
since the fork.

- **`uninstall::data_root()` returned `%APPDATA%\ai.speakeasy.desktop`.** Setup
  puts the model weights under that root, so a fresh install would have
  downloaded ~2.3 GB into SpeakEasy's data directory and the app — which reads
  `ai.speakeasy.mini`, verified, because that is where a working dictation
  found them — would then have reported Granite as not installed. In the other
  direction it is worse: uninstalling Mini removes this tree, which was
  SpeakEasy's.
- **The ARP key was `…\Uninstall\ai.speakeasy.desktop`**, so setup registered
  Mini over SpeakEasy's Add/Remove Programs entry — its `DisplayName`, version
  and uninstall command — and Mini's uninstaller then deleted it, leaving the
  parent installed and unlisted.
- **`VERSION_KEY` was `Software\SpeakEasy\LocalDevelopment`.** Found while
  fixing the other two, and the same shape one level down: installing Mini
  overwrote SpeakEasy's version stamp, and Mini's downgrade refusal compared
  against whatever SpeakEasy had installed, so the two products could refuse
  each other's upgrades. Its inherited justification — that the key is shared
  with the NSIS hooks so an upgrade finds its predecessor's stamp — cannot
  apply to a product that has never shipped and has no predecessor.

Also renamed: the desktop binary is `ai-speakeasy-mini.exe`, in the cargo
manifest, `install.rs`'s `APP_EXE` and `RUNNING_NAMES`,
`speakeasy-windows::startup`, and the seven scripts that name it. The
refuse-while-running check lost `speakeasy-v2-preview.exe`, which belonged to
the parent's legacy preview and shared its install directory, never Mini's;
keeping it only risked refusing a Mini install because something unrelated was
running.

Two things deliberately left alone. The Rust **crate** names still say
`speakeasy-*`, per the original decision, because renaming them churns every
manifest and path dependency for no observable difference. And the IPC schema
`$id`s still read `ai.speakeasy.desktop/ipc/...`; a schema identifier is
neither user-visible nor a filesystem path, which is the line that decision
drew.

Since verified: an installer was built and `Test-InstallerLifecycle.ps1` was
run against it. It found three more collisions this section had not — the
default install root, the Start Menu folder and the Add/Remove Programs
`DisplayName` — all recorded above under "The installer has now been built and
run". The lifecycle test now passes.

### 5. Dead onboarding plumbing — done 2026-08-19
Larger than the 10 references recorded here: 16 across 7 files, plus the
persisted `OnboardingProgress`, its profile IPC field, the `onboarding_advance`
command, and a `current_step > 7` validation that could reject a settings file.
All gone. `setup_requirement` was listed with them and **is live** — it is on
the 10 Hz HUD poll and answers whether this profile can dictate right now.

The persisted field was safe to drop because `Settings` has a
`#[serde(default, flatten)]` catch-all; the migration test pins that rather
than assuming it.

### 6. Two comments that outlived their reasoning — done 2026-08-18
Both were found while fixing the launch. One turned out to be a live defect
rather than a stale comment.

**The dictation floor was 4 GiB and Granite's is 8 GiB**, and the split was
deliberate: `runtime_wizard`'s gate is asked before the engine is chosen, so
holding it below Granite's meant a mid-range machine still dictated through the
streaming path and merely declined the second pass. With one engine that split
only bought the user a delay. A machine with, say, 6 GiB passed the gate,
recorded, waited out the whole post-recording pass, and got `GraniteUnavailable`
— after they had already spoken, with nothing to fall back to. The floor is now
Granite's floor. Refusing at `begin`, before a sample is captured, says the same
thing at the only moment it is useful.

The test that pinned the old ordering asserted the dictation floor stayed
*strictly below* Granite's, "or the split is meaningless". It now asserts the
opposite bound — the floor must never sit below Granite's, or a dictation is
admitted that cannot possibly finish — and keeps its original body, which is
still worth having: too little memory must read as "Granite is not part of this
install", never as a fault worth a quarantine strike.

**`run_granite_final_pass`'s doc** said `Ok(None)` let "the ordinary
single-engine fallback" run. There is no fallback. The behaviour was already
right — `judge_granite_pass` maps it to `FinalSourceReason::GraniteUnavailable`,
checked rather than assumed — so this was a comment fix.

### 7. Documentation debt the fork left behind
Found while bringing the markdown up to date on 2026-08-18. None of it breaks a
build; all of it misleads a reader.

- **Named citations of deleted docs: done on 2026-08-19.** 35 of them across 27
  files, not the 25 across 21 recorded here — the original count searched three
  deleted docs and there were **six**: `granite-final-pass.md`,
  `setup-wizard-redesign.md`, `hud-side-dock.md`, `migrate-to-nvidia-gpu.md`,
  `transcribe-cpp-benchmark.md` and `docs/archive/UI-REDESIGN.md`. **None of the
  six was ever in this repository's history** (`git log --all` finds no commit
  touching any of them), so there was nothing to rewrite most citations
  *against*: each one either dropped, or absorbed into the sentence the fact it
  had been carrying. Four were rewritten against something that still exists —
  three proof scripts now cite `docs/UI-GUIDE.md`, and `speakeasy-worker`'s
  `BatchFinalPass` now points at `admissible_delivered_transcript` for the
  `NoSpeechDetected` split. Bulk `sed` would have mangled these: the citation is
  fused into the sentence in most of them.
- **Nemotron licence material: deleted on 2026-08-19.**
  `models/huggingface/nemotron-3.5-streaming-en-cuda/` (README, NOTICE, licence,
  checksums) and `packaging/licenses/OpenMDW-1.1.txt`. Nothing bundled or copied
  either — no packaging script touches `packaging/licenses/` or
  `models/huggingface/` — and the fork distributes no NVIDIA model bytes, so no
  licence obligation attached to keeping the text. Both are recoverable from git
  if that judgement is ever revisited. Two further Nemotron residuals went with
  them: `crates/speakeasy-models/examples/phase2_current_host.rs`, which
  resolved the removed `nemotron-3.5-streaming-en-cpu` pack id and so could
  never have succeeded since the fork, and the dead `nemotron_3_5_streaming`
  row in `catalog.ts`'s display-name table.
- **The rest of it went on 2026-08-19 too.** All four remaining categories, and
  three defects found while doing it. `grep` for any of these now returns zero:
  - **96 bare `§N` references across 30 files**, which pointed at
    `UI-REDESIGN.md`'s section numbering. `docs/UI-GUIDE.md` does not number its
    headings, so a number could not be carried across; each was mapped to a
    named heading (`UI-GUIDE "Information architecture"`, matching the one
    pre-existing citation of that form in `styles.css`) or dropped where it was
    pointing at implementation rather than spec. **Every heading cited this way
    is checked to exist** — replacing a dead reference with a new dead reference
    is the failure mode to avoid here.
  - **22 bare `Phase N` references across 6 files**, pointing at
    `granite-final-pass.md`'s phases. Dropped, or replaced with the fact the
    phase number was standing in for (`Phase 9` → `2026-08-04`, `Known risk #12`
    → "the stale-clock deadline bug").
  - **6 prose references** — "the handoff", "the brief", "the GPU migration
    handoff, item 14" — which no grep for a filename or a `§` would have found.
    Worth knowing that this class exists before believing a citation sweep is
    complete.
  - **51 references to deleted crates and files**, of which **12 remain
    deliberately.** Every survivor is *history* that reads as history:
    "It was `speakeasy-asr`, and it did link one", "the crate was renamed when
    the streaming engine left", "were listed here until the fork removed the
    engine". Those are correct and load-bearing — rewriting them would make them
    wrong. What was fixed is the ~39 that made **present-tense claims** about
    things that no longer exist.
- **Three defects found while doing that pass**, none of which a citation sweep
  was looking for:
  - **`speakeasy-granite`'s crate doc claimed the wrong engine shipped.** Its
    "What it is for" section said the delivered transcript came from the
    *streaming* model run a second time over the retained audio. Corrected to
    what the fork actually does.
  - **8 invisible U+009D control characters** in comments across 5 files, each
    an em-dash followed by a stray byte from some encoding round-trip, all
    present since the first commit. They render as nothing and survive review.
    Found only because a scripted replacement refused to match a line that
    looked identical on screen — the failure was the instrument working.
  - **A doc comment attached to the wrong item.** In
    `workers/granite-worker/tests/granite_worker_smoke.rs`, the 11-line
    description of the residency proof sat on the `run_dictation` *helper*
    below it, so the helper's own three-line description was fused onto the end
    of it and the test itself had none. Reattached.
- **The shipped notices were describing a different product.**
  `THIRD-PARTY-NOTICES.txt` declared sherpa-onnx, ONNX Runtime and the CUDA
  redistributables as bundled, and `MODEL-NOTICES.md` described two Nemotron
  packs as the models in use. Both are corrected — a legal notice claiming to
  distribute something it does not is worse than one that is merely stale — but
  they are worth re-reading whenever the payload changes, because nothing checks
  them against `Build-LocalInstaller.ps1`'s actual output.

### 8. The engine-reason sentence contradicted the device — done 2026-08-21

Found on hardware by reading the running Settings window; fixed the same way,
and the fix was verified the same way rather than by reading the code back.

Before, on a machine with a graphics-card worker staged:

```text
Dictation runs on: Graphics card (GPU) — this computer's graphics card is
supported, but the graphics-card model is not installed, so the processor model
is being used.
```

After, read out of the running window with `Invoke-WebviewProbe.ps1` against a
warm with `engine=cpu_gpu_pack_not_installed device=cuda`:

```text
Dictation runs on: Graphics card (GPU)
This computer's graphics card is supported, but this installation includes only
the processor model.
Dictation is running on the graphics card, which is more than this installation
was recorded as providing. Nothing is wrong — the graphics-card engine was
staged after setup ran.
```

**Both halves were fixed, on the owner's decision, because either alone leaves
the trap.** The strings: all four `engineReasons` entries now describe what the
*installation includes* and none of them names a device, `probe_preferred`
included — it was not the reported defect, but "the best engine this hardware
supports" becomes false the day a graphics-card pack is preferred and the driver
refuses it, and a latent copy of the bug just fixed is not worth the smaller
diff. The structure: the reason is its own `<p data-testid="engine-reason">`
rather than a clause hung off the device after an em-dash, so a future reason
cannot re-create the compound sentence by wording alone.

Dropping the reason was rejected, per the original note: it is the load-bearing
half on an ordinary machine.

**Where the graphics-card reading came from**, since it is not the default state
of a dev launch — see the new trap below about `Stage-DevRuntime.ps1` reverting
the staged worker. The CUDA worker was copied back into `target/debug/proof`,
`npm run dev` and `target/debug/ai-speakeasy-mini.exe` were started separately so
nothing re-staged, and the log line
`granite_warm result=ok engine=cpu_gpu_pack_not_installed device=cuda
installed=cpu provider=running_beyond_record` confirms the combination being
rendered.

### 9. `cudart64_13.dll` is required and never loaded — done 2026-08-21

Settled by experiment rather than by argument, which changed the answer to one
of the two open questions. Owner decision: **keep cudart pinned, and correct the
claim.**

All of it measured against the worker this workspace builds, with the CUDA
Toolkit stripped from `PATH` — which is the whole trick, because the toolkit puts
`bind` on it and Windows then resolves a library that is not beside the worker
at all:

| File | Named in the image | Deleting it |
| --- | --- | --- |
| `cublas64_13.dll` | yes | the process does not start |
| `cublasLt64_13.dll` | no | starts, loads the weights, fails **~36 s in** at the first matmul with `AdapterFailed` |
| `cudart64_13.dll` | no | nothing: transcribes, and NVML confirms the context |

**`cublasLt` was the unverified half of the item and it turned out to matter
most.** The standing explanation — cuBLAS loads it at run time — is now measured,
and the *shape* of its failure is the finding: a gate that concluded "the worker
started, so its libraries are fine" would have passed that payload and lost a
dictation half a minute later. That is the argument for the check being a
precondition, and nothing had stated it.

`cudart` stays enforced although it is never loaded, and the reasoning is
written where it can be found: `CMAKE_CUDA_RUNTIME_LIBRARY` is one build flag
from making it load-bearing again with nothing noticing, and every file this
catalog requires is a file it pins by digest — the property that lets presence
imply provenance. The cost is 551 KB and a refusal no published payload can
trigger, since the worker and its libraries ship as one artifact. What was wrong
was the *claim*, and the three places that made it (`granite_gpu.rs`'s header,
its `CUDA_RUNTIME_ARTIFACT_IDS` doc, `runtime_wizard.rs`) plus
`ARCHITECTURE.md` now say what was measured.

### 10. An unprovable context reported the actionable fault — done 2026-08-21

A fifth `ProviderIntegrity` state, `GpuRecordUnconfirmed` /
`gpu_record_unconfirmed`, approved by the owner with its copy. Not a fault, and
deliberately not folded into `Matches` — that would claim an agreement nothing
verified, which is the same mistake pointing the other way.

The fix is a split rather than a new branch: `WorkerProvider::disproved_graphics_card`
is now a separate question from `!proved_graphics_card()`, because three states
exist and only two of them are answers. A binary with no CUDA backend cannot be
on the card and NVML listing no context for this pid is the definitive negative;
a probe that could not be asked, and a worker that never answered its handshake,
prove nothing either way. Only the definitive negative is the fault.

The verdict and the device now agree by construction, and
`only_a_definitive_processor_run_may_be_reported_as_the_fault` pins the
correspondence across all five worker states rather than leaving two `match`
arms that happen to line up. **Two existing tests asserted the old behaviour and
argued for it in their comments** — "an installation that now cannot prove it is
one whose card stopped being used" — and both were rewritten to say why that
argument is wrong: what stopped is the query. All three were made to fail by
restoring the collapsed arm.

### 11. The truncation everyone is hunting is unreachable — found 2026-08-25

**Not a defect. A retired risk, and a warning about how it survived.**

`max_new_tokens` is 2048 and the generation loop stops on reaching it with no
error and no end-of-generation token, which is all true and is why this has been
carried as a live hazard since the fork. What nobody did was compare it against
the *other* limit. `MAX_CAPTURE_SECONDS` is 120. Two minutes of speech is around
310 words, which is roughly **400 tokens** — a fifth of the budget. There is no
utterance the hotkey path can produce that reaches the ceiling, because the
thing that would have to be long is capped five times lower.

Confirmed empirically as far as one run can: a 120.183 s capture, the longest
this product can make, transcribed complete with a six-word tripwire intact.

Two things follow, and the second is the reason this is written down rather than
deleted.

- **The hazard is real but latent.** It becomes reachable the moment
  `MAX_CAPTURE_SECONDS` is raised, and `capture_wizard.rs` already documents
  wanting thirty minutes once per-sample metadata stops being retained. Thirty
  minutes is ~4,600 tokens and would truncate silently at about the nine-minute
  mark. **Anyone raising the ceiling has to raise `max_new_tokens` with it**, and
  nothing in the tree connected the two — no test, no comment, no assertion. That
  was the actual finding, and it is **closed as of 2026-08-26**.

  `the token budget covers the longest dictation the ceiling allows` compares the
  two. They cannot be compared in Rust: `MAX_CAPTURE_SECONDS` is in
  `speakeasy-desktop` and `max_new_tokens` is in `speakeasy-granite`, and the
  desktop crate deliberately does not depend on the one that compiles llama.cpp
  — so the scaffold suite reads both as source, which is what this repository
  already does for cross-file invariants. The rates are deliberately pessimistic
  against the only real measurement there is (312 words in 120.183 s, ~1.29
  tokens per word): **200 wpm and 1.5 tokens per word**, so the check complains
  before a real user is truncated. It binds at about **410 s** of ceiling.
  Proved able to fail by raising the ceiling to thirty minutes, which reports "a
  1800 s ceiling can need 9000 tokens and max_new_tokens is 2048".
- **It survived because the arithmetic was never done.** The handoff prompt for
  this session asked for "two to three minutes" of speech to approach a limit
  that two minutes cannot reach, and reproduced the correct token estimate
  (~400 tokens for a 120 s clip) directly above the claim, in the same
  paragraph. Both numbers were right and nobody divided one by the other. A risk
  register inherits its entries by copying; it does not re-derive them.

### 12. `running_beyond_record` cannot be cleared on any machine — found 2026-08-25

A locally staged CUDA worker reports `provider=running_beyond_record` **forever**,
and re-proving the marker does not change it. `--verify-provider` answers
`device=cpu evidence=gpu_worker_not_published`, because the first of the three
gates asks whether a CUDA worker is *published* in the trusted manifest and
`granite_gpu.rs` says of that variant, correctly, "Today this is every machine's
answer."

Nothing here is behaving wrongly — the disclosure is honest and is displayed as
not-a-fault, which is exactly the design. What is wrong is the **documentation**,
in two places that both told this session to expect an outcome the code cannot
produce: item 1b step 3 offered "no integrity line beneath it if the marker was
re-proved", and `Enable-GraniteCuda.ps1`'s own header says skipping the re-prove
"is not neutral" and leaves the marker at `cpu` — implying that running it does
not. It re-proves `cpu` either way.

The re-prove is still worth doing, for the opposite reason: on `-Revert` it is
what stops the marker manufacturing `gpu_install_not_operational`. That half of
the script's reasoning holds. **Closing this is item 3**, and it closes on its
own the moment a CUDA worker is published.

### 13. Delivery follows the foreground at *completion*, and inference is long — found 2026-08-25

Both processor transcripts were delivered into a window that was not the target,
with `integrity=Equal` and `result=committed`. The app did nothing wrong: the
target is whatever Windows reports as the foreground window at the moment a
dictation *finishes*, which `ARCHITECTURE.md` states plainly.

The finding is the size of the window that opens. On the processor, inference on
a full-length dictation is **44.5 s**, so there are forty-four seconds between
the user stopping speaking and the paste landing, and every window they touch in
that time is a candidate delivery target. On the card the same exposure is
4.2 s. This is not the `target_inspect_refused` hazard the traps list already
covers — that is SpeakEasy Mini hijacking its own dictation, is detected, and
falls back to the clipboard. This is a *successful* delivery into the wrong
application, indistinguishable in the log from a correct one.

It bit the session that was measuring it, which is the point: the owner stopped
speaking, moved to another window to report back, and the transcript arrived
there. On a CPU-only install this is the normal case rather than an edge one —
44 s is long enough that moving on is the *reasonable* thing for a user to do.

Nothing is proposed here. The behaviour may well be correct and the alternatives
are worse (pasting into a window that has since closed, or holding text
hostage). But it is not currently written down as a consequence of slow
inference, and the two facts live in different documents.

### 14. Settings permanently reported the shortcut as unregistered — found 2026-08-25, closed 2026-08-26

`Settings → General` read **"Shortcut not registered yet"** while the shortcut
was registered and working. Confirmed both ways on 2026-08-25: the panel said it
for the life of the process, and `hotkey_status` invoked directly against the
same window returned `binding: "Ctrl+Alt+P", registration: "registered",
enabled: true`. Dictation then worked twice.

`General.tsx` read it with a bare `invoke<HotkeyStatus>("hotkey_status")` — no
rejection handler, no retry — and rendered
`formatShortcutState(hotkey?.registration ?? "pending")`. Every window's webview
loads while `setup` is still managing coordinators, so that read can be refused
with "state not managed for field `state` on command …", and `hotkey` then stayed
`null` for the life of the process.

**This was the 2026-08-20 `personalization_status` defect in a second location**,
and the one-line fix was never the task. The lesson recorded from the first
occurrence was "one reader had carried a retry since the day it was found and
nothing else did" — and the fix for it then repeated exactly that, because the
*test* written to prevent a recurrence named one command in one file. It was
green on the day this was found.

#### What the sweep found

Every mount-time read in `apps/desktop/src` was enumerated, and the hazard was
defined rather than guessed at: a `#[tauri::command]` taking a `tauri::State` is
what Tauri refuses when the coordinator behind it is not managed yet. **41 of the
56 commands take one.** Of the reads that reach them from a `useEffect`:

| Reader | Command | Was | Now |
| --- | --- | --- | --- |
| `General.tsx` | `hotkey_status` | bare, no `catch` | retried, reports |
| `Advanced.tsx` | `diagnostics_status` | bare, no `catch` | retried, reports |
| `Advanced.tsx` | `credential_status` | bare, no `catch` | retried, reports |
| `OutputPrivacy.tsx` | `result_status` | bare, no `catch` | retried, reports |
| `Transcription.tsx` | `model_catalog`, `gpu_status` | bare, behind `refreshCatalog` | retried |
| `Transcription.tsx` | `diagnostics_status` | bare, `catch` hid the panel | retried |
| `useProfile.ts` | `profile_status` | hand-rolled retry, silent | shared retry, reports |
| `Transcription.tsx` | `model_hardware`, `personalization_status` | retried (2026-08-20) | unchanged |

**Four had no rejection handler at all** — three of them outside `General.tsx`,
each one an unhandled promise rejection whose only symptom was a missing answer.
Two of those four were worse than the empty dictionary list that started all of
this:

- **`refreshCatalog` was the worst.** A refusal landed in its `catch`, which sets
  `modelStatus` to failed and puts the **raw error string** on screen beside an
  empty model list — "no models exist", said by an error path, about a machine
  with 2.14 GB of weights on disk.
- **`Advanced.tsx` lost both its sections silently.** Neither read had a
  handler, and the page renders each block only when its value is non-null, so a
  lost race left two headings with nothing under them on the one page someone
  opens to find out what the app is running. Six coordinators stand behind
  `diagnostics_status` alone, which makes it the read most exposed to the race.

**Cleared, with the reason:** `capture_level`, `capture_wizard_status` and
`capture_hud_status` are all read on an interval, so the first refusal costs one
tick and the next one heals it. `session_transcript_log` reads on an event now
and carries its own retry. `capture_devices` takes no `State`.
`model_install_status` resolves its own coordinator with `try_state` and returns
`verifying` when it is absent, so it cannot be refused this way. `history_list`
and `startup_status_view` were registered and never invoked from the frontend at
all; both were deleted rather than cleared.

#### The reported symptom had a second cause, and it is the one that reproduced

`readWithRetry` was in place, the release frontend was bundled, the backend
answered `registration: "registered"` — and the panel still read **"Shortcut not
registered yet"**. The retry was not wrong; there were two causes with one
symptom, and only one of them is a refused read.

`HotkeyCoordinator` starts at `registration: "pending"` and
`register_activation_hotkey` runs at the **end** of `setup`, after the tray is
built. All three eagerly mounted pages have already read by then. So the read
**succeeded** and returned a value that was true for the first moment of the
process, and nothing ever re-read it.

**That is the documented trap one level deeper than it was written.** The handoff
warned that the rendered string cannot tell you whether the backend is wrong or
the read was refused. It also cannot tell you whether the answer arrived early
and never changed — same string, same null-ish page state, same everything. The
two were separated by reloading the settings webview and watching the same page,
with the same backend, report "Shortcut active" (2026-08-26, installed release
frontend). Item 14's original diagnosis was structurally sound and never proved,
and the fix it implied would have shipped without fixing the reported symptom.

`readWithRetry` now takes an optional `settled` predicate: an unsettled answer is
retried like a refusal, and if every attempt is unsettled the **last value is
returned** rather than thrown — a startup value still there after five seconds
has stopped being transient, and `pending` then really does mean the shortcut was
never registered, which is the one case that copy exists for.

#### Both causes reproduced on a build, and the shipped one is worse than reported

Measured 2026-08-26 through CDP against release frontends. The instrument matters:
every reading here compares what the *page rendered from its mount-time read*
against what the *backend answers when asked directly*, because that is the only
comparison that can tell a wrong answer from a missing one.

| | installed 1.5.1 | this build |
| --- | --- | --- |
| Settings → General, shortcut | "Shortcut not registered yet" | **"Shortcut active"** |
| `hotkey_status`, asked directly | `registration: "registered"` | `registration: "registered"` |
| Advanced, fact grids rendered | **0** | **4** |
| Advanced, headings rendered | 5 | 5 |
| `diagnostics_status` / `credential_status`, asked directly | both answer correctly | both answer correctly |

**Advanced on the shipped build renders five headings and no facts at all.**
Runtime, Performance and Credentials are empty — every fact on the page a user
opens to find out what the app is running — while both commands behind them
answer correctly when invoked directly. That is the refused read, and it had
never been seen: item 14 predicted it from the source and this is the measurement.
So the shipped defect is two independent faults on two pages, not one.

**Which pages can lose the startup race is decided by `SettingsApp.tsx`, and by
accident.** General, Transcription and Advanced are mounted eagerly; Audio,
Output & Privacy and the transcript log are mounted only while their tab is
active, so their reads happen long after `setup`. The three eager pages are
exactly the three that had the defect. That correlation is not a mitigation
anybody chose — Audio's conditional mount exists so a hidden page does not sample
the microphone — and making Output eager would bring the race back with it. The
retries on the three lazy pages are therefore defence in depth rather than fixes
for observed faults, and the enforcement lives in the test, which does not know
or care which pages mount when.

#### Two things a retry cannot fix, found while fixing it

- **A default that renders as a claim about the system.** `?? "pending"` is not
  a neutral fallback: `pending` is a real backend state meaning "registration has
  not been attempted", and its copy is "Shortcut not registered yet". So an
  unanswered *read* was reported as an unregistered *shortcut*, and the remedy the
  panel implies — pressing Save hotkey — fixes a problem the user does not have.
  It is now `?? "unknown"`, which reads "Shortcut state unknown". The same
  substitution was needed on Output & Privacy, where `?? "empty"` claimed "No
  result" about a read that had not answered.
- **A page holding a value it has never read, and this one was destructive.**
  `General.tsx` initialised its binding field to **`Ctrl+Alt+L`** — SpeakEasy's
  shortcut, inherited by the fork and never rebranded, one of the identity items
  item 4 missed. With the read lost, the field showed a shortcut this app does
  not use, and the Save the panel invites would have **rebound the working
  `Ctrl+Alt+P` to the other product's shortcut**, on a machine where both are
  installed side by side and would then conflict. The field starts empty, Save is
  disabled until the status has been read, and no page may hold a shortcut as a
  value.

#### The part that stops a third occurrence

`no effect can read a race-prone command without retrying or polling` replaces
the named assertion. Nothing in it is listed by hand: the hazard is derived from
the Rust signatures, and the readers are found by scanning every `useEffect` in
`src/`, following one level of local function calls — which is what reaches
`refreshCatalog`, the read a body-only scan would have missed. A command is
cleared two ways only, retried or polled, and once a command is retried anywhere
no file may also read it bare.

Every assertion in it is of the form "nothing was found", which is exactly what a
broken scanner reports, and two of its three components (balanced-delimiter
scanning, one-level call resolution) fail silently rather than throwing. So it
carries **instrument self-checks**: the derived hazard must contain three commands
known to take a `State` and exclude three known not to, and the scan must be
shown reaching through a helper and reaching the poll exemption. It was then
**proved able to fail** by restoring the original defect in `General.tsx` and,
separately, in `Advanced.tsx` — two files, two commands.

One deliberate consolidation came out of it. `useProfile.ts` carried a
hand-rolled copy of `readWithRetry`'s own 20 x 250 ms, in two files, and
`readWithRetry`'s comment already named the risk: one page recovering from a
startup the other reported as broken. There is one retry now, and the profile
being unread is reported — a null profile renders unchecked boxes and a delivery
preference nobody chose, across three pages, which the old comment acknowledged
and nothing said out loud.

### 15. A press between recording and paste started a second dictation — found 2026-08-25, closed 2026-08-26

Observed on the processor run. The ceiling stopped the capture at 120,183 ms;
the owner pressed `Ctrl+Alt+P` **490 ms later** — the second press of a normal
dictation, for a recording that had already ended — and that press opened a new
one. It ran 6.875 s, was queued behind the first, waited **36.6 s** for it, and
delivered its own transcript.

```text
event=dictation_ceiling_stop result=delivering state=captured code=none quality=none
event=hotkey_capture_device_selected                <- 490 ms later, the user's stop press
event=dictation_start result=ok                     <- a second dictation
```

Everything there was working as built. `on_event` toggles, the queue serialises so
utterances cannot race, and both transcripts were delivered. There was no error to
report and none was reported — which is why only a rule could catch it.

**Owner decision 2026-08-26: one dictation at a time.** A press in the window
between recording ending and the transcript landing is **refused, not queued**,
because the user pressing it is ending a recording that has already ended rather
than asking for another one.

#### The two controllers already disagreed, and that is where the fix went

The dock's Start button had refused this press all along. `can_start` is
`setup.is_none() && !running`, and `running` includes `finalizing` — which
`capture_hud_status` reaches by promoting `complete` to `finalizing` while
delivery is unresolved. The **global shortcut had no such rule**. So the same key
was declined by one controller and accepted by the other, on a codebase whose
stated single-controller principle says "a button press and a shortcut press are
competing for one session rather than opening two".

So the guard is in **`start_dictation`**, which is the one function both paths
converge on and whose own doc says there is deliberately no second start path.
Three parts, and the middle one is the reason this can stay fixed:

- `hud_session_with_delivery` holds the promotion, and both readers call it.
  `capture_hud_status` had its own copy (`session == "complete" &&
  delivery_pending`), and a copy is exactly what let the two disagree. A scaffold
  assertion refuses a second statement of it, proved able to fail by restoring
  the copy.
- `dictation_is_finishing` is the question, answered once. It **fails open** —
  `false` when a coordinator is absent or a lock is poisoned — on purpose: this
  guard exists to suppress an unwanted dictation, and a broken read must never
  suppress a wanted one.
- The refusal is **logged** (`dictation_start result=dictation_still_finishing`)
  and has catalog copy. "The shortcut did nothing" is indistinguishable from a
  broken shortcut, so the log is the only thing that separates them, and the copy
  is what a user gets instead of `errorUnknown`.

The window matters more than it looks: inference alone is 4.2 s on the card and
44.5 s on the processor, and the promotion is what stops the guard opening the
moment inference ends but before the paste. `a_dictation_is_still_finishing_until_
its_transcript_is_delivered` pins that, including that `arming` and `capturing`
are **never** refused — a guard reaching those would stop the shortcut being able
to end a dictation.

### 16. Three true sentences that read as a contradiction — found 2026-08-25

With the CUDA worker staged and NVML confirming a live compute context, Settings
-> Transcription displayed, in this order:

> Dictation runs on: Graphics card (GPU)
>
> This computer's graphics card is supported, but this installation includes only
> the processor model.
>
> Dictation is running on the graphics card, which is more than this installation
> was recorded as providing. Nothing is wrong — the graphics-card engine was
> staged after setup ran.
>
> **The graphics-card engine is detected but has not passed its local execution
> check yet.**

Every sentence is true and the last one is deliberate, not stale.
`GpuQualification::Admissible` means "clears the capability floor, nothing has
been executed on it", `Qualified` means an execution test ran, and
`is_qualified` exists precisely so that "should work" is never advertised as
"has worked" — a distinction this codebase is right to draw and has drawn
carefully. `coordinators.rs` records that the promotion from one to the other
was removed rather than kept as something nothing could trigger, and that it
"comes back with the CUDA worker, not before".

The finding is only about the stack. A user reading four sentences, the first of
which says dictation runs on the graphics card and the last of which says the
graphics-card engine has not passed its check, is being asked to hold an
internal distinction the panel never explains — and the honest middle sentence
already told them nothing is wrong, which the last one then appears to withdraw.
Granite *had* just executed on that card for 4.2 s, so the sentence is
defensible only under a definition of "execution check" that means a formal
smoke test rather than a dictation.

Cheapest correct fix is probably to suppress the qualification line whenever the
device line already reports `cuda`, since the two are then answering the same
question with different vocabularies. Also item 3's dependency: a published
worker restores the promotion and the sentence stops being reachable in this
combination.

### 17. The notice window was 16 px too short for its own copy — found 2026-08-25, closed 2026-08-26

Found the first time anybody saw the window, minutes after item 1b closed. The
`notice` window declares **360x172** and its content needs **188 CSS px**, so the
dismiss button — its only control — sits **16 px below the fold** behind a
scrollbar.

Measured through CDP against the running release build, which is the only
instrument that can answer it (the stylesheet cannot: `height: 100vh` and
`justify-content: space-between` describe a box that always looks full):

```text
viewport (CSS px)     : 360 x 172      <- declared size holds; enforce_declared_size is fine
.capture-notice       : client 311x170   scroll 311x188
  VERTICAL OVERFLOW   : 18 px
  title               : 279x40   (2 lines at line-height 19.76)
  body                : 279x76   (4 lines at line-height 19.02)
  button              : 61x40  top=148 bottom=188
  button below fold by: 16 px
```

**It is not DPI-dependent, which was the first guess and was wrong.** Every
metric derives from a 16 px root in CSS px — title `0.95rem` is 15.2 px, body
`0.82rem` is 13.12 px — so the wrap is identical at every scale. It was found on
a 250% display but it clips on every machine. The second guess was also wrong and
is worth recording because it sounded mechanical: the scrollbar narrows the
content box from 326 px to 311 px, so it *looked* like the classic overflow ->
scrollbar -> narrower -> more wrapping loop. Suppressing the scrollbar and
re-measuring left the title at 40 px and the body at 76 px unchanged. The text
wraps the same either way and the shortfall is 16 px regardless. **A plausible
mechanism is not a measured one**, and both wrong guesses cost one CDP call each
to kill.

The severity is narrower than the numbers suggest, and both halves matter:

- The **informational copy fits**. What happened, that the transcript is safe,
  and what to do next are all readable, so the window discharges its actual
  obligation. This is not a user losing information.
- The **clipped part is the only control**, in a window that deliberately never
  takes keyboard focus, so it is reachable only by mouse-wheeling a window nobody
  has a reason to think scrolls. The 15 s auto-dismiss is what keeps this from
  mattering — the button is a convenience, not the exit.

Raising the declared height to **192** clears it with 4 px spare, measured rather
than estimated: with no overflow the content box is 326 px wide, the text still
wraps to 2 and 4 lines, and 188 px is still what it needs. That is the smaller
change and it keeps the copy, which was an owner decision. Trimming the body to
three lines is the alternative and re-opens a settled decision to save 19 px.

`minHeight` is 172 as well and would have to move with it. Whatever changes,
**measure the running window afterwards** — this entry exists because a
stylesheet that reads correctly described a window that clipped.

**Closed 2026-08-26** at 192, `minHeight` with it, copy unchanged. Measured
afterwards on the running window, which is the whole point of this entry:

```text
viewport (CSS px)     : 360 x 192      <- declared size holds
.capture-notice       : client 326x190   scroll 326x190
  VERTICAL OVERFLOW   : 0 px
  button              : top=148 bottom=188
  clear of the fold by: 2 px
```

**The spare is 2 px, not the 4 px predicted above.** The prediction was made with
the scrollbar suppressed and read the requirement as 188; the real box needs 190.
Both readings were taken the same way and the earlier one was 2 px optimistic —
which is a small number and the right direction to record it in, because the
estimate that produced it was the sound kind and was still wrong. The content box
is 326 px wide as predicted, the scrollbar is gone, and the only control is fully
on screen.

Two other things came out of doing it that are worth more than the two lines of
JSON:

- **`Invoke-WebviewProbe.ps1` could not address the window.** It knew
  `settings`, `dock` and `log`, so the one window whose defect only a CDP
  reading can see was the one window the instrument could not reach. It takes
  `notice` now — and because a window declared `visible: false` still runs its
  React tree, it answers without provoking a ceiling stop, so this is measurable
  in seconds rather than needing two minutes of speech.
- **The stylesheet's own account of the size is now checked against the config.**
  `styles.css` opens the notice's rules by naming its declared size, which was
  the number that was wrong; nothing compared the two. `every window is declared`
  now parses both and requires them to agree, plus `minHeight == height` for a
  window that cannot be resized. That cannot measure the running window — nothing
  in the gate can — but it stops the wrong number being restated as though it
  were checked.

## Decisions already made — do not re-open without new evidence

Every one of these was an explicit owner decision this session.

- **One engine, one pass, no fallback.** Granite produces the transcript and its
  punctuation together. A pass that fails, returns nothing, or fails the
  plausibility gate **ends the dictation with a named reason**. Substituting a
  weaker result is the behaviour this fork exists to remove.
- **Failures must be actionable.** Two surfaces: the dock's action row, and a
  panel at the top of Settings → Transcription. A Windows toast was specified,
  then refused — the WinRT route needs an AppUserModelID from an installed Start
  Menu shortcut and otherwise displays nothing *while reporting success*.
- **No provider-override control.** Granite's GPU support is a build feature; no
  setting can conjure a worker binary.
- **The dictation floor is Granite's floor** (8 GiB), raised from 4 GiB on
  2026-08-18. The two were split so a machine that could not host Granite still
  dictated through the streaming path; with one engine that split only let
  someone speak into a guaranteed `GraniteUnavailable`. Refusing before capture
  is the same answer at the only useful moment.
- **No in-app setup wizard.** The installer is the only setup path.
- **`immediate_repetitions` and `self_corrections` never run**, and have no
  toggles.
- **Q4_K_M ships; Q8_0 is catalogued and not install-eligible.** That asymmetry
  is deliberate and is now pinned by a test.
- **The transcript log is pinnable**, into its own non-focusable always-on-top
  window, and retention defaults to clearing on close — implemented by never
  writing rather than deleting on exit, because a delete-on-exit is a promise a
  crash breaks.
- **Distribution is two paths**: a one-exe installer from GitHub Releases, and
  handing the repo to an LLM so a cautious user can watch every step. Both are
  in `README.md` and both must keep working.

### 18. `--keep-user-data` deleted the user's data — found and fixed 2026-08-26

**It cost a real profile**: 4.28 GB of weights, `settings.json`, the vocabulary
in `personalization.json`, both `.bak` copies and the log, on this machine,
during the 1.6.0 release run. Removal does not use the recycle bin, so none of it
came back.

The command was `speakeasy-bootstrapper --uninstall --keep-user-data`, run to
clear the machine for `Test-InstallerLifecycle.ps1`. `remove()` computes
`Removals::default()` from that flag — select nothing — and then **discards it**:

```rust
let removals = if silent { removals } else { uninstall_page::ask(...) };
```

`uninstall_page::ask` took no argument and built every check box with
`check_state: co::BST::CHECKED`. So the interactive path drew a page primed to
delete the whole profile, from a command whose name says the opposite, and the
owner clicked the answer the page was already set to. **The click is not the
defect.** A page that opens on the inverse of what the caller asked for is.

**Why nothing had caught it.** The flag was only ever honoured in company with
`--silent`, and `--uninstall /S --keep-user-data` is exactly what both proof
scripts pass — `Test-InstallerLifecycle.ps1` twice and `Test-SetupWizard.ps1`
once. The interactive-plus-flag combination had never been run by anything. The
2026-08-21 work that inverted the default and built the page is where the two
diverged; `Removals::default()` still "selects nothing, because a *caller* that
forgets to ask must delete nothing", and that reasoning is intact — what was
missing is that a caller which *did* ask never reached the control.

**The fix is that `ask` takes the `Removals` and seeds the boxes from it.** The
page still decides, because someone is looking at it and can tick anything back
on; it just opens on the answer that was requested. One caller, so the change is
contained.

**The first regression test would not have caught a revert**, and that is worth
more than the fix. It asserted a `check_state` mapping the test itself defined,
so restoring `check_state: co::BST::CHECKED` in the real code satisfied every
assertion — the same trap as
`the_ceiling_stays_inside_the_pipeline_byte_limit` holding its own copy of a
constant. The version that shipped reads `include_str!("uninstall_page.rs")`,
slices out `check_boxes`, and requires the state to be *derived* from `initial`;
it carries an instrument self-check that the slice is not empty, and it was
proved able to fail by restoring the bug.

**Two general lessons.** A flag that states an intention must reach the control
that acts on it — computing it and passing it to one of two branches is not the
same as honouring it. And **a safe-sounding flag is not evidence**: the pre-flight
guard named the remedy, the flag read as the careful choice, and nothing checked
that it arrived. Verify the outcome on disk, which is what exposed this: the
uninstall printed `Removed: … configuration, transcript history, installed
models, …` and the profile directory was gone one command later.

### 19. History was written before the target was classified — fixed 2026-08-28

`run_retained_transcription` ended a successful pass with
`history.record(&TranscriptResult { .. secure_target: false })?` — before the
session log, `results.accept`, `mark_transcription_finished`, and before
`deliver_final_text` had inspected any window.

Two defects from one statement. **Privacy:** the flag is the only thing
`HistoryRepository::record` refuses on, and the only production caller passed a
literal, so a dictation into a password field was refused delivery and stored in
plaintext — against `historyDisclosure` and `PRIVACY.md`. A guard whose input is
a constant is not a guard, and its unit test proves nothing about the shipped
build. **Reliability:** the `?` meant any `SQLite` error discarded a good
transcript and skipped `mark_transcription_finished`, latching the dock on
`finalizing`.

**Fix:** `deliver_final_text` returns a `DeliveryTarget`; only
`persist_delivered_history` writes the database, and only after delivery. The
sensitive-refusal list is stated once in `refusal_is_sensitive`.

**The ordering is a data dependency, not a convention.** `FinalizedDictation`
carries the pending history row and is produced *only* by
`publish_successful_transcript`, which records the session transcript, accepts
the result and finishes the capture before returning it. A history row therefore
cannot exist until everything a dictation depends on has been published.
`history_is_persisted_only_after_delivery_classifies_the_target` is a source
scan and is labelled in its own doc comment as a temporary architecture guard —
delete it if the persistence call becomes structurally immovable, do not grow it.

**Two test lessons, both from getting this wrong first.** The original
regression test broke the database at *open* time, which the old code handled by
skipping the write and returning `Ok` — so it would have passed against the very
bug it was written for. **A control has to restore the real defect, not a
plausible-looking one**: restoring a fallible pre-publication write makes the
current test fail with `publish the final: "history_write_failed"`, which is the
original symptom exactly. And that test asserted on `HistoryCoordinator.session`,
an in-memory list production writes and never reads — the test was its only
reader, which would have preserved dead state to satisfy a test. `session` and
`retain` are deleted; the state that matters is `SessionTranscriptCoordinator`,
which the pinned log window actually renders, and that is what the test asserts
now. `a_write_can_fail_after_the_repository_opened_successfully` in
`speakeasy-storage` proves the post-open failure shape is reachable at all.

**Open:** with auto-paste off nothing is inspected, so dictate-then-paste-by-hand
into a password field still leaves a history row. Closing it means inspecting the
foreground on a path that does not deliver.

### 20. The 10 Hz dock poll walked the audio devices — fixed 2026-08-28

`setup_requirement` called `CaptureWizardCoordinator::devices` — a full WASAPI
walk — on every tick, four lines below a comment saying enumeration is far too
expensive at 10 Hz. A comment stating a constraint is not enforcement. Cached for
two seconds; nothing is gated on it, so a stale answer only delays "Setup needed"
clearing. `useHudStatus.ts`'s `inFlight` ref was documented as guarding the poll
and did not; the poll self-schedules now.

### 21. Startup hashed the 2.30 GB model three times — fixed 2026-08-29

`InstallManager::reverify` collected every file's bytes before verifying any, a
single 2.30 GB allocation inside `ModelCoordinator::new`. It streams now.

**It was three hashes, not two, and the entry above undercounted.** `readiness`
called `reverify`, and `readiness` runs **twice** on a configured launch — once
synchronously inside `ModelCoordinator::new` on the `setup` path, and again from
what was then `refresh_readiness`, after the warm. With the warm's own
`verify_pack_files` that is 3 x 2.30 GB, about **6.90 GB** of reading before the
app is usable. The first draft said "twice" because it counted the two functions
and not the two calls.

**Now one, and it is the warm's.** `readiness` uses `InstallManager::is_present`
— exactly what that function is documented for — and reports `verifying`.
`ModelCoordinator::settle_after_warm` promotes to `verified_on_disk` on a warm
that reached `ready`, or to `failed` with `granite_model_files_unverified` when
the warm says the bytes are wrong. Any *other* warm failure leaves `verifying`,
because a missing worker or a memory floor says nothing about the model's bytes
and condemning it on that evidence would be a manufactured fault.
`setup_requirement` accepts `verifying`, so the dock does not flash "Setup
needed" for the seconds a warm takes.

The surviving hash is the right one to keep: it is taken immediately before the
worker is handed the `model_root`, and it already runs on its own thread — so
the asynchronous verification this entry deferred is what the code now does,
without the separate state machine that deferral assumed it would need.

**It is still not an execution-time check, and that stays open.** It runs in the
desktop process and the worker reopens the files by path; `granite-worker`'s own
docs say it checks presence rather than digests, deliberately, because the
caller verified. Closing that gap means verifying inside the worker or handing it
already-verified handles. Needs a threat-model decision, not more code.

### 22. The premature "delivered" notice, and five other honesty repairs — 2026-08-29

**The ceiling notice claimed delivery before transcription started.**
`show_capture_limit_notice` runs *before* `transcribe_and_deliver`, and the pass
that follows can find no speech, fail the plausibility gate, time out or be
refused by a password field. On a CPU install the user read "your transcript was
delivered" up to 44 s before the text landed. It now says it is being
transcribed; what became of the text is the dock's to report.

**Bootstrapper tests were not hermetic.** One read, overwrote and deleted the
developer's real `install-provider.txt` under `%APPDATA%`, restoring it only on
the success path — so any panic left a real installation reading `unrecorded`.
`clear_installed_provider_in` takes a directory now and the test uses a
`tempfile::tempdir`. The registry test got a per-process key name and a `Drop`
guard, and sixteen fixed paths under the shared temp directory got the process
id, so two `cargo test` runs no longer collide. 76 pass, 0 fail; it was 74 pass,
2 fail in the reviewer's sandbox.

**Every dictation ran a full hardware inventory for one number.**
`SafeStandardHardwareProbe::probe` builds a `System::new_all`, refreshes the disk
list, walks the registry for display adapters and reads the OS build;
`run_retained_transcription` called it immediately before inference to get
`total_memory_bytes`, which cannot change without a reboot.
`speakeasy_models::total_physical_memory_bytes` is the memory-only query. The
full probe stays where the Advanced page renders every field of it.

**Settings actions had no failure state.** Diagnostics export, reset preview,
history export and history delete were all `void invoke(...).then(set...)` with
no rejection handler: an unhandled rejection, a control that reports success by
not erroring, and optimistic state that stayed changed after a refusal. The
delete was the worst — it cleared its own confirmation check box and printed
"Deleted" whether or not the database had been touched. `useMutation` gives them
pending, success and catalog-mapped error states, refuses double submission, and
updates caller state only on success.

**Public documentation contradicted the product.** The testing guide said the
repository was private and that setup fetches WebView2 — it detects and
instructs, and the dead `downloadBootstrapper` in `tauri.proof.conf.json` is
gone. Model sizes disagreed across three documents and none matched the
manifest: it is **2,298,601,952 bytes** (2.30 GB decimal, 2.14 GiB), plus
438.5 MB of transfer on the graphics-card path, downloaded by *setup* rather
than at first launch. The privacy table described a "session clear" control that
does not exist and auto-copy/auto-paste as separate settings. `SECURITY.md`
named the native bridge and ONNX Runtime, both removed by the fork. The user
guide numbered its steps 1-5 then 4-5 and documented a "Capture and Transcribe"
workflow whose buttons do not exist.

**Two rustdoc warnings fixed**, so `RUSTDOCFLAGS=-D warnings` is clean on the
crates that had them: a public method linked to a private associated constant,
and a Windows command line indented inside a doc comment was parsed as Rust.

### 23. Three hashes, and the fix for them shipped two new defects — 2026-08-29

**The count was three, not two.** `readiness` called `reverify`, and `readiness`
runs twice on a configured launch — once synchronously in `ModelCoordinator::new`
on the `setup` path, once after the warm — so with the warm's own
`verify_pack_files` a launch read the 2.30 GB pack three times, about 6.90 GB,
before the app was usable. Item 21 said "twice" because it counted the two
*functions* and not the two *calls*. `admit_streaming_runtime` holds a fourth
`reverify` and is dead code with no callers.

**The first fix introduced two defects of its own**, and both are the kind this
repository exists to catch.

`readiness` was made to report `verifying`, and `settle_after_warm` promoted out
of it only on `ready` or `granite_model_files_unverified`. Every other way a warm
can end — no worker, memory below the floor, nothing configured, quarantine —
fell through a catch-all that returned the presence answer unchanged. So a
*finished* warm left the model line reading "Verifying installed model" for the
life of the process, with the Transcription page polling `model_install_status`
every 750 ms behind it. **A state that claims an action is in progress when no
thread is doing it is a manufactured claim**, and it was introduced by the change
that removed two others.

And the promotion was keyed on a `&'static str` warm state. `settle_after_warm`
**re-resolves** which pack is installed, using the post-warm CUDA answer — which
is precisely the thing the warm discovers and can change. So "the warm said
ready" was being read as "these bytes were checked", and on a machine where the
worker's capability flipped the resolution it would have stamped pack B verified
on pack A's digests.

**What it is now.** `readiness` reports `installed_unverified`: files present at
their pinned lengths, bytes unread. `WarmVerification` carries the pack id and
revision the digest pass actually hashed, and `settled_model_state` promotes only
on an exact match of both — matching the id alone is the obvious half-fix and it
passes a same-pack-different-revision upgrade. `verifying` is set by
`mark_verifying` for exactly as long as a pass is running, and the settle maps any
`verifying` input to `installed_unverified` so the invariant holds regardless of
what `readiness` reports. `setup_requirement` returns `model_verifying` and
`start_dictation` refuses on the same state, in the one function both the dock
and the shortcut go through.

**Two test lessons.** The first regression test drove `ModelCoordinator` over an
empty root, so `readiness` answered `absent` and the buggy branch was never
reached — the test stayed green when the bug was restored. The decision is pure
and is tested as such now. And the identity test asserted a *source string*,
which broke the moment the logic was extracted while the invariant was untouched;
it drives the function instead.

### 24. The rest of the batch — 2026-08-29

**Bootstrapper temp paths are real `TempDir`s.** Twelve sites named a fixed path
under the shared temp directory; the pid suffix fixed cross-process collision but
not leak-on-panic. The two `absent` sites keep a pid-suffixed name deliberately —
they must name a path that does *not* exist.

**Total physical memory is measured once per process.** A `OnceLock` rather than
a TTL, because installed memory cannot change without a reboot and a process does
not outlive one; a refreshing cache would be a mechanism defending against
nothing. `TotalMemoryProbe` makes the measurement injectable so the test can
*count* it — the claim the cache makes is unobservable otherwise.

**React Hooks lint is enforced.** `App.tsx` carried a comment recording a real
Rules-of-Hooks violation and noting the plugin was not installed. Both recommended
rules are errors, and they were proved live by injecting a conditional hook.

**Three stale comments corrected**: the GPU-qualification deletion note, which
said no CUDA worker was published and named its own trigger — that trigger fired
on 2026-08-26 and nothing came back, and restoring `record` is still refused
because `ExecutionEvidence` needs an `inference_sample_count` nothing at warm time
has; `runtime_wizard.rs`'s "cuBLAS and cuDNN", when the manifest pins cudart and
cuBLAS; and Advanced's "ONNX Runtime · CPU" display-name example, naming a runtime
the fork removed.

### 25. A shared "last warm" field, and a floor checked after the recording — 2026-08-29

Two more corrections to the verification work, both found by review rather than
by a test.

**`ensure_ready` returns a resident adapter before any digest pass**, so a second
warm hashes nothing. `settle_after_warm` was reading the coordinator's shared
`WarmVerification` field, which still held the *first* warm's verdict — a claim
about a pass that did not run in that invocation, and the pack-mismatch defect
re-entering by another door, since the resolution can change between two warms.
The outcome is per-invocation now, `AlreadyLoaded` is its own variant, and every
early return in `warm_granite_if_configured` records `NotAttempted` rather than
leaving the previous verdict standing. `AlreadyLoaded` still promotes when the
identity matches — a pass did run earlier in this process, on those exact bytes —
and it is still compared, so it cannot vouch for a pack it was never about.

**`mark_verifying` ran inside the spawned thread and fired unconditionally.** Two
faults: a window between spawn and the digest pass in which the dock and the
shortcut were exposed to a model about to be hashed that did not say so, and a
`verifying` flash on machines with **no model installed** — the app announcing it
was checking something the user does not have. It runs before the thread now, and
is a no-op unless a pack is present.

**The 8 GiB memory floor was checked after the recording.** `runtime.begin` is
called from `run_retained_transcription`, so a machine below the floor let the
user speak for up to two minutes and *then* reported that the engine could not
start. The recorded decision says the opposite in as many words — "Refusing at
`begin`, before a sample is captured, is the same answer at the only useful
moment" — so this was a decision that had quietly stopped being implemented.
`setup_requirement` now reports the three terminal engine states
(`granite_worker_missing`, `memory_below_granite_floor`, `granite_quarantined`)
as setup reasons, which gates the dock and, through `start_dictation`, the
shortcut. Only terminal answers gate: `cold` and `warming` are a warm in flight,
and `granite_model_files_unverified` is the model's fault and already reported.

**The memory-cache test was weak and is now real.** It asserted value stability
of the process-global `OnceLock`, which a function with no cache at all also
satisfies. `cached_total_memory` takes the cell as a parameter so the test owns a
fresh one and counts the probe: a hundred calls, one measurement. Proved by
removing the cache, which turns it red.

**Remaining test-only pid paths are `TempDir`s**, including the two `absent`
sites, which now name a child of a directory that exists rather than guessing at
an unused name in the shared temp directory.

**The `HKCU` test is reclassified rather than faked.** A double would assert this
test's model of the registry rather than the registry, and the thing under test
*is* the registry semantics. It keeps the per-process key and the `Drop` guard,
and declines loudly when the environment cannot write to `HKCU` — printing that
the behaviour was **not** exercised, because a silent skip is indistinguishable
from a pass.

**A counting verifier over synthetic files** now pins that a digest pass reads
each required file once and refuses a single flipped byte by name — at the same
length, so presence and length both still pass and only the digest can catch it.
The shipped pack is 2.30 GB and cannot be laid out in a test; this is the same
production function over files a checkout can hold.

**The fourth stale comment**: `CudaRuntimeView` described a 2.97 GB on-demand
runtime download that left with the streaming engine. Marked historical, with the
hazard it recorded kept — a transient state written to the model coordinator
makes a ready app announce "Setup needed", which is exactly what a `verifying`
leak did.

### 26. Engine warm and dictation-refusal invariants — as of 2026-08-30

State, not history. Four things hold and each has a test that fails without it.

- **A warm's verdict is returned, never stored.** `warm_granite_if_configured`
  and `GraniteEngineCoordinator::ensure_ready` answer with a `WarmOutcome` /
  `EnsureReadyOutcome` carrying the verification *and* the error, so a warm that
  hashed a pack and then failed to spawn a worker can still condemn those bytes.
  There is no "last warm" field on the coordinator: a dictation's own warm calls
  the same `ensure_ready`, and a field would let it overwrite a verdict the
  launch warm's settle was about to act on.
- **A resident adapter answers only for the pack it holds.** The loaded identity
  lives with the adapter (`ResidentPack`). `resident_answer` returns `Reuse` on
  an exact id-and-revision match, carrying an `AlreadyLoaded` built from the
  *resident* identity; anything else is `Refuse`, which carries no adapter at
  all. `ensure_ready` turns `Refuse` into an `Err`, clears the slot so the next
  call warms the requested pack, and reports
  `granite_resident_pack_mismatch`. Latent while one Granite pack is admitted;
  `granite_selection` re-resolves per warm with `cuda_worker_available()`.
- **One rule refuses a dictation.** `dictation_blocker` is pure and is consumed
  by both `setup_requirement` (the dock) and `start_dictation` (the shortcut and
  the dock's button). It answers `model_missing`, `model_verifying`,
  `granite_worker_missing`, `memory_below_granite_floor`, `granite_quarantined`
  or `microphone_missing`, all before any audio is captured, and every code has
  catalog copy. An unmanaged coordinator refuses nothing.
- **A digest pass is counted, not asserted.** `PackVerifier` is injected through
  `GraniteEnvironment`; `one_warm_takes_exactly_one_digest_pass` counts what the
  warm asked. The shipped binary names `TrustedDigestVerifier` at both
  composition roots.

**Open:** the surviving digest pass runs in the desktop process and the worker
reopens the model by path, so it is not an execution-time integrity check. Item
21. Needs a threat-model decision.

### 27. Settings mutations, the component harness, and coverage floors — as of 2026-08-30

- **Every settings action reports its own refusal.** All of them go through
  `useMutation`, which resolves to `null` on rejection so callers update state
  only on success, and refuses a second submission while one is in flight. The
  five profile writers share one mutation, because they write one `ProfileView`;
  `SettingsApp` renders its error once for the whole workspace. The install
  status poll is the one exception and is not a mutation: it reports that the
  *progress* is stale, because a read that did not arrive says nothing about the
  bytes on disk. Every code any of them can return has catalog copy.
- **`tests/components/` is vitest over jsdom.** The only suite that can press a
  button; `tests/*.test.mjs` stays with `node --test` for reducers and source
  scans. Two things to know before writing one: the `invoke` double rejects with
  a **bare string**, because that is what Tauri hands back for a
  `Result<_, &'static str>` and an `Error` maps to no catalog entry; and an
  unanswered read resolves to `undefined`, which a component stores and then
  dereferences, so a missing stub surfaces as a `TypeError` inside a render.
- **Coverage is a floor per file**, in `dependency-policy/coverage-floors.json`,
  over the privacy, delivery and mutation modules, checked by
  `scripts/Test-CoverageFloors.ps1`. A file named there and **absent from the
  report fails** — that is how a coverage check silently stops guarding anything.
  Floors only go up; the script prints the headroom rather than failing on it,
  because a self-tightening ratchet goes red on unrelated work and then gets
  disabled.
- **The recent-transcripts list is not "this session only".** It is seeded at
  launch from the optional on-disk history, so with retention on it spans earlier
  runs, and `history_delete_all` clears it. Both facts are in
  `sessionLogDetail`, because a user can infer neither.
- **The `HKCU` uninstall proof is `#[ignore]`d.** It writes to the real registry,
  so it is out of the default gate and explicitly invoked; once invoked, an
  environment that cannot write `HKCU` fails rather than skipping.


### 28. Polling — as of 2026-08-30

Three periodic reads, and what each is now.

- **The transcript list does not poll.** One retried read on mount, then one read
  per `transcript-log-changed` — a content-free event emitted to `main` and `log`
  by `notify_transcript_log_changed` when a transcript is published and when the
  saved history is deleted. **The payload must stay `()`**: an event reaches every
  listener in the window with no per-command guard, so text in a payload would
  bypass `session_transcript_log`'s `require_main_or_log_window`. The read is
  through `readWithRetry` because an event-driven read has no next tick, and both
  windows mount before `setup` manages the coordinator — the `log` window runs its
  React tree whether or not it is shown.
- **Audio makes one call, not two.** `capture_audio_snapshot` returns the meter
  and the device-health fields together, so the two halves cannot describe
  different moments, and the poll is self-scheduling: at most one request
  outstanding, the 100 ms gap measured from when the last one settled.
- **The install poll is self-scheduling too**, same shape, 750 ms gap. It reaches
  the model coordinator's lock, which an in-progress install holds.

Measured by counting `invoke` calls under fake timers
(`tests/components/polling.test.tsx`): the transcript list goes from 40 calls a
minute while idle to 0, and Audio from 20 IPC calls a second to at most 10.

**The rule that enforces it** is `no effect can read a race-prone command
without retrying or polling`. Two exemptions, both shown reachable by the test's
own self-checks: retried through `readWithRetry`, or repeated by a timer in the
effect (`setInterval` or the self-scheduling `setTimeout`). Anything else is a
finding.

## Mistakes made this session, so they are not repeated

- **A whole crate went red unnoticed.** The manifest trim broke
  `speakeasy-models`, and "tests pass" was reported after running three crates
  by name. Run the workspace, not a list.
- **A code was renamed without following it.** Silence changed from
  `runtime_no_speech_detected` to `no_speech`, and three match arms still tested
  the old spelling — so saying nothing took the failure path and earned a
  quarantine strike. `is_no_speech` and a test cover it now. When a stable code
  changes, grep every literal.
- **A replacement test asserted the opposite of the decision.** Rewriting the
  pack test, the first version demanded both Granite packs be installable. Q8_0
  is deliberately not. A test written from the code's shape rather than the
  decision's intent will happily pin the bug.
- **Two edits were lost to a script that raised before writing.** Several
  multi-edit Python passes assert on each replacement and write at the end; when
  a later assertion failed, the earlier successful edits were discarded silently
  and the reported "ok" lines were lies. Write incrementally or verify after.

  **This recurred on 2026-08-18**, in a two-edit pass over
  `Stage-DevRuntime.ps1`, having read the warning above earlier the same
  session. The second assertion failed on a backslash the heredoc had mangled,
  and the first — already applied in memory — went with it. The rule is not
  "be careful"; it is *write after each replacement*, because the failure is
  silent and the transcript still reads as progress.

- **A syntax check reported success without running.** A PowerShell parse check
  passed `[ref]$errs` for an undeclared `$errs`; the statement failed, `$errs`
  stayed null, and the `else` branch printed "parses clean" — for any input,
  including a file that had never been read. The re-run declared the variable
  *and* fed a deliberately broken script through the same check to confirm it
  reported 2 errors. A verification that cannot fail is not a verification, and
  this one had the exact silent-success shape the rest of this project is
  written to avoid.

- **Three checks reported clean because they were broken, not because
  anything passed.** A PowerShell parse check passed `[ref]$errs` for an
  undeclared variable, so it printed "parses clean" for any input, including
  a file it had never read. A wait loop grepped for `error` and matched
  `thiserror`, reporting a build finished while it was still compiling. A
  `Win32_Process` filter used URL-encoded quotes, matched nothing, and was
  read as "no instance running" -- a `Get-Process` a second later found one.
  Each was caught only by looking at something else afterwards. **An
  instrument that fails silently is indistinguishable from one that passes**,
  which is the failure this codebase's comments are mostly about, so run the
  control *first* rather than after being surprised.

- **A regression was reported that was not one.** Two window measurements
  taken during a slower cold start showed the dock hidden, focusable and the
  wrong width; both were mid-startup, before `configure_hud` had run. The
  settled state was correct. Measure after the thing has settled, and say
  "not yet settled" rather than "regressed" when it has not.

- **A failure was diagnosed twice from a line number.** The installer
  lifecycle error pointed inside a helper called three times, and it was read
  as the first call. The first call had succeeded; the failure was two calls
  later, under different conditions. A line number inside a shared helper does
  not say which invocation.

### 2026-08-19, second session

- **The outstanding entry described one third of the work, again.** "Finish the
  seed channel" named a missing writer. The writer was the small half: three of
  the wizard's eight pages were placeholders, so there were no answers to write,
  and the two failures that actually stopped a user — no single file to download,
  and a Finish button that started nothing — were not in this file at all.
  **Both are gaps *between* components, which is why nothing recorded them:**
  every component's own tests pass over a gap. Before believing an outstanding
  entry, walk the user's path once and see where it stops.
- **A proof that drives the developer's layout proves the developer's layout.**
  `Test-InstallerLifecycle.ps1` was green for weeks against `bootstrapper.exe`
  plus a sibling `payload\`, a shape no user has ever had. It now drives the
  single file as well, and compares every placed byte against the packaged one.
- **Two instruments failed silently inside one afternoon, both in the new
  proof.** `FindWindow($null, …)` searched for class `""` because PowerShell
  marshals `$null` as an empty string, and reported the wizard missing with the
  wizard on screen. `, @(...)` handed a whole control list to `Where-Object` as
  one object, where `-eq` filters instead of comparing, so the page heading came
  back as the entire window's text. Neither errored. Both were caught only
  because the assertion downstream of them was specific enough to notice.
- **A race read as data loss.** The vocabulary lands in `personalization.json`
  slightly after the seeds are consumed, so checking for it the instant the last
  seed disappeared found no file and reported the words lost. They were there a
  moment later. When asserting on a side effect of startup, wait for it.

### 2026-08-19

- **A bulk edit cut the wrong lines, twice, and only a dry run caught it.**
  A script deleting whole Rust items ended each cut at "the first line equal to
  `}`", which for any item with a nested block is an *inner* brace: it removed
  the first third of two functions and left the remainder dangling. The second
  attempt matched multi-line byte strings and silently found nothing, because
  the files are CRLF and the patterns were LF. Both were caught by printing the
  intended cut before writing, not by reading the result afterwards. **A
  structural edit needs a dry run and brace counting**; line-shape heuristics
  are guesses about syntax.

- **A reflow pass churned comments nobody had edited.** Rewrapping every ragged
  paragraph in a touched *file* rewrote text the change never went near, which
  buries the real diff. Scope a formatting sweep to paragraphs containing a line
  the diff actually added. Related: a short comment line is only a defect when
  it is **not** the paragraph's last line — a one-line doc comment is supposed
  to be short, and a detector that misses that flags dozens of false positives.

- **`repr()` is not a way to count backslashes.** Repairing a mangled JS regex,
  a Python `repr` showing `/\\/g` was read as two backslashes when it is one, so
  the "fix" replaced the broken form with itself and reported success. The file
  still had a syntax error. Build such strings from `chr(92)` and assert on the
  count, or check with something that is not itself escaping the output.

- **A heredoc is the wrong tool for a file full of quotes.** Two attempts to
  write `smoke.rs` through a shell heredoc died on its own quoting before a byte
  reached disk. The dedicated file-writing tool took it unchanged. Reach for the
  shell for edits, not for authoring source that is dense in `"` and `\`.

- **New assertions found real things immediately, which is the argument for
  writing them.** "The flag appears in exactly one place" found a second
  `creation_flags` call in its first run. "The generator's sentence matches the
  constant" existed only as a promise in a comment. Neither was hypothetical.

- **Three scoping questions were answered by looking rather than guessing**, and
  every one changed the work: `install_root`'s recorded defect was already
  fixed and a worse one sat beside it; onboarding reached persisted settings and
  an IPC contract; the retention question turned out to be one third of a seed
  channel with no writer. **Cost a few minutes of grep each.** The pattern in
  this file is that an outstanding entry describes the symptom someone noticed,
  not the condition under it.

## Repository facts worth knowing

- `kwp490/speakeasy-granite-rust-mini` on GitHub, public, releases included.
  Six of them are 2026-08-19's, `e03eb78`..`da612fa`.
- The tree is ~300 files, down from 2,611 — `vendor/transcribe.cpp` alone was
  2,265 of them.
- `speakeasy-worker` (was `speakeasy-asr`) links **no native libraries** and
  checks in seconds. Only `speakeasy-granite` compiles C++.
- `.cargo/config.toml` is gone. It existed only to point `SHERPA_ONNX_LIB_DIR`
  at a pre-fetched archive. **`.cargo/audit.toml` is back**, though: it was
  collateral damage of deleting that directory wholesale, and the gate needs it.
- `docs/handoff/` and `docs/archive/` were deleted with the streaming engine.
  This file is the new one.
