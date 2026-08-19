//! The files setup installs, carried inside setup's own executable.
//!
//! # Why this exists
//!
//! Setup used to look for a `payload/` directory beside the bootstrapper, which
//! is correct for a locally built tree and useless for a download. `README.md`
//! tells a user to fetch one file from Releases and run it; one file that
//! installs nothing is the gap this closes.
//!
//! The alternative was `include_bytes!` in the bootstrapper itself. It was
//! rejected because it inverts the build order — `speakeasy-bootstrapper` would
//! have to be compiled *after* the desktop executable it embeds, and a fresh
//! `cargo build -p speakeasy-bootstrapper` would fail on a missing file or, if
//! the missing file were papered over, produce an installer carrying an empty
//! payload that still ran. That is the silent-success shape this project spends
//! most of its comments avoiding.
//!
//! So the payload is appended to the finished executable instead. Windows'
//! loader ignores bytes past the end of a PE image, so the file is still a
//! perfectly ordinary program with an archive stuck to its back.
//!
//! # The format
//!
//! Little-endian throughout, entries first and a fixed trailer last:
//!
//! ```text
//! <the bootstrapper's own PE image>
//! <entry>*   u32 path length | path (UTF-8, `/`-separated)
//!            u64 data length | data | [u8; 32] SHA-256 of data
//! <trailer>  u64 archive length | u32 entries | u32 format | [u8; 8] magic
//! ```
//!
//! Deliberately not a zip. A zip needs a dependency, and the two things this
//! format has to do — say where the archive starts when only the end of the
//! file is known, and refuse a byte that is not the byte that was packed — are
//! the whole of it. There is no compression: 36 MB is an ordinary download, and
//! a decompressor is a second thing that can be wrong about a file.
//!
//! # Why every entry is hashed
//!
//! Not to defend against tampering — the whole executable is untrusted until
//! the user chooses to run it, and a digest inside it proves nothing about
//! that. It is to catch a **truncated download**, which is the failure this
//! shape invites. Trailing bytes are not part of the image, so a setup
//! executable that arrived 90% complete still starts, still draws the wizard,
//! and would still install whatever fragment of the payload it could parse.
//! With the digests it stops and says the download is incomplete.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Marks the last eight bytes of an executable that carries a payload.
///
/// Read from the end, because that is the only end whose offset is known: the
/// image's own length is not recorded anywhere the bootstrapper can reach.
const MAGIC: [u8; 8] = *b"SEMINIPL";

/// Bumped only if the layout above changes. A reader that meets a format it
/// does not know refuses rather than guessing at the fields it recognises.
const FORMAT_VERSION: u32 = 1;

/// `u64` archive length, `u32` entry count, `u32` format, `[u8; 8]` magic.
const TRAILER_BYTES: usize = 8 + 4 + 4 + 8;

/// Why a payload could not be read or written.
///
/// A type rather than a sentence, because `catalog.rs` owns every word the
/// wizard shows and this module is also compiled into `bin/pack-payload.rs`,
/// which cannot reach the catalog. The `Display` text is the developer-facing
/// half: it goes to the packer's stderr and never in front of a user.
#[derive(Debug)]
pub enum ArchiveError {
    /// Truncated, or a digest that does not match its bytes.
    ///
    /// One variant for both, because a user's action is the same for either and
    /// neither tells them anything they can act on beyond "this is not the file
    /// that was published".
    Damaged,
    /// A payload written by a newer packer than this reader understands.
    UnknownFormat { found: u32 },
    /// A path that would write outside the payload root.
    UnsafePath { path: String },
    /// Anything the filesystem refused, carried verbatim.
    Io { detail: String },
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Damaged => formatter.write_str("the payload archive is truncated or corrupt"),
            Self::UnknownFormat { found } => {
                write!(
                    formatter,
                    "payload format {found} is newer than this build reads"
                )
            }
            Self::UnsafePath { path } => write!(formatter, "unsafe payload path: {path}"),
            Self::Io { detail } => formatter.write_str(detail),
        }
    }
}

fn io(error: &std::io::Error) -> ArchiveError {
    ArchiveError::Io {
        detail: error.to_string(),
    }
}

/// One packed file, and where it goes relative to the payload root.
pub struct Entry {
    /// `/`-separated and relative, always. See [`checked_relative_path`].
    pub path: String,
    pub bytes: Vec<u8>,
}

/// A payload ready to be installed from, and whether it is ours to delete.
///
/// The distinction is the whole reason this is a type rather than a `PathBuf`:
/// an extracted payload lives in the temporary directory this created and must
/// be removed, while a `payload/` directory beside a locally built bootstrapper
/// belongs to the build and must not be.
pub struct Staged {
    directory: PathBuf,
    temporary: bool,
}

