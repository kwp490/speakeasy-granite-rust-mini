[CmdletBinding()]
param()

$repositoryRoot = Split-Path -Parent $PSScriptRoot

foreach ($requiredFile in @(
    '.cargo/audit.toml', 'Cargo.lock', 'package-lock.json', 'deny.toml',
    'dependency-policy/advisory-allowlist.json', 'dependency-policy/tools.json',
    'dependency-policy/build-scripts.json'
)) {
    if (-not (Test-Path (Join-Path $repositoryRoot $requiredFile))) {
        throw "Required dependency-policy file is missing: $requiredFile"
    }
}

$rootPackage = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'package.json') | ConvertFrom-Json
if ($rootPackage.packageManager -ne 'npm@10.9.2') {
    throw 'packageManager must remain pinned to npm@10.9.2.'
}
if ($rootPackage.engines.node -ne '=22.16.0' -or $rootPackage.engines.npm -ne '=10.9.2') {
    throw 'Node and npm engines must remain exact pins.'
}

$packageFiles = @(
    Join-Path $repositoryRoot 'package.json'
    Join-Path $repositoryRoot 'apps/desktop/package.json'
)
foreach ($packageFile in $packageFiles) {
    $package = Get-Content -Raw -Encoding utf8 $packageFile | ConvertFrom-Json
    foreach ($group in @('dependencies', 'devDependencies', 'optionalDependencies')) {
        if ($null -eq $package.$group) {
            continue
        }
        foreach ($dependency in $package.$group.PSObject.Properties) {
            if ($dependency.Value -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') {
                throw "$packageFile has a non-exact $group pin: $($dependency.Name)=$($dependency.Value)"
            }
        }
    }
}

$metadataJson = & cargo metadata --format-version 1 --locked --no-deps
if ($LASTEXITCODE -ne 0) {
    throw 'cargo metadata failed.'
}
$metadata = $metadataJson | ConvertFrom-Json

