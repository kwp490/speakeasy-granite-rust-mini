# SpeakEasy Mini UI Guide

Design system reference for the desktop UI: tokens, components, information
architecture, and accessibility rules.

## Principles and terminology

SpeakEasy Mini is calm, explicit, and evidence-based. It says what happened,
what remains recoverable, and what action is safe next. It never describes
detected hardware as qualified, queued input as delivered, or a failed
transcription as anything other than a failure.

**Failure is a first-class state here, not an edge case.** There is one engine
and nothing behind it, so a dictation that does not produce text produces an
explanation instead. Every reason code has a catalog entry naming a condition
the user can act on; a code that falls through to `errorUnknown` is a bug, not a
gap. This is the single largest difference from SpeakEasy, where a failed pass
could quietly hand over a weaker transcript.

**Two vocabulary registers.** Everyday surfaces — the dock and the General,
Audio, Transcription, Output & Privacy, and Transcript log pages — use plain
language: Recording, Working…, Text inserted, Copied to clipboard, Microphone,
Shortcut active. The **Advanced** page, the raw-values disclosure, log event
codes, and this repository's documentation keep the product-contract terms:
`capture`, `utterance`, `final transcript`, `commit on finish`, `local model`.

Two terms stay precise in **both** registers, because truthful disclosure
depends on them: **final** (there is only one transcript, and it is either
delivered or it does not exist) and **unmeasured / unqualified** (never
softened). The three streaming tiers — stable display, mutable, final — are gone
along with the engine that produced them.

Prefer short verbs: Start, Stop, Try, Copy, Retry, Install, Remove, Export, Delete. Destructive actions name their exact scope and require confirmation. Credentials are described only as present, missing, inaccessible, or legacy; values never appear.

All visible and assistive text comes from the locale catalog. Rust returns stable data and reason/action codes, never arbitrary user-facing prose. Model names, licenses, revisions, sources, capabilities, and transcript content are untrusted inert text.

## Design tokens

| Token | Light | Dark | Purpose |
|---|---|---|---|
| `--surface` | `#f5f7f2` | `#121915` | Window background |
| `--surface-raised` | `#ffffff` | `#1b2520` | Cards and grouped controls |
| `--text` | `#17201c` | `#f2f4ee` | Primary text |
| `--text-muted` | `#4c5c52` | `#becabf` | Supporting text |
| `--accent` | `#8d3524` | `#ff9a7f` | Interactive and brand emphasis |
| `--recording` | `#c2442c` | same | **Active capture, and nothing else** |
| `--success` | `#216e3a` | `#8ee0ab` | Verified success/final state |
| `--warning` | `#8a4d00` | `#ffd08a` | Limitation or consent |
| `--danger` | `#a21f2d` | `#ff9aa6` | Destructive/error state |
| `--border` | `#9ca79f` | `#66736b` | Dividers between regions |
| `--border-strong` | `#5d6b62` | `#8b9a90` | Outlines of controls the user operates |
| `--focus` | `#005fcc` | `#8bc4ff` | 3 px focus ring |
| `--space-1..6` | 4, 8, 12, 18, 24, 32 px | same | Spacing scale |
| `--radius` | 6 px | 6 px | Controls/cards |

Red carries exactly one operational meaning. `--accent` is interactive and brand
emphasis; `--recording` marks an active capture and is used in one place;
`--warning` and `--danger` keep their own meanings. A divider and a control
outline are different requirements, so they are different tokens: WCAG 1.4.11
holds a control's outline to 3:1 because that outline is how the control is
identified, while a decorative divider carries no such information.

**The always-on-top windows have their own palette and do not follow the system
theme.** The dock and the pinned log must stay legible over whatever window is
behind them, so `--hud-surface`, `--hud-text`, `--hud-accent` and the rest are
defined once and never flipped by `prefers-color-scheme`. Reusing the shared
`--accent` there was a real defect: it flips to a light coral under a dark system
theme, and a primary button rendered near-white text on it at 1.9:1.

Interface text uses `"Segoe UI Variable", "Segoe UI", system-ui, sans-serif` at 16
px minimum and 1.5 line height. Transcript and result surfaces are the only places
that opt back into a serif, where longer passages read better. Never encode state
by color alone. Icons, when present, accompany text.

## Components

- Buttons have Primary, Secondary, and Destructive variants. Disabled controls retain readable contrast and an adjacent explanation when the reason is not obvious.
- Inputs have a persistent label, optional description, error text linked with `aria-describedby`, and at least a 40 by 40 CSS-pixel target.
- Status badges combine a catalog label with state text. Progress uses a native progress element or equivalent value semantics and includes transferred/total bytes where known.
- Disclosure panels precede consent controls and state data, recipient/destination, purpose, retention, and reset behavior.
- Transcript/result surfaces use text nodes or `<pre>` with wrapping. No HTML interpretation, Markdown rendering, linkification, or model-provided URL action is allowed.
- Tables have a caption and row/column headers. On narrow windows they become labeled definition rows rather than horizontal scroll traps.
- Notifications are inline and non-modal. `role="alert"` is reserved for a new blocking error; routine state uses `aria-live="polite"`.

## Information architecture

Settings has exactly six top-level groups:

| Group | Scope |
|---|---|
| General | Shortcut, the dock, recording sounds, Windows startup, keyboard access |
| Audio | Microphone selection, input level, microphone status. **No capture controls.** |
| Transcription | Language, the engine and model behind a Technical details disclosure, personalization |
| Output & Privacy | Delivery choice, diagnostic log, protected targets |
| Transcript log | Every delivered transcript with Copy, the pin control, and retention |
| Advanced | Runtime and performance, credentials as presence only, maintenance, About, and the Show raw values disclosure |

The sixth group is the transcript log, promoted out of Output & Privacy where it
used to be a section at the bottom. It earned its own page when the large HUD
was removed: that window showed the last transcript with its own Copy button,
and the result view behind it kept the text when a paste was refused. Both are
gone, so this page is the only place a delivered transcript can be read back —
which makes it the thing people come to settings for most often, not a footnote
under privacy.

The groups sit in a left navigation rail, which is a vertical `tablist`: exactly
one page is visible at a time. The active group is represented in local UI state,
announced as the current tab, and reachable with Tab plus **ArrowUp / ArrowDown /
Home / End** — the vertical tab pattern, declared with `aria-orientation`. A
keyboard user can move through the entire window in visual order without a
pointer. No feature creates a seventh top-level group.

### Reads that lose the startup race

Every window's webview loads while Tauri's `setup` is still managing
coordinators, so any `#[tauri::command]` taking a `tauri::State` can be refused
on mount — "state not managed for field `state` on command …". A window does not
have to be visible for this: all four are declared statically and all four run
their React tree from launch.

**A refused read has to recover, and if it cannot it has to say so.** There are
exactly two ways a settings surface is allowed to read a status command:

- **Through `readWithRetry`**, which is bounded at 20 x 250 ms and still rejects,
  so the caller has to decide what to show; or
- **on an interval**, because a poll heals itself on the next tick and the first
  refusal costs nothing.

Five surfaces read this way: General (`hotkey_status`), Advanced
(`diagnostics_status`, `credential_status`), Output & Privacy (`result_status`),
Transcription (`model_hardware`, `personalization_status`, `diagnostics_status`,
`model_catalog`, `gpu_status`) and the shared profile (`profile_status`). Each
carries a line saying the answer did not arrive, in the place the answer would
have been.

**A default may not render as a claim about the system.** This is the half a
retry cannot fix, and it is what made the second occurrence worse than the
first. `hotkey?.registration ?? "pending"` looks like a harmless fallback;
`pending` is a real backend state meaning "registration has not been attempted",
and its copy reads "Shortcut not registered yet". So an unanswered *read* was
rendered as an unregistered *shortcut* — and the remedy the panel implies,
pressing Save hotkey, fixes a problem the user does not have. `undefined` means
the page does not know; only the `unknown` codes say that. The same substitution
was made on Output & Privacy, where `?? "empty"` claimed "No result".

**And the page may not hold a value it has not read.** General's binding field
was initialised to `Ctrl+Alt+L` — SpeakEasy's shortcut, inherited by the fork and
never rebranded — which turned the lost read from wrong into destructive: Save
would have rebound this app's working `Ctrl+Alt+P` to the other product's
shortcut, on a machine where both are installed side by side. The field starts
empty and Save is disabled until the status has actually been read.

None of this is enforced by naming a command. The previous check named
`personalization_status` in one file and was green on the day the same defect was
found in another file with another command. The scaffold suite now derives the
hazard from the Rust signatures and scans every `useEffect` in the tree — see
"Automated checks".