impl Staged {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.temporary {
            // Best effort. A temporary directory that outlives setup is litter;
            // failing an otherwise complete install over it would be worse.
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

/// Produce a directory holding the files to install.
///
/// Embedded payload first, `payload/` beside the executable second. That order
/// is not arbitrary: a developer running the locally built bootstrapper has no
/// embedded payload and gets the directory, while a downloaded setup has an
/// embedded payload and must never prefer whatever happens to sit beside it in
/// a Downloads folder.
///
/// # Errors
///
/// Only when an archive is present and cannot be trusted — a damaged download,
/// a format from a future version, or a temporary directory that cannot be
/// written. An executable with no archive at all is not an error; it is the
/// locally built case.
pub fn stage() -> Result<Staged, ArchiveError> {
    let executable = std::env::current_exe().map_err(|error| io(&error))?;
    let Some(entries) = read_archive(&executable)? else {
        let directory = executable
            .parent()
            .ok_or_else(|| ArchiveError::Io {
                detail: "setup has no directory to read a payload from".to_owned(),
            })?
            .join("payload");
        return Ok(Staged {
            directory,
            temporary: false,
        });
    };
    // Named for the process rather than randomly, so two concurrent runs cannot
    // extract over each other and an abandoned directory says which run left it.
    let directory =
        std::env::temp_dir().join(format!("speakeasy-mini-setup-{}", std::process::id()));
    let staged = Staged {
        directory,
        temporary: true,
    };
    // Cleared first. A directory left by an earlier run that happened to hold
    // the same process id would otherwise contribute files this archive does
    // not contain, and `install::perform` merges rather than replaces.
    let _ = std::fs::remove_dir_all(staged.directory());
    for entry in entries {
        let destination = staged.directory().join(entry.path.replace('/', "\\"));
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| io(&error))?;
        }
        std::fs::write(&destination, &entry.bytes).map_err(|error| io(&error))?;
    }
    Ok(staged)
}

/// Read the archive an executable carries, if it carries one.
///
/// `Ok(None)` means there is no archive — the ordinary locally built case, and
/// deliberately not an error. `Err` means there is one and it cannot be used.
///
/// # Errors
///
/// A trailer that does not describe the file it is in, an entry whose digest
/// does not match its bytes, or a path that tries to escape the payload root.
pub fn read_archive(executable: &Path) -> Result<Option<Vec<Entry>>, ArchiveError> {
    let mut file = File::open(executable).map_err(|error| io(&error))?;
    let total = file.seek(SeekFrom::End(0)).map_err(|error| io(&error))?;
    if total < TRAILER_BYTES as u64 {
        return Ok(None);
    }
    let mut trailer = [0u8; TRAILER_BYTES];
    // From the start rather than a negative offset from the end: the same seek
    // spelled backwards needs an `i64`, and casting a `usize` into one is a
    // wrap the compiler is right to point at even where the value is 24.
    file.seek(SeekFrom::Start(total - TRAILER_BYTES as u64))
        .map_err(|error| io(&error))?;
    file.read_exact(&mut trailer).map_err(|error| io(&error))?;
    if trailer[16..24] != MAGIC {
        return Ok(None);
    }
    let archive_bytes = u64::from_le_bytes(read_field(&trailer[0..8]));
    let count = u32::from_le_bytes(read_field(&trailer[8..12]));
    let format = u32::from_le_bytes(read_field(&trailer[12..16]));
    if format != FORMAT_VERSION {
        return Err(ArchiveError::UnknownFormat { found: format });
    }
    // The first thing a truncation breaks. A file cut short reports an archive
    // longer than what is left of it, and every field after this point would
    // otherwise be read out of the image's own bytes.
    let start = total
        .checked_sub(TRAILER_BYTES as u64)
        .and_then(|end| end.checked_sub(archive_bytes))
        .ok_or(ArchiveError::Damaged)?;
    file.seek(SeekFrom::Start(start))
        .map_err(|error| io(&error))?;
    let mut reader = std::io::BufReader::new(file.take(archive_bytes));
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        entries.push(read_entry(&mut reader)?);
    }
    Ok(Some(entries))
}

/// A fixed-width field out of the trailer.
///
/// The slices are all taken from one array of known length, so the conversion
/// cannot fail; this exists to say that once rather than four times.
fn read_field<const N: usize>(slice: &[u8]) -> [u8; N] {
    slice
        .try_into()
        .expect("a fixed-width slice of the fixed-size trailer")
}

