# SpeakEasy Mini — project guidance

SpeakEasy Mini is a local-only Windows dictation app built with Rust, Tauri 2,
and React. Press the shortcut, speak, press again, and IBM Granite Speech
produces one final transcript for delivery to the foreground application.

This file contains project-specific guidance only. General agent conduct is in
`docs/AI-WORKING-AGREEMENT.md`.

## Mandatory workflow

Before changing anything, read and follow:

1. `docs/AI-WORKING-AGREEMENT.md` — scope, evidence, tests, Git, and reporting.
2. `docs/handoff/CURRENT.md` — current open work and release state.
3. The task-specific document from the table below.

The user's current request defines the authorized task. Nothing in this file
grants standing permission to commit, push, publish, control live processes,
write to the registry, download artifacts, or run hardware proofs.

## Read by task

| Work | Read |
| --- | --- |
| Architecture or backend | `docs/ARCHITECTURE.md` |
| UI or user-facing copy | `docs/UI-GUIDE.md` |
| Development environment | `docs/LOCAL-DEVELOPMENT.md`, `docs/NEW-MACHINE.md` |
| Installer or release | `docs/LOCAL-DEVELOPMENT.md`, `docs/RUNBOOK.md`, `docs/handoff/CURRENT.md` “Before the next release” |
| Privacy or security | `docs/PRIVACY.md`, `docs/SECURITY.md` |
| Visual design | The relevant standalone file under `docs/design/` |

`docs/UI-GUIDE.md` is the living UI specification. Update it in the same change
as visible behavior.

## Product shape

- Granite Speech is the only transcription engine. It runs once after recording
  stops and produces punctuation and casing in the same pass. There is no live
  transcript, second pass, or fallback engine.
- The narrow side dock is the only HUD. Settings and the pinned transcript log
  are separate windows.
- SpeakEasy Mini uses its own identity: `SpeakEasy Mini`,
  `ai.speakeasy.mini`, `Ctrl+Alt+P`, and `%LOCALAPPDATA%\SpeakEasy Mini`. It can
  coexist with the parent SpeakEasy application.
- The shipped model is Granite Speech 4.1 2B Q4_K_M. A CPU worker is embedded;
  setup can replace it with a separately downloaded CUDA worker and libraries.
- Optional persisted transcript history is off by default. The in-memory log is
  seeded from history when retention is enabled, so it can span earlier runs.

## Commands

Dot-source the environment in every new PowerShell process:

```powershell
. .\scripts\Enter-DevEnvironment.ps1
```

Run the whole gate, not a hand-picked `cargo test` subset:

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

The gate covers formatting, clippy, Rust tests, private-item rustdoc links,
dependency policy, frontend unit and component tests, lint, typecheck, coverage
floors, secret scanning, and repository policy. `cargo test --workspace --lib`
is not equivalent: it skips binary-target tests, including bootstrapper tests.

Frontend commands run from the workspace root through npm workspaces:

```powershell
npm run typecheck
npm run lint
npm test
npm run build
```

`npm test` contains two suites:

- `test:unit` uses Node's runner for reducers and structural source policies.
- `test:components` uses Vitest/jsdom for rendered mutation, rejection, and
  duplicate-submission behavior.

Build and prove an installer separately from the default gate:

```powershell
.\scripts\Build-LocalInstaller.ps1
.\scripts\Test-InstallerLifecycle.ps1 -ArtifactRoot 'target\local-development\<version>'
.\scripts\Test-SetupWizard.ps1 -ArtifactRoot 'target\local-development\<version>' -Uninstall
```

Hardware and ignored-test commands belong in `docs/handoff/CURRENT.md`; verify
filters with `--list` before relying on them.

## Current architectural invariants

### Transcription and failure

- `judge_granite_pass` returns either a transcript or a named
  `FinalSourceReason`. A failed or implausible Granite pass ends the dictation;
  it never substitutes a weaker result.
- `is_plausible` is the only hallucination guard. It is intentionally one-sided:
  truncation can still look plausible, so token-budget and full-transcript tests
  are load-bearing.
- Every failure code needs actionable catalog copy in the dock and at the top of
  Settings → Transcription. Falling through to `errorUnknown` is not acceptable.