### The active provider is the device, never the pack

Settings → Transcription discloses what dictation runs on. It reads the **device
the worker is on**, not the provider of the selected pack: there is one Granite
GGUF and a graphics-card worker offloads that same file, so the pack reads `cpu`
on a machine holding the card — and the disclosure was rendering the pack. The
four values are `cpu`, `cuda`, `cuda_unverified` (a graphics-card engine whose
context could not be confirmed — neither of the other two would be true) and
`unknown` (a worker that never answered its handshake).

**The reason is its own sentence, and it never asserts a device** (2026-08-21).
It used to hang off the device line after an em-dash, and on a machine with a
graphics-card worker staged the page read, verbatim: `Dictation runs on:
Graphics card (GPU) — this computer's graphics card is supported, but the
graphics-card model is not installed, so the processor model is being used.` The
device was right and the reason code was right; the *sentence* was false, because
a reason about the pack and a device are two facts that disagree on exactly that
machine — which `ARCHITECTURE.md` had predicted and nobody had looked at the
rendering of. Both halves were fixed, because either alone leaves the trap:
every `engineReasons` string now describes what the **installation includes**,
and the page renders it in its own element rather than joining it to the device.
Dropping the reason instead was rejected — it is the load-bearing half on an
ordinary machine, where "running on the processor" reads identically whether
there is no graphics card or a good one whose model was never installed.

**The same rule reaches Advanced, and did not until 2026-08-28.** That page's
`PROVIDER` field was filled from the selected pack's provider capability, which
is `cpu` on every machine for the reason above — so it read `Processor (CPU)` on
a machine whose worker held 2,365 MiB of VRAM and had just transcribed 24 s of
speech in 1,424 ms, and the owner reasonably concluded dictation had fallen back.
It reads the device now. The fix that landed on Transcription in 2026-08-21 was
scoped to the page somebody was looking at; a rule stated for one surface is not
a rule until every surface that answers the same question obeys it. The pack is
still disclosed on Advanced twice over, by `ENGINE` (its id) and `WORKER` (the
reason it was chosen), so nothing was lost.

**`cpu_gpu_pack_not_installed` is the healthy graphics-card state, and its copy
now says so.** Selection reaches that code only when the machine prefers CUDA
*and* a CUDA-capable worker is installed *and* no CUDA pack is — and no CUDA pack
will ever exist. Its sentence read "this installation includes only the processor
model", directly beneath `Dictation runs on: Graphics card (GPU)`. Rewritten to
"One speech model serves both devices, so it is named for the processor even when
the graphics card is running it." The reason *code* is unchanged: it is a wire
value in `granite_warm` and in `docs/RUNBOOK.md` under "Diagnostics".

**No qualification sentence** (removed 2026-08-28). A line beneath the device read
`gpu.qualified ? "…has passed its local execution check." : "…has not passed its
local execution check yet."`, and only the negative half was reachable:
`GpuQualificationCoordinator::record`, the sole promotion from admissible to
proven, was deleted on 2026-08-21 with a note saying it comes back with the CUDA
worker. The worker shipped on 2026-08-26 and it did not come back. So every
graphics-card user was told the engine had not passed a check, beside a device
line saying it was running on the card, with a "Re-test graphics-card engine"
button implying a remedy that could not reach it. Removed rather than reworded,
because the device line and the provider-integrity line already answer the same
question from evidence that *is* reachable — NVML on the worker's own pid. The
button stays: `gpu_retest` invalidates the engine and re-warms it, so its effect
lands on both lines above. Restoring the promotion needs an `ExecutionEvidence`
with a true `inference_sample_count`, which nothing at warm time has; that is an
open gap in `docs/handoff/CURRENT.md`, not a thing to fake.

Beneath them, and **only when it says something**, sits the provider-integrity
line. Three of the five states have copy: `gpu_install_not_operational`, the
actionable fault, which carries what to do about it; `running_beyond_record`,
which is not a fault and says so; and `gpu_record_unconfirmed`, added 2026-08-21,
for a graphics-card record this run could not check. `ok` and `unrecorded` have
no copy at all — a line that appears on every launch to confirm nothing is wrong
is a line people stop reading, and the requirement is to never *hide* the
provider, not to narrate it. Whether an outcome is a fault is decided in Rust and
sent as a boolean, so the page cannot classify it differently from the enum that
defines it.

**A driver that will not answer is not a fault.** `gpu_record_unconfirmed` exists
because the fault above used to swallow it, and the fault's copy says dictation
"is running on the processor instead" — a claim about a device, made on evidence
that establishes no device at all. `device=` had this right the whole time and
reported `cuda_unverified`; it was the comparison one layer up that collapsed the
three answers into two. Folding it into `ok` was rejected for the mirror-image
reason: that would claim an agreement nothing verified.

**All four device values have now been produced on hardware** (2026-08-21, RTX
4070 Laptop GPU): `cuda` against a real driver, `cuda_unverified` against a
staged probe that refuses to answer, and `cpu` both from a processor build and
from a CUDA build NVML says is not on a device. Two of them are staged
deliberately, and that is the point — a working card will always answer and will
always take the worker, so "the driver would not answer" and "the driver said no"
are exactly the cases a real machine cannot be asked to produce.

**The `gpu_install_not_operational` copy names the cheap remedy, not the
expensive one.** It said "reinstall SpeakEasy Mini" until 2026-08-21, because a
reinstall was the only thing that re-ran the three-gate proof; the bootstrapper's
`--verify-provider` verb now runs the identical check against an installed build
in seconds. The wording leads with the two things a user can act on without a
command line — the graphics driver, and something else holding the card — and
names the verb after them. Copy that recommends the costliest available fix is
not neutral: it is what most people will do.

Settings never starts, stops or cancels a dictation. There is one dictation
controller — the dock and the global shortcut, which share a single
implementation — and no second path that could deliver differently.

## Fresh-profile flow

There isn't one. A profile arrives already set up, because the installer is the
only way in and it does the work: hardware check, download, verification, engine
smoke test, the shortcut, the vocabulary and the retention question, all before
the app launches for the first time — and it is the installer that launches it.

This replaced a seven-step in-app stepper that ran on first launch and knew
nothing about what the installer had done. Two surfaces that both claimed to own
setup could disagree about what was installed, and the one that ran second had
no way to tell whether the first had succeeded.

What survives in the app is **repair, not setup**: when a model file goes
missing or a verification fails, the Transcription page reports what is wrong
and offers to re-download. That is a different question from "has this user been
onboarded", and it is answered from the filesystem rather than from a stored
flag.

Reset shows an exact preview of app-owned settings, history, caches, and models
affected. It never includes custom paths or credentials unless separately named,
and never performs deletion while dictation is active.

## Setup installer

A **second UI surface**, and the only one that is not React: `apps/bootstrapper`
draws a native Win32 window through `winsafe`. It is what a user sees before the
app exists, so it cannot depend on WebView2 — the thing that provisions WebView2
is the thing being replaced, and repair mode runs on already-broken machines.
The principles above still bind it; only the toolkit differs.

**Copy lives in `apps/bootstrapper/src/catalog.rs`**, which is the wizard's half
of the "all visible text comes from the catalog" rule. Logic must not build
user-facing prose inline.

**One file.** Setup is a single executable with the files it installs appended
to it past the end of the PE image, which Windows' loader ignores. There is no
directory to keep beside it and nothing to unzip — a user downloads
`SpeakEasyMiniSetup.exe` and runs it. Every appended file carries a SHA-256 that
is checked before it is written, and that is not about tampering: an installer
truncated by a dropped download **still runs**, because the missing part is not
part of the program, and would otherwise install whatever fragment parsed.

Eight steps, in order: can this computer run it?; where should it run?; download
the models; install; pick your shortcut; add your words; what should it keep?;
does dictation actually work? Back and Next; the last step's Next reads
**Finish**. Back is disabled on the first step rather than hidden, because a
button that disappears moves the ones beside it and the target under the cursor
changes.

**Every page is a question, one key line, and at most two short sentences.**
The heading asks; a single **key line** directly beneath the step counter carries
the one thing that must survive being the only thing read; the body adds context
and nothing else. Rewritten to that shape on 2026-08-20, from pages of correct,
careful four-sentence paragraphs — which is the failure it fixes: nobody reads an
installer, so an obligation discharged in the fourth sentence of the third
paragraph is discharged on paper and not in fact. `catalog::Step` carries the
three parts separately so a page cannot quietly grow back into prose.

