//! Every word the wizard shows, in one place.
//!
//! `docs/UI-GUIDE.md` requires that all visible text come from the locale
//! catalog and that Rust return stable codes rather than arbitrary user-facing
//! prose. The React app satisfies that with `apps/desktop/src/catalog.ts`, which
//! a native wizard cannot reach — so this is the wizard's half of the same rule,
//! and it exists for the same reason: copy that is scattered through the logic
//! that produces it cannot be reviewed as copy, and this project's copy carries
//! obligations. Setup must not describe detected hardware as qualified, must not
//! claim delivery that did not happen, and must not imply this build is signed.
//!
//! English only, deliberately. `UI-GUIDE.md` records that the shipped app is
//! `en-US`; inventing a localization mechanism the rest of the product does not
//! have would be scope this feature did not ask for. The shape here — codes to
//! strings, one module — is what makes adding one later mechanical.

/// The wizard's window title. Not the product tagline: this is what appears in
/// the taskbar while setup runs.
pub const WINDOW_TITLE: &str = "SpeakEasy Mini setup";

/// Navigation. Short verbs, per the UI guide's preference.
pub const BACK: &str = "Back";
pub const NEXT: &str = "Next";
pub const CANCEL: &str = "Cancel";

/// Shown in place of `NEXT` on the last page.
pub const FINISH: &str = "Finish";

/// The steps, in order.
///
/// One entry per page the wizard specifies, so navigation is real before any page
/// has content. Each carries the heading and the body the page shows; the pages
/// that need controls grow them in their own stage, and a page whose work is not
/// built yet says so rather than showing a plausible-looking blank.
pub struct Step {
    pub heading: &'static str,
    pub body: &'static str,
}

pub const STEPS: &[Step] = &[
    Step {
        heading: "Check this computer",
        body: "SpeakEasy Mini looks at the processor, memory, disk space and graphics \
               card, and reports what it finds. Speech recognition and the \
               punctuation pass are checked separately: they use different \
               graphics-card runtimes, so this computer can be suitable for one \
               and not the other.\n\n\
               Nothing is installed or downloaded during this step.",
    },
    Step {
        heading: "Choose how it runs",
        body: "SpeakEasy Mini preselects the fastest option each engine can actually \
               use on this computer, and shows what it costs to download and \
               install. You can override either one to run on the processor.\n\n\
               A graphics card that clears the requirements has not yet been \
               tested — setup runs a real execution check later, and reports \
               what that check found rather than what was expected.",
    },
    Step {
        heading: "Download what is needed",
        // Names the two models and nothing else, because those are the two
        // things this step actually fetches today. It read "the app ... and any
        // graphics-card runtime" while fetching neither, which is the shape of
        // overstatement this surface is least allowed: the app is placed from
        // files setup already carries, and the graphics-card runtime is still
        // downloaded by the app on demand. Both lines come back here when the
        // step fetches them, and not before.
        body: "The speech model matching your choice and the punctuation model \
               are downloaded and checked against a checksum fixed in advance. A \
               file that does not match is discarded rather than used.\n\n\
               If the download is interrupted — a closed lid, a dropped \
               connection, a stopped setup — restarting continues from where it \
               stopped rather than starting again.",
    },
    Step {
        heading: "Install",
        body: "Files are placed, shortcuts are created, and SpeakEasy Mini is \
               registered so it can be removed from Settings later.\n\n\
               This build is not code-signed, so Windows SmartScreen may warn \
               about it. That is expected and will not change.",
    },
    Step {
        heading: "Choose your shortcut",
        body: "Press this key combination anywhere in Windows to start \
               dictation, then press it again to stop.\n\n\
               Setup registers the combination you choose before accepting it, \
               so a shortcut another application already owns is reported here \
               rather than failing silently the first time you use it.",
    },
    Step {
        heading: "Add your words",
        body: "Names, jargon and spellings you want protected — one per line.\n\n\
               These are applied when the transcript is finished, correcting the \
               spelling of words that were recognised. They do not change how \
               speech is recognised, so a word that is misheard will still be \
               misheard.",
    },
    Step {
        heading: "Check that dictation works",
        // Was "Setup runs the graphics-card check for each engine that
        // chose one, then a real dictation you can watch succeed." Two
        // engines, and a dictation the *user* performs; neither is true. One
        // engine now, and the check is a bundled recording rather than a live
        // microphone -- setup cannot ask someone to speak into a machine it
        // has not finished configuring, and a clip with known words is a
        // better instrument anyway.
        body: "Setup dictates a short recording that ships with SpeakEasy Mini \
               and compares what comes back, word for word, against what the \
               recording says.\n\n\
               This is the only check that proves the speech model can \
               actually hear. A model whose audio component failed to load \
               does not report an error — it writes fluent text that \
               has nothing to do with the audio, so a transcript on its own \
               proves nothing.",
    },
];