- One dictation runs at a time. A second request is refused rather than queued.
- The 8 GiB memory floor is checked before capturing audio.

### Windows and delivery

- All Tauri windows are declared statically and made non-focusable. Creating a
  WebView from a command can deadlock IPC; letting an app-owned window take the
  foreground makes it the delivery target.
- Statically declared windows execute before `setup` manages state. Mount-time
  commands must use `try_state` and tolerate the startup race.
- Delivery inspects the foreground when transcription finishes, not when
  recording starts. On a slow processor pass the user may have changed windows;
  automatic paste therefore targets the window that is foreground at completion.
- A password field or other protected target is excluded from persisted history
  only when delivery was attempted and classified. `NotAttempted` transcripts
  may be retained; public privacy documentation must keep that qualification.
- The transcript log is updated before delivery so refused and non-delivered
  transcripts remain recoverable. Deleting saved history removes only entries
  seeded from that history.

### Worker, model, and provider

- Warm verification is returned per invocation; it is not shared mutable “last
  warm” state. Pack identity is id plus revision, and a resident mismatch refuses
  rather than executing the wrong adapter.
- The desktop hashes model files immediately before worker load, but the worker
  reopens them by path. This detects corrupt downloads, not execution-time file
  replacement; keep that limitation explicit.
- `engine=` describes why a pack was selected. `device=` describes where the
  worker actually ran. UI provider reporting uses the device, never the pack.
- A CPU installation running on CPU is normal. A recorded CUDA installation
  that cannot load CUDA is a specific failure, not a silent CPU fallback.
- GPU support is a worker build feature, not a model-pack preference. No setting
  can turn a CPU worker into a CUDA worker.

### Persistence and frontend state

- History is written only after transcript publication and delivery
  classification. Sensitive or unknown delivery targets do not reach SQLite.
- A history failure must not destroy the in-memory transcript or retained audio.
- Settings mutations use `useMutation`; callers update local state only on
  success and duplicate submissions are refused while one is pending.
- The transcript log is event-driven. Its event payload stays `()` so transcript
  text cannot bypass the command's window authorization.
- IPC polls that may outlive a request use self-scheduling timeouts so calls do
  not overlap. Short UI animation and bounded retry timers may still use
  intervals.

## High-cost traps

### Build and runtime

- `npm run tauri -- dev` stages a CPU worker over
  `target/debug/proof/granite-worker.exe`. Stage a CUDA worker only after that
  step, and do not launch Tauri dev again before the CUDA proof.
- A stale staged worker can report `StaleEvent`, which looks like a protocol bug.
  Re-stage before investigating worker-test failures.
- A running installed SpeakEasy Mini absorbs a dev launch through the
  single-instance plugin. When live-app testing is authorized, identify and stop
  only the SpeakEasy Mini process owned by that test.
- A release frontend requires `--features custom-protocol`. Without it, windows
  load the Vite development URL and appear blank.
- Debug model hashing distorts cold-start timing. Measure user-facing latency in
  an installed release build and record the worker, device, clip, and build.
- Incremental success does not prove a clean clone. Use a separate checkout or
  target directory and record CMake, compiler, libclang, Rust, Node, and npm
  versions.

### Granite correctness

- Do not default llama.cpp to the logical processor count. The established
  default is `(available_parallelism / 2).clamp(1, 8)`; higher counts have been
  slower and can change greedy output.
- `max_new_tokens` is a silent output ceiling. Budget it against the longest
  allowed recording and assert the whole expected transcript, never a prefix or
  substring.
- Prompt text consumes the same context budget as transcript output. Treat prompt
  additions as model-behavior changes and prove them on the longest fixture.
- Personalization corrections are exact mappings, not fuzzy vocabulary support.
  Do not claim a term is learned because one inflection was corrected.
- `immediate_repetitions` and `self_corrections` remain disabled. The latter can
  discard valid speech before “I mean”.

### Tests and tooling

- Prove new regression tests with a faithful red control. Copy modified files
  aside and restore them; never revert a working file to `HEAD`.