$allowedDependencies = @{
    # serde/serde_json exist for exactly one thing: the framed-JSON worker
    # protocol in `worker_protocol.rs`. That module lives here rather than in
    # `speakeasy-asr` so a second inference runtime (Granite) can speak the
    # same wire protocol without linking ONNX Runtime — see
    # `docs/handoff/granite-final-pass.md`, Phase 4.
    'speakeasy-domain' = @('serde', 'serde_json')
    'speakeasy-audio' = @('allocation-counter', 'cpal', 'speakeasy-domain')
    # nvml-wrapper is a dev-dependency only: the CUDA smoke test reads free VRAM
    # to tell a real CUDA session from a silent CPU fallback. The shipping code
    # in this crate does not touch NVML.
    'speakeasy-asr' = @(
        'nvml-wrapper', 'sherpa-onnx', 'speakeasy-domain', 'transcribe-cpp',
        'unicode-segmentation'
    )
    # `llama-cpp-2` is the second inference runtime in this workspace, and the
    # only dependency here that *compiles* a C++ project rather than linking a
    # pre-fetched one. It exists because the delivered transcript comes from
    # Granite Speech, which is a GGUF model and has no ONNX path.
    #
    # It is a single direct dependency on purpose: `mtmd` (the audio projector)
    # is taken as a feature rather than as a separate crate, and token decoding
    # goes through `token_to_piece_bytes` so that `encoding_rs` is not pulled in
    # for what amounts to one `String::from_utf8`.
    'speakeasy-granite' = @('llama-cpp-2')
    # `zip` is here for one reason: NVIDIA ships every Windows CUDA and cuDNN
    # redistributable as a zip, and the Decisions table has the app fetch them
    # from NVIDIA's own hosts rather than rehosting them as tar. It is taken with
    # `default-features = false` and only `deflate-flate2`, which reuses the
    # flate2 already in this list; that resolves to two new packages in total
    # (`zip`, `typed-path`) rather than the aes/zstd/lzma/ppmd set the default
    # features pull in. Extraction is a trust boundary here, so the narrow
    # feature set is the point, not an optimisation.
    'speakeasy-models' = @(
        'bzip2', 'flate2', 'fs2', 'nvml-wrapper', 'reqwest', 'semver', 'serde', 'serde_json',
        'sha2', 'speakeasy-domain', 'sysinfo', 'tar', 'tempfile', 'unicode-normalization',
        'winreg', 'zip'
    )
    'speakeasy-delivery' = @('speakeasy-domain', 'unicode-segmentation')
    'speakeasy-remote' = @('reqwest', 'serde', 'serde_json', 'sha2', 'speakeasy-domain', 'subtle')
    'speakeasy-transforms' = @('serde', 'serde_json', 'sha2', 'unicode-normalization', 'unicode-segmentation')
    'speakeasy-storage' = @('serde', 'serde_json', 'sha2', 'rusqlite', 'speakeasy-domain', 'speakeasy-transforms', 'tempfile')
    'speakeasy-windows' = @('keyring', 'sha2', 'speakeasy-delivery', 'speakeasy-domain', 'uiautomation', 'win32job', 'winreg', 'winsafe')
    'speakeasy-test-support' = @('speakeasy-domain', 'speakeasy-windows')
    # A measurement rig, not shipped code: `speakeasy-transcription-bench` times
    # the two inference runtimes against each other. `hound` reads the WAV
    # fixtures. `sysinfo` and `nvml-wrapper` record the host the numbers came
    # from, which is not incidental — this project has invalidated its own
    # measurements twice over an unrecorded host detail (debug vs release, and
    # which runtime pack a rig actually resolved), so a result without the
    # machine attached is not reproducible. `transcribe-cpp` is feature-gated
    # here exactly as it is in `speakeasy-asr`.
    #
    # The crate was added without an entry here, and because this check throws on
    # the first unknown workspace package it took the whole dependency-policy
    # step down with it — every exact-pin and registry rule for every other crate
    # stopped being evaluated.
    'speakeasy-bench' = @(
        'hound', 'nvml-wrapper', 'serde', 'serde_json', 'speakeasy-asr',
        'speakeasy-domain', 'speakeasy-granite', 'sysinfo', 'transcribe-cpp'
    )
    'speakeasy-desktop' = @(
        'serde', 'serde_json',
        'speakeasy-domain', 'speakeasy-audio', 'speakeasy-asr', 'speakeasy-models',
        'speakeasy-delivery', 'speakeasy-remote', 'speakeasy-storage', 'speakeasy-transforms',
        'speakeasy-windows', 'tauri', 'tauri-build', 'tauri-plugin-global-shortcut',
        'tauri-plugin-single-instance', 'tracing',
        'sha2',
        # Dev-only: the Granite hash-verification test stages files under a real
        # temp directory, matching `speakeasy-models`/`speakeasy-storage`'s own
        # use of this crate.
        'tempfile'
    )
    # Setup and repair in one binary (owner decision, 2026-08-14). The wizard's
    # own dependencies -- hardware probing, downloads, the native GUI -- are
    # added here as the stages that use them land, not ahead of them.
    # `winsafe` for the native wizard: `gui` for its chrome, `shell` for the
    # IShellLink that Start Menu shortcuts need once NSIS is gone. Both are safe
    # bindings, which is what makes a native wizard possible under this
    # workspace's `unsafe_code = "forbid"`.
    # `speakeasy-domain` supplies `CancelToken` and nothing else: it is what makes
    # the reused download and install lifecycle interruptible, and setup's whole
    # resume story rests on being able to stop a transfer without discarding it.
    'speakeasy-bootstrapper' = @(
        'semver', 'speakeasy-domain', 'speakeasy-models', 'speakeasy-storage',
        'speakeasy-windows', 'sysinfo', 'winreg', 'winsafe'
    )
    'speakeasy-inference-worker' = @('sha2', 'speakeasy-domain', 'speakeasy-asr', 'speakeasy-models')
    # No `speakeasy-models` yet: there is no Granite entry in the trusted
    # manifest until Phase 5 lands it, so `LoadModel` here checks that the
    # conventional GGUF filenames exist on disk and nothing more. See
    # `docs/handoff/granite-final-pass.md`, Phase 4 and Phase 5.
    'speakeasy-granite-worker' = @('speakeasy-domain', 'speakeasy-granite')
}