/// The check passed.
pub const SMOKE_VERIFIED: &str =
    "The speech model transcribed the recording correctly. Dictation works on this computer.";

/// Shown while the engine is loading and transcribing.
///
/// Says a wait is expected. A cold model load is seconds at best, and without
/// this the step reads as stalled.
pub const SMOKE_RUNNING: &str = "Loading the speech model and transcribing the recording. The first load \
     takes longer than later ones.";

/// The engine ran and did not hear the clip.
///
/// Names a likely cause and an action, because "verification failed" is not
/// something a user can do anything with. Retry leads, because the cheapest
/// explanation is a file still being flushed after a large download.
///
/// The last line is the honest cost of continuing. Setup does not block here --
/// owner decision, 2026-08-19 -- so it has to say what continuing means rather
/// than let the user infer that a skipped check is a passed one.
pub const SMOKE_MISMATCH: &str = "The speech model produced text, but not what the recording says. That \
     usually means its audio component did not load, which the model does not \
     report as an error.\n\n\
     Try Retry first — a first run after a large download sometimes fails on a \
     file still being written. If it fails again, the model files are likely \
     damaged despite matching their checksums; remove SpeakEasy Mini and run \
     setup again to fetch them fresh.\n\n\
     You can continue without this check. Dictation may produce text unrelated \
     to what you say.";

/// The engine never produced a transcript to compare.
///
/// Deliberately different advice from [`SMOKE_MISMATCH`]. Nothing ran, so the
/// model files are not implicated the way they are when text came back wrong,
/// and telling the user to re-download them would be a guess.
pub const SMOKE_UNAVAILABLE: &str = "The speech model did not run, so there is nothing to compare. Setup could \
     not start it, load it, or reach the end of the recording.\n\n\
     Check that this computer still has the free memory the first step \
     reported, and close other large applications before Retry. If it keeps \
     failing, the installed files are incomplete — remove SpeakEasy Mini and \
     run setup again.\n\n\
     You can continue without this check. Dictation will fail the same way \
     until this is resolved.";

/// Label for the control that runs the check again.
pub const RETRY: &str = "Retry";

/// Shown under the heading as "Step N of M".
pub fn step_position(index: usize) -> String {
    format!("Step {} of {}", index + 1, STEPS.len())
}

/// Render a byte count the way a person reads one.
///
/// Binary units with the decimal-looking names Windows itself uses, because the
/// figure sits beside disk and memory numbers the user will compare against
/// Explorer and Task Manager, and those use the same convention.
fn gigabytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{value:.1} GB")
}