**The key line is coloured, and colour is never the only signal.**
`catalog::Tone` marks each line as ordinary, accent, warning or good, and
`wizard.rs` maps that to ink. The tone is a *copy* attribute and lives with the
copy, because which sentence a reader must not skip is a decision about the
writing. Every tone is also carried by the words — a reader who cannot see the
colour loses emphasis, never information.

**Both wizard windows set their own type, and there are exactly two sizes.**
Body text is four thirds of the system UI font and the step heading is five
thirds of it, semibold. Everything that carries words takes one of the two —
the step counter, the key line, the body, the findings, the status line, both
radio groups, both check boxes, the vocabulary box, every button, and the
uninstall page's list and its "This cannot be undone." A page whose prose is a
third larger than its own buttons reads as a page that forgot half of itself.

`apps/bootstrapper/src/typeface.rs` owns this, and the sizes are a *ratio* of
what `SPI_GETNONCLIENTMETRICS` reports rather than absolute points — a reader
who has raised Windows' own text size is exactly the person the ratio is for,
and forcing "12pt" would shrink the wizard for them. Every layout height in
`wizard.rs` and `uninstall_page.rs` grew with it; the windows are 620x606 and
560x494 logical.

> **Superseded.** Until 2026-08-26 this surface drew everything at `winsafe`'s
> one process-wide font — Segoe UI 9pt, what Windows uses for menu bars — on a
> measure of about 105 characters, and **bold was recorded here as
> unavailable**: emphasising a label's font means `WM_SETFONT`, `winsafe` sends
> messages only through an `unsafe` call, and the workspace sets
> `unsafe_code = "forbid"`. Nothing was mis-scaled; the type was correctly sized
> for a menu bar and too small to read comfortably, which is what the owner
> reported. `apps/bootstrapper/Cargo.toml` now declares its own `deny` in place
> of the inherited `forbid` for the two calls that fix it, so weight is
> available on this surface as well — the heading uses it. Colour still goes
> through `WM_CTLCOLORSTATIC` and is still never the only signal.

**Finish starts the app**, and says so if it could not. Setup that ends by
closing its own window leaves a user who watched every step succeed looking at
an empty desktop.