$workspaceIds = [Collections.Generic.HashSet[string]]::new([string[]]$metadata.workspace_members)
# Names of the workspace's own crates, so the path-dependency exemption below can
# be limited to them. A path dependency pointing outside the workspace — a
# vendored copy of someone else's crate, say — would otherwise escape both the
# exact-pin rule and any registry review, which is the opposite of the point.
$workspaceNames = [Collections.Generic.HashSet[string]]::new(
    [string[]]($metadata.packages |
        Where-Object { $workspaceIds.Contains([string]$_.id) } |
        ForEach-Object { [string]$_.name })
)
foreach ($package in $metadata.packages) {
    if (-not $workspaceIds.Contains([string]$package.id)) {
        continue
    }
    if (-not $allowedDependencies.ContainsKey($package.name)) {
        throw "Workspace package is absent from the dependency policy: $($package.name)"
    }
    $allowed = $allowedDependencies[$package.name]
    foreach ($dependency in $package.dependencies) {
        # Exactness is a supply-chain rule: it stops cargo silently resolving a
        # different build of someone else's crate out of a registry. A path
        # dependency inside this workspace never resolves from a registry — the
        # code is the source in this tree, and cargo uses the path regardless of
        # what the requirement says — so an exact requirement there buys no
        # protection at all. What it did buy was an un-bumpable version: eleven
        # `=1.0.0` requirements that had to be rewritten in lockstep with
        # `[workspace.package] version` or the workspace stopped resolving.
        #
        # Narrowed rather than dropped. The allowlist check below still applies to
        # every dependency including these, so a crate still cannot take on a new
        # internal dependency without that being recorded here — which is the part
        # of this policy that was ever doing work for internal crates.
        $isWorkspacePath = -not [string]::IsNullOrEmpty($dependency.path)
        if (-not $isWorkspacePath -and $dependency.req -notmatch '^=') {
            throw "$($package.name) has a non-exact Cargo requirement: $($dependency.name) $($dependency.req)"
        }
        if ($isWorkspacePath -and -not $workspaceNames.Contains([string]$dependency.name)) {
            throw ("$($package.name) depends on $($dependency.name) by path, but it is not a " +
                'member of this workspace. Path dependencies outside the workspace are not ' +
                'covered by the exact-pin rule and are refused.')
        }
        if ($dependency.name -notin $allowed) {
            throw "$($package.name) has a forbidden direct dependency: $($dependency.name)"
        }
    }
}

# Patched crates: the shape of the patch, and the bytes it resolves to.
#
# This exists because of what a `path` patch actually did here. Until 2026-08-04
# `[patch.crates-io]` pointed `llama-cpp-2` and `llama-cpp-sys-2` at gitignored
# `.tools/llama-cpp-rs`, and the consequences were worse than "one machine":
#
#   * `Cargo.lock` recorded neither `source` nor `checksum` for either crate, so
#     the lockfile pinned nothing at all about a third party's code.
#   * cargo treats a path patch as a *local* crate, so `--cap-lints allow` did not
#     apply and `cargo clippy --workspace -- -D warnings` failed on 24 warnings
#     inside llama-cpp-rs itself. That gate had never passed with the crate in it.
#
# Both are fixed by a `rev`-pinned git dependency. What made that incident bad,
# though, was not `path` — it was that the path pointed *outside version control*.
# `.tools/` is gitignored, so nothing recorded the bytes anywhere: not the
# lockfile, not this repository.
#
# A path into a **committed** directory is a different thing, and forbidding it
# outright blocked a configuration this workspace needs. `vendor/transcribe.cpp`
# is patched by path because the workspace carries local commits to that crate's
# own build script — `9ec0ae0` retries the build against OUT_DIR when the short
# junction fails, without which it does not build here at all. Those commits do
# not exist at any upstream rev, so a `rev`-pinned git dependency would silently
# discard them and reintroduce the failure.
#
# So what survives is deliberately narrow (owner decision 2026-08-11):
#
#   * A git patch must still be pinned to a full 40-character rev.
#   * A path patch is accepted without inspecting the path. An earlier version of
#     this change required it to be in-tree and git-tracked; that was removed on
#     purpose. The consequence is that a patch pointing at a gitignored or
#     out-of-tree directory — the `.tools/llama-cpp-rs` case above — would now
#     pass, and nothing would record its bytes.
#   * Either way the crate must appear in `$allowedPatchedSources`, so patching
#     something nobody reviewed still fails.
#
# Measured 2026-08-11 with the vendored patch in place: `cargo clippy --workspace
# --all-targets -- -D warnings` passes, so the `--cap-lints` half of the original
# incident is not biting for this crate. If a future vendored fork does drag lint
# noise in, that shows up as a clippy failure, which is the right place for it.
#
# Every entry here is meant to be temporary. When upstream catches up, the patch
# is deleted and this table has to be emptied or this check fails — which is the
# point: retiring a fork should be a diff someone has to make, not something
# everyone forgets.
#
# A `path:` value states the repo-relative directory the patch must resolve to;
# any other value is the exact `Cargo.lock` source string expected for it.
$allowedPatchedSources = @{
    'llama-cpp-sys-2' = 'git+https://github.com/kwp490/llama-cpp-rs?rev=d989ffdb0caf3d809cdcff1850629bd3da6ed067#d989ffdb0caf3d809cdcff1850629bd3da6ed067'
    'transcribe-cpp' = 'path:vendor/transcribe.cpp/bindings/rust/transcribe-cpp'
    'transcribe-cpp-sys' = 'path:vendor/transcribe.cpp'
}