/// The compatibility report, in plain words.
///
/// Everyday register — "graphics card", not "CUDA execution provider" — per the
/// UI guide's two-register rule. The stable codes stay in the log, where the
/// product-contract vocabulary belongs.
pub fn describe_machine(report: &crate::probe::MachineReport) -> String {
    use speakeasy_models::{GpuQualification, GpuRejection};

    let hardware = &report.hardware;
    let mut lines = Vec::new();

    lines.push(format!("Windows: {}", hardware.operating_system));
    lines.push(format!(
        "Processor: {} logical processors{}",
        hardware.logical_processors,
        if hardware.has_avx2 {
            ""
        } else {
            ", without AVX2"
        }
    ));
    if let Some(memory) = hardware.total_memory_bytes {
        lines.push(format!("Memory: {}", gigabytes(memory)));
    }
    if let Some(disk) = hardware.available_disk_bytes {
        lines.push(format!("Free disk space: {}", gigabytes(disk)));
    }

    // A rejected verdict carries no device, so the name comes from the raw
    // snapshot in that case: a card that is merely too old or too full still
    // has to appear, or "processor" reads as a failure to notice hardware the
    // user owns.
    lines.push(report.admissibility.device().map_or_else(
        || {
            let named = report
                .gpu
                .devices
                .first()
                .map_or_else(|| "none detected".to_owned(), |device| device.name.clone());
            format!("Graphics card: {named}")
        },
        |device| {
            format!(
                "Graphics card: {} ({} free of {})",
                device.name,
                gigabytes(device.free_vram_bytes),
                gigabytes(device.total_vram_bytes)
            )
        },
    ));

    lines.push(String::new());
    // One verdict, one line. This was a loop over two engines with a label each
    // — "Speech recognition" and "Punctuation pass" — because a machine could
    // run one on the graphics card and the other on the processor, and the
    // explanation that followed existed for exactly that case. Both are the
    // same pass now, so there is nothing left to disagree.
    let weights = report.granite_weights_bytes;
    // Never "your graphics card works". Admissible means the card is new enough
    // and has room for the weights; the engine check later is the only thing
    // that turns that into a claim.
    let verdict_words = match &report.admissibility {
        GpuQualification::Qualified { .. } => "graphics card, tested and working".to_owned(),
        // A zero means the manifest lookup found no pack, not a model that
        // occupies nothing. "0.0 GB of weights" would be a confident false
        // statement; saying less is the honest form of the same line.
        GpuQualification::Admissible { .. } if weights == 0 => {
            "graphics card, not yet tested".to_owned()
        }
        GpuQualification::Admissible { .. } => format!(
            "graphics card ({} of weights), not yet tested",
            gigabytes(weights)
        ),
        GpuQualification::Rejected(reason) => match reason {
            GpuRejection::InsufficientFreeVram { free, required } => format!(
                "processor — needs {} free graphics memory, {} available now",
                gigabytes(*required),
                gigabytes(*free)
            ),
            GpuRejection::ComputeCapabilityTooLow { .. } => {
                "processor — this graphics card is older than SpeakEasy Mini supports".to_owned()
            }
            GpuRejection::NoCudaDevice => "processor — no NVIDIA graphics card detected".to_owned(),
            GpuRejection::ProbeUnavailable(_) => {
                "processor — the NVIDIA driver did not answer".to_owned()
            }
        },
    };
    lines.push(format!("Transcription: {verdict_words}"));

    lines.join("\n")
}