fn read_entry(reader: &mut impl Read) -> Result<Entry, ArchiveError> {
    let mut length = [0u8; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| ArchiveError::Damaged)?;
    let mut path = vec![0u8; u32::from_le_bytes(length) as usize];
    reader
        .read_exact(&mut path)
        .map_err(|_| ArchiveError::Damaged)?;
    let path = String::from_utf8(path).map_err(|_| ArchiveError::Damaged)?;
    let path = checked_relative_path(&path)?;

    let mut length = [0u8; 8];
    reader
        .read_exact(&mut length)
        .map_err(|_| ArchiveError::Damaged)?;
    let declared =
        usize::try_from(u64::from_le_bytes(length)).map_err(|_| ArchiveError::Damaged)?;
    let mut bytes = vec![0u8; declared];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| ArchiveError::Damaged)?;

    let mut expected = [0u8; 32];
    reader
        .read_exact(&mut expected)
        .map_err(|_| ArchiveError::Damaged)?;
    if Sha256::digest(&bytes).as_slice() != expected {
        return Err(ArchiveError::Damaged);
    }
    Ok(Entry { path, bytes })
}

/// Refuse any path that would write outside the payload root.
///
/// The archive is built by this repository's own packaging step, so this is not
/// defending against a hostile archive so much as against a packaging mistake
/// that would otherwise write into an arbitrary directory and be discovered by
/// its damage. `install::perform` joins these onto the install root, and
/// `Path::join` with an absolute path *replaces* the root rather than appending
/// to it — which is the specific way this goes wrong quietly.
fn checked_relative_path(path: &str) -> Result<String, ArchiveError> {
    let rejected = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|part| part.is_empty() || part == "..");
    if rejected {
        return Err(ArchiveError::UnsafePath {
            path: path.to_owned(),
        });
    }
    Ok(path.to_owned())
}

/// Append an archive of `entries` to `executable`, in place.
///
/// Lives here rather than in the packaging script so that the format has one
/// implementation. A PowerShell writer and a Rust reader are two descriptions
/// of the same layout that agree until somebody edits one of them, and the
/// disagreement would not surface on the build machine — it would surface on a
/// user's, as setup reporting a damaged download of a file that downloaded
/// perfectly. `bin/pack-payload.rs` is the command
/// `scripts/Build-LocalInstaller.ps1` calls, and it compiles this same file.
///
/// # Errors
///
/// Anything that stops the bytes reaching the disk, and any entry path that
/// [`checked_relative_path`] would refuse to read back.
pub fn append_archive(executable: &Path, entries: &[Entry]) -> Result<(), ArchiveError> {
    let mut archive = Vec::new();
    for entry in entries {
        // Checked on the way out as well as the way in. A packer able to write
        // something the reader must refuse produces an installer that fails on
        // the user's machine rather than on the build machine.
        checked_relative_path(&entry.path)?;
        let path = entry.path.as_bytes();
        let path_length = u32::try_from(path.len()).map_err(|_| ArchiveError::UnsafePath {
            path: entry.path.clone(),
        })?;
        archive.extend_from_slice(&path_length.to_le_bytes());
        archive.extend_from_slice(path);
        archive.extend_from_slice(&(entry.bytes.len() as u64).to_le_bytes());
        archive.extend_from_slice(&entry.bytes);
        archive.extend_from_slice(&Sha256::digest(&entry.bytes));
    }
    let count = u32::try_from(entries.len()).map_err(|_| ArchiveError::Damaged)?;
    let mut trailer = Vec::with_capacity(TRAILER_BYTES);
    trailer.extend_from_slice(&(archive.len() as u64).to_le_bytes());
    trailer.extend_from_slice(&count.to_le_bytes());
    trailer.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    trailer.extend_from_slice(&MAGIC);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(executable)
        .map_err(|error| io(&error))?;
    file.write_all(&archive).map_err(|error| io(&error))?;
    file.write_all(&trailer).map_err(|error| io(&error))?;
    file.flush().map_err(|error| io(&error))
}