$cargoManifestText = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'Cargo.toml')
$patchSection = [regex]::Match(
    $cargoManifestText,
    '(?ms)^\[patch\.crates-io\]\s*$(.*?)(?=^\[|\z)')
$patchedNames = [Collections.Generic.HashSet[string]]::new()
if ($patchSection.Success) {
    foreach ($line in $patchSection.Groups[1].Value -split "`r?`n") {
        $entry = [regex]::Match($line.Trim(), '^([A-Za-z0-9_-]+)\s*=\s*(\{.*\})$')
        if (-not $entry.Success) {
            continue
        }
        $name = $entry.Groups[1].Value
        $spec = $entry.Groups[2].Value
        [void]$patchedNames.Add($name)

        if (-not $allowedPatchedSources.ContainsKey($name)) {
            throw "[patch.crates-io] patches $name, which is not in `$allowedPatchedSources."
        }
        if ($spec -match '\bpath\s*=') {
            # A path patch is accepted as-is. Owner decision 2026-08-11: the shape of
            # the patch is not what this policy is for, and the rule that forbade
            # `path` blocked a configuration the workspace needs.
            #
            # What is deliberately given up: nothing here checks that the path is
            # inside this repository or tracked by git. `Cargo.lock` records no
            # source and no checksum for a path patch, so for a path pointing
            # somewhere untracked — the gitignored `.tools/llama-cpp-rs` case that
            # the old rule was written for — nothing would record the bytes at all.
            # `vendor/transcribe.cpp` is committed, so today the bytes are fixed by
            # this repo's history; that is a property of the current patch rather
            # than something enforced.
            #
            # What still applies: the crate has to be in `$allowedPatchedSources`
            # above, so patching something nobody reviewed still fails.
            continue
        }
        if ($spec -notmatch '\bgit\s*=' -or $spec -notmatch '\brev\s*=\s*"[0-9a-f]{40}"') {
            throw ("[patch.crates-io] entry for $name must be a git dependency pinned by a " +
                'full 40-character rev, so the bytes are fixed by the lockfile.')
        }
    }
}
foreach ($name in $allowedPatchedSources.Keys) {
    if (-not $patchedNames.Contains($name)) {
        throw ("`$allowedPatchedSources lists $name but [patch.crates-io] no longer patches " +
            'it. If the fork has been retired, remove it here too.')
    }
}