/// What setup will do to this machine, in plain words.
///
/// Every refusal names the version involved and what to do instead. NSIS said
/// "use the Repair shortcut" for both refusals; that shortcut is now this same
/// binary, so the wording points at what the user actually has.
pub fn describe_install_decision(decision: &crate::install::Decision) -> String {
    use crate::install::Decision;

    match decision {
        Decision::Fresh => format!(
            "SpeakEasy Mini is not currently installed. Setup will install it for this user \
             account only, without asking for administrator rights.\n\n{}{}",
            destinations(),
            prerequisites()
        ),
        Decision::Upgrade { from } => format!(
            "SpeakEasy Mini {from} is installed and will be upgraded. Your settings, \
             personalization and any installed models are kept.\n\n{}{}",
            destinations(),
            prerequisites()
        ),
        Decision::RefuseRunning => {
            "SpeakEasy Mini is running, so its files cannot be replaced. Finish or cancel \
             dictation, then quit SpeakEasy Mini from its tray menu and run setup again."
                .to_owned()
        }
        Decision::RefuseSameVersion { installed } => format!(
            "SpeakEasy Mini {installed} is already installed — the same version this setup \
             carries. Installing again is refused, because it is not a fix for a \
             broken installation. Run this program from a command line with the \
             repair commands instead."
        ),
        Decision::RefuseDowngrade { installed } => format!(
            "A newer SpeakEasy Mini ({installed}) is already installed. Going back to an \
             older version is never done automatically, because it can leave data \
             written by the newer one behind. Use the repair commands to choose an \
             earlier version deliberately."
        ),
        // Says what was found. A user reporting this needs the actual value, and
        // it is the only thing that distinguishes a corrupt stamp from a version
        // this build cannot parse.
        Decision::RefuseUnreadableStamp { found } => format!(
            "SpeakEasy Mini appears to be installed, but its recorded version cannot be \
             read — it says \"{found}\". Setup will not overwrite an installation it \
             cannot identify. Use the repair commands, or remove SpeakEasy Mini from \
             Windows Settings first."
        ),
    }
}

/// Where an install will write, named before it writes there.
///
/// Shown rather than assumed: everything goes under the user's own profile, and
/// saying so is what makes "without asking for administrator rights" a checkable
/// claim instead of a reassurance.
fn destinations() -> String {
    // Neither line is guessed. No `APPDATA` means the shortcut cannot be
    // created; no `LOCALAPPDATA` means there is nowhere to install at all, and
    // `place` refuses on that separately. Setup says only what it knows.
    let mut lines = Vec::new();
    match crate::probe::install_root() {
        Some(program) => lines.push(format!("Program files: {}", program.display())),
        None => lines.push("Program files: cannot be determined on this account".to_owned()),
    }
    if let Some(shortcuts) = crate::shortcut::start_menu_folder() {
        lines.push(format!("Start Menu: {}", shortcuts.display()));
    }
    lines.join("\n")
}

/// Shown when placing the files failed.
///
/// Carries the underlying reason rather than a generic apology: the reasons here
/// are things a user can act on — a full disk, a locked file, a payload that
/// never arrived — and a message that hides which one it was turns a fixable
/// problem into a support conversation.
pub fn install_failed(reason: &str) -> String {
    format!(
        "SpeakEasy Mini was not installed.\n\n{reason}\n\n\
         Nothing was registered, so this computer is in the state it was in \
         before setup ran."
    )
}

/// Anything that must be present before the app can start.
///
/// Empty when there is nothing to say. Reported on the install step rather than
/// discovered after it, because a missing `WebView2` runtime makes the app fail
/// to launch with no message a user can act on — the whole reason setup checks.
fn prerequisites() -> String {
    if crate::webview2::is_present() {
        String::new()
    } else {
        format!("\n\n{WEBVIEW2_MISSING}")
    }
}

/// Shown when the bootstrapper cannot find its own directory.
pub const PAYLOAD_UNLOCATABLE: &str = "Setup could not locate its own directory, so it cannot find the files to \
     install. Nothing was changed.";

/// Shown when a recognised verb carried arguments setup could not understand.
///
/// Names the unquoted-path case explicitly rather than printing usage, because
/// that is what this actually was every time it has happened here: an install
/// root containing a space, passed by a caller that joined its arguments without
/// quoting them. Generic usage text would have been correct and useless — the
/// reader's command *looks* right, and the only thing that identifies the fault
/// is seeing where the argument was split.
pub fn arguments_not_understood(detail: &str) -> String {
    format!(
        "Setup did not run, because part of the command line was not understood:\n\n  \
         {detail}\n\n\
         This is almost always a path containing spaces that was not quoted — the \
         whole path has to arrive as a single argument. Nothing was installed, \
         removed or changed."
    )
}

