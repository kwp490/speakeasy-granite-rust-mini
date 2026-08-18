use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use speakeasy_domain::CancelToken;
use tar::{Archive, EntryType};

use crate::{
    ArchiveEntry, ArchiveEntryKind, ArchiveLimits, ArchiveValidationError, InstallFile,
    InstallSpec, validate_archive_plan,
};

const MAXIMUM_ARCHIVE_ENTRIES: usize = 4_096;
const MAXIMUM_COMPRESSION_RATIO: u64 = 100;

#[derive(Debug)]
pub enum ArchiveExtractionError {
    Validation(ArchiveValidationError),
    UnsupportedEntry(PathBuf),
    MissingRequiredFiles,
    FileMismatch(PathBuf),
    Cancelled,
    SizeOverflow,
    UnsupportedCompression,
    Io(io::Error),
}

/// How an archive is compressed.
///
/// Decided from the archive's own leading bytes rather than from its file name.
/// The name comes from the manifest, and a pack whose URL ends `.tar.gz` while
/// serving bzip2 should fail as "not the compression you said" rather than as an
/// inscrutable tar parse error fifty entries in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Compression {
    Gzip,
    Bzip2,
    /// Not a compressed tar at all but a container in its own right, which is
    /// why it takes a separate code path below rather than another decoder.
    /// NVIDIA publishes every Windows CUDA and cuDNN redistributable this way.
    Zip,
}

impl Compression {
    fn detect(archive_path: &Path) -> Result<Self, ArchiveExtractionError> {
        let mut magic = [0_u8; 4];
        let read = File::open(archive_path)?.read(&mut magic)?;
        match &magic[..read] {
            [0x1f, 0x8b, ..] => Ok(Self::Gzip),
            // "BZh"
            [0x42, 0x5a, 0x68, ..] => Ok(Self::Bzip2),
            // "PK\x03\x04" local file header, or "PK\x05\x06" for an empty
            // archive. An empty one is accepted here and rejected later by the
            // missing-required-files check, which gives a better error than
            // "unsupported compression" would.
            [0x50, 0x4b, 0x03, 0x04] | [0x50, 0x4b, 0x05, 0x06] => Ok(Self::Zip),
            _ => Err(ArchiveExtractionError::UnsupportedCompression),
        }
    }
}

/// Open `archive_path` as a tar stream, whatever it is compressed with.
///
/// sherpa-onnx publishes its model packs as `.tar.bz2` and the self-exported
/// CUDA pack ships as `.tar.gz`, so both have to work; the decoder is chosen
/// per archive rather than fixed for the product.
fn open_tar(archive_path: &Path) -> Result<Archive<Box<dyn Read>>, ArchiveExtractionError> {
    let source = File::open(archive_path)?;
    let reader: Box<dyn Read> = match Compression::detect(archive_path)? {
        Compression::Gzip => Box::new(GzDecoder::new(source)),
        Compression::Bzip2 => Box::new(BzDecoder::new(source)),
        // Reached only if a caller bypasses `extract_required_files`, which
        // routes zip away from the tar path before this runs. Refusing beats a
        // `todo!()`: a zip fed to the tar reader is a caller bug, not a panic
        // the installer should take at runtime.
        Compression::Zip => return Err(ArchiveExtractionError::UnsupportedCompression),
    };
    Ok(Archive::new(reader))
}

impl Display for ArchiveExtractionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "archive extraction failed: {self:?}")
    }
}

impl Error for ArchiveExtractionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ArchiveExtractionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ArchiveValidationError> for ArchiveExtractionError {
    fn from(error: ArchiveValidationError) -> Self {
        Self::Validation(error)
    }
}

pub(crate) fn extract_required_files(
    archive_path: &Path,
    destination: &Path,
    spec: &InstallSpec,
    cancel: &CancelToken,
) -> Result<(), ArchiveExtractionError> {
    if Compression::detect(archive_path)? == Compression::Zip {
        return extract_required_files_from_zip(archive_path, destination, spec, cancel);
    }
    inspect_complete_archive(archive_path, spec.archive_bytes, cancel)?;
    fs::create_dir_all(destination)?;
    let required: HashMap<_, _> = spec
        .required_files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect();
    let mut found = HashSet::new();
    let mut archive = open_tar(archive_path)?;
    for entry in archive.entries()? {
        if cancel.is_cancelled() {
            return Err(ArchiveExtractionError::Cancelled);
        }
        let mut entry = entry?;
        let archive_path = entry.path()?.into_owned();
        let logical_path = archive_path
            .strip_prefix(&spec.archive_prefix)
            .unwrap_or(&archive_path);
        let Some(expected) = required.get(logical_path) else {
            continue;
        };
        if !entry.header().entry_type().is_file() || !found.insert(logical_path.to_path_buf()) {
            return Err(ArchiveExtractionError::FileMismatch(
                logical_path.to_path_buf(),
            ));
        }
        extract_one(&mut entry, destination, expected, cancel)?;
    }
    if found.len() != required.len() {
        return Err(ArchiveExtractionError::MissingRequiredFiles);
    }
    Ok(())
}