**The answers reach the app through one-shot seed files** under
`%APPDATA%\ai.speakeasy.mini\config\`, which the app reads and deletes on first
launch. Deleting them is the contract: a seed is a starting value, never a
policy, so a setting the user changes afterwards must never revert. They are
written when the user leaves the last question rather than at Finish, because
the engine check runs after that and takes seconds — someone who closes the
window while it works has still answered every question.

The single exception is the **installed configuration**, which is a record
rather than a seed and is not deleted. It is what lets the app tell "running on
the processor because that is what was installed", which is normal, from
"running on the processor because the graphics-card engine will not load", which
is a fault. Without it those are the same silent state; it appears in the
`granite_warm` log line as `installed=`.

Six obligations this surface carries more sharply than the app does, because it
is where the claims are easiest to overstate:

- **Detected is not qualified.** Setup may say a graphics card meets the
  requirements. It may not say the card works until the execution check has run,
  and then it reports what that check found.
- **Installed is not working.** The engine check exists because everything
  before it only put files on disk. Its copy says so — and the check is a real
  transcription of a bundled clip compared word for word against known text,
  because a speech model whose audio projector failed to attach writes fluent
  text without listening to anything. "It produced a transcript" is not
  evidence; matching content is.
- **Keywords do not change what is heard.** They correct spelling and spacing in
  the finished transcript. Copy must not imply a misheard word becomes correctly
  heard — `catalog.ts`'s `hotwordLimitation` records why.

  **Do not reach for prompt bias to close this gap; it was tried.** Appending the
  terms to Granite's transcription instruction was built, measured on a recorded
  55 s clip, and reverted on 2026-08-27. It genuinely worked — 16 of 16 term
  occurrences against 9 without it — and it returned the entire dictation as one
  unpunctuated run-on. The cause is not the keywords: an appended sentence about
  the weather cost the punctuation just as completely. `CLAUDE.md` carries the
  table. The obligation this bullet has always stated is what the measurement
  then confirmed, so it is stronger now rather than weaker.
- **A collected answer that does not arrive is a defect, not a detail.** Every
  answer setup takes has to be visible in the app afterwards, and the vocabulary
  page failed this from the fork until 2026-08-20 — see the vocabulary rules
  below. An answer applied through a `let _ =` is an answer nobody can prove
  arrived.
- **This build is never signed.** SmartScreen may warn, that will not change, and
  setup says so plainly rather than implying a missing prerequisite.
- **Removed means removed, and kept means kept.** An uninstall reports the
  things it actually deleted, names everything it spared, and says "SpeakEasy
  Mini does not appear to be installed" rather than announcing a removal when it
  found nothing to remove. This is not a style point: an uninstall aimed at the
  wrong directory reported "SpeakEasy has been removed. Removed: program files"
  and exited zero, having deleted nothing at all (2026-08-15). A file it could
  not remove is named, with its path, separately from an outright failure.
- **An uninstall leaves nothing, and asks before it does** (owner decision,
  2026-08-21). It removes the program directory whole and the profile with it —
  settings, transcript history, the model weights, recovery backups, logs — and
  removes the directories themselves rather than leaving an empty tree that reads
  as clean. The default was keep-everything, inherited from NSIS's `/SD IDYES`,
  and it meant a user who asked the product to go was left with 2.14 GB of
  weights and a settings tree, told "SpeakEasy Mini has been removed".
  `--keep-user-data` is the opt-out and is a **testing** affordance, not a user
  one: an install/uninstall cycle would otherwise re-download the weights every
  time, and both proof scripts pass it.

  The interactive path — which is what the Add/Remove Programs entry invokes —
  **asks once, on one page, with the whole scope on it**. A check box per
  removable item, every box checked, rendered from `Removable::ALL` so a sixth
  thing the uninstaller learns to remove cannot become a thing the page forgets
  to offer. Files in `proof/` that setup did not put there are named separately
  and last, because they are the part a user cannot predict: today that is
  `Enable-GraniteCuda.ps1`'s staged CUDA libraries, and until 2026-08-21 they
  survived every uninstall silently. A silent run cannot ask and so proceeds —
  `/S` is a caller asserting it already knows.

  **The page is the confirmation, and its Remove button is focused** (owner
  decision, 2026-08-21). Nothing follows it: a second dialog re-asking what the
  page just asked is the sequential-prompts-answered-blind shape the page exists
  to replace, and it teaches people that the way through an uninstaller is to
  press Enter twice. `BS::DEFPUSHBUTTON` alone does not focus a button — measured
  here, with the explicit `SetFocus` removed the focus lands on the heading
  static — so the focus is forced on the first paint and only then. `Cancel`, the
  close box, and a window that could not be drawn all mean remove nothing,
  because a page nobody saw is not consent.

  **Only the models entry names a size, and the size is measured.** It is the one
  item whose cost is large and invisible; four kilobyte figures beside it would
  bury the one worth reading. `uninstall::measure` walks the same path table the
  deletion uses, so the figure cannot describe one set of files while another is
  removed. The label this descends from said "about 2.3 GB" for a downloaded
  runtime this fork never had, which is what a written-down size eventually
  becomes. Measured on this machine at 250%, the page is 560x494 logical and
  every control fits its box, the models row reading
  `Downloaded speech models (5.6 GB)` — it was 480x398 before the type grew on
  2026-08-26, and the figure is whatever this machine happens to hold.
- **The two-minute ceiling ends the recording out loud, and says so.** Reaching
  it is not a failure and never costs the user their words. Three things happen,
  in this order (owner decision, 2026-08-25): the stop cue sounds, a notice
  window appears, and the transcript is delivered normally.

  The **cue sounds whether or not there is a transcript**. It answers "did the
  microphone stop?", which is a different question from "did I get my text", and
  the ceiling is the one ending the user did not ask for. It used to sound only
  on the delivering branch, so a capture that ended in failure ended in silence
  and the user kept talking to a microphone that had stopped listening.

  **Confirmed by ear on 2026-08-25**, on a real ceiling stop at 120,183 ms in an
  installed release build — the first time anyone had heard it rather than
  asserted it. The notice window was seen on the same run, and seeing it found a
  defect: **its content needs 188 CSS px against a declared 172**, so the dismiss
  button sat 16 px below the fold behind a scrollbar. The copy fits and reads;
  the only control did not. Not DPI-dependent — the wrap is identical at every
  scale — so it had clipped on every machine since the window shipped. **Raised
  to 192 on 2026-08-26**, `minHeight` with it, measured on the running window at
  zero overflow with the button 2 px clear of the fold; the copy is unchanged,
  which was an owner decision. The estimate that justified 192 predicted 4 px of
  clearance and the running window gave 2 — both readings were taken the same
  way, and the earlier one was optimistic by exactly that difference because it
  suppressed the scrollbar. The real box needs 190.

  What that run also showed is that the cue arrives a long way ahead of the
  text. On the processor, inference on a full-length dictation is ~44 s, so the
  sequence a user experiences is cue, notice, then three quarters of a minute of
  nothing before the transcript lands. A stop press in that gap used to start a
  new dictation, which then queued behind the first — observed 490 ms after the
  ceiling fired, which is where a habitual second press falls.

  **One dictation at a time** since 2026-08-26 (owner decision). A press between
  recording ending and the transcript landing is **refused, not queued**: the
  user pressing it is ending a recording that has already ended. The dock's
  `can_start` had always refused it and the global shortcut had not, so the same
  key was declined by one controller and accepted by the other — on a product
  whose single-controller rule exists to stop exactly that. The guard is in
  `start_dictation`, the one function both paths converge on, and the refusal is
  logged (`dictation_start result=dictation_still_finishing`) with its own catalog
  copy, because "the shortcut did nothing" is otherwise indistinguishable from a
  broken shortcut.

  **And the same is true of every other reason a dictation is refused**, since
  2026-08-29. The dock's "Setup needed" reason and the shortcut's refusal are one
  function — `dictation_blocker` — consumed by `setup_requirement` and by
  `start_dictation`. It answers `model_missing`, `model_verifying`,
  `granite_worker_missing`, `memory_below_granite_floor`, `granite_quarantined`
  or `microphone_missing`, and all six now refuse **before** any audio is
  captured, on both controllers. Stating the rule twice failed again in the batch
  that fixed the last instance: the three terminal engine states were added to
  the dock's copy alone, so a machine below the 8 GiB floor had its Start button
  disabled while its hotkey recorded two minutes and reported afterwards that the
  engine could not start. Every code the function returns has catalog copy,
  because a refused `dictation_start` is rendered through
  `messages.errors[code]`; a scaffold test derives the list from the function's
  own source rather than from a list somebody maintains.

  A coordinator that is not managed yet answers nothing and refuses nothing. The
  dock's poll and the shortcut both reach this before `setup` has finished, and a
  guard that cannot see the machine must not be able to suppress a dictation the
  user asked for.

  The **notice is its own window** (`notice`, 360x192 logical, always on top,
  `focus: false`, placed beside the dock). It is not a dock glyph because the
  dock is 62 px wide and cannot hold a sentence; it is not a Windows toast
  because that route needs an AUMID from an installed Start Menu shortcut and
  otherwise displays nothing while reporting success. It says what happened,
  that the transcript is safe, and what to do next — in that order, because the
  first thing a user asks when a recording stops by itself is whether they lost
  anything. It dismisses itself after 15 seconds and has one button.

  **Never call `set_focus` on it.** It is shown while `deliver_final_text` is
  inspecting the foreground window to choose a paste target, which is the exact
  hazard that has hijacked three dictations.
- **Imperfect audio is delivered with a warning, never discarded.** Of the six
  conditions a finished capture can report, only "no frames at all"
  (`capture_empty`) means there is nothing to transcribe. The other five — a
  dropped block, a processing overrun, and the three buffer limits — describe
  audio that exists and will transcribe, so they annotate the result instead of
  replacing it. Until 2026-08-25 every one of them was an error that threw the
  recording away, and because the buffer's byte limit bound 3.5 s inside the
  two-minute ceiling, that destroyed **every** maximum-length dictation while
  short ones worked perfectly. All five now carry copy that says the transcript
  was delivered and what may be missing from it; four of them had none and
  rendered as "The operation stopped safely".
- **A progress bar that is not moving must say why it is not moving.** The
  download step has three phases and only one of them advances a bar:
  transferring, re-checking a file that was already downloaded, and unpacking.
  The last two are long — digesting a multi-gigabyte model measured tens of
  seconds — and they are indistinguishable from a hang unless the copy names
  them. Reporting "Downloading — 0 MB of 4.4 GB transferred" through a
  verification pass was the concrete failure (2026-08-17):
  false, and the exact shape of a stalled download. `catalog::Phase` carries the
  distinction so it cannot be reduced back to a boolean.

Two structural rules follow from the same step. **Nothing to download is its own
message**, not a bar that fills instantly — those are different claims, and the
true one is that the files are present and their digests still match. And **a
bar means one thing**: the transfer bar is a second control rather than the step
indicator retargeted, because one bar cannot honestly mean "step 3 of 7" and
"41% of 453 MB". Controls are all created up front and hidden when they do not
apply, since `winsafe` panics if a control is created after its parent window.

A step whose controls are not yet built says exactly that. An empty step is
indistinguishable from a step whose controls failed to appear. **Every step is
built as of 2026-08-19**, so nothing reaches that message today; the rule stays
because the next step added will pass through the same state.

Four of the eight ask something, and each one carries a rule the placeholder
version could not:

- **An option that cannot be installed is shown and disabled, with the reason.**
  Hiding the option would read as setup not having looked at the card; enabling
  it where it cannot be installed would be a control that silently installs
  something else. The step says which half is missing, and the answer is derived
  from the manifest.

  **A CUDA worker has been published since 2026-08-26**, so a machine whose card
  clears the requirements is now offered it, and it is the pre-selected answer.
  Before that date this rule read "a machine whose graphics card clears the
  requirements is still offered only the processor", which was true of every
  machine for as long as no worker existed. What is unchanged is the shape: the
  option's availability is a fact about the release plus the card, read from the
  catalog, never a preference.

  **The disabling was specified here and not implemented until 2026-08-20**, and
  the cost was the whole class of defect this page exists to prevent: the option
  was selectable, selecting it wrote `installed=cuda`, and the app then ran on
  the processor and reported the installation it had been told about. The rule
  is now pinned against source by `apps/desktop/tests/scaffold.test.mjs` and
  against the running window by `Test-SetupWizard.ps1`, which asserts the
  graphics-card option is present **and** disabled before it clicks Next.

  Three refusals, three sentences, because they are three different things to
  do: the release has no graphics-card engine; it has one and this installation
  does not carry it; or it is here and the libraries it loads are not — that
  last one names the files, because a CUDA build whose imports Windows cannot
  resolve does not start, and the error for that names nothing anyone can act on.

  **The refusal is asked of the release, not of the disk** (2026-08-26). This
  page is shown before the payload is extracted, so a machine on its first
  install has no `proof/granite-worker.exe` to look at — an "is it here" check
  answers *not installed* for every fresh machine however the manifest is
  pinned, while answering *yes* on a development machine with a worker staged by
  hand. The wizard asks `gpu_configuration_is_installable`, which touches no
  disk; `inspect_gpu_payload` keeps the "installed here" answer for the readers
  that come after the files exist.
- **The answer decides what is downloaded.** Choosing the graphics card is what
  makes setup fetch the CUDA engine and the two libraries it loads; choosing the
  processor fetches the weights alone. A question whose answer nothing reads is
  the same defect as an option that installs something else — the page would be
  offering a choice and quietly making its own. The graphics-card button is
  pre-selected when the card qualifies, so the ordinary path is one Next press.

  This was not true until 2026-08-26, and could not have been noticed before:
  the plan read the machine's capability rather than the answer, which is the
  same value for as long as the option is disabled. Pinning the worker is what
  separates them.
- **The installed configuration is recorded from proof, on the last page.** Not
  from this page's radio button, and not with the seeds. It takes a published
  and complete payload, a worker that reported a CUDA backend at its startup
  handshake, and NVML placing that worker's own process on a device. A check
  that never ran records nothing, which the app reads as `unrecorded` — a third
  state, deliberately, because guessing `cpu` would be a claim about a
  configuration nobody verified.
- **A shortcut is verified by taking it, not by looking at it.** Windows will
  not say who owns a global hotkey, so setup registers the chosen combination
  and releases it again; if the registration fails, Next is disabled and the
  copy says another program holds it and does not guess which. Three named
  alternatives are on screen, so this gate can never trap anyone. Setup must
  release the shortcut immediately — holding it would make setup the owner, and
  the app's own registration would then fail against a conflict setup created.
- **The vocabulary box takes a comma-separated list, and says back what it
  read.** Commas since 2026-08-20; one word per line before that, which is more
  typing and one more convention to remember. Newlines still separate, because a
  user who types them means the same thing. The page reports "3 words will be
  added: …" from the *same* parse that writes the seed file, so the count can
  never describe a list the file disagrees with — and echoing the words is the
  only form in which a missing comma is visible before it is installed.

  **The box arrives filled**, since 2026-08-27, from
  `catalog::DEFAULT_VOCABULARY` — the common tools whose names a recogniser
  reliably splits or mis-cases. Nobody reads an installer, so a page that opens
  empty is a page most people click past, and the feature then reaches only
  users who already knew they wanted it. Starting filled inverts that at no cost
  to control: it is an ordinary editable box, and clearing it installs nothing.

  **A profile that already has words is offered its own**, never the default,
  and this is a guard rather than a nicety. `add_protected_terms` *replaces*
  every entry setup owns rather than merging — deliberately, because the merge
  it replaced left stale entries that collided with the new list and cost the
  user every word. The price of that choice is that any non-empty seed is
  authoritative, so a reinstall offering a canned list over a curated one would
  discard the curation silently. Handing the existing words back makes pressing
  Next idempotent instead. `seed::vocabulary_to_offer` is the one place the rule
  lives, and it reads the profile without creating one.

  **A silent install seeds the default too, and only onto an empty profile.**
  `--install` asks nothing, so it records no other answer — every other seed
  would assert a choice nobody made, and the app has its own defaults for those.
  The vocabulary is the exception because there is no app-side default for it at
  all: without this a scripted deployment installs a product whose word list is
  empty forever. It reports `vocabulary=seeded` or `vocabulary=kept` on the
  install line, so which of the two happened is never a guess.

  **And it clears `install-provider.txt`**, since 2026-08-27. That file is not a
  seed and the app never consumes it, so a `cuda` written by an earlier *wizard*
  install outlives a silent reinstall and goes on describing a configuration
  that was just replaced. Measured: a silent install over a graphics-card one
  left the processor worker on disk beside `installed=cuda`, and the app
  correctly raised `gpu_install_not_operational` — a real fault banner on a
  machine whose only fault was the stale claim. Clearing rather than writing
  `cpu` follows the rule the record already carries: a check that never ran
  writes nothing, and absence reads as `unrecorded`. The install line says
  `provider=unrecorded` or `provider=stale`.

  Three rules follow. The default is **a starting value and never a policy** —
  the same contract every seed here carries, so a word removed in Settings
  afterwards stays removed. The **box is the example**: the key line no longer
  carries a specimen list, because a made-up example sitting above a box full of
  real words reads as the instruction for an empty one. And the page must
  **count what it arrived holding** without anyone touching the box —
  `WM_SETTEXT` does not raise `EN_CHANGE` on a multi-line edit, so a page that
  recomputed only on change would say "No words yet" over a full box.
  `Test-SetupWizard.ps1` asserts the arrival count before it overwrites
  anything, because every later assertion passes against a box the script filled
  itself.

  **A compound term also gets a spaced companion**, since 2026-08-27:
  `LogicMonitor` produces a second, visible entry rewriting `Logic Monitor` to
  it. That is how a recogniser writes a compound product name, and the identity
  rule cannot match it — the term is one needle and a space is not in it.
  Measured on a recorded clip, an unbiased pass wrote `logic monitor` twice and
  `Pager Duty` twice while the shipped dictionary correctly fixed `Jira` to
  `JIRA` on the same transcript; the companions take that transcript from 10 of
  16 term occurrences to 14, with all eight sentence stops intact.

  Two rules about them. **They are visible entries, not matcher behaviour** — a
  user who finds "I need service now" rewritten to "ServiceNow" has to be able
  to see the rule that did it and delete it, and a quietly fuzzy matcher offers
  nowhere to look. And **a companion that collides with a term the user actually
  typed is dropped, never added**: two entries with one source are a
  `ConflictingRule`, which rejects the whole batch, which is precisely the defect
  the next paragraph exists for. Typing both `ServiceNow` and `Service Now` is an
  ordinary thing to do.

  **The words must actually arrive**, and for months they did not. Setup writes
  them to a one-shot seed; the app applies them as dictionary entries named
  `installer-0`, `installer-1`, … by position. An uninstall run with
  `--keep-user-data` keeps `personalization.json` — which is what the proof
  scripts pass — so a second install merged a new list over
  the old ids — and where a stale entry it did not displace held a word the new
  list also held, the two were a `ConflictingRule` to the dictionary validator,
  which rejects **the whole batch**. The user got none of their words, kept the
  previous install's, and nothing reported it: the apply site swallowed the error
  and nothing logged. Two entries differing only in case ("Ken, ken") did the
  same thing on a first install. The fix is three-part and all three parts are
  load-bearing: the parse de-duplicates case-insensitively, the repository
  *replaces* every entry setup owns instead of merging
  (`replace_user_entry_terms`), and the outcome is written to the log as
  `installer_vocabulary count= result=`.

  **And a fourth part, on the other side of the IPC boundary**, found the same
  day with the words already correct on disk: the Transcription page read
  `personalization_status` **once, on mount, with no rejection handler**. Every
  window's webview loads while `setup` is still managing coordinators, so that
  read can be refused with "state not managed for field `state` on command …" —
  and the page then showed an empty dictionary list for the life of the process.
  An empty list is not a blank page anyone reports; it says "you have no
  protected terms". `useProfile.ts` had carried a retry for this race since it
  was first observed and nothing else did. Status reads that can lose that race
  now go through `readWithRetry`, and one that never succeeds says so beside the
  list rather than rendering an empty one.

  **That sweep stopped at one file, and the same defect was then found in a
  second** — `hotkey_status`, on 2026-08-25, reporting a registered and working
  shortcut as "Shortcut not registered yet" for the life of the process. It is
  the worse of the two: an empty list is a passive wrong answer, and this names a
  working feature as broken in the one panel someone opens *because* their
  shortcut appears not to work. All five settings surfaces were converted on
  2026-08-26 and the rule is now enforced by derivation rather than by name — see
  "Reads that lose the startup race" below.
- **The retention default is off, and stated as a promise rather than a
  checkbox.** Unticked means transcripts are never written to disk at all, which
  is a stronger claim than deleting them on exit and is worth making in words: a
  delete-on-exit is a promise a crash breaks.

**The last step runs the engine and checks what it heard.** Setup transcribes a
recording compiled into the installer and compares the result against the words
that recording speaks. This is the only step that proves the speech model can
hear: a model whose audio projector failed to attach does not error, it answers
the prompt from the instruction alone and writes fluent invented text, so a
transcript on its own is evidence of nothing.

It compares **words**, case-folded and stripped of punctuation, not the
transcript verbatim — and that is measured rather than cautious. On 2026-08-19
the clip's "…the lazy dog, and Monday begins at dawn." came back as "…the lazy
dog. And Monday begins at dawn.": every word right, a period for a comma. An
exact comparison would have refused a working install. The verbatim pin belongs
in `granite_worker_smoke.rs`, where a change is a finding for a developer rather
than a blocked user.

**A failed check does not block the install** (owner decision, 2026-08-19). The
step offers **Retry**, and both Continue and Cancel stay available. The copy
names a likely cause and an action, and states what continuing costs — a skipped
check must never read as a passed one. The two failures get different advice
because they have different causes: a mismatch means the engine ran and cannot
hear, which implicates the model files; an engine that never ran does not.

**The last step also says which provider it proved**, because that is the moment
the record is written and the claim being made about someone's machine should be
visible to them. "Dictation works, on the processor" is not an apology — a
processor installation running on the processor is complete and working exactly
as installed — and the graphics-card line is never shown for a run that happened
on the processor.

**The wizard must never hold the foreground while the engine check runs.**
Anything SpeakEasy Mini puts in the foreground becomes the delivery target, and the
failure is silent — delivery refuses with `target_inspect_refused` and falls back
to the clipboard, which reads as a bug in some other subsystem.

Measure the running window here too, with
`scripts/Measure-NativeWindow.ps1`. The declared size is a request, and a
DPI-unaware measurement of it is worse than none: Windows virtualizes every
coordinate such a process reads, so a window that is exactly right and one that
never scaled report the same figures. That is what happened here — the client
area was recorded as its declared *logical* size under three different DPI
manifest declarations and read as evidence of a broken window. Re-measured
2026-08-17 from a per-monitor-aware probe it is 1550x1150 physical at 240 dpi,
which is correct. The wizard is deliberately **system-DPI aware, not
per-monitor**: `winsafe` holds one font for the whole process, so per-monitor
sizing would shrink the controls under text that stayed put. The manifest's own
comment carries the measurements and what would have to change.

`-Fit` is the other half, and it is the one that matters as steps gain copy: it
reports whether a control's text actually fits its box. That has a DPI-dependent
answer, because a font cell is not linear in DPI — Segoe UI 9pt is 15 px at 96
and 41 px at 240, where linear predicts 37.5 — so a box in fixed logical units
holds a different number of lines at each scale. A step whose copy fits at 100%
can clip at 250%.

## Main window and focus

The settings/main window is a normal focusable document. Initial focus goes to the page heading; validation moves focus only after a submitted action and announces the error. Opening a dialog traps focus until close and returns focus to its opener. Escape closes only the top transient surface and never silently cancels or deletes data.

The settings window is hidden at startup and opened
deliberately from the dock's right-click menu or the tray. Closing it **hides**
it — it never destroys
the window, never quits the app, and never disturbs an active dictation.

**No window of SpeakEasy Mini's own may hold the foreground it did not ask for.**
Delivery chooses its target by inspecting the foreground window, so a SpeakEasy Mini
window sitting there quietly redirects the user's dictation into SpeakEasy Mini. Hidden
is not sufficient: `main` is `visible: false` *and* `focus: false`, because
`focus` defaults to true and a hidden-but-focused window held the foreground
indefinitely — the first dictation after launch then landed on the clipboard
instead of in the user's document. The same rule covers processes: the app is
built `windows_subsystem = "windows"` and the workers are spawned
`CREATE_NO_WINDOW`, because a console window is a visible foreground window like
any other. Focus is taken only when the user asks for it — `show_settings_window`
calls `show()` and `set_focus()` together.

**The dock.** The app's only HUD and its permanent furniture. A narrow strip
that clings to a screen edge, always-on-top, moved by dragging it, with no
taskbar button. Six rows in a fixed order, none of them conditional: the chrome
row (settings and close), the vertical wordmark (which is also this undecorated
window's whole titlebar), the level meter, the engine indicator, the status row,
and the action row.

**The action row holds one button, and it is present in every state**
(2026-08-28). It reads `Ready`, `Stop`, or the working dots, and pressing it
starts or ends the dictation. It used to render only while listening, which left
the dock unable to *begin* one at all: the surface whose whole promise is staying
reachable while the user works elsewhere offered a control that appeared only
once they had already started. The resting label is keyed off `idle` and nothing
else, because `Ready` while the model loads is the defect fixed one row up in
1.7.0 and a permanent button is a louder place to repeat it.

`Transcribing` is not one of the labels, and that was measured rather than
argued: the label has 36px between the row's inset and the button's own padding,
and the word needs 60.2px at the button's 0.68rem and still 47.8px at 0.54rem,
below the smallest type anywhere in the app. The dots carry that state and the
full `Transcribing...` is in the accessible name and the tooltip.

**The status row carries the elapsed time during a run and the outcome mark
after one.** It was the clock row; the outcome was in the action row, which a
permanently present button leaves no room for at a 52px card. Neither fact is
ever wanted at the same moment as the other, so they share one 16px row rather
than the card growing again to find a seventh.

The dock's account of what happened after the user let go of the key is the
status row and the action row together. That last state matters more here than it did in
SpeakEasy: with no fallback engine, "ended" is sometimes "failed", and the dock
is where the user finds out. It shows the failure; **Settings → Transcription**
carries the reason and the fix, in a panel that appears only when the last
dictation failed.

Two surfaces, not three. A Windows toast was specified here and is deliberately
not built: the WinRT route needs an AppUserModelID from an installed Start Menu
shortcut, so it succeeds and displays nothing in a dev run or any install
without one — a silent success, which is the exact failure shape this project
spends most of its comments avoiding. The cost is honestly stated: a user
looking away from the dock sees only a small failed state, and has to open
settings to learn why. That is worse discoverability and not a false claim.

Stop is on the dock despite dictation being hotkey-driven; why, and how it is
painted, are in "The dock's geometry" below rather than stated twice.

Right-click opens the native menu (Settings, Close). There is deliberately no
left-click equivalent, so a drag in progress can never be mistaken for either.

**The pinned transcript log.** A second always-on-top window, shown only when
the user pins it from settings, holding the same list the Transcript log page
shows — the same component, not a second implementation, so the two cannot
disagree about what was said. Undecorated: its header row is the titlebar, the
drag handle, and the close control. Non-focusable, like the dock.

**App icon.** One artwork — `src-tauri/icons/` — identifies SpeakEasy Mini on every
surface it appears on: the executable in Explorer, the installer and its
shortcuts, each window, and the notification-area entry. The set is declared in
`bundle.icon`; Tauri would otherwise fall back to a hardcoded path and get the
executable resource right while leaving the installer on NSIS defaults. The tray
is given the same icon explicitly, because Tauri does not default it and the
shell registers an entry with no icon as an **empty cell that still occupies a
slot** — it does not error, so the only symptom is a blank gap that appears
whenever SpeakEasy Mini is running.

**Dictation lifecycle.** The hotkey starts a recording and the hotkey stops it;
the dock's Stop button also stops one. There is no VAD. Each recording can run
for up to two minutes; at the ceiling it auto-stops and transcribes what it
captured.

That ceiling matters more than it used to. Transcription now happens entirely
*after* the recording ends, so a two-minute dictation is followed by a wait —
around 30 s on CPU, under 5 s on the GPU. The dock's working indicator is the
only thing on screen during it, which is why the action row is unconditional and
why silence there would read as a hang.

Close quits the whole app; closing during an active dictation asks first via a
native dialog and never discards speech silently. Launching the app again while
it is already running restores **the dock**, without focusing it.

These properties are **unchanged and remain load-bearing**: the dock is
`no-activate` and **never captures keyboard focus**, so using it cannot steal
focus from the application the text is being delivered into. It has **no
clipboard command beyond copying the final it just produced, no OS-input
command, and no privileged command** — that prohibition is absolute and enforced
by test. Only a narrow allowlist of session-control commands is reachable from
it, and the addressable transcript copy is not among them.

Because it never takes focus, the dock is **not keyboard operable by design**.
Every action it offers has a keyboard path elsewhere: the global shortcut for
start and stop, the settings window for everything else.

Capture state, device, and the reason a dictation ended remain visible. There is
no live text to classify: the three-tier stable/mutable/final vocabulary left
with the streaming engine, and a transcript either exists or does not.

**The dock's geometry.** A narrow 62×400 window (label `hud-dock`) that clings
to a screen edge. It was one of two HUD presentations and is now the only one,
but its measured constraints are unchanged and are recorded here because they
were expensive to find.

The width was 130, then 96, and is 62. 130 was a measured floor and the floor
is real: Windows clamps a window to the default minimum tracking size *while it
is being created*, so declaring 60, or 96, or 96 with a matching `minWidth`, all
produced ~130 with nothing reported anywhere. What gets past it is re-applying
the size after creation, which is not subject to that clamp —
`enforce_dock_size`, before the dock is ever shown and before its placement is
computed. The size is still declared once, in `tauri.conf.json`, and read back
from the config rather than restated in Rust.

**The height was 360 and is 400** (owner, 2026-08-28). The 40px pays for the
action button being present in every state; the meter is the only `1fr` row, so
it absorbed all of it and went 112 to 152 -- more than the 134 it had before the
engine row existed. The engine row also moved *below* the meter in the same
change: above it, the chip was a filled horizontal pill cutting across a
52px-wide vertical column, so it severed the wordmark from the waveform and sat
at the card's visual centre while the bottom third was empty. Measured on the
running window both ways, the reorder costs the waveform nothing.

**The chrome row holds two controls and its icons are 20px, not the shared
24px.** The row's content box is 44px at a 52px card, so two 24px buttons want 48
and overflow into the card's `overflow: hidden` -- which does not error, it clips
whichever glyph loses, exactly as happened to the close button once before. Two
20px buttons take 40 and leave 4. The narrowing is scoped to the dock rather than
applied to `.hud-icon`, which the log and notice windows also spend on strips that
are not 52px wide. A gear here duplicates the right-click menu's Settings item on
purpose: a native popup is discoverable only by someone who already guessed to
try it, on a window that never takes keyboard focus.

52px of card is what every control in it now answers to, and the one it
genuinely strains is Stop: the row's inset and the button's own padding are 4px
rather than 8px, and the label sits at the caption size. It keeps its word.
An icon-only stop was the alternative and it is worse here — this is the
presentation with no transcript and no status line, so that label is the only
text in the window that says what pressing it does.

The window is transparent and what it draws is a rounded card inset inside it,
so the dock reads as floating over the desktop rather than welded to it. It
has six rows in a fixed order — close, vertical wordmark, engine indicator,
level meter, elapsed clock, Stop — and none of them is conditional: the last
two are empty outside a dictation rather than absent, so the meter's box cannot
move under a running recording. No transcript and no microphone picker; both
belong to the presentation with room for them.

**The engine indicator says which device Granite runs on, and whether it is
up.** A small chip naming the device in text — `CPU` or `GPU` — with the
engine's health carried by the chip's colour *and* by the shape of the mark
beside it: a filled dot when ready, a hollow ring while loading or before a
model is installed, a triangle when it has failed. Colour is never the only
signal, and under `forced-colors` every fill flattens to one system colour, so
the shape is what survives.

**It never names a device the worker has not reported.** The engine answers
`unknown` for a pre-v2 worker and `not_configured` before any warm, and during
the load the worker has usually not completed its handshake — so the honest
chip there is an em dash, not a guess. Inferring `GPU` from a CUDA-capable
*binary* is the overreach that once put `device=cuda` in a support log for a
worker running entirely on the processor; see "The active provider is the
device, never the pack".

Not-yet-installed and failed are deliberately different states. A machine
part-way through setup has not failed at anything and the dock already says
"Setup needed" in words, so it takes the same amber as loading; a missing
worker binary is a broken installation and takes the failure treatment. The
row costs the meter 22px — it went from 134 to 112, a 16% reduction in the
dock's most characterful element, accepted knowingly because a device you have
to hover for is a device you do not know while working in another window.

The level meter is drawn as a symmetric waveform rather than as the default
HUD's button fill: one row of bars per retained sample, the newest in the
middle and older samples spreading outward in both directions. At rest it is a
flat grey rail, never the varying coloured bars — a resting *meter* would say
the app is listening when it is not. The bars are decoration; the value itself
stays on a native `<meter>` that is visually hidden but in the accessibility
tree, and under `forced-colors: active` the bars drop the same way the default
HUD's fill does.

**The waveform is amplified, and banded by loudness.** The level reaching it is
a 100 ms *peak* of samples normalised to ±1.0 with no gain stage anywhere
behind it, so speech at a comfortable distance peaks around 0.1–0.3 — and drawn
unshaped into a tapered box that was a stub a few pixels long. It carries a
gain and a curve now, tuned so ordinary speech fills about half the card, a
genuinely loud passage pegs, and a quiet room stays near the rail. Loud bars
are purple, middling blue, quiet green; the band comes from the sample's own
loudness rather than from the drawn width, which also carries the age taper, so
a bar keeps the colour of the moment it recorded as it travels outward. Colour
is not the only signal — width is what moves first, and the whole meter is grey
until capture is actually running. The `<meter>` still reports the value
unshaped.

**The action row says what is happening after the key is released.** It held
Stop and nothing else, which meant that from the moment a dictation ended until
the text appeared — Granite's pass plus finalization, comfortably over a second
— the dock was indistinguishable from idle, and a failure was indistinguishable
from idle permanently. Three dots now fade in sequence while the app is
stopping or transcribing, in the same `--hud-busy` amber the deleted HUD's
record button used for its own processing tone — one colour for "working"
across the app, rather than each surface inventing one. They are three separate
elements and the animation is a phase offset per dot, so under
`prefers-reduced-motion` the mark degrades to three static dots rather than to
nothing.

Two endings get a mark and the rest do not. A delivery the target app refused
shows a clipboard — the text is on the clipboard and that is what to do about
it — and a failed dictation shows a warning triangle with the specific error on
hover, the only place the dock can name which failure it was. They differ by
glyph and not only by tone, which is what has to tell them apart under
`forced-colors: active` where both flatten to the same system colour. A
successful insertion shows nothing at all: the text arriving in the app the
user was typing into is the confirmation, and a mark for it would either have
to clear itself or stop meaning anything.

Stop is on the dock, though it was not on the first version of it. The
argument for leaving it off was that dictation is hotkey-driven — but the
hotkey has three activation modes and hands-free has no key that ends a
recording at all, so a dock without Stop leaves the user's only way out on the
window they docked to get away from. It paints in the `--recording` fill while
recording, on a rule scoped to the listening state rather than to the button,
because red carries one operational meaning here and that has to be true of the
stylesheet and not only of the component that renders it. That fill was the
deleted HUD's record button's, kept so a user who had seen both did not have to
learn a second appearance for the same press. Its own close button, and "Close"
on its right-click menu, quit the app through the graceful path, mid-dictation
confirmation included; the menu's only other item is Settings.

Dragging the dock snaps it against whichever edge (left or right) it lands
nearest, seated 24 logical pixels in from that edge — flush would clip the
card's own corners and shadow against the screen boundary, and would put it
exactly where Windows' snap target and auto-hide taskbar reveal strip already
live. Both the snap and the vertical clamp are measured against the monitor's
**work area** rather than the whole display: a dock dropped low must land above
the taskbar, because it is `alwaysOnTop` and `skipTaskbar` and there would be
nothing to click to get it back. That edge plus its vertical position persist
across restarts. There is no presentation to switch between any more, so the
stored HUD mode and the large window's own placement are both gone from the
profile; a profile written by SpeakEasy carries them and this app ignores them.

## Accessibility and input

**Scope limitation, recorded honestly.** Screen-reader support
is **deprioritized** as of 2026-07-29 and is not actively verified. The product
does not claim assistive-technology conformance. Existing ARIA attributes are
retained and asserted by tests, but announcement quality is not tuned or tested
beyond that. This is a stated limitation, not a claim of coverage.

Still required:

- Every interactive element has an accessible name and programmatic state.
- Heading levels and landmarks describe the page without visual-position assumptions.
- In the settings window, tab order matches reading order and custom controls implement expected keyboard patterns. The dock and the pinned log are exempt — neither takes focus, and their actions are reachable by shortcut or from settings.
- Recording state has redundant visual and optional audible feedback with independent volume/off settings. State is never encoded by color alone.
- **The two cues say which transition happened.** They were `MessageBeep`
  system sounds — chimes the OS also plays for unrelated notifications, so
  neither identified the transition or the app that made it. They are
  synthesised tones now: a rising perfect fifth to start, the same fifth
  falling to stop. The interval is chosen for being consonant enough to hear
  thirty times a day, and its *direction* is the message — rising opens,
  falling resolves — so nothing has to be learned and neither is a word. They
  are still gated on the General page's recording-sounds setting, still
  redundant to the visual state, and playback never blocks or fails a
  transition: it runs on its own thread and every error is dropped, because a
  machine with no output device must still record.
- **A cue is held open until the device has taken it, not for as long as it
  lasts.** The first version of this shipped silent: it slept for the cue's own
  duration plus a margin and then closed the stream, and on the machine it was
  reported broken on the output device took *zero* samples in the first 400ms
  and the whole 245ms cue by 653ms — so the stream was destroyed before one
  sample left, on every dictation, with nothing logged and nothing failing.
  That every error on this path is dropped is what made it invisible. The wait
  is now on the device signalling it has the last sample, with a ceiling and a
  drain margin behind it.
- Errors identify the field or subsystem and a safe recovery action without exposing paths or content.
- Timeouts never remove a recoverable transcript.
- The polite live region carries **state transitions only** — started, stopped, working, delivered, failed. Input-level values never enter it. There are no hypotheses to announce; an earlier build announced every revision, roughly twice a second, which was a defect and is now structurally impossible.
- Targets remain usable at 200% zoom. No critical content depends on hover.

**The dock is not keyboard operable, by design** — it never takes focus, so
using it cannot steal focus from the application the text is being delivered
into. That trade only holds if every action it offers is reachable by keyboard
somewhere else, so each one is:

| Dock action | Keyboard path |
|---|---|
| Stop (and transcribe) | The global shortcut |
| Microphone | Settings › Audio |
| Open settings | The tray's Settings item, or the dock's right-click menu |
| Close / quit | Settings › General › Keyboard access, which takes the same confirmed graceful path |
| Read a past transcript | Settings › Transcript log |
| Pin the log on top | Settings › Transcript log |

The table shrank when the large HUD went. Minimize and reset-position were that
window's, and listing a keyboard path to a control nobody can reach by mouse
either would be worse than listing nothing.

## Contrast, themes, and motion

Light and dark palettes meet WCAG AA contrast for normal text and UI boundaries.
This is **measured, not asserted** — `scripts/Invoke-ContrastProof.ps1` reads the
computed colours out of both running windows, resolves each element against the
background actually painted behind it, and reports every ratio below 4.5:1 for
normal text, 3:1 for large text, and 3:1 for a control's outline. It reports the
live `--surface` with each measurement, so a run that measured the wrong palette
fails instead of passing quietly.

`forced-colors: active` removes decorative backgrounds, uses system colors,
retains visible focus, does not hide meter/progress state, and keeps the
navigation rail's current page and the setup stepper's current step distinguishable
— both are marked by a border colour, which forced colors would otherwise flatten
away. User/system theme selection is persisted as `system`, `light`, or `dark`;
system is the default. The dock and the pinned log are exempt and stay dark (see
Design tokens).

Animation is limited to progress/state transitions, never recording-state ambiguity. Under `prefers-reduced-motion: reduce`, transitions and smooth scrolling are disabled. No flashing, pulsing transcript, parallax, or auto-advancing carousel is permitted.

## Responsive, high-DPI, and multi-monitor behavior

The settings window supports its 720×500 logical minimum through large desktop
widths without clipped actions. The minimum height is 500 rather than the 560 the
redesign brief specified, because 560 does not fit: a 1920×1080 panel at 200%
scaling leaves 516 logical pixels of work area, and a minimum the user cannot
shrink out of would put the window partly off-screen. Below 760 px the navigation
rail becomes a full-width row of buttons above the content — still one `tablist`
with the same keyboard pattern — and the content column keeps a bounded readable
line length above it. Text reflows at 200% zoom without loss or two-dimensional
scrolling except data tables. **No supported size requires horizontal
scrolling**, measured rather than assumed: see Automated checks.

Dock size is computed in device-independent units and placed fully inside the
selected monitor work area, including negative virtual-screen coordinates.
Placement is recalculated after DPI, monitor, taskbar/work-area, and resume
changes. It never assumes the primary monitor or a fixed scale factor.

The dock is a fixed 62×400 logical and is not resizable, so its CSS layout is
identical at every display scaling — only its physical size changes. It is moved
by dragging it, and its edge and vertical position are persisted, clamped to the
current work area on restore, and fall back to the computed default when the
saved monitor is gone.

The pinned log is resizable and has no persisted placement, deliberately: it is
opened for a task and closed again, and restoring it to wherever it sat days ago
is not obviously what anyone wants.

## Localization

Catalog keys are stable semantic identifiers. Sentences are complete strings; do not concatenate grammar fragments or assume English word order. Use locale-aware number, byte, duration, date, and list formatting. Allow at least 50% text expansion and unbounded wrapping. Avoid embedded keyboard letters; shortcut names come from platform formatting.

Do not translate model IDs, revisions, provider IDs, hashes, paths, license identifiers, or diagnostic reason codes. Display names and explanatory labels may be localized separately. Localized UI availability never implies ASR or translation capability. Bidirectional text uses isolation around user/model/endpoint values, and transcript containers preserve Unicode without normalization or HTML interpretation.

## Automated checks

`apps/desktop/tests/scaffold.test.mjs` covers catalog ownership, semantic roles,
keyboard structure, inert rendering, the five-group settings limit,
theme/reduced-motion/forced-colors rules, colour and type going through the
tokens, narrow dock IPC, and the declared app-icon set together with the
tray's use of it. It reads `.tsx` for rules about markup and
`.ts` as well for rules about wiring, because a guarantee that moves into a hook
would otherwise stop being checked.

The no-activate windows' IPC assertions are explicit **allowlists**, one per
authority rather than one shared list:

- The **dock** may invoke only its session-control commands plus
  `hud_transcript_copy`, which takes no argument and resolves the newest final
  in Rust — so it can copy what it just produced and cannot name anything else.
- The **pinned log** may invoke `session_transcript_log`,
  `session_transcript_copy` and `transcript_log_unpin`. It gets the
  *addressable* copy the dock is refused, because browsing the log is the whole
  purpose of that window, and it is one the user opened deliberately rather than
  permanent furniture that is on screen during every dictation.

Both must name every command they invoke as a string literal, so the allowlists
are checkable at all, and the assertion that neither invokes a paste, OS-input or
delivery command keeps passing. The frontend may *configure* delivery; it may
never perform it. Rust tests assert non-allowlisted commands are refused per
window, and the frontend suite asserts the same sets against the IPC schema — so
widening either one in code without amending both fails the gate.

Two of those rules are derived rather than listed, which is the difference
between a check and a record of where a bug has already been seen:

- **No effect may read a race-prone command without retrying or polling.** The
  hazard is read out of the Rust signatures — every `#[tauri::command]` taking a
  `tauri::State` — and the readers are found by scanning every `useEffect` in
  `src/`, following one level of local function calls so a read behind a helper
  is not missed. The rule it replaced named one command in one file, and was
  green on the day the identical defect was found in a second file with a second
  command. Because every assertion in it is of the form "nothing was found",
  which is also what a broken scanner reports, it carries instrument
  self-checks: the derived hazard must contain commands known to take a `State`
  and exclude ones known not to, and the scan must be shown reaching through a
  helper and reaching the poll exemption.
