# Handoff — SpeakEasy Mini, as of 2026-08-20 (third session)

The state of the fork, what is finished, what is not, and the things that will
cost you an afternoon if you rediscover them yourself.

Read `CLAUDE.md` first. This file assumes it.

> **Picking this up cold?** Items 0, 0b, 1b, 8, 9 and 10 are all **done** — read
> them for what they found, not for what to do. The one open item left needs the
> owner rather than an agent: **item 3** (publishing the CUDA worker,
> deliberately deferred past 1.5.0). Item 2b is the release itself.
>
> The three findings item 0 produced were all closed on 2026-08-21, and two of
> them were closed by *measuring* rather than reasoning: `cudart64_13.dll` is
> genuinely never loaded and `cublasLt64_13.dll` genuinely is, both proved by
> deleting them.
>
> **Item 1b closed on 2026-08-25 and produced seven findings of its own** — items
> 11 to 17, of which **14 and 17 are defects** and the rest are a retired risk,
> two documentation errors and two honest behaviours nobody had written down.
> Item 17 is the cheapest fix in this file and was found by *looking at a window
> for the first time*. The single most
> useful thing it settled is arithmetic rather than a measurement: the
> `max_new_tokens` truncation this repository has been hunting since the fork is
> **unreachable through the hotkey path**, because the capture ceiling caps a
> dictation at roughly a fifth of the token budget. Item 11.

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
| The ceiling's notice window | **seen for the first time, and 16 px too short for its own copy** — item 17 |
| Installer lifecycle | `Test-InstallerLifecycle.ps1` passes, including the single-file path |
| **The wizard, end to end** | `Test-SetupWizard.ps1` passes: eight pages, real install, engine check, launched app |
| Setup's engine check | transcribes the bundled clip through the real worker in ~5 s |
| `speakeasy-granite` | compiles, ~2 min cold |
| Broken doc links | none, `--document-private-items` and denied, workspace-wide (four were reintroduced on 2026-08-19 and cleared 2026-08-20) |
| **The provider a machine reports** | proved, never chosen. All five states now produced on hardware — see item 0 |
| Graphics-card path | **run on hardware** (2026-08-21, RTX 4070 Laptop): resident pass 361 ms on CUDA against 2,928 ms on the processor, transcript byte-identical. Still nothing published |
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

**Colour, and no bold.** Emphasising a label's font needs `WM_SETFONT`, `winsafe`
only sends messages through an `unsafe` call, and the workspace forbids `unsafe`.
`WM_CTLCOLORSTATIC` is safely wrapped, so the emphasis a reader gets is ink plus
position plus brevity. Verified on screen at 250% rather than assumed: a
`WM_CTLCOLORSTATIC` handler that is never reached looks identical in every
measurement. Accent renders blue, warning red, good green, and
`Measure-NativeWindow.ps1 -Fit` reports every label on all eight pages fitting
its box at 240 dpi — the key band holds two lines with room, the body four.

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

- The wizard at 250% on a 3840x2400 display: client rect **1550x1250 physical =
  620x500 logical**, exactly as declared, and **every label and control fits its
  box** by `Measure-NativeWindow.ps1 -Fit`. The longest new string — the
  diagnostic-log checkbox, 70 characters — wraps to one line in a 1470 px box.
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

Three defects fell out that no citation sweep was looking for:

- `speakeasy-granite`'s crate doc claimed the delivered transcript came from the
  *streaming* model run twice. It has not since the fork.
- **Eight invisible U+009D control characters** in comments across five files,
  present since the first commit, each following an em-dash. Found only because
  a scripted replacement refused to match a line identical on screen.
- **Three broken rustdoc links**, which `cargo doc` had never reported —
  see `CLAUDE.md`'s entry on `--document-private-items`.

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
**the dock is the only HUD**. Five commits, on a fresh history, pushed to a
private repository.

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
four of the roughly five lines it reserves.

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
  nothing in the tree connects the two — no test, no comment, no assertion. That
  is the actual finding.
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

### 14. Settings permanently reports the shortcut as unregistered — found 2026-08-25

`Settings -> General` reads **"Shortcut not registered yet"** while the shortcut
is registered and working. Confirmed both ways on 2026-08-25: the panel said it
for the life of the process, and `hotkey_status` invoked directly against the
same window returned `binding: "Ctrl+Alt+P", registration: "registered",
enabled: true`. Dictation then worked twice.

`General.tsx` reads it with a bare `invoke<HotkeyStatus>("hotkey_status")` — no
rejection handler, no retry — and renders
`formatShortcutState(hotkey?.registration ?? "pending")`. Every window's webview
loads while `setup` is still managing coordinators, so that read can be refused
with "state not managed for field `state` on command …", and `hotkey` then stays
`null` for the life of the process.

**This is the 2026-08-20 defect in a second location.** That one was
`personalization_status` showing an empty dictionary with three words on disk,
and the fix was `readWithRetry`. Only `Transcription.tsx` was converted —
`model_hardware` and `personalization_status` — and the sweep stopped there.
`readWithRetry.ts` has exactly one importer.

It is worse than the original in one respect and that is why it is written up
rather than merely listed. An empty list says "you have no protected terms",
which is wrong but passive. This says a working feature is broken, in the one
panel a user opens *because* their shortcut appears not to work — and the
remedy it implies, pressing "Save hotkey" to re-register, is a fix for a problem
they do not have.

The fix is a one-line change to the import and the call. It was deliberately not
made in the session that found it, because that session's job was to measure a
dictation and a UI change would have invalidated the build under test. **Check
every other status read for the same shape while fixing it** — the lesson of the
first occurrence was that one reader had carried a retry since the day the race
was found and nothing else did.

### 15. The habitual stop press after a ceiling stop starts a new dictation — found 2026-08-25

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

Everything here is working as built. `on_event` toggles, the queue serialises so
utterances cannot race, and both transcripts were delivered. There is no error to
report and none was reported.

What makes it worth an entry is the interaction with the timings above. The user
who reaches the ceiling is by definition the user who was still talking, so a
stop press is the *expected* next input, and it now costs them a spurious
recording whose transcript is pasted wherever they happen to be up to a minute
later. The notice window says the recording stopped, but it appears while the
first transcript is still 44 s from arriving, so on the processor the sequence a
user actually experiences is: cue, notice, silence, an unexpected second
recording, then two pastes.

Whether a press shortly after an automatic stop should be swallowed is an owner
decision and is not proposed here. It is recorded because one run surfaced it
immediately, which suggests most ceiling stops will hit it.

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

### 17. The notice window is 16 px too short for its own copy — found 2026-08-25

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

- 21 commits on `main`, private, `kwp490/speakeasy-granite-rust-mini`.
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