/// The zip equivalent of the tar path above, holding the same guarantees.
///
/// It is a separate function rather than another decoder because zip is a
/// container, not a compression wrapper: entries are reached through a central
/// directory instead of streamed in order. The guarantees are deliberately the
/// same ones, in the same order — inspect the whole archive and validate the
/// plan before writing a single byte, then extract only what `required_files`
/// names, hashing as it goes.
///
/// Two properties carry over unchanged and are worth naming, because they are
/// what make an untrusted archive safe here: the write path is
/// `destination.join(&expected.path)` from the **manifest**, never a path the
/// archive supplied, so a `..` or absolute member name cannot escape; and
/// `extract_one` refuses the moment the byte count exceeds the recorded length,
/// so a zip bomb cannot be written out even if it slipped the ratio check.
fn extract_required_files_from_zip(
    archive_path: &Path,
    destination: &Path,
    spec: &InstallSpec,
    cancel: &CancelToken,
) -> Result<(), ArchiveExtractionError> {
    let mut archive = open_zip(archive_path)?;
    inspect_complete_zip(&mut archive, spec.archive_bytes, cancel)?;
    fs::create_dir_all(destination)?;
    let required: HashMap<_, _> = spec
        .required_files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect();
    let mut found = HashSet::new();
    for index in 0..archive.len() {
        if cancel.is_cancelled() {
            return Err(ArchiveExtractionError::Cancelled);
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|_| ArchiveExtractionError::UnsupportedCompression)?;
        // `enclosed_name` is the crate's own traversal-safe accessor: it returns
        // `None` for absolute paths, drive prefixes and any `..` component. A
        // hostile name therefore never even reaches the lookup.
        let Some(entry_path) = entry.enclosed_name() else {
            continue;
        };
        let logical_path = entry_path
            .strip_prefix(&spec.archive_prefix)
            .unwrap_or(&entry_path)
            .to_path_buf();
        let Some(expected) = required.get(&logical_path) else {
            continue;
        };
        if !entry.is_file() || !found.insert(logical_path.clone()) {
            return Err(ArchiveExtractionError::FileMismatch(logical_path));
        }
        extract_one(&mut entry, destination, expected, cancel)?;
    }
    if found.len() != required.len() {
        return Err(ArchiveExtractionError::MissingRequiredFiles);
    }
    Ok(())
}

fn open_zip(archive_path: &Path) -> Result<zip::ZipArchive<File>, ArchiveExtractionError> {
    zip::ZipArchive::new(File::open(archive_path)?)
        .map_err(|_| ArchiveExtractionError::UnsupportedCompression)
}

/// Builds the same entry plan `inspect_complete_archive` builds, from the zip
/// central directory, and hands it to the same validator.
///
/// The central directory is metadata the archive asserts about itself, so the
/// declared sizes here are claims rather than measurements — which is exactly
/// how the tar path treats `entry.size()` too. Neither is trusted for
/// correctness; both feed the ratio and count limits so an obviously hostile
/// archive is refused before anything is written, and the real enforcement
/// stays the per-file length-and-digest check in `extract_one`.
fn inspect_complete_zip(
    archive: &mut zip::ZipArchive<File>,
    compressed_bytes: u64,
    cancel: &CancelToken,
) -> Result<(), ArchiveExtractionError> {
    if archive.len() > MAXIMUM_ARCHIVE_ENTRIES {
        return Err(ArchiveExtractionError::Validation(
            ArchiveValidationError::TooManyFiles {
                actual: archive.len(),
                maximum: MAXIMUM_ARCHIVE_ENTRIES,
            },
        ));
    }
    let mut entries = Vec::new();
    let mut regular_paths = HashSet::new();
    for index in 0..archive.len() {
        if cancel.is_cancelled() {
            return Err(ArchiveExtractionError::Cancelled);
        }
        let entry = archive
            .by_index(index)
            .map_err(|_| ArchiveExtractionError::UnsupportedCompression)?;
        // A name the crate refuses to enclose is a traversal attempt. The tar
        // path has no equivalent because `validate_archive_plan` sees the raw
        // path there; here the safe accessor is the check, so a rejected name is
        // an error rather than something to skip past quietly.
        let path = entry
            .enclosed_name()
            .ok_or_else(|| ArchiveExtractionError::UnsupportedEntry(PathBuf::from(entry.name())))?;
        let kind = classify_zip(&entry, &path);
        if kind == ArchiveEntryKind::File {
            regular_paths.insert(path.clone());
        }
        entries.push(ArchiveEntry {
            path,
            kind,
            compressed_bytes,
            extracted_bytes: entry.size(),
        });
    }
    let maximum_extracted_bytes = compressed_bytes
        .checked_mul(MAXIMUM_COMPRESSION_RATIO)
        .ok_or(ArchiveExtractionError::SizeOverflow)?;
    validate_archive_plan(
        entries,
        &regular_paths,
        ArchiveLimits {
            maximum_files: MAXIMUM_ARCHIVE_ENTRIES,
            maximum_extracted_bytes,
            maximum_compression_ratio: MAXIMUM_COMPRESSION_RATIO,
        },
    )?;
    Ok(())
}