- **The token budget must cover the longest dictation the capture ceiling
  allows.** Granite's `max_new_tokens` truncates with no error and no
  end-of-generation token, and nothing downstream can catch it, because
  `is_plausible` only rejects transcripts that are too long. It is unreachable
  today only because `MAX_CAPTURE_SECONDS` caps a dictation at about a fifth of
  the budget — arithmetic nobody did for months while carrying the risk. The two
  constants cannot be compared in Rust: they live in `speakeasy-desktop` and
  `speakeasy-granite`, and the desktop crate deliberately does not depend on the
  one that compiles llama.cpp. So they are compared as source, at deliberately
  pessimistic speaking and tokenising rates, and raising the ceiling now fails
  the gate until the budget moves with it.

Four proof scripts drive the **running** app, because a green suite is not
evidence that anything works — each of these has found a defect the suite missed:

| Script | What it measures |
|---|---|
| `Invoke-TranscriberProof.ps1` | A real dictation: speech through the speakers, heard by the microphone, delivered into a real target. Locates controls by selector, not by pixel offset. Now also the only end-to-end check that Granite ran at all, since there is no second engine whose output could mask a failure. |
| `Invoke-SettingsProof.ps1` | Every settings page at several widths: horizontal overflow, scroll regions, and the rail's collapse point. |
| `Invoke-ContrastProof.ps1` | Rendered contrast in both windows and both palettes. |
| `Invoke-ScalingProof.ps1` | Both windows at 100%, 125%, 150% and 200%. |
| `Get-SpeakEasyWindows.ps1` | Which windows exist, are visible, and own a taskbar button. |

