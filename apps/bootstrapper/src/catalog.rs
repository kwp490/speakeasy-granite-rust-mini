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

use speakeasy_models::GpuPayloadRejection;

/// The wizard's window title. Not the product tagline: this is what appears in
/// the taskbar while setup runs.
pub const WINDOW_TITLE: &str = "SpeakEasy Mini setup";

/// Navigation. Short verbs, per the UI guide's preference.
pub const BACK: &str = "Back";
pub const NEXT: &str = "Next";
pub const CANCEL: &str = "Cancel";

/// Shown in place of `NEXT` on the last page.
pub const FINISH: &str = "Finish";

/// How much weight a line of copy carries, so the wizard can colour it.
///
/// A *copy* attribute rather than a colour, and it lives here for the same
/// reason every other word does: which sentence on a page is the one a reader
/// must not skip is a decision about the writing, and it has to be reviewable
/// beside the writing. `wizard.rs` owns the mapping to actual pixels.
///
/// Colour is never the only signal — `UI-GUIDE.md`'s rule, and the reason each
/// tone below is also carried by the words. A reader who cannot see the colour
/// loses emphasis, never information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    /// The window's ordinary text colour.
    Plain,
    /// The one line on this page worth reading first.
    Accent,
    /// Something the reader has to accept or act on.
    Warning,
    /// Something worked, and it was not certain that it would.
    Good,
}

/// The steps, in order.
///
/// One entry per page the wizard specifies, so navigation is real before any page
/// has content.
///
/// Three fields rather than two, and the split is the whole point of the 2026-08-20
/// rewrite: nobody reads an installer. `heading` asks the page's question,
/// `key` is the single line that must survive being the only line read, and
/// `body` is at most two short sentences of context. The pages used to carry
/// four-sentence paragraphs of correct, careful prose that a user scrolls past
/// on the way to Next — which meant the honesty obligations in them were being
/// met on paper and not in fact.
pub struct Step {
    pub heading: &'static str,
    /// The line the page is about. Coloured per [`Self::key_tone`].
    pub key: &'static str,
    pub key_tone: Tone,
    pub body: &'static str,
}

pub const STEPS: &[Step] = &[
    Step {
        heading: "Can this computer run it?",
        key: "Nothing is installed or downloaded on this page.",
        key_tone: Tone::Accent,
        body: "Setup reads the processor, memory, free disk space and graphics card. \
               What it found is below.",
    },
    Step {
        heading: "Where should it run?",
        // Was two paragraphs about two engines and an override of each; neither
        // was true after the fork. One engine now, and one honest override.
        key: "The speech model is the same file either way. Only the engine changes.",
        key_tone: Tone::Accent,
        body: "The processor is always available. The graphics card is offered only when \
               there is a graphics-card engine to install — a setting cannot make one exist.",
    },
    Step {
        heading: "Download the models",
        // Names only what this step fetches. It read "the app ... and any
        // graphics-card runtime" while fetching neither.
        key: "Every file is checked against a checksum fixed in advance. A file that does \
              not match is discarded, never used.",
        key_tone: Tone::Accent,
        body: "An interrupted download — a closed lid, a dropped connection, a stopped \
               setup — continues from where it stopped rather than starting again.",
    },
    Step {
        heading: "Install",
        key: "This build is not code-signed, so Windows SmartScreen may warn about it. \
              That is expected and will not change.",
        key_tone: Tone::Warning,
        body: "Files are placed, shortcuts created, and SpeakEasy Mini registered so \
               Windows Settings can remove it later.",
    },
    Step {
        heading: "Pick your shortcut",
        key: "Press it anywhere in Windows to start dictation. Press it again to stop.",
        key_tone: Tone::Accent,
        body: "Setup registers your choice to check it, so a shortcut another program owns \
               is reported here instead of doing nothing the first time you press it.",
    },
    Step {
        heading: "Add your words",
        // The box is the example now. It arrives holding `DEFAULT_VOCABULARY`,
        // so the comma format is demonstrated by the thing the user is about to
        // edit rather than by a specimen list above it -- and a made-up example
        // sitting over a box full of real words would read as the instruction
        // for a box that was empty. Before 2026-08-27 the box started empty and
        // this line carried "Kenneth, Anthropic, Granite"; before 2026-08-20 the
        // box took one word per line, which was more typing to no end.
        key: "Common tools are filled in already. Add your own, or clear the box.",
        key_tone: Tone::Accent,
        // "Spelling and spacing" rather than just spelling, since 2026-08-27: a
        // compound name also gets a rule for the two-word form a recogniser
        // writes, so `Logic Monitor` becomes `LogicMonitor`. The second
        // sentence is unchanged and still the honest one -- these act on the
        // finished transcript, never on what was heard.
        body: "Names, jargon and spellings to protect. They fix spelling and spacing in the \
               finished transcript and do not change what the model hears, so a misheard word \
               stays misheard.\n\
               Optional — Settings has the same list.",
    },
    Step {
        heading: "What should it keep?",
        // The retention default is a privacy promise, and it is stated as one
        // rather than as a checkbox nobody reads.
        key: "Neither of these ever leaves this computer.",
        key_tone: Tone::Accent,
        body: "Left unticked, transcripts are never written to disk at all — not written \
               and then deleted.\n\
               Both are in Settings at any time.",
    },
    Step {
        heading: "Does dictation actually work?",
        // Was a paragraph about "the graphics-card check for each engine that
        // chose one, then a real dictation you can watch succeed". One engine
        // now, and a bundled clip rather than a live microphone.
        key: "This is the only check that proves the speech model can hear.",
        key_tone: Tone::Accent,
        body: "Setup transcribes a recording that ships with the app and compares it word \
               for word.\n\
               A model whose audio component failed to load reports no error — it writes \
               fluent text with nothing to do with the audio.",
    },
];