/// What an ordinary uninstall keeps inside the install directory.
///
/// True again as of 2026-08-17, and it was not before. `proof/` used to be
/// spared whole, so this sentence named about a fifth of what actually survived
/// — the app's own workers and speech-runtime DLLs stayed too. The uninstall now
/// takes its own files out of that directory and leaves only what was fetched,
/// so the claim and the behaviour agree without the wording having to hedge.
pub const KEPT_WITH_GPU_RUNTIME: &str = "downloaded graphics-card runtime";

/// Shown when an uninstall is refused because the app is running.
pub const UNINSTALL_REFUSED_RUNNING: &str = "SpeakEasy Mini is running, so its files cannot be removed. Finish or cancel \
     dictation, then quit SpeakEasy Mini from its tray menu and try again. Nothing \
     has been removed.";

/// Shown before an interactive uninstall that keeps everything optional.
pub fn uninstall_keeps_user_data() -> String {
    // Listed from the same source the checkboxes will render from, so the two
    // cannot end up describing different sets.
    let kept = crate::uninstall::Removable::ALL
        .iter()
        .map(|item| format!("  - {}", item.label()))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "SpeakEasy Mini will be removed.\n\n\
         These are kept:\n{kept}\n\n\
         Choosing which of them to remove is not built yet, so this removes only \
         the program itself. Run with --remove-all to remove everything."
    )
}

/// What an uninstall actually did.
///
/// Reports removed and kept separately, and never summarises to "done": a user
/// who chose to keep their history needs to see that it was kept, and a partial
/// failure is the case where a summary would be a lie.
pub fn describe_uninstall(outcome: &crate::uninstall::Outcome) -> String {
    let mut lines = Vec::new();
    if !outcome.failed.is_empty() {
        lines.push("SpeakEasy Mini was only partly removed.".to_owned());
    } else if outcome.removed.is_empty() {
        // Nothing was found to remove, which is not the same as having removed
        // it. Announcing success here is what let an uninstall pointed at the
        // wrong directory report a job it had not done, and it is also the
        // ordinary result of running an uninstall twice — a user who sees
        // "removed" both times learns the message means nothing.
        lines.push("SpeakEasy Mini does not appear to be installed on this computer.".to_owned());
    } else {
        lines.push("SpeakEasy Mini has been removed.".to_owned());
    }
    if !outcome.removed.is_empty() {
        lines.push(String::new());
        lines.push(format!("Removed: {}", outcome.removed.join(", ")));
    }
    if !outcome.kept.is_empty() {
        lines.push(format!("Kept: {}", outcome.kept.join(", ")));
    }
    if !outcome.failed.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Could not be removed:\n  {}",
            outcome.failed.join("\n  ")
        ));
    }
    if !outcome.left_behind.is_empty() {
        // Named rather than summarised, and separated from the failures above,
        // because it is neither a failure nor something to hide: the product is
        // gone and one file of setup's own is not. Saying which file and where
        // is what lets someone delete it themselves; saying nothing would leave
        // them to find it later and conclude the uninstall did not work.
        lines.push(String::new());
        lines.push(format!(
            "Setup could not remove its own program file while it was running, so \
             this is still on disk and can be deleted:\n  {}",
            outcome.left_behind.join("\n  ")
        ));
    }
    lines.join("\n")
}

/// Shown when the runtime the app cannot start without is missing.
///
/// Names the runtime and where to get it, and does not offer to fetch it: the
/// Evergreen Bootstrapper is served from a redirect whose bytes change by
/// design and therefore cannot be pinned, and this project downloads nothing it
/// cannot pin. Saying so is better than a download that skips verification.
pub const WEBVIEW2_MISSING: &str = "Microsoft Edge WebView2 Runtime is not installed on this computer. \
     SpeakEasy Mini cannot start without it.\n\n\
     Install it from Microsoft's website (search for \"WebView2 Runtime\"), then \
     run setup again. SpeakEasy Mini does not download it, because Microsoft's \
     installer is served from a link whose contents change and cannot be \
     verified against a fixed checksum the way everything else here is.";