# Now the lockfile: every package is registry-sourced, a workspace member, or an
# expected patch resolving to exactly the reviewed rev.
$lockText = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'Cargo.lock')
foreach ($block in ($lockText -split '(?ms)^\[\[package\]\]\s*$') | Select-Object -Skip 1) {
    $lockName = [regex]::Match($block, '(?m)^name\s*=\s*"([^"]+)"').Groups[1].Value
    if ([string]::IsNullOrEmpty($lockName)) {
        continue
    }
    $sourceMatch = [regex]::Match($block, '(?m)^source\s*=\s*"([^"]+)"')
    $lockSource = ''
    if ($sourceMatch.Success) {
        $lockSource = $sourceMatch.Groups[1].Value
    }

    if ($allowedPatchedSources.ContainsKey($lockName)) {
        $expected = $allowedPatchedSources[$lockName]
        if ($expected -like 'path:*') {
            # cargo records a path patch as a local crate: no `source`, no
            # `checksum`. Asserting the absence is the whole check available here —
            # what fixes these bytes is the git-tracked vendor/ tree, verified
            # against `$allowedPatchedSources` where the patch is declared. A
            # `source` appearing means the patch stopped applying and the crate
            # came from a registry or a fork instead, which must not pass quietly.
            if (-not [string]::IsNullOrEmpty($lockSource)) {
                throw ("Cargo.lock has $lockName from '$lockSource', but the policy expects " +
                    "the in-tree patch at '$($expected.Substring(5))'. The path patch is no " +
                    'longer applying.')
            }
            continue
        }
        if ($lockSource -ne $expected) {
            throw ("Cargo.lock has $lockName from '$lockSource', but the policy expects " +
                "'$expected'. A patched crate must resolve to the exact reviewed rev.")
        }
        continue
    }
    if ($lockSource -eq 'registry+https://github.com/rust-lang/crates.io-index') {
        continue
    }
    if ([string]::IsNullOrEmpty($lockSource) -and $workspaceNames.Contains($lockName)) {
        continue
    }
    throw ("Cargo.lock has $lockName from an unreviewed source '$lockSource'. Every package " +
        'must come from crates.io, be a workspace member, or be a reviewed patch.')
}

# Every crate with a build script must be inventoried in build-scripts.json (see
# docs/handoff/granite-final-pass.md, known-risk item 6). `implicit-build-downloads
# = false` is declared but nothing had ever enforced it; this at least forces a
# human to look whenever a *new* build-script dependency shows up, rather than
# discovering one only by reading every Cargo.lock diff by hand.
#
# The filtering has to happen inside `node`, not `ConvertFrom-Json`, because the
# full (non---no-deps) metadata graph is large enough to contain sibling object
# keys that only differ by case (observed: "USB"/"usb"), and ConvertFrom-Json
# builds a case-insensitive dictionary that throws on that collision. `node` has
# no such limitation, and this workspace already pins an exact Node version.
#
# The filter script is written to a temp file rather than passed via `node -e`:
# PowerShell's native-argument marshalling mangles embedded double quotes in an
# inline `-e` string (observed corrupting `""` mid-argument), which a file path
# argument avoids entirely.
$buildScriptFilterScript = Join-Path ([IO.Path]::GetTempPath()) 'speakeasy-build-script-filter.js'
@(
    'let data = "";'
    'process.stdin.on("data", (chunk) => { data += chunk; });'
    'process.stdin.on("end", () => {'
    '    // PowerShell''s native-to-native piping re-encodes the stream and'
    '    // prepends a UTF-8 BOM; .trim() strips it (U+FEFF is JS whitespace).'
    '    const metadata = JSON.parse(data.trim());'
    '    const rows = [];'
    '    for (const pkg of metadata.packages) {'
    '        if (!pkg.targets.some((t) => t.kind.includes("custom-build"))) continue;'
    '        let source = "workspace";'
    '        if (pkg.source && pkg.source.startsWith("registry+")) source = "crates.io";'
    '        else if (pkg.source && pkg.source.startsWith("git+")) source = "git (patched fork)";'
    '        rows.push({ name: pkg.name, version: pkg.version, source });'
    '    }'
    '    process.stdout.write(JSON.stringify(rows));'
    '});'
) | Set-Content -Encoding utf8 -Path $buildScriptFilterScript
try {
    $buildScriptPackagesJson = & cargo metadata --format-version 1 --locked | & node $buildScriptFilterScript
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo metadata (full graph, for the build-script inventory) failed.'
    }
} finally {
    Remove-Item -Force $buildScriptFilterScript
}
$buildScriptPackages = $buildScriptPackagesJson | ConvertFrom-Json
$buildScriptInventory = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'dependency-policy/build-scripts.json') | ConvertFrom-Json
$inventoried = [Collections.Generic.HashSet[string]]::new(
    [string[]]($buildScriptInventory.entries | ForEach-Object { "$($_.name)@$($_.version)" }))