/// The check passed, and which provider it proved.
///
/// Says the provider because this is the moment setup writes it down, and the
/// claim being recorded about someone's machine should be visible to them. Never
/// "ready on the graphics card" for a run that happened on the processor: that
/// sentence, generated from an intention rather than a result, is the whole
/// defect this reporting exists to close.
///
/// The processor line is deliberately not an apology. A processor installation
/// running on the processor is a complete installation working exactly as
/// installed, and the only machine that needs more than one sentence about it is
/// one whose card *could* have been used — which is the provider page's job to
/// have said, before anything was installed.
///
/// `evidence` is a stable code, and it is shown rather than translated. It only
/// appears where the answer is "processor", it names which of the three gates
/// closed, and it is the one thing a support reader needs that no prose here can
/// carry — the alternative is seven sentences for six conditions no user of a
/// CPU-only release will ever see.
pub fn smoke_verified(graphics_card: bool, evidence: &str) -> String {
    if graphics_card {
        "Dictation works, on the graphics card. The model transcribed the recording word for \
         word, and setup confirmed the engine is holding the card."
            .to_owned()
    } else {
        format!(
            "Dictation works, on the processor. The model transcribed the recording word for \
             word.\n\n\
             Recorded as a processor installation ({evidence}), which is what SpeakEasy Mini \
             will report from now on."
        )
    }
}

/// Shown while the engine is loading and transcribing.
///
/// Says a wait is expected. A cold model load is seconds at best, and without
/// this the step reads as stalled.
pub const SMOKE_RUNNING: &str =
    "Loading the speech model and transcribing. The first load takes the longest.";

/// The engine ran and did not hear the clip.
///
/// Names a likely cause and an action, because "verification failed" is not
/// something a user can do anything with. Retry leads, because the cheapest
/// explanation is a file still being flushed after a large download.
///
/// The last line is the honest cost of continuing. Setup does not block here --
/// owner decision, 2026-08-19 -- so it has to say what continuing means rather
/// than let the user infer that a skipped check is a passed one.
pub const SMOKE_MISMATCH: &str = "The model produced text, but not what the recording says. Its audio component \
     most likely did not load, which the model does not report as an error.\n\n\
     Press Retry — a first run after a large download can fail on a file still \
     being written. If it fails again, remove SpeakEasy Mini and run setup again.\n\n\
     You can continue. Dictation may then produce text unrelated to what you say.";