/// Zip has no entry-type byte; a symlink is a Unix mode in the external
/// attributes, which Windows-produced archives leave unset. Anything that is
/// not plainly a directory or a regular file is reported as a symlink so
/// `validate_archive_plan` refuses it, matching how the tar path treats
/// everything exotic.
fn classify_zip<R: Read>(entry: &zip::read::ZipFile<'_, R>, path: &Path) -> ArchiveEntryKind {
    const UNIX_FILE_TYPE_MASK: u32 = 0xF000;
    const UNIX_SYMLINK: u32 = 0xA000;
    if entry.is_dir() || path.as_os_str().is_empty() {
        return ArchiveEntryKind::Directory;
    }
    match entry.unix_mode() {
        Some(mode) if mode & UNIX_FILE_TYPE_MASK == UNIX_SYMLINK => ArchiveEntryKind::Symlink,
        _ if entry.is_file() => ArchiveEntryKind::File,
        _ => ArchiveEntryKind::Symlink,
    }
}

fn inspect_complete_archive(
    archive_path: &Path,
    compressed_bytes: u64,
    cancel: &CancelToken,
) -> Result<(), ArchiveExtractionError> {
    let mut archive = open_tar(archive_path)?;
    let mut entries = Vec::new();
    let mut regular_paths = HashSet::new();
    for entry in archive.entries()? {
        if cancel.is_cancelled() {
            return Err(ArchiveExtractionError::Cancelled);
        }
        let entry = entry?;
        let path = entry.path()?.into_owned();
        let kind = classify(entry.header().entry_type(), &path)?;
        if kind == ArchiveEntryKind::File {
            regular_paths.insert(path.clone());
        }
        entries.push(ArchiveEntry {
            path,
            kind,
            compressed_bytes,
            extracted_bytes: entry.size(),
        });
        if entries.len() > MAXIMUM_ARCHIVE_ENTRIES {
            return Err(ArchiveExtractionError::Validation(
                ArchiveValidationError::TooManyFiles {
                    actual: entries.len(),
                    maximum: MAXIMUM_ARCHIVE_ENTRIES,
                },
            ));
        }
    }
    let maximum_extracted_bytes = compressed_bytes
        .checked_mul(MAXIMUM_COMPRESSION_RATIO)
        .ok_or(ArchiveExtractionError::SizeOverflow)?;
    validate_archive_plan(
        entries,
        &regular_paths,
        ArchiveLimits {
            maximum_files: MAXIMUM_ARCHIVE_ENTRIES,
            maximum_extracted_bytes,
            maximum_compression_ratio: MAXIMUM_COMPRESSION_RATIO,
        },
    )?;
    Ok(())
}

fn classify(
    entry_type: EntryType,
    path: &Path,
) -> Result<ArchiveEntryKind, ArchiveExtractionError> {
    match entry_type {
        EntryType::Regular => Ok(ArchiveEntryKind::File),
        EntryType::Directory => Ok(ArchiveEntryKind::Directory),
        EntryType::Link => Ok(ArchiveEntryKind::HardLink),
        EntryType::Symlink => Ok(ArchiveEntryKind::Symlink),
        EntryType::Char
        | EntryType::Block
        | EntryType::Fifo
        | EntryType::Continuous
        | EntryType::GNUSparse
        | EntryType::__Nonexhaustive(_) => Ok(ArchiveEntryKind::ReparsePoint),
        EntryType::GNULongName
        | EntryType::GNULongLink
        | EntryType::XGlobalHeader
        | EntryType::XHeader => Err(ArchiveExtractionError::UnsupportedEntry(path.to_path_buf())),
    }
}