/// Collect a directory tree into entries, in a stable order.
///
/// Sorted, so the packed archive — and therefore the installer's SHA-256 — is
/// the same for the same inputs. `read_dir` order is the filesystem's, and a
/// published checksum that changes when nothing did is a checksum nobody
/// trusts.
///
/// # Errors
///
/// Any file that cannot be read, and any name that is not valid UTF-8.
pub fn collect(root: &Path) -> Result<Vec<Entry>, ArchiveError> {
    let mut entries = Vec::new();
    collect_into(root, "", &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn collect_into(
    directory: &Path,
    prefix: &str,
    entries: &mut Vec<Entry>,
) -> Result<(), ArchiveError> {
    for item in std::fs::read_dir(directory).map_err(|error| io(&error))? {
        let item = item.map_err(|error| io(&error))?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|name| ArchiveError::UnsafePath {
                path: name.to_string_lossy().into_owned(),
            })?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if item.file_type().map_err(|error| io(&error))?.is_dir() {
            collect_into(&item.path(), &path, entries)?;
        } else {
            entries.push(Entry {
                path,
                bytes: std::fs::read(item.path()).map_err(|error| io(&error))?,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file standing in for the bootstrapper's own image.
    fn host(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("speakeasy-payload-test-{name}"));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            b"MZ this is not really a PE image, and does not need to be",
        )
        .expect("a temporary file");
        path
    }

    fn entry(path: &str, bytes: &[u8]) -> Entry {
        Entry {
            path: path.to_owned(),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn an_appended_archive_reads_back_exactly() {
        let file = host("round-trip");
        let packed = vec![
            entry("ai-speakeasy-mini.exe", b"the desktop executable"),
            entry("proof/granite-worker.exe", b"the worker, in a subdirectory"),
        ];
        append_archive(&file, &packed).expect("append");

        let read = read_archive(&file)
            .expect("a well-formed archive")
            .expect("an archive is present");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].path, "ai-speakeasy-mini.exe");
        assert_eq!(read[0].bytes, b"the desktop executable");
        assert_eq!(read[1].path, "proof/granite-worker.exe");
        assert_eq!(read[1].bytes, b"the worker, in a subdirectory");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn an_executable_with_no_archive_is_not_an_error() {
        // The locally built bootstrapper. It has to fall through to the
        // `payload/` directory beside it, not refuse to install.
        let file = host("bare");
        assert!(
            read_archive(&file)
                .expect("a bare executable is readable")
                .is_none(),
            "a plain executable must read as carrying no payload"
        );
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_truncated_download_is_refused_rather_than_partly_installed() {
        // The failure this whole format is shaped around: trailing bytes are not
        // part of the PE image, so a half-downloaded installer still runs.
        let file = host("truncated");
        append_archive(&file, &[entry("ai-speakeasy-mini.exe", &[7u8; 4096])]).expect("append");
        let whole = std::fs::read(&file).expect("read back");
        // Cut from the middle of the payload and keep the trailer, which is what
        // an interrupted transfer of a large file looks like rather than a clean
        // cut that would take the magic with it.
        let mut damaged = whole[..whole.len() - 2048].to_vec();
        damaged.extend_from_slice(&whole[whole.len() - TRAILER_BYTES..]);
        std::fs::write(&file, &damaged).expect("write damaged");

        assert!(
            matches!(read_archive(&file), Err(ArchiveError::Damaged)),
            "a truncated archive must be refused"
        );
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_single_flipped_byte_is_refused() {
        // The digest's own job. Length and trailer are intact here, so nothing
        // but the hash distinguishes this from a good archive.
        let file = host("flipped");
        append_archive(&file, &[entry("ai-speakeasy-mini.exe", &[3u8; 512])]).expect("append");
        let mut bytes = std::fs::read(&file).expect("read back");
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        std::fs::write(&file, &bytes).expect("write flipped");

        assert!(
            matches!(read_archive(&file), Err(ArchiveError::Damaged)),
            "a flipped byte must not read back as the packed byte"
        );
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_path_that_escapes_the_payload_root_is_refused_by_both_halves() {
        // `Path::join` with an absolute path replaces the root, so this is the
        // difference between writing into the install directory and writing into
        // `C:\Windows`.
        let file = host("escape");
        for escape in [
            "../outside.exe",
            "/absolute.exe",
            r"C:\windows\evil.exe",
            "",
        ] {
            assert!(
                checked_relative_path(escape).is_err(),
                "{escape} must not be accepted"
            );
            assert!(
                append_archive(&file, &[entry(escape, b"x")]).is_err(),
                "{escape} must not be packable either"
            );
        }
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn collect_orders_its_entries_so_the_installer_hashes_the_same_twice() {
        let root = std::env::temp_dir().join("speakeasy-payload-collect");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proof")).expect("a temporary tree");
        std::fs::write(root.join("zeta.exe"), b"z").expect("write");
        std::fs::write(root.join("alpha.exe"), b"a").expect("write");
        std::fs::write(root.join("proof/granite-worker.exe"), b"g").expect("write");

        let collected = collect(&root).expect("collect");
        assert_eq!(
            collected
                .iter()
                .map(|item| item.path.as_str())
                .collect::<Vec<_>>(),
            ["alpha.exe", "proof/granite-worker.exe", "zeta.exe"]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