/// The engine never produced a transcript to compare.
///
/// Deliberately different advice from [`SMOKE_MISMATCH`]. Nothing ran, so the
/// model files are not implicated the way they are when text came back wrong,
/// and telling the user to re-download them would be a guess.
pub const SMOKE_UNAVAILABLE: &str = "The model did not run, so there is nothing to compare. Setup could not start \
     it, load it, or reach the end of the recording.\n\n\
     Close other large applications and press Retry. If it keeps failing, the \
     installed files are incomplete — remove SpeakEasy Mini and run setup again.\n\n\
     You can continue. Dictation will fail the same way until this is fixed.";

/// Label for the control that runs the check again.
pub const RETRY: &str = "Retry";

/// The two configurations setup can install.
///
/// "Graphics card" and "processor", not "CUDA" and "CPU": the everyday register
/// the UI guide asks for on a surface a user reads. The stable codes go in the
/// seed file and the app's log.
pub const PROVIDER_GRAPHICS_CARD: &str = "Use the graphics card";
pub const PROVIDER_PROCESSOR: &str = "Use the processor";

/// Why the provider step offers what it offers.
///
/// Four states, and the one that matters is the last: a machine with a capable
/// card, where the graphics-card option is still unavailable. Saying nothing
/// there would read as setup not having looked, and saying "your card is not
/// supported" would be false. The honest answer is that the part that runs on
/// the card is not published yet, which is a fact about this release.
///
/// The tone is returned with the words, because the middle case is the only one
/// where the reader is being told they cannot have the faster option.
pub fn describe_provider_options(
    card_is_capable: bool,
    // `None` means the graphics-card configuration is installable. An `Option`
    // rather than a `Result`, because a caller holding a `Result<(), _>` has to
    // map the unit away to borrow the error and the map reads as though it did
    // something.
    rejection: Option<&GpuPayloadRejection>,
) -> (String, Tone) {
    match (card_is_capable, rejection) {
        (true, None) => (
            "This graphics card can run SpeakEasy Mini, and the graphics-card \
             configuration is available. It is faster; the processor uses no graphics memory."
                .to_owned(),
            Tone::Good,
        ),
        // The case that matters, and the one the option is disabled for. Names
        // *which* half is missing, because the three are different things to do
        // about — and because saying only "not available" is what let this page
        // look like setup had not examined the card.
        (true, Some(rejection)) => (
            format!(
                "This graphics card meets the requirements, and {} So the graphics-card option \
                 is unavailable: SpeakEasy Mini will run on the processor, and will say so \
                 rather than appear to use the card.",
                describe_gpu_rejection(rejection)
            ),
            Tone::Warning,
        ),
        (false, _) => (
            "No graphics card here clears the requirements — the first page says which one. \
             The processor configuration is a complete install, not a reduced one."
                .to_owned(),
            Tone::Plain,
        ),
    }
}

/// Why the graphics-card configuration cannot be installed, as half a sentence.
///
/// Three conditions, three instructions. The runtime-files case names the files:
/// a CUDA build whose libraries are not beside it does not run slower, it fails
/// to start, and the error Windows gives for that names nothing anyone can act
/// on.
fn describe_gpu_rejection(rejection: &GpuPayloadRejection) -> String {
    match rejection {
        GpuPayloadRejection::WorkerNotPublished => {
            "this version of SpeakEasy Mini does not include a graphics-card engine to install."
                .to_owned()
        }
        GpuPayloadRejection::WorkerNotInstalled => {
            "the graphics-card engine is published but is not part of this installation.".to_owned()
        }
        GpuPayloadRejection::RuntimeFilesMissing(files) => format!(
            "the graphics-card engine is here but the libraries it loads are not: {}.",
            files.join(", ")
        ),
    }
}

