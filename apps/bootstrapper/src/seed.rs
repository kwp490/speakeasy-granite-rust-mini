//! What setup tells the app about the answers it collected.
//!
//! # The channel
//!
//! One small file per answer, under `%APPDATA%\ai.speakeasy.mini\config\`, read
//! and **deleted** by `apps/desktop` the first time it starts. `composition.rs`
//! calls those readers `consume_installer_*_seed`, and the deletion is the
//! important half: a seed is a starting value, not a policy, so a user who
//! changes the setting afterwards must never have setup's answer come back on
//! the next launch.
//!
//! The read side was built first and has been sitting here with nothing on the
//! other end — `consume_installer_hotkey_seed` and
//! `consume_installer_logging_seed` both looked for files that nothing wrote,
//! so the shortcut and the logging choice were collected by the wizard and
//! silently discarded. That is the same shape that left `smoke.rs` unbuilt
//! behind a comment promising it existed, and it is why this module writes
//! every answer the wizard asks for rather than the two that had readers.
//!
//! # Why files rather than the registry
//!
//! Because the app already reads files here, and because a seed has to be
//! removable by the thing that consumes it. The version stamp and the
//! Add/Remove Programs entry are registry state that outlives a run; these are
//! one-shot messages between two processes that never meet.
//!
//! # Failure is never fatal
//!
//! A seed that cannot be written costs the user a default they will meet again
//! in Settings. Failing an otherwise complete install over it would be the
//! wrong trade, so every function here reports what it could not do and setup
//! carries on. What it must not do is claim to have recorded something it did
//! not — see [`Written`].

use std::path::{Path, PathBuf};

/// The provider setup **proved** this installation runs on.
///
/// Recorded because the app cannot work it out afterwards, and the two states it
/// cannot otherwise tell apart owe the user opposite messages: a CPU install
/// running on the CPU is working exactly as installed, while a graphics-card
/// install running on the CPU is a fault.
///
/// # It is proof, not a preference
///
/// This came from the provider page's radio button until 2026-08-20, and the
/// radio button was never disabled — so a user on a CUDA-capable machine could
/// select "Use the graphics card", setup would install the only configuration it
/// has, and this file would say `cuda`. The app then correctly found no GPU path
/// and logged `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`: three
/// fields, one of them a claim nothing had ever checked.
///
/// It is now written by [`record_installed_provider`] from
/// `smoke::ProviderEvidence`, after the engine check has actually run — which
/// means [`Self::GraphicsCard`] requires a published and complete CUDA payload,
/// a worker that reported a CUDA backend at `Hello`, and NVML placing that
/// worker's own process on a device. Today no release satisfies the first, so
/// only [`Self::Processor`] is reachable; that is a fact about the release
/// rather than about this channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provider {
    Processor,
    GraphicsCard,
}

impl Provider {
    /// The word written to disk. Stable — `apps/desktop` matches on it.
    const fn code(self) -> &'static str {
        match self {
            Self::Processor => "cpu",
            Self::GraphicsCard => "cuda",
        }
    }
}

/// Everything the wizard collected, in one place.
///
/// A struct rather than five arguments so that adding a sixth question is one
/// field and one line in [`write()`], and so the wizard can hand over exactly
/// what it holds without deciding an order.
pub struct Answers {
    /// The activation shortcut, in the spelling
    /// `speakeasy_storage::Settings::hotkey::activation_binding` uses.
    pub shortcut: String,
    /// Words to protect, already split out of what was typed.
    ///
    /// Parsed rather than raw, and that is the contract: the wizard tells the
    /// user how many words it read, so the same parse has to be what reaches
    /// disk. A raw string here would let the box say "3 words" and the file
    /// carry two.
    pub vocabulary: Vec<String>,
    /// Whether transcripts survive closing the app.
    pub keep_transcripts: bool,
    /// Whether the diagnostic log is written to disk.
    pub disk_logging: bool,
}

/// What [`write()`] managed to record.
///
/// Carries the failures rather than a `bool`, because the honest thing to show
/// a user is which answer was not kept. A seed channel that reports success
/// having written nothing is the failure this module exists on the wrong side
/// of once already.
#[derive(Debug, Default)]
pub struct Written {
    /// The names of the seeds that could not be written, if any.
    pub failed: Vec<&'static str>,
}