# Workspace-own crates are matched by name, deliberately, while everything from
# a registry or a fork stays pinned to name@version.
#
# The version in the key exists to force a fresh look when a *third party's*
# build script arrives at a version nobody has read. That reasoning does not
# transfer to a crate in this repository: its build.rs is in-tree, so any change
# to it shows up in an ordinary diff and is reviewed as code. Keying it by version
# bought no review and cost a chore -- and not a harmless one. Bumping
# [workspace.package] version made this throw on speakeasy-desktop, which took the
# whole dependency-policy step down until the JSON was hand-edited to match, with
# nothing announcing that as a release step. Same shape as the model-pack version
# ceilings: a bump-coupled manual edit with no build-time failure when forgotten.
$inventoriedWorkspaceNames = [Collections.Generic.HashSet[string]]::new(
    [string[]]($buildScriptInventory.entries |
        Where-Object { $_.source -eq 'workspace' } |
        ForEach-Object { $_.name }))
foreach ($package in $buildScriptPackages) {
    $key = "$($package.name)@$($package.version)"
    if ($inventoried.Contains($key)) {
        continue
    }
    if ($package.source -eq 'workspace' -and $inventoriedWorkspaceNames.Contains($package.name)) {
        continue
    }
    throw ("$key has a build script but is absent from " +
        "dependency-policy/build-scripts.json. Add an entry (see the file's own " +
        "'regenerate' field) noting whether its build.rs shows a network-client " +
        'indicator, before this counts as reviewed.')
}

$allowlist = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'dependency-policy/advisory-allowlist.json') | ConvertFrom-Json
foreach ($ecosystem in @('rust', 'npm')) {
    foreach ($entry in $allowlist.$ecosystem) {
        foreach ($requiredField in @('advisory', 'package', 'reason', 'expires')) {
            if ([string]::IsNullOrWhiteSpace($entry.$requiredField)) {
                throw "The $ecosystem advisory allowlist has an entry without $requiredField."
            }
        }

        try {
            $expiry = [DateTime]::ParseExact(
                [string]$entry.expires,
                'yyyy-MM-dd',
                [Globalization.CultureInfo]::InvariantCulture,
                [Globalization.DateTimeStyles]::AssumeUniversal)
        } catch {
            throw "The $ecosystem advisory allowlist has an invalid expiry date: $($entry.expires)."
        }

        $daysRemaining = ($expiry.Date - [DateTime]::UtcNow.Date).Days
        if ($daysRemaining -lt 0) {
            throw "The $ecosystem advisory allowlist entry $($entry.advisory) expired on $($entry.expires)."
        }
        if ($daysRemaining -le 30) {
            Write-Warning "The $ecosystem advisory allowlist entry $($entry.advisory) expires in $daysRemaining day(s), on $($entry.expires)."
        }
    }
}

$denyConfig = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'deny.toml')
$auditConfig = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot '.cargo/audit.toml')
foreach ($entry in $allowlist.rust) {
    if ($denyConfig -notmatch [regex]::Escape($entry.advisory)) {
        throw "Rust advisory is missing from deny.toml: $($entry.advisory)"
    }
    if ($auditConfig -notmatch [regex]::Escape($entry.advisory)) {
        throw "Rust advisory is missing from .cargo/audit.toml: $($entry.advisory)"
    }
}

$policyTools = Get-Content -Raw -Encoding utf8 (Join-Path $repositoryRoot 'dependency-policy/tools.json') | ConvertFrom-Json
$denyVersion = (& cargo-deny --version).Trim()
if ($LASTEXITCODE -ne 0 -or $denyVersion -ne "cargo-deny $($policyTools.cargo_deny)") {
    throw "Expected cargo-deny $($policyTools.cargo_deny); found '$denyVersion'."
}
$auditVersion = (& cargo-audit --version).Trim()
if ($LASTEXITCODE -ne 0 -or $auditVersion -ne "cargo-audit $($policyTools.cargo_audit)") {
    throw "Expected cargo-audit $($policyTools.cargo_audit); found '$auditVersion'."
}
$coverageVersion = (& cargo llvm-cov --version).Trim()
if ($LASTEXITCODE -ne 0 -or $coverageVersion -ne "cargo-llvm-cov $($policyTools.cargo_llvm_cov)") {
    throw "Expected cargo-llvm-cov $($policyTools.cargo_llvm_cov); found '$coverageVersion'."
}

Write-Host 'SpeakEasy dependency policy: ok'
