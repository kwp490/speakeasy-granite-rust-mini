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

/// The provider setup installed for.
///
/// Recorded because the app cannot work it out afterwards, and the two states
/// it cannot otherwise tell apart owe the user opposite messages: a CPU install
/// running on the CPU is working exactly as installed, while a graphics-card
/// install running on the CPU is a fault. Today only [`Self::Processor`] is
/// reachable, because no CUDA-built worker has been published; that is a fact
/// about the release, not about this channel.
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
/// field and one line in [`write`], and so the wizard can hand over exactly
/// what it holds without deciding an order.
pub struct Answers {
    /// The activation shortcut, in the spelling
    /// `speakeasy_storage::Settings::hotkey::activation_binding` uses.
    pub shortcut: String,
    /// Words to protect, one per line, exactly as typed.
    pub vocabulary: String,
    /// Whether transcripts survive closing the app.
    pub keep_transcripts: bool,
    /// Whether the diagnostic log is written to disk.
    pub disk_logging: bool,
    /// Which configuration was installed.
    pub provider: Provider,
}

/// What [`write`] managed to record.
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
/// [`tests::the_seed_directory_is_the_one_the_app_reads`], because nothing else
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
const PROVIDER: &str = "install-provider.txt";

/// Every seed, for the cases where none of them can be written.
const ALL: [&str; 5] = [SHORTCUT, LOGGING, RETENTION, VOCABULARY, PROVIDER];

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
    let items: [(&'static str, String); 5] = [
        (SHORTCUT, answers.shortcut.clone()),
        (LOGGING, flag(answers.disk_logging)),
        (RETENTION, flag(answers.keep_transcripts)),
        (VOCABULARY, answers.vocabulary.clone()),
        (PROVIDER, answers.provider.code().to_owned()),
    ];
    for (name, contents) in items {
        if !write_one(&directory, name, &contents) {
            written.failed.push(name);
        }
    }
    written
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
        for seed in ALL {
            assert!(
                sources.iter().any(|source| source.contains(seed)),
                "{seed} is written by setup and read by nothing"
            );
        }
    }

    #[test]
    fn the_provider_codes_are_the_ones_the_app_matches_on() {
        assert_eq!(Provider::Processor.code(), "cpu");
        assert_eq!(Provider::GraphicsCard.code(), "cuda");
    }
}
