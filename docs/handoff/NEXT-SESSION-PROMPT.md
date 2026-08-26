# Prompt for the next session — publish the CUDA worker

Copy everything below the line into a new Claude Code session in
`C:\Coding Projects\speakeasy-granite-rust-mini`.

> **Superseded.** The previous version staged item 16, four sentences of copy.
> That is still open but item 3 closes it on its own, so it is no longer the
> task. Items 11, 14, 15 and 17 all closed on 2026-08-26.

---

You are picking up work on SpeakEasy Mini, a local-only Windows dictation app
(Rust + Tauri 2 + React, all inference on-device) at
`C:\Coding Projects\speakeasy-granite-rust-mini`, branch `main`, released
version **1.5.1** with unreleased fixes on `main`.

**This session is item 3: publish the CUDA Granite worker and finish the path
that lets a user install it.** It is the last item in the file that changes what
the product can do. Everything else left is a release and four sentences of copy
that this closes on its own.

## Read first, in this order

1. `CLAUDE.md` — orientation. "Traps that fail silently" and "Settled decisions"
   are not optional.
2. `docs/handoff/CURRENT.md` — the banner, then **item 3** in full, including the
   two sections added on 2026-08-26: what is already built and proven, and the
   gap. Then **item 12** and **item 16**, which item 3 closes.
3. `docs/UI-GUIDE.md` — living spec. Amend it in the same change.

## Where it already got to, so you do not redo it