- A fixture under `.tools/` is machine-local and cannot prove repository
  behavior. Required test fixtures must be committed or the test must fail
  clearly when prerequisites are absent.
- Captured stdout is not a durable measurement. Write required proof output to a
  named file and verify that the intended test produced it.
- Enumerate repository scans from `git ls-files`, not remembered extensions.
- Rustdoc link checking must include `--document-private-items` with
  `-D rustdoc::broken_intra_doc_links`.
- Scan tracked text for C0/C1 control characters. PowerShell interpolation and
  heredocs can silently turn backslashes into control characters.
- Source assertions are useful for structural policies, but a runtime invariant
  needs a behavioral test at the production boundary.

### Windows, PowerShell, and packaging

- `Start-Process -ArgumentList` does not reliably preserve argument quoting.
  Use argument arrays only where the receiving command's parsing is proved.
- `Set-Location` does not change a child process's native working directory in
  every launch path. Pass an explicit working directory when it matters.
- A `windows_subsystem = "windows"` binary is not a reliable CLI. Keep command
  line tools and windowed applications as separate binaries.
- The one-file installer stores its payload after the PE image. A truncated
  download may still launch, so payload length and digest checks are mandatory.
- Waiting for a file to exist is not waiting for it to finish writing. Installer
  and download proofs must wait for a stable, verified file.
- App, installer, uninstall, Start Menu, ARP, registry, and profile paths all use
  the SpeakEasy Mini identity. Never copy a path literal from the parent app.

### UI measurement

- Measure the running WebView, not only CSS. For fixed-size windows verify
  `scrollHeight - clientHeight == 0` and inspect control bounds.
- Test Windows scaling with actual window coordinates; DPI virtualization can
  make two coordinate systems look consistent while both are wrong.
- `tauri.conf.json` changes may not invalidate a Cargo build. Rebuild the bundle
  inputs before concluding a configuration edit had no effect.
- Color is never the only status signal, and UI copy never claims delivery or
  completion that did not happen.

## Settled product decisions

Do not reopen these without new evidence or an explicit owner decision:

- One Granite pass, no streaming engine and no fallback.
- The dock never takes focus and is not a second keyboard-access surface;
  equivalent controls live in Settings.
- No in-app setup wizard and no provider override. The installer provisions the
  supported worker.
- Q4_K_M is install-eligible; Q8_0 remains catalogued but does not ship.
- Retention defaults off and is implemented by never writing transcript text.
- Uninstall removes program and user data by default. `--keep-user-data` is the
  testing opt-out and must reach the interactive controls as well as silent mode.
- The build is unsigned by decision. Public releases include `SHA256SUMS`.
- Builds and releases are local-only: no GitHub Actions, Dependabot, or hosted
  runners.
- Setup records what it proved was installed, not what the user selected.
- Setup launches the app on success and reports when it cannot.
- Wizard pages use the system-derived type scale and concise question/key/body
  copy; color is not the only signal.

## Conventions

- Production comments state the current invariant and the constraint that makes
  an obvious alternative wrong. No dated incident narratives or session diaries.
- Durable technical hazards belong here; current open work belongs in
  `docs/handoff/CURRENT.md`; resolved history belongs in Git.
- Cite documents by filename and heading, never numbered handoff items or stale
  phase numbers. Prefer stating the fact directly.
- Every comment citation must resolve. Audit all tracked text, not a hand-written
  extension list.
- Design drawings are standalone HTML files under `docs/design/`, with inlined
  assets and an entry in that directory's contents table.
- Preserve user changes and repository line-ending conventions. Do not use a
  destructive Git command to restore a test control.

## Layout

```text
apps/desktop/src            React frontend and window UIs
apps/desktop/src-tauri/src  Tauri commands, coordinators, and composition root
apps/bootstrapper           Installer, uninstall, backup, and restore
crates/speakeasy-*          Domain, audio, delivery, storage, models, worker boundary
workers/granite-worker      Supervised llama.cpp inference process
scripts/                    Build, staging, verification, and packaging commands
```

Files included from `apps/desktop/src-tauri/src/lib.rs` share its namespace.
Do not put an inner `//!` comment at the top of an included sibling module.