/// Shown in place of a step's controls while its stage is unbuilt.
///
/// Deliberately explicit. A step that renders empty looks like a step whose
/// controls failed to appear, and this project's convention is that a surface
/// says what actually happened.
pub const STEP_NOT_BUILT: &str = "This step's controls are not built yet. Navigation works, so the order and \
     wording can be reviewed now.";

// ---------------------------------------------------------------------------
// The download step.
// ---------------------------------------------------------------------------

/// What each fetched artifact is called, in the everyday register.
///
/// Not the pack id. A user reading "nemotron-3.5-streaming-en-cpu" learns
/// nothing they can act on; the log keeps the id, which is where the
/// product-contract vocabulary belongs.
pub const ARTIFACT_STREAMING: &str = "Speech recognition model";
pub const ARTIFACT_GRANITE: &str = "Punctuation model";

/// Shown when the pinned catalog compiled into this binary will not parse.
pub const CATALOG_UNAVAILABLE: &str = "Setup's list of verified downloads could not be read, so nothing was fetched. \
     This is a fault in this copy of setup rather than anything on this computer.";

/// Shown when the app's own data directory cannot be located.
/// No `LOCALAPPDATA`, so setup does not know where the per-user program
/// directory is.
///
/// Named rather than guessed. The guess used to be `C:\`, which would have
/// unpacked the payload into the drive root and left uninstall walking it.
pub const INSTALL_ROOT_UNLOCATABLE: &str = "Setup could not work out where to put the program on this computer, because \
     this account's local application-data folder is not set. Nothing was \
     installed. Sign in as a normal user and run setup again.";

pub const DATA_ROOT_UNLOCATABLE: &str = "Setup could not find where SpeakEasy Mini keeps its models on this computer, so \
     nothing was downloaded. Nothing was changed.";

/// Shown when the catalog has no eligible pack for what this machine chose.
pub fn pack_unavailable(label: &str, detail: &str) -> String {
    format!("There is no verified download for the {label} on this computer.\n\n{detail}")
}

/// Shown when a pack exists but carries no address to fetch it from.
///
/// Distinct from [`pack_unavailable`] on purpose: this one is a catalog that
/// names an artifact without saying where it lives, which is a fault in setup
/// rather than an unsupported machine, and the two need different answers from
/// whoever reads them.
pub fn pack_not_downloadable(id: &str) -> String {
    format!(
        "The catalog entry for \"{id}\" does not say where to download it from, so \
         setup could not fetch it."
    )
}

/// Shown when the transfer itself failed.
///
/// Says what is still on disk, because that is the part a user would otherwise
/// assume the worst about. Bytes already fetched are kept and reused, so a
/// second attempt continues rather than starting again — and saying so is what
/// stops someone deleting the partial files to "start clean" and paying for the
/// whole transfer twice.
pub fn download_failed(label: &str, detail: &str) -> String {
    format!(
        "The {label} was not downloaded.\n\n{detail}\n\n\
         What was already fetched has been kept and verified as far as it went. \
         Running setup again continues from there rather than starting over."
    )
}

/// Shown when a downloaded artifact could not be unpacked or verified.
///
/// Deliberately not folded into [`download_failed`]: the bytes arrived and
/// matched their checksum, and it is what came after that failed — usually disk
/// space, since an archive expands well beyond its transfer size. A message that
/// blamed the download would send someone to check their network.
pub fn install_of_artifact_failed(label: &str, detail: &str) -> String {
    format!(
        "The {label} was downloaded and verified, but could not be unpacked.\n\n{detail}\n\n\
         The downloaded copy has been kept, so this does not need fetching again."
    )
}