/// The shortcuts setup offers.
///
/// Spelled the way Windows writes them, because that is how the user will read
/// them back in Settings and on a keyboard.
pub const SHORTCUT_CTRL_ALT_P: &str = "Ctrl + Alt + P   (recommended)";
pub const SHORTCUT_CTRL_ALT_D: &str = "Ctrl + Alt + D";
pub const SHORTCUT_CTRL_SHIFT_SPACE: &str = "Ctrl + Shift + Space";

/// The chosen shortcut is free, proved by taking it and letting it go.
pub fn shortcut_available(binding: &str) -> String {
    format!("{binding} is free. Setup registered it to check, then released it.")
}

/// Another program owns it.
///
/// Names the fact and the way out, and does not guess at which program: Windows
/// does not say who holds a shortcut, and inventing a likely culprit would send
/// the reader looking in the wrong place.
pub fn shortcut_taken(binding: &str) -> String {
    format!(
        "{binding} is already in use by another program, and Windows does not say which. \
         Choose one of the others above."
    )
}

/// No shortcut is selected, which the control should make impossible.
pub const SHORTCUT_UNKNOWN: &str =
    "No shortcut is selected, so setup cannot check whether it is free. Choose one above.";

/// The vocabulary the words page starts with, already in the box.
///
/// Every one of these is a name a speech recogniser gets wrong in a way the
/// finishing pass can fix, and they were chosen by measurement rather than
/// guessed: an unbiased pass over a recorded clip containing all of them wrote
/// `logic monitor`, `Pager Duty` and `Jira`, and the entries below plus their
/// spaced companions correct every one. `CLAUDE.md` carries the table.
///
/// # Why a default at all, and why it is only a default
///
/// Nobody reads an installer, and a page that opens empty is a page most people
/// click past — so an empty box means the feature reaches only the users who
/// already knew they wanted it. Starting filled inverts that: the common tools
/// work out of the box, and the box is still an ordinary editable control that
/// the user can add to or clear entirely.
///
/// It is **a starting value and never a policy**, which is the same contract
/// every other seed here carries. The list is written to
/// `install-vocabulary.txt` exactly as the box reads at Next, so clearing it
/// installs nothing; and the app deletes the seed after applying it, so a term
/// the user removes in Settings afterwards stays removed.
///
/// Ordered longest-compound first so the read-back on the page shows the shape
/// of the list — commas, mixed case — before it scrolls.
pub const DEFAULT_VOCABULARY: &str = "LogicMonitor, PagerDuty, ServiceNow, Atlassian, \
                                      Anthropic, OpenAI, ChatGPT, Claude, Splunk, JIRA, \
                                      VLAN, HUIT, Hellen";

/// What the words page says back about an empty box.
///
/// Its own sentence rather than "0 words": an empty list is a perfectly good
/// answer on this page, and a zero beside an empty box reads as a rejection.
pub const WORDS_NONE: &str =
    "No words yet. Settings has the same list if you would rather add them later.";

/// What the words page says back once something is typed.
///
/// Echoes the words as well as the count, because the count alone cannot show
/// that a missing comma joined two of them — "1 word: Kenneth Perry" is the
/// only form in which that mistake is visible before it is installed.
pub fn words_counted(words: &[String]) -> String {
    format!(
        "{} word{} will be added: {}",
        words.len(),
        if words.len() == 1 { "" } else { "s" },
        words.join(", ")
    )
}

/// The two questions about what `SpeakEasy Mini` keeps.
pub const KEEP_TRANSCRIPTS: &str = "Keep my transcripts after I close SpeakEasy Mini";
pub const DISK_LOGGING: &str =
    "Write a diagnostic log (error codes and counters, never what you said)";

/// Some answers could not be written down.
///
/// Names the count rather than the file names: the names mean nothing to a
/// reader, and what they need to know is that the install is fine and these
/// particular choices did not stick.
pub fn seeds_not_recorded(failed: &[&str]) -> String {
    format!(
        "Installed, but {} of your answers could not be saved for the first start, so it \
         begins with defaults. All of them are in Settings.",
        failed.len()
    )
}