`Invoke-WebviewProbe.ps1` underlies the layout and contrast proofs: it talks to
WebView2's debugging protocol, so layout rules are read as numbers and controls are
clicked by selector. A click either lands on the real element or fails loudly. It
addresses all four windows, `notice` included since 2026-08-26 — a window
declared `visible: false` still runs its React tree, so the notice can be
measured without provoking a ceiling stop, and measuring it is the only way to
see that its content overflowed its declared height.

Installed high-DPI review is a manual, owner-run check. Interactive
assistive-technology review is **no longer performed** — see the scope limitation
under Accessibility and input.

## Personalization surface

Personalization lives inside **Transcription**, preserving the six top-level
groups. The surface names the `en-US`-only locale support, the
unmeasured hotword limitation, and disabled contacts import. Corrections
require separate observed/corrected fields and an explicit save. Snippets
show the whole-final-utterance grammar and action prohibition, render
bodies in inert text/pre elements, and expose exact delete/reset. JSON
import shows counts/conflicts before a separate commit; it never renders
imported markup or creates links/actions.

## Optional polish surface

Rule cleanup and writing profiles live inside **Transcription**; provider consent
lives inside **Output & Privacy**, preserving six top-level groups.
Each deterministic rule has an independent switch plus a master Off state,
and the UI labels the exact `en-US` grammar without claiming pause/tone
punctuation.

**Two rules have no switch, deliberately.** `immediate_repetitions` and
`self_corrections` are forced off and are not offered. `resolve_self_correction`
discards everything before `" I mean "` — live data loss on any transcript — and
it fires more often on Granite's fluent output than on a transducer's. They were
already disabled whenever Granite produced the transcript; Granite produces every
transcript now, so the switches could only ever have been decorative.

Cloud polish is not currently wired into the desktop app (see
[ARCHITECTURE.md](ARCHITECTURE.md)); if it's connected later, credential
values must never enter the WebView, and provider/model text, imported
prompts, transcript output, and suggestions must render only as inert text.