impl Written {
    #[must_use]
    pub fn all_recorded(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Where the app looks for seeds.
///
/// `config` under the data root, matching `app_root.join("config/…")` in
/// `apps/desktop`'s `commands/dictation.rs` exactly. Pinned by
/// `the_seed_directory_is_the_one_the_app_reads`, because nothing else
/// would notice the two drifting apart — setup would report every answer
/// recorded and the app would start with every default.
#[must_use]
pub fn directory() -> Option<PathBuf> {
    Some(crate::uninstall::data_root()?.join("config"))
}

/// File names, spelled once. `apps/desktop` reads these exact names.
const SHORTCUT: &str = "install-hotkey.txt";
const LOGGING: &str = "install-logging.txt";
const RETENTION: &str = "install-retention.txt";
const VOCABULARY: &str = "install-vocabulary.txt";

/// The installed-configuration record.
///
/// **Not a seed**, and it is the one file here the app does not consume: a seed
/// is a starting value the user may then change, and this is a statement about
/// what is on disk that stays true for the life of the installation.
///
/// Written separately from [`write()`], by [`record_installed_provider`], because
/// the two are answers to different questions asked at different moments. The
/// seeds record what the user chose, on leaving the last question. This records
/// what setup *proved*, after the engine check — and it cannot be written before
/// that check, because before it there is nothing to be right about.
const PROVIDER: &str = "install-provider.txt";

/// Every seed, for the cases where none of them can be written.
const ALL: [&str; 4] = [SHORTCUT, LOGGING, RETENTION, VOCABULARY];

/// Record the wizard's answers for the app's first launch.
///
/// Never returns `Err`: see this module's header. The report says what did not
/// land.
#[must_use]
pub fn write(answers: &Answers) -> Written {
    let mut written = Written::default();
    // No `APPDATA`, or a config directory that cannot be created: every seed
    // fails together, and the caller shows one sentence rather than five.
    let Some(directory) = directory() else {
        written.failed.extend(ALL);
        return written;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        written.failed.extend(ALL);
        return written;
    }
    // `0`/`1` for the two booleans, matching what `consume_installer_logging_seed`
    // already parses. Anything else is ignored by the reader rather than
    // guessed at, so a malformed seed leaves the app's own default in place.
    let items: [(&'static str, String); 4] = [
        (SHORTCUT, answers.shortcut.clone()),
        (LOGGING, flag(answers.disk_logging)),
        (RETENTION, flag(answers.keep_transcripts)),
        // Comma-separated, which is both what the box now asks for and what the
        // app parses. Written from the already-parsed list rather than from the
        // typed text, so the file cannot carry a different set of words than
        // the count the user was shown.
        (VOCABULARY, answers.vocabulary.join(", ")),
    ];
    for (name, contents) in items {
        if !write_one(&directory, name, &contents) {
            written.failed.push(name);
        }
    }
    written
}

/// The protected words this profile already has, or an empty list.
///
/// Read-only, and side-effect free: `PersonalizationRepository::open` reads the
/// file when it exists and holds an in-memory default when it does not, writing
/// nothing either way. Setup must be able to ask this question without creating
/// the profile it is asking about.
///
/// Only the *identity* entries — those whose source and replacement are the same
/// word, which is what "protect this word" means and what setup collects. The
/// spaced companions the app derives (`Logic Monitor` -> `LogicMonitor`) are
/// deliberately excluded: they are generated from these, so listing them back
/// would show the user rules they never typed and re-seed them as terms in their
/// own right.
#[must_use]
pub fn existing_protected_terms() -> Vec<String> {
    let Some(directory) = directory() else {
        return Vec::new();
    };
    let Ok(repository) =
        speakeasy_storage::PersonalizationRepository::open(directory.join("personalization.json"))
    else {
        return Vec::new();
    };
    repository
        .state()
        .dictionary
        .iter()
        .filter(|entry| entry.enabled && entry.source == entry.replacement)
        .map(|entry| entry.source.clone())
        .collect()
}

/// What the words box should start with, and what a silent install should seed.
///
/// A profile that already has words gets its own back; one that has none gets
/// [`crate::catalog::DEFAULT_VOCABULARY`].
///
/// **This is the guard against setup replacing a vocabulary somebody curated.**
/// `add_protected_terms` *replaces* every entry setup owns rather than merging —
/// deliberately, because the id-keyed merge it replaced left stale entries that
/// collided with the new list and cost the user every word. The cost of that
/// choice is that any non-empty seed is authoritative, so a reinstall that
/// seeded a canned list over a customised one would silently discard the
/// customisation. Returning the existing words makes the re-seed idempotent
/// instead: the same set goes back, and the derived companions are regenerated.
#[must_use]
pub fn vocabulary_to_offer() -> String {
    offer_from(&existing_protected_terms())
}

/// [`vocabulary_to_offer`]'s decision, without the disk.
///
/// Separated so the rule can be asserted directly: the two callers differ only
/// in what they do with the answer, and the answer is the part worth pinning.
fn offer_from(existing: &[String]) -> String {
    if existing.is_empty() {
        crate::catalog::DEFAULT_VOCABULARY.to_owned()
    } else {
        existing.join(", ")
    }
}

/// Whether an install with no wizard behind it should seed the default list.
///
/// Only onto a profile with no words of its own. A scripted reinstall must be
/// able to run against a machine somebody has been using without discarding
/// what they added — and because the seed *replaces*, "write it anyway" and
/// "overwrite their list" are the same act.
const fn should_seed_silently(existing: &[String]) -> bool {
    existing.is_empty()
}

/// Writes the vocabulary seed alone, for an install with no wizard behind it.
///
/// `--install` asks the user nothing, so it has no answers to record — every
/// other seed would be setup asserting a choice nobody made, and the app's own
/// defaults are already the right answer for those. The vocabulary is the
/// exception because there is no app-side default for it: without this, a
/// scripted deployment gets an empty dictionary and the feature reaches nobody.
///
/// **Writes nothing when the profile already has words.** See
/// [`vocabulary_to_offer`] for why that is not merely polite.
///
/// Returns whether a seed was written, so the caller can say so.
pub fn write_default_vocabulary() -> bool {
    if !should_seed_silently(&existing_protected_terms()) {
        return false;
    }
    let Some(directory) = directory() else {
        return false;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return false;
    }
    // Through the same parse the wizard's count comes from, so a silent install
    // cannot seed a list the interactive one would have rejected or trimmed.
    let terms = parse_vocabulary(crate::catalog::DEFAULT_VOCABULARY);
    !terms.is_empty() && write_one(&directory, VOCABULARY, &terms.join(", "))
}

/// The longest word the app will accept from the vocabulary seed.
///
/// Matched to `consume_installer_vocabulary_seed`, which enforces the same bound
/// against a file anything with write access to the profile could have replaced.
/// Applied here too so the count the wizard shows is the count that survives —
/// a box that reported five words where the app took four would be a small lie
/// in the one place this feature is judged.
const MAX_TERM_CHARS: usize = 64;

/// The most words the app will take from the seed.
const MAX_TERMS: usize = 128;

/// Split what was typed into the words setup will record.
///
/// **Commas, since 2026-08-20.** The box asked for one word per line before
/// that, which is more typing and one more convention to remember; a
/// comma-separated list is the form people already use. Newlines still separate,
/// because a user who types them anyway means the same thing and losing their
/// words to a formatting rule would be indefensible.
///
/// Case-insensitively deduplicated, and that is not tidiness. Two entries whose
/// source differs only in case are a *conflicting rule* to the dictionary
/// validator, which rejects the whole batch — so a user who typed "Ken, ken"
/// would have had every one of their words silently dropped. That is exactly the
/// failure this change was made to fix, one layer up.
#[must_use]
pub fn parse_vocabulary(typed: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for candidate in typed.split([',', '\n', '\r']) {
        let word = candidate.trim();
        if word.is_empty() || word.chars().count() > MAX_TERM_CHARS {
            continue;
        }
        // `to_lowercase`, not `eq_ignore_ascii_case`: the dictionary validator
        // builds its match key with Unicode lowercasing, so a pair this misses
        // is a pair it would still call a conflict.
        let folded = word.to_lowercase();
        if words.iter().any(|kept| kept.to_lowercase() == folded) {
            continue;
        }
        words.push(word.to_owned());
        if words.len() == MAX_TERMS {
            break;
        }
    }
    words
}

/// Record which configuration setup proved this installation runs on.
///
/// Called once the engine check has settled, and **only** from its verdict.
/// Never from a choice, a preference or a probe: the app reads this for the life
/// of the installation to tell an expected processor run from a broken
/// graphics-card one, and a value that describes an intention makes both of those
/// unanswerable.
///
/// Absent is a meaningful third state and is deliberately reachable: an install
/// where the check never ran writes nothing, and `apps/desktop` reads that as
/// `"unrecorded"` rather than guessing `cpu`. Guessing would be a claim about a
/// configuration nobody verified.
///
/// Returns whether it landed. Failure costs a diagnostic field rather than the
/// install, so the caller reports it and carries on.
#[must_use]
pub fn record_installed_provider(provider: Provider) -> bool {
    let Some(directory) = directory() else {
        return false;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return false;
    }
    write_one(&directory, PROVIDER, provider.code())
}

/// Discard the provider record, for an install that ran no engine check.
///
/// **A stale record is worse than no record**, and this exists because one was
/// measured on 2026-08-27. `--install` places the payload's own processor worker
/// and never runs `smoke::verify_engine`, so it has nothing to record — but the
/// file is not a seed and the app never consumes it, so a `cuda` written by an
/// *earlier* wizard install survives the reinstall and goes on describing a
/// configuration no longer on disk. The app then correctly reported
/// `engine=cpu_gpu_runtime_missing device=cpu installed=cuda
/// provider=gpu_install_not_operational`: a real fault banner, on a machine
/// whose only problem was that nobody had corrected the record.
///
/// Clearing rather than writing `cpu` is the same rule
/// [`record_installed_provider`] states: a check that never ran writes nothing,
/// and `apps/desktop` reads absence as `"unrecorded"`. Writing `cpu` here would
/// be a claim about a configuration nobody verified — true by accident today,
/// because a silent install happens to place the processor worker, and a lie
/// the moment that stops being so.
///
/// Missing already is success: the post-condition is that no record is left, not
/// that a file was deleted.
pub fn clear_installed_provider() -> bool {
    let Some(directory) = directory() else {
        return false;
    };
    match std::fs::remove_file(directory.join(PROVIDER)) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// `"1"` or `"0"`, the whole vocabulary of the boolean seeds.
///
/// Spelled as the app parses them: `consume_installer_logging_seed` matches the
/// two literals and ignores anything else, so a seed written as `true` would be
/// silently dropped and the answer lost.
fn flag(value: bool) -> String {
    if value { "1" } else { "0" }.to_owned()
}

fn write_one(directory: &Path, name: &str, contents: &str) -> bool {
    std::fs::write(directory.join(name), contents).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seed_directory_is_the_one_the_app_reads() {
        // `apps/desktop/src-tauri/src/commands/dictation.rs` joins
        // `config/install-hotkey.txt` onto Tauri's `app_data_dir`, which is
        // `%APPDATA%\ai.speakeasy.mini`. If these disagree, setup records every
        // answer successfully and the app starts with every default -- with
        // nothing anywhere reporting a problem.
        let Some(directory) = directory() else {
            // No APPDATA in this environment; there is nothing to compare.
            return;
        };
        assert!(
            directory.ends_with("config"),
            "seeds must live in the config directory the app reads: {}",
            directory.display()
        );
        assert!(
            directory
                .parent()
                .is_some_and(|parent| parent.ends_with("ai.speakeasy.mini")),
            "seeds must sit under this product's identifier: {}",
            directory.display()
        );
    }

    /// Every seed this writes is read by something on the desktop side.
    ///
    /// Read out of the app's own source, per this project's habit of pinning
    /// invariants against the file rather than against a copy of it. The
    /// failure it guards is precisely the one that produced this module: a
    /// reader and a writer that name different files agree about everything
    /// except the file, and the symptom is setup reporting five answers
    /// recorded while the app starts with five defaults. Nothing logs it,
    /// nothing errors, and the user's shortcut is simply not the one they
    /// chose.
    #[test]
    fn every_seed_written_here_is_read_by_the_app() {
        const CONSUMERS: &[&str] = &[
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../desktop/src-tauri/src/commands/dictation.rs"
            ),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../desktop/src-tauri/src/composition.rs"
            ),
        ];
        let sources = CONSUMERS
            .iter()
            .map(|path| {
                std::fs::read_to_string(path).unwrap_or_else(|error| {
                    panic!("the app's seed readers must be readable at {path}: {error}")
                })
            })
            .collect::<Vec<_>>();
        // `PROVIDER` alongside `ALL`: it is not a seed, but it is still written
        // here and read there, and the drift this guards against is the same.
        for name in ALL.iter().copied().chain(std::iter::once(PROVIDER)) {
            assert!(
                sources.iter().any(|source| source.contains(name)),
                "{name} is written by setup and read by nothing"
            );
        }
    }

    #[test]
    fn a_comma_separated_list_becomes_one_word_each() {
        assert_eq!(
            parse_vocabulary("Kenneth, Anthropic , Granite"),
            vec![
                "Kenneth".to_owned(),
                "Anthropic".to_owned(),
                "Granite".to_owned()
            ]
        );
    }

    #[test]
    fn newlines_still_separate_and_empty_pieces_vanish() {
        // A user who types the old one-per-line form, or leaves a trailing
        // comma, means what they wrote. Losing a word to punctuation would be
        // the least defensible failure this box could have.
        assert_eq!(
            parse_vocabulary("Kenneth\r\nAnthropic,,  ,Granite,"),
            vec![
                "Kenneth".to_owned(),
                "Anthropic".to_owned(),
                "Granite".to_owned()
            ]
        );
    }

    #[test]
    fn a_case_only_duplicate_is_dropped_rather_than_taking_the_list_with_it() {
        // The precise failure that motivated the parse. Two entries differing
        // only in case are a conflicting rule to the dictionary validator, which
        // refuses the *whole* batch -- so "Ken, ken" used to cost the user every
        // word they typed, with nothing reported anywhere.
        assert_eq!(
            parse_vocabulary("Ken, ken, KEN, Granite"),
            vec!["Ken".to_owned(), "Granite".to_owned()]
        );
    }

    #[test]
    fn an_over_long_word_is_skipped_and_the_rest_survive() {
        let long = "x".repeat(MAX_TERM_CHARS + 1);
        assert_eq!(
            parse_vocabulary(&format!("Granite, {long}, Anthropic")),
            vec!["Granite".to_owned(), "Anthropic".to_owned()]
        );
    }

    #[test]
    fn the_written_seed_is_the_comma_form_the_app_parses() {
        let answers = Answers {
            shortcut: "Ctrl+Alt+P".to_owned(),
            vocabulary: parse_vocabulary("Kenneth, Anthropic"),
            keep_transcripts: false,
            disk_logging: true,
        };
        assert_eq!(answers.vocabulary.join(", "), "Kenneth, Anthropic");
    }

    #[test]
    fn the_installed_configuration_is_not_one_of_the_answers() {
        // The separation, asserted. `write` records what the user chose; the
        // provider record states what setup proved, and it is written by a
        // different function at a later moment. Folding it back into `Answers`
        // is exactly how it came to be derived from a radio button.
        assert!(
            !ALL.contains(&PROVIDER),
            "the provider record is not a seed"
        );
    }

    #[test]
    fn the_provider_codes_are_the_ones_the_app_matches_on() {
        assert_eq!(Provider::Processor.code(), "cpu");
        assert_eq!(Provider::GraphicsCard.code(), "cuda");
    }

    /// The list the box arrives holding has to survive the parse that writes
    /// the seed, or setup ships a default that quietly loses words.
    ///
    /// Every assertion here is against `parse_vocabulary`'s own answer rather
    /// than against a second copy of the list, so a term added to the catalog
    /// is covered by this the moment it is added.
    #[test]
    fn the_prefilled_vocabulary_survives_its_own_parse() {
        let parsed = parse_vocabulary(crate::catalog::DEFAULT_VOCABULARY);

        // Nothing lost. A dropped term means a typo, a duplicate differing only
        // in case, or a word past the length bound -- all three are silent.
        assert_eq!(
            parsed.len(),
            crate::catalog::DEFAULT_VOCABULARY.split(',').count(),
            "the prefilled list lost a term to the parse: {parsed:?}"
        );

        // Round-trips, so what the app reads back is what the page showed.
        assert_eq!(
            parse_vocabulary(&parsed.join(", ")),
            parsed,
            "the prefilled list is not stable through a write/read cycle"
        );

        for term in &parsed {
            assert!(
                term.chars().count() <= MAX_TERM_CHARS,
                "{term} is past the seed's own length bound"
            );
            assert_eq!(term.trim(), term, "{term} carries surrounding whitespace");
            assert!(!term.is_empty(), "an empty term reached the parsed list");
        }

        // Case-insensitively unique, which is the condition the dictionary
        // validator refuses the *whole batch* over. A default list that trips it
        // would cost every user every word, on every install, silently.
        let mut folded: Vec<String> = parsed.iter().map(|term| term.to_lowercase()).collect();
        folded.sort();
        let before = folded.len();
        folded.dedup();
        assert_eq!(
            before,
            folded.len(),
            "the prefilled list has a case-only duplicate"
        );
    }

    /// The guard that keeps a reinstall from discarding a curated word list.
    ///
    /// `add_protected_terms` *replaces* rather than merges, so any non-empty
    /// seed is authoritative -- which means offering a canned list to a profile
    /// that already has one would silently overwrite it. Asserted on the
    /// decision itself rather than through the filesystem, because the two
    /// callers differ only in what they do with the answer.
    #[test]
    fn a_profile_with_words_is_offered_its_own_and_an_empty_one_gets_the_default() {
        // The shape of the answer for a profile with nothing: the shipped list,
        // verbatim, so the box and the silent seed agree with the catalog.
        assert_eq!(
            offer_from(&[]),
            crate::catalog::DEFAULT_VOCABULARY,
            "an empty profile gets the default"
        );

        // And for a profile with words: its own, never the default. A returning
        // user pressing Next re-seeds what they already had.
        let existing = vec!["Kenneth".to_owned(), "Hellen".to_owned()];
        assert_eq!(offer_from(&existing), "Kenneth, Hellen");
        assert_ne!(offer_from(&existing), crate::catalog::DEFAULT_VOCABULARY);

        // Round-trips through the parse that writes the file, so re-seeding an
        // existing list cannot lose a word to the formatting.
        assert_eq!(parse_vocabulary(&offer_from(&existing)), existing);
    }

    /// A silent install seeds only when there is nothing to lose.
    #[test]
    fn the_silent_seed_is_written_for_an_empty_profile_and_withheld_otherwise() {
        assert!(should_seed_silently(&[]), "a fresh profile must be seeded");
        assert!(
            !should_seed_silently(&["Kenneth".to_owned()]),
            "an existing list must survive"
        );
    }

    /// Clearing is idempotent, and absence is the success condition rather
    /// than a delete having happened. A silent install runs on machines with
    /// and without a prior record, and both have to end in the same state.
    #[test]
    fn clearing_the_provider_record_is_idempotent_and_leaves_nothing() {
        let Some(directory) = directory() else {
            return;
        };
        let path = directory.join(PROVIDER);
        let restore = std::fs::read_to_string(&path).ok();

        assert!(std::fs::create_dir_all(&directory).is_ok());
        assert!(write_one(
            &directory,
            PROVIDER,
            Provider::GraphicsCard.code()
        ));
        assert!(path.exists(), "the fixture must be in place to be cleared");

        assert!(
            clear_installed_provider(),
            "a present record must be removed"
        );
        assert!(!path.exists());
        assert!(clear_installed_provider(), "already absent is success");

        // Put back whatever this machine had, so running the suite does not
        // silently reconfigure the developer's own installation.
        if let Some(previous) = restore {
            assert!(write_one(&directory, PROVIDER, &previous));
        }
    }
}