/// Render a byte count for a progress line.
///
/// Megabytes below a gigabyte, because a 453 MB download reported as "0.4 GB"
/// moves its first visible digit once in five minutes and reads as stalled.
fn transfer_size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0)
    } else {
        format!("{megabytes:.0} MB")
    }
}

/// What the download step says before anything starts.
pub fn describe_download_plan(labels: &[&str], total_bytes: u64) -> String {
    let mut lines = vec![format!(
        "Setup will download {} in total:",
        transfer_size(total_bytes)
    )];
    lines.push(String::new());
    for label in labels {
        lines.push(format!("  {label}"));
    }
    lines.push(String::new());
    lines.push(
        "Each one is checked against a checksum fixed in advance. A file that does \
         not match is discarded rather than used."
            .to_owned(),
    );
    lines.join("\n")
}

/// What the download step says when everything is already on this computer.
///
/// Its own message rather than a progress bar that fills instantly, because
/// those are not the same claim: nothing was transferred, and the reason is that
/// the files are present and their digests still match.
pub const DOWNLOAD_ALREADY_PRESENT: &str = "Everything needed is already on this computer and still matches its checksum, \
     so there is nothing to download.";

/// Which of the three things the download step does is happening now.
///
/// Named rather than a pair of booleans because two of the three look identical
/// from the outside — a bar that is not moving — and the whole point of the
/// distinction is to say which one it is. A boolean pair also admits a fourth
/// state that cannot occur.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Bytes are arriving.
    Transferring,
    /// Nothing is arriving; a complete file is being re-digested.
    Verifying,
    /// Unpacking and per-file verification, after the transfer.
    Installing,
}

/// The live progress line.
///
/// Reports the phase as well as the bytes. Extraction and verification of a
/// multi-hundred-megabyte archive takes long enough that a bar which stops
/// moving reads as a hang, and the only person who can tell us it did is the one
/// who gave up and killed setup.
pub fn describe_download_progress(
    labels: &[&str],
    current: usize,
    done_bytes: u64,
    total_bytes: u64,
    phase: Phase,
) -> String {
    let name = labels.get(current).copied().unwrap_or(ARTIFACT_STREAMING);
    let position = format!("{} of {}", (current + 1).min(labels.len()), labels.len());
    if phase == Phase::Installing {
        return format!(
            "Unpacking and checking the {name} ({position}).\n\n\
             This takes a while and the bar does not move while it happens — the \
             archive is larger unpacked than it was to download, and every file in \
             it is checked."
        );
    }
    if phase == Phase::Verifying {
        // Says what is actually happening. This step used to report "Downloading
        // — 0 MB transferred" for the twenty-four seconds it takes to digest a
        // 2.3 GB archive that was already on disk, which is both untrue and
        // exactly what a stalled download looks like.
        return format!(
            "Checking the {name} ({position}) against its checksum.\n\n\
             It was already downloaded, so nothing is being transferred — this is \
             re-reading what is on disk to confirm it is intact."
        );
    }
    format!(
        "Downloading the {name} ({position}).\n\n{} of {} transferred.\n\n\
         If this is interrupted — a closed lid, a dropped connection, setup \
         stopped — what has arrived is kept and the next attempt continues from \
         here.",
        transfer_size(done_bytes),
        transfer_size(total_bytes)
    )
}