- **The worker is built and proven on the card.** From `67c9498`, with
  `--features cuda`: `target\release\speakeasy-granite-worker.exe`, 57,052,672
  bytes, **sha256 `1d4a3ad57e72acaaa55a507f88733ac18f77909349c4bf7c7bc017269499170a`**.
  Both hardware tests pass against that exact binary, staged into
  `target\debug\proof\` with the three CUDA libraries beside it —
  `granite_final_pass_transcribes_the_fixture_through_the_real_worker_process`
  reported `worker=cuda first=41.1286058s second=361.2019ms`.
- **Publish those bytes, not a rebuild of them.** MSVC embeds a timestamp, so a
  rebuild produces a different digest — three CUDA workers on this machine have
  three digests and two of them are byte-for-byte the same *size*. If that file
  is gone, rebuild it and re-run both hardware tests before pinning anything.
- **Three decisions are already taken** (owner, 2026-08-26): Hugging Face carries
  the **worker only**, with the CUDA DLLs still fetched from NVIDIA's CDN where
  the manifest already pins both archives by digest; the repo is
  `speakeasy-mini-runtime` under whichever account the token belongs to; and the
  order is **upload, then close the gap, then pin** — nothing user-visible
  changes until the whole path works.

## Step 1 — the one human step

`hf` (huggingface_hub 1.13.0) is installed and **not logged in**. Ask the owner
to run this themselves, in their own terminal:

```bash
hf auth login
```

**Never handle the token.** Do not ask for it, do not echo it, do not write it to
a file or an environment variable. `hf auth login` stores it in the user's own
credential store. Then `hf auth whoami` names the account, which is what decides
the repo owner.

## Step 2 — upload

Create `<account>/speakeasy-mini-runtime` and upload the proven worker. **Confirm
the exact repo name, file and visibility with the owner before uploading** — this
publishes to a public host and is not undoable by deleting the file afterwards.
Record the resolved download URL, the byte count and the digest; those three are
what the manifest entry needs.

## Step 3 — close the gap, and this is the real work

Item 3 promises that pinning the artifact makes every layer answer `true` "without
a second edit". **It does not.** The wizard gates its graphics-card radio on
`download::graphics_card_configuration_available()`, which calls
`inspect_gpu_payload` against
`%LOCALAPPDATA%\SpeakEasy Mini\proof\granite-worker.exe` — a file that does not
exist during the provider page of a **first** install, because setup has not
extracted the payload yet. Pinning alone leaves the option disabled everywhere,
reporting `WorkerNotInstalled` instead of `WorkerNotPublished`.

`inspect_gpu_payload` conflates two questions:

| Question | Who asks | Needs |
| --- | --- | --- |
| Is a graphics-card configuration **installable**? | the wizard's provider page | published, and fetchable |
| Is one **installed here**? | the app's warm path, `--verify-provider`, `smoke::gpu_payload_rejection` | files present in `proof/` |

Split them, and give `download::plan` its second item — it already takes
`provider` and ignores it, with a comment saying it is for exactly this.

**This machine cannot see the bug.** It has the CUDA worker and all three
libraries staged, so with the artifact pinned the option lights up here and looks
right. Test the installable path against a directory that does *not* contain a
worker, or you will ship a wizard that only offers the graphics card to machines
that already have it.

## Step 4 — pin, then prove

Add the artifact to `models/trusted-manifest.json` under
`granite-worker-cuda-windows-x64` (`kind: "native-runtime"`), following the shape
of the two NVIDIA entries. Then:

- `--verify-provider` becomes able to answer for the first time. Until the worker
  is published it reports `device=cpu evidence=gpu_worker_not_published`, which
  is item 12 and is why the proof was circular. Item 12 closes here.
- Item 16's fourth sentence stops being reachable in that combination, because a
  published worker restores the `Admissible`→`Qualified` promotion. Check the
  four-sentence stack on the running Settings window rather than assuming.
- Run the gate, then `Build-LocalInstaller.ps1`,
  `Test-InstallerLifecycle.ps1` and `Test-SetupWizard.ps1 -Uninstall`. The
  packager builds a CPU worker and only refuses a *staged* CUDA worker with no
  libraries, so pinning does not break the build — verified 2026-08-26 — but the
  wizard now has a live graphics-card option and `Test-SetupWizard.ps1` asserts
  the page it is on before every click, so it is the thing that will catch a
  regression there.

## Then the release (item 2b)

`main` carries the item 11, 14, 15 and 17 fixes. Worth a release note that says
plainly what stopped lying: on 1.5.1 `Settings → Advanced` renders five headings
and **no facts at all**, and `Settings → General` reports a working shortcut as
unregistered. Cutting it is four commands and no automation — item 2b has them —
then `gh release create` with `SpeakEasyMiniSetup.exe` and `SHA256SUMS`. `gh` is
authenticated as `kwp490`.

## State of the machine

- Installed at `%LOCALAPPDATA%\SpeakEasy Mini`, **1.5.1**, with the freshly built
  CUDA worker staged over it and all three CUDA 13 libraries beside it. The
  install marker reads `cpu`, correctly, because the worker is not published yet.
- `target\debug\proof\` also holds that worker plus the three libraries, which is
  what the hardware tests read. **`npm run tauri -- dev` silently reverts it** to
  the CPU worker via `Stage-DevRuntime.ps1` — re-stage before re-running them.
- Toolchain **Rust 1.98.0** (pinned), CUDA Toolkit **13.3**, CMake present.
  Cargo/rustc are **not on PATH** — dot-source
  `. .\scripts\Enter-DevEnvironment.ps1` in every new shell.
- `Get-Process SpeakEasy*` before launching anything: the single-instance lock
  silently absorbs a second launch and you then test the old binary. You have
  standing permission to stop and start SpeakEasy Mini's own processes.
- Hardware: RTX 4070 Laptop GPU, compute 8.9.

## Two habits worth carrying in

**A release frontend needs `--features custom-protocol`.** Without it the binary
embeds `devUrl`, every window loads `http://localhost:1420`, and with no Vite
server they all come up blank while the probe reports each one absent — which
reads exactly like the app failing to start.

```powershell
cargo build --release -p speakeasy-desktop --features custom-protocol
```

**Ask the backend, then ask it a second way.** A rendered string cannot tell you
whether the backend is wrong, the read was refused, or the answer arrived before
it was true — three causes, one appearance, and that cost a whole diagnosis on
2026-08-25.

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
```

```powershell
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression 'window.__p=null; window.__TAURI_INTERNALS__.invoke("hotkey_status").then(r=>window.__p={ok:r}).catch(e=>window.__p={err:String(e)}); "fired"'
.\scripts\Invoke-WebviewProbe.ps1 -Window settings -Expression 'JSON.stringify(window.__p)'
```

`-Window` takes `settings`, `dock`, `log` or `notice`. `window.__TAURI__` is
undefined; `window.__TAURI_INTERNALS__` is the bridge.

## When you are done

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

Amend `docs/UI-GUIDE.md` in the same change, and update item 3 in
`docs/handoff/CURRENT.md` with what you found — including what you checked and
cleared, because the next gap will be in whichever layer nobody listed.