/// The installed-configuration record could not be written.
///
/// Not fatal, and its own message rather than folded into
/// [`seeds_not_recorded`]: what is lost is not a setting the user can redo in
/// Settings, it is the app's ability to tell an expected processor run from a
/// broken graphics-card one. Saying "an answer was not saved" would understate
/// that and point at the wrong place to look.
pub const PROVIDER_NOT_RECORDED: &str = "Dictation works, but setup could not record which configuration it installed. \
     SpeakEasy Mini will report its provider as unrecorded; nothing else is affected.";

/// The `--verify-provider` verb re-proved the installation, and recorded it.
///
/// A short key-value line rather than a sentence, because the only readers are a
/// script and whoever is reading its transcript afterwards. `device` is the
/// provider that was proved and written to `install-provider.txt`; `evidence` is
/// the stable code naming which of the three gates decided it, and it is present
/// on a graphics-card result too -- `cuda_context_held` is as much a fact worth
/// recording as the reason a processor result stayed one.
pub fn provider_recorded(device: &str, evidence: &str) -> String {
    format!("provider_recorded device={device} evidence={evidence}")
}

/// The verb ran the engine and it did not hear the clip.
///
/// Separate from [`SMOKE_MISMATCH`] because the audience is. That one is a
/// wizard page with a Retry button beside it; this is a script's stderr, and
/// telling a script to press Retry tells nobody anything.
///
/// Nothing is recorded. The engine demonstrably did not work, and a provider
/// written from a run that produced the wrong words would be a claim about a
/// configuration that just failed.
pub const PROVIDER_VERIFY_MISMATCH: &str = "The engine ran and did not transcribe the recording, so no configuration was recorded. The installed files are suspect: remove SpeakEasy Mini and run setup again.";

/// The verb could not run the engine at all.
pub const PROVIDER_VERIFY_UNAVAILABLE: &str = "The engine did not run, so there is nothing to record. Check that SpeakEasy Mini is not running and that its models are installed, then try again.";

/// The engine could not start and the graphics-card libraries are missing.
///
/// The one case where the reason a run failed is knowable and specific, so it is
/// said instead of the general advice above. A CUDA worker with a library
/// missing does not run slower -- Windows cannot resolve its imports, refuses to
/// start the process, and names no file in the error. Naming them *is* the
/// instruction, which is why `GpuPayloadRejection::RuntimeFilesMissing` carries
/// them this far.
/// The remedy is **setup**, and it was a developer script until 2026-08-26.
/// This said "re-run scripts/Enable-GraniteCuda.ps1", which was the only way to
/// get a CUDA worker onto a machine before one was published — and which stopped
/// existing when setup learned to fetch it. Naming a script that is not there
/// is worse than naming nothing: it is an instruction that cannot be followed,
/// in the one message whose whole job is to be actionable.
pub fn provider_verify_runtime_missing(files: &[String]) -> String {
    format!(
        "The graphics-card engine cannot start: {} is not beside the worker. Run setup again and \
         choose the graphics card — it fetches these and puts them back. Choosing the processor \
         works too, and is slower.",
        files.join(", ")
    )
}

/// Setup finished and there is no app where it recorded one.
pub const APP_NOT_FOUND: &str = "SpeakEasy Mini was not started: its program file is not where setup recorded it. \
     Nothing else on this computer is wrong. Run setup again.";