/// Shown when everything in the plan arrived and was verified.
pub fn describe_download_complete(labels: &[&str], total_bytes: u64) -> String {
    format!(
        "{} downloaded and verified: {}.\n\n\
         Nothing here has been proven to work yet — it has been fetched and \
         checked, which is not the same thing. The last step runs it.",
        transfer_size(total_bytes),
        labels.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use speakeasy_models::{
        ComputeCapability, CudaDevice, GpuQualification, GpuRejection, GpuSnapshot,
        HardwareSnapshot,
    };

    use super::*;
    use crate::probe::MachineReport;

    fn hardware() -> HardwareSnapshot {
        HardwareSnapshot {
            operating_system: "Windows 11 Pro".to_owned(),
            operating_system_build: None,
            architecture: "x86_64".to_owned(),
            physical_cores: Some(8),
            logical_processors: 16,
            has_avx2: true,
            total_memory_bytes: Some(32 * 1024 * 1024 * 1024),
            available_disk_bytes: Some(300 * 1024 * 1024 * 1024),
            detected_adapters: Vec::new(),
            limitations: Vec::new(),
        }
    }

    fn card() -> CudaDevice {
        CudaDevice {
            name: "NVIDIA GeForce RTX 4070 Laptop GPU".to_owned(),
            compute_capability: ComputeCapability { major: 8, minor: 9 },
            total_vram_bytes: 8 * 1024 * 1024 * 1024,
            free_vram_bytes: 4 * 1024 * 1024 * 1024,
        }
    }

    #[test]
    fn an_admissible_engine_is_never_described_as_working() {
        // The single most important line in this file. `Admissible` means the
        // card is new enough and has room; only an execution test earns "works".
        let report = MachineReport {
            hardware: hardware(),
            gpu: GpuSnapshot {
                driver_version: None,
                devices: vec![card()],
                unavailable: None,
            },
            admissibility: GpuQualification::Admissible { device: card() },
            granite_weights_bytes: 2 * 1024 * 1024 * 1024,
        };

        let described = describe_machine(&report);

        assert!(described.contains("not yet tested"));
        assert!(!described.contains("tested and working"));
    }

    #[test]
    fn an_unknown_weights_figure_is_omitted_rather_than_reported_as_zero() {
        // Reported "0.0 GB of weights" for a multi-gigabyte model on 2026-08-15,
        // because the manifest lookup filtered for a pack that does not exist.
        // The lookup is fixed; this keeps the copy from ever stating a size it
        // does not have.
        let report = MachineReport {
            hardware: hardware(),
            gpu: GpuSnapshot {
                driver_version: None,
                devices: vec![card()],
                unavailable: None,
            },
            admissibility: GpuQualification::Admissible { device: card() },
            granite_weights_bytes: 0,
        };

        let described = describe_machine(&report);

        // The whole phrase, not just "0.0 GB": a free-disk figure of 300.0 GB
        // contains that substring, and asserting the loose form failed on
        // correct output — the assertion has to name the claim being forbidden.
        assert!(!described.contains("0.0 GB of weights"));
        assert!(described.contains("graphics card, not yet tested"));
    }

    #[test]
    fn a_card_too_small_is_reported_as_a_size_problem_and_still_named() {
        // This was `engines_that_disagree_are_explained_rather_than_warned_about`,
        // which pinned that one engine on the card and the other on the
        // processor read as supported rather than broken. There is one engine,
        // so what survives is the half that was never about the split: a
        // rejection has to say *why* in numbers, and must still name the card,
        // so falling back reads as a decision about hardware the user owns
        // rather than a failure to detect it.
        let report = MachineReport {
            hardware: hardware(),
            gpu: GpuSnapshot {
                driver_version: None,
                devices: vec![card()],
                unavailable: None,
            },
            admissibility: GpuQualification::Rejected(GpuRejection::InsufficientFreeVram {
                free: 4 * 1024 * 1024 * 1024,
                required: 6 * 1024 * 1024 * 1024,
            }),
            granite_weights_bytes: 6 * 1024 * 1024 * 1024,
        };

        let described = describe_machine(&report);

        assert!(described.contains("Transcription: processor"));
        assert!(described.contains("needs 6.0 GB free graphics memory"));
        assert!(described.contains("4.0 GB available now"));
        assert!(described.contains("NVIDIA GeForce RTX 4070 Laptop GPU"));
    }
}