fn extract_one<R: Read>(
    entry: &mut R,
    destination: &Path,
    expected: &InstallFile,
    cancel: &CancelToken,
) -> Result<(), ArchiveExtractionError> {
    let path = destination.join(&expected.path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(ArchiveExtractionError::Cancelled);
        }
        let count = entry.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or(ArchiveExtractionError::SizeOverflow)?;
        if bytes > expected.bytes {
            return Err(ArchiveExtractionError::FileMismatch(expected.path.clone()));
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    output.sync_all()?;
    if bytes != expected.bytes
        || !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(&expected.sha256)
    {
        return Err(ArchiveExtractionError::FileMismatch(expected.path.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod zip_tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Writes a zip whose members are `(name, bytes, unix_mode)`.
    fn write_zip(path: &Path, members: &[(&str, &[u8], Option<u32>)]) -> u64 {
        let mut writer = zip::ZipWriter::new(File::create(path).expect("create zip"));
        for (name, bytes, mode) in members {
            let mut options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            if let Some(mode) = mode {
                options = options.unix_permissions(*mode);
            }
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish zip");
        fs::metadata(path).expect("zip metadata").len()
    }

    fn spec_for(archive_bytes: u64, prefix: &str, files: Vec<InstallFile>) -> InstallSpec {
        InstallSpec {
            id: "synthetic-zip".to_owned(),
            revision: "r1".to_owned(),
            archive_prefix: PathBuf::from(prefix),
            archive_bytes,
            archive_sha256: String::new(),
            installed_bytes: files.iter().map(|file| file.bytes).sum(),
            required_files: files,
        }
    }

    #[test]
    fn a_zip_extracts_only_its_required_files_and_strips_the_prefix() {
        let temp = tempfile::tempdir().expect("temp");
        let archive = temp.path().join("pack.zip");
        let wanted = b"cuda runtime bytes".as_slice();
        let bytes = write_zip(
            &archive,
            &[
                ("redist/bin/wanted.dll", wanted, None),
                ("redist/include/ignored.h", b"header".as_slice(), None),
            ],
        );
        let spec = spec_for(
            bytes,
            "redist",
            vec![InstallFile {
                path: PathBuf::from("bin/wanted.dll"),
                bytes: wanted.len() as u64,
                sha256: digest(wanted),
            }],
        );

        let destination = temp.path().join("out");
        extract_required_files(&archive, &destination, &spec, &CancelToken::default())
            .expect("zip extraction");

        assert_eq!(
            fs::read(destination.join("bin/wanted.dll")).expect("extracted"),
            wanted
        );
        assert!(
            !destination.join("include/ignored.h").exists(),
            "only required_files are written, exactly as the tar path behaves"
        );
    }

    #[test]
    fn a_zip_member_whose_bytes_do_not_match_is_refused() {
        let temp = tempfile::tempdir().expect("temp");
        let archive = temp.path().join("pack.zip");
        let actual = b"actual bytes".as_slice();
        let bytes = write_zip(&archive, &[("redist/bin/wanted.dll", actual, None)]);
        // Right length, wrong digest -- the case a length-only check would pass.
        let spec = spec_for(
            bytes,
            "redist",
            vec![InstallFile {
                path: PathBuf::from("bin/wanted.dll"),
                bytes: actual.len() as u64,
                sha256: digest(b"different bytes entirely"),
            }],
        );

        let error = extract_required_files(
            &archive,
            &temp.path().join("out"),
            &spec,
            &CancelToken::default(),
        )
        .expect_err("a digest mismatch must refuse");
        assert!(matches!(error, ArchiveExtractionError::FileMismatch(_)));
    }

    #[test]
    fn a_zip_missing_a_required_file_is_refused_rather_than_partially_installed() {
        let temp = tempfile::tempdir().expect("temp");
        let archive = temp.path().join("pack.zip");
        let present = b"present".as_slice();
        let bytes = write_zip(&archive, &[("redist/bin/present.dll", present, None)]);
        let mut files = vec![InstallFile {
            path: PathBuf::from("bin/present.dll"),
            bytes: present.len() as u64,
            sha256: digest(present),
        }];
        files.push(InstallFile {
            path: PathBuf::from("bin/absent.dll"),
            bytes: 4,
            sha256: digest(b"gone"),
        });
        let spec = spec_for(bytes, "redist", files);

        let error = extract_required_files(
            &archive,
            &temp.path().join("out"),
            &spec,
            &CancelToken::default(),
        )
        .expect_err("a missing required file must refuse");
        assert!(matches!(
            error,
            ArchiveExtractionError::MissingRequiredFiles
        ));
    }

    #[test]
    fn a_symlink_entry_is_refused_before_anything_is_written() {
        // Zip has no entry-type byte, so a symlink is a Unix mode smuggled in
        // the external attributes. Windows-produced archives never set it,
        // which is exactly why it must be checked rather than assumed absent.
        //
        // Written with `add_symlink` rather than `unix_permissions`: the latter
        // masks the mode to 0o777 and silently drops the S_IFLNK type bits, so
        // an earlier version of this test created an ordinary file, extracted
        // it happily, and proved nothing.
        let temp = tempfile::tempdir().expect("temp");
        let archive = temp.path().join("pack.zip");
        let target = "../../elsewhere";
        let bytes = {
            let mut writer = zip::ZipWriter::new(File::create(&archive).expect("create zip"));
            writer
                .add_symlink("redist/bin/link.dll", target, SimpleFileOptions::default())
                .expect("write symlink entry");
            writer.finish().expect("finish zip");
            fs::metadata(&archive).expect("zip metadata").len()
        };
        let spec = spec_for(
            bytes,
            "redist",
            vec![InstallFile {
                path: PathBuf::from("bin/link.dll"),
                bytes: target.len() as u64,
                sha256: digest(target.as_bytes()),
            }],
        );

        let destination = temp.path().join("out");
        let error = extract_required_files(&archive, &destination, &spec, &CancelToken::default())
            .expect_err("a symlink must refuse");
        assert!(
            matches!(
                error,
                ArchiveExtractionError::Validation(ArchiveValidationError::UnsupportedEntry { .. })
            ),
            "got {error:?}"
        );
        assert!(
            !destination.join("bin/link.dll").exists(),
            "and it must refuse during inspection, before any byte is written"
        );
    }

    /// The spike: the real NVIDIA redistributable, not a fixture.
    ///
    /// `cuda_cudart` is the smallest of the four (3.5 MB, one required DLL), so
    /// it proves the whole zip path end to end -- central directory, prefix
    /// stripping, length and digest enforcement -- against bytes NVIDIA actually
    /// served, at the digest `models/trusted-manifest.json` pins.
    ///
    /// Ignored because it needs `scripts/Get-GpuRuntime.ps1` to have run. The
    /// values are restated here rather than read from the manifest only because
    /// `ProofArtifact::NativeRuntime` exposes nothing but `id()` today; the
    /// production path must read them from the manifest, not duplicate them.
    #[test]
    #[ignore = "requires .tools/gpu-runtime/download populated by scripts/Get-GpuRuntime.ps1"]
    fn the_real_nvidia_cudart_redistributable_extracts_at_its_pinned_digest() {
        let archive = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tools/gpu-runtime/download")
            .join("cuda_cudart-windows-x86_64-12.9.79-archive.zip");
        assert!(archive.is_file(), "missing {}", archive.display());

        let spec = InstallSpec {
            id: "nvidia-cuda-cudart-windows-x64-12.9.79".to_owned(),
            revision: "12.9.79".to_owned(),
            archive_prefix: PathBuf::from("cuda_cudart-windows-x86_64-12.9.79-archive"),
            archive_bytes: 3_521_238,
            archive_sha256: "179e9c43b0735ffe67207b3da556eb5a0c50f3047961882b7657d3b822d34ef8"
                .to_owned(),
            installed_bytes: 583_680,
            required_files: vec![InstallFile {
                path: PathBuf::from("bin/cudart64_12.dll"),
                bytes: 583_680,
                sha256: "760c38928bbe5759f7b31ed6692599eb7ec83cedd5702e84c2b72028a89837e1"
                    .to_owned(),
            }],
        };

        let temp = tempfile::tempdir().expect("temp");
        let destination = temp.path().join("proof");
        extract_required_files(&archive, &destination, &spec, &CancelToken::default())
            .expect("the real cudart archive must extract at its pinned digest");

        let installed = destination.join("bin/cudart64_12.dll");
        assert_eq!(
            fs::metadata(&installed).expect("installed dll").len(),
            583_680,
            "and the file on disk is the length the manifest recorded"
        );
    }
}