/// Setup finished and Windows refused to start the app.
pub fn app_did_not_start(detail: &str) -> String {
    format!("Installed, but Windows would not start it. Start it from the Start menu.\n\n{detail}")
}

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
///
/// Returns the tone with the words: five of these seven outcomes are a refusal,
/// and a refusal that looks like the paragraph beside it on the previous page is
/// a refusal the reader walks past.
pub fn describe_install_decision(decision: &crate::install::Decision) -> (String, Tone) {
    use crate::install::Decision;

    match decision {
        Decision::Fresh => (
            format!(
                "Not currently installed. Setup installs it for this user account only, \
                 without administrator rights.\n\n{}{}",
                destinations(),
                prerequisites()
            ),
            Tone::Plain,
        ),
        Decision::Upgrade { from } => (
            format!(
                "SpeakEasy Mini {from} is installed and will be upgraded. Settings, \
                 personalization and installed models are kept.\n\n{}{}",
                destinations(),
                prerequisites()
            ),
            Tone::Plain,
        ),
        Decision::RefuseRunning => (
            "SpeakEasy Mini is running, so its files cannot be replaced. Quit it from its \
             tray menu, then run setup again."
                .to_owned(),
            Tone::Warning,
        ),
        Decision::RefuseSameVersion { installed } => (
            format!(
                "SpeakEasy Mini {installed} is already installed — the same version this \
                 setup carries, so installing again is refused. It is not a fix for a broken \
                 installation; use the repair commands from a command line for that."
            ),
            Tone::Warning,
        ),
        Decision::RefuseDowngrade { installed } => (
            format!(
                "A newer SpeakEasy Mini ({installed}) is already installed. Going back is \
                 never automatic — it can leave data written by the newer one behind. Use the \
                 repair commands to choose an earlier version deliberately."
            ),
            Tone::Warning,
        ),
        // Says what was found. A user reporting this needs the actual value, and
        // it is the only thing that distinguishes a corrupt stamp from a version
        // this build cannot parse.
        Decision::RefuseUnreadableStamp { found } => (
            format!(
                "SpeakEasy Mini appears to be installed, but its recorded version reads \
                 \"{found}\". Setup will not overwrite an installation it cannot identify. \
                 Remove it from Windows Settings first, or use the repair commands."
            ),
            Tone::Warning,
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
        "Not installed.\n\n{reason}\n\n\
         Nothing was registered, so this computer is as it was before setup ran."
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

/// Shown when the files setup should install cannot be read out of setup.
///
/// Three sentences for four causes, because only one of them is something the
/// reader can do anything about — and it is by far the most likely. A setup
/// file downloaded over a dropped connection is still a runnable program, since
/// the payload lives past the end of the image where Windows' loader never
/// looks; see `payload.rs`. So the instruction leads with downloading it again
/// and the rest is there to stop that advice reading as a guess.
pub fn describe_payload_failure(failure: &crate::payload::ArchiveError) -> String {
    use crate::payload::ArchiveError;

    match failure {
        ArchiveError::Damaged => {
            "This setup file is incomplete or damaged, so the files it installs cannot be \
             read. Nothing was changed.\n\n\
             Download it again. An interrupted download produces exactly this — the file \
             still runs, because the missing part sits past the end of the program."
                .to_owned()
        }
        ArchiveError::UnknownFormat { found } => format!(
            "This setup file was packed by a newer version than it carries (format \
             {found}). Nothing was changed.\n\n\
             Download it again from the release you meant to install."
        ),
        ArchiveError::UnsafePath { path } => format!(
            "This setup file asks to write outside the folder it installs into, so setup \
             stopped. Nothing was changed.\n\n\
             The file is: {path}"
        ),
        ArchiveError::Io { detail } => {
            format!("Setup could not read the files it installs. Nothing was changed.\n\n{detail}")
        }
    }
}

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

// `KEPT_WITH_GPU_RUNTIME` stood here and said "downloaded graphics-card
// runtime". Deleted 2026-08-21 with the behaviour it described: an uninstall
// keeps nothing inside the install directory now, so there is nothing for this
// sentence to be true about. It had already been wrong once in the other
// direction — `proof/` used to be spared whole, and the sentence named about a
// fifth of what actually survived.

/// Shown when an uninstall is refused because the app is running.
pub const UNINSTALL_REFUSED_RUNNING: &str = "SpeakEasy Mini is running, so its files cannot be removed. Finish or cancel \
     dictation, then quit SpeakEasy Mini from its tray menu and try again. Nothing \
     has been removed.";

/// The uninstall page's title bar, heading and key line.
///
/// Its own title rather than `WINDOW_TITLE`: the same executable is the setup
/// wizard and the uninstaller, and a window labelled "`SpeakEasy Mini setup`"
/// asking to delete everything is the kind of mismatch someone dismisses
/// without reading.
pub const UNINSTALL_WINDOW_TITLE: &str = "Remove SpeakEasy Mini";
pub const UNINSTALL_HEADING: &str = "Remove SpeakEasy Mini?";

/// What is not optional, above the checkboxes.
///
/// The program always goes; only user data is a choice. Saying so before the
/// list is what makes the list mean "and also", rather than reading as the
/// whole of what an uninstall does.
pub const UNINSTALL_INTRO: &str = "The program will be removed. Also remove:";

/// The line the page is really asking about.
///
/// [`Tone::Warning`], and last, immediately above the buttons: everything above
/// it is a list of items and this is the sentence that says the list cannot be
/// got back.
pub const UNINSTALL_IRREVERSIBLE: &str = "This cannot be undone.";

/// The button that does it, and the one that does not.
pub const UNINSTALL_REMOVE: &str = "Remove";
pub const UNINSTALL_KEEP_EVERYTHING: &str = "Cancel";

/// Heads the list of files in `proof/` that setup did not put there.
pub const UNINSTALL_UNRECOGNISED: &str =
    "Also in the program folder, and not placed there by setup:";

/// A checkbox label with the space it is holding, e.g. `Downloaded speech
/// models (2.1 GB)`.
///
/// Only the models item gets one. It is the only entry whose cost is both large
/// and invisible, and four kilobyte-sized figures beside it would bury the one
/// worth reading. The figure is **measured** at page-build time by
/// `uninstall::measure` — the label this descends from named "about 2.3 GB" for
/// a downloaded runtime this fork never had, which is what a written-down size
/// eventually becomes.
pub fn removable_label_with_size(label: &str, bytes: u64) -> String {
    // Decimal GB, matching what Explorer's "Size on disk" column and every
    // download page a user has seen would say for the same files. Below a
    // gigabyte the same rounding would print `0.1 GB` for a hundred megabytes,
    // so the smaller unit takes over.
    const GB: f64 = 1_000_000_000.0;
    const MB: f64 = 1_000_000.0;
    #[expect(
        clippy::cast_precision_loss,
        reason = "a display figure rounded to one decimal; the loss is far below the rounding"
    )]
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{label} ({:.1} GB)", bytes / GB)
    } else {
        format!("{label} ({:.0} MB)", (bytes / MB).max(1.0))
    }
}

