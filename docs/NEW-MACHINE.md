# Bootstrapping a new development machine

Everything below is what a `git clone` does *not* give you. The repo carries
the code, the version pins and the trusted manifest; it deliberately carries
none of the toolchain and none of the ~2.2 GB of model weights, which are
fetched and hash-verified by the scripts here. There is no native runtime to
stage any more — that left with the streaming engine; see "Fetch the native
runtime" below.

Budget roughly an hour, most of it downloads, plus one long first compile —
`speakeasy-granite` builds llama.cpp from source.

**Check before you read.** `scripts/Initialize-DevMachine.ps1` reports every
prerequisite below as PRESENT or MISSING with the reason, stages the pinned Node
(checksum-verified against nodejs.org) and re-links the native runtime if it is
already fetched, then exits non-zero if anything a build needs is absent:

```powershell
.\scripts\Initialize-DevMachine.ps1
```

Read this document for whatever it reports missing. It deliberately installs
none of Visual Studio Build Tools, LLVM, or the CUDA Toolkit — each needs an
interactive UAC prompt that cannot be driven unattended, and a script that
appeared to install them and silently had not would be worse than one that
reports the truth. It resolves tools in the same places
`Enter-DevEnvironment.ps1` does rather than asking PATH alone, because
`~/.cargo/bin` is on the user PATH and CMake on the machine PATH, and a shell
that inherited a reduced environment has neither.

## 1. Prerequisites

Windows 11 x64. Install these before cloning; the first four are hard
requirements and the last is optional.

| | Why | Note |
| --- | --- | --- |
| **Visual Studio Build Tools** (Desktop C++) | MSVC toolset and the Windows SDK | `llama-cpp-sys-2` compiles C++. If installed via `winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools"`, the `--override` can silently no-op — verify with `vswhere -requires Microsoft.VisualStudio.Workload.VCTools`; if that prints nothing, re-run the installer's `setup.exe modify --add Microsoft.VisualStudio.Workload.VCTools` directly |
| **CMake** | `llama-cpp-sys-2` builds llama.cpp with it | Or stage it under `.tools/cmake-<version>-windows-x86_64` |
| **LLVM** (for `libclang`) | `bindgen`, via `llama-cpp-sys-2` | Or set `LIBCLANG_PATH` |
| **Node** and **npm** | Frontend and the Tauri CLI | Versions pinned in `.node-version` and `package.json` `engines` — match them exactly, or `npm ci` fails fast with `EBADENGINE`. Simplest on a machine that already has a different Node: stage the official portable zip as `.tools/node-v<version>-win-x64` (verify it against `https://nodejs.org/dist/v<version>/SHASUMS256.txt` before extracting) and `Enter-DevEnvironment.ps1` will put it ahead of the system Node in the shell. That needs no elevation, leaves the existing Node alone, and is undone by deleting the directory. nvm-windows (`winget install CoreyButler.NVMforWindows`) also works, but `nvm use` requires elevation. Do **not** add a second standalone MSI install — a mismatched one cannot be uninstalled non-interactively (`msiexec /X` returns error 1730 for a non-elevated caller even on an admin account; a UAC prompt is unavoidable) |
| **Rust** | The workspace | Pinned by `rust-toolchain.toml`. A normal rustup install is fine; a workspace-local toolchain under `.tools/{cargo,rustup}` is used in preference if present |
| CUDA Toolkit + NVIDIA driver | *Optional.* Building the GPU Granite worker | CPU is a full fallback. Without it you simply run the CPU pack. The Toolkit installer needs an interactive UAC prompt — it cannot be driven unattended |
| `cargo-llvm-cov`, `cargo-deny`, `cargo-audit`, `gitleaks`, `ripgrep` | *Optional.* Only needed for the **full** `Invoke-ScaffoldChecks.ps1` gate (coverage, dependency/advisory scans, secret scanning, the worker-client-definition check) | All five are pinned in `dependency-policy/tools.json`, and the gate asserts each version, so install them exactly: `cargo install --locked cargo-deny@<v> cargo-audit@<v> cargo-llvm-cov@<v> ripgrep` and `winget install Gitleaks.Gitleaks`. `cargo-llvm-cov` also needs `rustup component add llvm-tools-preview`. Nothing above this line needs them — `cargo build`/`clippy`/`test` and the app itself run without them |

