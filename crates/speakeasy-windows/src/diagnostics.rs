//! The single file-writing boundary for diagnostics, including worker stderr.
//!
//! Here rather than in `apps/desktop` because `worker_process` in this crate
//! writes through it, and a second writer beside this one would be a second
//! place for the redaction below to be missed. `apps/bootstrapper` reaches it
//! too, so setup's own worker stderr lands under the same rule.

use std::fs;
use std::io::Write as _;
use std::path::Path;

/// Rotation threshold. Public so `apps/desktop`'s rotation test can seed a
/// log just past it rather than hardcoding a second copy of the number.
pub const DIAGNOSTICS_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// The single file-writing boundary for diagnostics, including worker stderr.
/// Redaction happens here as a defense in depth even when a caller has already
/// reduced its values to structured fields. This keeps future writers from
/// quietly bypassing the privacy promise.
///
/// # Errors
///
/// Returns the underlying `io::Error` when the parent directory cannot be
/// created, the rotation rename fails, or the append itself fails. Callers on
/// the diagnostic path deliberately discard it: a log that cannot be written
/// must not fail a dictation or a worker spawn.
pub fn append_diagnostics_line(path: &Path, line: &str) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "diagnostic path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() > DIAGNOSTICS_LOG_MAX_BYTES) {
        let rotated = path.with_extension("log.1");
        // Windows cannot rename over an existing destination. Only the prior
        // generation is removed; the active log is preserved until rename.
        let _ = fs::remove_file(&rotated);
        fs::rename(path, rotated)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let sanitized = redact_diagnostic_text(line);
    file.write_all(sanitized.as_bytes())
}

/// Redacts path-shaped substrings from native panic and loader messages before
/// they can reach the persistent diagnostic surface. Transcript text is not
/// sent through this path, but native error strings are not trusted to obey
/// that boundary themselves.
pub fn redact_diagnostic_text(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut token_start = 0;
    for (index, character) in line.char_indices() {
        if character.is_whitespace() {
            if token_start < index {
                output.push_str(&redact_diagnostic_token(&line[token_start..index]));
            }
            output.push(character);
            token_start = index + character.len_utf8();
        }
    }
    if token_start < line.len() {
        output.push_str(&redact_diagnostic_token(&line[token_start..]));
    }
    output
}

fn redact_diagnostic_token(token: &str) -> String {
    let bytes = token.as_bytes();
    let mut path_start = None;
    for index in 0..bytes.len() {
        let windows_drive = index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/');
        let unc = index + 1 < bytes.len() && bytes[index] == b'\\' && bytes[index + 1] == b'\\';
        let unix = bytes[index] == b'/'
            && (index == 0 || matches!(bytes[index - 1], b'=' | b'(' | b'[' | b'"' | b'\''));
        if windows_drive || unc || unix {
            path_start = Some(index);
            break;
        }
    }
    let Some(path_start) = path_start else {
        return token.to_owned();
    };
    let prefix = &token[..path_start];
    format!("{prefix}<redacted-path>")
}