/// Shown when the user declines the confirmation above.
///
/// Its own message rather than silence: a dialog that closes on No looks
/// identical to a dialog that crashed, and the one thing the user needs to know
/// is that their files are still where they were.
pub const UNINSTALL_CANCELLED: &str = "Nothing was removed. SpeakEasy Mini and all of its data are still on this \
     computer.";

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
pub const WEBVIEW2_MISSING: &str = "Microsoft Edge WebView2 Runtime is missing. SpeakEasy Mini cannot start \
     without it.\n\n\
     Install it from Microsoft's website (search for \"WebView2 Runtime\"), then run \
     setup again. Setup does not fetch it: Microsoft serves it from a link whose \
     contents change, so it cannot be checked against a fixed checksum the way \
     everything else here is.";

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

/// The graphics-card configuration's three artifacts, named separately.
///
/// Separately because they are fetched separately and from different places:
/// this project publishes the engine, NVIDIA's own servers serve the two
/// libraries. One label covering all of them would make a failure in one read as
/// a failure in the others, and the remedies are not the same.
///
/// **Distinct strings, and that is a requirement rather than a nicety.** The
/// download step lists these one per line and names one of them per progress
/// line, so two artifacts sharing a label print the same sentence twice and read
/// as a defect in setup. `every_graphics_card_artifact_gets_its_own_name` holds
/// this against the shipped catalog.
///
/// "Engine" rather than "worker": `worker` is this repository's word for the
/// child process, and it is not a word a user has been given.
pub const ARTIFACT_GPU_ENGINE: &str = "Graphics-card engine";
pub const ARTIFACT_GPU_CUDA_RUNTIME: &str = "Graphics-card runtime";
pub const ARTIFACT_GPU_MATH_LIBRARY: &str = "Graphics-card maths library";
/// For a redistributable this catalog pins that the two names above do not
/// cover. Generic on purpose: a wrong specific name is worse than an honest
/// vague one, and the test above turns the fallback into a failure rather than
/// letting it ship as a label.
pub const ARTIFACT_GPU_SUPPORT_LIBRARY: &str = "Graphics-card support library";