`scripts/Enter-DevEnvironment.ps1` wires CMake, `LIBCLANG_PATH` and the CUDA
paths into the current shell, and **warns rather than throws** when CMake or
libclang are missing — only crates that bind native libraries care, so a miss
surfaces later as `is 'cmake' not installed?` or `Unable to find libclang`
rather than at setup time. Read its output: it now names the CMake it resolved,
its version, and whether that came from `.tools` or from PATH, which is the
question a failing native build asks first. Any staged
`.tools\cmake-<version>-windows-x86_64` is preferred over the ambient one, at
whatever version you staged; the script reports the choice rather than enforcing
a particular version.

## 2. Clone and install

Clone to **`C:\Coding_Projects\speakeasy-granite-rust-mini`** if you want Claude
Code's project memory to carry over without renaming anything — see
"Machine-local state worth carrying over" below.

```powershell
git clone https://github.com/kwp490/speakeasy-granite-rust-mini.git
cd speakeasy-granite-rust-mini
npm ci
```

## 3. Fetch the native runtime

There isn't one to fetch any more. This step used to stage ~2.9 GB of
sherpa-onnx and ONNX Runtime archives across six downloads, and nothing in the
workspace would link without them (`LNK1181`).

The streaming engine is gone, and with it that whole dependency. The only native
code left is llama.cpp, which `speakeasy-granite` compiles from source using the
toolchain from step 2 — so what you need is a C++ compiler, CMake and libclang,
not a download.

```powershell
. .\scripts\Enter-DevEnvironment.ps1
```

## 4. Fetch the models

One pack, and setup normally fetches it. For a dev checkout there is a script:

```powershell
.\scripts\Get-Granite.ps1
```

That downloads and verifies into `.tools\granite-speech-4.1-2b`. To make the
*app* see it, stage the two files it loads into the profile's model directory:

```powershell
$pack = "$env:APPDATA\ai.speakeasy.mini\model-lifecycle\models\granite-speech-4.1-2b-q4_k_m-cpu\q4_k_m-2026-05-11"
New-Item -ItemType Directory -Force -Path $pack | Out-Null
Copy-Item .\.tools\granite-speech-4.1-2b\granite-speech-4.1-2b-Q4_K_M.gguf $pack
Copy-Item .\.tools\granite-speech-4.1-2b\mmproj-model-f16.gguf $pack
```


Copying that directory straight off the old machine works too and skips the
download.

## 5. Optional: put Granite on the GPU

**Granite's GPU support is a compile-time choice, not a download.** This is the
single most surprising thing about the setup, and it is why the app has no
provider-override control: no setting can conjure a CUDA-built worker binary.

**The installer does this for you now** (since 1.6.0, 2026-08-26). Its "choose
how it runs" page offers the graphics card on a machine with a supported NVIDIA
card, and setup fetches the CUDA worker — pinned by digest in
`models/trusted-manifest.json`, like the model weights — along with the two CUDA
libraries it loads. It records what it *proved* it installed, not what was
chosen: look for `installed=` and `device=` in the `granite_warm` log line, and
`speakeasy-bootstrapper --verify-provider` re-asks the same question of an
installed build without a reinstall. An upgrade re-stages the worker, so it no
longer reverts to the processor silently.

So the rest of this section is only for a **dev checkout**, which always builds
the CPU worker because `speakeasy-granite`'s `cuda` feature would make the CUDA
Toolkit a hard build prerequisite for the whole gate. There used to be a
`scripts\Enable-GraniteCuda.ps1` for this; it was retired on 2026-08-26 with the
release that made it unnecessary, and staging a worker by hand is now:

```powershell
cargo build --release -p speakeasy-granite-worker --features cuda
```

then copy that exe over `target\debug\proof\granite-worker.exe` and put the three
CUDA libraries beside it — the easiest source is an installed 1.6.0's own
`proof\` directory, where setup has already placed the versions the catalog pins.
`Stage-DevRuntime.ps1` overwrites that worker on every `npm run tauri -- dev`, so
run `npm run dev` and the binary separately when the card is what you are
testing.

Measured on an RTX 5090: Granite's resident pass is 1,571.9 ms on CPU against
156.4 ms on CUDA — RTF 0.158 versus 0.0157 — holding ~3.3 GiB of VRAM. Cold load
is 5,218 ms against 2,104 ms.

Verify by GPU context, never by transcript. A CPU and a GPU build decode the
same words, so the text cannot tell you which one ran:

```powershell
nvidia-smi --query-compute-apps=pid,process_name --format=csv
```

`granite-worker.exe` should be listed. Match by **PID**, not by process name: `nvidia-smi` prints `[Insufficient Permissions]`
instead of a path for many processes depending on who is asking, so a name
match silently finds nothing and reads exactly like a CPU fallback.

The log says it too, from worker protocol v2 onward:

```text
granite_warm result=ok engine=cpu_gpu_pack_not_installed device=cuda
```

`device` is the authority on where Granite ran, and only the worker can answer
it — the CUDA backend is compiled into the binary, so nothing on the filesystem
can tell. `engine` is a different fact: which *pack* was selected and why. They
disagree here for a good reason, because there is only one Granite pack and the
same GGUF runs on either device.

**After changing the worker protocol, the app and both workers must be rebuilt
and reinstalled together**, and the CUDA worker re-staged. A mismatched pair
reports `ProtocolMismatch` rather than failing silently, but Granite stops
delivering until you fix it.

## 6. Verify

```powershell
.\scripts\Invoke-ScaffoldChecks.ps1 -SkipNpmInstall
```

Then the app itself, from `apps\desktop`:

```powershell
npm run tauri -- dev
```

The first launch spends a minute in `Stage-DevRuntime.ps1` building the two
worker binaries before Vite starts — that is expected, and it is what gives
the dev build a `proof/` directory to resolve. Without it every dictation
fails at finalization with `runtime_resources_unavailable`. See `CLAUDE.md`
for the rest of the traps.

Dictate once and confirm `hotkey_delivery result=committed` in
`%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log`. For anything you intend
to *measure*, build and install a real installer first — see
`docs/LOCAL-DEVELOPMENT.md`; dev and installed builds differ in ways that have
invalidated measurements before.

## 7. Machine-local state worth carrying over

None of this is in git. All of it is optional except the first, which is the
only one that is hard to reconstruct.

**Claude Code's project memory.** Copy the whole directory:

```
%USERPROFILE%\.claude\projects\C--Coding-Projects-speakeasy-granite-rust-mini\
```

The folder name is derived from the checkout path, so cloning to the same path
means it lands correctly with no edits. Clone elsewhere and you must rename
the folder to match — `C:\dev\speakeasy` becomes `C--dev-speakeasy`. The
durable project knowledge is in `CLAUDE.md` and travels with the repo either
way; what this directory adds is the personal preferences and the running
project state.

**The app profile** — hotkey binding, chosen microphone, HUD mode and
position, history consent:

```
%APPDATA%\ai.speakeasy.mini\config\settings.json
```

**The diagnostic log**, if you want the history:
`%APPDATA%\ai.speakeasy.mini\logs\speakeasy.log`.

**Do not copy** `target\` or most of `.tools\` — they are build output and
fetched artifacts, they are large, and they are wrong on a different machine.
The one exception worth copying to save a download is the model directory under
"Fetch the models".