/// Shown when the pinned catalog compiled into this binary will not parse.
pub const CATALOG_UNAVAILABLE: &str = "Setup's list of verified downloads could not be read, so nothing was fetched. \
     That is a fault in this copy of setup, not in this computer.";

/// Shown when the app's own data directory cannot be located.
/// No `LOCALAPPDATA`, so setup does not know where the per-user program
/// directory is.
///
/// Named rather than guessed. The guess used to be `C:\`, which would have
/// unpacked the payload into the drive root and left uninstall walking it.
pub const INSTALL_ROOT_UNLOCATABLE: &str = "Setup could not work out where to put the program: this account's local \
     application-data folder is not set. Nothing was installed. Sign in as a \
     normal user and run setup again.";

pub const DATA_ROOT_UNLOCATABLE: &str =
    "Setup could not find where SpeakEasy Mini keeps its models. Nothing was changed.";

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
         What arrived is kept and verified as far as it went; running setup again \
         continues from there."
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
        "The {label} downloaded and verified, but could not be unpacked.\n\n{detail}\n\n\
         The downloaded copy is kept, so this does not need fetching again."
    )
}

/// Shown when the graphics-card payload was downloaded but could not be put
/// beside the app.
///
/// Its own sentence rather than [`install_of_artifact_failed`], which says the
/// artifact "could not be unpacked" — here it unpacked perfectly and the copy
/// into the program directory is what failed, so that message would send
/// someone to look at their download. The instruction is the processor, because
/// that is what the installation now is: the weights are in place and the app
/// runs, more slowly, and the engine check on the next page will say so rather
/// than this sentence having to be believed.
pub fn gpu_staging_failed(label: &str, detail: &str) -> String {
    format!(
        "The {label} was downloaded and verified but could not be put in place, so this \
         installation will use the processor.\n\n{detail}\n\n\
         Everything else installed. Running setup again is the fix; nothing needs \
         downloading a second time."
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
    lines.push("Each is checked against a checksum fixed in advance.".to_owned());
    lines.join("\n")
}

/// What the download step says when everything is already on this computer.
///
/// Its own message rather than a progress bar that fills instantly, because
/// those are not the same claim: nothing was transferred, and the reason is that
/// the files are present and their digests still match.
pub const DOWNLOAD_ALREADY_PRESENT: &str =
    "Everything needed is already here and still matches its checksum. Nothing to download.";

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
             The bar does not move while this happens, and it takes a while — the \
             archive is bigger unpacked than downloaded, and every file is checked."
        );
    }
    if phase == Phase::Verifying {
        // Says what is actually happening. This step used to report "Downloading
        // — 0 MB transferred" for the twenty-four seconds it takes to digest a
        // 2.3 GB archive that was already on disk, which is both untrue and
        // exactly what a stalled download looks like.
        return format!(
            "Checking the {name} ({position}) against its checksum.\n\n\
             It was already downloaded, so nothing is transferring — this re-reads \
             what is on disk to confirm it is intact."
        );
    }
    format!(
        "Downloading the {name} ({position}).\n\n{} of {} transferred.\n\n\
         If this is interrupted, what has arrived is kept and the next attempt \
         continues from here.",
        transfer_size(done_bytes),
        transfer_size(total_bytes)
    )
}

/// Shown when everything in the plan arrived and was verified.
pub fn describe_download_complete(labels: &[&str], total_bytes: u64) -> String {
    format!(
        "{} downloaded and verified: {}.\n\n\
         Fetched and checked is not the same as proven to work. The last page runs it.",
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
